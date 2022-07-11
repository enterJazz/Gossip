use rand::random;
use thiserror::Error;
use tokio::sync::mpsc;
use crate::common;
use crate::common::Data;
use crate::communication::api;
use crate::communication::api::message::ApiMessage;
use async_trait::async_trait;


#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ApiServerError(#[from] api::server::Error),

    #[error("the passed message is not well-formed")]
    Invalid,

    #[error("received an unexpected message")]
    Unexpected,
}

#[async_trait]
pub trait Publish {
    async fn publish(&mut self, data: common::Data) -> Result<(), Error>;
}

struct Publisher {
    pub_api_tx: mpsc::Sender<api::message::ApiMessage>,
    api_pub_rx: mpsc::Receiver<Result<ApiMessage, api::server::Error>>,
}

impl Publisher {
    pub fn new(pub_api_tx: mpsc::Sender<api::message::ApiMessage>, api_pub_rx: mpsc::Receiver<Result<ApiMessage, api::server::Error>>) -> Self {
        Self {
            pub_api_tx,
            api_pub_rx,
        }
    }
}

#[async_trait]
impl Publish for Publisher {
    async fn publish(&mut self, data: common::Data) -> Result<(), Error> {
        // wrap data in Notification
        let message_id = random();
        let pub_msg = api::message::ApiMessage::Notification(
            api::payload::notification::Notification {
                message_id,
                data_type: data.data_type,
                data: data.data,
            }
        );

        self.pub_api_tx.send(pub_msg).await.expect("failed to send pub message to api server");
        let api_msg = self.api_pub_rx.recv().await.unwrap()?;

        match api_msg {
            ApiMessage::Validation(val) => {
                if message_id != val.message_id {
                    return Err(Error::Unexpected);
                }
                if !val.well_formed {
                    return Err(Error::Invalid);
                }
            }
            _ => return Err(Error::Unexpected),
        };

        Ok(())
    }
}