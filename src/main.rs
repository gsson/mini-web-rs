mod observability;

use std::sync::Arc;
use axum::body::Body;
use axum::extract::Path;
use axum::http::header::CONTENT_TYPE;
use axum::http::Response;
use axum::routing::get;
use axum::Router;
use opentelemetry::sdk::export::metrics::{aggregation, AggregatorSelector};
use opentelemetry::sdk::metrics::{aggregators, controllers, processors};
use opentelemetry::sdk::metrics::aggregators::Aggregator;
use opentelemetry::sdk::metrics::sdk_api::Descriptor;
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use tracing_logstash::logstash::LogstashFormat;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

#[tracing::instrument(skip_all)]
async fn hello(Path(name): Path<String>) -> String {
    format!("Hello, {}!", name)
}

struct MiniWebAggregatorSelector;
impl AggregatorSelector for MiniWebAggregatorSelector {
    fn aggregator_for(&self, descriptor: &Descriptor) -> Option<Arc<dyn Aggregator + Send + Sync>> {
        match descriptor.name() {
            "http_server_requests_seconds" => Some(Arc::new(aggregators::histogram(
                &[0.01, 0.05, 0.1, 0.5, 1.0, 5.0]
            ))),
            _ => Some(Arc::new(aggregators::sum())),
        }
    }
}

fn init_observability() -> Result<PrometheusExporter, anyhow::Error> {
    let logger = tracing_logstash::Layer::default().event_format(
        LogstashFormat::default()
            .with_timestamp(false)
            .with_version(false),
    );
    let controller = controllers::basic(
        processors::factory(
            MiniWebAggregatorSelector,
            aggregation::cumulative_temporality_selector(),
        )
            .with_memory(true),
    )
        .build();
    let prometheus_exporter = opentelemetry_prometheus::exporter(controller)
        .try_init()?;

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
    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    let metric_families = prometheus_exporter.registry().gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, encoder.format_type())
        .body(Body::from(buffer))
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let prometheus_exporter = init_observability()?;
    let app = Router::new()
        .route(
            "/prometheus",
            get(|| async { prometheus(prometheus_exporter).await }),
        )
        .route("/api/hello/:name", get(hello))
        .layer(observability::Layer::new(
            opentelemetry::global::meter_provider(),
        ));

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    opentelemetry::global::shutdown_tracer_provider();
    Ok(())
}
