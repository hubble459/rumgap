use proto::user_server::{User, UserServer};
use proto::{Id, PaginateQuery, UserReply, UserRequest, UsersReply};
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tonic_reflection::server::Builder;

pub mod proto {
    tonic::include_proto!("rumgap");

    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("descriptor");
}

#[derive(Debug, Default)]
pub struct MyUser {}

#[tonic::async_trait]
impl User for MyUser {
    async fn create(&self, request: Request<UserRequest>) -> Result<Response<UserReply>, Status> {
        let req = request.into_inner();
        Ok(Response::new(UserReply {
            id: 1,
            username: format!("Hello {}!", req.username),
            password: format!("Hello {}!", req.password),
        }))
    }

    async fn get(&self, request: Request<Id>) -> Result<Response<UserReply>, Status> {
        let req = request.into_inner();
        Ok(Response::new(UserReply {
            id: req.id,
            username: format!("Hello {}!", req.id),
            password: format!("Hello {}!", "req.password"),
        }))
    }

    async fn index(&self, request: Request<PaginateQuery>) -> Result<Response<UsersReply>, Status> {
        let req = request.into_inner();
        Ok(Response::new(UsersReply {
            items: vec![
                UserReply {
                    id: req.page,
                    username: format!("Hello {}!", "nghh"),
                    password: format!("Hello {}!", "req.password"),
                },
                UserReply {
                    id: req.limit,
                    username: format!("Hello {}!", "nghh"),
                    password: format!("Hello {}!", "req.password"),
                },
            ],
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:8080".parse()?;
    let user = MyUser::default();

    Server::builder()
        .add_service(UserServer::new(user))
        .add_service(
            Builder::configure()
                .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
                .build()?,
        )
        .serve(addr)
        .await?;

    Ok(())
}
