/// context.rs — Message assembly, system prompt construction, tool-result
/// pruning, and post-compact restore.
///
/// Mirrors Go's context.go faithfully.
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::Value;

use crate::llm::{ImageContent, Message, SystemPromptPart, ToolCall};
use crate::session::session::{
    CompactionData, EntryType, MessageData, SessionEntry, ToolCallData, ToolResultData,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Truncate tool results longer than this.
pub const MAX_TOOL_RESULT_LEN: usize = 4000;

/// Maximum total bytes of ROBIN.md / AGENTS.md injected into the static prompt.
pub const MAX_AGENT_MEMORY_BYTES: usize = 40 * 1024;

pub const DEFAULT_IDENTITY_BASE: &str = "You are Robin, an AI agent. Conduct yourself professionally and politely. Be concise and direct. When executing tasks, think step by step and use your tools to accomplish the user's goals. When you need to call multiple independent tools to gather information, emit them in a single response (parallel tool calls) rather than waiting for each one — this cuts response latency on local models.";

pub const TRUNCATION_MARKER: &str = "[truncated — ";
pub const SPILL_MARKER: &str = "[spilled — ";

/// Tool-name → usage hint injected into the default identity.
fn tool_hints() -> &'static [(&'static str, &'static str)] {
    &[
        ("read_file", "You can read files. You have vision capabilities — you can see and analyze images by using read_file on image files. Do not say you cannot see or analyze images."),
        ("write_file", "You can create or overwrite files."),
        ("edit_file", "You can make targeted edits to existing files."),
        ("bash", r#"You can execute bash commands on the user's machine. ALWAYS wrap file paths in double quotes when invoking bash (e.g. ls "/path/with spaces/file.txt") so paths with spaces or special characters survive shell tokenization."#),
        ("web_fetch", "You can fetch web pages using the web_fetch tool."),
        ("web_search", "You can search the web using the web_search tool."),
        ("browser", "You can automate a headless browser for interactive pages using the browser tool."),
        ("send_message", "You can send messages to other users or channels using the send_message tool."),
        ("cron", "You can schedule recurring tasks using the cron tool."),
        ("todo_write", "You have a todo_write tool, but use it sparingly. Start working on the user's request directly — do NOT pre-plan with a sequence of todo_write calls. Reserve todo_write for genuinely long, multi-stage work (roughly 5+ independent subtasks that will span many turns). When you do initialize a list, emit every `add` as a parallel tool call in a single assistant response — never one item per turn."),
        ("load_skill", "You can load a skill body on demand by name via load_skill. Consult the Skills Index in your system prompt to pick the right skill name; the body is returned as the tool output."),
        ("load_memory", "You can load a memory entry body on demand by id via load_memory. Consult the Memory Index in your system prompt to pick the right entry id."),
    ]
}

// ── Image MIME detection ──────────────────────────────────────────────────────

/// Returns the MIME type based on magic bytes. Falls back to `hint`.
pub fn detect_image_mime(data: &[u8], hint: &str) -> String {
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return "image/jpeg".to_owned();
    }
    if data.len() >= 4 && data[0] == 0x89 && data[1] == b'P' && data[2] == b'N' && data[3] == b'G' {
        return "image/png".to_owned();
    }
    if data.len() >= 4 && data[0] == b'G' && data[1] == b'I' && data[2] == b'F' && data[3] == b'8' {
        return "image/gif".to_owned();
    }
    if data.len() >= 4 && data[0] == b'R' && data[1] == b'I' && data[2] == b'F' && data[3] == b'F' {
        return "image/webp".to_owned();
    }
    hint.to_owned()
}

// ── Default identity ──────────────────────────────────────────────────────────

/// Constructs the default identity prompt tailored to the tools available.
pub fn build_default_identity(tool_names: &[String]) -> String {
    if tool_names.is_empty() {
        return DEFAULT_IDENTITY_BASE.to_owned();
    }
    let available: std::collections::HashSet<&str> =
        tool_names.iter().map(|s| s.as_str()).collect();
    let hints: Vec<&str> = tool_hints()
        .iter()
        .filter(|(name, _)| available.contains(name))
        .map(|(_, hint)| *hint)
        .collect();
    if hints.is_empty() {
        return DEFAULT_IDENTITY_BASE.to_owned();
    }
    format!("{} {}", DEFAULT_IDENTITY_BASE, hints.join(" "))
}

