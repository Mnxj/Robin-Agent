use tracing::Subscriber;
use tracing_subscriber::layer::Layer;

/// FanoutHandler dispatches every tracing event to multiple inner layers.
/// Used to plumb log records into both the local TextHandler and the OTel
/// bridge so the same record reaches the local log file and the configured
/// OTLP/logs collector.
pub struct FanoutHandler<S> {
    children: Vec<Box<dyn Layer<S> + Send + Sync + 'static>>,
}

impl<S: Subscriber> FanoutHandler<S> {
    /// new_fanout_handler returns a Layer that dispatches to all supplied non-nil children.
    /// If only one is provided it is returned directly to avoid dispatch overhead.
    pub fn new(children: Vec<Box<dyn Layer<S> + Send + Sync + 'static>>) -> Self {
        FanoutHandler { children }
    }
}

impl<S: Subscriber> Layer<S> for FanoutHandler<S> {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        for child in &self.children {
            child.on_event(event, ctx.clone());
        }
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        for child in &self.children {
            child.on_new_span(attrs, id, ctx.clone());
        }
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        for child in &self.children {
            child.on_record(id, values, ctx.clone());
        }
    }

    fn on_enter(&self, id: &tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        for child in &self.children {
            child.on_enter(id, ctx.clone());
        }
    }

    fn on_exit(&self, id: &tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        for child in &self.children {
            child.on_exit(id, ctx.clone());
        }
    }

    fn on_close(&self, id: tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        for child in &self.children {
            child.on_close(id.clone(), ctx.clone());
        }
    }
}