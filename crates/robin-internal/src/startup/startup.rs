use std::sync::Arc;
use std::future::Future;

use tracing::{info, warn};

use crate::agent::builder::{build_runtime_for_agent, RuntimeDeps, RuntimeInputs};
use crate::config::config::{default_config_path, load as load_config, Config};
use crate::gateway::{
    logs::LogBuffer,
    memory::{MemoryEntry as GatewayMemoryEntry, MemoryHandlerState, MemoryManagerTrait},
    server::{Server, ServerOptions},
    settings::SettingsHandlerState,
    skills::{SkillHandlerState, SkillParsed, SkillParser, SkillReloader},
    websocket::{
        AgentBuilder, AgentConfig as WsAgentConfig, AgentEvent as WsAgentEvent, AgentRuntime,
        HistoryEntry, ImageData as WsImageData, SessionSummary, WebSocketHandlerState,
        ConfigSurface, SessionStoreTrait,
    },
};
use crate::llm::new_provider;
use crate::memory::Manager as MemoryManager;
use crate::skill::{embed::seed_bundled_skills, skill::Loader as SkillLoader};
use crate::session::{
    session::{EntryType, MessageData, ToolCallData, ToolResultData},
    store::Store as SessionStore,
};
use crate::tools::{
    register_send_message, BashTool, EditFileTool, ExecPolicy, Executor, LoadMemoryTool, LoadSkillTool,
    ReadFileTool, Registry as ToolRegistry, TodoWriteTool, WebFetchTool, WebSearchTool,
    WriteFileTool,
};

/// Result holds the running gateway components.
pub struct Result {
    pub config: Arc<Config>,
    pub cleanup: Box<dyn FnOnce() + Send>,
}

/// Options configures gateway startup behavior.
pub struct Options {
    pub log_file: Option<String>,
}

/// ResolveProviderOpts builds provider options for a given provider name from config.
pub fn resolve_provider_opts(name: &str, cfg: &Config) -> ProviderOptions {
    let pcfg = cfg.get_provider(name);
    ProviderOptions {
        api_key: pcfg.api_key.clone(),
        base_url: pcfg.base_url.clone(),
        kind: pcfg.kind.clone(),
        ca_bundle: pcfg.ca_bundle.clone(),
    }
}

/// ProviderOptions describes configuration for an LLM provider.
pub struct ProviderOptions {
    pub api_key: String,
    pub base_url: String,
    pub kind: String,
    pub ca_bundle: String,
}

/// PersistedJob is the on-disk shape for a cron job.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistedJob {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    pub paused: bool,
}

/// JobInfo is a public view of a scheduled cron job.
pub struct JobInfo {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    pub paused: bool,
}

/// CronSchedulerAdapter adapts a cron Scheduler to the tools.JobScheduler interface.
pub struct CronSchedulerAdapter {
    pub jobs_file: String,
    jobs: parking_lot::Mutex<Vec<PersistedJob>>,
    scheduler: Arc<crate::cron::cron::Scheduler>,
    agent_builder: parking_lot::RwLock<Option<Arc<dyn AgentBuilder>>>,
}

impl CronSchedulerAdapter {
    pub fn new(jobs_file: &str) -> Self {
        CronSchedulerAdapter {
            jobs_file: jobs_file.to_string(),
            jobs: parking_lot::Mutex::new(vec![]),
            scheduler: Arc::new(crate::cron::cron::Scheduler::new()),
            agent_builder: parking_lot::RwLock::new(None),
        }
    }

    pub fn set_agent_builder(&self, ab: Arc<dyn AgentBuilder>) {
        *self.agent_builder.write() = Some(ab);
    }

    pub fn start_scheduler(&self, cancel: tokio_util::sync::CancellationToken) {
        // sync all current jobs to the scheduler
        let jobs = self.jobs.lock().clone();
        for j in jobs {
            if !j.paused {
                if let Some(cron_job) = self.create_cron_job(&j) {
                    let _ = self.scheduler.add(cron_job);
                }
            }
        }
        self.scheduler.start(cancel);
    }

