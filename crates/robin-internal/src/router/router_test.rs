#[cfg(test)]
mod tests {
    use crate::channel::channel::{ChatType, InboundMessage};
    use crate::config::config::{Binding, BindingMatch, PeerMatch};
    use crate::router::router::Router;

    fn make_msg(channel: &str, sender_id: &str) -> InboundMessage {
        InboundMessage {
            channel: channel.to_string(),
            sender_id: sender_id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_router_channel_match() {
        let r = Router::new(
            vec![
                Binding {
                    agent_id: "agent-tg".to_string(),
                    r#match: BindingMatch {
                        channel: "telegram".to_string(),
                        ..Default::default()
                    },
                },
                Binding {
                    agent_id: "agent-cli".to_string(),
                    r#match: BindingMatch {
                        channel: "cli".to_string(),
                        ..Default::default()
                    },
                },
            ],
            "fallback",
        );

        let mut msg = make_msg("cli", "user1");
        assert_eq!(r.route(&msg), "agent-cli");

        msg.channel = "telegram".to_string();
        assert_eq!(r.route(&msg), "agent-tg");
    }

    #[test]
    fn test_router_peer_match() {
        let r = Router::new(
            vec![
                Binding {
                    agent_id: "vip-agent".to_string(),
                    r#match: BindingMatch {
                        channel: "telegram".to_string(),
                        peer: Some(PeerMatch {
                            id: "user123".to_string(),
                            kind: String::new(),
                        }),
                        ..Default::default()
                    },
                },
                Binding {
                    agent_id: "default-tg".to_string(),
                    r#match: BindingMatch {
                        channel: "telegram".to_string(),
                        ..Default::default()
                    },
                },
            ],
            "fallback",
        );

        let mut msg = make_msg("telegram", "user123");
        assert_eq!(r.route(&msg), "vip-agent");

        msg.sender_id = "other".to_string();
        assert_eq!(r.route(&msg), "default-tg");
    }

    #[test]
    fn test_router_fallback() {
        let r = Router::new(
            vec![Binding {
                agent_id: "agent-tg".to_string(),
                r#match: BindingMatch {
                    channel: "telegram".to_string(),
                    ..Default::default()
                },
            }],
            "default",
        );

        let msg = make_msg("unknown", "user1");
        assert_eq!(r.route(&msg), "default");
    }
}