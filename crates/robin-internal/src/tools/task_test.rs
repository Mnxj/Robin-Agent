#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::super::{AgentEventLike, SubagentRunner, TaskTool};
    use crate::tools::tool::Tool;

    struct StubRunner {
        events: Vec<AgentEventLike>,
        run_err: Option<String>,
    }

    impl SubagentRunner for StubRunner {
        fn run(&self, _prompt: String) -> anyhow::Result<mpsc::Receiver<AgentEventLike>> {
            if let Some(e) = &self.run_err {
                anyhow::bail!("{}", e);
            }
            let (tx, rx) = mpsc::channel();
            for ev in &self.events {
                tx.send(ev.clone()).unwrap();
            }
            Ok(rx)
        }
    }

    fn make_factory_stub(runner: StubRunner) -> super::super::SubagentFactory {
        let runner = std::sync::Arc::new(runner);
        Box::new(move |_agent_id: &str, _depth: i32| {
            Ok(Box::new(StubRunner {
                events: runner.events.clone(),
                run_err: runner.run_err.clone(),
            }) as Box<dyn SubagentRunner>)
        })
    }

    #[test]
    fn test_task_tool_unknown_agent_returns_error() {
        let mut eligible = std::collections::HashMap::new();
        eligible.insert("researcher".to_owned(), "Web research".to_owned());

        let factory = Box::new(|_: &str, _: i32| -> anyhow::Result<Box<dyn SubagentRunner>> {
            panic!("factory should not be called for unknown agent_id");
        });
        let tt = TaskTool::new(factory, 0, eligible);

        let res = tt.execute(serde_json::json!({"agent_id": "ghost", "prompt": "hi"})).unwrap();
        assert!(!res.error.is_empty(), "expected error");
        assert!(res.error.contains("ghost"), "error: {}", res.error);
        assert!(res.error.contains("researcher"), "error: {}", res.error);
    }

    #[test]
    fn test_task_tool_delegates_and_captures_text() {
        let runner = StubRunner {
            events: vec![
                AgentEventLike { text: "hello ".to_owned(), ..Default::default() },
                AgentEventLike { text: "world".to_owned(), ..Default::default() },
                AgentEventLike { done: true, ..Default::default() },
            ],
            run_err: None,
        };
        let mut eligible = std::collections::HashMap::new();
        eligible.insert("researcher".to_owned(), "Web research".to_owned());
        let tt = TaskTool::new(make_factory_stub(runner), 1, eligible);

        let res = tt.execute(serde_json::json!({"agent_id": "researcher", "prompt": "summarize Go"})).unwrap();
        assert!(res.error.is_empty(), "error: {}", res.error);
        assert_eq!(res.output, "hello world");
    }

    #[test]
    fn test_task_tool_subagent_abort_returns_error_result() {
        let runner = StubRunner {
            events: vec![
                AgentEventLike { text: "partial...".to_owned(), ..Default::default() },
                AgentEventLike { aborted: true, ..Default::default() },
            ],
            run_err: None,
        };
        let mut eligible = std::collections::HashMap::new();
        eligible.insert("r".to_owned(), "desc".to_owned());
        let tt = TaskTool::new(make_factory_stub(runner), 0, eligible);

        let res = tt.execute(serde_json::json!({"agent_id": "r", "prompt": "go"})).unwrap();
        assert!(!res.error.is_empty(), "expected abort error");
        assert!(res.error.contains("abort"), "error: {}", res.error);
    }

    #[test]
    fn test_task_tool_factory_depth_error_passes_through() {
        let mut eligible = std::collections::HashMap::new();
        eligible.insert("r".to_owned(), "desc".to_owned());
        let factory: super::super::SubagentFactory = Box::new(|_, _| {
            anyhow::bail!("subagent depth limit 3 reached")
        });
        let tt = TaskTool::new(factory, 3, eligible);

        let res = tt.execute(serde_json::json!({"agent_id": "r", "prompt": "go"})).unwrap();
        assert!(!res.error.is_empty(), "expected depth-limit error");
        assert!(res.error.contains("depth limit"), "error: {}", res.error);
    }

    #[test]
    fn test_task_tool_malformed_input_returns_error() {
        let mut eligible = std::collections::HashMap::new();
        eligible.insert("r".to_owned(), "desc".to_owned());
        let factory: super::super::SubagentFactory = Box::new(|_, _| {
            panic!("factory should not be called");
        });
        let tt = TaskTool::new(factory, 0, eligible);

        let cases = vec![
            serde_json::json!({"prompt": "go"}),             // missing agent_id
            serde_json::json!({"agent_id": "r"}),            // missing prompt
            serde_json::json!({"agent_id": "", "prompt": ""}), // empty fields
        ];
        for case in cases {
            let res = tt.execute(case.clone()).unwrap();
            assert!(!res.error.is_empty(), "expected error for input: {}", case);
        }
    }
}