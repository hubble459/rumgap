use tonic::{Request, Response, Status};

use crate::interceptor::auth::UserPermissions;
use crate::proto::verify_server::{Verify, VerifyServer};
use crate::proto::Empty;

#[derive(Debug, Default)]
pub struct MyVerify {}

#[tonic::async_trait]
impl Verify for MyVerify {
    async fn token(&self, _req: Request<Empty>) -> Result<Response<Empty>, Status> {
        Ok(Response::new(Empty::default()))
    }
}

crate::export_service!(VerifyServer, MyVerify, auth = UserPermissions::USER);
