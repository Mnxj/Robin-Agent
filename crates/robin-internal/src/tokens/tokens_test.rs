#[cfg(test)]
mod tests {
    use crate::tokens::tokens::{
        context_window, context_window_for, estimate, Calibrator,
    };
    use crate::llm::{Message, ToolDef};

    fn make_msg(role: &str, content: &str) -> Message {
        Message { role: role.to_string(), content: content.to_string(), ..Default::default() }
    }

    #[test]
    fn test_estimate_basic() {
        let msgs = vec![make_msg("user", "hello world"), make_msg("assistant", "hi there")];
        let got = estimate(&msgs, "", &[]);
        assert!(got >= 9 && got <= 12, "got={}", got);
    }

    #[test]
    fn test_estimate_with_system_and_tools() {
        let msgs = vec![make_msg("user", "hi")];
        let tools = vec![ToolDef {
            name: "read_file".to_string(),
            description: "read a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            ..Default::default()
        }];
        let without = estimate(&msgs, "", &[]);
        let with_sys = estimate(&msgs, "you are a helpful assistant", &tools);
        assert!(with_sys > without);
    }

    #[test]
    fn test_context_window_known_models() {
        let cases = vec![
            ("anthropic/claude-3-5-sonnet-20241022", 200000),
            ("anthropic/claude-3-opus-20240229", 200000),
            ("openai/gpt-4o", 128000),
            ("openai/gpt-4-turbo", 128000),
            ("google/gemini-1.5-pro", 2000000),
            ("google/gemini-1.5-flash", 1000000),
        ];
        for (model, want) in cases {
            assert_eq!(context_window(model), want, "model={}", model);
        }
    }

    #[test]
    fn test_context_window_unknown_returns_fallback() {
        assert_eq!(context_window("weird/unknown-model"), 128000);
        assert_eq!(context_window(""), 128000);
        assert_eq!(context_window("openai-compatible/some-model"), 128000);
    }

    #[test]
    fn test_context_window_for_override_wins() {
        assert_eq!(context_window_for("anthropic/claude-3-opus", 64000), 64000);
        assert_eq!(context_window_for("anthropic/claude-3-opus", 0), 200000);
        assert_eq!(context_window_for("anthropic/claude-3-opus", -1), 200000);
    }

    #[test]
    fn test_context_window_proxy_provider_by_model_family() {
        let cases = vec![
            ("platformai/claude-sonnet-4-6-asia-southeast1", 200000),
            ("openrouter/anthropic/claude-3-opus", 200000),
            ("bedrock/anthropic.claude-3-haiku", 200000),
            ("vertex/gemini-1.5-pro-001", 2000000),
            ("openrouter/openai/gpt-4o-2024-08-06", 128000),
        ];
        for (model, want) in cases {
            assert_eq!(context_window(model), want, "model={}", model);
        }
    }

    #[test]
    fn test_calibrator_starts_at_one() {
        let c = Calibrator::new();
        assert_eq!(c.adjust(100), 100);
    }

    #[test]
    fn test_calibrator_converges_toward_actual() {
        let c = Calibrator::new();
        for _ in 0..5 {
            c.update(150, 100);
        }
        let got = c.adjust(100);
        assert!(got >= 148 && got <= 150, "got={}", got);
    }

    #[test]
    fn test_calibrator_ignores_zero_or_negative() {
        let c = Calibrator::new();
        c.update(0, 100);
        c.update(100, 0);
        assert_eq!(c.adjust(100), 100);
    }
}