use std::task::{Context, Poll};

use hyper::body::Incoming;
use prost::Message;
use tonic::body::BoxBody;
use tonic::Status;
use tower::Service;

use crate::proto::ScrapeError;

#[derive(Debug, Clone)]
pub struct Logger<S> {
    inner: S,
}
#[allow(dead_code)]
impl<S> Logger<S> {
    pub fn new(inner: S) -> Self {
        Logger { inner }
    }
}

impl<S> Service<hyper::Request<Incoming>> for Logger<S>
where
    S: Service<hyper::Request<Incoming>, Response = hyper::Response<BoxBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Error = S::Error;
    type Future = futures::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = S::Response;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<Incoming>) -> Self::Future {
        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let target = req.uri().clone();
        req.extensions_mut().insert(target);

        Box::pin(async move {
            let response = inner.call(req).await;

            if let Ok(response) = &response {
                let grpc_status = Status::from_header_map(response.headers());
                if let Some(grpc_status) = grpc_status {
                    error!("Response error: {:#?}", grpc_status);
                    let details = grpc_status.details();
                    if !details.is_empty() {
                        error!("Response error details: {:#?}", ScrapeError::decode(details).ok());
                    }
                }
            }

            response
        })
    }
}
