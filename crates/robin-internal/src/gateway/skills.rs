use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, StatusCode},
    response::Response,
};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::SystemTime,
};

/// Maximum upload size for a skill file (256 KB).
pub const MAX_SKILL_UPLOAD_BYTES: usize = 256 * 1024;

/// Matches a safe skill filename: [A-Za-z0-9._-]+.md
static SKILL_NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9._-]+\.md$").unwrap());

/// Validates that a skill filename is safe.
pub fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name is empty".to_string());
    }
    if !SKILL_NAME_RE.is_match(name) {
        return Err(format!("name {:?} is not a valid skill filename", name));
    }
    Ok(())
}

/// A skill entry returned by the List endpoint.
#[derive(Serialize, Default)]
pub struct SkillListEntry {
    pub name: String,
    pub filename: String,
    pub description: String,
    pub tags: Vec<String>,
    pub size_bytes: u64,
    pub modified: String,
    pub unavailable: bool,
    pub missing_bins: Vec<String>,
    pub parse_error: String,
}

/// Trait for the skill loader (allows test injection).
pub trait SkillReloader: Send + Sync {
    fn load_from(&self, dirs: &[&str]) -> anyhow::Result<()>;
}

/// Trait for parsing individual skill files (for listing).
pub trait SkillParser: Send + Sync {
    fn parse(&self, path: &std::path::Path) -> Result<SkillParsed, String>;
    fn split_frontmatter<'a>(&self, content: &'a str) -> (&'a str, &'a str);
    fn missing_bins(&self, parsed: &SkillParsed) -> Vec<String>;
}

/// A parsed skill.
pub struct SkillParsed {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub required_bins: Vec<String>,
}

/// State for skill handlers.
#[derive(Clone)]
pub struct SkillHandlerState {
    pub loader: Arc<dyn SkillReloader>,
    pub parser: Option<Arc<dyn SkillParser>>,
    pub skills_dir: PathBuf,
    pub reload_dirs: Vec<String>,
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    let body = serde_json::json!({"error": msg});
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn json_ok(body: serde_json::Value) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

/// Handler: GET /settings/api/skills
pub async fn list_skills(State(state): State<SkillHandlerState>) -> Response {
    let entries = match std::fs::read_dir(&state.skills_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return json_ok(serde_json::json!({"skills": []}));
        }
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("read skills dir: {}", e),
            );
        }
    };

    let mut skills: Vec<SkillListEntry> = Vec::new();

    for entry in entries.flatten() {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy().to_string();
        if !fname.to_lowercase().ends_with(".md") {
            continue;
        }
        let path = state.skills_dir.join(&fname);
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let modified = meta
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let modified_str = chrono::DateTime::<chrono::Utc>::from(modified)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let mut skill_entry = SkillListEntry {
            filename: fname.clone(),
            tags: vec![],
            missing_bins: vec![],
            size_bytes: meta.len(),
            modified: modified_str,
            ..Default::default()
        };

        // Try to parse the skill file
        if let Some(parser) = &state.parser {
            match parser.parse(&path) {
                Ok(parsed) => {
                    skill_entry.name = parsed.name.clone();
                    skill_entry.description = parsed.description.clone();
                    if !parsed.tags.is_empty() {
                        skill_entry.tags = parsed.tags.clone();
                    }
                    let missing = parser.missing_bins(&parsed);
                    if !missing.is_empty() {
                        skill_entry.unavailable = true;
                        skill_entry.missing_bins = missing;
                    }
                }
                Err(e) => {
                    // Use filename (sans extension) as fallback name
                    skill_entry.name = fname.trim_end_matches(".md").to_string();
                    skill_entry.parse_error = e;
                }
            }
        } else {
            skill_entry.name = fname.trim_end_matches(".md").to_string();
        }

        skills.push(skill_entry);
    }

    skills.sort_by(|a, b| a.filename.cmp(&b.filename));

    json_ok(serde_json::json!({"skills": skills}))
}

