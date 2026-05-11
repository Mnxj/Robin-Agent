// Package-level doc: compaction provides session compaction — detect long
// sessions, summarize the older portion, preserve recent turns verbatim.

/// overflowSignatures lists substrings that indicate a model returned a
/// context-window-too-long error. All matches are case-insensitive.
static OVERFLOW_SIGNATURES: &[&str] = &[
    // Anthropic
    "request_too_large",
    "context length exceeded",
    "input is too long",
    // OpenAI
    "context_length_exceeded",
    "maximum context length",
    // Gemini
    "input token count exceeds",
    "request payload size exceeds",
];

/// Reports whether err looks like a provider returning "your prompt is too
/// big". Reactive compaction triggers on this.
pub fn is_context_overflow(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    for sig in OVERFLOW_SIGNATURES {
        if msg.contains(sig) {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "overflow_test.rs"]
mod overflow_test;