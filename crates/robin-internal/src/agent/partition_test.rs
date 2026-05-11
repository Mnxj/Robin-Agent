/// partition_test.rs — Tests for partition_tool_calls and max_tool_concurrency.
///
/// Mirrors Go's partition_test.go.
use std::collections::HashMap;

use crate::llm::ToolCall;

use crate::agent::test_support::{ClassifyExecutor, minimal_runtime};
use super::{is_call_concurrency_safe, partition_tool_calls};

fn tc(name: &str) -> ToolCall {
    ToolCall {
        id: format!("tc_{}", name),
        name: name.to_owned(),
        input: serde_json::json!({}),
    }
}

fn safe_executor(names: &[&str]) -> ClassifyExecutor {
    let safe = names.iter().map(|n| (n.to_string(), true)).collect();
    ClassifyExecutor {
        safe,
        panics: HashMap::new(),
    }
}

fn unsafe_executor(names: &[&str]) -> ClassifyExecutor {
    let safe = names.iter().map(|n| (n.to_string(), false)).collect();
    ClassifyExecutor {
        safe,
        panics: HashMap::new(),
    }
}

#[test]
fn test_partition_empty() {
    let ex = safe_executor(&[]);
    assert!(partition_tool_calls(&[], &ex).is_empty());
    assert!(partition_tool_calls(&[], &ex).is_empty());
}

#[test]
fn test_partition_all_safe() {
    let ex = safe_executor(&["r"]);
    let calls = vec![tc("r"), tc("r"), tc("r")];
    let batches = partition_tool_calls(&calls, &ex);
    assert_eq!(batches.len(), 1);
    assert!(batches[0].concurrency_safe);
    assert_eq!(batches[0].calls.len(), 3);
}

#[test]
fn test_partition_all_unsafe() {
    let ex = unsafe_executor(&["w"]);
    let calls = vec![tc("w"), tc("w"), tc("w")];
    let batches = partition_tool_calls(&calls, &ex);
    assert_eq!(batches.len(), 3);
    for b in &batches {
        assert!(!b.concurrency_safe);
        assert_eq!(b.calls.len(), 1);
    }
}

#[test]
fn test_partition_mixed() {
    // [safe, safe, unsafe, safe] → 3 batches: [{safe,2}, {unsafe,1}, {safe,1}]
    let mut safe_map = HashMap::new();
    safe_map.insert("r".to_owned(), true);
    safe_map.insert("w".to_owned(), false);
    let ex = ClassifyExecutor { safe: safe_map, panics: HashMap::new() };
    let calls = vec![tc("r"), tc("r"), tc("w"), tc("r")];
    let batches = partition_tool_calls(&calls, &ex);
    assert_eq!(batches.len(), 3);
    assert!(batches[0].concurrency_safe);
    assert_eq!(batches[0].calls.len(), 2);
    assert!(!batches[1].concurrency_safe);
    assert_eq!(batches[1].calls.len(), 1);
    assert!(batches[2].concurrency_safe);
    assert_eq!(batches[2].calls.len(), 1);
}

#[test]
fn test_partition_tool_not_found_is_unsafe() {
    let ex = safe_executor(&[]); // no tools registered
    let batches = partition_tool_calls(&[tc("missing")], &ex);
    assert_eq!(batches.len(), 1);
    assert!(!batches[0].concurrency_safe, "unknown tool must be treated as unsafe");
}

#[test]
fn test_partition_panic_is_recovered_as_unsafe() {
    let mut safe_map = HashMap::new();
    safe_map.insert("p".to_owned(), true); // would be safe...
    let mut panics_map = HashMap::new();
    panics_map.insert("p".to_owned(), true); // ...but IsConcurrencySafe panics
    let ex = ClassifyExecutor { safe: safe_map, panics: panics_map };
    let batches = partition_tool_calls(&[tc("p")], &ex);
    assert_eq!(batches.len(), 1);
    assert!(!batches[0].concurrency_safe, "panic must be recovered and treated as unsafe");
}

// ── max_tool_concurrency tests ────────────────────────────────────────────────

// Serialize env-var tests so they don't race with each other.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_max_tool_concurrency_default() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
    assert_eq!(minimal_runtime().max_tool_concurrency(), 10);
}

#[test]
fn test_max_tool_concurrency_env_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_TOOL_CONCURRENCY", "3");
    assert_eq!(minimal_runtime().max_tool_concurrency(), 3);
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
}

#[test]
fn test_max_tool_concurrency_invalid_env_falls_back() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_TOOL_CONCURRENCY", "garbage");
    assert_eq!(minimal_runtime().max_tool_concurrency(), 10);
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
}

#[test]
fn test_max_tool_concurrency_zero_falls_back() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_TOOL_CONCURRENCY", "0");
    assert_eq!(
        minimal_runtime().max_tool_concurrency(),
        10,
        "0 is invalid; fall back to default"
    );
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
}

#[test]
fn test_runtime_max_tool_concurrency_config_wins_over_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_TOOL_CONCURRENCY", "7");
    let mut rt = minimal_runtime();
    rt.agent_loop.max_tool_concurrency = 4;
    assert_eq!(rt.max_tool_concurrency(), 4, "config should win over env");
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
}

#[test]
fn test_runtime_max_tool_concurrency_env_when_config_zero() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_TOOL_CONCURRENCY", "7");
    let rt = minimal_runtime(); // max_tool_concurrency == 0
    assert_eq!(rt.max_tool_concurrency(), 7, "env should fill in when config is zero");
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
}

#[test]
fn test_runtime_max_tool_concurrency_default_when_both_unset() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
    let rt = minimal_runtime();
    assert_eq!(rt.max_tool_concurrency(), 10, "default 10 when neither set");
}

#[test]
fn test_runtime_max_tool_concurrency_config_zero_or_negative_falls_back_to_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for v in &[0i32, -1, -10] {
        std::env::set_var("ROBIN_MAX_TOOL_CONCURRENCY", "9");
        let mut rt = minimal_runtime();
        rt.agent_loop.max_tool_concurrency = *v;
        assert_eq!(
            rt.max_tool_concurrency(),
            9,
            "config={} should fall back to env",
            v
        );
    }
    std::env::remove_var("ROBIN_MAX_TOOL_CONCURRENCY");
}
