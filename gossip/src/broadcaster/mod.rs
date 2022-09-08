mod knowledge;

use clap::error;
use log::{debug, error, info, log_enabled, warn, Level};
use std::{net::SocketAddr, sync::Arc};
use thiserror::Error;

use tokio::{sync::{mpsc, Mutex}, net::TcpListener, io};

use crate::{
    communication::{api, p2p},
    config::Config, publisher,
};

#[derive(Error, Debug)]
pub enum BroadcasterError {
    #[error("p2p server failed: {0}")]
    P2PServerFailed(#[from] p2p::server::ServerError),
    #[error("api server failed: {0}")]
    APIServerFailed(#[from] api::server::Error),
    #[error("io error: {0}")]
    IOError(#[from] io::Error),
}

struct View {
    cache_size: usize,

    // some hashmap of messages and which peers it was sent to ; s.t. we know if `degree` peers was reached (in this case we can remove message)
    // knowledge_base: Vec<KnowledgeItem>,
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
                // knowledge_base: Vec::new(),
            })),
            config,
        }
    }

    pub async fn run(mut self) -> Result<(), BroadcasterError> {
        todo!("bootstrap");
        todo!("start publisher");
        todo!("start api server");
        todo!("start p2p server");

        // broadcaster - api channels
        let (broadcaster_api_tx, mut broadcaster_api_rx) =
            mpsc::channel::<api::message::ApiMessage>(512);
        let (api_broadcaster_tx, api_broadcaster_rx) =
            mpsc::channel::<Result<api::message::ApiMessage, api::server::Error>>(512);
        // broadcaster - p2p channels
        let (broadcaster_p2p_tx, mut broadcaster_p2p_rx) = mpsc::channel::<p2p::message::Data>(512);
        let (p2p_broadcaster_tx, p2p_broadcaster_rx) =
            mpsc::channel::<(p2p::message::Data, SocketAddr)>(512);
        // api - publisher channels
        let (pub_api_tx, pub_api_rx) = mpsc::channel(512);
        let (api_pub_tx, api_pub_rx) = mpsc::channel(512);
        
        let publisher = publisher::Publisher::new(pub_api_tx, api_pub_rx).await;
        let api_listener = TcpListener::bind(self.config.get_api_address()).await?;
        let rps_address = todo!("get the rps addr from config");
        tokio::spawn(async move {
            api::server::run(
                api_listener,
                pub_api_rx,
                api_pub_tx,
                broadcaster_api_rx,
                api_broadcaster_tx,
            rps_address).await;
        });


        let p2p_server = p2p::server::run(
            self.config.get_p2p_address(),
            self.config.get_host_pub_key(),
            p2p_broadcaster_tx,
        ).await;

        let mut knowledge_base = knowledge::KnowledgeBase::new(self.config.get_cache_size(), self.config.get_degree()).await;

        // control loop
        loop {
            tokio::select! {
                            incoming_api_msg = api_broadcaster_rx.recv() => {
                                match incoming_api_msg {
                                    Some(Ok(api::message::ApiMessage::Announce(msg))) => {
                                        let data = p2p_msg_from_api(msg);
                                        let reached_peers = p2p_server.broadcast(data).await.unwrap_or_else(|e| {
                                            error!("failed to broadcast msg to peers: {}", e);
                                            vec![]
                                        });
                                        knowledge_base.update_sent_item_to_peers(data, reached_peers)
                                            .await
                                            .unwrap_or_else(|e| error!("failed to push knowledge item: {}", e));
                                    },
                                    Some(Ok(api::message::ApiMessage::RPSPeer(peer))) => {
                                        // TODO: add host key parameter
                                        // TODO: parse PortMapRecord to get correct port for P2P peer
                                        // try connecting to new peer
                                        // TODO: @wlad do we need any extra handling here if rps peer connect fails
                                        if let Err(e) = p2p_server.connect(peer.address).await {
                                            error!("failed to connect to RPS-supplied peer: {}", e);
                                            continue
                                        };

                                        // TODO: get peer identity after connecting to new RPS-supplied peer s.t. we can dump knowledge base items into new peer
                                    },
                                    Some(Ok(_)) => error!("received unexpected message from API in broadcaster"),
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