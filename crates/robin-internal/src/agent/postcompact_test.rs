/// postcompact_test.rs — Tests for provider_supports_mid_loop_compaction,
/// record_file_touch, snapshot_touched_files, and extract_path_from_input.
///
/// Mirrors Go's postcompact_test.go.
use crate::agent::context::extract_path_from_input;
use crate::agent::test_support::minimal_runtime;

// ── provider_supports_mid_loop_compaction tests ───────────────────────────────

#[test]
fn test_provider_supports_mid_loop_compaction_matrix() {
    let cases: &[(&str, bool)] = &[
        ("anthropic", true),
        ("openai", true),
        ("gemini", true),
        ("local", false),
        ("ollama", false),
        ("", false),
        ("deepseek", false),
    ];
    for (provider, want) in cases {
        let mut rt = minimal_runtime();
        rt.provider = provider.to_string();
        assert_eq!(
            rt.provider_supports_mid_loop_compaction(),
            *want,
            "provider={:?}",
            provider
        );
    }
}

// ── record_file_touch + snapshot_touched_files tests ──────────────────────────

#[test]
fn test_record_file_touch_appends_and_dedupes_by_move_to_back() {
    let rt = minimal_runtime();
    rt.record_file_touch("a.go");
    rt.record_file_touch("b.go");
    rt.record_file_touch("c.go");
    rt.record_file_touch("a.go"); // re-touch — should move to back, not duplicate
    let got = rt.snapshot_touched_files();
    assert_eq!(got, vec!["b.go", "c.go", "a.go"]);
}

#[test]
fn test_record_file_touch_ignores_empty_path() {
    let rt = minimal_runtime();
    rt.record_file_touch("");
    rt.record_file_touch("x.go");
    rt.record_file_touch("");
    assert_eq!(rt.snapshot_touched_files(), vec!["x.go"]);
}

#[test]
fn test_snapshot_touched_files_returns_copy() {
    let rt = minimal_runtime();
    rt.record_file_touch("a.go");
    let mut snap = rt.snapshot_touched_files();
    snap[0] = "mutated".to_owned();
    // Mutation of the snapshot must not bleed back into the live slice.
    assert_eq!(rt.snapshot_touched_files(), vec!["a.go"]);
}

// ── extract_path_from_input tests ─────────────────────────────────────────────

#[test]
fn test_extract_path_from_input_happy_path() {
    let input = serde_json::json!({"path": "/tmp/foo.go"});
    assert_eq!(extract_path_from_input(&input), "/tmp/foo.go");
}

#[test]
fn test_extract_path_from_input_missing_field_returns_empty() {
    let input = serde_json::json!({"command": "ls"});
    assert_eq!(extract_path_from_input(&input), "");
}

#[test]
fn test_extract_path_from_input_null_returns_empty() {
    let input = serde_json::Value::Null;
    assert_eq!(extract_path_from_input(&input), "");
}