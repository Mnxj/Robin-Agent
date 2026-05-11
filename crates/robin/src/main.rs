mod mcp_cmd;
mod mcp_cmd_test;
mod repl_ws;
mod repl_ws_test;

use clap::{Arg, Command};
use std::io::{BufRead as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process;
use std::time::Duration;

use robin_internal::config::config::{
    default_config, default_config_path, default_data_dir, load as load_config, ProviderConfig,
};
use robin_internal::llm::provider::{new_provider, parse_provider_model, ProviderOptions};
use robin_internal::session::store::Store as SessionStore;
use robin_internal::startup::startup::{resolve_provider_opts, start_gateway};

static VERSION: &str = env!("CARGO_PKG_VERSION");
const COMMIT: &str = match option_env!("GIT_COMMIT") {
    Some(s) => s,
    None => "none",
};

fn main() {
    let matches = cli().get_matches();
    // Build a tokio runtime so async commands (run_status, run_start,
    // run_chat) can be driven from the sync dispatch function.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    if let Err(e) = rt.block_on(async { dispatch(matches).await }) {
        eprintln!("Error: {e:#}");
        process::exit(1);
    }
}

fn cli() -> Command {
    Command::new("robin")
        .about("Robin — self-hosted AI agent gateway")
        .subcommand(start_cmd())
        .subcommand(chat_cmd())
        .subcommand(clear_cmd())
        .subcommand(sessions_cmd())
        .subcommand(status_cmd())
        .subcommand(version_cmd())
        .subcommand(onboard_cmd())
        .subcommand(doctor_cmd())
        .subcommand(mcp_cmd::mcp_cmd())
}

async fn dispatch(matches: clap::ArgMatches) -> anyhow::Result<()> {
    match matches.subcommand() {
        Some(("start", sub)) => {
            run_start(sub.get_one::<String>("config").map(|s| s.as_str()).unwrap_or("")).await
        }
        Some(("chat", sub)) => {
            let agent_id = sub.get_one::<String>("agent").cloned().unwrap_or_default();
            let config = sub.get_one::<String>("config").cloned().unwrap_or_default();
            let model = sub.get_one::<String>("model").cloned().unwrap_or_default();
            let no_gw = sub.get_flag("no-gateway");
            run_chat(agent_id, config, model, no_gw).await
        }
        Some(("clear", sub)) => {
            let agent_id = sub
                .get_one::<String>("agent")
                .map(|s| s.as_str())
                .unwrap_or("default");
            run_clear(agent_id)
        }
        Some(("sessions", sub)) => {
            let agent_id = sub
                .get_one::<String>("agent")
                .map(|s| s.as_str())
                .unwrap_or("default");
            run_sessions(agent_id)
        }
        Some(("status", _)) => run_status().await,
        Some(("version", _)) => {
            println!("robin {} (commit: {})", VERSION, COMMIT);
            Ok(())
        }
        Some(("onboard", _)) => run_onboard(),
        Some(("doctor", sub)) => {
            run_doctor(sub.get_one::<String>("config").map(|s| s.as_str()).unwrap_or(""))
        }
        Some(("mcp", sub)) => mcp_cmd::handle_mcp(sub),
        _ => {
            cli().print_help()?;
            Ok(())
        }
    }
}

// ── Subcommand builders ────────────────────────────────────────────────────────

fn start_cmd() -> Command {
    Command::new("start")
        .about("Start the Robin gateway server")
        .arg(Arg::new("config").short('c').long("config").value_name("FILE"))
}

fn chat_cmd() -> Command {
    Command::new("chat")
        .about("Start an interactive chat session")
        .arg(Arg::new("agent").help("Agent ID to chat with"))
        .arg(Arg::new("config").short('c').long("config").value_name("FILE"))
        .arg(Arg::new("model").short('m').long("model").value_name("MODEL"))
        .arg(
            Arg::new("no-gateway")
                .long("no-gateway")
                .action(clap::ArgAction::SetTrue),
        )
}

