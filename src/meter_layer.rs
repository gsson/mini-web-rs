use axum::http::{Method, Request, Response, Uri};
use opentelemetry::metrics::{MeterProvider, ValueRecorder};
use opentelemetry::KeyValue;
use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use tower_layer::Layer;
use tower_service::Service;

#[derive(Clone)]
pub struct MeterLayer {
    value_recorder: ValueRecorder<f64>,
}

struct RequestAttributes {
    uri: Uri,
    method: Method,
    start_time: Instant,
}

impl MeterLayer {
    pub fn new<P: MeterProvider>(meter_provider: P) -> Self {
        let value_recorder = meter_provider
            .meter("http_server_requests", None)
            .f64_value_recorder("http_server_requests_seconds")
            .with_description("Server request metrics")
            .init();

        Self { value_recorder }
    }
}

impl<S> Layer<S> for MeterLayer {
    type Service = MeterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MeterService::new(inner, self.value_recorder.clone())
    }
}

#[derive(Clone)]
pub struct MeterService<S> {
    inner: S,
    value_recorder: ValueRecorder<f64>,
}

impl<S> MeterService<S> {
    pub fn new(inner: S, value_recorder: ValueRecorder<f64>) -> Self {
        Self {
            inner,
            value_recorder,
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
        ResponseFuture {
            value_recorder: self.value_recorder.clone(),
            attributes: Some(RequestAttributes {
                uri: req.uri().clone(),
                method: req.method().clone(),
                start_time: Instant::now(),
            }),
            future: self.inner.call(req),
        }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    value_recorder: ValueRecorder<f64>,
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
        let res = this.future.poll(cx).ready()??;
        if let Some(RequestAttributes {
            uri,
            method,
            start_time,
        }) = this.attributes.take()
        {
            let elapsed = start_time.elapsed();
            let status = res.status();
            let requested_uri = uri.to_string();

            let elapsed_millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            tracing::info!(
                status = status.as_str(),
                elapsed_time = elapsed_millis,
                method = method.as_str(),
                requested_uri = requested_uri.as_str(),
                "Request complete: {}",
                status
            );

            let attributes = [
                KeyValue::new("uri", requested_uri),
                KeyValue::new("method", method.to_string()),
                KeyValue::new("status", status.as_u16() as i64),
            ];

            this.value_recorder
                .record(elapsed.as_secs_f64(), &attributes);
        }
        Poll::Ready(Ok(res))
    }
}
