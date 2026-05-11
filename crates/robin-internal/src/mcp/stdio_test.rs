#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::stdio::{connect_stdio, merged_env};

    #[test]
    fn test_merged_env_overrides_and_appends() {
        let parent = vec![
            "PATH=/usr/bin".to_owned(),
            "HOME=/root".to_owned(),
            "FOO=old".to_owned(),
        ];
        let mut overrides = HashMap::new();
        overrides.insert("FOO".to_owned(), "new".to_owned());
        overrides.insert("BAR".to_owned(), "fresh".to_owned());

        let got = merged_env(parent, &overrides);

        // Build a lookup map.
        let m: HashMap<String, String> = got.iter()
            .map(|kv| {
                let eq = kv.find('=').unwrap();
                (kv[..eq].to_owned(), kv[eq + 1..].to_owned())
            })
            .collect();

        assert_eq!(m.get("PATH").map(|s| s.as_str()), Some("/usr/bin"), "PATH should be inherited");
        assert_eq!(m.get("HOME").map(|s| s.as_str()), Some("/root"), "HOME should be inherited");
        assert_eq!(m.get("FOO").map(|s| s.as_str()), Some("new"), "FOO should be overridden");
        assert_eq!(m.get("BAR").map(|s| s.as_str()), Some("fresh"), "BAR should be appended");

        // Length: parent (3) + 1 new key (BAR); FOO is replaced in place.
        assert_eq!(got.len(), 4);
    }

    #[test]
    fn test_merged_env_nil_overrides_returns_parent() {
        let parent = vec!["A=1".to_owned(), "B=2".to_owned()];
        let got = merged_env(parent.clone(), &HashMap::new());
        assert_eq!(got, parent);
    }

    #[test]
    fn test_connect_stdio_nonexistent_binary_fails() {
        let result = connect_stdio("test-bad", "/no/such/binary-robin-test", &[], &HashMap::new());
        assert!(result.is_err(), "connecting to a nonexistent binary should fail");
    }

    #[test]
    fn test_connect_stdio_empty_command_fails() {
        let result = connect_stdio("test-empty", "", &[], &HashMap::new());
        assert!(result.is_err(), "empty command should fail");
        assert!(result.unwrap_err().to_string().contains("empty command"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn test_connect_stdio_handshake_fails_on_non_mcp_process() {
        // Spawn `cat` (which doesn't speak MCP) and assert ConnectStdio fails
        // at the JSON-RPC handshake. Skip if cat is not in PATH.
        if which::which("cat").is_err() {
            return;
        }
        let result = connect_stdio("cat-test", "cat", &[], &HashMap::new());
        assert!(result.is_err(), "cat does not speak MCP; handshake must fail");
    }
}
