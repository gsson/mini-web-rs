use axum::http::Request;
use axum::response::Response;
use axum_extra::headers::{Error, Header, HeaderName, HeaderValue};
use pin_project::pin_project;
use rusty_ulid::generate_ulid_string;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tower_layer::Layer;
use tower_service::Service;
use tracing::info_span;

#[derive(Clone, Debug)]
pub struct CorrelationId(pub String);

impl Header for CorrelationId {
    fn name() -> &'static HeaderName {
        static NAME: HeaderName = HeaderName::from_static("x-correlation-id");
        &NAME
    }

    fn decode<'i, I>(values: &mut I) -> Result<Self, Error>
    where
        Self: Sized,
        I: Iterator<Item = &'i HeaderValue>,
    {
        let value = values.next().ok_or_else(Error::invalid)?;
        Self::from_header_value(value)
    }

    fn encode<E: Extend<HeaderValue>>(&self, values: &mut E) {
        values.extend([HeaderValue::from_str(&self.0).unwrap()]);
    }
}

impl CorrelationId {
    pub fn generate() -> Self {
        Self(generate_ulid_string())
    }

    pub fn from_header_value(value: &HeaderValue) -> Result<Self, Error> {
        let value = value.to_str().map_err(|_| Error::invalid())?;
        Ok(Self(value.to_string()))
    }

    pub fn header_value(&self) -> Result<HeaderValue, Error> {
        HeaderValue::from_str(&self.0).map_err(|_| Error::invalid())
    }
}

impl Display for CorrelationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct CorrelationIdLayer;

impl<S> Layer<S> for CorrelationIdLayer {
    type Service = CorrelationIdService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CorrelationIdService::new(inner)
    }
}

#[derive(Clone, Debug)]
pub struct CorrelationIdService<S> {
    inner: S,
}

impl<S> CorrelationIdService<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<ReqBody, ResBody, S> Service<Request<ReqBody>> for CorrelationIdService<S>
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

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let correlation_id = req
            .headers_mut()
            .remove(CorrelationId::name())
            .map(|value| CorrelationId::from_header_value(&value).expect("invalid correlation id"))
            .unwrap_or_else(CorrelationId::generate);

        req.extensions_mut().insert(correlation_id.clone());

        ResponseFuture {
            correlation_id,
            future: self.inner.call(req),
        }
    }
}

#[pin_project]
#[derive(Debug)]
pub struct ResponseFuture<F> {
    correlation_id: CorrelationId,
    #[pin]
    future: F,
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let span = info_span!("correlation_id", correlation_id = %self.correlation_id);
        let _guard = span.enter();
        let this = self.project();
        let mut res = ready!(this.future.poll(cx)?);

        res.headers_mut().insert(
            CorrelationId::name(),
            this.correlation_id.header_value().unwrap(),
        );

        Poll::Ready(Ok(res))
    }
}
