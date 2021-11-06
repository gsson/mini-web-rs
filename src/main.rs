#![feature(poll_ready)]

mod json_logging;
mod observability;

use crate::json_logging::logstash::LogstashFormat;
use crate::json_logging::DisplayLevelFilter;
use axum::body::Body;
use axum::extract::Path;
use axum::http::header::CONTENT_TYPE;
use axum::http::Response;
use axum::routing::get;
use axum::Router;
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

#[tracing::instrument(skip_all)]
async fn hello(Path(name): Path<String>) -> String {
    format!("Hello, {}!", name)
}

fn init_observability() -> Result<PrometheusExporter, anyhow::Error> {
    let logger = json_logging::Layer::default();
    let telemetry = tracing_opentelemetry::layer();

    let prometheus_exporter = opentelemetry_prometheus::exporter().try_init()?;

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
