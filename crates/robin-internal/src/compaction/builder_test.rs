use std::collections::HashMap;

use crate::config::config::{
    AgentConfig, AgentsConfig, AgentsDefaults, CompactionConfig, Config, ProviderConfig,
};

use super::build_manager;

#[test]
fn test_build_manager_wires_threshold() {
    let mut providers = HashMap::new();
    providers.insert(
        "local".to_string(),
        ProviderConfig {
            kind: "openai-compatible".to_string(),
            base_url: "http://127.0.0.1:11434/v1".to_string(),
            api_key: String::new(),
            ca_bundle: String::new(),
        },
    );

    let cfg = Config {
        providers,
        agents: AgentsConfig {
            list: vec![AgentConfig {
                id: "default".to_string(),
                model: "local/qwen2.5:3b-instruct".to_string(),
                ..Default::default()
            }],
            defaults: AgentsDefaults {
                compaction: CompactionConfig {
                    enabled: true,
                    model: "local/qwen2.5:3b-instruct".to_string(),
                    threshold: 0.42,
                    preserve_turns: 4,
                    timeout_sec: 60,
                    message_cap: 0,
                },
            },
        },
        ..Default::default()
    };

    let mgr = build_manager(&cfg);
    assert!(mgr.is_some(), "expected manager to be built");
    let mgr = mgr.unwrap();

    let diff = (mgr.threshold - 0.42f64).abs();
    assert!(diff < 0.001, "threshold mismatch: got {}", mgr.threshold);
    assert_eq!(mgr.preserve_turns, 4);
}
