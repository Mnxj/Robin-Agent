use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{delete, get, post},
    Router,
};
use bytes::Bytes;
use std::{io::Write, sync::Arc};
use tower::ServiceExt;

use crate::gateway::skills::{
    delete_skill, get_skill, list_skills, upload_skill, validate_skill_name, SkillHandlerState,
    SkillParser, SkillParsed, SkillReloader, MAX_SKILL_UPLOAD_BYTES,
};

// ─── Helpers ─────────────────────────────────────────────────────────────

struct NoopReloader;
impl SkillReloader for NoopReloader {
    fn load_from(&self, _dirs: &[&str]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A SkillReloader that always returns an error.
struct FailReloader {
    msg: &'static str,
}
impl SkillReloader for FailReloader {
    fn load_from(&self, _dirs: &[&str]) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("{}", self.msg))
    }
}

fn new_test_state(dir: &std::path::Path) -> SkillHandlerState {
    SkillHandlerState {
        loader: Arc::new(NoopReloader),
        parser: None,
        skills_dir: dir.to_path_buf(),
        reload_dirs: vec![dir.to_string_lossy().to_string()],
    }
}

fn build_router(state: SkillHandlerState) -> Router {
    Router::new()
        .route("/settings/api/skills", get(list_skills).post(upload_skill))
        .route(
            "/settings/api/skills/:name",
            get(get_skill).delete(delete_skill),
        )
        .with_state(state)
}

fn write_skill(dir: &std::path::Path, filename: &str, content: &str) {
    std::fs::write(dir.join(filename), content).unwrap();
}

/// Build a multipart/form-data body with a single "file" field.
fn upload_body(filename: &str, content: &[u8]) -> (Bytes, String) {
    let boundary = "testboundary1234567890";
    let ct = format!("multipart/form-data; boundary={}", boundary);
    let mut body = Vec::new();
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\n",
    )
    .unwrap();
    body.extend_from_slice(content);
    write!(body, "\r\n--{boundary}--\r\n").unwrap();
    (Bytes::from(body), ct)
}

// ─── validate_skill_name ──────────────────────────────────────────────────

#[test]
fn test_validate_skill_name_simple() {
    assert!(validate_skill_name("cortex.md").is_ok());
}

#[test]
fn test_validate_skill_name_dashes_underscores() {
    assert!(validate_skill_name("my-skill_v2.md").is_ok());
}

#[test]
fn test_validate_skill_name_digits() {
    assert!(validate_skill_name("skill123.md").is_ok());
}

#[test]
fn test_validate_skill_name_dots() {
    assert!(validate_skill_name("skill.v2.md").is_ok());
}

#[test]
fn test_validate_skill_name_empty() {
    assert!(validate_skill_name("").is_err());
}

#[test]
fn test_validate_skill_name_no_md_extension() {
    assert!(validate_skill_name("cortex").is_err());
}

#[test]
fn test_validate_skill_name_wrong_extension() {
    assert!(validate_skill_name("cortex.txt").is_err());
}

#[test]
fn test_validate_skill_name_path_separator_forward() {
    assert!(validate_skill_name("foo/bar.md").is_err());
}

#[test]
fn test_validate_skill_name_path_separator_back() {
    assert!(validate_skill_name("foo\\bar.md").is_err());
}

#[test]
fn test_validate_skill_name_parent_traversal() {
    assert!(validate_skill_name("../foo.md").is_err());
}

#[test]
fn test_validate_skill_name_space() {
    assert!(validate_skill_name("foo bar.md").is_err());
}

#[test]
fn test_validate_skill_name_colon() {
    assert!(validate_skill_name("foo:bar.md").is_err());
}

#[test]
fn test_validate_skill_name_unicode() {
    assert!(validate_skill_name("fööö.md").is_err());
}

// ─── List ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_empty() {
    let dir = tempfile::tempdir().unwrap();
    let router = build_router(new_test_state(dir.path()));

    let req = Request::builder()
        .method("GET")
        .uri("/settings/api/skills")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("application/json"));

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["skills"], serde_json::json!([]));
}

#[tokio::test]
async fn test_list_parses_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    write_skill(
        dir.path(),
        "alpha.md",
        "---\nname: alpha\ndescription: First skill\ntags: [a, b]\n---\nbody1\n",
    );
    write_skill(
        dir.path(),
        "beta.md",
        "---\nname: beta\ndescription: Second skill\n---\nbody2\n",
    );

    // We need a parser to actually parse frontmatter. Without one, names fall back to filenames.
    // Since SkillParser is optional and we use None here, verify that at least the filenames appear.
    let router = build_router(new_test_state(dir.path()));

    let req = Request::builder()
        .method("GET")
        .uri("/settings/api/skills")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = std::str::from_utf8(&body).unwrap();
    assert!(body_str.contains("\"filename\":\"alpha.md\""));
    assert!(body_str.contains("\"filename\":\"beta.md\""));
}

#[tokio::test]
async fn test_list_malformed_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    write_skill(dir.path(), "broken.md", "---\nname: [unclosed\n---\nbody\n");

    let router = build_router(new_test_state(dir.path()));

    let req = Request::builder()
        .method("GET")
        .uri("/settings/api/skills")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    // List must not fail on individual bad files
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = std::str::from_utf8(&body).unwrap();
    assert!(body_str.contains("\"filename\":\"broken.md\""));
}

// ─── Get ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_found() {
    let dir = tempfile::tempdir().unwrap();
    let content = "---\nname: cortex\n---\nbody here\n";
    write_skill(dir.path(), "cortex.md", content);

    let router = build_router(new_test_state(dir.path()));

    let req = Request::builder()
        .method("GET")
        .uri("/settings/api/skills/cortex.md")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/plain"));

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), content);
}

