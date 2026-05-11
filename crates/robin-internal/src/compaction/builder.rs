use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tracing::warn;

use crate::config::config::Config;
use crate::llm::provider::{new_provider, parse_provider_model, ProviderOptions};

use super::compaction::Manager;
use super::summarizer::Summarizer;

/// BuildManager constructs a compaction Manager pinned to the default agent's
/// model (or cfg.agents.defaults.compaction.model if explicitly set).
///
/// Returns None when compaction is disabled or no provider is configured —
/// callers must treat None as "compaction off".
pub fn build_manager(cfg: &Config) -> Option<Arc<Manager>> {
    if !cfg.agents.defaults.compaction.enabled {
        return None;
    }
    let mut model_str = cfg.agents.defaults.compaction.model.clone();
    if model_str.is_empty() {
        if let Some(first) = cfg.agents.list.first() {
            model_str = first.model.clone();
        }
    }
    build_manager_for_model(cfg, &model_str)
}

/// Provider builds and caches per-agent compaction Managers, keyed by the
/// chatting agent's "provider/model".  The cache matters because Manager has
/// per-session locks — two requests on the same session must hit the same
/// Manager instance to serialize correctly.
pub struct Provider {
    cfg: Arc<Config>,
    mu: Mutex<HashMap<String, Option<Arc<Manager>>>>,
}

impl Provider {
    /// NewProvider returns a per-agent compaction Manager factory.
    /// Returns None when compaction is globally disabled.
    pub fn new(cfg: Arc<Config>) -> Option<Arc<Self>> {
        if !cfg.agents.defaults.compaction.enabled {
            return None;
        }
        Some(Arc::new(Provider {
            cfg,
            mu: Mutex::new(HashMap::new()),
        }))
    }

    /// For returns the Manager for the given chat-agent model. If
    /// compaction.model is explicitly pinned in config, that overrides
    /// agent_model. Returns None if the resolved provider is missing.
    pub fn for_model(&self, agent_model: &str) -> Option<Arc<Manager>> {
        let model_str = if self.cfg.agents.defaults.compaction.model.is_empty() {
            agent_model.to_string()
        } else {
            self.cfg.agents.defaults.compaction.model.clone()
        };

        let mut cache = self.mu.lock();
        if let Some(entry) = cache.get(&model_str) {
            return entry.clone();
        }
        let m = build_manager_for_model(&self.cfg, &model_str);
        cache.insert(model_str, m.clone());
        m
    }
}

/// buildManagerForModel builds a single Manager wired to the given
/// "provider/model" string. Returns None when the provider is unconfigured.
fn build_manager_for_model(cfg: &Config, model_str: &str) -> Option<Arc<Manager>> {
    let c = &cfg.agents.defaults.compaction;
    let (mut provider_name, model) = {
        let (p, m) = parse_provider_model(model_str);
        (p.to_string(), m.to_string())
    };
    if provider_name.is_empty() {
        provider_name = "local".to_string();
    }

    let pcfg = cfg.providers.get(&provider_name).cloned().unwrap_or_default();

    // "Configured enough to talk to" means we have either an API key (native
    // SDKs like anthropic/openai/gemini work without a baseURL) or a baseURL
    // (local Ollama, openai-compatible proxies).
    if pcfg.api_key.is_empty() && pcfg.base_url.is_empty() {
        warn!(
            provider = %provider_name,
            model = %model_str,
            "compaction disabled: provider not configured"
        );
        return None;
    }

    let llm_prov = match new_provider(
        &provider_name,
        ProviderOptions {
            api_key: pcfg.api_key.clone(),
            base_url: pcfg.base_url.clone(),
            kind: pcfg.kind.clone(),
            ca_bundle: pcfg.ca_bundle.clone(),
        },
    ) {
        Ok(p) => p,
        Err(e) => {
            warn!(
                error = %e,
                model = %model_str,
                "compaction disabled: failed to build provider"
            );
            return None;
        }
    };

    let timeout_sec = c.timeout_sec;
    let timeout = if timeout_sec == 0 {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(timeout_sec as u64)
    };

    tracing::info!(provider = %provider_name, model = %model, "compaction manager built");

    Some(Arc::new(Manager::new(
        Arc::new(Summarizer {
            provider: Arc::from(llm_prov),
            model,
            timeout,
        }),
        if c.preserve_turns <= 0 { 0 } else { c.preserve_turns as usize },
        c.threshold,
        c.message_cap,
    )))
}

#[cfg(test)]
#[path = "builder_test.rs"]
mod builder_test;