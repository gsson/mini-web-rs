use axum::extract::MatchedPath;
use axum::http::{Method, Request, Response, Uri};
use opentelemetry::metrics::{Histogram, MeterProvider, Unit};
use opentelemetry::KeyValue;
use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use tower_service::Service;

#[derive(Clone)]
pub struct Layer {
    http_server_requests_seconds: Histogram<f64>,
}

struct RequestAttributes {
    matched_path: Option<MatchedPath>,
    requested_uri: Uri,
    method: Method,
    start_time: Instant,
}

impl Layer {
    pub fn new<P: MeterProvider>(meter_provider: P) -> Self {
        let http_server_requests_seconds = meter_provider
            .meter("http_server_requests")
            .f64_histogram("http_server_requests_seconds")
            .with_unit(Unit::new("seconds"))
            .with_description("Server request metrics")
            .init();

        Self {
            http_server_requests_seconds,
        }
    }
}

impl<S> tower_layer::Layer<S> for Layer {
    type Service = MeterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MeterService::new(inner, self.http_server_requests_seconds.clone())
    }
}

#[derive(Clone)]
pub struct MeterService<S> {
    inner: S,
    http_server_requests_seconds: Histogram<f64>,
}

impl<S> MeterService<S> {
    pub fn new(inner: S, http_server_requests_seconds: Histogram<f64>) -> Self {
        Self {
            inner,
            http_server_requests_seconds,
        }
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

        ResponseFuture {
            http_server_requests_seconds: self.http_server_requests_seconds.clone(),
            attributes: Some(RequestAttributes {
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
    http_server_requests_seconds: Histogram<f64>,
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
        let res = match this.future.poll(cx) {
            Poll::Ready(t) => t?,
            Poll::Pending => return Poll::Pending,
        };

        if let Some(RequestAttributes {
            matched_path,
            requested_uri,
            method,
            start_time,
        }) = this.attributes.take()
        {
            let elapsed = start_time.elapsed();
            let status = res.status();

            let elapsed_millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            if let Some(matched_path) = &matched_path {
                tracing::info!(
                    status = status.as_str(),
                    elapsed_time = elapsed_millis,
                    method = %method,
                    requested_uri = %requested_uri,
                    matched_path = matched_path.as_str(),
                    "Request complete: {}",
                    status
                );
            } else {
                tracing::info!(
                    status = status.as_str(),
                    elapsed_time = elapsed_millis,
                    method = %method,
                    requested_uri = %requested_uri,
                    "Request complete: {}",
                    status
                );
            }

            let matched_path = if let Some(matched_path) = matched_path {
                matched_path.as_str().to_string()
            } else {
                String::new()
            };

            let attributes = [
                KeyValue::new("uri", matched_path),
                KeyValue::new("method", method.to_string()),
                KeyValue::new("status", status.as_u16() as i64),
            ];

            let context = opentelemetry::Context::current();

            this.http_server_requests_seconds
                .record(&context, elapsed.as_secs_f64(), &attributes);
        }
        Poll::Ready(Ok(res))
    }
}