    fn create_cron_job(&self, pj: &PersistedJob) -> Option<crate::cron::cron::Job> {
        let ab_lock = self.agent_builder.read();
        let ab_opt = ab_lock.clone()?;

        let agent_fn: crate::cron::cron::AgentFunc = Arc::new(move |_cancel, prompt| {
            let ab = ab_opt.clone();
            Box::pin(async move {
                let agent = ab.build("default", "ws_default").await?;
                let mut rx = agent.run(prompt, vec![]).await?;
                let mut full = String::new();
                while let Some(ev) = rx.recv().await {
                    match ev {
                        crate::gateway::websocket::AgentEvent::TextDelta(t) => full.push_str(&t),
                        crate::gateway::websocket::AgentEvent::Error(e) => return Err(anyhow::anyhow!(e)),
                        _ => {}
                    }
                }
                Ok(full)
            })
        });

        Some(crate::cron::cron::Job {
            name: pj.name.clone(),
            schedule: pj.schedule.clone(),
            prompt: pj.prompt.clone(),
            paused: pj.paused,
            agent_fn,
            output_fn: None,
            interval: std::time::Duration::from_secs(0),
        })
    }

    pub fn add_job(&self, name: &str, schedule: &str, prompt: &str) -> anyhow::Result<()> {
        let pj = PersistedJob {
            name: name.to_string(),
            schedule: schedule.to_string(),
            prompt: prompt.to_string(),
            paused: false,
        };
        self.jobs.lock().push(pj.clone());
        if let Some(cj) = self.create_cron_job(&pj) {
            let _ = self.scheduler.add(cj);
        }
        self.persist();
        Ok(())
    }

    pub fn remove_job(&self, name: &str) -> anyhow::Result<()> {
        self.jobs.lock().retain(|j| j.name != name);
        let _ = self.scheduler.remove(name);
        self.persist();
        Ok(())
    }

    pub fn list_jobs(&self) -> Vec<JobInfo> {
        self.jobs.lock().iter().map(|j| JobInfo {
            name: j.name.clone(),
            schedule: j.schedule.clone(),
            prompt: j.prompt.clone(),
            paused: j.paused,
        }).collect()
    }

    pub fn pause_job(&self, name: &str) -> anyhow::Result<()> {
        let mut jobs = self.jobs.lock();
        if let Some(j) = jobs.iter_mut().find(|j| j.name == name) {
            j.paused = true;
            let _ = self.scheduler.remove(name); // removing from active scheduler pauses it
            drop(jobs);
            self.persist();
            Ok(())
        } else {
            anyhow::bail!("job {:?} not found", name)
        }
    }

    pub fn resume_job(&self, name: &str) -> anyhow::Result<()> {
        let mut jobs = self.jobs.lock();
        if let Some(j) = jobs.iter_mut().find(|j| j.name == name) {
            j.paused = false;
            let pj = j.clone();
            drop(jobs);
            if let Some(cj) = self.create_cron_job(&pj) {
                let _ = self.scheduler.add(cj);
            }
            self.persist();
            Ok(())
        } else {
            anyhow::bail!("job {:?} not found", name)
        }
    }

    pub fn update_job_schedule(&self, name: &str, schedule: &str) -> anyhow::Result<()> {
        let mut jobs = self.jobs.lock();
        if let Some(j) = jobs.iter_mut().find(|j| j.name == name) {
            j.schedule = schedule.to_string();
            drop(jobs);
            self.persist();
            Ok(())
        } else {
            anyhow::bail!("job {:?} not found", name)
        }
    }

    fn persist(&self) {
        if self.jobs_file.is_empty() { return; }
        let jobs = self.jobs.lock().clone();
        match serde_json::to_vec_pretty(&jobs) {
            Err(e) => warn!("cron persist marshal failed error={}", e),
            Ok(data) => {
                let tmp = format!("{}.tmp", self.jobs_file);
                if let Err(e) = std::fs::write(&tmp, &data) {
                    warn!("cron persist write failed path={} error={}", tmp, e);
                    return;
                }
                if let Err(e) = std::fs::rename(&tmp, &self.jobs_file) {
                    warn!("cron persist rename failed path={} error={}", self.jobs_file, e);
                }
            }
        }
    }

    pub fn restore(&self) -> anyhow::Result<()> {
        if self.jobs_file.is_empty() { return Ok(()); }
        let data = match std::fs::read(&self.jobs_file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
            Ok(d) => d,
        };
        let stored: Vec<PersistedJob> = serde_json::from_slice(&data)
            .map_err(|e| anyhow::anyhow!("parse {}: {}", self.jobs_file, e))?;
        if !stored.is_empty() {
            info!("cron jobs restored count={} path={}", stored.len(), self.jobs_file);
            *self.jobs.lock() = stored;
        }
        Ok(())
    }
}

