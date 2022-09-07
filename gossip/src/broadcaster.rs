use clap::error;
use log::{debug, error, info, log_enabled, warn, Level};
use std::{net::SocketAddr, sync::Arc};
use thiserror::Error;

use tokio::sync::{mpsc, Mutex};

use crate::{
    communication::{api, p2p},
    config::Config,
};

#[derive(Error, Debug)]
pub enum BroadcasterError {
    #[error("p2p server failed: {0}")]
    P2PServerFailed(#[from] p2p::server::ServerError),
}

struct KnowledgeItem {
    data: p2p::message::Data,

    sent_to: Vec<p2p::peer::PeerIdentity>,
}

struct View {
    cache_size: usize,

    // some hashmap of messages and which peers it was sent to ; s.t. we know if `degree` peers was reached (in this case we can remove message)
    knowledge_base: Vec<KnowledgeItem>,
}

struct Broadcaster {
    // broadcaster_api_tx: mpsc::Sender<api::message::ApiMessage>,
    // api_broadcaster_rx: mpsc::Receiver<Result<api::message::ApiMessage, api::message::Error>>,
    // p2p_broadcast_tx: mpsc::Sender<(p2p::message::Data, SocketAddr)>,
    // broadcast_p2p_rx: mpsc::Receiver<(p2p::message::Data, SocketAddr)>,
    config: Config,

    view: Arc<Mutex<View>>,
}

fn api_msg_from_p2p(data: p2p::message::Data) -> api::message::ApiMessage {
    api::message::ApiMessage::Announce(api::payload::announce::Announce {
        ttl: data.ttl as u8,
        data_type: data.data_type as u16,
        data: bytes::Bytes::from(data.payload),
    })
}

fn p2p_msg_from_api(msg: api::payload::announce::Announce) -> p2p::message::Data {
    p2p::message::Data {
        data_type: msg.data_type as u32,
        ttl: msg.ttl as u32,
        payload: msg.data.to_vec(),
    }
}

impl Broadcaster {
    pub async fn new(config: Config) -> Broadcaster {
        Broadcaster {
            view: Arc::new(Mutex::new(View {
                cache_size: config.get_cache_size(),
                knowledge_base: Vec::new(),
            })),
            config,
        }
    }

    pub async fn run(mut self) -> Result<(), BroadcasterError> {
        todo!("bootstrap");
        todo!("start publisher");
        todo!("start api server");
        todo!("start p2p server");

        let (broadcaster_api_tx, mut broadcaster_api_rx) =
            mpsc::channel::<api::message::ApiMessage>(512);
        let (api_broadcaster_tx, api_broadcaster_rx) =
            mpsc::channel::<Result<api::message::ApiMessage, api::message::Error>>(512);
        let (broadcaster_p2p_tx, mut broadcaster_p2p_rx) = mpsc::channel::<p2p::message::Data>(512);
        let (p2p_broadcaster_tx, p2p_broadcaster_rx) =
            mpsc::channel::<(p2p::message::Data, SocketAddr)>(512);

        let p2p_server = p2p::server::run(
            self.config.get_p2p_address(),
            self.config.get_host_pub_key(),
            p2p_broadcaster_tx,
        )
        .await;
        // control loop
        loop {
            tokio::select! {
                            incoming_api_msg = api_broadcaster_rx.recv() => {
                                match incoming_api_msg {
                                    Some(Ok(api::message::ApiMessage::Announce(msg))) => {
                                        p2p_server.broadcast(p2p_msg_from_api(msg)).await;
                                    },
                                    Some(Ok(api::message::ApiMessage::RPSPeer(peer))) => {
                                        // TODO: add host key parameter
                                        // TODO: parse PortMapRecord to get correct port for P2P peer
                                        // try connecting to new peer
                                        p2p_server.connect(peer.address);
                                    },
                                    Some(Ok(_)) => error!("received unexepcted message from API in brodcaster"),
                                    Some(Err(err)) => {
                                        warn!("message invalid, forwarding prevented: {}", err);
                                        // TODO: handle invalid messages or do nothing
                                    }
                                    None => (),
                                }
                                todo!("broadcast via p2p")
                            },
                            incoming_broadcast_msg = p2p_broadcaster_rx.recv() => {
                                match incoming_broadcast_msg {
                                    Some((data, peer_addr)) => {
                                        let msg = api_msg_from_p2p(data);
                                        // for now only forward message to api
                                        broadcaster_api_tx.send(msg);
                                    }
                                    _ => ()
                                }

                                todo!("broadcast via p2p")
                            },
            //                _ = todo!("periodic push / pull ?").await => {
            //                    todo!("do some things; send to rps ?");
            //                }
                        };
        }
    }
}
