#[cfg(test)]
mod tests {
    use super::super::{LoadSkillTool, LoadMemoryTool};
    use crate::tools::tool::Tool;

    // ── LoadSkillTool ─────────────────────────────────────────────────────────

    #[test]
    fn test_load_skill_tool_returns_body() {
        let tool = LoadSkillTool::new(|name| {
            if name == "ffmpeg" { Some("FFMPEG_BODY".to_owned()) } else { None }
        });
        let input = serde_json::json!({"name": "ffmpeg"});
        let res = tool.execute(input).unwrap();
        assert_eq!(res.output, "FFMPEG_BODY");
        assert!(res.error.is_empty());
    }

    #[test]
    fn test_load_skill_tool_not_found_returns_error() {
        let tool = LoadSkillTool::new(|_| None);
        let input = serde_json::json!({"name": "ghost"});
        let res = tool.execute(input).unwrap();
        assert!(res.output.is_empty());
        assert!(res.error.contains("skill not found"), "error: {}", res.error);
        assert!(res.error.contains("ghost"), "error: {}", res.error);
    }

    #[test]
    fn test_load_skill_tool_missing_name_rejected() {
        let tool = LoadSkillTool::new(|_| Some("x".to_owned()));
        let res = tool.execute(serde_json::json!({})).unwrap();
        assert_eq!(res.error, "name is required");
    }

    #[test]
    fn test_load_skill_tool_nil_lookup() {
        let tool = LoadSkillTool { lookup: None };
        let input = serde_json::json!({"name": "x"});
        let res = tool.execute(input).unwrap();
        assert!(res.error.contains("no skill loader configured"), "error: {}", res.error);
    }

    #[test]
    fn test_load_skill_tool_invalid_json() {
        // simulate invalid input by passing a non-object (null name field)
        let tool = LoadSkillTool::new(|_| Some("x".to_owned()));
        // null means name is missing (parsed as None → empty)
        let res = tool.execute(serde_json::json!({"name": null})).unwrap();
        assert!(res.error.contains("name is required"), "error: {}", res.error);
    }

    #[test]
    fn test_load_skill_tool_empty_body() {
        let tool = LoadSkillTool::new(|_| Some(String::new()));
        let input = serde_json::json!({"name": "blank"});
        let res = tool.execute(input).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert!(res.output.contains("blank"), "output: {}", res.output);
        assert!(res.output.contains("no body content"), "output: {}", res.output);
    }

    #[test]
    fn test_load_skill_tool_name_and_description() {
        let tool = LoadSkillTool { lookup: None };
        assert_eq!(tool.name(), "load_skill");
        assert!(!tool.description().is_empty());
        assert!(tool.is_concurrency_safe(&serde_json::json!(null)));
    }

    // ── LoadMemoryTool ────────────────────────────────────────────────────────

    #[test]
    fn test_load_memory_tool_returns_body() {
        let tool = LoadMemoryTool::new(|id| {
            if id == "feedback_xyz" { Some("FEEDBACK_BODY".to_owned()) } else { None }
        });
        let input = serde_json::json!({"id": "feedback_xyz"});
        let res = tool.execute(input).unwrap();
        assert_eq!(res.output, "FEEDBACK_BODY");
    }

    #[test]
    fn test_load_memory_tool_not_found() {
        let tool = LoadMemoryTool::new(|_| None);
        let input = serde_json::json!({"id": "ghost"});
        let res = tool.execute(input).unwrap();
        assert!(res.error.contains("memory entry not found"), "error: {}", res.error);
    }

    #[test]
    fn test_load_memory_tool_missing_id_rejected() {
        let tool = LoadMemoryTool::new(|_| Some("x".to_owned()));
        let res = tool.execute(serde_json::json!({})).unwrap();
        assert_eq!(res.error, "id is required");
    }

    #[test]
    fn test_load_memory_tool_name_and_description() {
        let tool = LoadMemoryTool { lookup: None };
        assert_eq!(tool.name(), "load_memory");
        assert!(!tool.description().is_empty());
        assert!(tool.is_concurrency_safe(&serde_json::json!(null)));
    }
}
