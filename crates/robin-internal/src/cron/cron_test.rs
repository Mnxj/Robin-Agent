#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicI32, Ordering},
        Arc,
    };
    use std::time::Duration;

    use futures::future::BoxFuture;
    use tokio_util::sync::CancellationToken;

    use crate::cron::cron::{AgentFunc, Job, Scheduler};

    fn make_agent_fn(counter: Arc<AtomicI32>) -> AgentFunc {
        Arc::new(move |_cancel: CancellationToken, prompt: String| {
            let _ = prompt;
            let counter = Arc::clone(&counter);
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok("done".to_string())
            }) as BoxFuture<'static, anyhow::Result<String>>
        })
    }

    #[tokio::test]
    async fn test_scheduler_add_and_run() {
        let call_count = Arc::new(AtomicI32::new(0));
        let s = Arc::new(Scheduler::new());
        s.add(Job {
            name: "test-job".to_string(),
            schedule: "50ms".to_string(),
            prompt: "do something".to_string(),
            paused: false,
            agent_fn: make_agent_fn(Arc::clone(&call_count)),
            output_fn: None,
            interval: Duration::ZERO,
        })
        .unwrap();

        assert_eq!(s.jobs().len(), 1);

        let cancel = CancellationToken::new();
        s.start(cancel.clone());
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();

        assert!(call_count.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn test_scheduler_invalid_schedule() {
        let s = Scheduler::new();
        let counter = Arc::new(AtomicI32::new(0));
        let err = s.add(Job {
            name: "bad-job".to_string(),
            schedule: "invalid".to_string(),
            prompt: "test".to_string(),
            paused: false,
            agent_fn: make_agent_fn(counter),
            output_fn: None,
            interval: Duration::ZERO,
        });
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_scheduler_stop() {
        let s = Arc::new(Scheduler::new());
        let counter = Arc::new(AtomicI32::new(0));
        s.add(Job {
            name: "slow-job".to_string(),
            schedule: "1h".to_string(),
            prompt: "test".to_string(),
            paused: false,
            agent_fn: make_agent_fn(counter),
            output_fn: None,
            interval: Duration::ZERO,
        })
        .unwrap();

        let cancel = CancellationToken::new();
        s.start(cancel.clone());

        tokio::time::timeout(Duration::from_secs(2), async {
            cancel.cancel();
        })
        .await
        .expect("Stop did not return in time");
    }

    #[tokio::test]
    async fn test_scheduler_multiple_jobs() {
        let count1 = Arc::new(AtomicI32::new(0));
        let count2 = Arc::new(AtomicI32::new(0));

        let s = Arc::new(Scheduler::new());
        s.add(Job {
            name: "job1".to_string(),
            schedule: "50ms".to_string(),
            prompt: "first".to_string(),
            paused: false,
            agent_fn: make_agent_fn(Arc::clone(&count1)),
            output_fn: None,
            interval: Duration::ZERO,
        })
        .unwrap();
        s.add(Job {
            name: "job2".to_string(),
            schedule: "50ms".to_string(),
            prompt: "second".to_string(),
            paused: false,
            agent_fn: make_agent_fn(Arc::clone(&count2)),
            output_fn: None,
            interval: Duration::ZERO,
        })
        .unwrap();

        assert_eq!(s.jobs().len(), 2);

        let cancel = CancellationToken::new();
        s.start(cancel.clone());
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();

        assert!(count1.load(Ordering::SeqCst) >= 1);
        assert!(count2.load(Ordering::SeqCst) >= 1);
    }
}