use std::collections::HashMap;
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::warn;

// ── Output types for ResolveMCPServers (mirrors mcp package in Go) ───────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MCPHTTPAuthConfig {
    pub kind: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
    pub auth_url: String,
    pub redirect_uri: String,
    pub token_store_path: String,
    pub bearer_token: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MCPHTTPServerConfig {
    pub url: String,
    pub auth: MCPHTTPAuthConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MCPStdioServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MCPManagerServerConfig {
    pub id: String,
    pub tool_prefix: String,
    pub transport: String,
    pub http: Option<MCPHTTPServerConfig>,
    pub stdio: Option<MCPStdioServerConfig>,
    pub parallel_safe: bool,
}

// ── Config structs ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub enabled: bool,
    pub bot_token: String,
    pub default_chat_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPAuthConfig {
    pub kind: String,
    pub token_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub client_secret_env: Option<String>,
    pub scope: Option<String>,
    pub auth_url: Option<String>,
    pub redirect_uri: Option<String>,
    pub token: Option<String>,
    pub token_env: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MCPHTTPBlock {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub auth: MCPAuthConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MCPStdioBlock {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MCPServerConfig {
    pub id: String,
    pub transport: String,
    pub http: Option<MCPHTTPBlock>,
    pub stdio: Option<MCPStdioBlock>,
    pub url: String,
    pub auth: MCPAuthConfig,
    pub enabled: bool,
    pub parallel_safe: bool,
    pub tool_prefix: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub kind: String,
    pub base_url: String,
    pub api_key: String,
    pub ca_bundle: String,
}


#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub reload: ReloadConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub token: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReloadConfig {
    pub mode: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    pub list: Vec<AgentConfig>,
    pub defaults: AgentsDefaults,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsDefaults {
    pub compaction: CompactionConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompactionConfig {
    pub enabled: bool,
    pub model: String,
    pub threshold: f64,
    pub preserve_turns: i32,
    pub timeout_sec: i32,
    pub message_cap: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub workspace: String,
    pub model: String,
    pub reasoning: String,
    pub fallbacks: Vec<String>,
    pub sandbox: String,
    pub max_turns: i32,
    pub system_prompt: String,
    pub tools: ToolPolicy,
    pub cron: Vec<CronConfig>,
    pub subagent: bool,
    pub description: String,
    pub inherit_context: bool,
    pub fallback_model: String,
    pub context_window: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CronConfig {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPolicy {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub agent_id: String,
    pub r#match: BindingMatch,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingMatch {
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub chat_type: String,
    pub peer: Option<PeerMatch>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PeerMatch {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    pub cli: CLIConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CLIConfig {
    pub enabled: bool,
    pub interactive: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub max_entries: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OTelConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub service_name: String,
    pub sample_ratio: f64,
    pub headers: HashMap<String, String>,
    pub signals: OTelSignals,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OTelSignals {
    pub traces: bool,
    pub metrics: bool,
    pub logs: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CortexConfig {
    pub enabled: bool,
    pub db_path: String,
    pub provider: String,
    pub llm_model: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentLoopConfig {
    pub max_tool_concurrency: i32,
    pub max_agent_depth: i32,
    pub streaming_tools: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityConfig {
    pub exec_approvals: ExecApprovalsConfig,
    pub group_policy: GroupPolicyConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecApprovalsConfig {
    pub level: String,
    pub allowlist: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GroupPolicyConfig {
    pub require_mention: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    pub backend: String,
    pub api_key: String,
    pub base_url: String,
}

// ── Main Config ────────────────────────────────────────────────────────────────

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    pub agents: AgentsConfig,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    pub channels: ChannelsConfig,
    pub memory: MemoryConfig,
    pub cortex: CortexConfig,
    #[serde(default, rename = "agentLoop")]
    pub agent_loop: AgentLoopConfig,
    pub security: SecurityConfig,
    pub telegram: TelegramConfig,
    pub web_search: WebSearchConfig,
    #[serde(default, rename = "mcp_servers")]
    pub mcp_servers: Vec<MCPServerConfig>,
    pub o_tel: OTelConfig,

    #[serde(skip)]
    pub(crate) path: String,
    #[serde(skip)]
    pub(crate) mcp_auto_added_names: Vec<String>,
    #[serde(skip)]
    pub(crate) mu: RwLock<()>,
}

impl Config {
    pub fn default_data_dir_path() -> PathBuf {
        PathBuf::from(default_data_dir())
    }

    pub fn data_dir(&self) -> String {
        if self.path.is_empty() { return default_data_dir(); }
        Path::new(&self.path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(default_data_dir)
    }

    pub fn path(&self) -> &str { &self.path }
    pub fn set_path(&mut self, path: &str) { self.path = path.to_string(); }

    pub fn get_provider(&self, name: &str) -> ProviderConfig {
        let _g = self.mu.read();
        self.providers.get(name).cloned().unwrap_or_default()
    }

    pub fn get_agent(&self, id: &str) -> Option<AgentConfig> {
        let _g = self.mu.read();
        self.agents.list.iter().find(|a| a.id == id).cloned()
    }

    pub fn eligible_subagents(&self) -> HashMap<String, String> {
        let _g = self.mu.read();
        self.agents.list.iter()
            .filter(|a| a.subagent)
            .map(|a| (a.id.clone(), a.description.clone()))
            .collect()
    }

    pub fn is_server_parallel_safe(&self, id: &str) -> bool {
        let _g = self.mu.read();
        self.mcp_servers.iter().find(|s| s.id == id).map(|s| s.parallel_safe).unwrap_or(false)
    }

    pub fn validate(&mut self) -> anyhow::Result<()> {
        if self.gateway.port == 0 { self.gateway.port = 18789; }
        if self.gateway.host.is_empty() { self.gateway.host = "127.0.0.1".to_string(); }
        if self.gateway.reload.mode.is_empty() { self.gateway.reload.mode = "hybrid".to_string(); }

        if self.memory == (MemoryConfig::default()) {
            self.memory = default_config().memory;
        } else if self.memory.embedding_model.is_empty() {
            self.memory.embedding_model = "nomic-embed-text".to_string();
        }

        if self.cortex == (CortexConfig::default()) {
            self.cortex = CortexConfig { enabled: true, ..Default::default() };
        }

        if self.agents.list.is_empty() {
            anyhow::bail!("at least one agent must be configured");
        }
        for (i, a) in self.agents.list.iter_mut().enumerate() {
            if a.id.is_empty() { anyhow::bail!("agent at index {} has no id", i); }
            if a.model.is_empty() { anyhow::bail!("agent {:?} has no model", a.id); }
            if a.workspace.is_empty() {
                a.workspace = format!("{}/workspace-{}", default_data_dir(), a.id);
            }
            if a.sandbox.is_empty() { a.sandbox = "none".to_string(); }
            validate_reasoning_mode(&a.reasoning)
                .map_err(|e| anyhow::anyhow!("agent {:?}: {}", a.id, e))?;
            if a.subagent && a.description.is_empty() {
                anyhow::bail!("agent {:?}: subagent=true requires non-empty description", a.id);
            }
        }
        Ok(())
    }

    pub fn update_from(&mut self, src: &Config) {
        let _g = self.mu.write();
        self.gateway = src.gateway.clone();
        self.providers = src.providers.clone();
        self.agents = src.agents.clone();
        self.bindings = src.bindings.clone();
        self.channels = src.channels.clone();
        self.memory = src.memory.clone();
        self.cortex = src.cortex.clone();
        self.agent_loop = src.agent_loop.clone();
        self.security = src.security.clone();
        self.telegram = src.telegram.clone();
        self.web_search = src.web_search.clone();
        self.mcp_servers = src.mcp_servers.clone();
        self.o_tel = src.o_tel.clone();
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = if self.path.is_empty() { default_config_path() } else { self.path.clone() };
        let data = serde_json::to_vec_pretty(self)?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    pub fn apply_mcp_tool_names_to_allowlists(&mut self, names: &[String]) {
        if names.is_empty() { return; }
        let _g = self.mu.write();
        for agent in self.agents.list.iter_mut() {
            if agent.tools.allow.is_empty() { continue; }
            let existing: std::collections::HashSet<_> = agent.tools.allow.iter().cloned().collect();
            for n in names {
                if !existing.contains(n) {
                    agent.tools.allow.push(n.clone());
                }
            }
        }
        self.mcp_auto_added_names = names.to_vec();
    }

    pub fn apply_task_tool_to_allowlists(&mut self) {
        let has_subagent = self.agents.list.iter().any(|a| a.subagent);
        if !has_subagent { return; }
        const TASK: &str = "task";
        let mut added = false;
        for agent in self.agents.list.iter_mut() {
            if agent.tools.allow.is_empty() { continue; }
            if !agent.tools.allow.iter().any(|n| n == TASK) {
                agent.tools.allow.push(TASK.to_string());
                added = true;
            }
        }
        if added && !self.mcp_auto_added_names.iter().any(|n| n == TASK) {
            self.mcp_auto_added_names.push(TASK.to_string());
        }
    }

    pub fn strip_mcp_auto_added(&self, other: &mut Config) {
        let _g = self.mu.read();
        if self.mcp_auto_added_names.is_empty() { return; }
        let name_set: std::collections::HashSet<_> = self.mcp_auto_added_names.iter().cloned().collect();
        for agent in other.agents.list.iter_mut() {
            if agent.tools.allow.is_empty() { continue; }
            agent.tools.allow.retain(|n| !name_set.contains(n));
        }
    }

    pub fn resolve_mcp_servers(&self) -> anyhow::Result<Vec<MCPManagerServerConfig>> {
        let mut out = Vec::new();
        for s in &self.mcp_servers {
            if !s.enabled { continue; }
            if s.id.is_empty() {
                warn!("mcp_servers: skipping entry with empty id");
                continue;
            }
            let transport = if s.transport.is_empty() { "http" } else { s.transport.as_str() };
            match transport {
                "http" => {
                    let (http_block, skip) = resolve_http_block(s, &self.data_dir())?;
                    if skip { continue; }
                    out.push(MCPManagerServerConfig {
                        id: s.id.clone(),
                        tool_prefix: s.tool_prefix.clone(),
                        transport: "http".to_string(),
                        http: http_block,
                        parallel_safe: s.parallel_safe,
                        ..Default::default()
                    });
                }
                "stdio" => {
                    let stdio = s.stdio.as_ref()
                        .filter(|st| !st.command.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("mcp_servers[{}]: stdio transport requires stdio.command", s.id))?;
                    out.push(MCPManagerServerConfig {
                        id: s.id.clone(),
                        tool_prefix: s.tool_prefix.clone(),
                        transport: "stdio".to_string(),
                        stdio: Some(MCPStdioServerConfig {
                            command: stdio.command.clone(),
                            args: stdio.args.clone(),
                            env: stdio.env.clone(),
                        }),
                        parallel_safe: s.parallel_safe,
                        ..Default::default()
                    });
                }
                other => anyhow::bail!("mcp_servers[{}]: unsupported transport {:?}", s.id, other),
            }
        }
        Ok(out)
    }

    /// Returns a simple allow/deny policy map for use with tools::PermissionChecker.
    pub fn tool_policies(&self) -> HashMap<String, (Vec<String>, Vec<String>)> {
        self.agents.list.iter()
            .map(|a| (a.id.clone(), (a.tools.allow.clone(), a.tools.deny.clone())))
            .collect()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn default_data_dir() -> String {
    dirs::home_dir()
        .map(|h| h.join(".robin").to_string_lossy().to_string())
        .unwrap_or_else(|| ".robin".to_string())
}

pub fn default_config_path() -> String {
    format!("{}/robin.json5", default_data_dir())
}

pub fn load(path: &str) -> anyhow::Result<Config> {
    let resolved = if path.is_empty() { default_config_path() } else { path.to_string() };
    let data = match std::fs::read_to_string(&resolved) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut cfg = default_config();
            cfg.path = resolved;
            return Ok(cfg);
        }
        Err(e) => anyhow::bail!("read config: {}", e),
        Ok(d) => d,
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&resolved) {
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                warn!(
                    path = %resolved,
                    mode = format!("{:04o}", mode),
                    recommended = "0600",
                    "config file has overly permissive permissions"
                );
            }
        }
    }

    let cleaned = strip_json5(&data);
    let mut cfg: Config = serde_json::from_str(&cleaned)
        .map_err(|e| anyhow::anyhow!("parse config: {}", e))?;
    cfg.path = resolved;

    apply_otel_env_overrides(&mut cfg.o_tel);
    backfill_compaction_defaults(&mut cfg);
    cfg.validate()?;
    Ok(cfg)
}

fn backfill_compaction_defaults(cfg: &mut Config) {
    let d = default_config().agents.defaults.compaction;
    let cur = &mut cfg.agents.defaults.compaction;
    if cur.threshold == 0.0 && cur.preserve_turns == 0 && cur.timeout_sec == 0 {
        let model = cur.model.clone();
        *cur = d;
        cur.model = model;
    } else {
        if cur.threshold == 0.0 { cur.threshold = d.threshold; }
        if cur.preserve_turns == 0 { cur.preserve_turns = d.preserve_turns; }
        if cur.timeout_sec == 0 { cur.timeout_sec = d.timeout_sec; }
        if cur.message_cap == 0 { cur.message_cap = d.message_cap; }
    }
}

pub fn default_config() -> Config {
    Config {
        gateway: GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 18789,
            reload: ReloadConfig { mode: "hybrid".to_string() },
            ..Default::default()
        },
        providers: HashMap::new(),
        agents: AgentsConfig {
            list: vec![AgentConfig {
                id: "default".to_string(),
                name: "Robin".to_string(),
                workspace: format!("{}/workspace-default", default_data_dir()),
                model: String::new(),
                sandbox: "none".to_string(),
                tools: ToolPolicy {
                    allow: vec!["read_file", "write_file", "edit_file", "bash", "web_fetch", "web_search", "browser", "cron"]
                        .into_iter().map(|s| s.to_string()).collect(),
                    deny: vec![],
                },
                ..Default::default()
            }],
            defaults: AgentsDefaults {
                compaction: CompactionConfig {
                    enabled: true,
                    model: String::new(),
                    threshold: 0.6,
                    preserve_turns: 4,
                    timeout_sec: 60,
                    message_cap: 50,
                },
            },
        },
        bindings: vec![Binding {
            agent_id: "default".to_string(),
            r#match: BindingMatch { channel: "cli".to_string(), ..Default::default() },
        }],
        channels: ChannelsConfig { cli: CLIConfig { enabled: true, interactive: true } },
        memory: MemoryConfig {
            enabled: true,
            embedding_provider: String::new(),
            embedding_model: String::new(),
            max_entries: 0,
        },
        cortex: CortexConfig { enabled: true, ..Default::default() },
        o_tel: OTelConfig {
            enabled: false,
            service_name: "robin".to_string(),
            sample_ratio: 1.0,
            signals: OTelSignals { traces: true, metrics: true, logs: true },
            ..Default::default()
        },
        security: SecurityConfig {
            exec_approvals: ExecApprovalsConfig {
                level: "full".to_string(),
                allowlist: vec!["ls", "cat", "find", "grep", "head", "tail", "wc", "pwd", "date"]
                    .into_iter().map(|s| s.to_string()).collect(),
            },
            group_policy: GroupPolicyConfig { require_mention: true },
        },
        ..Default::default()
    }
}

fn resolve_http_block(s: &MCPServerConfig, data_dir: &str) -> anyhow::Result<(Option<MCPHTTPServerConfig>, bool)> {
    let (url, auth) = if let Some(http) = &s.http {
        (http.url.clone(), http.auth.clone())
    } else if !s.url.is_empty() || !s.auth.kind.is_empty() {
        (s.url.clone(), s.auth.clone())
    } else {
        anyhow::bail!("mcp_servers[{}]: http transport requires either http block or legacy url field", s.id);
    };
    if url.is_empty() {
        anyhow::bail!("mcp_servers[{}]: http.url is required", s.id);
    }

    let mut resolved = MCPHTTPAuthConfig { kind: auth.kind.clone(), ..Default::default() };
    match auth.kind.as_str() {
        "oauth2_client_credentials" => {
            let token_url = auth.token_url.as_deref().unwrap_or("");
            let client_id = auth.client_id.as_deref().unwrap_or("");
            if token_url.is_empty() || client_id.is_empty() {
                anyhow::bail!("mcp_servers[{}]: oauth2_client_credentials requires token_url and client_id", s.id);
            }
            let secret = auth.client_secret.as_deref().filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| auth.client_secret_env.as_deref().and_then(|e| std::env::var(e).ok()))
                .unwrap_or_default();
            if secret.is_empty() {
                warn!(id = %s.id, "mcp_servers: skipping server with no resolvable client secret");
                return Ok((None, true));
            }
            resolved.token_url = token_url.to_string();
            resolved.client_id = client_id.to_string();
            resolved.client_secret = secret;
            resolved.scope = auth.scope.clone().unwrap_or_default();
        }
        "oauth2_authorization_code" => {
            let auth_url = auth.auth_url.as_deref().unwrap_or("");
            let token_url = auth.token_url.as_deref().unwrap_or("");
            let client_id = auth.client_id.as_deref().unwrap_or("");
            let redirect_uri = auth.redirect_uri.as_deref().unwrap_or("");
            if auth_url.is_empty() || token_url.is_empty() || client_id.is_empty() || redirect_uri.is_empty() {
                anyhow::bail!("mcp_servers[{}]: oauth2_authorization_code requires auth_url, token_url, client_id, redirect_uri", s.id);
            }
            let secret = auth.client_secret.as_deref().filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| auth.client_secret_env.as_deref().and_then(|e| std::env::var(e).ok()))
                .unwrap_or_default();
            let scope = auth.scope.as_deref().filter(|s| !s.is_empty()).unwrap_or("openid offline_access");
            let ddir = if data_dir.is_empty() { default_data_dir() } else { data_dir.to_string() };
            resolved.auth_url = auth_url.to_string();
            resolved.token_url = token_url.to_string();
            resolved.client_id = client_id.to_string();
            resolved.client_secret = secret;
            resolved.scope = scope.to_string();
            resolved.redirect_uri = redirect_uri.to_string();
            resolved.token_store_path = format!("{}/mcp-tokens/{}.json", ddir, s.id);
        }
        "bearer" => {
            let token = auth.token.as_deref().filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| auth.token_env.as_deref().and_then(|e| std::env::var(e).ok()))
                .unwrap_or_default();
            if token.is_empty() {
                warn!(id = %s.id, "mcp_servers: skipping bearer server with no resolvable token");
                return Ok((None, true));
            }
            resolved.bearer_token = token;
        }
        "none" | "" => { resolved.kind = "none".to_string(); }
        other => anyhow::bail!("mcp_servers[{}]: unsupported auth.kind {:?}", s.id, other),
    }
    Ok((Some(MCPHTTPServerConfig { url, auth: resolved }), false))
}

fn apply_otel_env_overrides(cfg: &mut OTelConfig) {
    if let Ok(v) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        let v = v.trim().to_string();
        if !v.is_empty() { cfg.endpoint = v; cfg.enabled = true; }
    }
    if let Ok(v) = std::env::var("OTEL_SERVICE_NAME") {
        let v = v.trim().to_string();
        if !v.is_empty() { cfg.service_name = v; }
    }
    if let Ok(v) = std::env::var("OTEL_EXPORTER_OTLP_HEADERS") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            for pair in v.split(',') {
                if let Some((k, val)) = pair.trim().split_once('=') {
                    cfg.headers.insert(k.trim().to_string(), val.trim().to_string());
                }
            }
        }
    }
    if let Ok(v) = std::env::var("OTEL_TRACES_SAMPLER_ARG") {
        if let Ok(r) = v.trim().parse::<f64>() {
            if r >= 0.0 { cfg.sample_ratio = r; }
        }
    }
    if std::env::var("OTEL_SDK_DISABLED").map(|v| v.to_lowercase() == "true").unwrap_or(false) {
        cfg.enabled = false;
    }
    if cfg.enabled && cfg.signals == (OTelSignals::default()) {
        cfg.signals = OTelSignals { traces: true, metrics: true, logs: true };
    }
    if cfg.enabled && cfg.sample_ratio == 0.0 { cfg.sample_ratio = 1.0; }
    if cfg.enabled && cfg.service_name.is_empty() { cfg.service_name = "robin".to_string(); }
}

pub fn strip_json5(s: &str) -> String {
    let mut b = String::new();
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") { continue; }
        let line = if let Some(idx) = find_inline_comment(line) {
            &line[..idx]
        } else {
            line
        };
        b.push_str(line);
        b.push('\n');
    }
    remove_trailing_commas(&b)
}

fn find_inline_comment(line: &str) -> Option<usize> {
    let mut in_str = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_str = !in_str;
        }
        if !in_str && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn remove_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ',' {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], ' ' | '\t' | '\n' | '\r') { j += 1; }
            if j < chars.len() && matches!(chars[j], '}' | ']') {
                i += 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.into_iter().collect()
}

pub fn validate_reasoning_mode(s: &str) -> anyhow::Result<()> {
    match s {
        "" | "off" | "low" | "medium" | "high" => Ok(()),
        other => anyhow::bail!("reasoning {:?} invalid (want off|low|medium|high)", other),
    }
}

// ── PermissionChecker ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DecisionBehavior { Allow, Deny }

#[derive(Debug, Clone)]
pub struct PermissionDecision { pub behavior: DecisionBehavior }

pub struct PermissionChecker {
    policies: HashMap<String, (Vec<String>, Vec<String>)>,
}

impl PermissionChecker {
    pub fn check(&self, _agent_id: &str, tool_name: &str, _input: &[u8]) -> PermissionDecision {
        match self.policies.get(_agent_id) {
            None => PermissionDecision { behavior: DecisionBehavior::Allow },
            Some((allow, deny)) => {
                if deny.iter().any(|d| d == tool_name) {
                    return PermissionDecision { behavior: DecisionBehavior::Deny };
                }
                if !allow.is_empty() && !allow.iter().any(|a| a == tool_name) {
                    return PermissionDecision { behavior: DecisionBehavior::Deny };
                }
                PermissionDecision { behavior: DecisionBehavior::Allow }
            }
        }
    }
}

impl Config {
    pub fn build_permission_checker(&self) -> PermissionChecker {
        PermissionChecker { policies: self.tool_policies() }
    }
}