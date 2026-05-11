/// streaming_test.rs — Tests for streaming_tools_enabled().
///
/// Mirrors the simple streaming flag tests from Go's streaming_test.go.
use crate::config::config::AgentLoopConfig;

use crate::agent::test_support::minimal_runtime;

// Serialize env-var tests so they don't race with each other.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_streaming_tools_enabled_default() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("ROBIN_STREAMING_TOOLS");
    assert!(!minimal_runtime().streaming_tools_enabled());
}

#[test]
fn test_streaming_tools_enabled_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_STREAMING_TOOLS", "1");
    assert!(minimal_runtime().streaming_tools_enabled());
    std::env::remove_var("ROBIN_STREAMING_TOOLS");
}

#[test]
fn test_streaming_tools_enabled_invalid_falls_back() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for v in &["0", "true", "True", "garbage", " 1 ", "01", "yes"] {
        std::env::set_var("ROBIN_STREAMING_TOOLS", *v);
        assert!(
            !minimal_runtime().streaming_tools_enabled(),
            "expected false for {:?}",
            v
        );
    }
    std::env::remove_var("ROBIN_STREAMING_TOOLS");
}

#[test]
fn test_runtime_streaming_tools_config_true_wins_over_env_unset() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("ROBIN_STREAMING_TOOLS");
    let mut rt = minimal_runtime();
    rt.agent_loop.streaming_tools = true;
    assert!(rt.streaming_tools_enabled());
}

#[test]
fn test_runtime_streaming_tools_config_true_wins_over_env_zero() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_STREAMING_TOOLS", "0");
    let mut rt = minimal_runtime();
    rt.agent_loop.streaming_tools = true;
    assert!(rt.streaming_tools_enabled(), "config=true even with env=0");
    std::env::remove_var("ROBIN_STREAMING_TOOLS");
}

#[test]
fn test_runtime_streaming_tools_config_false_falls_back_to_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_STREAMING_TOOLS", "1");
    let rt = minimal_runtime(); // streaming_tools == false
    assert!(rt.streaming_tools_enabled(), "env=1 should make it true");
    std::env::remove_var("ROBIN_STREAMING_TOOLS");
}

#[test]
fn test_runtime_streaming_tools_both_unset_is_off() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("ROBIN_STREAMING_TOOLS");
    let rt = minimal_runtime();
    assert!(!rt.streaming_tools_enabled());
}
