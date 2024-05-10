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
use opentelemetry_sdk::metrics::{
    new_view, Aggregation, Instrument, SdkMeterProvider, Stream, View,
};
use prometheus::{Encoder, TextEncoder};
use serde::Serialize;
use std::backtrace::Backtrace;
use std::net::SocketAddr;
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

fn request_metrics_view() -> Box<dyn View> {
    new_view(
        Instrument::new().name(observability::REQUEST_HISTOGRAM_NAME),
        Stream::new().aggregation(Aggregation::ExplicitBucketHistogram {
            boundaries: vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0],
            record_min_max: true,
        }),
    )
    .expect("failed to create metrics view")
}

fn init_observability() -> Result<(), anyhow::Error> {
    let logger = tracing_logstash::Layer::default().event_format(
        LogstashFormat::default()
            .with_span_fields(vec!["correlation_id".into()])
            .with_timestamp(false)
            .with_version(false),
    );

    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(prometheus::default_registry().clone())
        .without_target_info()
        .without_scope_info()
        .build()
        .expect("failed to build exporter");

    let meter_provider = SdkMeterProvider::builder()
        .with_reader(exporter)
        .with_view(request_metrics_view())
        .build();

    opentelemetry::global::set_meter_provider(meter_provider);

    let telemetry = tracing_opentelemetry::layer();

    let env_filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("info"))?;
    let collector = Registry::default()
        .with(telemetry)
        .with(logger)
        .with(env_filter);

    tracing::subscriber::set_global_default(collector)?;
    Ok(())
}

async fn prometheus() -> Response<Body> {
    let metric_families = prometheus::default_registry().gather();

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
    init_observability()?;
    std::panic::set_hook(Box::new(|panic_info| {
        error!(stack_trace=%Backtrace::capture(), "{}", panic_info);
    }));
    let app = Router::new()
        .route("/prometheus", get(|| async { prometheus().await }))
        .route("/api/hello/:name", get(hello))
        .route("/api/panic/:name", get(danger))
        .layer(PanicHandlerLayer)
        .layer(ObservabilityLayer::global())
        .layer(CorrelationIdLayer);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    opentelemetry::global::shutdown_tracer_provider();
    Ok(())
}
