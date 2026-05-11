use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use serde::Serialize;
use std::sync::Arc;
use tracing::info;

/// Trait providing the settings/config surface.
pub trait ConfigTrait: Send + Sync {
    fn to_json(&self) -> anyhow::Result<serde_json::Value>;
    fn from_json(&self, v: serde_json::Value) -> anyhow::Result<Box<dyn ConfigTrait>>;
    fn validate(&self) -> anyhow::Result<()>;
    fn save(&self) -> anyhow::Result<()>;
    fn path(&self) -> String;
}

/// Trait for the tool registry listing surface.
pub trait ToolRegistryTrait: Send + Sync {
    fn names(&self) -> Vec<String>;
    fn description(&self, name: &str) -> Option<String>;
}

/// Trait for bootstrap status snapshots.
pub trait BootstrapSnapshotter: Send + Sync {
    fn snapshot(&self) -> serde_json::Value;
}

/// State for settings handlers.
#[derive(Clone)]
pub struct SettingsHandlerState {
    pub config_json: Arc<parking_lot::RwLock<serde_json::Value>>,
    pub config_path: String,
    pub tool_registry: Option<Arc<dyn ToolRegistryTrait>>,
    pub bootstrap: Option<Arc<dyn BootstrapSnapshotter>>,
    pub on_save: Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>,
}

fn json_error_resp(status: StatusCode, msg: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"error":"{}"}}"#, msg.replace('"', "\\\""))))
        .unwrap()
}

/// Handler: GET /settings  (serves the settings HTML page)
pub async fn settings_page_handler() -> impl IntoResponse {
    let mut resp = Html(SETTINGS_HTML).into_response();
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    resp
}

/// Handler: GET /settings/api/config
pub async fn get_config_handler(
    State(state): State<SettingsHandlerState>,
) -> Response {
    let cfg = state.config_json.read();
    match serde_json::to_string_pretty(&*cfg) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => json_error_resp(StatusCode::INTERNAL_SERVER_ERROR, "marshal config"),
    }
}

/// Handler: POST /settings/api/config
pub async fn save_config_handler(
    State(state): State<SettingsHandlerState>,
    body: axum::body::Bytes,
) -> Response {
    if body.len() > 1 << 20 {
        return json_error_resp(StatusCode::BAD_REQUEST, "request body too large");
    }

    let new_cfg: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"error":"invalid JSON: {}"}}"#, e)))
                .unwrap();
        }
    };

    // Write updated config to disk (path from state)
    let path = &state.config_path;
    match std::fs::write(
        path,
        serde_json::to_string_pretty(&new_cfg).unwrap_or_default(),
    ) {
        Ok(_) => {}
        Err(e) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"error":"save: {}"}}"#, e)))
                .unwrap();
        }
    }

    // Update in-memory config
    *state.config_json.write() = new_cfg.clone();

    info!("config saved via settings page");

    // Trigger hot-reload callback if configured
    if let Some(cb) = &state.on_save {
        cb(new_cfg);
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"ok":true}"#))
        .unwrap()
}

/// Handler: GET /settings/api/tools
pub async fn list_tools_handler(
    State(state): State<SettingsHandlerState>,
) -> Response {
    #[derive(Serialize)]
    struct ToolDTO {
        name: String,
        description: String,
    }
    #[derive(Serialize)]
    struct Out {
        tools: Vec<ToolDTO>,
    }

    let tools = match &state.tool_registry {
        None => vec![],
        Some(reg) => {
            let mut names = reg.names();
            names.sort();
            names
                .into_iter()
                .map(|n| {
                    let desc = reg.description(&n).unwrap_or_default();
                    ToolDTO {
                        name: n,
                        description: desc,
                    }
                })
                .collect()
        }
    };

    let out = Out { tools };
    match serde_json::to_string(&out) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => json_error_resp(StatusCode::INTERNAL_SERVER_ERROR, "marshal tools"),
    }
}

/// Handler: GET /settings/api/bootstrap
pub async fn bootstrap_status_handler(
    State(state): State<SettingsHandlerState>,
) -> Response {
    let snap = match &state.bootstrap {
        Some(b) => b.snapshot(),
        None => serde_json::json!({ "active": false, "models": {} }),
    };

    match serde_json::to_string(&snap) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => json_error_resp(StatusCode::INTERNAL_SERVER_ERROR, "marshal bootstrap"),
    }
}

const SETTINGS_HTML: &str = include_str!("settings_template.html");