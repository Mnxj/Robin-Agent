use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::Notify;
use tracing::{debug, info, warn};

use crate::session::{compaction_entry, Session};

use super::overflow::is_context_overflow;
use super::splitter::split;
use super::summarizer::Summarizer;

/// MaxConsecutiveFailures is the per-session circuit-breaker threshold.
/// After this many consecutive autocompact attempts that drop to the
/// placeholder stage (stage 3), MaybeCompact stops attempting compaction
/// for the session and returns skipped = "circuit_breaker".
pub const MAX_CONSECUTIVE_FAILURES: usize = 3;

/// Reason identifies why compaction was triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Preventive,
    Reactive,
    Manual,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::Preventive => write!(f, "preventive"),
            Reason::Reactive => write!(f, "reactive"),
            Reason::Manual => write!(f, "manual"),
        }
    }
}

/// CompactionResult describes the outcome of a maybe_compact call.
#[derive(Debug, Default, Clone)]
pub struct CompactionResult {
    pub compacted: bool,
    pub reason: Option<Reason>,
    pub skipped: String,
    pub turns_compacted: usize,
    pub tokens_before: i64,
    pub tokens_after: i64,
    pub summary: String,
    pub duration_ms: i64,
}

// ── InFlightCompaction ────────────────────────────────────────────────────────

struct InFlightCompaction {
    done: Arc<Notify>,
    result: Mutex<Option<CompactionResult>>,
}

// ── Manager ───────────────────────────────────────────────────────────────────

/// Manager orchestrates compaction for sessions.  One Manager is shared across
/// the whole agent runtime; it tracks per-session mutexes internally.
pub struct Manager {
    pub summarizer: Arc<Summarizer>,
    /// K; default 4 if zero
    pub preserve_turns: usize,
    /// fraction of context window that triggers preventive compaction (e.g. 0.6)
    pub threshold: f64,
    /// MessageCap is a hard backstop on total message count.  0 disables.
    pub message_cap: i32,

    /// per-session async mutexes (keyed by stable_key)
    pub(crate) locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// per-session consecutive-failure counts
    pub(crate) fail_mu: Mutex<HashMap<String, usize>>,
    /// in-flight async compaction handles
    in_flight_mu: Mutex<HashMap<String, Arc<InFlightCompaction>>>,
}

impl Manager {
    pub fn new(
        summarizer: Arc<Summarizer>,
        preserve_turns: usize,
        threshold: f64,
        message_cap: i32,
    ) -> Self {
        Manager {
            summarizer,
            preserve_turns,
            threshold,
            message_cap,
            locks: Mutex::new(HashMap::new()),
            fail_mu: Mutex::new(HashMap::new()),
            in_flight_mu: Mutex::new(HashMap::new()),
        }
    }

    /// maybe_compact runs a compaction pass on sess if the session has more
    /// than K user turns. Safe to call concurrently; calls serialize per-session.
    ///
    /// Errors are returned only for true unexpected failures. Routine "skip"
    /// outcomes (too short, empty summary, provider error) come back via
    /// CompactionResult.skipped with Ok so callers can treat them uniformly.
    pub async fn maybe_compact(
        &self,
        sess: &Session,
        reason: Reason,
        instructions: &str,
    ) -> anyhow::Result<CompactionResult> {
        let key = stable_key(sess);

        let fc = self.failure_count(&key);
        if fc >= MAX_CONSECUTIVE_FAILURES {
            info!(
                session_id = %sess.id,
                reason = %reason,
                skipped = "circuit_breaker",
                consecutive_failures = fc,
                "compaction skipped"
            );
            return Ok(CompactionResult {
                reason: Some(reason),
                skipped: "circuit_breaker".to_string(),
                ..Default::default()
            });
        }

        let k = if self.preserve_turns == 0 { 4 } else { self.preserve_turns };

        let session_lock = self.lock_for(&key);
        let _guard = session_lock.lock().await;

        let start = Instant::now();
        let view = sess.view();
        let (to_compact, to_preserve) = match split(&view, k) {
            None => {
                debug!(
                    session_id = %sess.id,
                    reason = %reason,
                    skipped = "too_short",
                    "compaction skipped"
                );
                return Ok(CompactionResult {
                    reason: Some(reason),
                    skipped: "too_short".to_string(),
                    ..Default::default()
                });
            }
            Some(pair) => pair,
        };

        info!(session_id = %sess.id, reason = %reason, "compaction triggered");

        let summary = match self.summarizer.summarize(&to_compact, instructions).await {
            Err(e) => {
                let skip_reason = classify_summarizer_error(&e);
                warn!(
                    session_id = %sess.id,
                    reason = %reason,
                    skipped = %skip_reason,
                    detail = %e,
                    "compaction skipped"
                );
                self.increment_failure(&key);
                return Ok(CompactionResult {
                    reason: Some(reason),
                    skipped: skip_reason,
                    ..Default::default()
                });
            }
            Ok(s) => s,
        };

        // Detect placeholder summaries (stage-3 fallback) for circuit-breaker
        // accounting. Real summaries reset the counter.
        let is_placeholder =
            summary.contains("compaction failed and the summary could not be generated");
        if is_placeholder {
            self.increment_failure(&key);
        } else {
            self.reset_failures(&key);
        }

        let first = &to_compact[0];
        let last = &to_compact[to_compact.len() - 1];

        // Build the compaction entry.
        let mut entry = compaction_entry(
            &summary,
            &first.id,
            &last.id,
            &self.summarizer.model,
            0,
            0,
            to_compact.len() as i64,
        );

        // Splice the compaction entry between the to-be-compacted range and the
        // preserved range so View()'s walk-back from leaf hits:
        //   leaf → ... → preserved[0] → compaction → STOP.
        entry.parent_id = to_preserve[0].parent_id.clone();
        sess.append(entry);

        // Re-append preserved entries. The first preserved entry must re-parent
        // to the newly appended compaction entry.
        let comp_id = sess.leaf_id();
        for (i, e) in to_preserve.iter().enumerate() {
            let mut e2 = e.clone();
            if i == 0 {
                e2.parent_id = comp_id.clone();
            }
            sess.append(e2);
        }

        let dur = start.elapsed().as_millis() as i64;
        info!(
            session_id = %sess.id,
            reason = %reason,
            turns_compacted = to_compact.len(),
            duration_ms = dur,
            "compaction complete"
        );

        Ok(CompactionResult {
            compacted: true,
            reason: Some(reason),
            turns_compacted: to_compact.len(),
            summary,
            duration_ms: dur,
            ..Default::default()
        })
    }

