use serde_json::Value;

use crate::gateway::websocket::safe_raw_message;

// ─── safe_raw_message ──────────────────────────────────────────────────────
//
// Empty or invalid input must return Value::Null (marshals to JSON null)
// instead of triggering a parse error at write time — which would abort
// the entire WebSocket write and leave the chat client's tool_call entry
// stuck in a pending state.

#[test]
fn test_safe_raw_message_nil() {
    let got = safe_raw_message(None);
    assert_eq!(got, Value::Null);
    // Must round-trip through serde_json without error.
    let enc = serde_json::to_string(&serde_json::json!({"v": got})).unwrap();
    assert!(enc.contains("null"));
}

#[test]
fn test_safe_raw_message_empty() {
    let got = safe_raw_message(Some(""));
    assert_eq!(got, Value::Null);
}

#[test]
fn test_safe_raw_message_whitespace_only() {
    let got = safe_raw_message(Some("   "));
    assert_eq!(got, Value::Null);
}

#[test]
fn test_safe_raw_message_truncated_object() {
    let got = safe_raw_message(Some(r#"{"a":"#));
    assert_eq!(got, Value::Null);
    serde_json::to_string(&serde_json::json!({"v": got})).unwrap();
}

#[test]
fn test_safe_raw_message_plain_text_invalid() {
    let got = safe_raw_message(Some("hello world"));
    assert_eq!(got, Value::Null);
}

#[test]
fn test_safe_raw_message_valid_object() {
    let got = safe_raw_message(Some(r#"{"a":1}"#));
    assert_eq!(got, serde_json::json!({"a": 1}));
    serde_json::to_string(&serde_json::json!({"v": got})).unwrap();
}

#[test]
fn test_safe_raw_message_valid_null() {
    let got = safe_raw_message(Some("null"));
    assert_eq!(got, Value::Null);
    // null is valid JSON — marshal must succeed.
    serde_json::to_string(&serde_json::json!({"v": got})).unwrap();
}

#[test]
fn test_safe_raw_message_valid_array() {
    let got = safe_raw_message(Some("[1,2,3]"));
    assert_eq!(got, serde_json::json!([1, 2, 3]));
}

#[test]
fn test_safe_raw_message_valid_string() {
    let got = safe_raw_message(Some(r#""hi""#));
    assert_eq!(got, serde_json::json!("hi"));
}

// ─── Concurrent write safety ───────────────────────────────────────────────
//
// The gateway has multiple paths that write to the same WS connection from
// different goroutines/tasks (main event drain loop, trace callbacks,
// mid-stream compact responses).  In the Rust translation we serialise all
// writes through a single `mpsc::unbounded_channel` — the writer task owns
// the sink and drains the channel sequentially.  This test fans 200 writes
// across 50 tasks and verifies that no message is dropped.
//
// Because axum WebSocket only works inside a live HTTP server, we model the
// channel directly: we spin up a real mpsc channel, fan 50 tasks that each
// push 4 messages, and verify every message reaches the receiver.

#[tokio::test]
async fn test_write_channel_is_goroutine_safe() {
    use tokio::sync::mpsc;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    const GOROUTINES: usize = 50;
    const WRITES_PER_G: usize = 4;

    let mut handles = Vec::with_capacity(GOROUTINES);
    for g in 0..GOROUTINES {
        let tx = tx.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..WRITES_PER_G {
                let msg = serde_json::to_string(&serde_json::json!({
                    "goroutine": g,
                    "index": i,
                }))
                .unwrap();
                tx.send(msg).unwrap();
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Drop the last sender so the receiver loop terminates.
    drop(tx);

    let mut count = 0usize;
    while let Some(msg) = rx.recv().await {
        // Each message must be valid JSON with the expected fields.
        let v: Value = serde_json::from_str(&msg).unwrap();
        assert!(v["goroutine"].is_u64());
        assert!(v["index"].is_u64());
        count += 1;
    }

    assert_eq!(
        count,
        GOROUTINES * WRITES_PER_G,
        "every write should arrive intact, none dropped"
    );
}