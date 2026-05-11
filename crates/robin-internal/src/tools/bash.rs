use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use super::tool::{resolve_existing_path_strict, Tool, ToolResult};

// Re-export from tool for test convenience
pub use super::tool::sanitize_llm_text;

const DEFAULT_BASH_TIMEOUT: Duration = Duration::from_secs(120);

/// Regex matching absolute-path tokens in a shell command:
/// - inside single quotes: '/...'
/// - inside double quotes: "/..."
/// - bare with optional backslash-escaped chars
fn bash_abs_path_re() -> &'static Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"'(/[^']*)'|"(/[^"]*)"|(/(?:[^\s\\]|\\.)+)"#).unwrap()
    })
}

/// Removes single-character backslash escapes so the string can be stat()'d.
fn unescape_bash_token(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            out.push(bytes[i + 1] as char);
            i += 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Wraps `s` in single quotes, escaping embedded single quotes.
pub fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Scans `cmd` for absolute-path tokens and substitutes any that don't exist
/// on disk with their Unicode-whitespace-normalized counterparts.
/// Returns (rewritten_cmd, substitutions).
pub fn resolve_bash_command_paths(cmd: &str) -> (String, Vec<[String; 2]>) {
    let re = bash_abs_path_re();
    let mut subs: Vec<[String; 2]> = Vec::new();
    let result = re.replace_all(cmd, |caps: &regex::Captures| {
        let raw = if let Some(m) = caps.get(1) {
            m.as_str().to_owned()
        } else if let Some(m) = caps.get(2) {
            m.as_str().to_owned()
        } else if let Some(m) = caps.get(3) {
            unescape_bash_token(m.as_str())
        } else {
            return caps[0].to_owned();
        };
        if raw.is_empty() {
            return caps[0].to_owned();
        }
        let resolved = resolve_existing_path_strict(&raw);
        if resolved == raw {
            return caps[0].to_owned();
        }
        subs.push([raw, resolved.clone()]);
        shell_single_quote(&resolved)
    });
    (result.into_owned(), subs)
}

/// Formats a notice block listing path substitutions made.
fn path_subs_notice(subs: &[[String; 2]]) -> String {
    if subs.is_empty() {
        return String::new();
    }
    let mut b = String::from(
        "[robin] adjusted paths in command (Unicode-whitespace recovery):\n",
    );
    for s in subs {
        b.push_str(&format!("  {:?} -> {:?}\n", s[0], s[1]));
    }
    b.push_str("---\n");
    b
}

/// Extracts executable names from a bash command string.
fn extract_commands(cmd: &str) -> Vec<String> {
    let operators = ["&&", "||", "|", ";"];
    let mut parts: Vec<String> = Vec::new();
    let mut remaining = cmd;

    loop {
        let mut min_idx = remaining.len();
        let mut op_len = 0usize;
        for op in &operators {
            if let Some(idx) = remaining.find(op) {
                if idx < min_idx {
                    min_idx = idx;
                    op_len = op.len();
                }
            }
        }
        let part = remaining[..min_idx].trim().to_owned();
        if !part.is_empty() {
            parts.push(part);
        }
        if min_idx + op_len >= remaining.len() {
            break;
        }
        remaining = &remaining[min_idx + op_len..];
    }

    let mut cmds = Vec::new();
    for part in &parts {
        for tok in part.split_whitespace() {
            // Skip env var assignments (FOO=bar)
            if tok.contains('=') && !tok.starts_with('-') {
                continue;
            }
            let base = std::path::Path::new(tok)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(tok)
                .to_owned();
            cmds.push(base);
            break;
        }
    }
    cmds
}

/// Controls which commands the bash tool is allowed to execute.
#[derive(Debug, Clone, Default)]
pub struct ExecPolicy {
    /// "deny", "allowlist", or "full"
    pub level: String,
    /// Command basenames allowed when `level` is "allowlist".
    pub allowlist: Vec<String>,
}

/// Executes shell commands.
pub struct BashTool {
    pub work_dir: String,
    pub exec_policy: Option<ExecPolicy>,
}

impl Default for BashTool {
    fn default() -> Self {
        Self { work_dir: String::new(), exec_policy: None }
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }

    fn description(&self) -> &str {
        "Execute a bash command and return its output. The command runs in a shell with a configurable timeout (default 120 seconds). IMPORTANT: always wrap file paths in double quotes (e.g. cat \"/path/with spaces/file.txt\") so paths containing spaces or special characters survive shell tokenization."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120)"
                }
            },
            "required": ["command"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { false }

    fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c.to_owned(),
            _ => return Ok(ToolResult::err("command is required")),
        };
        let timeout_secs = input.get("timeout").and_then(|v| v.as_u64()).unwrap_or(0);

        // Recover from Unicode-whitespace path mismatches
        let (resolved_cmd, path_subs) = resolve_bash_command_paths(&command);
        let command = resolved_cmd;

        // Enforce exec policy
        if let Some(policy) = &self.exec_policy {
            match policy.level.as_str() {
                "deny" => return Ok(ToolResult::err("bash execution is disabled by policy")),
                "allowlist" => {
                    let metacharacters = ["$(", "`", "<(", ">(", "${", "\\n"];
                    for meta in &metacharacters {
                        if command.contains(meta) {
                            return Ok(ToolResult::err(
                                "command contains shell metacharacters not allowed in allowlist mode",
                            ));
                        }
                    }
                    let cmds = extract_commands(&command);
                    let allowed: std::collections::HashSet<&str> =
                        policy.allowlist.iter().map(|s| s.as_str()).collect();
                    for cmd in &cmds {
                        if !allowed.contains(cmd.as_str()) {
                            return Ok(ToolResult::err(format!(
                                "command {:?} is not in the exec allowlist",
                                cmd
                            )));
                        }
                    }
                }
                _ => {} // "full" or unrecognized: allow everything
            }
        }

        let timeout = if timeout_secs > 0 {
            Duration::from_secs(timeout_secs)
        } else {
            DEFAULT_BASH_TIMEOUT
        };

        let notice = path_subs_notice(&path_subs);

        // Run the command
        #[cfg(target_os = "windows")]
        let shell_args = vec!["cmd", "/c", &command];
        #[cfg(not(target_os = "windows"))]
        let shell_args = vec!["bash", "-c", &command];

        let mut cmd = std::process::Command::new(shell_args[0]);
        cmd.args(&shell_args[1..]);
        if !self.work_dir.is_empty() {
            cmd.current_dir(&self.work_dir);
        }

        // Use a thread to enforce timeout
        let command_clone = command.clone();
        let work_dir_clone = self.work_dir.clone();

        let result = std::thread::scope(|_| {
            let mut child = match cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => return Err(format!("spawn failed: {}", e)),
            };

            // Wait with timeout using a thread
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(child.wait_with_output());
            });

            match rx.recv_timeout(timeout) {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(e)) => Err(format!("command failed: {}", e)),
                Err(_) => Err("command timed out".to_owned()),
            }
        });

        let _ = command_clone;
        let _ = work_dir_clone;

        match result {
            Err(msg) => Ok(ToolResult { output: notice, error: msg, ..Default::default() }),
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                if !output.status.success() {
                    let msg = if !stderr.is_empty() {
                        stderr.clone()
                    } else {
                        format!("exit status: {}", output.status)
                    };
                    Ok(ToolResult {
                        output: notice + &stdout,
                        error: msg,
                        ..Default::default()
                    })
                } else {
                    let mut combined = stdout;
                    if !stderr.is_empty() {
                        combined.push_str("\nSTDERR:\n");
                        combined.push_str(&stderr);
                    }
                    Ok(ToolResult::ok(notice + &combined))
                }
            }
        }
    }
}

#[path = "bash_test.rs"]
#[cfg(test)]
mod bash_test;