fn clear_cmd() -> Command {
    Command::new("clear")
        .about("Clear the chat session history")
        .arg(Arg::new("agent").help("Agent ID"))
}

fn sessions_cmd() -> Command {
    Command::new("sessions")
        .about("List all sessions for an agent")
        .arg(Arg::new("agent").help("Agent ID"))
}

fn status_cmd() -> Command {
    Command::new("status").about("Show gateway and agent status")
}
fn version_cmd() -> Command {
    Command::new("version").about("Print version information")
}
fn onboard_cmd() -> Command {
    Command::new("onboard").about("Interactive setup wizard for Robin")
}

fn doctor_cmd() -> Command {
    Command::new("doctor")
        .about("Run diagnostic checks on your Robin setup")
        .arg(Arg::new("config").short('c').long("config").value_name("FILE"))
}

// ── run_start ─────────────────────────────────────────────────────────────────

async fn run_start(config_path: &str) -> anyhow::Result<()> {
    let result = start_gateway(config_path, VERSION)?;

    // Wait for Ctrl-C / SIGTERM, then call cleanup.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down gateway...");
    (result.cleanup)();
    Ok(())
}

// ── run_chat ──────────────────────────────────────────────────────────────────

async fn run_chat(
    mut agent_id: String,
    config_path: String,
    model_override: String,
    no_gateway: bool,
) -> anyhow::Result<()> {
    let cfg = load_config(&config_path).map_err(|e| anyhow::anyhow!("load config: {}", e))?;

    // Resolve agent ID: prefer "default", else first agent in list.
    if agent_id.is_empty() {
        if cfg.get_agent("default").is_some() {
            agent_id = "default".to_string();
        } else if let Some(first) = cfg.agents.list.first() {
            agent_id = first.id.clone();
        } else {
            anyhow::bail!("no agents configured in {}", config_path);
        }
    }

    let agent_cfg = cfg
        .get_agent(&agent_id)
        .ok_or_else(|| anyhow::anyhow!("agent {:?} not found in config", agent_id))?;

    // Gateway-mode probe: if a gateway is running, hand off to repl_ws.
    if !no_gateway && model_override.is_empty() {
        let base_url =
            repl_ws::gateway_base_url(&cfg.gateway.host, cfg.gateway.port);
        let auth_token = cfg.gateway.auth.token.clone();
        if repl_ws::probe_gateway(&base_url, &auth_token, Duration::from_millis(250)) {
            return repl_ws::run_chat_via_gateway(
                &agent_id,
                &agent_cfg.model,
                &base_url,
                &auth_token,
            )
            .await;
        }
    } else if !no_gateway && !model_override.is_empty() {
        // Gateway is running but -m forces in-process mode.
        let base_url =
            repl_ws::gateway_base_url(&cfg.gateway.host, cfg.gateway.port);
        if repl_ws::probe_gateway(&base_url, &cfg.gateway.auth.token, Duration::from_millis(250)) {
            eprintln!(
                "Gateway is running but -m forces in-process mode \
                 (gateway has no per-call model override). \
                 Stop the gateway or omit -m to share state."
            );
        }
    }

    // ── In-process REPL path ──────────────────────────────────────────────────
    //
    // The full in-process REPL requires the agent runtime, skills, memory,
    // cortex, tools, MCP, cron, and compaction — all of which are stubs with
    // todo!() in the current robin-internal codebase. The provider resolution
    // and session-store wiring below are fully implemented; the runtime
    // construction is deferred pending those stubs.

    let model_str = if model_override.is_empty() {
        agent_cfg.model.clone()
    } else {
        model_override.clone()
    };

    let (provider_name_from_model, _model_name) = parse_provider_model(&model_str);
    let provider_name = if !provider_name_from_model.is_empty() {
        provider_name_from_model.to_string()
    } else {
        let (pn, _) = parse_provider_model(&agent_cfg.model);
        if !pn.is_empty() {
            pn.to_string()
        } else {
            "anthropic".to_string()
        }
    };

    let opts = resolve_provider_opts(&provider_name, &cfg);
    if opts.api_key.is_empty() && opts.kind != "openai-compatible" {
        anyhow::bail!(
            "no API key set for provider {:?} (set {}_API_KEY or {}_AUTH_TOKEN env var)",
            provider_name,
            provider_name.to_uppercase(),
            provider_name.to_uppercase()
        );
    }

    let _provider = new_provider(
        &provider_name,
        ProviderOptions {
            api_key: opts.api_key,
            base_url: opts.base_url,
            kind: opts.kind,
            ca_bundle: opts.ca_bundle,
        },
    )
    .map_err(|e| anyhow::anyhow!("create LLM provider: {}", e))?;

    // Session store
    let data_dir = default_data_dir();
    let sessions_dir = Path::new(&data_dir).join("sessions");
    std::fs::create_dir_all(&sessions_dir).ok();
    let session_store = SessionStore::new(&sessions_dir);
    let _sess = session_store
        .load(&agent_id, "cli_local")
        .map_err(|e| anyhow::anyhow!("load session: {}", e))?;

    // In-process REPL: wire to agent runtime once robin-internal stubs are
    // filled in (agent::build_runtime_for_agent, skills, memory, cortex,
    // tools, mcp, cron, compaction).
    eprintln!(
        "In-process REPL for agent {:?} (model: {}) — agent runtime stubs pending.",
        agent_id, model_str
    );
    eprintln!(
        "Start 'robin start' and re-run 'robin chat' to use the gateway path instead."
    );

    // Interactive REPL loop (gateway path already handles slash commands;
    // in-process path will be wired once the runtime stubs are complete).
    println!(
        "Robin chat — agent {:?} (model: {})",
        agent_id, model_str
    );
    println!("Type /quit to exit.");
    println!();

    let stdin = std::io::stdin();
    loop {
        print!("> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => {
                println!("Goodbye!");
                return Ok(());
            }
            Ok(_) => {}
        }
        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        if input == "/quit" || input == "/exit" {
            println!("Goodbye!");
            return Ok(());
        }
        eprintln!(
            "(In-process agent runtime not yet wired — \
             start the gateway with 'robin start' and reconnect.)"
        );
    }
}