impl crate::gateway::websocket::JobSchedulerTrait for CronSchedulerAdapter {
    fn list_jobs(&self) -> Vec<serde_json::Value> {
        self.jobs.lock().iter().map(|j| {
            serde_json::json!({
                "name": j.name,
                "schedule": j.schedule,
                "prompt": j.prompt,
                "paused": j.paused
            })
        }).collect()
    }

    fn pause_job(&self, name: &str) -> anyhow::Result<()> {
        self.pause_job(name)
    }

    fn resume_job(&self, name: &str) -> anyhow::Result<()> {
        self.resume_job(name)
    }

    fn remove_job(&self, name: &str) -> anyhow::Result<()> {
        self.remove_job(name)
    }

    fn add_job(&self, name: &str, schedule: &str, prompt: &str) -> anyhow::Result<()> {
        self.add_job(name, schedule, prompt)
    }

    fn update_job_schedule(&self, name: &str, schedule: &str) -> anyhow::Result<()> {
        self.update_job_schedule(name, schedule)
    }
}

impl crate::tools::JobScheduler for CronSchedulerAdapter {
    fn add_job(&self, name: &str, schedule: &str, prompt: &str) -> anyhow::Result<()> {
        self.add_job(name, schedule, prompt)
    }
    fn remove_job(&self, name: &str) -> anyhow::Result<()> {
        self.remove_job(name)
    }
    fn list_jobs(&self) -> Vec<crate::tools::JobInfo> {
        self.list_jobs().into_iter().map(|j| crate::tools::JobInfo {
            name: j.name,
            schedule: j.schedule,
            prompt: j.prompt,
            paused: j.paused,
        }).collect()
    }
    fn pause_job(&self, name: &str) -> anyhow::Result<()> {
        self.pause_job(name)
    }
    fn resume_job(&self, name: &str) -> anyhow::Result<()> {
        self.resume_job(name)
    }
    fn update_job_schedule(&self, name: &str, schedule: &str) -> anyhow::Result<()> {
        self.update_job_schedule(name, schedule)
    }
}

#[derive(Clone)]
struct ArcCronSchedulerAdapter(Arc<CronSchedulerAdapter>);

impl crate::tools::JobScheduler for ArcCronSchedulerAdapter {
    fn add_job(&self, name: &str, schedule: &str, prompt: &str) -> anyhow::Result<()> {
        self.0.add_job(name, schedule, prompt)
    }
    fn remove_job(&self, name: &str) -> anyhow::Result<()> {
        self.0.remove_job(name)
    }
    fn list_jobs(&self) -> Vec<crate::tools::JobInfo> {
        self.0.list_jobs().into_iter().map(|j| crate::tools::JobInfo {
            name: j.name,
            schedule: j.schedule,
            prompt: j.prompt,
            paused: j.paused,
        }).collect()
    }
    fn pause_job(&self, name: &str) -> anyhow::Result<()> {
        self.0.pause_job(name)
    }
    fn resume_job(&self, name: &str) -> anyhow::Result<()> {
        self.0.resume_job(name)
    }
    fn update_job_schedule(&self, name: &str, schedule: &str) -> anyhow::Result<()> {
        self.0.update_job_schedule(name, schedule)
    }
}

