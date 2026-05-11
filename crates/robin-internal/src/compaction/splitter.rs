use crate::session::{EntryType, SessionEntry};

/// Split divides history into (to_compact, to_preserve) at a clean turn boundary.
///
/// Algorithm: walk backwards from the leaf, count user messages. After we have
/// seen K of them, the next encountered user message is the cutoff. Everything
/// from that user message forward is preserved verbatim; everything before is
/// the to-be-compacted range.
///
/// Returns `None` when the path contains <= K user messages — there is no
/// cutoff that preserves K turns. Caller should refuse to compact rather than
/// over-compacting.
///
/// A user message is always a clean boundary by construction in Robin's
/// runtime (user msg → assistant text → tool_call → tool_result → next user
/// msg). Splitting before a user message therefore never orphans a tool pair.
pub fn split(
    history: &[SessionEntry],
    k: usize,
) -> Option<(Vec<SessionEntry>, Vec<SessionEntry>)> {
    if k == 0 || history.is_empty() {
        return None;
    }

    // Walk backwards counting user messages. cutoff_idx will land on the
    // (K+1)-th user message from the end (i.e. the first user message that
    // belongs to the to-be-compacted range — preserved range starts at the
    // next user message we already counted).
    let mut user_count = 0usize;
    let mut cutoff_idx: Option<usize> = None;
    for i in (0..history.len()).rev() {
        let e = &history[i];
        if e.entry_type != EntryType::Message || e.role != "user" {
            continue;
        }
        user_count += 1;
        if user_count > k {
            cutoff_idx = Some(i);
            break;
        }
    }
    let cutoff_idx = cutoff_idx?;

    // Find the next user message AFTER cutoff_idx — that is the start of the
    // preserved range. Everything strictly before it is compacted.
    let mut preserve_start: Option<usize> = None;
    for i in (cutoff_idx + 1)..history.len() {
        let e = &history[i];
        if e.entry_type == EntryType::Message && e.role == "user" {
            preserve_start = Some(i);
            break;
        }
    }
    let preserve_start = preserve_start?;

    let to_compact = history[..preserve_start].to_vec();
    let to_preserve = history[preserve_start..].to_vec();
    Some((to_compact, to_preserve))
}

#[cfg(test)]
#[path = "splitter_test.rs"]
mod splitter_test;