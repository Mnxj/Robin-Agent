#[cfg(test)]
mod tests {
    use super::super::provider::{
        parse_provider_model, new_provider, ProviderOptions, LLMProvider,
        SystemPromptPart, join_system_prompt_parts,
    };
    use super::super::anthropic::AnthropicProvider;
    use super::super::openai::OpenAIProvider;
    use super::super::gemini::GeminiProvider;

    #[test]
    fn test_parse_provider_model() {
        let (p, m) = parse_provider_model("anthropic/claude-sonnet");
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet");

        let (p, m) = parse_provider_model("model-only");
        assert_eq!(p, "");
        assert_eq!(m, "model-only");

        let (p, m) = parse_provider_model("");
        assert_eq!(p, "");
        assert_eq!(m, "");

        let (p, m) = parse_provider_model("openai/gpt-4/turbo");
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4/turbo");
    }

    #[test]
    fn test_new_provider_anthropic() {
        let p = new_provider("anthropic", ProviderOptions { api_key: "test-key".into(), ..Default::default() });
        assert!(p.is_ok());
    }

    #[test]
    fn test_new_provider_openai() {
        let p = new_provider("openai", ProviderOptions { api_key: "test-key".into(), ..Default::default() });
        assert!(p.is_ok());
    }

    #[test]
    fn test_new_provider_openai_compatible() {
        let p = new_provider("openai-compatible", ProviderOptions {
            api_key: "test-key".into(),
            base_url: "http://localhost:8080/v1".into(),
            kind: "openai-compatible".into(),
            ca_bundle: String::new(),
        });
        assert!(p.is_ok());
    }

    #[test]
    fn test_new_provider_unknown() {
        let p = new_provider("unknown-provider", ProviderOptions::default());
        assert!(p.is_err());
        assert!(p.err().unwrap().to_string().contains("unknown LLM provider kind"));
    }

    #[test]
    fn test_new_provider_base_url_default() {
        let p = new_provider("anything", ProviderOptions {
            api_key: "test-key".into(),
            base_url: "http://localhost:11434/v1".into(),
            ..Default::default()
        });
        assert!(p.is_ok());
    }

    #[test]
    fn test_anthropic_provider_models() {
        let p = AnthropicProvider::new("test-key", "");
        let models = p.models();
        assert!(!models.is_empty());
        for m in &models {
            assert!(!m.id.is_empty());
            assert!(!m.name.is_empty());
            assert_eq!(m.provider, "anthropic");
        }
    }

    #[test]
    fn test_openai_provider_models() {
        let p = OpenAIProvider::new("test-key", "", "openai", "");
        let models = p.models();
        assert!(!models.is_empty());
        for m in &models {
            assert!(!m.id.is_empty());
            assert!(!m.name.is_empty());
            assert_eq!(m.provider, "openai");
        }
    }

    #[test]
    fn test_new_provider_gemini() {
        let p = new_provider("gemini", ProviderOptions { api_key: "test-key".into(), kind: "gemini".into(), ..Default::default() });
        assert!(p.is_ok());
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
    fn test_concat_system_prompt_parts() {
        let cases = vec![
            (vec![], ""),
            (vec![SystemPromptPart { text: "A".into(), cache: false }], "A"),
            (vec![
                SystemPromptPart { text: "A".into(), cache: false },
                SystemPromptPart { text: "B".into(), cache: false },
            ], "A\nB"),
            (vec![
                SystemPromptPart { text: "A".into(), cache: false },
                SystemPromptPart { text: "".into(), cache: false },
                SystemPromptPart { text: "B".into(), cache: false },
            ], "A\nB"),
            (vec![
                SystemPromptPart { text: "A".into(), cache: true },
                SystemPromptPart { text: "B".into(), cache: false },
            ], "A\nB"),
        ];
        for (parts, want) in cases {
            assert_eq!(join_system_prompt_parts(&parts), want);
        }
    }
}
