use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::channel::{Channel, ChannelStatus, ChatType, InboundMessage, OutboundMessage};

pub struct CLIChannel {
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<InboundMessage>>>,
    status: Mutex<ChannelStatus>,
    cancel: Mutex<Option<CancellationToken>>,
}

impl CLIChannel {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(10);
        Self {
            inbound_tx: tx,
            inbound_rx: Arc::new(tokio::sync::Mutex::new(rx)),
            status: Mutex::new(ChannelStatus::Disconnected),
            cancel: Mutex::new(None),
        }
    }
}

impl Default for CLIChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Channel for CLIChannel {
    fn name(&self) -> &str {
        "cli"
    }

    async fn connect(&self, cancel: CancellationToken) -> anyhow::Result<()> {
        {
            let mut status = self.status.lock();
            *status = ChannelStatus::Connected;
            *self.cancel.lock() = Some(cancel.clone());
        }

        let tx = self.inbound_tx.clone();
        tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let mut reader = BufReader::new(stdin).lines();

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    line = reader.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let text = line.trim().to_string();
                                if text.is_empty() {
                                    continue;
                                }
                                if text == "/quit" || text == "/exit" {
                                    return;
                                }
                                let msg = InboundMessage {
                                    channel: "cli".to_string(),
                                    chat_type: ChatType::Direct,
                                    sender_id: "local".to_string(),
                                    sender_name: "User".to_string(),
                                    text,
                                    timestamp: std::time::SystemTime::now(),
                                    ..Default::default()
                                };
                                if tx.send(msg).await.is_err() {
                                    return;
                                }
                            }
                            _ => return,
                        }
                    }
                }
            }
        });

        Ok(())
    }

    async fn disconnect(&self) -> anyhow::Result<()> {
        let mut status = self.status.lock();
        *status = ChannelStatus::Disconnected;
        if let Some(cancel) = self.cancel.lock().take() {
            cancel.cancel();
        }
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        println!("{}", msg.text);
        Ok(())
    }

    fn receive(&self) -> Arc<tokio::sync::Mutex<mpsc::Receiver<InboundMessage>>> {
        Arc::clone(&self.inbound_rx)
    }

    fn status(&self) -> ChannelStatus {
        self.status.lock().clone()
    }
}