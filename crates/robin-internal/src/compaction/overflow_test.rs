use super::is_context_overflow;

#[test]
fn test_is_context_overflow_cases() {
    let cases: &[(&str, bool)] = &[
        ("anthropic: request_too_large: prompt is too long", true),
        ("context length exceeded for model claude", true),
        ("input is too long for the model", true),
        ("openai: error code 400 — context_length_exceeded", true),
        ("This model's maximum context length is 8192 tokens", true),
        ("gemini: input token count exceeds the maximum", true),
        ("request payload size exceeds the limit", true),
        ("connection refused", false),
        ("401 unauthorized", false),
    ];

    for (msg, want) in cases {
        let err = anyhow::anyhow!("{}", msg);
        assert_eq!(
            is_context_overflow(&err),
            *want,
            "is_context_overflow({:?}) should be {}",
            msg,
            want
        );
    }
}

#[test]
fn test_is_context_overflow_nil_equivalent() {
    let err = anyhow::anyhow!("some unrelated error");
    assert!(!is_context_overflow(&err));
}

#[test]
fn test_is_context_overflow_case_insensitive() {
    let err1 = anyhow::anyhow!("CONTEXT LENGTH EXCEEDED");
    assert!(is_context_overflow(&err1));
    let err2 = anyhow::anyhow!("Request_Too_Large");
    assert!(is_context_overflow(&err2));
}
