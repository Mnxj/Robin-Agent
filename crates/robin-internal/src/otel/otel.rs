use std::collections::HashMap;

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_otlp::{LogExporter, MetricsExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::logs::{Config as LogConfig, LoggerProvider};
use opentelemetry_sdk::metrics::{reader::{DefaultAggregationSelector, DefaultTemporalitySelector}, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{self as sdktrace, TracerProvider};
use opentelemetry_sdk::{runtime, Resource};
use tracing::warn;
use url::Url;

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub enabled: bool,
    pub endpoint: String,
    pub service_name: String,
    pub version: String,
    pub sample_ratio: f64,
    pub headers: HashMap<String, String>,
    pub traces: bool,
    pub metrics: bool,
    pub logs: bool,
}

pub struct Provider {
    pub cfg: Config,
    pub tracer_provider: Option<TracerProvider>,
    pub meter_provider: Option<SdkMeterProvider>,
    pub logger_provider: Option<LoggerProvider>,
}

impl Provider {
    pub fn disabled() -> Self {
        Self {
            cfg: Config::default(),
            tracer_provider: None,
            meter_provider: None,
            logger_provider: None,
        }
    }

    pub fn tracer(&self, name: &str) -> opentelemetry::global::BoxedTracer {
        match &self.tracer_provider {
            Some(_) => global::tracer(name.to_string()),
            None => global::tracer(name.to_string()),
        }
    }

    pub fn meter(&self, name: &str) -> opentelemetry::metrics::Meter {
        match &self.meter_provider {
            Some(mp) => mp.meter(name.to_string()),
            None => global::meter(name.to_string()),
        }
    }

    pub fn shutdown(&self) {
        if let Some(mp) = &self.meter_provider {
            if let Err(e) = mp.shutdown() {
                warn!("otel: meter shutdown error: {}", e);
            }
        }
        if let Some(lp) = &self.logger_provider {
            if let Err(e) = lp.shutdown() {
                warn!("otel: logger shutdown error: {}", e);
            }
        }
        // TracerProvider shuts down on Drop; also call the global shutdown.
        if self.tracer_provider.is_some() {
            global::shutdown_tracer_provider();
        }
    }
}

pub fn disabled() -> Provider {
    Provider::disabled()
}

pub async fn setup(mut cfg: Config) -> anyhow::Result<Provider> {
    if !cfg.enabled {
        return Ok(disabled());
    }
    if cfg.service_name.is_empty() {
        cfg.service_name = "robin".to_string();
    }
    if cfg.sample_ratio <= 0.0 {
        cfg.sample_ratio = 1.0;
    }

    let endpoint = normalize_endpoint(&cfg.endpoint)?;
    let resource = build_resource(&cfg);
    let mut prov = Provider {
        cfg: cfg.clone(),
        tracer_provider: None,
        meter_provider: None,
        logger_provider: None,
    };

    if cfg.traces {
        match build_tracer_provider(&endpoint, &cfg.headers, resource.clone(), cfg.sample_ratio).await {
            Ok(tp) => {
                global::set_tracer_provider(tp.clone());
                prov.tracer_provider = Some(tp);
            }
            Err(e) => warn!("otel: traces disabled (exporter init failed): {}", e),
        }
    }

    if cfg.metrics {
        match build_meter_provider(&endpoint, &cfg.headers, resource.clone()).await {
            Ok(mp) => {
                prov.meter_provider = Some(mp);
            }
            Err(e) => warn!("otel: metrics disabled (exporter init failed): {}", e),
        }
    }

    if cfg.logs {
        match build_logger_provider(&endpoint, &cfg.headers, resource.clone()).await {
            Ok(lp) => {
                prov.logger_provider = Some(lp);
            }
            Err(e) => warn!("otel: logs disabled (exporter init failed): {}", e),
        }
    }

    tracing::info!(
        endpoint = %cfg.endpoint,
        service_name = %cfg.service_name,
        traces = prov.tracer_provider.is_some(),
        metrics = prov.meter_provider.is_some(),
        logs = prov.logger_provider.is_some(),
        sample_ratio = cfg.sample_ratio,
        "otel: setup complete"
    );

    Ok(prov)
}

/// Normalize the OTLP endpoint URL: strip path/query/fragment, prepend http:// if schemeless.
pub fn normalize_endpoint(raw: &str) -> anyhow::Result<String> {
    if raw.is_empty() {
        anyhow::bail!("endpoint is empty");
    }
    let raw = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{}", raw)
    };
    let mut u = Url::parse(&raw)?;
    if u.host_str().map(|h| h.is_empty()).unwrap_or(true) {
        anyhow::bail!("endpoint has no host: {:?}", raw);
    }
    u.set_path("");
    u.set_query(None);
    u.set_fragment(None);
    let s = u.to_string();
    Ok(s.trim_end_matches('/').to_string())
}