// ── run_clear ─────────────────────────────────────────────────────────────────

fn run_clear(agent_id: &str) -> anyhow::Result<()> {
    let data_dir = default_data_dir();
    let store = SessionStore::new(Path::new(&data_dir).join("sessions"));
    store
        .delete(agent_id, "cli_local")
        .map_err(|e| anyhow::anyhow!("clear session: {}", e))?;
    println!("Session cleared for agent {:?}.", agent_id);
    Ok(())
}

// ── run_sessions ──────────────────────────────────────────────────────────────

fn run_sessions(agent_id: &str) -> anyhow::Result<()> {
    let data_dir = default_data_dir();
    let store = SessionStore::new(Path::new(&data_dir).join("sessions"));
    let sessions = store
        .list(agent_id)
        .map_err(|e| anyhow::anyhow!("list sessions: {}", e))?;

    if sessions.is_empty() {
        println!("No sessions found for agent {:?}.", agent_id);
        return Ok(());
    }

    println!("Sessions for agent {:?}:\n", agent_id);
    println!(
        "  {:<20}  {:>6}  {:<20}  {:<20}",
        "KEY", "ENTRIES", "CREATED", "LAST ACTIVITY"
    );
    println!(
        "  {:<20}  {:>6}  {:<20}  {:<20}",
        "---", "------", "-------", "-------------"
    );

    for s in sessions {
        let created = if s.created_at.timestamp() == 0 {
            "-".to_string()
        } else {
            s.created_at.format("%Y-%m-%d %H:%M:%S").to_string()
        };
        let last_act = if s.last_activity.timestamp() == 0 {
            "-".to_string()
        } else {
            s.last_activity.format("%Y-%m-%d %H:%M:%S").to_string()
        };
        println!(
            "  {:<20}  {:>6}  {:<20}  {:<20}",
            s.key, s.entry_count, created, last_act
        );
    }
    Ok(())
}

