use tonic::{Request, Status};

pub trait Authorize {
    fn authorize(&self) -> Result<&entity::user::Model, Status>;
}

impl<T> Authorize for Request<T> {
    fn authorize(&self) -> Result<&entity::user::Model, Status> {
        self.extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::unauthenticated(
                "Missing bearer token! Log in first",
            ))
    }
}
