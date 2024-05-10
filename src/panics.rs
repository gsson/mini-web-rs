use crate::correlation_id::CorrelationId;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use futures_util::future::{CatchUnwind, FutureExt};
use http_api_problem::HttpApiProblem;
use pin_project::pin_project;
use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tower_layer::Layer;
use tower_service::Service;

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

impl<ReqBody, S> Service<Request<ReqBody>> for PanicHandlerService<S>
where
    S: Service<Request<ReqBody>, Response = Response>,
{
    type Response = Response;
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

impl<F> ResponseFuture<F>
where
    F: Future,
{
    fn future(correlation_id: Option<CorrelationId>, future: F) -> Self {
        Self {
            correlation_id,
            kind: Kind::Future {
                future: AssertUnwindSafe(future).catch_unwind(),
            },
        }
    }
    fn panicked(
        correlation_id: Option<CorrelationId>,
        panic_err: Box<dyn Any + Send + 'static>,
    ) -> Self {
        Self {
            correlation_id,
            kind: Kind::Panicked {
                panic_err: Some(panic_err),
            },
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
    },
}

impl<F, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response, E>>,
{
    type Output = Result<Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.kind.project() {
            KindProj::Panicked { panic_err } => {
                let panic_err = panic_err.take().expect("future polled after completion");
                Poll::Ready(Ok(response_for_panic(
                    this.correlation_id.as_ref(),
                    panic_err,
                )))
            }
            KindProj::Future { future } => match ready!(future.poll(cx)) {
                Ok(Ok(res)) => Poll::Ready(Ok(res)),
                Ok(Err(svc_err)) => Poll::Ready(Err(svc_err)),
                Err(panic_err) => Poll::Ready(Ok(response_for_panic(
                    this.correlation_id.as_ref(),
                    panic_err,
                ))),
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
    correlation_id: Option<&CorrelationId>,
    err: Box<dyn Any + Send + 'static>,
) -> Response {
    let mut problem =
        HttpApiProblem::with_title_and_type(StatusCode::INTERNAL_SERVER_ERROR).detail(message(err));

    if let Some(correlation_id) = correlation_id {
        problem = problem.value("correlation_id", &correlation_id.0)
    }

    problem.to_axum_response()
}