// ── Config summary ─────────────────────────────────────────────────────────────

/// Returns a brief summary of configured agents and channels.
pub fn build_config_summary(cfg: &crate::config::config::Config) -> String {
    let mut sb = String::new();

    if !cfg.agents.list.is_empty() {
        sb.push_str("Configured agents:");
        for a in &cfg.agents.list {
            let tools_str = if a.tools.allow.is_empty() {
                String::new()
            } else {
                format!(", tools: {}", a.tools.allow.join(", "))
            };
            sb.push_str(&format!(
                "\n- {} (id: {}, model: {}{})",
                a.name, a.id, a.model, tools_str
            ));
        }
    }

    if cfg.channels.cli.enabled {
        sb.push_str("\n\nConfigured channels: cli");
    }

    sb
}

// ── Static system prompt ───────────────────────────────────────────────────────

/// Assembles the cacheable portion of the system prompt.
pub fn build_static_system_prompt(
    workspace: &str,
    system_prompt: &str,
    agent_id: &str,
    agent_name: &str,
    tool_names: &[String],
    config_summary: &str,
    skills_index: &str,
    memory_index: &str,
    memory_files: &str,
) -> String {
    let mut base: String;

    if !system_prompt.is_empty() {
        base = system_prompt.to_owned();
    } else {
        let identity_path = Path::new(workspace).join("IDENTITY.md");
        match std::fs::read_to_string(&identity_path) {
            Ok(s) => base = s,
            Err(_) => base = build_default_identity(tool_names),
        }
    }

    if !agent_id.is_empty() {
        base.push_str(&format!(
            "\n\nYou are the {:?} agent (id: {}).",
            agent_name, agent_id
        ));
    }

    base.push_str(&format!(
        "\n\nYour configuration file is at {} and your data directory is {}.",
        crate::config::config::default_config_path(),
        crate::config::config::default_data_dir(),
    ));

    if !config_summary.is_empty() {
        base.push_str("\n\n");
        base.push_str(config_summary);
    }
    if !skills_index.is_empty() {
        base.push_str(skills_index);
    }
    if !memory_index.is_empty() {
        base.push_str(memory_index);
    }
    if !memory_files.is_empty() {
        base.push_str(memory_files);
    }

    base
}

/// Concatenates the per-turn dynamic context (date line + cortex hint).
pub fn build_dynamic_system_prompt_suffix(date_line: &str, cortex_context: &str) -> String {
    let mut sb = String::new();
    if !date_line.is_empty() {
        sb.push_str(date_line);
    }
    if !cortex_context.is_empty() {
        sb.push_str(cortex_context);
    }
    sb
}

/// Returns the canonical date line: "Today's date is YYYY-MM-DD."
pub fn format_date_line(now: &chrono::DateTime<chrono::Local>) -> String {
    format!("Today's date is {}.", now.format("%Y-%m-%d"))
}

// ── Memory files ──────────────────────────────────────────────────────────────