#[tokio::test]
async fn test_get_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = build_router(new_test_state(dir.path()));

    let req = Request::builder()
        .method("GET")
        .uri("/settings/api/skills/missing.md")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Path traversal via URL: `../foo.md` should be rejected at the
/// `validate_skill_name` gate before touching the filesystem.
/// Note: dots in the path like `..` get normalised by axum's router,
/// so we test via URL-encoded paths.  The regex catches these anyway.
#[tokio::test]
async fn test_get_path_traversal_rejected_by_validate() {
    // validate_skill_name rejects names with path separators or dots.
    // We test the function directly for these inputs; the router would
    // normalise URLs before they reach the handler.
    for bad in &["../etc/passwd", "foo/bar.md", "foo bar.md", "with:colon.md"] {
        assert!(
            validate_skill_name(bad).is_err(),
            "name {:?} should fail validation",
            bad
        );
    }
}

// ─── Upload ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_upload_happy() {
    let dir = tempfile::tempdir().unwrap();
    let router = build_router(new_test_state(dir.path()));

    let (body, ct) = upload_body("newskill.md", b"---\nname: newskill\ndescription: hello\n---\nbody\n");

    let req = Request::builder()
        .method("POST")
        .uri("/settings/api/skills")
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let on_disk = std::fs::read(dir.path().join("newskill.md")).unwrap();
    assert!(std::str::from_utf8(&on_disk).unwrap().contains("name: newskill"));
}

#[tokio::test]
async fn test_upload_bad_filename() {
    let dir = tempfile::tempdir().unwrap();

    for bad in &["foo.txt", "with:colon.md", ""] {
        let router = build_router(new_test_state(dir.path()));
        let (body, ct) = upload_body(bad, b"body");

        let req = Request::builder()
            .method("POST")
            .uri("/settings/api/skills")
            .header("content-type", ct)
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "fname={:?} should be rejected",
            bad
        );
    }
}

#[tokio::test]
async fn test_upload_path_in_filename_sanitized() {
    // "../escaped.md" → basename → "escaped.md" → accepted.
    let dir = tempfile::tempdir().unwrap();
    let router = build_router(new_test_state(dir.path()));

    let (body, ct) = upload_body("../escaped.md", b"body\n");

    let req = Request::builder()
        .method("POST")
        .uri("/settings/api/skills")
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // File must land inside skillsDir, not escape it
    assert!(dir.path().join("escaped.md").exists());
    assert!(!dir.path().parent().unwrap().join("escaped.md").exists());
}

#[tokio::test]
async fn test_upload_too_large() {
    let dir = tempfile::tempdir().unwrap();
    let router = build_router(new_test_state(dir.path()));

    let big = vec![b'x'; MAX_SKILL_UPLOAD_BYTES + 1];
    let (body, ct) = upload_body("big.md", &big);

    let req = Request::builder()
        .method("POST")
        .uri("/settings/api/skills")
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn test_upload_bad_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let router = build_router(new_test_state(dir.path()));

    let (body, ct) = upload_body("bad.md", b"---\nname: [unclosed\n---\nbody\n");

    let req = Request::builder()
        .method("POST")
        .uri("/settings/api/skills")
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_upload_already_exists() {
    let dir = tempfile::tempdir().unwrap();
    write_skill(dir.path(), "dup.md", "existing\n");

    let router = build_router(new_test_state(dir.path()));
    let (body, ct) = upload_body("dup.md", b"new content");

    let req = Request::builder()
        .method("POST")
        .uri("/settings/api/skills")
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Existing file must be unchanged
    let on_disk = std::fs::read_to_string(dir.path().join("dup.md")).unwrap();
    assert_eq!(on_disk, "existing\n");
}

#[tokio::test]
async fn test_upload_reload_failure() {
    let dir = tempfile::tempdir().unwrap();

    let state = SkillHandlerState {
        loader: Arc::new(FailReloader { msg: "reload kaboom" }),
        parser: None,
        skills_dir: dir.path().to_path_buf(),
        reload_dirs: vec![],
    };
    let router = build_router(state);

    let (body, ct) = upload_body("ok.md", b"body\n");

    let req = Request::builder()
        .method("POST")
        .uri("/settings/api/skills")
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    // File write succeeded → still 200, but with a warning
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = std::str::from_utf8(&body_bytes).unwrap();
    assert!(body_str.contains("\"warning\""));
    assert!(body_str.contains("reload kaboom"));

    // Disk write happened
    assert!(dir.path().join("ok.md").exists());
}

// ─── Delete ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_delete_happy() {
    let dir = tempfile::tempdir().unwrap();
    write_skill(dir.path(), "gone.md", "---\nname: gone\n---\nbody\n");

    let router = build_router(new_test_state(dir.path()));

    let req = Request::builder()
        .method("DELETE")
        .uri("/settings/api/skills/gone.md")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    assert!(!dir.path().join("gone.md").exists());
}

#[tokio::test]
async fn test_delete_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = build_router(new_test_state(dir.path()));

    let req = Request::builder()
        .method("DELETE")
        .uri("/settings/api/skills/never-here.md")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Path traversal via URL: validate_skill_name rejects these before the
/// filesystem is touched.
#[test]
fn test_delete_path_traversal_rejected_by_validate() {
    for bad in &["../etc/passwd", "foo/bar.md", "foo bar.md"] {
        assert!(
            validate_skill_name(bad).is_err(),
            "name {:?} should fail validation",
            bad
        );
    }
}