#[cfg(test)]
mod tests {
    use super::super::adapter::is_auth_failure;
    use super::super::manager::{Manager, ServerEntry};
    use super::super::types::{HttpAuthConfig, HttpServerConfig, ManagerServerConfig};

    fn make_entry(id: &str) -> ServerEntry {
        ServerEntry {
            id: id.to_owned(),
            tool_prefix: String::new(),
            parallel_safe: false,
            client: parking_lot::RwLock::new(None),
            cfg: ManagerServerConfig {
                id: id.to_owned(),
                tool_prefix: String::new(),
                transport: "http".to_owned(),
                http: None,
                stdio: None,
                parallel_safe: false,
            },
            consecutive_failures: parking_lot::Mutex::new(0),
        }
    }

    #[test]
    fn test_is_auth_failure_recognizes_common_signatures() {
        let cases: Vec<(&str, &str, bool)> = vec![
            ("401_status",                  "unexpected status 401: bad token",                                         true),
            ("403_status",                  "unexpected status 403: forbidden",                                         true),
            ("unauthorized",                "HTTP 401: Unauthorized",                                                   true),
            ("unauthenticated_grpc",        "code = Unauthenticated desc = invalid token",                             true),
            ("invalid_token_oauth",         "oauth2: server response: invalid_token",                                  true),
            ("token_expired",               "access token has expired",                                                 true),
            ("session_expired",             "MCP session expired, please reconnect",                                    true),
            ("expired_token",               "expired_token: refresh required",                                          true),
            ("access_denied",               "access denied",                                                            true),
            ("permission_denied",           "permission denied",                                                        true),
            ("session_not_found",           "mcp tools/call echo: session not found",                                  true),
            ("session_not_found_underscore","server returned session_not_found",                                        true),
            ("session_terminated",          "mcp: session terminated by server",                                        true),
            ("session_no_longer_valid",     "mcp: session is no longer valid",                                         true),
            ("must_reauthenticate",         "upstream says you must re-authenticate",                                   true),
            ("please_reauthenticate",       "please re-authenticate to continue",                                       true),
            ("invalid_grant",              r#"oauth2: server response: {"error":"invalid_grant"}"#,                    true),
            ("oauth2_cannot_fetch_token",   "oauth2: cannot fetch token: 400 Bad Request",                             true),
            ("client_is_closing",           r#"mcp tools/call x: connection closed: calling "tools/call": client is closing: sending "tools/call": Bad Request"#, true),
            ("connection_closed_calling",   r#"connection closed: calling "tools/list": context canceled"#,            true),
            ("sending_tools_call_bad_req",  r#"mcp tools/call x: calling "tools/call": sending "tools/call": Bad Request"#, true),

            ("nil_like",                    "",                                                                          false),
            ("context_canceled",            "context canceled",                                                         false),
            ("network_unreachable",         "dial tcp: connection refused",                                             false),
            ("timeout",                     "Client.Timeout exceeded while awaiting headers",                           false),
            ("500_server_error",            "HTTP 500: internal server error",                                          false),
            ("tool_validation",             "invalid arguments: missing field \"query\"",                               false),
            ("503_no_longer_available",     "HTTP 503: service is temporarily unavailable",                            false),
            ("plain_bad_request",           "HTTP 400: Bad Request from upstream cdn",                                 false),
        ];

        for (name, msg, want) in cases {
            let err = anyhow::anyhow!("{}", msg);
            assert_eq!(
                is_auth_failure(&err), want,
                "case {:?}: is_auth_failure({:?}) = {}, want {}",
                name, msg, is_auth_failure(&err), want
            );
        }
    }

    #[tokio::test]
    async fn test_reconnect_server_unknown_id_errors() {
        let mgr = Manager { servers: vec![] };
        let err = mgr.reconnect_server("ghost").await.unwrap_err();
        assert!(err.to_string().contains("ghost"), "error should mention 'ghost', got: {}", err);
    }

    #[test]
    fn test_server_entry_live_returns_none_when_no_client() {
        let entry = make_entry("x");
        // live() should return None when no client is set.
        assert!(entry.live().is_none());
    }

    #[test]
    fn test_server_entry_failure_count_tracking() {
        // Verify the basic failure-count / success-reset property.
        // The full Reconnect-driven swap is tested in adapter_test.
        let entry = make_entry("x");
        assert_eq!(entry.failure_count(), 0);
        entry.record_failure();
        assert_eq!(entry.failure_count(), 1);
        entry.record_success();
        assert_eq!(entry.failure_count(), 0);
    }
}