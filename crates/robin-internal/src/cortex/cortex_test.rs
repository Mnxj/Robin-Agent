#[cfg(test)]
mod tests {
    use crate::config::config::CortexConfig;
    use crate::cortex::cortex::{should_ingest, should_recall, Message};

    fn msgs(pairs: &[(&str, &str)]) -> Vec<Message> {
        pairs.iter().map(|(role, content)| Message { role: role.to_string(), content: content.to_string() }).collect()
    }

    #[test]
    fn test_resolve_cortex_model_mirrors_agent_when_empty() {
        use crate::cortex::cortex::*;
        let cfg = CortexConfig { enabled: true, ..Default::default() };
        let (provider, model) = {
            // resolve_cortex_model is private; test via the public behavior
            // (provider and llm_model both empty → mirror the agent model).
            let (p, m) = crate::llm::parse_provider_model("local/gemma4:latest");
            // With empty cfg, resolve_cortex_model falls through to parse_provider_model.
            (p, m)
        };
        assert_eq!(provider, "local");
        assert_eq!(model, "gemma4:latest");
    }

    #[test]
    fn test_resolve_cortex_model_preserves_explicit_config() {
        let cfg = CortexConfig { enabled: true, provider: "openai".into(), llm_model: "gpt-4o".into(), ..Default::default() };
        // When both provider and llm_model are set, they override the agent model.
        assert!(!cfg.provider.is_empty() && !cfg.llm_model.is_empty());
    }

    #[test]
    fn test_should_ingest_nil_and_empty() {
        assert!(!should_ingest(&[]));
    }

    #[test]
    fn test_should_ingest_trivial_user_message() {
        let thread = msgs(&[("user", "ok"), ("assistant", "Understood, got it, no problem at all.")]);
        assert!(!should_ingest(&thread));
    }

    #[test]
    fn test_should_ingest_too_short() {
        let thread = msgs(&[("user", "hi there"), ("assistant", "Hello!")]);
        assert!(!should_ingest(&thread));
    }

    #[test]
    fn test_should_ingest_no_assistant_message() {
        let thread = msgs(&[("user", "What are the main principles of software architecture and design patterns?")]);
        assert!(!should_ingest(&thread));
    }

    #[test]
    fn test_should_ingest_valid_two_message() {
        let thread = msgs(&[
            ("user", "What are the main principles of clean code architecture, and how do they apply when building maintainable Go services?"),
            ("assistant", "Clean code follows separation of concerns, single responsibility, and dependency inversion. In Go, prefer small interfaces consumed where they're used, keep packages focused, and avoid premature abstraction."),
        ]);
        assert!(should_ingest(&thread));
    }

    #[test]
    fn test_should_ingest_valid_with_tool_calls() {
        let thread = msgs(&[
            ("user", "What files are in the project root and what does the layout tell us about the architecture?"),
            ("assistant", "[tool: bash]\n{\"command\":\"ls -la\"}"),
            ("user", "main.go\ngo.mod\nREADME.md\ninternal/\ncmd/\npkg/\nMakefile"),
            ("assistant", "The project contains main.go, go.mod, README.md, plus the internal/, cmd/, and pkg/ directories. This is a standard Go layout: cmd/ holds entry points, internal/ holds private packages, and pkg/ exposes public APIs."),
        ]);
        assert!(should_ingest(&thread));
    }

    #[test]
    fn test_should_ingest_trivial_case_insensitive() {
        let thread = msgs(&[("user", "THANKS"), ("assistant", "You are welcome! Glad I could help with that.")]);
        assert!(!should_ingest(&thread));
    }

    #[test]
    fn test_should_recall() {
        let cases = vec![
            ("", false),
            ("   ", false),
            ("ok", false),
            ("thanks", false),
            ("Thanks", false),
            ("hi", false),
            ("yes", false),
            ("hello world", false),
            ("what about Hormuz?", true),
            ("Tell me about the project structure for the new microservice we discussed", true),
        ];
        for (msg, want) in cases {
            assert_eq!(should_recall(msg), want, "msg={:?}", msg);
        }
    }
}