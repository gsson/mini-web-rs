mod correlation_id;
mod observability;
mod panics;

use crate::correlation_id::{CorrelationId, CorrelationIdLayer};
use crate::panics::PanicHandlerLayer;
use axum::body::Body;
use axum::extract::Path;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::Response;
use axum::routing::get;
use axum::{Extension, Json, Router};
use observability::ObservabilityLayer;
use opentelemetry::sdk::export::metrics::{aggregation, AggregatorSelector};
use opentelemetry::sdk::metrics::aggregators::Aggregator;
use opentelemetry::sdk::metrics::sdk_api::Descriptor;
use opentelemetry::sdk::metrics::{aggregators, controllers, processors};
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use serde::Serialize;
use std::backtrace::Backtrace;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::error;
use tracing_logstash::logstash::LogstashFormat;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

#[derive(Serialize)]
struct Hello {
    message: &'static str,
    name: String,
}

#[tracing::instrument(skip_all)]
async fn hello(Path(name): Path<String>, correlation_id: Extension<CorrelationId>) -> Json<Hello> {
    eprintln!("{:?}", correlation_id);
    Json(Hello {
        message: "Nice to meet you",
        name,
    })
}

#[tracing::instrument(skip_all)]
async fn danger(Path(name): Path<String>) -> String {
    panic!("Hello, {}!", name)
}

struct MiniWebAggregatorSelector;

impl AggregatorSelector for MiniWebAggregatorSelector {
    fn aggregator_for(&self, descriptor: &Descriptor) -> Option<Arc<dyn Aggregator + Send + Sync>> {
        match descriptor.name() {
            "http_server_requests_seconds" => Some(Arc::new(aggregators::histogram(&[
                0.01, 0.05, 0.1, 0.5, 1.0, 5.0,
            ]))),
            _ => Some(Arc::new(aggregators::sum())),
        }
    }
}

fn init_observability() -> Result<PrometheusExporter, anyhow::Error> {
    let logger = tracing_logstash::Layer::default().event_format(
        LogstashFormat::default()
            .with_span_fields(vec!["correlation_id".into()])
            .with_timestamp(false)
            .with_version(false),
    );
    let controller = controllers::basic(
        processors::factory(
            MiniWebAggregatorSelector,
            aggregation::cumulative_temporality_selector(),
        )
        .with_memory(false),
    )
    .build();
    let prometheus_exporter = opentelemetry_prometheus::exporter(controller).try_init()?;

    let telemetry = tracing_opentelemetry::layer();

    let env_filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("info"))?;
    let collector = Registry::default()
        .with(telemetry)
        .with(logger)
        .with(env_filter);

    tracing::subscriber::set_global_default(collector)?;
    Ok(prometheus_exporter)
}

async fn prometheus(prometheus_exporter: PrometheusExporter) -> Response<Body> {
    let metric_families = prometheus_exporter.registry().gather();

    let encoder = TextEncoder::new();
    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer).unwrap();

    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, encoder.format_type())
        .header(CACHE_CONTROL, "no-cache")
        .body(Body::from(buffer))
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let prometheus_exporter = init_observability()?;
    std::panic::set_hook(Box::new(|panic_info| {
        error!(stack_trace=%Backtrace::capture(), "{}", panic_info);
    }));
    let app = Router::new()
        .route(
            "/prometheus",
            get(|| async { prometheus(prometheus_exporter).await }),
        )
        .route("/api/hello/:name", get(hello))
        .route("/api/panic/:name", get(danger))
        .layer(PanicHandlerLayer)
        .layer(ObservabilityLayer::global())
        .layer(CorrelationIdLayer);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;

    opentelemetry::global::shutdown_tracer_provider();
    Ok(())
}
