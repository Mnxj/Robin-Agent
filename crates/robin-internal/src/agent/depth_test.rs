/// depth_test.rs — Tests for Runtime::max_agent_depth().
///
/// Mirrors Go's depth_test.go.

// depth_test is a submodule of `depth`, so we need to go up two levels
// to reach agent::test_support.
use crate::agent::test_support::minimal_runtime;

// Serialize env-var tests so they don't race with each other.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_max_agent_depth_default() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("ROBIN_MAX_AGENT_DEPTH");
    assert_eq!(minimal_runtime().max_agent_depth(), 3);
}

#[test]
fn test_max_agent_depth_env_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_AGENT_DEPTH", "5");
    assert_eq!(minimal_runtime().max_agent_depth(), 5);
    std::env::remove_var("ROBIN_MAX_AGENT_DEPTH");
}

#[test]
fn test_max_agent_depth_invalid_falls_back() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for val in &["garbage", "0", "-1", "1.5"] {
        std::env::set_var("ROBIN_MAX_AGENT_DEPTH", val);
        assert_eq!(
            minimal_runtime().max_agent_depth(),
            3,
            "env={:?}: expected default 3",
            val
        );
    }
    std::env::remove_var("ROBIN_MAX_AGENT_DEPTH");
}

#[test]
fn test_runtime_max_agent_depth_config_wins_over_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_AGENT_DEPTH", "7");
    let mut rt = minimal_runtime();
    rt.agent_loop.max_agent_depth = 5;
    assert_eq!(rt.max_agent_depth(), 5, "config wins over env");
    std::env::remove_var("ROBIN_MAX_AGENT_DEPTH");
}

#[test]
fn test_runtime_max_agent_depth_env_when_config_zero() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("ROBIN_MAX_AGENT_DEPTH", "7");
    let rt = minimal_runtime(); // agent_loop.max_agent_depth == 0
    assert_eq!(rt.max_agent_depth(), 7, "env fills in when config is zero");
    std::env::remove_var("ROBIN_MAX_AGENT_DEPTH");
}

#[test]
fn test_runtime_max_agent_depth_default_when_both_unset() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("ROBIN_MAX_AGENT_DEPTH");
    let rt = minimal_runtime();
    assert_eq!(rt.max_agent_depth(), 3, "default 3 when neither set");
}

#[test]
fn test_runtime_max_agent_depth_config_zero_or_negative_falls_back_to_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for v in &[0i32, -1, -10] {
        std::env::set_var("ROBIN_MAX_AGENT_DEPTH", "9");
        let mut rt = minimal_runtime();
        rt.agent_loop.max_agent_depth = *v;
        assert_eq!(
            rt.max_agent_depth(),
            9,
            "config={}: should fall back to env",
            v
        );
    }
    std::env::remove_var("ROBIN_MAX_AGENT_DEPTH");
}