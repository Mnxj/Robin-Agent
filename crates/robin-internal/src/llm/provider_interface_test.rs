#[cfg(test)]
mod tests {
    use super::super::provider::{
        ChatRequest, Diagnostic, LLMProvider, ReasoningMode, ToolDef,
        parse_reasoning_mode,
    };
    use super::super::anthropic::AnthropicProvider;
    use super::super::openai::OpenAIProvider;
    use super::super::gemini::GeminiProvider;
    use super::super::qwen::QwenProvider;
    use super::super::llmtest::Stub;

    #[test]
    fn test_llm_provider_interface_has_normalize_tool_schema() {
        let s = Stub::default();
        let tools = vec![ToolDef {
            name: "x".into(),
            description: "y".into(),
            parameters: serde_json::json!({}),
        }];
        let (out, diags) = s.normalize_tool_schema(tools.clone());
        assert_eq!(out.len(), tools.len());
        assert!(diags.is_empty());
    }

    #[test]
    fn test_diagnostic_fields() {
        let d = Diagnostic {
            tool_name: "read_file".into(),
            field: "properties.url.format".into(),
            action: "stripped".into(),
            reason: "gemini does not support format".into(),
        };
        assert_eq!(d.tool_name, "read_file");
        assert_eq!(d.action, "stripped");
    }

