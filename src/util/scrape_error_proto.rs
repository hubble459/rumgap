use manga_parser::error::ScrapeError;
use prost::bytes::BytesMut;
use prost::Message;
use tonic::Status;

use crate::proto::{self};

impl From<ScrapeError> for proto::ScrapeError {
    fn from(value: ScrapeError) -> Self {
        Self {
            r#type: proto::ScrapeErrorType::from_str_name(value.as_ref())
                .unwrap_or_default()
                .into(),
            message: value.to_string(),
        }
    }
}

pub struct StatusWrapper(pub Status);

impl From<ScrapeError> for StatusWrapper {
    fn from(value: ScrapeError) -> Self {
        let message: String = value.to_string();
        let mut buffer = BytesMut::with_capacity(4096);
        Into::<proto::ScrapeError>::into(value)
            .encode(&mut buffer)
            .expect("encode error");

        let details = proto::DetailedError {
            status: tonic::Code::Internal as i32,
            message: message.clone(),
            details: vec![prost_types::Any {
                type_url: "rumgap.v1.ScrapeError".to_string(),
                value: buffer.to_vec(),
            }],
        };

        buffer.clear();
        details.encode(&mut buffer).expect("encode error");

        Self(Status::with_details(
            tonic::Code::Internal,
            message,
            buffer.into(),
        ))
    }
}

impl From<StatusWrapper> for Status {
    fn from(value: StatusWrapper) -> Self {
        value.0
    }
}
