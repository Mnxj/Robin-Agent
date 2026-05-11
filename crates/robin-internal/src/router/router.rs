use crate::channel::channel::InboundMessage;
use crate::config::config::{Binding, BindingMatch};

pub struct Router {
    bindings: Vec<Binding>,
    fallback: String,
}

impl Router {
    pub fn new(bindings: Vec<Binding>, fallback_agent_id: impl Into<String>) -> Self {
        Self {
            bindings,
            fallback: fallback_agent_id.into(),
        }
    }

    /// Route returns the agent ID that should handle the given message.
    /// Matching priority: peer.id > peer.kind > account_id > channel > default.
    pub fn route(&self, msg: &InboundMessage) -> &str {
        let mut channel_match: Option<&str> = None;

        for b in &self.bindings {
            let m = &b.r#match;

            // Most specific: peer.id match
            if let Some(peer) = &m.peer {
                if !peer.id.is_empty() && peer.id == msg.sender_id {
                    if m.channel.is_empty() || m.channel == msg.channel {
                        return &b.agent_id;
                    }
                }
            }

            // Peer kind match
            if let Some(peer) = &m.peer {
                if !peer.kind.is_empty() && peer.kind == msg.chat_type.to_string() {
                    if m.channel.is_empty() || m.channel == msg.channel {
                        return &b.agent_id;
                    }
                }
            }

            // Account ID match
            if !m.account_id.is_empty() && m.account_id == msg.account_id {
                if m.channel.is_empty() || m.channel == msg.channel {
                    return &b.agent_id;
                }
            }

            // Channel match (least specific of the explicit matches)
            if m.channel == msg.channel && m.peer.is_none() && m.account_id.is_empty() {
                channel_match = Some(&b.agent_id);
            }
        }

        if let Some(ch) = channel_match {
            return ch;
        }

        &self.fallback
    }

    /// IsKnownPeer returns true if the given sender ID appears as a peer.id in any binding.
    pub fn is_known_peer(&self, sender_id: &str) -> bool {
        self.bindings.iter().any(|b| {
            b.r#match
                .peer
                .as_ref()
                .map(|p| p.id == sender_id)
                .unwrap_or(false)
        })
    }
}