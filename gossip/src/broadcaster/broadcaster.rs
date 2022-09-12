use core::panic;
use log::{debug, error, info, log_enabled, warn, Level};
use std::{
    collections::{HashMap, HashSet},
    convert::TryInto,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;

use tokio::{
    io,
    net::TcpListener,
    sync::{mpsc, Mutex, RwLock},
    time::interval,
};

use crate::{
    broadcaster::knowledge::{KnowledgeBase, KnowledgeItem},
    broadcaster::view::View,
    communication::{api, p2p},
    config::Config,
    publisher,
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

pub struct Broadcaster {
    // broadcaster_api_tx: mpsc::Sender<api::message::ApiMessage>,
    // api_broadcaster_rx: mpsc::Receiver<Result<api::message::ApiMessage, api::message::Error>>,
    // p2p_broadcast_tx: mpsc::Sender<(p2p::message::Data, SocketAddr)>,
    // broadcast_p2p_rx: mpsc::Receiver<(p2p::message::Data, SocketAddr)>,
    config: Config,

    view: Arc<RwLock<View>>,

    // some hashmap of messages and which peers it was sent to ; s.t. we know if `degree` peers was reached (in this case we can remove message)
    knowledge_base: Vec<KnowledgeItem>,
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
            view: Arc::new(RwLock::new(View::new(config.get_cache_size()))),
            knowledge_base: Vec::new(),
            config,
        }
    }

    async fn view_snapshot(self) -> View {
        self.view.read().await.clone()
    }

    pub async fn run(self) -> Result<(), BroadcasterError> {
        let view = self.view;

        // broadcaster - api channels
        let (broadcaster_api_tx, mut broadcaster_api_rx) =
            mpsc::channel::<api::message::ApiMessage>(512);
        let (api_broadcaster_tx, mut api_broadcaster_rx) =
            mpsc::channel::<Result<api::message::ApiMessage, api::server::Error>>(512);

        // broadcaster - p2p channels
        let (broadcaster_p2p_tx, mut broadcaster_p2p_rx) = mpsc::channel::<p2p::message::Data>(512);
        let (p2p_broadcaster_tx, mut p2p_broadcaster_rx) = mpsc::channel(512);
        let (pull_request_tx, mut pull_request_rx) = mpsc::channel(512);

        let (p2p_broadcaster_connection_tx, mut p2p_broadcaster_connection_rx) = mpsc::channel(512);

        // api - publisher channels
        let (pub_api_tx, pub_api_rx) = mpsc::channel(512);
        let (api_pub_tx, api_pub_rx) = mpsc::channel(512);

        debug!("starting publisher addr={}", self.config.get_api_address());
        let mut publisher = publisher::Publisher::new(pub_api_tx, api_pub_rx).await;
        let api_listener = match TcpListener::bind(self.config.get_api_address()).await {
            Ok(l) => l,
            Err(err) => panic!("failed to start publisher {}", err),
        };
        let rps_address = self.config.get_rps_address();
        tokio::spawn(async move {
            api::server::run(
                api_listener,
                pub_api_rx,
                api_pub_tx,
                broadcaster_api_rx,
                api_broadcaster_tx,
                rps_address,
            )
            .await;
        });

        debug!("starting P2P server");
        let p2p_server = p2p::server::run(
            self.config.get_p2p_address(),
            self.config.get_host_pub_key(),
            p2p_broadcaster_tx,
            p2p_broadcaster_connection_tx,
        )
        .await;

        if let Some(bootstrapper) = self.config.get_bootstrapper() {
            info!("bootstrapping node provided adding connecting");
            match p2p_server.connect(bootstrapper).await {
                Ok(_) => info!("connected to bootstrapping node"),
                Err(err) => error!("failed to connect to bootstrapping node {}", err),
            };
        }

        // instantiate push pull interval timers
        let mut push_interval = interval(Duration::from_secs(10));
        let mut pull_interval = interval(Duration::from_secs(15));
        let p2p_push_pull_clone = p2p_server.clone();
        let view_push_pull_clone = view.clone();
        // Gossip Push&Pull loop for spread and acquisition of information (Rule number 21 :)
        tokio::spawn(async move {
            loop {
                // TODO: stop this loop if server is down

                tokio::select! {
                    _ = push_interval.tick() => {
                        let snapshot = view_push_pull_clone.clone().read().await.clone();

                        // skip push if nothing to push
                        if snapshot.known_peers.len() <= 0 {
                            debug!("not enough peers to push");
                            continue;
                        }

                        let rumor = snapshot.into_rumor();

                        // push current view to a random peer
                        match p2p_push_pull_clone.push(rumor).await {
                            Ok(_) => debug!("push completed"),
                            Err(err) => error!("push failed {}", err),
                        };
                    }

                    _ = pull_interval.tick() => {
                        match p2p_push_pull_clone.pull().await {
                            Ok(_) => debug!("pull completed"),
                            Err(err) => error!("pull failed {}", err),
                        };
                    }

                    pull_request_msg = pull_request_rx.recv() => {
                        match &pull_request_msg {
                            Some((identity, req)) => {
                                debug!("received pull request from {}", p2p::peer::peer_into_str(*identity));
                                let snapshot = view_push_pull_clone.clone().read().await.clone();

                                let rumor = snapshot.into_rumor();
                                _ = p2p_push_pull_clone.send_to_peer(*identity, p2p::message::envelope::Msg::PullResponse(rumor)).await;
                            }
                            _ => ()
                        }
                    }
                }
            }
        });

        // TODO: send knowledge item when: new peer arrives, new knowledge item arrives -> integrate into main control loop

        // Main control loop handling interaction between P2P and API submodules
        let mut knowledge_base =
            KnowledgeBase::new(self.config.get_cache_size(), self.config.get_degree());

        // control loop
        loop {
            tokio::select! {
                incoming_api_msg = api_broadcaster_rx.recv() => {
                    match incoming_api_msg {
                       // in case of an incoming gossip announce:
                       // attempt to broadcast to available peers
                       // save item and peers broadcasted to in knowledge base
                       Some(Ok(api::message::ApiMessage::Announce(msg))) => {
                            let data = p2p_msg_from_api(msg);
                            let reached_peers = p2p_server.broadcast(data).await.unwrap_or_else(|e| {
                                error!("failed to broadcast msg to peers: {}", e);
                                vec![]
                            });
                            // knowledge_base.update_sent_item_to_peers(data, reached_peers)
                            //     .unwrap_or_else(|e| error!("failed to push knowledge item: {}", e));
                        },
                        // in case of incoming RPSPeer message:
                        // TODO: @wlad
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
                }

                // External ---> P2P ---> Broadcaster
                // handle messages from other peers
                // NOTE:
                // - not all messages are exepcted here (only relevant for upper layers)
                // - peers sending messages here must have completed the PoW verification process
                incoming_broadcast_msg = p2p_broadcaster_rx.recv() => {
                    match incoming_broadcast_msg {
                        Some((msg, identity, addr)) => {
                            match msg {
                                // Data messages from p2p module are forwarded here to the Api level for further processing
                                // Data items will be added to the knowledge base after completing the verification step
                                // within the P2P module
                                p2p::message::envelope::Msg::Data(mut data) => {
                                    // check if ttl is 0; if yes, drop
                                    // @wlad: is this correct ttl handling?
                                    if data.ttl == 0 {
                                        info!("dropping incoming broadcast message as ttl is 0");
                                        continue
                                    } else {
                                        // decrement ttl
                                        data.ttl -= 1;
                                    }
                                    // for now only forward message to api
                                    if let Err(e) = publisher.publish(
                                        crate::common::Data { data_type: (data.data_type as u16), data: bytes::Bytes::from(data.payload) })
                                        .await {
                                        match e {
                                            publisher::Error::ApiServerError(e) => {
                                                error!("api server error: {e} - skipping broadcast");
                                                continue
                                            },
                                            publisher::Error::Invalid => {
                                                error!("received message is not well-formed - skipping broadcast");
                                                continue
                                            },
                                            publisher::Error::Unexpected => {
                                                error!("received unexpected validation reply");
                                            },
                                        }
                                    };
                                    // TODO: @wlad how do we make sure we don't broadcast the message back to the sender? (avoid echo)
                                    // TODO: duplicate code
                                    // let reached_peers = p2p_server.broadcast(data).await.unwrap_or_else(|e| {
                                    //     error!("failed to broadcast msg to peers: {}", e);
                                    //     vec![]
                                    // });
                                    // knowledge_base.update_sent_item_to_peers(data, reached_peers)
                                    //     .unwrap_or_else(|e| error!("failed to push knowledge item: {}", e));
                                    // TODO: add messages for verification tracking
                                },
                                // Rumors are used to update the peers view of the "world"
                                // these messages might be received via a Push or Pull (asynchronously after pull request)
                                p2p::message::envelope::Msg::PullResponse(rumor) |
                                p2p::message::envelope::Msg::Rumor(rumor) => {
                                    view.clone().write().await.merge(rumor);
                                    view.clone().read().await.print();

                                    // TODO: move somewhere else
                                    // connect to unconnected peers
                                    for (id, peer) in &view.clone().read().await.known_peers {
                                        // skip if is self
                                        if *id == p2p_server.get_identity() {
                                            continue;
                                        }

                                        if peer.is_active() {
                                            continue;
                                        }

                                        if let Some(peer_addr) = peer.get_addr() {
                                            info!("connecting to newly found peer {:?}", peer_addr);
                                            match p2p_server.connect(peer_addr).await {
                                                Ok(_) => info!("connected to peer"),
                                                Err(err) => error!("could not connect to peer {}", err)
                                            }
                                        }


                                    }
                                }
                                p2p::message::envelope::Msg::Pull(req) => {
                                    _ = pull_request_tx.send((identity, req)).await;
                                }
                                _ => unreachable!("no further messages should be delivered to the broadcastery")
                            }
                        }
                        // ignore empty results
                        _ => ()
                    }
                    // todo!("broadcast via p2p")
                }


                p2p_connection_update_msg = p2p_broadcaster_connection_rx.recv() => {
                    match p2p_connection_update_msg {
                        Some((identity, status, peer)) => {
                            info!("received peer update");
                            view.clone().write().await.process_peer_update(identity, status, peer);
                            view.clone().read().await.print();
                        }
                        // ignore empty results
                        _ => ()
                    }
                }

            };
        }
    }
}
