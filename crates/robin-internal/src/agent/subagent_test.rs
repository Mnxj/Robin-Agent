/// subagent_test.rs — Tests for subagent factory and runner.
///
/// Mirrors Go's subagent_test.go. Uses Tokio for async runtime tests.
use std::sync::Arc;

use crate::config::config::{AgentConfig, AgentsConfig, Config};
use crate::session::session::Session;

use super::super::subagent::new_subagent_session;

#[test]
fn test_new_subagent_session_is_ephemeral() {
    let sess = new_subagent_session("researcher");
    assert_eq!(sess.key, "subagent");
    assert_eq!(sess.agent_id, "researcher");
    // Should have no entries yet
    assert!(sess.view().is_empty());
}

#[test]
fn test_new_subagent_session_key_is_subagent() {
    // The key must be "subagent" — this is what BuildRuntimeForAgent checks to
    // skip calibrator seeding for ephemeral sessions.
    let sess = new_subagent_session("any-agent");
    assert_eq!(sess.key, "subagent");
}