/// start_gateway starts the full gateway and returns the result.
/// The caller is responsible for calling Result.cleanup on shutdown.
pub fn start_gateway(config_path: &str, version: &str) -> anyhow::Result<crate::startup::startup::Result> {
    let cfg = load_config(config_path)
        .map_err(|e| anyhow::anyhow!("load config: {}", e))?;

    info!("gateway startup version={}", version);

    let host = cfg.gateway.host.clone();
    let port = cfg.gateway.port;
    let auth_token = cfg.gateway.auth.token.clone();
    let data_dir = cfg.data_dir();
    let cfg_path = if config_path.is_empty() {
        default_config_path()
    } else {
        config_path.to_string()
    };

    let cfg = Arc::new(cfg);
    let bash_policy = bash_exec_policy(&cfg);

    // Config surface for the WebSocket handler.
    let config_surface: Arc<dyn ConfigSurface> = Arc::new(ConfigAdapter(cfg.clone()));

    // Session store wrapping the on-disk JSONL store.
    let session_store_inner = SessionStore::new(format!("{}/sessions", data_dir));
    let session_store: Arc<dyn SessionStoreTrait> = Arc::new(SessionStoreAdapter(session_store_inner.clone()));

    // Skills: serve files from ~/.robin/skills
    let skills_dir = std::path::PathBuf::from(&data_dir).join("skills");
    std::fs::create_dir_all(&skills_dir).ok();
    let _ = seed_bundled_skills(skills_dir.to_string_lossy().as_ref())
        .map_err(|e| warn!("seed bundled skills failed: {}", e));

    let mut reload_dirs = vec![skills_dir.to_string_lossy().to_string()];
    for a in &cfg.agents.list {
        if a.workspace.is_empty() {
            continue;
        }
        let d = std::path::PathBuf::from(&a.workspace).join("skills");
        reload_dirs.push(d.to_string_lossy().to_string());
    }
    reload_dirs.sort();
    reload_dirs.dedup();

    let skill_loader = Arc::new(SkillLoader::new());
    let refs: Vec<&str> = reload_dirs.iter().map(|s| s.as_str()).collect();
    if let Err(e) = skill_loader.load_from(&refs) {
        warn!("initial skill load failed: {}", e);
    }
    let memory_manager = Arc::new(MemoryManager::new(
        std::path::PathBuf::from(&data_dir).join("memory"),
    ));

    let cron_adapter = Arc::new(CronSchedulerAdapter::new(&format!("{}/cron-jobs.json", data_dir)));
    let _ = cron_adapter.restore();

    let agent_builder: Arc<dyn AgentBuilder> = Arc::new(AgentBuilderImpl {
        config: cfg.clone(),
        session_store: session_store_inner,
        skill_loader: skill_loader.clone(),
        memory_manager: memory_manager.clone(),
        cron_adapter: Some(cron_adapter.clone()),
    });

    let mut ws_state_inner = WebSocketHandlerState::new(config_surface, session_store);
    ws_state_inner.set_agent_builder(agent_builder);
    ws_state_inner.set_job_scheduler(cron_adapter.clone());

    let ws_state = Arc::new(ws_state_inner);

    // Settings page — expose current config as JSON.
    let config_json = serde_json::to_value(&*cfg)
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let settings_tool_registry =
        build_tool_registry(&data_dir, &data_dir, skill_loader.clone(), bash_policy.clone(), Some(cron_adapter.clone()));
    let settings = SettingsHandlerState {
        config_json: Arc::new(parking_lot::RwLock::new(config_json)),
        config_path: cfg_path,
        tool_registry: Some(Arc::new(ToolRegistryAdapter(settings_tool_registry))),
        bootstrap: None,
        on_save: None,
    };

    let skills = SkillHandlerState {
        loader: skill_loader.clone(),
        parser: Some(Arc::new(LiveSkillParser)),
        skills_dir,
        reload_dirs,
    };

    // Log buffer: ring buffer for /logs page
    let log_buffer = LogBuffer::new(2000);

    // Initialize tracing to both stdout and the LogBuffer
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_ansi(true))
        .with(crate::gateway::logs::LogBufferLayer::new(log_buffer.clone()))
        .try_init();

    let opts = ServerOptions {
        auth_token,
        settings: Some(settings),
        skills: Some(skills),
        memory: Some(MemoryHandlerState {
            manager: if cfg.memory.enabled {
                Some(Arc::new(MemoryManagerAdapter(memory_manager.clone())) as Arc<dyn MemoryManagerTrait>)
            } else {
                None
            },
        }),
        log_buffer: Some(log_buffer),
        chat_port: Some(port),
        ..Default::default()
    };

    let mut server = Server::new(host, port, ws_state, opts);
    let shutdown_handle = server.shutdown_handle();

    tokio::spawn(async move {
        if let Err(e) = server.start().await {
            tracing::error!("gateway server stopped error={}", e);
        }
    });

    info!("gateway listening on {}:{}", cfg.gateway.host, port);

    let cleanup = Box::new(move || {
        info!("gateway cleanup");
        let _ = shutdown_handle.send(true);
    });

    Ok(crate::startup::startup::Result {
        config: cfg,
        cleanup,
    })
}

