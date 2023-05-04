use std::any::Any;
use axum::http::{Request, StatusCode};
use axum::response::{Response};
use pin_project::pin_project;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use axum::body::{Body, Bytes, HttpBody};
use axum::{BoxError};
use axum::http::header::CONTENT_TYPE;
use bytes::{BufMut, BytesMut};
use futures_util::future::{CatchUnwind, FutureExt};
use http_body::combinators::UnsyncBoxBody;
use tower_layer::Layer;
use tower_service::Service;
use crate::correlation_id::CorrelationId;
use crate::ErrorResponse;


#[derive(Copy, Clone, Debug)]
pub struct PanicHandlerLayer;

impl<S> Layer<S> for PanicHandlerLayer {
    type Service = PanicHandlerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PanicHandlerService::new(inner)
    }
}

#[derive(Clone, Debug)]
pub struct PanicHandlerService<S> {
    inner: S,
}

impl<S> PanicHandlerService<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<ReqBody, ResBody, S> Service<Request<ReqBody>> for PanicHandlerService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: HttpBody<Data=Bytes> + Send + 'static,
    ResBody::Error: Into<BoxError>,
{
    type Response = Response<UnsyncBoxBody<Bytes, BoxError>>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let correlation_id = req.extensions().get::<CorrelationId>().cloned();
        match std::panic::catch_unwind(AssertUnwindSafe(|| self.inner.call(req))) {
            Ok(future) => ResponseFuture::future(correlation_id, future),
            Err(panic_err) => ResponseFuture::panicked(correlation_id, panic_err),
        }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    correlation_id: Option<CorrelationId>,
    #[pin]
    kind: Kind<F>,
}

impl <F> ResponseFuture<F> where F: Future {
    fn future(correlation_id: Option<CorrelationId>, future: F) -> Self {
        Self {
            correlation_id,
            kind: Kind::Future {
                future: AssertUnwindSafe(future).catch_unwind(),
            },
        }
    }
    fn panicked(correlation_id: Option<CorrelationId>, panic_err: Box<dyn Any + Send + 'static>) -> Self {
        Self {
            correlation_id,
            kind: Kind::Panicked { panic_err: Some(panic_err) },
        }
    }
}


#[pin_project(project = KindProj)]
enum Kind<F> {
    Panicked {
        panic_err: Option<Box<dyn Any + Send + 'static>>,
    },
    Future {
        #[pin]
        future: CatchUnwind<AssertUnwindSafe<F>>,
    }
}


impl<F, ResBody, E> Future for ResponseFuture<F>
    where
        F: Future<Output = Result<Response<ResBody>, E>>,
        ResBody: HttpBody<Data=Bytes> + Send + 'static,
        ResBody::Error: Into<BoxError>,
{
    type Output = Result<Response<UnsyncBoxBody<Bytes, BoxError>>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.kind.project() {
            KindProj::Panicked {
                panic_err,
            } => {
                let panic_err = panic_err.take().expect("future polled after completion");
                Poll::Ready(Ok(response_for_panic(this.correlation_id.clone(), panic_err)))
            }
            KindProj::Future {
                future,
            } => match ready!(future.poll(cx)) {
                Ok(Ok(res)) => {
                    Poll::Ready(Ok(res.map(|body| body.map_err(Into::into).boxed_unsync())))
                }
                Ok(Err(svc_err)) => Poll::Ready(Err(svc_err)),
                Err(panic_err) => Poll::Ready(Ok(response_for_panic(this.correlation_id.clone(), panic_err))),
            },
        }
    }
}

fn message(err: Box<dyn Any + Send + 'static>) -> String {
    if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        StatusCode::INTERNAL_SERVER_ERROR.to_string()
    }
}

fn response_for_panic(
    correlation_id: Option<CorrelationId>,
    err: Box<dyn Any + Send + 'static>,
) -> Response<UnsyncBoxBody<Bytes, BoxError>> {
    let response = ErrorResponse {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: message(err),
        correlation_id,
    };


    let mut buf = BytesMut::with_capacity(128).writer();
    serde_json::to_writer(&mut buf, &response).expect("failed to serialize error response");
    let body = buf.into_inner().freeze();
    let body = Body::from(body).map_err(Into::into).boxed_unsync();

    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .expect("failed to build error response")
}




/*

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
*/