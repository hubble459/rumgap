use std::task::{Context, Poll};

use hyper::Body;
use tonic::body::BoxBody;
use tower::{Layer, Service};

#[derive(Debug, Clone, Default)]
pub struct LoggerLayer;

impl<S> Layer<S> for LoggerLayer {
    type Service = Logger<S>;

    fn layer(&self, service: S) -> Self::Service {
        Logger { inner: service }
    }
}

#[derive(Debug, Clone)]
pub struct Logger<S> {
    inner: S,
}

impl<S> Service<hyper::Request<Body>> for Logger<S>
where
    S: Service<hyper::Request<Body>, Response = hyper::Response<BoxBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Error = S::Error;
    type Future = futures::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = S::Response;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<Body>) -> Self::Future {
        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let target = req.uri().clone();
        req.extensions_mut().insert(target);

        Box::pin(async move {
            inner.call(req).await
        })
    }
}
