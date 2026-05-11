#[cfg(test)]
mod tests {
    use crate::otel::otel::{disabled, normalize_endpoint, setup, Config};

    #[test]
    fn test_normalize_endpoint_strip_trailing_slash() {
        let got = normalize_endpoint(
            "http://aiap-nprd-otel-inet-nlb-6752644d1ca24fad.elb.ap-southeast-1.amazonaws.com/",
        )
        .unwrap();
        assert_eq!(
            got,
            "http://aiap-nprd-otel-inet-nlb-6752644d1ca24fad.elb.ap-southeast-1.amazonaws.com"
        );
    }

    #[test]
    fn test_normalize_endpoint_https_with_port() {
        let got = normalize_endpoint("https://collector.example.com:4318/").unwrap();
        assert_eq!(got, "https://collector.example.com:4318");
    }

    #[test]
    fn test_normalize_endpoint_bare_host_port() {
        let got = normalize_endpoint("localhost:4318").unwrap();
        assert_eq!(got, "http://localhost:4318");
    }

    #[test]
    fn test_normalize_endpoint_strips_path() {
        let got = normalize_endpoint("https://collector.example.com/some/path").unwrap();
        assert_eq!(got, "https://collector.example.com");
    }

    #[test]
    fn test_normalize_endpoint_empty_errors() {
        assert!(normalize_endpoint("").is_err());
    }

    #[test]
    fn test_normalize_endpoint_no_host_errors() {
        assert!(normalize_endpoint("http://").is_err());
    }

    #[tokio::test]
    async fn test_setup_disabled() {
        let p = setup(Config { enabled: false, ..Default::default() }).await.unwrap();
        assert!(p.tracer_provider.is_none());
        assert!(p.meter_provider.is_none());
        assert!(p.logger_provider.is_none());
        // Safe to call shutdown on a disabled provider.
        p.shutdown();
    }

    #[test]
    fn test_disabled_provider_is_safe() {
        let p = disabled();
        // These return valid (no-op) instruments without panicking.
        let _tracer = p.tracer("test");
        let _meter = p.meter("test");
        p.shutdown();
    }
}