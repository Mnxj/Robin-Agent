/// builder_test.rs — Tests for build_runtime_for_agent.
///
/// Mirrors Go's builder_test.go.
use std::sync::Arc;

use crate::config::config::{AgentConfig, Config};

use super::super::builder::{build_runtime_for_agent, RuntimeDeps, RuntimeInputs};

fn agent_cfg(id: &str, name: &str, model: &str) -> AgentConfig {
    AgentConfig {
        id: id.to_owned(),
        name: name.to_owned(),
        model: model.to_owned(),
        ..Default::default()
    }
}

#[test]
fn test_build_runtime_for_agent_sets_provider_and_static_prompt() {
    let mut cfg = Config::default();
    cfg.agents.list = vec![AgentConfig {
        id: "a".to_owned(),
        name: "A".to_owned(),
        model: "anthropic/claude-sonnet-4-5".to_owned(),
        ..Default::default()
    }];
    cfg.channels.cli.enabled = true;

    let a = &cfg.agents.list[0].clone();
    let deps = RuntimeDeps {
        config: Some(Arc::new(cfg)),
        ..Default::default()
    };
    let rt = build_runtime_for_agent(deps, RuntimeInputs::default(), a).unwrap();
    assert_eq!(rt.provider, "anthropic");
    assert_eq!(rt.model, "claude-sonnet-4-5");
    assert!(!rt.static_system_prompt.is_empty());
    assert!(rt.static_system_prompt.contains("\"A\" agent (id: a)"));
    assert!(rt.static_system_prompt.contains("Configured channels: cli"));
}

#[test]
fn test_build_runtime_for_agent_local_provider() {
    let a = agent_cfg("x", "X", "local/qwen2.5:3b");
    let rt = build_runtime_for_agent(RuntimeDeps::default(), RuntimeInputs::default(), &a).unwrap();
    assert_eq!(rt.provider, "local");
}

#[test]
fn test_build_runtime_for_agent_nil_config_safe() {
    let a = agent_cfg("a", "A", "anthropic/claude-sonnet-4-5");
    let rt = build_runtime_for_agent(RuntimeDeps::default(), RuntimeInputs::default(), &a).unwrap();
    assert_eq!(rt.provider, "anthropic");
    assert!(!rt.static_system_prompt.is_empty());
}

#[test]
fn test_build_runtime_for_agent_loads_memory_files_into_static_prompt() {
    let workspace = tempfile::tempdir().unwrap();
    let memory_path = workspace.path().join("ROBIN.md");
    std::fs::write(&memory_path, "MEMFILE_END_TO_END_SENTINEL").unwrap();

    let a = AgentConfig {
        id: "a".to_owned(),
        name: "A".to_owned(),
        workspace: workspace.path().to_string_lossy().into_owned(),
        model: "anthropic/claude-sonnet-4-5".to_owned(),
        ..Default::default()
    };
    let rt = build_runtime_for_agent(RuntimeDeps::default(), RuntimeInputs::default(), &a).unwrap();
    assert!(rt.static_system_prompt.contains("MEMFILE_END_TO_END_SENTINEL"));
    assert!(rt.static_system_prompt.contains("## Project memory:"));
}

#[test]
fn test_build_runtime_for_agent_injects_memory_index_from_inputs() {
    let a = agent_cfg("a", "A", "anthropic/claude-sonnet-4-5");
    let mut inputs = RuntimeInputs::default();
    inputs.memory_index = "\n\n## Memory Index\n\n- **m1** — test entry\n".to_string();
    let rt = build_runtime_for_agent(RuntimeDeps::default(), inputs, &a).unwrap();
    assert!(rt.static_system_prompt.contains("## Memory Index"));
    assert!(rt.static_system_prompt.contains("**m1**"));
}
