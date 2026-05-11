/// context_test.rs — Tests for context/prompt building functions.
///
/// Mirrors Go's context_test.go and the context-related tests in agent_test.go.
use std::path::Path;

use crate::config::config::{AgentConfig, AgentsConfig, ChannelsConfig, CLIConfig, Config, ToolPolicy};
use crate::session::session::{
    assistant_message_entry, compaction_entry, tool_call_entry, tool_result_entry,
    user_message_entry, EntryType, SessionEntry,
};

use crate::agent::context::{
    assemble_messages, build_config_summary, build_default_identity, build_static_system_prompt,
    inject_missing_tool_results, prune_tool_results, SpillConfig, DEFAULT_IDENTITY_BASE,
    SPILL_MARKER, TRUNCATION_MARKER,
};

// ── BuildConfigSummary tests ──────────────────────────────────────────────────

#[test]
fn test_build_config_summary_with_agents_and_cli() {
    let mut cfg = Config::default();
    cfg.agents.list = vec![
        AgentConfig {
            id: "alpha".to_owned(),
            name: "Alpha".to_owned(),
            model: "anthropic/claude-sonnet-4-5".to_owned(),
            tools: ToolPolicy {
                allow: vec!["read_file".to_owned(), "bash".to_owned()],
                deny: vec![],
            },
            ..Default::default()
        },
        AgentConfig {
            id: "beta".to_owned(),
            name: "Beta".to_owned(),
            model: "openai/gpt-4o".to_owned(),
            ..Default::default()
        },
    ];
    cfg.channels.cli.enabled = true;

    let got = build_config_summary(&cfg);
    assert!(got.contains("Configured agents:"));
    assert!(got.contains("Alpha (id: alpha, model: anthropic/claude-sonnet-4-5, tools: read_file, bash)"));
    assert!(got.contains("Beta (id: beta, model: openai/gpt-4o)"));
    assert!(got.contains("Configured channels: cli"));
}

#[test]
fn test_build_config_summary_empty_config() {
    let cfg = Config::default();
    let got = build_config_summary(&cfg);
    assert!(got.trim().is_empty());
}

// ── BuildStaticSystemPrompt tests ─────────────────────────────────────────────

#[test]
fn test_build_static_system_prompt_with_identity_file() {
    let dir = tempfile::tempdir().unwrap();
    let identity_path = dir.path().join("IDENTITY.md");
    std::fs::write(&identity_path, "CUSTOM IDENTITY").unwrap();

    let got = build_static_system_prompt(
        dir.path().to_str().unwrap(),
        "",
        "alpha",
        "Alpha",
        &["read_file".to_owned()],
        "Configured channels: cli",
        "\n\n## Skills Index\n\n- foo",
        "",
        "",
    );
    assert!(got.contains("CUSTOM IDENTITY"));
    assert!(got.contains("\"Alpha\" agent (id: alpha)"));
    assert!(got.contains("Configured channels: cli"));
    assert!(got.contains("## Skills Index"));
}

#[test]
fn test_build_static_system_prompt_config_override() {
    let dir = tempfile::tempdir().unwrap();
    let identity_path = dir.path().join("IDENTITY.md");
    std::fs::write(&identity_path, "FROM_IDENTITY_FILE").unwrap();

    let got = build_static_system_prompt(
        dir.path().to_str().unwrap(),
        "FROM CONFIG",
        "id",
        "Name",
        &[],
        "",
        "",
        "",
        "",
    );
    assert!(got.contains("FROM CONFIG"));
    assert!(!got.contains("FROM_IDENTITY_FILE"));
}

#[test]
fn test_build_static_system_prompt_default_identity() {
    let dir = tempfile::tempdir().unwrap(); // no IDENTITY.md
    let got = build_static_system_prompt(
        dir.path().to_str().unwrap(),
        "",
        "id",
        "Name",
        &["read_file".to_owned(), "bash".to_owned()],
        "",
        "",
        "",
        "",
    );
    assert!(got.contains(DEFAULT_IDENTITY_BASE));
    assert!(got.contains("read files"));
    assert!(got.contains("bash commands"));
}

#[test]
fn test_build_static_system_prompt_self_identity_line() {
    let dir = tempfile::tempdir().unwrap();
    let got = build_static_system_prompt(
        dir.path().to_str().unwrap(),
        "",
        "supervisor",
        "Supervisor",
        &[],
        "",
        "",
        "",
        "",
    );
    assert!(got.contains("\"Supervisor\" agent (id: supervisor)"));
}

#[test]
fn test_build_default_identity_tool_specific() {
    let result = build_default_identity(&[
        "read_file".to_owned(),
        "web_search".to_owned(),
        "web_fetch".to_owned(),
    ]);
    assert!(result.contains("read files"));
    assert!(result.contains("search the web"));
    assert!(result.contains("fetch web pages"));
    assert!(!result.contains("bash commands"));
    assert!(!result.contains("send_message"));
}

// ── assemble_messages tests ───────────────────────────────────────────────────

#[test]
fn test_assemble_messages_user_and_assistant() {
    let history = vec![
        user_message_entry("hello"),
        assistant_message_entry("hi there"),
    ];
    let msgs = assemble_messages(&history);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[0].content, "hello");
    assert_eq!(msgs[1].role, "assistant");
    assert_eq!(msgs[1].content, "hi there");
}

