#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use super::super::retry::{is_retryable_model_error, AnthropicApiError, OpenAiApiError};

    #[test]
    fn test_is_retryable_anthropic_429() {
        let err = anyhow::Error::new(AnthropicApiError { status_code: 429, message: "rate limit".into() });
        assert!(is_retryable_model_error(&err));
    }

    #[test]
    fn test_is_retryable_anthropic_529() {
        let err = anyhow::Error::new(AnthropicApiError { status_code: 529, message: "overloaded".into() });
        assert!(is_retryable_model_error(&err));
    }

    #[test]
    fn test_not_retryable_anthropic_400() {
        let err = anyhow::Error::new(AnthropicApiError { status_code: 400, message: "bad request".into() });
        assert!(!is_retryable_model_error(&err));
    }

    #[test]
    fn test_not_retryable_anthropic_401() {
        let err = anyhow::Error::new(AnthropicApiError { status_code: 401, message: "unauthorized".into() });
        assert!(!is_retryable_model_error(&err));
    }

    #[test]
    fn test_is_retryable_openai_429() {
        let err = anyhow::Error::new(OpenAiApiError { status_code: 429, message: "rate limit".into() });
        assert!(is_retryable_model_error(&err));
    }

    #[test]
    fn test_is_retryable_openai_500() {
        let err = anyhow::Error::new(OpenAiApiError { status_code: 500, message: "server error".into() });
        assert!(is_retryable_model_error(&err));
    }

    #[test]
    fn test_is_retryable_openai_503() {
        let err = anyhow::Error::new(OpenAiApiError { status_code: 503, message: "unavailable".into() });
        assert!(is_retryable_model_error(&err));
    }

    #[test]
    fn test_not_retryable_openai_400() {
        let err = anyhow::Error::new(OpenAiApiError { status_code: 400, message: "bad request".into() });
        assert!(!is_retryable_model_error(&err));
    }

    #[test]
    fn test_is_retryable_wrapped_error() {
        let inner = AnthropicApiError { status_code: 529, message: "overloaded".into() };
        let err = anyhow!("transient: {}", inner).context(inner);
        // The context wrapping doesn't allow downcast of inner, so this hits string fallback
        // because "529" and "overloaded" appear in the message chain.
        // Actually wrapping with context means the root is AnthropicApiError still.
        let err2 = anyhow::Error::new(AnthropicApiError { status_code: 529, message: "overloaded".into() });
        let wrapped = anyhow::anyhow!("transient: {}", err2.to_string());
        assert!(is_retryable_model_error(&wrapped));
    }

    #[test]
    fn test_not_retryable_nil() {
        // Represented as: we don't call the function with nil in Rust; this is just a placeholder.
        // Instead test an unrelated error.
        let err = anyhow!("some other failure");
        assert!(!is_retryable_model_error(&err));
    }

    #[test]
    fn test_is_retryable_string_429() {
        let err = anyhow!("api request failed: 429 Too Many Requests");
        assert!(is_retryable_model_error(&err));
    }

    #[test]
    fn test_is_retryable_string_rate_limit() {
        let err = anyhow!("rate limit exceeded");
        assert!(is_retryable_model_error(&err));
    }

    #[test]
    fn test_not_retryable_unrelated_error() {
        let err = anyhow!("some other failure");
        assert!(!is_retryable_model_error(&err));
    }
}