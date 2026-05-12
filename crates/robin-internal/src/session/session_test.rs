#[cfg(test)]
mod tests {
    use crate::session::session::{
        CompactionData, EntryType, ImageData, Session, SessionEntry, ToolCallData, ToolResultData,
        aborted_tool_result_entry, assistant_message_entry, compaction_entry, tool_call_entry,
        tool_result_entry, user_message_entry,
    };
    use crate::session::store::Store;

    #[test]
    fn test_session_append_and_history() {
        let sess = Session::new("default", "test");

        sess.append(user_message_entry("hello"));
        sess.append(assistant_message_entry("hi there"));
        sess.append(user_message_entry("how are you?"));

        let history = sess.history();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].entry_type, EntryType::Message);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[2].role, "user");
    }

    #[test]
    fn test_session_dag_traversal() {
        let sess = Session::new("default", "test");

        sess.append(user_message_entry("first"));
        sess.append(assistant_message_entry("second"));

        let history = sess.history();
        assert_eq!(history.len(), 2);

        // Parent chain should be connected.
        assert!(history[0].parent_id.is_empty());
        assert_eq!(history[1].parent_id, history[0].id);
    }

    #[test]
    fn test_store_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        // Create and populate a session.
        let sess = store.load("agent1", "test_peer").unwrap();
        sess.append(user_message_entry("hello"));
        sess.append(assistant_message_entry("world"));

        // Reload from disk.
        let sess2 = store.load("agent1", "test_peer").unwrap();
        let history = sess2.history();
        assert_eq!(history.len(), 2);

        // Check file exists.
        let path = dir.path().join("test_peer.jsonl");
        assert!(path.exists());
    }

    #[test]
    fn test_tool_call_entries() {
        let sess = Session::new("default", "test");

        sess.append(user_message_entry("run ls"));
        sess.append(tool_call_entry("tc_1", "bash", br#"{"command":"ls"}"#));
        sess.append(tool_result_entry("tc_1", "file1\nfile2", "", vec![]));
        sess.append(assistant_message_entry("Here are the files."));

        let history = sess.history();
        assert_eq!(history.len(), 4);
        assert_eq!(history[1].entry_type, EntryType::ToolCall);
        assert_eq!(history[2].entry_type, EntryType::ToolResult);
    }

    /// Regression guard for the "data:null" bug.
    #[test]
    fn test_tool_call_entry_sanitises_empty_input() {
        let cases: &[(&str, &[u8])] = &[
            ("nil_input", b""),
            ("truncated_object", b"{\"a\":"),
            ("plain_text", b"hello"),
        ];

        for (name, input) in cases {
            let e = tool_call_entry("toolu_x", "search", input);
            assert!(
                e.data.get() != "null" && !e.data.get().is_empty(),
                "{}: Data must not be null",
                name
            );
            let td: ToolCallData = serde_json::from_str(e.data.get())
                .unwrap_or_else(|err| panic!("{}: failed to unmarshal: {}", name, err));
            assert_eq!(td.id, "toolu_x", "{}: ID must round-trip", name);
            assert_eq!(td.tool, "search", "{}: Tool must round-trip", name);
            assert!(
                serde_json::from_str::<serde_json::Value>(td.input.get()).is_ok(),
                "{}: Input must be valid JSON",
                name
            );
        }
    }

    #[test]
    fn test_session_branch() {
        let sess = Session::new("default", "test");

        sess.append(user_message_entry("first"));
        let first_id = sess.leaf_id();
        sess.append(assistant_message_entry("response 1"));
        sess.append(user_message_entry("second"));

        // Branch back to first entry.
        sess.branch(&first_id).unwrap();
        assert_eq!(sess.leaf_id(), first_id);

        // Append on the branch.
        sess.append(assistant_message_entry("alternate response"));

        // History should follow the branch.
        let history = sess.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
    }

    #[test]
    fn test_session_branch_invalid_id() {
        let sess = Session::new("default", "test");
        sess.append(user_message_entry("hello"));

        let result = sess.branch("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_session_compact() {
        let sess = Session::new("default", "test");

        for i in 0u8..10 {
            sess.append(user_message_entry(&format!("question {}", i)));
            sess.append(assistant_message_entry(&format!("answer {}", i)));
        }

        let history = sess.history();
        assert_eq!(history.len(), 20);

        // Compact, keeping last 4 entries.
        sess.compact("Summary of conversation: discussed topics 0-7", 4);

        let history = sess.history();
        // Should have: 1 summary + 4 kept entries = 5.
        assert_eq!(history.len(), 5);

        // First entry should be the summary meta entry.
        assert_eq!(history[0].entry_type, EntryType::Meta);
        assert_eq!(history[0].role, "system");
    }

    #[test]
    fn test_session_compact_no_op() {
        let sess = Session::new("default", "test");
        sess.append(user_message_entry("hello"));
        sess.append(assistant_message_entry("world"));

        // Compacting with keep_entries >= history length should be a no-op.
        sess.compact("summary", 10);

        let history = sess.history();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_session_compact_with_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        let sess = store.load("agent1", "compact_test").unwrap();

        for i in 0u8..10 {
            sess.append(user_message_entry(&format!("msg {}", i)));
            sess.append(assistant_message_entry(&format!("reply {}", i)));
        }

        sess.compact("Summary of conversation", 4);

        // Reload and verify.
        let sess2 = store.load("agent1", "compact_test").unwrap();
        let history = sess2.history();
        assert_eq!(history.len(), 5); // 1 summary + 4 kept
        assert_eq!(history[0].entry_type, EntryType::Meta);
    }

    #[test]
    fn test_estimate_tokens() {
        let sess = Session::new("default", "test");
        sess.append(user_message_entry("Hello, how are you doing today?"));
        sess.append(assistant_message_entry("I'm doing well, thank you for asking!"));

        let tokens = sess.estimate_tokens();
        assert!(tokens > 0);
    }

    #[test]
    fn test_session_view_without_compaction_matches_history() {
        let sess = Session::new("default", "test");
        sess.append(user_message_entry("hi"));
        sess.append(assistant_message_entry("hello"));
        sess.append(user_message_entry("hello again"));

        let view = sess.view();
        let hist = sess.history();
        assert_eq!(view.len(), hist.len());
        for (v, h) in view.iter().zip(hist.iter()) {
            assert_eq!(v.id, h.id);
        }
    }

    #[test]
    fn test_session_view_with_single_compaction() {
        let sess = Session::new("default", "test");
        sess.append(user_message_entry("u1"));
        sess.append(assistant_message_entry("a1"));
        sess.append(user_message_entry("u2"));
        sess.append(compaction_entry(
            "summary of u1/a1/u2",
            "",
            "",
            "ollama/qwen2.5:3b-instruct",
            100,
            25,
            3,
        ));
        sess.append(assistant_message_entry("a2 after compaction"));
        sess.append(user_message_entry("u3"));

        let view = sess.view();
        assert_eq!(view.len(), 3);
        assert_eq!(view[0].entry_type, EntryType::Compaction);
        assert_eq!(view[1].entry_type, EntryType::Message);
        assert_eq!(view[1].role, "assistant");
        assert_eq!(view[2].role, "user");
    }

    #[test]
    fn test_session_view_with_multiple_compactions() {
        let sess = Session::new("default", "test");
        sess.append(user_message_entry("old"));
        sess.append(compaction_entry("first summary", "", "", "m", 0, 0, 1));
        sess.append(user_message_entry("middle"));
        sess.append(compaction_entry("second summary", "", "", "m", 0, 0, 1));
        sess.append(user_message_entry("recent"));

        let view = sess.view();
        assert_eq!(view.len(), 2);
        // Most recent compaction supersedes the first.
        let cd: CompactionData = serde_json::from_str(view[0].data.get()).unwrap();
        assert_eq!(cd.summary, "second summary");
        assert_eq!(view[1].role, "user");
    }

    #[test]
    fn test_compaction_entry_has_correct_fields() {
        let e = compaction_entry(
            "hello summary",
            "start_id",
            "end_id",
            "ollama/qwen2.5:3b",
            1000,
            250,
            12,
        );
        assert_eq!(e.entry_type, EntryType::Compaction);
        assert_eq!(e.role, "system");

        let cd: CompactionData = serde_json::from_str(e.data.get()).unwrap();
        assert_eq!(cd.summary, "hello summary");
        assert_eq!(cd.range_start_id, "start_id");
        assert_eq!(cd.range_end_id, "end_id");
        assert_eq!(cd.model, "ollama/qwen2.5:3b");
        assert_eq!(cd.tokens_before, 1000);
        assert_eq!(cd.tokens_estimated_after, 250);
        assert_eq!(cd.turns_compacted, 12);
    }

    #[test]
    fn test_tool_result_data_aborted_field_round_trip() {
        let entry = aborted_tool_result_entry("tc_abc");
        assert_eq!(entry.entry_type, EntryType::ToolResult);

        let data: ToolResultData = serde_json::from_str(entry.data.get()).unwrap();
        assert_eq!(data.tool_call_id, "tc_abc");
        assert_eq!(data.error, "aborted by user");
        assert!(data.is_error);
        assert!(data.aborted);
        assert!(data.output.is_empty());
    }

    #[test]
    fn test_tool_result_data_old_jsonl_without_aborted_field() {
        // Simulate an old session entry written before the Aborted field existed.
        let old_json = r#"{"tool_call_id":"tc_old","output":"hello","is_error":false}"#;
        let data: ToolResultData = serde_json::from_str(old_json).unwrap();

        assert_eq!(data.tool_call_id, "tc_old");
        assert_eq!(data.output, "hello");
        assert!(!data.is_error);
        assert!(!data.aborted, "missing field must default to false");
    }

    #[test]
    fn test_session_append_concurrent() {
        use std::sync::Arc;
        use std::thread;

        let sess = Arc::new(Session::new("a", "k"));

        const N: usize = 100;
        let mut handles = Vec::with_capacity(N);

        for i in 0..N {
            let sess_clone = sess.clone();
            handles.push(thread::spawn(move || {
                let mut e = user_message_entry("msg");
                e.id = format!("e_{}", i);
                sess_clone.append(e);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let view = sess.view();
        assert_eq!(view.len(), N, "every Append must land");

        let mut seen = std::collections::HashSet::new();
        for e in &view {
            assert!(!seen.contains(&e.id), "duplicate ID {}", e.id);
            seen.insert(e.id.clone());
        }
        assert_eq!(seen.len(), N);
    }

    #[test]
    fn test_session_view_returns_copy() {
        let sess = Session::new("a", "k");
        let mut e = user_message_entry("hi");
        e.id = "e_1".to_string();
        sess.append(e);

        let mut v1 = sess.view();
        assert_eq!(v1.len(), 1);
        // Mutate the returned slice.
        let mut mutated = user_message_entry("mutated");
        mutated.id = "MUTATED".to_string();
        v1[0] = mutated;

        let v2 = sess.view();
        assert_eq!(v2.len(), 1);
        assert_eq!(
            v2[0].id, "e_1",
            "internal state must not be mutated by caller's slice modification"
        );
    }
}