#[test]
fn test_assemble_messages_tool_call_and_result() {
    let tc = tool_call_entry("tc_1", "bash", br#"{"command":"echo hi"}"#);
    let tr = tool_result_entry("tc_1", "hi\n", "", vec![]);

    let history = vec![user_message_entry("run echo hi"), tc, tr];
    let msgs = assemble_messages(&history);
    assert_eq!(msgs.len(), 3);

    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[1].role, "assistant");
    assert_eq!(msgs[1].tool_calls.len(), 1);
    assert_eq!(msgs[1].tool_calls[0].id, "tc_1");
    assert_eq!(msgs[1].tool_calls[0].name, "bash");

    assert_eq!(msgs[2].role, "user");
    assert_eq!(msgs[2].tool_call_id, "tc_1");
    assert_eq!(msgs[2].content, "hi\n");
}

#[test]
fn test_assemble_messages_empty() {
    assert!(assemble_messages(&[]).is_empty());
    assert!(assemble_messages(&[]).is_empty());
}

#[test]
fn test_assemble_messages_orphaned_tool_call() {
    let tc = tool_call_entry("tc_orphan", "bash", br#"{"command":"pwd"}"#);

    let history = vec![
        user_message_entry("run pwd"),
        tc,
        user_message_entry("hello again"),
    ];
    let msgs = assemble_messages(&history);

    // Should have: user, assistant(tool_call), synthetic tool_result, user
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[1].role, "assistant");
    assert_eq!(msgs[1].tool_calls.len(), 1);
    // Synthetic result injected
    assert_eq!(msgs[2].role, "user");
    assert_eq!(msgs[2].tool_call_id, "tc_orphan");
    assert!(msgs[2].is_error);
    assert!(msgs[2].content.contains("interrupted"));
    // New user message
    assert_eq!(msgs[3].role, "user");
    assert_eq!(msgs[3].content, "hello again");
}

#[test]
fn test_assemble_messages_orphaned_tool_call_at_end() {
    let tc = tool_call_entry("tc_end", "bash", br#"{"command":"ls"}"#);

    let history = vec![user_message_entry("list files"), tc];
    let msgs = assemble_messages(&history);

    // Should have: user, assistant(tool_call), synthetic tool_result
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[2].tool_call_id, "tc_end");
    assert!(msgs[2].is_error);
}

#[test]
fn test_assemble_messages_compaction_entry() {
    let history = vec![
        compaction_entry(
            "we discussed feature X and chose option B",
            "",
            "",
            "m",
            0,
            0,
            4,
        ),
        user_message_entry("now what about feature Y?"),
    ];
    let msgs = assemble_messages(&history);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, "user");
    assert!(msgs[0].content.contains("[Previous conversation summary]"));
    assert!(msgs[0].content.contains("we discussed feature X"));
    assert_eq!(msgs[1].role, "user");
    assert_eq!(msgs[1].content, "now what about feature Y?");
}

// ── prune_tool_results tests ──────────────────────────────────────────────────

#[test]
fn test_prune_tool_results() {
    let long_content: String = "a".repeat(20000);
    let mut msgs = vec![
        crate::llm::Message {
            role: "user".to_owned(),
            content: "hello".to_owned(),
            ..Default::default()
        },
        crate::llm::Message {
            role: "user".to_owned(),
            content: long_content.clone(),
            tool_call_id: "tc_1".to_owned(),
            ..Default::default()
        },
    ];

    // Empty spillConfig → legacy in-place truncation.
    prune_tool_results(&mut msgs, 10000, &SpillConfig::default());

    assert_eq!(msgs[0].content, "hello");
    assert!(msgs[1].content.len() < 20000);
    assert!(msgs[1].content.contains(TRUNCATION_MARKER));
}

#[test]
fn test_prune_tool_results_short() {
    let mut msgs = vec![crate::llm::Message {
        role: "user".to_owned(),
        content: "short output".to_owned(),
        tool_call_id: "tc_1".to_owned(),
        ..Default::default()
    }];
    prune_tool_results(&mut msgs, 10000, &SpillConfig::default());
    assert_eq!(msgs[0].content, "short output");
}

#[test]
fn test_prune_tool_results_spills_to_disk() {
    let workspace = tempfile::tempdir().unwrap();
    let original: String = "a".repeat(20000);
    let mut msgs = vec![crate::llm::Message {
        role: "user".to_owned(),
        content: original.clone(),
        tool_call_id: "tc_42".to_owned(),
        ..Default::default()
    }];

    prune_tool_results(
        &mut msgs,
        10000,
        &SpillConfig {
            workspace: workspace.path().to_string_lossy().into_owned(),
            session_key: "sess_abc".to_owned(),
        },
    );

    let got = &msgs[0].content;
    assert!(got.len() < original.len());
    assert!(got.contains(SPILL_MARKER));
    assert!(!got.contains(TRUNCATION_MARKER));

    let want_path = workspace
        .path()
        .join(".robin")
        .join("spill")
        .join("sess_abc")
        .join("tc_42.txt");
    assert!(got.contains(want_path.to_str().unwrap()));
    let data = std::fs::read_to_string(&want_path).unwrap();
    assert_eq!(data, original);
}

#[test]
fn test_prune_tool_results_idempotent_after_spill() {
    let workspace = tempfile::tempdir().unwrap();
    let cfg = SpillConfig {
        workspace: workspace.path().to_string_lossy().into_owned(),
        session_key: "sess_xyz".to_owned(),
    };
    let mut msgs = vec![crate::llm::Message {
        role: "user".to_owned(),
        content: "b".repeat(20000),
        tool_call_id: "tc_99".to_owned(),
        ..Default::default()
    }];

    prune_tool_results(&mut msgs, 10000, &cfg);
    let after_first = msgs[0].content.clone();
    assert!(after_first.contains(SPILL_MARKER));

    // Second call: marker is present, so the message must be untouched.
    prune_tool_results(&mut msgs, 10000, &cfg);
    assert_eq!(msgs[0].content, after_first);
}