// ── run_status ────────────────────────────────────────────────────────────────

async fn run_status() -> anyhow::Result<()> {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let ws_url = "ws://127.0.0.1:18789/ws";
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .map_err(|e| anyhow::anyhow!("cannot connect to gateway (is it running?): {}", e))?;

    // Send agent.status JSON-RPC request.
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "agent.status",
        "id": 1
    });
    ws.send(WsMessage::Text(req.to_string()))
        .await
        .map_err(|e| anyhow::anyhow!("write request: {}", e))?;

    // Read one response message.
    let msg = ws
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("read response: connection closed"))
        .and_then(|m| m.map_err(|e| anyhow::anyhow!("read response: {}", e)))?;

    let text = msg.to_text().map_err(|e| anyhow::anyhow!("decode response: {}", e))?;
    let resp: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| anyhow::anyhow!("parse response: {}", e))?;

    let result = &resp["result"];
    let out = serde_json::to_string_pretty(result)
        .unwrap_or_else(|_| result.to_string());
    println!("Gateway status:");
    println!("{}", out);

    ws.close(None).await.ok();
    Ok(())
}

// ── run_onboard ───────────────────────────────────────────────────────────────

fn run_onboard() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin.lock());

    // Helper: print prompt, read trimmed line; return default_val if empty.
    let prompt_line =
        |reader: &mut std::io::BufReader<std::io::StdinLock<'_>>,
         question: &str,
         default_val: &str|
         -> String {
            if !default_val.is_empty() {
                print!("{} [{}]: ", question, default_val);
            } else {
                print!("{}: ", question);
            }
            std::io::stdout().flush().ok();
            let mut answer = String::new();
            let _ = reader.read_line(&mut answer);
            let answer = answer.trim().to_string();
            if answer.is_empty() {
                default_val.to_string()
            } else {
                answer
            }
        };

    let prompt_secret = |reader: &mut std::io::BufReader<std::io::StdinLock<'_>>,
                         question: &str|
     -> String {
        print!("{}: ", question);
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        let _ = reader.read_line(&mut answer);
        answer.trim().to_string()
    };

    // Welcome banner
    println!();
    println!("Welcome to Robin!");
    println!("==================");
    println!();
    println!("Robin is a self-hosted AI agent gateway that connects you");
    println!("(via CLI or web chat) to LLMs like Claude, GPT, and more.");
    println!();
    println!("This wizard will help you set up your configuration.");
    println!();

    let mut cfg = default_config();

    // Step 1: LLM Provider
    println!("Which LLM provider do you want to use?");
    let options = [
        "Anthropic (Claude)",
        "OpenAI (GPT)",
        "Custom/LiteLLM (OpenAI-compatible endpoint)",
    ];
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == 0 { "* " } else { "  " };
        println!("  {}{}. {}", marker, i + 1, opt);
    }

    let provider_idx: usize = loop {
        let choice = prompt_line(&mut reader, "Choose", "1");
        match choice.trim().parse::<usize>() {
            Ok(n) if n >= 1 && n <= options.len() => break n - 1,
            _ => println!("Invalid choice, try again."),
        }
    };

    let provider_name: String;
    let provider_kind: &str;
    let mut base_url = String::new();

    match provider_idx {
        0 => {
            provider_name = "anthropic".to_string();
            provider_kind = "anthropic";
        }
        1 => {
            provider_name = "openai".to_string();
            provider_kind = "openai";
        }
        2 => {
            provider_name = prompt_line(&mut reader, "Provider name", "litellm");
            provider_kind = "openai-compatible";
            base_url = prompt_line(&mut reader, "Base URL", "http://localhost:4000/v1");
        }
        _ => unreachable!(),
    }

    // Step 2: API Key
    let api_key = {
        let key = prompt_secret(
            &mut reader,
            &format!("Enter your {} API key", provider_name),
        );
        if key.is_empty() {
            println!(
                "Warning: No API key provided. You can set it later via \
                 environment variable or config file."
            );
        }
        key
    };

    // Step 3: Model selection
    println!();
    let model_str = match provider_idx {
        0 => {
            println!("Which Claude model?");
            let claude_opts = [
                ("claude-sonnet-4-5-20250514 (recommended)", "anthropic/claude-sonnet-4-5-20250514"),
                ("claude-opus-4-0-20250514", "anthropic/claude-opus-4-0-20250514"),
                ("claude-haiku-3-5-20241022", "anthropic/claude-haiku-3-5-20241022"),
            ];
            for (i, (label, _)) in claude_opts.iter().enumerate() {
                let marker = if i == 0 { "* " } else { "  " };
                println!("  {}{}. {}", marker, i + 1, label);
            }
            let idx: usize = loop {
                let choice = prompt_line(&mut reader, "Choose", "1");
                match choice.trim().parse::<usize>() {
                    Ok(n) if n >= 1 && n <= claude_opts.len() => break n - 1,
                    _ => println!("Invalid choice, try again."),
                }
            };
            claude_opts[idx].1.to_string()
        }
        1 => {
            println!("Which GPT model?");
            let gpt_opts = [
                ("gpt-4o (recommended)", "openai/gpt-4o"),
                ("gpt-4o-mini", "openai/gpt-4o-mini"),
                ("gpt-4-turbo", "openai/gpt-4-turbo"),
            ];
            for (i, (label, _)) in gpt_opts.iter().enumerate() {
                let marker = if i == 0 { "* " } else { "  " };
                println!("  {}{}. {}", marker, i + 1, label);
            }
            let idx: usize = loop {
                let choice = prompt_line(&mut reader, "Choose", "1");
                match choice.trim().parse::<usize>() {
                    Ok(n) if n >= 1 && n <= gpt_opts.len() => break n - 1,
                    _ => println!("Invalid choice, try again."),
                }
            };
            gpt_opts[idx].1.to_string()
        }
        _ => prompt_line(
            &mut reader,
            "Model name (provider/model format)",
            &format!("{}/default", provider_name),
        ),
    };

    // Update config
    cfg.providers.insert(
        provider_name.clone(),
        ProviderConfig {
            kind: provider_kind.to_string(),
            base_url: base_url.clone(),
            api_key: api_key.clone(),
            ca_bundle: String::new(),
        },
    );
    if let Some(agent) = cfg.agents.list.first_mut() {
        agent.model = model_str;
    }

    finish_onboard(&mut reader, cfg)
}

