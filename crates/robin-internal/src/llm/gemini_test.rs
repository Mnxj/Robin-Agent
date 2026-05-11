#[cfg(test)]
mod tests {
    use super::super::gemini::{GeminiProvider, gemini_resolve_system_prompt};
    use super::super::provider::{ChatRequest, LLMProvider, ReasoningMode, SystemPromptPart};

    #[test]
    fn test_gemini_resolve_system_prompt_prefers_parts() {
        let req = ChatRequest {
            system_prompt: "legacy".into(),
            system_prompt_parts: vec![
                SystemPromptPart { text: "alpha".into(), cache: false },
                SystemPromptPart { text: "beta".into(), cache: false },
            ],
            ..Default::default()
        };
        assert_eq!(gemini_resolve_system_prompt(&req), "alpha\nbeta");
    }

    #[test]
    fn test_gemini_resolve_system_prompt_falls_back_to_string() {
        let req = ChatRequest {
            system_prompt: "only-string".into(),
            ..Default::default()
        };
        assert_eq!(gemini_resolve_system_prompt(&req), "only-string");
    }

    #[test]
    fn test_gemini_resolve_system_prompt_empty() {
        let req = ChatRequest::default();
        assert_eq!(gemini_resolve_system_prompt(&req), "");
    }

    #[test]
    fn test_gemini_provider_models() {
        let p = GeminiProvider::new("test-key", "");
        let models = p.models();
        assert!(!models.is_empty());
        for m in &models {
            assert!(!m.id.is_empty());
            assert!(!m.name.is_empty());
            assert_eq!(m.provider, "gemini");
        }
    }

    #[test]
    fn test_gemini_reasoning_off() {
        let p = GeminiProvider::new("test-key", "");
        let budget = p.build_thinking_budget("gemini-2.5-pro", ReasoningMode::Off);
        assert!(budget.is_none());
    }

    #[test]
    fn test_gemini_reasoning_levels() {
        let p = GeminiProvider::new("test-key", "");
        assert_eq!(p.build_thinking_budget("gemini-2.5-pro", ReasoningMode::Low), Some(1024));
        assert_eq!(p.build_thinking_budget("gemini-2.5-pro", ReasoningMode::Medium), Some(4096));
        assert_eq!(p.build_thinking_budget("gemini-2.5-pro", ReasoningMode::High), Some(16384));
    }

    #[test]
    fn test_gemini_reasoning_unsupported_model() {
        let p = GeminiProvider::new("test-key", "");
        let budget = p.build_thinking_budget("gemini-1.5-flash", ReasoningMode::High);
        assert!(budget.is_none());
    }

    #[test]
    fn test_gemini_reasoning_supports_thinking_families() {
        let p = GeminiProvider::new("test-key", "");
        for model in &["gemini-2.0-flash-thinking-exp-1219", "gemini-2.5-pro", "gemini-2.5-flash"] {
            let budget = p.build_thinking_budget(model, ReasoningMode::Medium);
            assert!(budget.is_some(), "model {} should support thinking", model);
        }
    }
}