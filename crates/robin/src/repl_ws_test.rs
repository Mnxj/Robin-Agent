#[cfg(test)]
mod tests {
    use crate::repl_ws::{gateway_base_url, http_to_ws, probe_gateway, render_turn_event};
    use std::time::Duration;

    #[test]
    fn test_gateway_base_url_defaults() {
        assert_eq!("http://127.0.0.1:18789", gateway_base_url("", 0));
        assert_eq!("http://10.0.0.1:9000", gateway_base_url("10.0.0.1", 9000));
    }

    #[test]
    fn test_http_to_ws() {
        let cases: &[(&str, &str, bool)] = &[
            ("http://127.0.0.1:18789", "ws://127.0.0.1:18789/ws", false),
            ("https://gateway.example.com:443", "wss://gateway.example.com:443/ws", false),
            ("ftp://nope", "", true),
            (":not-a-url", "", true),
        ];
        for (input, expected, want_err) in cases {
            let result = http_to_ws(input);
            if *want_err {
                assert!(result.is_err(), "expected error for {input}");
            } else {
                assert_eq!(result.unwrap(), *expected, "input={input}");
            }
        }
    }

    #[test]
    fn test_render_turn_event_accumulates_text_and_flushes_on_done() {
        let mut buf = String::new();
        let done = render_turn_event(&serde_json::json!({"type":"text_delta","text":"foo"}), &mut buf).unwrap();
        assert!(!done);
        assert_eq!("foo", buf);

        let done2 = render_turn_event(&serde_json::json!({"type":"text_delta","text":" bar"}), &mut buf).unwrap();
        assert!(!done2);
        assert_eq!("foo bar", buf);

        let done3 = render_turn_event(&serde_json::json!({"type":"done"}), &mut buf).unwrap();
        assert!(done3);
        assert!(buf.is_empty(), "buffer should be reset after flush");
    }
}