fn finish_onboard(
    reader: &mut std::io::BufReader<std::io::StdinLock<'_>>,
    cfg: robin_internal::config::config::Config,
) -> anyhow::Result<()> {
    use std::io::BufRead as _;

    let prompt_line = |reader: &mut std::io::BufReader<std::io::StdinLock<'_>>,
                       question: &str,
                       default_val: &str|
     -> String {
        if !default_val.is_empty() {
            print!("{} [{}]: ", question, default_val);
        } else {
            print!("{}: ", question);
        }
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        let _ = reader.read_line(&mut answer);
        let answer = answer.trim().to_string();
        if answer.is_empty() {
            default_val.to_string()
        } else {
            answer
        }
    };

    println!();
    let data_dir = default_data_dir();
    let config_path = default_config_path();

    std::fs::create_dir_all(&data_dir).ok();

    // Check if config already exists.
    if Path::new(&config_path).exists() {
        let overwrite = prompt_line(reader, "Config file already exists. Overwrite? (y/n)", "n");
        if overwrite.to_lowercase() != "y" {
            println!("Setup cancelled. Existing config preserved.");
            return Ok(());
        }
    }

    // Write config as pretty JSON.
    let config_json = serde_json::to_vec_pretty(&cfg)
        .map_err(|e| anyhow::anyhow!("marshal config: {}", e))?;
    std::fs::write(&config_path, &config_json)
        .map_err(|e| anyhow::anyhow!("write config: {}", e))?;
    println!("Config written to {}", config_path);

    // Create agent workspace.
    if let Some(agent) = cfg.agents.list.first() {
        let workspace = &agent.workspace;
        std::fs::create_dir_all(workspace).ok();

        let identity_path = Path::new(workspace).join("IDENTITY.md");
        if !identity_path.exists() {
            let identity = "You are Robin, an AI agent. You can read files, write files, \
                edit files, execute bash commands on the user's machine, fetch web pages, \
                and search the web. Conduct yourself professionally and politely. Be concise \
                and direct. When executing tasks, think step by step and use your tools to \
                accomplish the user's goals.";
            std::fs::write(&identity_path, identity).ok();
            println!("Created workspace at {}", workspace);
        }
    }

    println!();
    println!("Setup complete! Next steps:");
    println!();
    println!("  robin start   — Start the gateway server");
    println!("  robin chat    — Start an interactive chat session");
    println!();

    Ok(())
}

