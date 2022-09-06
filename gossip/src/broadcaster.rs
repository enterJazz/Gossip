use std::net::SocketAddr;
use thiserror::Error;

use tokio::sync::mpsc;

use crate::{
    communication::{api, p2p},
    config::Config,
};

#[derive(Error, Debug)]
pub enum BroadcasterError {
    #[error("p2p server failed: {0}")]
    P2PServerFailed(#[from] p2p::server::ServerError),
}

struct Broadcaster {
    broadcaster_api_tx: mpsc::Sender<api::message::ApiMessage>,
    api_broadcaster_rx: mpsc::Receiver<Result<api::message::ApiMessage, api::message::Error>>,
    p2p_broadcast_tx: mpsc::Sender<(p2p::message::Data, SocketAddr)>,
    p2p_broadcast_rx: mpsc::Receiver<(p2p::message::Data, SocketAddr)>,

    config: Config,
}

impl Broadcaster {
    pub async fn run(mut self) -> Result<(), BroadcasterError> {
        todo!("start publisher");
        todo!("start api server");
        todo!("start p2p server");

        let p2p_server =
            p2p::server::run(self.config.get_p2p_address(), self.p2p_broadcast_tx).await;
        // control loop
        loop {
            tokio::select! {
                incoming_announce_msg = self.api_broadcaster_rx.recv() => {
                    todo!("broadcast via p2p")
                },
                incoming_broadcast_msg = self.p2p_broadcast_rx.recv() => {
                    todo!("broadcast via p2p")
                },
            };
        }
    }
}