    /// maybe_compact_async starts a background compaction task for sess if one
    /// is not already in flight for that session. No-op when one is already running.
    ///
    /// Designed for the "between turns" pattern: at the end of a chat turn the
    /// runtime calls this; the next turn calls wait_for_in_flight at the top of
    /// its loop and either finds the work already done or briefly waits.
    pub fn maybe_compact_async(self: &Arc<Self>, sess: Arc<Session>, reason: Reason) {
        let key = stable_key(&sess);
        {
            let in_flight = self.in_flight_mu.lock();
            if in_flight.contains_key(&key) {
                return;
            }
        }

        let fl = Arc::new(InFlightCompaction {
            done: Arc::new(Notify::new()),
            result: Mutex::new(None),
        });

        {
            let mut in_flight = self.in_flight_mu.lock();
            if in_flight.contains_key(&key) {
                return;
            }
            in_flight.insert(key.clone(), Arc::clone(&fl));
        }

        let timeout = if self.summarizer.timeout.is_zero() {
            Duration::from_secs(60)
        } else {
            self.summarizer.timeout
        };

        let mgr = Arc::clone(self);
        let key2 = key.clone();
        tokio::spawn(async move {
            let res = tokio::time::timeout(2 * timeout, mgr.maybe_compact(&sess, reason, "")).await;
            let result = match res {
                Ok(Ok(r)) => r,
                _ => CompactionResult::default(),
            };
            *fl.result.lock() = Some(result);
            fl.done.notify_waiters();

            mgr.in_flight_mu.lock().remove(&key2);
        });
    }

    /// wait_for_in_flight blocks until any in-flight async compaction for the
    /// given session completes or until the timeout elapses.
    /// Returns Some(result) on completion, None on timeout or no in-flight task.
    pub async fn wait_for_in_flight(
        &self,
        sess: &Session,
        timeout: Duration,
    ) -> Option<CompactionResult> {
        let key = stable_key(sess);
        let fl = {
            let in_flight = self.in_flight_mu.lock();
            in_flight.get(&key).cloned()
        };
        let fl = fl?;

        tokio::select! {
            _ = fl.done.notified() => fl.result.lock().clone(),
            _ = tokio::time::sleep(timeout) => None,
        }
    }

    /// has_in_flight reports whether an async compaction is currently running
    /// for the given session.
    pub fn has_in_flight(&self, sess: &Session) -> bool {
        let key = stable_key(sess);
        self.in_flight_mu.lock().contains_key(&key)
    }

    /// forget_session removes the per-session lock, failure counter, and any
    /// in-flight tracking entry for the given session.
    pub fn forget_session(&self, sess: &Session) {
        let key = stable_key(sess);
        self.locks.lock().remove(&key);
        self.fail_mu.lock().remove(&key);
        self.in_flight_mu.lock().remove(&key);
    }

    fn lock_for(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock();
        if let Some(mu) = locks.get(key) {
            return Arc::clone(mu);
        }
        let mu = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key.to_string(), Arc::clone(&mu));
        mu
    }

    fn increment_failure(&self, key: &str) -> usize {
        let mut failures = self.fail_mu.lock();
        let count = failures.entry(key.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    fn reset_failures(&self, key: &str) {
        self.fail_mu.lock().remove(key);
    }

    fn failure_count(&self, key: &str) -> usize {
        *self.fail_mu.lock().get(key).unwrap_or(&0)
    }
}

/// stable_key returns the per-session identifier used as the map key for
/// locks, failure counters, and in-flight async compactions.
///
/// Uses AgentID + Key (both persistent across loads) rather than Session.id
/// (a per-load instance id), so async compaction kicked off at the end of one
/// turn is observable by the next turn's wait_for_in_flight.
///
/// Falls back to Session.id when AgentID and Key are both empty (some tests
/// use Session::new("","") and rely on per-instance scoping).
pub(crate) fn stable_key(sess: &Session) -> String {
    if sess.agent_id.is_empty() && sess.key.is_empty() {
        return sess.id.clone();
    }
    format!("{}/{}", sess.agent_id, sess.key)
}

/// classify_summarizer_error maps an error to a skip reason string.
fn classify_summarizer_error(err: &anyhow::Error) -> String {
    let s = err.to_string();
    if s.contains("empty summary") {
        return "empty_summary".to_string();
    }
    if s.contains("deadline exceeded")
        || s.contains("timed out")
        || s.contains("DeadlineExceeded")
    {
        return "timeout".to_string();
    }
    if s.contains("canceled") || s.contains("cancelled") {
        return "cancelled".to_string();
    }
    if is_context_overflow(err) {
        return "context_overflow".to_string();
    }
    "summarizer_error".to_string()
}

#[cfg(test)]
#[path = "compaction_test.rs"]
mod compaction_test;