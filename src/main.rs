mod correlation_id;
mod observability;

use crate::correlation_id::{CorrelationId, CorrelationIdLayer};
use axum::body::{Body, BoxBody, HttpBody};
use axum::extract::Path;
use axum::headers::Header;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use opentelemetry::sdk::export::metrics::{aggregation, AggregatorSelector};
use opentelemetry::sdk::metrics::aggregators::Aggregator;
use opentelemetry::sdk::metrics::sdk_api::Descriptor;
use opentelemetry::sdk::metrics::{aggregators, controllers, processors};
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use std::any::Any;
use std::sync::Arc;
use tower_http::catch_panic::CatchPanicLayer;
use tracing_logstash::logstash::LogstashFormat;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

#[derive(Serialize)]
struct Hello {
    message: &'static str,
    name: String,
}

struct ErrorResponse {
    status: StatusCode,
    message: String,
    correlation_id: Option<CorrelationId>,
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> axum::response::Response {
        let mut builder = Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .status(self.status);

        if let Some(correlation_id) = self
            .correlation_id.as_ref()
            .and_then(|correlation_id| correlation_id.header_value().ok())
        {
            builder = builder.header(CorrelationId::name(), correlation_id);
        }

        let body = serde_json::to_vec(&self).expect("Failed to serialize error response");
        let body = Body::from(body)
            .map_err(axum::Error::new)
            .boxed_unsync();

        builder.body(body).unwrap()
    }
}

impl Serialize for ErrorResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let len = if self.correlation_id.is_some() { 3 } else { 2 };
        let mut s = serializer.serialize_struct("ErrorResponse", len)?;
        s.serialize_field("status", &self.status.as_u16())?;
        s.serialize_field("message", &self.message)?;
        if let Some(correlation_id) = &self.correlation_id {
            s.serialize_field("correlation_id", &correlation_id.0)?;
        }
        s.end()
    }
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

#[derive(Copy, Clone)]
struct MyPanicHandler;

impl MyPanicHandler {
    fn message(err: Box<dyn Any + Send + 'static>) -> String {
        if let Some(s) = err.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = err.downcast_ref::<&str>() {
            s.to_string()
        } else {
            StatusCode::INTERNAL_SERVER_ERROR.to_string()
        }
    }
}

impl tower_http::catch_panic::ResponseForPanic for MyPanicHandler {
    type ResponseBody = BoxBody;

    fn response_for_panic(
        &mut self,
        err: Box<dyn Any + Send + 'static>,
    ) -> Response<Self::ResponseBody> {
        let mut response = Json(ErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: Self::message(err),
            correlation_id: None, // I guess I need to write my own handler to get access to that :/
        })
        .into_response();
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        response
    }
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
        .route("/api/panic/:name", get(danger))
        .layer(CatchPanicLayer::custom(MyPanicHandler))
        .layer(observability::Layer::new(
            opentelemetry::global::meter_provider(),
        ))
        .layer(CorrelationIdLayer);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    opentelemetry::global::shutdown_tracer_provider();
    Ok(())
}
