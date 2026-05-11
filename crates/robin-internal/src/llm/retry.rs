use std::fmt;

/// AnthropicApiError wraps an HTTP status from the Anthropic API.
#[derive(Debug)]
pub struct AnthropicApiError {
    pub status_code: u16,
    pub message: String,
}

impl fmt::Display for AnthropicApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "anthropic api error {}: {}", self.status_code, self.message)
    }
}

impl std::error::Error for AnthropicApiError {}

/// OpenAiApiError wraps an HTTP status from the OpenAI API.
#[derive(Debug)]
pub struct OpenAiApiError {
    pub status_code: u16,
    pub message: String,
}

impl fmt::Display for OpenAiApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "openai api error {}: {}", self.status_code, self.message)
    }
}

impl std::error::Error for OpenAiApiError {}

/// is_retryable_model_error reports whether err is a transient capacity
/// failure from a hosted LLM provider. Recognised:
///   - Anthropic 429 and 529
///   - OpenAI 429 and 5xx
pub fn is_retryable_model_error(err: &anyhow::Error) -> bool {
    if let Some(e) = err.downcast_ref::<AnthropicApiError>() {
        return e.status_code == 429 || e.status_code == 529;
    }
    if let Some(e) = err.downcast_ref::<OpenAiApiError>() {
        return e.status_code == 429 || (e.status_code >= 500 && e.status_code < 600);
    }

    // Last-ditch: substring match on common error messages.
    let msg = err.to_string().to_lowercase();
    msg.contains("429") || msg.contains("529")
        || msg.contains("rate limit") || msg.contains("overloaded")
}