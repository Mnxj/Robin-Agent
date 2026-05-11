use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::llm::provider::{ChatEvent, ChatRequest, Diagnostic, EventType, LLMProvider, ModelInfo, ToolDef};
use crate::session::{EntryType, Session, assistant_message_entry, user_message_entry};

use super::{CompactionResult, Manager, Reason, MAX_CONSECUTIVE_FAILURES};
use crate::compaction::summarizer::Summarizer;

// ── fakeProvider ──────────────────────────────────────────────────────────────

pub struct FakeProvider {
    pub text: String,
}

#[async_trait::async_trait]
impl LLMProvider for FakeProvider {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        let text = self.text.clone();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = tx.send(ChatEvent {
                event_type: EventType::TextDelta,
                text,
                ..Default::default()
            }).await;
            let _ = tx.send(ChatEvent {
                event_type: EventType::Done,
                ..Default::default()
            }).await;
        });
        Ok(rx)
    }
    fn models(&self) -> Vec<ModelInfo> { vec![] }
    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (tools, vec![])
    }
}

// ── alwaysFailingProvider ─────────────────────────────────────────────────────

pub struct AlwaysFailingProvider;

#[async_trait::async_trait]
impl LLMProvider for AlwaysFailingProvider {
    async fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<ChatEvent>> {
        Err(anyhow::anyhow!("provider down"))
    }
    fn models(&self) -> Vec<ModelInfo> { vec![] }
    fn normalize_tool_schema(&self, tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<Diagnostic>) {
        (tools, vec![])
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn long_session() -> Session {
    let sess = Session::new("default", "test");
    for _ in 0..6 {
        sess.append(user_message_entry("user msg"));
        sess.append(assistant_message_entry("assistant reply"));
    }
    sess
}

fn long_session_with(agent_id: &str, key: &str) -> Session {
    let sess = Session::new(agent_id, key);
    for _ in 0..6 {
        sess.append(user_message_entry("user msg"));
        sess.append(assistant_message_entry("assistant reply"));
    }
    sess
}

fn make_manager(text: &str) -> Arc<Manager> {
    Arc::new(Manager::new(
        Arc::new(Summarizer {
            provider: Arc::new(FakeProvider { text: text.to_string() }),
            model: "m".to_string(),
            timeout: Duration::from_secs(1),
        }),
        4,
        0.0,
        0,
    ))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_manager_appends_compaction_entry() {
    let mgr = make_manager("summary text");
    let sess = long_session();
    let res = mgr.maybe_compact(&sess, Reason::Manual, "").await.unwrap();
    assert!(res.compacted);
    assert_eq!(res.reason.as_ref().unwrap(), &Reason::Manual);

    // First entry in view should be the compaction.
    let view = sess.view();
    assert!(!view.is_empty());
    assert_eq!(view[0].entry_type, EntryType::Compaction);
}

#[tokio::test]
async fn test_view_includes_preserved_range_after_compaction() {
    let mgr = make_manager("summary text");
    let sess = long_session();
    let res = mgr.maybe_compact(&sess, Reason::Manual, "").await.unwrap();
    assert!(res.compacted);

    let view = sess.view();
    assert_eq!(
        view.len(),
        9,
        "view must be [compaction, 4 user/assistant pairs]; got {} entries",
        view.len()
    );
    assert_eq!(view[0].entry_type, EntryType::Compaction, "first entry is the summary");
    for i in 1..9 {
        assert_eq!(
            view[i].entry_type,
            EntryType::Message,
            "entry {} is a preserved message",
            i
        );
    }
    assert_eq!(view[1].role, "user");
    assert_eq!(view[2].role, "assistant");
    assert_eq!(view[7].role, "user");
    assert_eq!(view[8].role, "assistant");
}

#[tokio::test]
async fn test_manager_refuses_short_session() {
    let mgr = make_manager("summary");
    let sess = Session::new("default", "test");
    sess.append(user_message_entry("only one"));
    let res = mgr.maybe_compact(&sess, Reason::Manual, "").await.unwrap();
    assert!(!res.compacted);
    assert_eq!(res.skipped, "too_short");
}

#[tokio::test]
async fn test_manager_summarizer_error_falls_back_to_placeholder() {
    // Empty model response → stage-3 placeholder — compaction "succeeds" with stub.
    let mgr = make_manager("");
    let sess = long_session();
    let res = mgr.maybe_compact(&sess, Reason::Manual, "").await.unwrap();
    assert!(res.compacted, "stage-3 placeholder counts as successful compaction");
    assert!(
        res.summary.contains("compaction failed"),
        "summary must contain the placeholder marker; got: {}",
        res.summary
    );
}

#[tokio::test]
async fn test_manager_forgets_session() {
    let mgr = make_manager("ok");
    let sess = long_session();
    let _ = mgr.maybe_compact(&sess, Reason::Manual, "").await;

    let key = format!("{}/{}", sess.agent_id, sess.key);
    let has_lock = mgr.locks.lock().contains_key(&key);
    assert!(has_lock, "manager should have a lock entry for this session");

    mgr.forget_session(&sess);

    let has_lock = mgr.locks.lock().contains_key(&key);
    assert!(!has_lock, "lock entry should be removed after forget_session");
}

#[tokio::test]
async fn test_circuit_breaker_trips_after_max_failures() {
    let mgr = Arc::new(Manager::new(
        Arc::new(Summarizer {
            provider: Arc::new(AlwaysFailingProvider),
            model: "m".to_string(),
            timeout: Duration::from_secs(1),
        }),
        4,
        0.0,
        0,
    ));

    let sess = long_session();

    for i in 0..MAX_CONSECUTIVE_FAILURES {
        let res = mgr.maybe_compact(&sess, Reason::Preventive, "").await.unwrap();
        assert!(res.compacted, "iteration {} should still attempt", i);
        // Add new turns so the next iteration has something to compact.
        sess.append(user_message_entry("follow-up"));
        sess.append(assistant_message_entry("more reply"));
    }

    let res = mgr.maybe_compact(&sess, Reason::Preventive, "").await.unwrap();
    assert!(!res.compacted, "call after MaxConsecutiveFailures must be skipped");
    assert_eq!(res.skipped, "circuit_breaker");
}

#[tokio::test]
async fn test_circuit_breaker_resets_on_success() {
    let mgr = make_manager("ok");
    let sess = long_session();

    for _ in 0..(MAX_CONSECUTIVE_FAILURES + 5) {
        let _ = mgr.maybe_compact(&sess, Reason::Preventive, "").await.unwrap();
    }
    let res = mgr.maybe_compact(&sess, Reason::Preventive, "").await.unwrap();
    assert_ne!(res.skipped, "circuit_breaker", "circuit breaker must not trip on success");
}

#[tokio::test]
async fn test_circuit_breaker_is_per_session() {
    let mgr = Arc::new(Manager::new(
        Arc::new(Summarizer {
            provider: Arc::new(AlwaysFailingProvider),
            model: "m".to_string(),
            timeout: Duration::from_secs(1),
        }),
        4,
        0.0,
        0,
    ));

    let sess_a = long_session_with("default", "a");
    let sess_b = long_session_with("default", "b");

    for _ in 0..MAX_CONSECUTIVE_FAILURES {
        let _ = mgr.maybe_compact(&sess_a, Reason::Preventive, "").await;
        sess_a.append(user_message_entry("follow-up"));
        sess_a.append(assistant_message_entry("more reply"));
    }

    let res = mgr.maybe_compact(&sess_b, Reason::Preventive, "").await.unwrap();
    assert_ne!(
        res.skipped, "circuit_breaker",
        "Session B must not be tripped by Session A's failures"
    );
}

#[tokio::test]
async fn test_deferred_forget_session_drains_locks_map() {
    let mgr = make_manager("ok");
    const TURNS: usize = 50;
    for _ in 0..TURNS {
        let sess = long_session();
        let _ = mgr.maybe_compact(&sess, Reason::Manual, "").await;
        mgr.forget_session(&sess);
    }
    let lock_count = mgr.locks.lock().len();
    assert_eq!(
        lock_count, 0,
        "defer forget_session on each turn must drain the locks map; got {} residual entries after {} turns",
        lock_count, TURNS
    );
    let fail_count = mgr.fail_mu.lock().len();
    assert_eq!(
        fail_count, 0,
        "defer forget_session must also drain the failures map"
    );
}