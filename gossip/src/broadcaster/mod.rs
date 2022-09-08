mod knowledge;
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
    sync::{mpsc, Mutex, RwLock},
    time::interval,
    net::TcpListener, io
};

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

struct KnowledgeItem {
    data: p2p::message::Data,

    sent_to: Vec<p2p::peer::PeerIdentity>,
}

struct PeerViewItem {
    is_active: bool,
    info: p2p::message::Peer,
}

impl PeerViewItem {
    fn from(peer: p2p::message::Peer) -> Self {
        PeerViewItem {
            is_active: false,
            info: peer,
        }
    }
}

fn parse_identity(buf: &[u8]) -> Option<p2p::peer::PeerIdentity> {
    match buf.try_into() {
        Ok(id) => Some(id),
        Err(err) => None,
    }
}

struct View {
    cache_size: usize,

    known_peers: HashMap<p2p::peer::PeerIdentity, PeerViewItem>,
}

impl View {
    fn new(cache_size: usize) -> Self {
        View {
            cache_size,
            known_peers: HashMap::new(),
        }
    }

    /// Merge combines the current view with incoming information about peers
    fn merge(&mut self, rumor: p2p::message::Rumor) {
        // find peers we know nothing about yet
        let new_unknown_peers: Vec<(p2p::peer::PeerIdentity, p2p::message::Peer)> = rumor
            .peers
            .iter()
            .filter_map(|peer| {
                if let Some(identity) = parse_identity(&peer.identity) {
                    // only returbn item if we don't already know about it
                    return if self.known_peers.contains_key(&identity) {
                        None
                    } else {
                        Some((identity, peer.clone()))
                    };
                }
                // TODO: maybe handle updates for already known peers
                None
            })
            .collect();

        for (identity, peer) in new_unknown_peers {
            if self.known_peers.len() < self.cache_size {
            } else {
                debug!("view cache already full dropping pull result");
                break;
            }
            self.known_peers.insert(identity, PeerViewItem::from(peer));
        }
    }

    fn into_rumor(self) -> p2p::message::Rumor {
        p2p::message::Rumor {
            ttl: 1,
            peers: self
                .known_peers
                .into_values()
                .map(|peer| peer.info)
                .collect(),
        }
    }
}

impl Clone for View {
    fn clone(&self) -> Self {
        todo!()
    }
}

struct Broadcaster {
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

    // Internal update method used to merge current view and incoming rumor
    async fn update_view(self, rumor: p2p::message::Rumor) {
        let mut view = self.view.write().await;
        view.merge(rumor)
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
            mpsc::channel::<(p2p::message::envelope::Msg, SocketAddr)>(512);
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
        )
        .await;

        // instantiate push pull interval timers
        let push_interval = interval(Duration::from_secs(4));
        let pull_interval = interval(Duration::from_secs(7));
        // Gossip Push&Pull loop for spread and acquisition of information (Rule number 21 :)
        tokio::spawn(async move {
            loop {
                // TODO: stop this loop if server is down

                tokio::select! {
                    push = push_interval.tick() => {
                        let snapshot = self.view_snapshot().await;
                        let rumor = snapshot.into_rumor();
                        p2p_server.push(rumor);
                    }

                    pull = push_interval.tick() => {
                        p2p_server.pull();
                    }

                }
            }
        });

        // Main control loop handling interaction between P2P and API submodules
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
                }

                // External ---> P2P ---> Broadcaster
                // handle messages from other peers
                // NOTE:
                // - not all messages are exepcted here (only relevant for upper layers)
                // - peers sending messages here must have completed the PoW verification process
                incoming_broadcast_msg = p2p_broadcaster_rx.recv() => {
                    match incoming_broadcast_msg {
                        Some((msg, peer_addr)) => {
                            match msg {
                                // Data messages from p2p module are forwarded here to the Api level for further processing
                                // Data items will be added to the knowledge base after completing the verification step
                                // within the P2P module
                                p2p::message::envelope::Msg::Data(data) => {
                                    let msg = api_msg_from_p2p(data);
                                    // for now only forward message to api
                                    broadcaster_api_tx.send(msg);
                                    // TODO: add messages for verification tracking
                                }

                                // Rumors are used to update the peers view of the "world"
                                // these messages might be received via a Push or Pull (asynchronously after pull request)
                                p2p::message::envelope::Msg::Rumor(rumor) => {
                                    // update view asynchronously to prevent halting the receiver future
                                    self.update_view(rumor);
                                }
                                _ => unreachable!("no further messages should be delivered to the broadcastery")
                            }
                        }
                        // ignore empty results
                        _ => ()
                    }

                    todo!("broadcast via p2p")
                },
            };
        }
    }
}