/// Handler: GET /settings/api/skills/:name
pub async fn get_skill(
    State(state): State<SkillHandlerState>,
    Path(raw): Path<String>,
) -> Response {
    if let Err(e) = validate_skill_name(&raw) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    // Defense-in-depth: use only the basename
    let name = std::path::Path::new(&raw)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&raw)
        .to_string();

    let full = state.skills_dir.join(&name);
    match std::fs::read(&full) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(data))
            .unwrap(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json_error(StatusCode::NOT_FOUND, "skill not found")
        }
        Err(e) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("read: {}", e),
        ),
    }
}

/// Handler: POST /settings/api/skills  (multipart upload)
pub async fn upload_skill(
    State(state): State<SkillHandlerState>,
    mut multipart: Multipart,
) -> Response {
    // Parse the first "file" field
    let (filename, data) = loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name() == Some("file") {
                    let filename = field
                        .file_name()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let data = match field.bytes().await {
                        Ok(b) => b,
                        Err(e) => {
                            return json_error(
                                StatusCode::BAD_REQUEST,
                                &format!("read upload: {}", e),
                            );
                        }
                    };
                    break (filename, data);
                }
            }
            Ok(None) => {
                return json_error(StatusCode::BAD_REQUEST, r#"missing "file" field"#);
            }
            Err(e) => {
                return json_error(StatusCode::BAD_REQUEST, &format!("parse multipart: {}", e));
            }
        }
    };

    if data.len() > MAX_SKILL_UPLOAD_BYTES {
        return json_error(StatusCode::PAYLOAD_TOO_LARGE, "upload exceeds 256KB limit");
    }

    // Sanitize via basename
    let name = std::path::Path::new(&filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&filename)
        .to_string();

    if let Err(e) = validate_skill_name(&name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    // Validate YAML frontmatter if present
    let content_str = String::from_utf8_lossy(&data);
    if content_str.starts_with("---") {
        let end = content_str[3..].find("\n---").map(|i| i + 3);
        if let Some(end_idx) = end {
            let fm = &content_str[3..end_idx];
            if let Err(e) = serde_yaml::from_str::<serde_yaml::Value>(fm) {
                return json_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &format!("invalid YAML frontmatter: {}", e),
                );
            }
        }
    }

    let target = state.skills_dir.join(&name);
    if target.exists() {
        return json_error(
            StatusCode::CONFLICT,
            &format!(
                "skill {:?} already exists; delete first to replace",
                name
            ),
        );
    }

    // Atomic write: write to tmp then rename
    let tmp = state.skills_dir.join(format!("{}.tmp", name));
    if let Err(e) = std::fs::write(&tmp, &data) {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write tmp: {}", e),
        );
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("rename: {}", e),
        );
    }

    let stem = name.trim_end_matches(".md").to_string();
    let reload_dirs: Vec<&str> = state.reload_dirs.iter().map(|s| s.as_str()).collect();
    let mut resp = serde_json::json!({
        "ok": true,
        "name": stem,
        "filename": name,
    });

    if let Err(e) = state.loader.load_from(&reload_dirs) {
        resp["warning"] = serde_json::json!(format!("reload failed: {}", e));
    }

    json_ok(resp)
}

/// Handler: DELETE /settings/api/skills/:name
pub async fn delete_skill(
    State(state): State<SkillHandlerState>,
    Path(raw): Path<String>,
) -> Response {
    if let Err(e) = validate_skill_name(&raw) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    let name = std::path::Path::new(&raw)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&raw)
        .to_string();

    let target = state.skills_dir.join(&name);
    match std::fs::metadata(&target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return json_error(StatusCode::NOT_FOUND, "skill not found");
        }
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("stat: {}", e),
            );
        }
        Ok(_) => {}
    }

    if let Err(e) = std::fs::remove_file(&target) {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("remove: {}", e),
        );
    }

    let reload_dirs: Vec<&str> = state.reload_dirs.iter().map(|s| s.as_str()).collect();
    let mut resp = serde_json::json!({"ok": true});

    if let Err(e) = state.loader.load_from(&reload_dirs) {
        resp["warning"] = serde_json::json!(format!("reload failed: {}", e));
    }

    json_ok(resp)
}

#[cfg(test)]
#[path = "skills_test.rs"]
mod skills_test;
