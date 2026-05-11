/// inheritance_test.rs — Tests for inherit_parent_history.
///
/// Mirrors Go's inheritance_test.go.
use std::sync::Arc;

use crate::session::session::{
    assistant_message_entry, user_message_entry, MessageData, Session,
};

use crate::agent::subagent::{inherit_parent_history, new_subagent_session};

fn decode_message(e: &crate::session::session::SessionEntry) -> MessageData {
    serde_json::from_str(e.data.get()).unwrap_or_default()
}

#[test]
fn test_inherit_parent_history_copies_view_into_subagent_session() {
    let parent = Arc::new(Session::new("parent", "key"));
    parent.append(user_message_entry("first user msg"));
    parent.append(assistant_message_entry("first reply"));
    parent.append(user_message_entry("second user msg"));

    let sub = new_subagent_session("sub");
    inherit_parent_history(&sub, &parent);

    let sub_view = sub.view();
    assert_eq!(sub_view.len(), 3, "all 3 parent entries must land in subagent");

    let got: Vec<String> = sub_view.iter().map(|e| decode_message(e).text).collect();
    assert_eq!(got, vec!["first user msg", "first reply", "second user msg"]);
}

#[test]
fn test_inherit_parent_history_walks_from_inherited_leaf() {
    // The first inherited entry must lose its parent_id — otherwise the
    // subagent's empty leaf_id lets Append leave a dangling pointer.
    let parent = Arc::new(Session::new("parent", "key"));
    parent.append(user_message_entry("u1"));
    parent.append(assistant_message_entry("a1"));

    let sub = new_subagent_session("sub");
    inherit_parent_history(&sub, &parent);

    // View walks back from leaf via parent_id; we must reach BOTH entries.
    assert_eq!(sub.view().len(), 2, "View must traverse all inherited entries");
}

#[test]
fn test_inherit_parent_history_empty_parent_is_noop() {
    let parent = Arc::new(Session::new("parent", "key")); // no entries
    let sub = new_subagent_session("sub");
    inherit_parent_history(&sub, &parent);
    assert!(sub.view().is_empty());
}
