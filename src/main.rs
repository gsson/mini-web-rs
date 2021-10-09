use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::Response;
use axum::{handler::get, Router};
use opentelemetry::metrics::MeterProvider;
use opentelemetry::KeyValue;
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use std::time::Duration;
use tower_http::trace::TraceLayer;
use tracing::Span;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Registry};

#[tracing::instrument]
async fn hello() -> &'static str {
    "Hello, World!"
}

fn init_telemetry() -> Result<PrometheusExporter, anyhow::Error> {
    let logger = tracing_subscriber::fmt::layer().compact();

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
    let prometheus_exporter = init_telemetry()?;
    let meter = opentelemetry::global::meter_provider().meter("http_server_requests", None);
    let server_request_recorder = meter
        .f64_value_recorder("http_server_requests_seconds")
        .with_description("Server request timing")
        .init();
    let app = Router::new()
        .route(
            "/prometheus",
            get(|| async { prometheus(prometheus_exporter).await }),
        )
        .route("/api/hello", get(hello))
        .layer(TraceLayer::new_for_http().on_response(
            move |response: &Response<_>, latency: Duration, _span: &Span| {
                let attributes = [
                    KeyValue::new("status", response.status().as_str().to_string()),
                    // KeyValue::new("path", ???),
                    // KeyValue::new("method", ???),
                ];
                server_request_recorder.record(latency.as_secs_f64(), &attributes);
            },
        ));

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    opentelemetry::global::shutdown_tracer_provider();
    Ok(())
}