pub async fn build_in_process_runtime(
    cfg: Arc<Config>,
    agent_id: &str,
    session_key: &str,
    model_override: Option<&str>,
) -> anyhow::Result<Arc<crate::agent::runtime::Runtime>> {
    let mut agent_cfg = cfg
        .get_agent(agent_id)
        .ok_or_else(|| anyhow::anyhow!("agent {:?} not found", agent_id))?;
    if let Some(m) = model_override {
        if !m.is_empty() {
            agent_cfg.model = m.to_string();
        }
    }

    let data_dir = cfg.data_dir();
    let bash_policy = bash_exec_policy(&cfg);

    let session_store = SessionStore::new(format!("{}/sessions", data_dir));

    let skills_dir = std::path::PathBuf::from(&data_dir).join("skills");
    std::fs::create_dir_all(&skills_dir).ok();
    let _ = seed_bundled_skills(skills_dir.to_string_lossy().as_ref())
        .map_err(|e| warn!("seed bundled skills failed: {}", e));

    let mut reload_dirs = vec![skills_dir.to_string_lossy().to_string()];
    for a in &cfg.agents.list {
        if a.workspace.is_empty() {
            continue;
        }
        let d = std::path::PathBuf::from(&a.workspace).join("skills");
        reload_dirs.push(d.to_string_lossy().to_string());
    }
    reload_dirs.sort();
    reload_dirs.dedup();

    let skill_loader = Arc::new(SkillLoader::new());
    let refs: Vec<&str> = reload_dirs.iter().map(|s| s.as_str()).collect();
    if let Err(e) = skill_loader.load_from(&refs) {
        warn!("initial skill load failed: {}", e);
    }

    let memory_manager = Arc::new(MemoryManager::new(
        std::path::PathBuf::from(&data_dir).join("memory"),
    ));

    if !session_store.exists(agent_id, session_key) {
        session_store.create(agent_id, session_key)?;
    }
    let session = session_store.load(agent_id, session_key)?;

    let provider_name = agent_cfg
        .model
        .split('/')
        .next()
        .unwrap_or("anthropic");
    let pcfg = cfg.get_provider(provider_name);
    let provider: Arc<dyn crate::llm::LLMProvider> = Arc::from(new_provider(
        provider_name,
        crate::llm::ProviderOptions {
            api_key: pcfg.api_key.clone(),
            base_url: pcfg.base_url.clone(),
            kind: pcfg.kind.clone(),
            ca_bundle: pcfg.ca_bundle.clone(),
        },
    )?);

    let (memory_index, memory_manager) = if cfg.memory.enabled {
        if let Err(e) = memory_manager.load().await {
            warn!("memory load failed for prompt index: {}", e);
        }
        (memory_manager.format_index(), Some(memory_manager))
    } else {
        (String::new(), None)
    };

    let deps = RuntimeDeps {
        config: Some(cfg.clone()),
        agent_loop: cfg.agent_loop.clone(),
        ..Default::default()
    };
    let inputs = RuntimeInputs {
        provider: Some(provider),
        tools: Some(
            build_tool_registry(
                &agent_cfg.workspace,
                &cfg.data_dir(),
                skill_loader.clone(),
                bash_policy,
                None,
            ) as Arc<dyn Executor>
        ),
        session: Some(session),
        skills_index: skill_loader.format_index(),
        memory_index,
        memory_manager,
        ..Default::default()
    };

    Ok(Arc::new(build_runtime_for_agent(deps, inputs, &agent_cfg)?))
}

// ── Skill adapters ─────────────────────────────────────────────────────────

impl SkillReloader for SkillLoader {
    fn load_from(&self, dirs: &[&str]) -> anyhow::Result<()> {
        crate::skill::skill::Loader::load_from(self, dirs)
    }
}

fn bash_exec_policy(cfg: &Config) -> Option<ExecPolicy> {
    let level = cfg.security.exec_approvals.level.trim();
    let allowlist = cfg.security.exec_approvals.allowlist.clone();
    if level.is_empty() && allowlist.is_empty() {
        return None;
    }
    Some(ExecPolicy { level: level.to_string(), allowlist })
}