fn build_resource(cfg: &Config) -> Resource {
    use opentelemetry_semantic_conventions::resource::{HOST_NAME, SERVICE_NAME, SERVICE_VERSION};
    let mut kv = vec![opentelemetry::KeyValue::new(SERVICE_NAME, cfg.service_name.clone())];
    if let Ok(hostname) = std::process::Command::new("hostname").output() {
        let h = String::from_utf8_lossy(&hostname.stdout).trim().to_string();
        if !h.is_empty() {
            kv.push(opentelemetry::KeyValue::new(HOST_NAME, h));
        }
    }
    if !cfg.version.is_empty() {
        kv.push(opentelemetry::KeyValue::new(SERVICE_VERSION, cfg.version.clone()));
    }
    Resource::new(kv)
}

async fn build_tracer_provider(
    endpoint: &str,
    headers: &HashMap<String, String>,
    resource: Resource,
    ratio: f64,
) -> anyhow::Result<TracerProvider> {
    let mut builder = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(format!("{}/v1/traces", endpoint));
    if !headers.is_empty() {
        builder = builder.with_headers(headers.clone());
    }
    let exp = builder.build_span_exporter()
        .map_err(|e| anyhow::anyhow!("span exporter: {}", e))?;

    let sampler = sdktrace::Sampler::TraceIdRatioBased(ratio);
    let tp = TracerProvider::builder()
        .with_batch_exporter(exp, runtime::Tokio)
        .with_config(sdktrace::Config::default().with_resource(resource).with_sampler(sampler))
        .build();
    Ok(tp)
}

async fn build_meter_provider(
    endpoint: &str,
    headers: &HashMap<String, String>,
    resource: Resource,
) -> anyhow::Result<SdkMeterProvider> {
    let mut builder = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(format!("{}/v1/metrics", endpoint));
    if !headers.is_empty() {
        builder = builder.with_headers(headers.clone());
    }
    let exp = builder.build_metrics_exporter(
        Box::new(DefaultAggregationSelector::new()),
        Box::new(DefaultTemporalitySelector::new()),
    ).map_err(|e| anyhow::anyhow!("metrics exporter: {}", e))?;

    let reader = PeriodicReader::builder(exp, runtime::Tokio).build();
    let mp = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();
    Ok(mp)
}

async fn build_logger_provider(
    endpoint: &str,
    headers: &HashMap<String, String>,
    resource: Resource,
) -> anyhow::Result<LoggerProvider> {
    let mut builder = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(format!("{}/v1/logs", endpoint));
    if !headers.is_empty() {
        builder = builder.with_headers(headers.clone());
    }
    let exp = builder.build_log_exporter()
        .map_err(|e| anyhow::anyhow!("log exporter: {}", e))?;

    let lp = LoggerProvider::builder()
        .with_batch_exporter(exp, runtime::Tokio)
        .with_config(LogConfig::default().with_resource(resource))
        .build();
    Ok(lp)
}