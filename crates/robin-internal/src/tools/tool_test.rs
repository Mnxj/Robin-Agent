#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::tools::tool::{Registry, Tool, ToolResult};

    struct StubTool {
        name: String,
    }

    impl Tool for StubTool {
        fn name(&self) -> &str { &self.name }
        fn description(&self) -> &str { "stub description" }
        fn parameters(&self) -> Value { serde_json::json!({}) }
        fn is_concurrency_safe(&self, _: &Value) -> bool { false }
        fn execute(&self, _: Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::default())
        }
    }

    #[test]
    fn test_tool_defs_are_deterministic() {
        let reg = Registry::new();
        for name in &["zebra", "alpha", "mango", "banana", "kiwi"] {
            reg.register(StubTool { name: name.to_string() });
        }

        let first = reg.tool_defs();
        for _ in 0..50 {
            let got = reg.tool_defs();
            assert_eq!(first.len(), got.len(), "length must be stable");
            for (a, b) in first.iter().zip(got.iter()) {
                assert_eq!(a.name, b.name, "position must be stable across calls");
            }
        }

        for i in 1..first.len() {
            assert!(
                first[i - 1].name <= first[i].name,
                "ToolDefs must be sorted by name: {} > {}",
                first[i - 1].name,
                first[i].name
            );
        }
    }

    #[test]
    fn test_names_are_deterministic() {
        let reg = Registry::new();
        for name in &["zebra", "alpha", "mango"] {
            reg.register(StubTool { name: name.to_string() });
        }

        let got = reg.names();
        assert_eq!(got, vec!["alpha", "mango", "zebra"], "Names() must return tools sorted by name");
    }
}