fn build_tool_registry(
    work_dir: &str,
    data_dir: &str,
    skill_loader: Arc<SkillLoader>,
    bash_policy: Option<ExecPolicy>,
    cron_adapter: Option<Arc<CronSchedulerAdapter>>,
) -> Arc<ToolRegistry> {
    let reg = Arc::new(ToolRegistry::new());

    if let Some(cron) = cron_adapter {
        reg.register(crate::tools::CronTool::new(Box::new(ArcCronSchedulerAdapter(cron))));
    }

    reg.register(ReadFileTool {
        work_dir: work_dir.to_string(),
    });
    reg.register(WriteFileTool {
        work_dir: work_dir.to_string(),
    });
    reg.register(EditFileTool {
        work_dir: work_dir.to_string(),
    });
    reg.register(BashTool {
        work_dir: work_dir.to_string(),
        exec_policy: bash_policy,
    });
    reg.register(WebFetchTool);
    reg.register(WebSearchTool::new());
    reg.register(TodoWriteTool::new(work_dir.to_string()));

    let skill_lookup_loader = skill_loader.clone();
    reg.register(LoadSkillTool::new(move |name| {
        skill_lookup_loader
            .skills()
            .into_iter()
            .find(|s| s.name == name)
            .map(|s| s.body)
    }));
    let memory_entries_dir = std::path::PathBuf::from(data_dir).join("memory").join("entries");
    reg.register(LoadMemoryTool::new(move |id| {
        let path = memory_entries_dir.join(format!("{}.md", id));
        std::fs::read_to_string(path).ok()
    }));

    register_send_message(&reg, None);
    reg
}

struct ToolRegistryAdapter(Arc<ToolRegistry>);

impl crate::gateway::settings::ToolRegistryTrait for ToolRegistryAdapter {
    fn names(&self) -> Vec<String> {
        self.0.names()
    }

    fn description(&self, name: &str) -> Option<String> {
        self.0.get(name).map(|t| t.description().to_string())
    }
}

struct MemoryManagerAdapter(Arc<MemoryManager>);

impl MemoryManagerAdapter {
    fn block_on<T>(&self, fut: impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| anyhow::anyhow!("create runtime: {}", e))?;
                rt.block_on(fut)
            }
        }
    }
}

impl MemoryManagerTrait for MemoryManagerAdapter {
    fn entries(&self) -> Vec<GatewayMemoryEntry> {
        let _ = self.block_on(self.0.load());
        self.0
            .entries()
            .into_iter()
            .map(|e| GatewayMemoryEntry {
                id: e.id,
                title: e.title,
                content: e.content,
                mod_time: e.mod_time.into(),
            })
            .collect()
    }

    fn get(&self, id: &str) -> Option<GatewayMemoryEntry> {
        let _ = self.block_on(self.0.load());
        self.0.get(id).map(|e| GatewayMemoryEntry {
            id: e.id,
            title: e.title,
            content: e.content,
            mod_time: e.mod_time.into(),
        })
    }

    fn save(&self, id: &str, content: &str) -> anyhow::Result<()> {
        self.block_on(self.0.save(id, content))
    }

    fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.block_on(self.0.delete(id))
    }
}

struct LiveSkillParser;

impl SkillParser for LiveSkillParser {
    fn parse(&self, path: &std::path::Path) -> std::result::Result<SkillParsed, String> {
        let p = path.to_str().ok_or_else(|| "non-utf8 path".to_string())?;
        let s = crate::skill::skill::parse_skill_file(p).map_err(|e| e.to_string())?;
        Ok(SkillParsed {
            name: s.name,
            description: s.description,
            tags: s.tags,
            required_bins: s.metadata.openclaw.requires.bins,
        })
    }

    fn split_frontmatter<'a>(&self, content: &'a str) -> (&'a str, &'a str) {
        crate::skill::skill::split_frontmatter(content)
    }

    fn missing_bins(&self, parsed: &SkillParsed) -> Vec<String> {
        parsed
            .required_bins
            .iter()
            .filter(|bin| which::which(bin).is_err())
            .cloned()
            .collect()
    }
}

// ── ConfigSurface adapter ─────────────────────────────────────────────────

struct ConfigAdapter(Arc<Config>);

impl ConfigSurface for ConfigAdapter {
    fn list_agents(&self) -> Vec<WsAgentConfig> {
        self.0.agents.list.iter().map(to_ws_agent).collect()
    }
    fn get_agent(&self, id: &str) -> Option<WsAgentConfig> {
        self.0.get_agent(id).map(|a| to_ws_agent(&a))
    }
}

