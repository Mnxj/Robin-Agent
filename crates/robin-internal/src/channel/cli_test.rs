#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use crate::channel::channel::{Channel, ChannelStatus, OutboundMessage};
    use crate::channel::cli::CLIChannel;

    #[test]
    fn test_new_cli_channel_initial_state() {
        let c = CLIChannel::new();
        assert_eq!(c.status(), ChannelStatus::Disconnected);
        assert_eq!(c.name(), "cli");
        // receive() must return a valid Arc even before connect
        let _rx = c.receive();
    }

    #[tokio::test]
    async fn test_cli_channel_connect_flips_status() {
        let c = CLIChannel::new();
        let cancel = CancellationToken::new();
        c.connect(cancel.clone()).await.unwrap();
        assert_eq!(c.status(), ChannelStatus::Connected);

        c.disconnect().await.unwrap();
        assert_eq!(c.status(), ChannelStatus::Disconnected);
    }

    #[tokio::test]
    async fn test_cli_channel_disconnect_without_connect() {
        let c = CLIChannel::new();
        assert!(c.disconnect().await.is_ok());
        assert_eq!(c.status(), ChannelStatus::Disconnected);
    }

    #[tokio::test]
    async fn test_cli_channel_double_disconnect() {
        let c = CLIChannel::new();
        let cancel = CancellationToken::new();
        c.connect(cancel).await.unwrap();
        assert!(c.disconnect().await.is_ok());
        assert!(c.disconnect().await.is_ok());
    }

    #[tokio::test]
    async fn test_cli_channel_send_returns_no_error() {
        let c = CLIChannel::new();
        assert!(c
            .send(OutboundMessage {
                text: "hello world".to_string(),
                ..Default::default()
            })
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_cli_channel_receive_channel_stable() {
        use crate::channel::channel::InboundMessage;
        use std::time::Duration;

        let c = CLIChannel::new();
        let a = c.receive();
        let b = c.receive();

        // Both arcs point to the same underlying receiver
        assert!(Arc::ptr_eq(&a, &b));

        c.inbound_tx
            .send(InboundMessage {
                text: "ping".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(1), async {
            a.lock().await.recv().await
        })
        .await
        .expect("timeout")
        .expect("channel closed");
        assert_eq!(msg.text, "ping");
    }

    #[test]
    fn test_cli_channel_interface_compliance() {
        fn _assert_channel(_: &dyn Channel) {}
        let c = CLIChannel::new();
        _assert_channel(&c);
    }

    use std::sync::Arc;
}