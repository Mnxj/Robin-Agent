#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::startup::start_gateway;

    #[test]
    fn test_start_gateway_smoke() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path().to_str().unwrap());

        let config_path = tmp.path().join("robin.json5");
        let workspace = tmp.path().join("workspace");
        let cfg_content = format!(
            r#"{{
  "gateway": {{"host": "127.0.0.1", "port": 0}},
  "providers": {{}},
  "agents": {{
    "list": [
      {{
        "id": "default",
        "name": "Robin",
        "model": "anthropic/claude-sonnet-4-6",
        "workspace": "{}",
        "sandbox": "none"
      }}
    ]
  }},
  "channels": {{"cli": {{"enabled": false}}}},
  "memory": {{"enabled": false}},
  "cortex": {{"enabled": false}},
  "local": {{"enabled": false}}
}}"#,
            workspace.to_str().unwrap()
        );
        std::fs::write(&config_path, &cfg_content).unwrap();

        let result = start_gateway(config_path.to_str().unwrap(), "test-version").unwrap();

        assert_eq!(result.config.gateway.host, "127.0.0.1");
        assert_eq!(result.config.agents.list.len(), 1);
        assert_eq!(result.config.agents.list[0].id, "default");

        // Cleanup must not panic
        (result.cleanup)();
    }
}