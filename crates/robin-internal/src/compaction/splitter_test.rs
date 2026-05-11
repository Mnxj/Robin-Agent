use super::split;
use crate::session::{
    EntryType,
    assistant_message_entry, tool_call_entry, tool_result_entry, user_message_entry,
};

/// make_history builds a slice of SessionEntry with the given roles, in order.
/// "user" or "assistant" → message; "tc" → tool_call; "tr" → tool_result.
fn make_history(roles: &[&str]) -> Vec<crate::session::SessionEntry> {
    let mut out = Vec::new();
    for &r in roles {
        match r {
            "user" => out.push(user_message_entry("u")),
            "assistant" => out.push(assistant_message_entry("a")),
            "tc" => out.push(tool_call_entry("id1", "bash", b"{}")),
            "tr" => out.push(tool_result_entry("id1", "out", "", vec![])),
            _ => {}
        }
    }
    out
}

#[test]
fn test_split_five_user_messages_k4() {
    // 5 user msgs → cutoff after the 1st user msg. compact = [u1, a1].
    let h = make_history(&[
        "user", "assistant", "user", "assistant", "user", "assistant",
        "user", "assistant", "user", "assistant",
    ]);
    let result = split(&h, 4);
    assert!(result.is_some(), "expected ok=true");
    let (to_compact, to_preserve) = result.unwrap();
    assert_eq!(to_compact.len(), 2, "first user+assistant");
    assert_eq!(to_preserve.len(), 8, "last 4 user msgs + their assistant replies");
}

#[test]
fn test_split_exactly_k_user_messages_refuses() {
    let h = make_history(&[
        "user", "assistant", "user", "assistant", "user", "assistant", "user", "assistant",
    ]);
    let result = split(&h, 4);
    assert!(result.is_none(), "exactly K user msgs → no cutoff exists");
}

#[test]
fn test_split_fewer_than_k_user_messages_refuses() {
    let h = make_history(&["user", "assistant", "user", "assistant"]);
    let result = split(&h, 4);
    assert!(result.is_none());
}

#[test]
fn test_split_zero_user_messages_refuses() {
    let h = make_history(&["assistant"]);
    let result = split(&h, 4);
    assert!(result.is_none());
}

#[test]
fn test_split_preserves_tool_pair() {
    // 5 user msgs, with a tool pair attached to the last assistant turn.
    let h = make_history(&[
        "user", "assistant", "user", "assistant", "user", "assistant",
        "user", "assistant", "user", "assistant", "tc", "tr",
    ]);
    let result = split(&h, 4);
    assert!(result.is_some());
    let (to_compact, to_preserve) = result.unwrap();
    // Cutoff is after first user+assistant.
    assert_eq!(to_compact.len(), 2);
    // Preserved tail must include the trailing tc/tr together.
    let last = to_preserve.last().unwrap();
    let prev_to_last = &to_preserve[to_preserve.len() - 2];
    assert_eq!(last.entry_type, EntryType::ToolResult);
    assert_eq!(prev_to_last.entry_type, EntryType::ToolCall);
}

#[test]
fn test_split_compact_range_never_contains_last_user_msg() {
    let h = make_history(&[
        "user", "assistant", "user", "assistant", "user", "user", "user", "user", "user",
    ]);
    let result = split(&h, 4);
    assert!(result.is_some());
    let (_to_compact, to_preserve) = result.unwrap();

    // Preserved must contain the last 4 user msgs.
    let user_in_preserve = to_preserve.iter()
        .filter(|e| e.entry_type == EntryType::Message && e.role == "user")
        .count();
    assert_eq!(user_in_preserve, 4);
}