/// Reads ROBIN.md and AGENTS.md from workspace and $HOME.
pub fn load_agent_memory_files(workspace: &str) -> String {
    struct Candidate {
        path: PathBuf,
        label: &'static str,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    if !workspace.is_empty() {
        candidates.push(Candidate {
            path: Path::new(workspace).join("ROBIN.md"),
            label: "Project memory",
        });
        candidates.push(Candidate {
            path: Path::new(workspace).join("AGENTS.md"),
            label: "Project memory",
        });
    }
    if let Some(home) = dirs::home_dir() {
        if !home.as_os_str().is_empty() {
            candidates.push(Candidate {
                path: home.join("ROBIN.md"),
                label: "User memory",
            });
            candidates.push(Candidate {
                path: home.join("AGENTS.md"),
                label: "User memory",
            });
        }
    }

    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut sb = String::new();
    let mut truncated = false;

    let truncation_notice = format!(
        "\n\n[truncated — over {} KB total agent memory]",
        MAX_AGENT_MEMORY_BYTES / 1024
    );

    for c in &candidates {
        if truncated {
            break;
        }
        let abs = match c.path.canonicalize().or_else(|_| {
            // File may not exist; use the path as-is for de-dup purposes.
            std::fs::canonicalize(c.path.parent().unwrap_or(Path::new(".")))
                .map(|p| p.join(c.path.file_name().unwrap_or_default()))
        }) {
            Ok(p) => p,
            Err(_) => c.path.clone(),
        };
        if seen.contains(&abs) {
            continue;
        }
        let data = match std::fs::read_to_string(&abs) {
            Ok(d) => d,
            Err(_) => continue,
        };
        seen.insert(abs.clone());
        let body = data.trim().to_owned();
        if body.is_empty() {
            continue;
        }

        let header = format!("\n\n## {}: {}\n\n", c.label, abs.display());
        let section = format!("{}{}", header, body);

        if sb.len() + section.len() > MAX_AGENT_MEMORY_BYTES {
            let remaining = MAX_AGENT_MEMORY_BYTES.saturating_sub(sb.len() + header.len());
            if remaining > 0 {
                let mut cut = &body[..remaining.min(body.len())];
                if let Some(idx) = cut.rfind('\n') {
                    if idx > remaining / 2 {
                        cut = &body[..idx];
                    }
                }
                sb.push_str(&header);
                sb.push_str(cut);
            }
            sb.push_str(&truncation_notice);
            truncated = true;
            continue;
        }
        sb.push_str(&section);
    }

    sb
}

// ── Message assembly ──────────────────────────────────────────────────────────

/// Converts session history into LLM messages.
pub fn assemble_messages(history: &[SessionEntry]) -> Vec<Message> {
    // First pass: collect tool-result IDs.
    let mut result_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for entry in history {
        if entry.entry_type == EntryType::ToolResult {
            if let Ok(tr) = serde_json::from_str::<ToolResultData>(entry.data.get()) {
                result_ids.insert(tr.tool_call_id);
            }
        }
    }

    let mut msgs: Vec<Message> = Vec::new();

    for entry in history {
        match entry.entry_type {
            EntryType::Compaction => {
                if let Ok(cd) = serde_json::from_str::<CompactionData>(entry.data.get()) {
                    let content = format!(
                        "[Previous conversation summary]\n\n{}\n\n\
                        Continue the conversation from where it left off without asking the user \
                        any further questions. Resume directly — do not acknowledge the summary, \
                        do not recap what was happening, do not preface with \"I'll continue\" or \
                        similar. Pick up the last task as if the break never happened.",
                        cd.summary
                    );
                    msgs.push(Message {
                        role: "user".to_owned(),
                        content,
                        ..Default::default()
                    });
                }
            }

            EntryType::Meta => {
                if let Ok(md) = serde_json::from_str::<MessageData>(entry.data.get()) {
                    msgs.push(Message {
                        role: "user".to_owned(),
                        content: format!("[Session Summary]\n{}", md.text),
                        ..Default::default()
                    });
                }
            }

            EntryType::Message => {
                if let Ok(md) = serde_json::from_str::<MessageData>(entry.data.get()) {
                    msgs = inject_missing_tool_results(msgs);
                    let mut msg = Message {
                        role: entry.role.clone(),
                        content: md.text.clone(),
                        ..Default::default()
                    };
                    if entry.role == "user" {
                        for img in &md.images {
                            if let Ok(data) = B64.decode(&img.data) {
                                let mime = detect_image_mime(&data, &img.mime_type);
                                msg.images.push(ImageContent { mime_type: mime, data });
                            }
                        }
                    }
                    msgs.push(msg);
                }
            }

            EntryType::ToolCall => {
                if let Ok(td) = serde_json::from_str::<ToolCallData>(entry.data.get()) {
                    if td.id.is_empty() {
                        continue; // skip corrupted entry (pre-fix "data":null)
                    }
                    if msgs.is_empty() || msgs.last().map(|m| m.role.as_str()) != Some("assistant") {
                        msgs.push(Message { role: "assistant".to_owned(), ..Default::default() });
                    }
                    let tc = ToolCall {
                        id: td.id,
                        name: td.tool,
                        input: td.input.get().parse::<Value>().unwrap_or(Value::Null),
                    };
                    if let Some(last) = msgs.last_mut() {
                        last.tool_calls.push(tc);
                    }
                }
            }

            EntryType::ToolResult => {
                if let Ok(tr) = serde_json::from_str::<ToolResultData>(entry.data.get()) {
                    if !last_assistant_has_tool_call(&msgs, &tr.tool_call_id) {
                        continue;
                    }
                    let content = if !tr.error.is_empty() {
                        tr.error.clone()
                    } else if !tr.output.is_empty() {
                        tr.output.clone()
                    } else {
                        "(no output)".to_owned()
                    };
                    let mut msg = Message {
                        role: "user".to_owned(),
                        content,
                        tool_call_id: tr.tool_call_id,
                        is_error: tr.is_error,
                        ..Default::default()
                    };
                    for img in &tr.images {
                        if let Ok(data) = B64.decode(&img.data) {
                            let mime = detect_image_mime(&data, &img.mime_type);
                            msg.images.push(ImageContent { mime_type: mime, data });
                        }
                    }
                    msgs.push(msg);
                }
            }
        }
    }

    msgs = inject_missing_tool_results(msgs);
    msgs
}

