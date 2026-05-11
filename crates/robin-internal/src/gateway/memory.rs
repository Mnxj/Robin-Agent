use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::SystemTime,
};

/// An entry returned by the List endpoint.
#[derive(Serialize)]
pub struct MemoryListEntry {
    pub id: String,
    pub title: String,
    pub modified: String,
    pub bytes: usize,
}

/// Trait for the memory manager (allows test injection).
pub trait MemoryManagerTrait: Send + Sync {
    fn entries(&self) -> Vec<MemoryEntry>;
    fn get(&self, id: &str) -> Option<MemoryEntry>;
    fn save(&self, id: &str, content: &str) -> anyhow::Result<()>;
    fn delete(&self, id: &str) -> anyhow::Result<()>;
}

/// A single memory entry.
#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub mod_time: SystemTime,
}

/// Maximum size of a single memory entry (256 KB).
const MAX_MEMORY_ENTRY_BYTES: usize = 256 * 1024;

static MEMORY_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9._-]+$").unwrap());

fn validate_memory_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id is empty".to_string());
    }
    if !MEMORY_ID_RE.is_match(id) {
        return Err(format!(
            "id {:?} is not a valid memory id (allowed: letters, digits, dot, dash, underscore)",
            id
        ));
    }
    Ok(())
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    let body = serde_json::json!({"error": msg});
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn json_ok(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

/// State wrapper for memory handlers.
#[derive(Clone)]
pub struct MemoryHandlerState {
    pub manager: Option<Arc<dyn MemoryManagerTrait>>,
}

fn disabled_response() -> Response {
    json_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "memory is disabled in config (set memory.enabled=true)",
    )
}

/// Handler: GET /settings/api/memory
pub async fn list_memory(State(state): State<MemoryHandlerState>) -> Response {
    let mgr = match &state.manager {
        Some(m) => m.clone(),
        None => return disabled_response(),
    };

    let mut entries = mgr.entries();
    // Sort by ID for stable ordering
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    let list: Vec<MemoryListEntry> = entries
        .into_iter()
        .map(|e| {
            let modified = chrono::DateTime::<chrono::Utc>::from(e.mod_time)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            MemoryListEntry {
                id: e.id,
                title: e.title,
                modified,
                bytes: e.content.len(),
            }
        })
        .collect();

    json_ok(
        StatusCode::OK,
        serde_json::json!({ "entries": list }),
    )
}

/// Handler: GET /settings/api/memory/:id
pub async fn get_memory(
    State(state): State<MemoryHandlerState>,
    Path(id): Path<String>,
) -> Response {
    let mgr = match &state.manager {
        Some(m) => m.clone(),
        None => return disabled_response(),
    };

    if let Err(e) = validate_memory_id(&id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    match mgr.get(&id) {
        Some(entry) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(entry.content))
            .unwrap(),
        None => json_error(StatusCode::NOT_FOUND, "entry not found"),
    }
}

#[derive(Deserialize)]
pub struct SaveMemoryPayload {
    pub id: String,
    pub content: String,
}

/// Handler: POST /settings/api/memory
pub async fn save_memory(
    State(state): State<MemoryHandlerState>,
    body: axum::body::Bytes,
) -> Response {
    let mgr = match &state.manager {
        Some(m) => m.clone(),
        None => return disabled_response(),
    };

    if body.len() > MAX_MEMORY_ENTRY_BYTES {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("entry exceeds {} byte limit", MAX_MEMORY_ENTRY_BYTES),
        );
    }

    let payload: SaveMemoryPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("invalid JSON: {}", e)),
    };

    if let Err(e) = validate_memory_id(&payload.id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    if payload.content.trim().is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "content is empty");
    }

    match mgr.save(&payload.id, &payload.content) {
        Ok(_) => json_ok(StatusCode::OK, serde_json::json!({"ok": true, "id": payload.id})),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("save: {}", e)),
    }
}

/// Handler: DELETE /settings/api/memory/:id
pub async fn delete_memory(
    State(state): State<MemoryHandlerState>,
    Path(id): Path<String>,
) -> Response {
    let mgr = match &state.manager {
        Some(m) => m.clone(),
        None => return disabled_response(),
    };

    if let Err(e) = validate_memory_id(&id) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    match mgr.delete(&id) {
        Ok(_) => json_ok(StatusCode::OK, serde_json::json!({"ok": true})),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                json_error(StatusCode::NOT_FOUND, &msg)
            } else {
                json_error(StatusCode::INTERNAL_SERVER_ERROR, &msg)
            }
        }
    }
}