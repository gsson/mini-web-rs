use axum::extract::{ConnectInfo, MatchedPath};
use axum::http::{Method, Request, Response, StatusCode, Uri};
use opentelemetry::metrics::{Histogram, Meter, MeterProvider, Unit};
use opentelemetry::KeyValue;
use pin_project::pin_project;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::{Duration, Instant};
use tower_service::Service;

pub const REQUEST_HISTOGRAM_NAME: &str = "http_server_requests_seconds";

struct RequestAttributes {
    remote: Option<SocketAddr>,
    matched_path: Option<MatchedPath>,
    requested_uri: Uri,
    method: Method,
    start_time: Instant,
}

#[derive(Clone)]
pub struct ServerMetrics {
    histogram: Histogram<f64>,
}

#[inline]
fn metric_attributes(request_attributes: &RequestAttributes, status: StatusCode) -> [KeyValue; 3] {
    [
        request_attributes
            .matched_path
            .as_ref()
            .map(|matched| KeyValue::new("uri", matched.as_str().to_string()))
            .unwrap_or_else(|| KeyValue::new("uri", "unknown")),
        KeyValue::new("method", request_attributes.method.to_string()),
        KeyValue::new("status", status.as_u16() as i64),
    ]
}

impl ServerMetrics {
    pub fn new(meter: &Meter) -> Self {
        let histogram = meter
            .f64_histogram(REQUEST_HISTOGRAM_NAME)
            .with_unit(Unit::new("seconds"))
            .with_description("Server request metrics")
            .init();
        Self { histogram }
    }

    fn record_request(
        &self,
        request_attributes: &RequestAttributes,
        status: StatusCode,
        elapsed: Duration,
    ) {
        self.histogram.record(
            elapsed.as_secs_f64(),
            &metric_attributes(request_attributes, status),
        );
    }
}

#[derive(Clone)]
pub struct ObservabilityLayer {
    _meter: Meter,
    metrics: ServerMetrics,
}

impl ObservabilityLayer {
    pub fn global() -> Self {
        Self::new(opentelemetry::global::meter_provider())
    }

    pub fn new<P: MeterProvider>(meter_provider: P) -> Self {
        let meter = meter_provider.meter(env!("CARGO_PKG_NAME"));
        let metrics = ServerMetrics::new(&meter);
        Self {
            _meter: meter,
            metrics,
        }
    }
}

impl<S> tower_layer::Layer<S> for ObservabilityLayer {
    type Service = MeterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MeterService::new(inner, self.metrics.clone())
    }
}

#[derive(Clone)]
pub struct MeterService<S> {
    inner: S,
    metrics: ServerMetrics,
}

impl<S> MeterService<S> {
    pub fn new(inner: S, metrics: ServerMetrics) -> Self {
        Self { inner, metrics }
    }
}

impl<ReqBody, ResBody, S> Service<Request<ReqBody>> for MeterService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let matched_path = req.extensions().get::<MatchedPath>().cloned();
        let requested_uri = req.uri().clone();
        let remote = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0);

        ResponseFuture {
            metrics: self.metrics.clone(),
            attributes: Some(RequestAttributes {
                remote,
                matched_path,
                requested_uri,
                method: req.method().clone(),
                start_time: Instant::now(),
            }),
            future: self.inner.call(req),
        }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    metrics: ServerMetrics,
    attributes: Option<RequestAttributes>,
    #[pin]
    future: F,
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let res = ready!(this.future.poll(cx)?);

        let attrs = this
            .attributes
            .take()
            .expect("future polled after completion");

        let elapsed = attrs.start_time.elapsed();
        let status = res.status();

        tracing::info!(
            remote_host = attrs.remote.map(|r| r.ip().to_string()),
            status = status.as_str(),
            elapsed_time = u64::try_from(elapsed.as_millis()).ok(),
            method = %attrs.method,
            requested_uri = %attrs.requested_uri,
            matched_path = attrs.matched_path.as_ref().map(|e| e.as_str().to_string()),
            "Request complete: {}",
            status
        );

        this.metrics.record_request(&attrs, status, elapsed);
        Poll::Ready(Ok(res))
    }
}
