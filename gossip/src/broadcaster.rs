use std::net::SocketAddr;

use tokio::sync::mpsc;

use crate::communication::{api, p2p};


struct Broadcaster {
    broadcaster_api_tx: mpsc::Sender<api::message::ApiMessage>,
    api_broadcaster_rx: mpsc::Receiver<Result<api::message::ApiMessage, api::message::Error>>,
    p2p_broadcast_tx: mpsc::Receiver<(p2p::message::Data, SocketAddr)>
}

impl Broadcaster {
    pub async fn run(mut self) {
        
        todo!("start publisher");
        todo!("start api server");
        todo!("start p2p server");

        // control loop
        loop {
            tokio::select! {
                incoming_announce_msg = self.api_broadcaster_rx.recv() => {
                    todo!("broadcast via p2p")
                },
                incoming_broadcast_msg = self.p2p_broadcast_tx.recv() => {
                    todo!("broadcast via p2p")
                },
            };
        }
    }
}

