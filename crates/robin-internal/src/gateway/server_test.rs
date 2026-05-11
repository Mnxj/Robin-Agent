use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

use crate::gateway::{
    server::{Server, ServerOptions},
    skills::{SkillHandlerState, SkillReloader, SkillParser},
    websocket::{
        AgentConfig, ConfigSurface, SessionStoreTrait, SessionSummary, HistoryEntry,
        WebSocketHandlerState,
    },
};

// ─── Minimal stubs ────────────────────────────────────────────────────────

struct NoopConfig;
impl ConfigSurface for NoopConfig {
    fn list_agents(&self) -> Vec<AgentConfig> {
        vec![]
    }
    fn get_agent(&self, _id: &str) -> Option<AgentConfig> {
        None
    }
}

struct NoopSessionStore;
impl SessionStoreTrait for NoopSessionStore {
    fn list(&self, _agent_id: &str) -> anyhow::Result<Vec<SessionSummary>> {
        Ok(vec![])
    }
    fn exists(&self, _agent_id: &str, _key: &str) -> bool {
        false
    }
    fn create(&self, _agent_id: &str, _key: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn delete(&self, _agent_id: &str, _key: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn history(&self, _agent_id: &str, _key: &str) -> anyhow::Result<Vec<HistoryEntry>> {
        Ok(vec![])
    }
}

fn new_test_server() -> Server {
    let config = Arc::new(NoopConfig);
    let store = Arc::new(NoopSessionStore);
    let ws_state = Arc::new(WebSocketHandlerState::new(config, store));
    Server::new("127.0.0.1".to_string(), 0, ws_state, ServerOptions::default())
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_health_endpoint() {
    let srv = new_test_server();
    let router = srv.router().clone();

    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["timestamp"].as_str().is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
async fn test_health_endpoint_content_type() {
    let srv = new_test_server();
    let router = srv.router().clone();

    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("application/json"), "content-type was: {}", ct);
}

/// Shutdown must be safe to call before start has run.
#[test]
fn test_shutdown_before_start() {
    let srv = new_test_server();
    // Should not panic.
    srv.shutdown();
}

/// Defensive: even a zeroed-out Server value should not panic on shutdown.
/// We model this by calling shutdown immediately without ever calling start.
#[test]
fn test_shutdown_noop_when_never_started() {
    let srv = new_test_server();
    srv.shutdown();
    // Call again — idempotent.
    srv.shutdown();
}

/// Skills routes must be mounted and reachable when opts.skills is set.
#[tokio::test]
async fn test_skill_routes_mounted() {
    use crate::gateway::skills::{SkillHandlerState, SkillListEntry, SkillParsed};
    use std::path::Path as FsPath;

    struct NoopReloader;
    impl SkillReloader for NoopReloader {
        fn load_from(&self, _dirs: &[&str]) -> anyhow::Result<()> {
            Ok(())
        }
    }

    let dir = tempfile::tempdir().unwrap();

    let config = Arc::new(NoopConfig);
    let store = Arc::new(NoopSessionStore);
    let ws_state = Arc::new(WebSocketHandlerState::new(config, store));

    let skills_state = SkillHandlerState {
        loader: Arc::new(NoopReloader),
        parser: None,
        skills_dir: dir.path().to_path_buf(),
        reload_dirs: vec![dir.path().to_string_lossy().to_string()],
    };

    let opts = ServerOptions {
        skills: Some(skills_state),
        ..Default::default()
    };

    let srv = Server::new("127.0.0.1".to_string(), 0, ws_state, opts);
    let router = srv.router().clone();

    // GET list — should return 200 with empty skills array
    let req = Request::builder()
        .method("GET")
        .uri("/settings/api/skills")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "list route should be mounted"
    );

    // GET specific — empty dir, should be 404 (route mounted, file missing)
    let req = Request::builder()
        .method("GET")
        .uri("/settings/api/skills/anything.md")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "get route should be mounted"
    );

    // DELETE specific — empty dir, should be 404
    let req = Request::builder()
        .method("DELETE")
        .uri("/settings/api/skills/anything.md")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "delete route should be mounted"
    );
}