/// Injects synthetic tool_result messages for orphaned tool_calls.
pub fn inject_missing_tool_results(msgs: Vec<Message>) -> Vec<Message> {
    if msgs.is_empty() {
        return msgs;
    }
    let mut out: Vec<Message> = Vec::with_capacity(msgs.len());
    let mut i = 0;
    while i < msgs.len() {
        let m = msgs[i].clone();
        out.push(m.clone());
        if m.role != "assistant" || m.tool_calls.is_empty() {
            i += 1;
            continue;
        }
        // Collect tool_call_ids present in immediately following user-role tool-result messages.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut j = i + 1;
        while j < msgs.len()
            && msgs[j].role == "user"
            && !msgs[j].tool_call_id.is_empty()
        {
            seen.insert(msgs[j].tool_call_id.clone());
            j += 1;
        }
        // For each tool_call without a matching result, append a synthetic.
        for tc in &m.tool_calls {
            if !seen.contains(&tc.id) {
                out.push(Message {
                    role: "user".to_owned(),
                    content: "(tool execution was interrupted)".to_owned(),
                    tool_call_id: tc.id.clone(),
                    is_error: true,
                    ..Default::default()
                });
            }
        }
        i += 1;
    }
    out
}

/// Returns true when the most recent assistant message contains a tool_call
/// with the given ID.
pub fn last_assistant_has_tool_call(msgs: &[Message], id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    for m in msgs.iter().rev() {
        if m.role != "assistant" {
            continue;
        }
        for tc in &m.tool_calls {
            if tc.id == id {
                return true;
            }
        }
        return false; // first assistant we hit didn't have it
    }
    false
}

// ── Tool-result spill / prune ─────────────────────────────────────────────────

/// Configuration for disk-spillover of oversized tool results.
#[derive(Clone, Default)]
pub struct SpillConfig {
    pub workspace: String,
    pub session_key: String,
}

/// Writes oversized content to the workspace-local spill directory and returns
/// the absolute path.
pub fn spill_tool_result(
    cfg: &SpillConfig,
    tool_call_id: &str,
    content: &str,
) -> anyhow::Result<std::path::PathBuf> {
    if cfg.workspace.is_empty() || cfg.session_key.is_empty() || tool_call_id.is_empty() {
        anyhow::bail!("spill_tool_result: workspace, session key, and tool call id are required");
    }
    let dir = Path::new(&cfg.workspace)
        .join(".robin")
        .join("spill")
        .join(&cfg.session_key);
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("spill mkdir: {}", e))?;
    let path = dir.join(format!("{}.txt", tool_call_id));
    std::fs::write(&path, content).map_err(|e| anyhow::anyhow!("spill write: {}", e))?;
    Ok(path)
}

