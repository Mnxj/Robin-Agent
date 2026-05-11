use std::time::SystemTime;

use async_trait::async_trait;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl std::fmt::Display for ChannelStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "disconnected"),
            Self::Connecting => write!(f, "connecting"),
            Self::Connected => write!(f, "connected"),
            Self::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatType {
    Direct,
    Group,
}

impl std::fmt::Display for ChatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => write!(f, "direct"),
            Self::Group => write!(f, "group"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MediaAttachment {
    pub r#type: String,
    pub file_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub caption: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub channel: String,
    pub account_id: String,
    pub chat_type: ChatType,
    pub sender_id: String,
    pub sender_name: String,
    pub text: String,
    pub reply_to: String,
    pub media: Vec<MediaAttachment>,
    pub timestamp: SystemTime,
}

impl Default for InboundMessage {
    fn default() -> Self {
        Self {
            channel: String::new(),
            account_id: String::new(),
            chat_type: ChatType::Direct,
            sender_id: String::new(),
            sender_name: String::new(),
            text: String::new(),
            reply_to: String::new(),
            media: Vec::new(),
            timestamp: SystemTime::now(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OutboundMessage {
    pub chat_id: String,
    pub text: String,
    pub parse_mode: String,
    pub reply_markup: Option<serde_json::Value>,
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn connect(&self, cancel: tokio_util::sync::CancellationToken) -> anyhow::Result<()>;
    async fn disconnect(&self) -> anyhow::Result<()>;
    async fn send(&self, msg: OutboundMessage) -> anyhow::Result<()>;
    fn receive(&self) -> std::sync::Arc<tokio::sync::Mutex<mpsc::Receiver<InboundMessage>>>;
    fn status(&self) -> ChannelStatus;
}