fn to_ws_agent(a: &crate::config::config::AgentConfig) -> WsAgentConfig {
    WsAgentConfig {
        id: a.id.clone(),
        name: a.name.clone(),
        model: a.model.clone(),
        workspace: a.workspace.clone(),
        context_window: a.context_window.max(0) as u64,
    }
}

// ── SessionStoreTrait adapter ─────────────────────────────────────────────

struct SessionStoreAdapter(Arc<SessionStore>);

impl SessionStoreTrait for SessionStoreAdapter {
    fn list(&self, agent_id: &str) -> anyhow::Result<Vec<SessionSummary>> {
        let infos = self.0.list(agent_id)?;
        Ok(infos.iter().map(|i| SessionSummary {
            key: i.key.clone(),
            entry_count: i.entry_count,
            created_at: i.created_at.timestamp(),
            last_activity: i.last_activity.timestamp(),
        }).collect())
    }

    fn exists(&self, agent_id: &str, key: &str) -> bool {
        self.0.exists(agent_id, key)
    }

    fn create(&self, agent_id: &str, key: &str) -> anyhow::Result<()> {
        self.0.create(agent_id, key)
    }

    fn delete(&self, agent_id: &str, key: &str) -> anyhow::Result<()> {
        self.0.delete(agent_id, key)
    }

