use std::net::SocketAddr;

use tokio::sync::mpsc;

use crate::{communication::{api, p2p}, config::Config};


struct View {
    cache_size: usize,
    // some hashmap of messages and which peers it was sent to ; s.t. we know if `degree` peers was reached (in this case we can remove message)
    // ring buf of actual messages
}

struct Broadcaster {
    broadcaster_api_tx: mpsc::Sender<api::message::ApiMessage>,
    api_broadcaster_rx: mpsc::Receiver<Result<api::message::ApiMessage, api::message::Error>>,
    p2p_broadcast_tx: mpsc::Receiver<(p2p::message::Data, SocketAddr)>,

    view: View,
    // publisher (api server)
    // p2p server
}

impl Broadcaster {
    pub async fn new(config: Config) {
        todo!("parse config into broadcaster")
    }

    pub async fn run(mut self) {
        
        todo!("bootstrap");
        todo!("start publisher");
        todo!("start api server");
        todo!("start p2p server");

        // control loop
        loop {
            tokio::select! {
                incoming_api_msg = self.api_broadcaster_rx.recv() => {
                    todo!("broadcast via p2p")
                },
                incoming_broadcast_msg = self.p2p_broadcast_tx.recv() => {
                    todo!("broadcast via p2p")
                },
//                _ = todo!("periodic push / pull ?").await => {
//                    todo!("do some things; send to rps ?");
//                }
            };
        }
    }
}

