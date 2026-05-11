#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;

    use super::super::creds::{load_env_file, load_token, require_keys, save_token, OAuthTokenStore};

    #[test]
    fn test_load_env_file_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds.txt");
        std::fs::write(&path, concat!(
            "# comment line\n",
            "\n",
            "MCP_SERVER_URL=https://example.com/mcp\n",
            "  LTM_CLIENT_ID=abc123  \n",
            "LTM_CLIENT_SECRET=shhh=with=equals\n",
            "LTM_TOKEN_URL=https://auth.example.com/token\n",
            "LTM_SCOPE=foo/bar\n",
        )).unwrap();

        let got = load_env_file(path.to_str().unwrap()).unwrap();
        let expected: HashMap<String, String> = [
            ("MCP_SERVER_URL", "https://example.com/mcp"),
            ("LTM_CLIENT_ID", "abc123"),
            ("LTM_CLIENT_SECRET", "shhh=with=equals"),
            ("LTM_TOKEN_URL", "https://auth.example.com/token"),
            ("LTM_SCOPE", "foo/bar"),
        ].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn test_load_env_file_missing_file() {
        let result = load_env_file("/nonexistent/path/nope.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_env_file_malformed_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.txt");
        std::fs::write(&path, "this line has no equals\n").unwrap();
        let err = load_env_file(path.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("line 1"), "error should mention line 1, got: {}", err);
    }

    #[test]
    fn test_load_env_file_empty_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.txt");
        std::fs::write(&path, "=value\n").unwrap();
        let err = load_env_file(path.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("empty key"), "error should mention empty key, got: {}", err);
    }

    #[test]
    fn test_require_keys_present() {
        let env: HashMap<String, String> = [("A", "1"), ("B", "2")]
            .iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert!(require_keys(&env, &["A", "B"]).is_ok());
    }

    #[test]
    fn test_require_keys_missing() {
        let env: HashMap<String, String> = [("A", "1"), ("B", "2")]
            .iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let err = require_keys(&env, &["A", "C", "D"]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains('C'), "error should mention C, got: {}", msg);
        assert!(msg.contains('D'), "error should mention D, got: {}", msg);
        assert!(!msg.contains('A') || msg.contains("A, "), "A should not be in missing list");
    }

    #[test]
    fn test_load_token_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let result = load_token(path.to_str().unwrap()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_token_empty_path_errors() {
        let result = load_token("");
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load_token_roundtrip() {
        use chrono::Timelike as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tok.json");
        let expiry = chrono::Utc::now() + chrono::Duration::hours(1);
        // Round to seconds for comparison.
        let expiry = expiry.with_nanosecond(0).unwrap_or(expiry);

        let input = OAuthTokenStore {
            access_token: "acc".to_owned(),
            refresh_token: "ref".to_owned(),
            token_type: "Bearer".to_owned(),
            expiry: Some(expiry),
        };
        save_token(path.to_str().unwrap(), &input).unwrap();

        let output = load_token(path.to_str().unwrap()).unwrap().unwrap();
        assert_eq!(output.access_token, "acc");
        assert_eq!(output.refresh_token, "ref");
        // Expiry should be within 1 second.
        let out_expiry = output.expiry.unwrap();
        let diff = (out_expiry - expiry).num_seconds().abs();
        assert!(diff <= 1, "expiry should be within 1 second, diff = {}", diff);
    }

    #[test]
    #[cfg(unix)]
    fn test_save_token_file_perms_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("tok.json");
        let tok = OAuthTokenStore {
            access_token: "x".to_owned(),
            refresh_token: String::new(),
            token_type: String::new(),
            expiry: None,
        };
        save_token(path.to_str().unwrap(), &tok).unwrap();

        let info = std::fs::metadata(&path).unwrap();
        assert_eq!(info.permissions().mode() & 0o777, 0o600);

        let dir_info = std::fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(dir_info.permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn test_token_is_usable() {
        let expired = OAuthTokenStore {
            access_token: "a".to_owned(),
            refresh_token: String::new(),
            token_type: String::new(),
            expiry: Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
        };
        assert!(!expired.is_usable());

        let good = OAuthTokenStore {
            access_token: "a".to_owned(),
            refresh_token: String::new(),
            token_type: String::new(),
            expiry: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        };
        assert!(good.is_usable());

        let long_lived = OAuthTokenStore {
            access_token: "a".to_owned(),
            refresh_token: String::new(),
            token_type: String::new(),
            expiry: None,
        };
        assert!(long_lived.is_usable());

        let empty = OAuthTokenStore {
            access_token: String::new(),
            refresh_token: String::new(),
            token_type: String::new(),
            expiry: None,
        };
        assert!(!empty.is_usable());
    }
}