/// Bounds oversized tool results in the message history.
pub fn prune_tool_results(msgs: &mut Vec<Message>, max_len: usize, cfg: &SpillConfig) {
    for msg in msgs.iter_mut() {
        if msg.tool_call_id.is_empty() || msg.content.len() <= max_len {
            continue;
        }
        if msg.content.contains(TRUNCATION_MARKER) || msg.content.contains(SPILL_MARKER) {
            continue;
        }
        let original_len = msg.content.len();
        let head_end = max_len.min(msg.content.len());
        let mut head = &msg.content[..head_end];
        if let Some(idx) = head.rfind('\n') {
            if idx > max_len / 2 {
                head = &msg.content[..idx];
            }
        }
        let head = head.to_owned();

        // Try spill.
        if !cfg.workspace.is_empty() && !cfg.session_key.is_empty() {
            if let Ok(path) =
                spill_tool_result(cfg, &msg.tool_call_id, &msg.content)
            {
                msg.content = format!(
                    "{}\n\n{}{} of {} chars saved to {}; use read_file to access the full output]",
                    head,
                    SPILL_MARKER,
                    head.len(),
                    original_len,
                    path.display()
                );
                continue;
            }
        }

        // Fall back to in-place truncation.
        msg.content = format!(
            "{}\n\n{}{} of {} chars; re-run the tool with offset/limit to see more]",
            head, TRUNCATION_MARKER, head.len(), original_len
        );
    }
}

// ── Path extraction ────────────────────────────────────────────────────────────

/// Returns the "path" field from a tool call's JSON input, or `""`.
pub fn extract_path_from_input(input: &Value) -> String {
    input
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned()
}

// ── Post-compact restore ──────────────────────────────────────────────────────

/// Number of recently-touched files to re-inject after compaction.
pub const POST_COMPACT_RESTORE_FILES: usize = 5;
/// Per-file byte budget for the restore message.
pub const POST_COMPACT_RESTORE_BYTES_PER_FILE: usize = 5 * 1024;

/// Builds the post-compact restore message.
pub fn build_post_compact_restore(
    files: &[String],
    max_files: usize,
    max_bytes_per_file: usize,
) -> Message {
    if files.is_empty() || max_files == 0 || max_bytes_per_file == 0 {
        return Message::default();
    }
    let mut sb = String::from(
        "<system-reminder>\nFiles you were recently working with — full contents below for \
        context restoration after history compaction. The file system is the source of truth; \
        re-read with the read_file tool if you need updated content.\n\n",
    );
    let mut picked = 0usize;

    for path in files.iter().rev() {
        if picked >= max_files {
            break;
        }
        let metadata = match std::fs::metadata(path) {
            Ok(m) if !m.is_dir() => m,
            _ => continue,
        };
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let truncated = data.len() > max_bytes_per_file;
        let mut data = data;
        if truncated {
            data.truncate(max_bytes_per_file);
            let s = String::from_utf8_lossy(&data);
            if let Some(idx) = s.rfind('\n') {
                if idx > max_bytes_per_file / 2 {
                    data.truncate(idx);
                }
            }
        }
        let text = String::from_utf8_lossy(&data);
        sb.push_str(&format!("<file path={:?}>\n", path));
        sb.push_str(&text);
        if !text.ends_with('\n') {
            sb.push('\n');
        }
        if truncated {
            sb.push_str(&format!("[truncated — over {} bytes]\n", max_bytes_per_file));
        }
        sb.push_str("</file>\n\n");
        picked += 1;
        let _ = metadata; // suppress unused warning
    }

    if picked == 0 {
        return Message::default();
    }
    sb.push_str("</system-reminder>");
    Message {
        role: "user".to_owned(),
        content: sb,
        ..Default::default()
    }
}

/// Prepends the post-compact restore message to `msgs` when non-empty.
pub fn prepend_post_compact_restore(msgs: Vec<Message>, touched: &[String]) -> Vec<Message> {
    let restore =
        build_post_compact_restore(touched, POST_COMPACT_RESTORE_FILES, POST_COMPACT_RESTORE_BYTES_PER_FILE);
    if restore.content.is_empty() {
        return msgs;
    }
    let mut out = Vec::with_capacity(msgs.len() + 1);
    out.push(restore);
    out.extend(msgs);
    out
}