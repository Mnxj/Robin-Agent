#[cfg(test)]
mod tests {
    use super::super::qwen::{QwenProvider, qwen_resolve_system_prompt};
    use super::super::provider::{ChatRequest, ReasoningMode, SystemPromptPart};

    #[test]
    fn test_qwen_resolve_system_prompt_prefers_parts() {
        let req = ChatRequest {
            system_prompt: "legacy".into(),
            system_prompt_parts: vec![
                SystemPromptPart { text: "new-a".into(), cache: false },
                SystemPromptPart { text: "new-b".into(), cache: false },
            ],
            ..Default::default()
        };
        assert_eq!(qwen_resolve_system_prompt(&req), "new-a\nnew-b");
    }

    #[test]
    fn test_qwen_resolve_system_prompt_fallback() {
        let req = ChatRequest {
            system_prompt: "legacy".into(),
            ..Default::default()
        };
        assert_eq!(qwen_resolve_system_prompt(&req), "legacy");
    }

    #[test]
    fn test_qwen_resolve_system_prompt_empty() {
        let req = ChatRequest::default();
        assert_eq!(qwen_resolve_system_prompt(&req), "");
    }

    #[test]
    fn test_qwen_reasoning_off() {
        let p = QwenProvider::new("fake", "");
        let result = p.build_enable_thinking("qwen3-32b", ReasoningMode::Off);
        assert!(result.is_none());
    }

    #[test]
    fn test_qwen_reasoning_clamps() {
        let p = QwenProvider::new("fake", "");
        for mode in &[ReasoningMode::Low, ReasoningMode::Medium, ReasoningMode::High] {
            let result = p.build_enable_thinking("qwen3-32b", *mode);
            assert!(result.is_some(), "mode should produce config");
            let (enabled, diag) = result.unwrap();
            assert!(enabled);
            assert_eq!(diag.action, "clamped");
            assert!(diag.reason.contains("boolean"));
        }
    }

    #[test]
    fn test_qwen_reasoning_unsupported_model() {
        let p = QwenProvider::new("fake", "");
        let result = p.build_enable_thinking("qwen-turbo", ReasoningMode::High);
        assert!(result.is_none());
    }

    #[test]
    fn test_qwen_reasoning_supports_known_thinking_models() {
        let p = QwenProvider::new("fake", "");
        for model in &["qwen-qwq-32b", "qwen3-32b", "qwen3-coder-30b"] {
            let result = p.build_enable_thinking(model, ReasoningMode::Medium);
            assert!(result.is_some(), "model {} should support thinking", model);
        }
    }
}