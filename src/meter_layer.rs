use axum::http::{Request, Response};
use opentelemetry::metrics::{MeterProvider, ValueRecorder};
use opentelemetry::KeyValue;
use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::Instant;
use tower_layer::Layer;
use tower_service::Service;

#[derive(Clone)]
pub struct MeterLayer {
    meter: ValueRecorder<f64>,
}

impl MeterLayer {
    pub fn new<P: MeterProvider>(meter_provider: P) -> Self {
        let meter = meter_provider
            .meter("request_metrics", None)
            .f64_value_recorder("http_server_requests_seconds")
            .with_description("Server request timing")
            .init();

        Self { meter }
    }
}

impl<S> Layer<S> for MeterLayer {
    type Service = MeterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MeterService::new(inner, self.meter.clone())
    }
}

#[derive(Clone)]
pub struct MeterService<S> {
    inner: S,
    meter: ValueRecorder<f64>,
}

impl<S> MeterService<S> {
    pub fn new(inner: S, meter: ValueRecorder<f64>) -> Self {
        Self { inner, meter }
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
            meter: self.meter.clone(),
            attributes: Some((
                req.uri().to_string(),
                req.method().to_string(),
                Instant::now(),
            )),
            future: self.inner.call(req),
        }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    attributes: Option<(String, String, Instant)>,
    meter: ValueRecorder<f64>,
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
        if let Some((uri, method, start_time)) = this.attributes.take() {
            let end_time = Instant::now();
            let elapsed = end_time - start_time;
            let status = res.status();

            let elapsed_millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            tracing::info!(
                status = status.as_str(),
                elapsed_time = elapsed_millis,
                method = method.as_str(),
                requested_uri = uri.as_str(),
                "Request complete: {}",
                status
            );

            let attributes = [
                KeyValue::new("uri", uri),
                KeyValue::new("method", method),
                KeyValue::new("status", status.as_str().to_string()),
            ];

            this.meter.record(elapsed.as_secs_f64(), &attributes);
        }
        Poll::Ready(Ok(res))
    }
}