// ── run_doctor ────────────────────────────────────────────────────────────────

fn run_doctor(config_path: &str) -> anyhow::Result<()> {
    let mut pass = 0usize;
    let mut warn = 0usize;
    let mut fail = 0usize;

    // Returns: Ok("") = pass, Ok("msg") = warn, Err = fail
    let mut check = |name: &str, result: Result<String, anyhow::Error>| {
        match result {
            Err(e) => {
                println!("  FAIL  {}: {}", name, e);
                fail += 1;
            }
            Ok(msg) if !msg.is_empty() => {
                println!("  WARN  {}: {}", name, msg);
                warn += 1;
            }
            Ok(_) => {
                println!("  OK    {}", name);
                pass += 1;
            }
        }
    };

    println!("Robin Doctor");
    println!("=============");
    println!();

    // Check 1: Config file
    println!("Configuration:");
    let cfg_result = load_config(config_path);
    check("Config file", cfg_result.as_ref().map(|cfg| {
        if cfg.path().is_empty() {
            "using defaults (no config file found)".to_string()
        } else if !Path::new(cfg.path()).exists() {
            "using defaults (no config file found)".to_string()
        } else {
            String::new()
        }
    }).map_err(|e| anyhow::anyhow!("{}", e)));

    let cfg = match cfg_result {
        Ok(c) => c,
        Err(_) => {
            println!("\nCannot continue without a valid config.");
            return Ok(());
        }
    };

    // Check 2: Data directories
    println!("\nData directories:");
    let data_dir = default_data_dir();
    for sub in &["", "sessions", "memory", "skills"] {
        let dir = if sub.is_empty() {
            data_dir.clone()
        } else {
            format!("{}/{}", data_dir, sub)
        };
        let dir_clone = dir.clone();
        check(&dir, {
            match std::fs::metadata(&dir_clone) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok("directory does not exist (will be created on start)".to_string())
                }
                Err(e) => Err(anyhow::anyhow!("{}", e)),
                Ok(m) if !m.is_dir() => {
                    Err(anyhow::anyhow!("path exists but is not a directory"))
                }
                Ok(_) => Ok(String::new()),
            }
        });
    }

    // Check 3: Agent workspaces
    println!("\nAgent workspaces:");
    for agent in &cfg.agents.list {
        let workspace = agent.workspace.clone();
        let agent_id = agent.id.clone();
        let label = format!("Agent {:?} workspace ({})", agent_id, workspace);
        check(&label, {
            match std::fs::metadata(&workspace) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok("workspace does not exist (will be created on start)".to_string())
                }
                Err(e) => Err(anyhow::anyhow!("{}", e)),
                Ok(_) => {
                    let identity = Path::new(&workspace).join("IDENTITY.md");
                    if !identity.exists() {
                        Ok("no IDENTITY.md found (default identity will be used)".to_string())
                    } else {
                        Ok(String::new())
                    }
                }
            }
        });
    }

    // Check 4: LLM providers
    println!("\nLLM providers:");
    for agent in &cfg.agents.list {
        let model_str = agent.model.clone();
        let agent_id = agent.id.clone();
        let label = format!("Provider for agent {:?} ({})", agent_id, model_str);
        check(&label, {
            let (prov_name, _) = parse_provider_model(&model_str);
            if prov_name.is_empty() {
                Err(anyhow::anyhow!("no provider prefix in model name"))
            } else {
                let opts = resolve_provider_opts(prov_name, &cfg);
                if opts.api_key.is_empty() && opts.kind != "openai-compatible" {
                    Err(anyhow::anyhow!(
                        "no API key configured (set {}_API_KEY env var or add to config)",
                        prov_name.to_uppercase()
                    ))
                } else {
                    new_provider(
                        prov_name,
                        ProviderOptions {
                            api_key: opts.api_key,
                            base_url: opts.base_url,
                            kind: opts.kind,
                            ca_bundle: opts.ca_bundle,
                        },
                    )
                    .map(|_| String::new())
                    .map_err(|e| anyhow::anyhow!("failed to create provider: {}", e))
                }
            }
        });
    }

    // Check 5: Gateway port availability
    println!("\nGateway:");
    let gw_port = cfg.gateway.port;
    let gw_host = cfg.gateway.host.clone();
    let port_label = format!("Port {}", gw_port);
    check(&port_label, {
        let addr = format!("{}:{}", gw_host, gw_port);
        match TcpStream::connect_timeout(
            &addr.parse().map_err(|e| anyhow::anyhow!("{}", e))?,
            Duration::from_secs(2),
        ) {
            Ok(_) => Ok("port is in use (gateway may already be running)".to_string()),
            Err(_) => Ok(String::new()),
        }
    });

    check("Auth token", {
        if cfg.gateway.auth.token.is_empty() {
            Ok("no auth token configured (API is unprotected)".to_string())
        } else {
            Ok(String::new())
        }
    });

    // Summary
    println!();
    println!("Results: {} passed, {} warnings, {} failed", pass, warn, fail);
    if fail > 0 {
        println!("\nFix the failures above before running 'robin start'.");
    } else if warn > 0 {
        println!("\nSetup looks good with minor warnings.");
    } else {
        println!("\nAll checks passed!");
    }

    Ok(())
}

// ── Formatting helpers (kept as-is) ───────────────────────────────────────────

const MAX_TOOL_OUTPUT_DISPLAY: usize = 1000;

pub fn format_tool_call_header(name: &str, input: &serde_json::Value) -> String {
    let get = |key: &str| {
        input
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    match name {
        "bash" => {
            let cmd = get("command");
            if cmd.is_empty() {
                String::new()
            } else {
                format!("$ {cmd}")
            }
        }
        "read_file" => get("path"),
        "write_file" => get("path"),
        "edit_file" => get("path"),
        "web_fetch" => get("url"),
        "web_search" => {
            let q = get("query");
            if q.is_empty() {
                String::new()
            } else {
                format!("{q:?}")
            }
        }
        _ => String::new(),
    }
}

pub fn format_tool_output(output: &str) -> &str {
    if output.len() <= MAX_TOOL_OUTPUT_DISPLAY {
        return output;
    }
    let truncated = &output[..MAX_TOOL_OUTPUT_DISPLAY];
    if let Some(idx) = truncated.rfind('\n') {
        if idx > MAX_TOOL_OUTPUT_DISPLAY / 2 {
            return &output[..idx];
        }
    }
    truncated
}

pub fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}