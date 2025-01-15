use sea_orm::DatabaseConnection;
use tonic::{Request, Status};

pub trait DatabaseRequest {
    fn db(&self) -> Result<&DatabaseConnection, Status>;
}

impl<T> DatabaseRequest for Request<T> {
    fn db(&self) -> Result<&DatabaseConnection, Status> {
        self.extensions()
            .get::<DatabaseConnection>()
            .ok_or(Status::internal("Lost connection to the database :("))
    }
}