    #[test]
    fn test_anthropic_normalize_tool_schema_is_identity() {
        let p = AnthropicProvider::new("fake-key", "");
        let tools = vec![ToolDef {
            name: "complex".into(),
            description: "".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "format": "uri"},
                    "items": {"oneOf": [{"type": "string"}, {"type": "number"}]}
                },
                "$ref": "#/defs/x",
                "definitions": {"x": {"type": "string"}}
            }),
        }];
        let (out, diags) = p.normalize_tool_schema(tools.clone());
        assert_eq!(out[0].parameters, tools[0].parameters, "Anthropic accepts full draft-7; nothing stripped");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_openai_normalize_tool_schema_strips_ref() {
        let p = OpenAIProvider::new("fake-key", "", "openai", "");
        let tools = vec![ToolDef {
            name: "lookup".into(),
            description: "".into(),
            parameters: serde_json::json!({
                "type": "object",
                "$ref": "#/defs/foo",
                "definitions": {"foo": {"type": "string"}},
                "properties": {"q": {"type": "string"}}
            }),
        }];
        let (out, diags) = p.normalize_tool_schema(tools);
        assert_eq!(out.len(), 1);
        let schema = out[0].parameters.as_object().unwrap();
        assert!(!schema.contains_key("$ref"), "$ref must be stripped");
        assert!(!schema.contains_key("definitions"), "definitions must be stripped");
        assert!(diags.len() >= 2, "expected diagnostics for $ref and definitions");
        let fields: Vec<&str> = diags.iter().map(|d| d.field.as_str()).collect();
        assert!(fields.contains(&"$ref"));
        assert!(fields.contains(&"definitions"));
        for d in &diags {
            assert_eq!(d.tool_name, "lookup");
            assert_eq!(d.action, "stripped");
        }
    }

    #[test]
    fn test_openai_normalize_keeps_any_of() {
        let p = OpenAIProvider::new("fake-key", "", "openai", "");
        let tools = vec![ToolDef {
            name: "x".into(),
            description: "".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "v": {"anyOf": [{"type": "string"}, {"type": "number"}]},
                    "u": {"oneOf": [{"type": "string"}, {"type": "null"}]},
                    "f": {"type": "string", "format": "uri"}
                }
            }),
        }];
        let (out, diags) = p.normalize_tool_schema(tools.clone());
        assert!(diags.is_empty(), "OpenAI accepts anyOf/oneOf/format; nothing should be stripped");
        assert_eq!(out[0].parameters, tools[0].parameters);
    }

    #[test]
    fn test_qwen_normalize_tool_schema_strips_ref() {
        let p = QwenProvider::new("fake-key", "");
        let tools = vec![ToolDef {
            name: "lookup".into(),
            description: "".into(),
            parameters: serde_json::json!({
                "$ref": "#/x",
                "definitions": {"x": {"type": "string"}},
                "properties": {"q": {"type": "string"}}
            }),
        }];
        let (out, diags) = p.normalize_tool_schema(tools);
        assert_eq!(out.len(), 1);
        let schema = out[0].parameters.as_object().unwrap();
        assert!(!schema.contains_key("$ref"));
        assert!(!schema.contains_key("definitions"));
        assert_eq!(diags.len(), 2);
        for d in &diags {
            assert_eq!(d.tool_name, "lookup");
            assert_eq!(d.action, "stripped");
        }
    }

    #[test]
    fn test_gemini_normalize_tool_schema_strips_all() {
        let p = GeminiProvider::new("fake-key", "");
        let tools = vec![ToolDef {
            name: "fetch".into(),
            description: "".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "format": "uri"},
                    "alt": {"anyOf": [{"type": "string"}, {"type": "null"}]},
                    "choice": {"oneOf": [{"type": "string"}, {"type": "number"}]},
                    "exclude": {"not": {"type": "boolean"}}
                },
                "$ref": "#/defs/x"
            }),
        }];
        let (out, diags) = p.normalize_tool_schema(tools);
        assert_eq!(out.len(), 1);
        let schema = out[0].parameters.as_object().unwrap();
        assert!(!schema.contains_key("$ref"), "$ref must be stripped at root");

        let props = schema["properties"].as_object().unwrap();
        assert!(!props["url"].as_object().unwrap().contains_key("format"));
        assert!(!props["alt"].as_object().unwrap().contains_key("anyOf"));
        assert!(!props["choice"].as_object().unwrap().contains_key("oneOf"));
        assert!(!props["exclude"].as_object().unwrap().contains_key("not"));

        assert_eq!(diags.len(), 5);
        for d in &diags {
            assert_eq!(d.tool_name, "fetch");
            assert_eq!(d.action, "stripped");
        }
    }

    #[test]
    fn test_reasoning_mode_constants() {
        assert_eq!(ReasoningMode::Off, ReasoningMode::Off);
        assert_eq!(ReasoningMode::Low, ReasoningMode::Low);
        assert_eq!(ReasoningMode::Medium, ReasoningMode::Medium);
        assert_eq!(ReasoningMode::High, ReasoningMode::High);
    }

    #[test]
    fn test_chat_request_reasoning_zero_value() {
        let req = ChatRequest::default();
        assert_eq!(req.reasoning, ReasoningMode::Off);
    }

    #[test]
    fn test_parse_reasoning_mode() {
        let cases = vec![
            ("", ReasoningMode::Off),
            ("off", ReasoningMode::Off),
            ("low", ReasoningMode::Low),
            ("medium", ReasoningMode::Medium),
            ("high", ReasoningMode::High),
        ];
        for (input, want) in cases {
            let got = parse_reasoning_mode(input).unwrap();
            assert_eq!(got, want, "input {:?}", input);
        }
        assert!(parse_reasoning_mode("ultra").is_err());
        assert!(parse_reasoning_mode("LOW").is_err(), "case-sensitive: uppercase must error");
    }

    #[test]
    fn test_anthropic_reasoning_off() {
        let p = AnthropicProvider::new("fake", "");
        let cfg = p.build_thinking_config("claude-sonnet-4-5", ReasoningMode::Off);
        assert!(cfg.is_none());
    }

    #[test]
    fn test_anthropic_reasoning_levels() {
        let p = AnthropicProvider::new("fake", "");
        let cases = vec![
            (ReasoningMode::Low, 1024i64),
            (ReasoningMode::Medium, 4096),
            (ReasoningMode::High, 16384),
        ];
        for (mode, want_budget) in cases {
            let cfg = p.build_thinking_config("claude-sonnet-4-5", mode);
            assert!(cfg.is_some(), "mode should produce config");
            assert_eq!(cfg.unwrap().budget_tokens, want_budget);
        }
    }

    #[test]
    fn test_anthropic_reasoning_unsupported_model() {
        let p = AnthropicProvider::new("fake", "");
        let cfg = p.build_thinking_config("claude-3-haiku-20240307", ReasoningMode::High);
        assert!(cfg.is_none());
    }

    #[test]
    fn test_anthropic_reasoning_unknown_model_defaults_supported() {
        let p = AnthropicProvider::new("fake", "");
        let cfg = p.build_thinking_config("claude-future-model-vNEW", ReasoningMode::Medium);
        assert!(cfg.is_some());
        assert_eq!(cfg.unwrap().budget_tokens, 4096);
    }

    #[test]
    fn test_openai_reasoning_off() {
        let p = OpenAIProvider::new("fake", "", "openai", "");
        let effort = p.build_reasoning_effort("o3-mini", ReasoningMode::Off);
        assert!(effort.is_none());
    }

    #[test]
    fn test_openai_reasoning_levels() {
        let p = OpenAIProvider::new("fake", "", "openai", "");
        let cases = vec![
            (ReasoningMode::Low, "low"),
            (ReasoningMode::Medium, "medium"),
            (ReasoningMode::High, "high"),
        ];
        for (mode, want) in cases {
            let effort = p.build_reasoning_effort("o3-mini", mode);
            assert!(effort.is_some(), "mode should produce effort");
            assert_eq!(effort.unwrap(), want);
        }
    }

    #[test]
    fn test_openai_reasoning_unsupported_model() {
        let p = OpenAIProvider::new("fake", "", "openai", "");
        let effort = p.build_reasoning_effort("gpt-4o", ReasoningMode::High);
        assert!(effort.is_none());
    }

    #[test]
    fn test_openai_compatible_suppresses_reasoning() {
        let p = OpenAIProvider::new("", "http://localhost:11434/v1", "openai-compatible", "");
        let effort = p.build_reasoning_effort("gpt-5-thinking", ReasoningMode::High);
        assert!(effort.is_none());
    }


    #[test]
    fn test_openai_default_constructor_is_openai_kind() {
        let p = OpenAIProvider::new("fake", "", "openai", "");
        let effort = p.build_reasoning_effort("o3-mini", ReasoningMode::Low);
        assert!(effort.is_some());
    }

    #[test]
    fn test_gemini_reasoning_off() {
        let p = GeminiProvider::new("fake", "");
        let budget = p.build_thinking_budget("gemini-2.5-pro", ReasoningMode::Off);
        assert!(budget.is_none());
    }

    #[test]
    fn test_gemini_reasoning_levels() {
        let p = GeminiProvider::new("fake", "");
        let cases: Vec<(ReasoningMode, i32)> = vec![
            (ReasoningMode::Low, 1024),
            (ReasoningMode::Medium, 4096),
            (ReasoningMode::High, 16384),
        ];
        for (mode, want) in cases {
            let budget = p.build_thinking_budget("gemini-2.5-pro", mode);
            assert_eq!(budget, Some(want));
        }
    }

    #[test]
    fn test_gemini_reasoning_unsupported_model() {
        let p = GeminiProvider::new("fake", "");
        let budget = p.build_thinking_budget("gemini-1.5-flash", ReasoningMode::High);
        assert!(budget.is_none());
    }

    #[test]
    fn test_gemini_reasoning_supports_thinking_families() {
        let p = GeminiProvider::new("fake", "");
        for model in &["gemini-2.0-flash-thinking-exp-1219", "gemini-2.5-pro", "gemini-2.5-flash"] {
            let budget = p.build_thinking_budget(model, ReasoningMode::Medium);
            assert!(budget.is_some(), "model {} should support thinking", model);
        }
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
            assert!(result.is_some());
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
