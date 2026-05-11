#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::llm::ToolDef;

    use super::super::{DecisionBehavior, Policy, PermissionChecker, StaticChecker};

    fn make_def(name: &str) -> ToolDef {
        ToolDef { name: name.to_owned(), description: String::new(), parameters: serde_json::json!({}) }
    }

    #[test]
    fn test_static_checker_allows_listed_tool() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy {
            allow: vec!["read_file".to_owned(), "bash".to_owned()],
            deny: vec![],
        });
        let c = StaticChecker::new(map);
        let d = c.check("agent1", "read_file", &serde_json::json!({}));
        assert_eq!(d.behavior, DecisionBehavior::Allow);
        assert!(d.reason.is_empty());
    }

    #[test]
    fn test_static_checker_denies_unlisted_tool() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy {
            allow: vec!["read_file".to_owned()],
            deny: vec![],
        });
        let c = StaticChecker::new(map);
        let d = c.check("agent1", "bash", &serde_json::json!({}));
        assert_eq!(d.behavior, DecisionBehavior::Deny);
        assert!(d.reason.contains("bash"));
        assert!(d.reason.contains("agent1"));
    }

    #[test]
    fn test_static_checker_denies_explicitly_denied_tool() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy {
            allow: vec![],
            deny: vec!["bash".to_owned()],
        });
        let c = StaticChecker::new(map);
        let d = c.check("agent1", "bash", &serde_json::json!({}));
        assert_eq!(d.behavior, DecisionBehavior::Deny);
    }

    #[test]
    fn test_static_checker_unknown_agent_defaults_to_allow() {
        let c = StaticChecker::new(HashMap::new());
        let d = c.check("agent_unknown", "bash", &serde_json::json!({}));
        assert_eq!(d.behavior, DecisionBehavior::Allow);
    }

    #[test]
    fn test_static_checker_nil_map_allow_all() {
        let c = StaticChecker::new(HashMap::new());
        let d = c.check("any", "any", &serde_json::json!(null));
        assert_eq!(d.behavior, DecisionBehavior::Allow);
    }

    #[test]
    fn test_filter_tool_defs_unknown_agent_returns_full_list() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy {
            allow: vec!["read_file".to_owned()],
            deny: vec![],
        });
        let c = StaticChecker::new(map);
        let defs = vec![make_def("read_file"), make_def("bash")];
        let out = c.filter_tool_defs(&defs, "unknown_agent");
        assert_eq!(out.len(), 2, "unknown agent must see the full toolset");
    }

    #[test]
    fn test_filter_tool_defs_allow_list() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy {
            allow: vec!["read_file".to_owned(), "bash".to_owned()],
            deny: vec![],
        });
        let c = StaticChecker::new(map);
        let defs = vec![make_def("read_file"), make_def("write_file"), make_def("bash")];
        let out = c.filter_tool_defs(&defs, "agent1");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "read_file");
        assert_eq!(out[1].name, "bash");
    }

    #[test]
    fn test_filter_tool_defs_deny_list() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy {
            allow: vec![],
            deny: vec!["bash".to_owned()],
        });
        let c = StaticChecker::new(map);
        let defs = vec![make_def("read_file"), make_def("bash"), make_def("web_fetch")];
        let out = c.filter_tool_defs(&defs, "agent1");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "read_file");
        assert_eq!(out[1].name, "web_fetch");
    }

    #[test]
    fn test_filter_tool_defs_empty_input() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy {
            allow: vec!["read_file".to_owned()],
            deny: vec![],
        });
        let c = StaticChecker::new(map);
        let out = c.filter_tool_defs(&[], "agent1");
        assert!(out.is_empty());
    }

    #[test]
    fn test_filter_tool_defs_order_preserved() {
        let mut map = HashMap::new();
        map.insert("agent1".to_owned(), Policy::default()); // no policy → allow-all
        let c = StaticChecker::new(map);
        let defs = vec![make_def("z_tool"), make_def("a_tool"), make_def("m_tool")];
        let out = c.filter_tool_defs(&defs, "agent1");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].name, "z_tool");
        assert_eq!(out[1].name, "a_tool");
        assert_eq!(out[2].name, "m_tool");
    }
}