    fn history(&self, agent_id: &str, key: &str) -> anyhow::Result<Vec<HistoryEntry>> {
        let sess = self.0.load(agent_id, key)?;
        let mut result = Vec::new();
        for e in sess.entries() {
            match e.entry_type {
                EntryType::Message => {
                    if let Ok(md) = serde_json::from_str::<MessageData>(e.data.get()) {
                        result.push(HistoryEntry::Message {
                            role: e.role.clone(),
                            text: md.text,
                            images: md.images.iter().map(|img| WsImageData {
                                mime_type: img.mime_type.clone(),
                                data: img.data.clone(),
                            }).collect(),
                        });
                    }
                }
                EntryType::ToolCall => {
                    if let Ok(tc) = serde_json::from_str::<ToolCallData>(e.data.get()) {
                        let input = serde_json::from_str(tc.input.get())
                            .unwrap_or(serde_json::Value::Null);
                        result.push(HistoryEntry::ToolCall {
                            tool: tc.tool,
                            id: tc.id,
                            input,
                        });
                    }
                }
                EntryType::ToolResult => {
                    if let Ok(tr) = serde_json::from_str::<ToolResultData>(e.data.get()) {
                        result.push(HistoryEntry::ToolResult {
                            tool_call_id: tr.tool_call_id,
                            output: tr.output,
                            error: tr.error,
                            images: tr.images.iter().map(|img| WsImageData {
                                mime_type: img.mime_type.clone(),
                                data: img.data.clone(),
                            }).collect(),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }
}

// ── AgentBuilder adapter ──────────────────────────────────────────────────

struct AgentBuilderImpl {
    config: Arc<Config>,
    session_store: Arc<SessionStore>,
    skill_loader: Arc<SkillLoader>,
    memory_manager: Arc<MemoryManager>,
    cron_adapter: Option<Arc<CronSchedulerAdapter>>,
}

impl AgentBuilder for AgentBuilderImpl {
    fn build(
        &self,
        agent_id: &str,
        session_key: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Arc<dyn AgentRuntime>>> + Send + '_>> {
        let config = self.config.clone();
        let store = self.session_store.clone();
        let skill_loader = self.skill_loader.clone();
        let memory_manager = self.memory_manager.clone();
        let cron_adapter = self.cron_adapter.clone();
        let agent_id = agent_id.to_string();
        let session_key = session_key.to_string();

        Box::pin(async move {
            let agent_cfg = config
                .get_agent(&agent_id)
                .ok_or_else(|| anyhow::anyhow!("agent {:?} not found", agent_id))?;

            // Build LLM provider from the agent's model config.
            let provider_name = agent_cfg.model
                .split('/')
                .next()
                .unwrap_or("anthropic");
            let pcfg = config.get_provider(provider_name);
            let provider: Arc<dyn crate::llm::LLMProvider> = Arc::from(new_provider(provider_name, crate::llm::ProviderOptions {
                api_key: pcfg.api_key.clone(),
                base_url: pcfg.base_url.clone(),
                kind: pcfg.kind.clone(),
                ca_bundle: pcfg.ca_bundle.clone(),
            })?);

            // Load or create the session.
            if !store.exists(&agent_id, &session_key) {
                store.create(&agent_id, &session_key)?;
            }
            let session = store.load(&agent_id, &session_key)?;

            let deps = RuntimeDeps {
                config: Some(config.clone()),
                agent_loop: config.agent_loop.clone(),
                ..Default::default()
            };
            let bash_policy = bash_exec_policy(&config);
            let inputs = RuntimeInputs {
                provider: Some(provider),
                tools: Some(
                    build_tool_registry(
                        &agent_cfg.workspace,
                        &config.data_dir(),
                        skill_loader.clone(),
                        bash_policy,
                        cron_adapter.clone(),
                    ) as Arc<dyn Executor>
                ),
                session: Some(session),
                skills_index: skill_loader.format_index(),
                memory_index: {
                    if let Err(e) = memory_manager.load().await {
                        warn!("memory load failed for prompt index: {}", e);
                    }
                    memory_manager.format_index()
                },
                memory_manager: Some(memory_manager),
                ..Default::default()
            };

            let runtime = Arc::new(build_runtime_for_agent(deps, inputs, &agent_cfg)?);
            Ok(Arc::new(RuntimeAdapter(runtime)) as Arc<dyn AgentRuntime>)
        })
    }
}

// ── RuntimeAdapter — maps agent::runtime events to gateway WS events ─────

struct RuntimeAdapter(Arc<crate::agent::runtime::Runtime>);

impl AgentRuntime for RuntimeAdapter {
    fn run(
        self: Arc<Self>,
        text: String,
        images: Vec<WsImageData>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<tokio::sync::mpsc::Receiver<WsAgentEvent>>> + Send>> {
        Box::pin(async move {
            let ctx = tokio_util::sync::CancellationToken::new();
            let mut decoded = Vec::with_capacity(images.len());
            for img in images {
                use base64::Engine;
                let data = base64::engine::general_purpose::STANDARD
                    .decode(img.data.as_bytes())
                    .map_err(|e| anyhow::anyhow!("decode image base64 failed: {}", e))?;
                decoded.push(crate::llm::ImageContent {
                    mime_type: img.mime_type,
                    data,
                });
            }
            let mut rx = self.0.run(ctx, text, decoded).await?;

            let (ws_tx, ws_rx) = tokio::sync::mpsc::channel(100);
            tokio::spawn(async move {
                use crate::agent::runtime::AgentEventType;
                while let Some(ev) = rx.recv().await {
                    let ws_ev = match ev.event_type {
                        AgentEventType::TextDelta => WsAgentEvent::TextDelta(ev.text),
                        AgentEventType::ToolCallStart => {
                            if let Some(tc) = ev.tool_call {
                                WsAgentEvent::ToolCallStart {
                                    tool: tc.name,
                                    id: tc.id,
                                    input: tc.input,
                                }
                            } else { continue; }
                        }
                        AgentEventType::ToolResult => {
                            let tc = ev.tool_call.unwrap_or_default();
                            let res = ev.result.unwrap_or_default();
                            let images = res.images.iter().map(|img| {
                                use base64::Engine;
                                WsImageData {
                                    mime_type: img.mime_type.clone(),
                                    data: base64::engine::general_purpose::STANDARD.encode(&img.data),
                                }
                            }).collect();
                            WsAgentEvent::ToolResult {
                                tool: tc.name,
                                id: tc.id,
                                input: tc.input,
                                output: res.output,
                                error: res.error,
                                images,
                                auth_required: None,
                            }
                        }
                        AgentEventType::Done => {
                            let u = ev.usage.unwrap_or_default();
                            WsAgentEvent::Done {
                                input_tokens: u.input_tokens.max(0) as u64,
                                output_tokens: u.output_tokens.max(0) as u64,
                                cache_creation_input_tokens: u.cache_creation_input_tokens.max(0) as u64,
                                cache_read_input_tokens: u.cache_read_input_tokens.max(0) as u64,
                                context_window: 0,
                                model: String::new(),
                            }
                        }
                        AgentEventType::Error => {
                            let msg = ev.error.map(|e| e.to_string()).unwrap_or_default();
                            WsAgentEvent::Error(msg)
                        }
                        AgentEventType::Aborted => WsAgentEvent::Aborted,
                        _ => continue,
                    };
                    if ws_tx.send(ws_ev).await.is_err() { break; }
                }
            });

            Ok(ws_rx)
        })
    }
}
