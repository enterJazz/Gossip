use crate::communication::p2p::peer::{parse_identity, peer_into_str};
use crate::communication::p2p::peer::{Peer, PeerError, PeerResult};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::net::AddrParseError;
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};

use super::message;
use super::peer::{into_addr, PeerConnectionStatus, PeerIdentity};

// message type used by messages originating from server
pub type ServerPeerMessage = (message::envelope::Msg, PeerIdentity, SocketAddr);
pub type PeerConnectionMessage = (PeerIdentity, PeerConnectionStatus, message::Peer);

type PeerReceiver = mpsc::Receiver<message::envelope::Msg>;
type PeerSender = mpsc::Sender<message::envelope::Msg>;
type PeerBroadcastReceiver = broadcast::Receiver<message::envelope::Msg>;
type PeerBroadcastSender = broadcast::Sender<message::envelope::Msg>;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("peer connection error ({0})")]
    PeerConnectionError(#[from] PeerError),
    #[error("peer limit reached")]
    PeerLimitReached,
    #[error("duplicate peer connection detected")]
    DuplicatePeer,
    #[error("server peer close error")]
    PeerCloseError,
    #[error("could not transfer message to peer handler")]
    PeerSendError,
    #[error("peer not found {identity}")]
    PeerNotFound { identity: String },
    #[error("failed to sample random peer")]
    PeerSamplingFailed,
    #[error("push failed")]
    PeerPushError,
    #[error("pull failed")]
    PeerPullError,
    #[error("server: rx channel err {0}")]
    RxChannelError(#[from] SendError<ServerPeerMessage>),
    #[error("server: rx data channel err")]
    RxDataChannelError,
}

struct ServerState {
    max_parallel_connections: usize,
    active_peers: HashMap<PeerIdentity, PeerHandler>,
}

pub struct Server {
    listener: TcpListener,
    state: Arc<RwLock<ServerState>>,
    addr: SocketAddr,

    host_identity: PeerIdentity,
    host_pub_key: bytes::Bytes,

    peer_p2p_tx: mpsc::Sender<ServerPeerMessage>,
    peer_p2p_rx: Arc<Mutex<mpsc::Receiver<ServerPeerMessage>>>,

    peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
    peer_connection_rx: Arc<Mutex<mpsc::Receiver<PeerConnectionMessage>>>,
}

impl ServerState {
    /// Create a new, empty, instance of `Shared`.
    fn new() -> Self {
        ServerState {
            active_peers: HashMap::new(),
            max_parallel_connections: 10,
        }
    }
}

struct PeerHandler {
    msg_tx: PeerSender,
    peer: Arc<Peer>,
}

impl PeerHandler {
    fn new(
        peer: Peer,
        peer_p2p_tx: mpsc::Sender<ServerPeerMessage>,
        peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
    ) -> PeerHandler {
        let (rx_write, mut rx_read) = mpsc::channel(512);
        let (tx_write, tx_read) = mpsc::channel(512);

        let (close_sender, mut close_receiver) = mpsc::channel::<Option<PeerError>>(1);

        let p = Arc::new(peer);
        let peer_run = p.clone();

        // run peer recv and send methods
        // on transmission error write into close channel
        tokio::spawn(async move {
            match peer_run.run(tx_read, rx_write).await {
                Ok(_) => close_sender.send(None).await,
                Err(err) => close_sender.send(Some(err)).await,
            }
        });

        let tx_clone = tx_write.clone();
        let peer_addr = p.remote_addr();
        let peer_identity = p.identity.clone();
        let peer_description = p.get_peer_description();
        tokio::spawn(async move {
            loop {
                tokio::select! {

                    // message comming in from peer
                    // receive message from peer and expand it by peer information
                    // so that in can easily be processes in the server level
                    peer_rx_msg = rx_read.recv() => {
                        match peer_rx_msg {
                            Some(msg) => match peer_p2p_tx.send((msg, peer_identity, peer_addr)).await {
                                Ok(_) => (),
                                Err(e) => {
                                    error!("failed to receive message {}", e);
                                    todo!();
                                }
                            }
                            None => warn!("got empty message from peer channel")
                        }
                    }

                    close_err = close_receiver.recv() => {
                        match close_err {
                            Some(Some(err)) => {
                                error!("peer_handler: closed with error {}", err);
                                match peer_connection_tx.send((
                                    peer_identity,
                                    PeerConnectionStatus::Closed("closed with error".to_string()), peer_description.clone()
                                )).await {
                                    Ok(_) => (),
                                    Err(err) => {
                                        panic!("failed to send peer connection update {}", err)
                                    }
                                }
                            }
                            _ => {debug!("peer_handler: closed without errors"); todo!();}
                        }
                    }
                }
            }
        });

        PeerHandler {
            msg_tx: tx_write,
            peer: p,
        }
    }

    async fn send_msg(
        &self,
        msg: message::envelope::Msg,
    ) -> Result<(), SendError<message::envelope::Msg>> {
        self.msg_tx.send(msg).await
    }
}

pub async fn run_from_str_addr(
    addr_str: &str,
    host_pub_key: bytes::Bytes,
    rx: mpsc::Sender<ServerPeerMessage>,
    peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
) -> Result<Arc<Server>, AddrParseError> {
    let addr: SocketAddr = match addr_str.parse() {
        Ok(a) => a,
        Err(err) => return Err(err),
    };

    Ok(run(addr, host_pub_key, rx, peer_connection_tx).await)
}

pub async fn run(
    addr: SocketAddr,
    host_pub_key: bytes::Bytes,
    rx: mpsc::Sender<ServerPeerMessage>,
    peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
) -> Arc<Server> {
    let server = Arc::new(Server::new(addr, host_pub_key).await);

    let s = server.clone();
    tokio::spawn(async move {
        match s.run(rx, peer_connection_tx).await {
            Ok(()) => info!("p2p/server: executed finished"),
            Err(err) => error!("p2p/server: execution aborted due to error {}", err),
        }
    });

    server
}

impl Server {
    pub async fn new(addr: SocketAddr, host_pub_key: bytes::Bytes) -> Self {
        // construct underlaying TCP connection
        // forcefully unwrap here, it is expected that server dies if tcp listener cannot be started
        let listener = TcpListener::bind(addr).await.unwrap();

        // create channels for peer orchestration and handling
        let (peer_p2p_tx, peer_p2p_rx) = mpsc::channel(512);

        // channel for handling peer removal
        let (peer_connection_tx, peer_connection_rx) = mpsc::channel(512);

        let identity = *blake3::hash(&host_pub_key.clone()).as_bytes();

        info!(
            "starting P2P server {} id={}",
            addr,
            peer_into_str(identity)
        );

        Server {
            state: Arc::new(RwLock::new(ServerState::new())),
            listener,
            addr,

            host_pub_key: host_pub_key,
            host_identity: identity,

            // channel for receiving messages from peers
            peer_p2p_tx,
            peer_p2p_rx: Arc::new(Mutex::new(peer_p2p_rx)),

            peer_connection_tx,
            peer_connection_rx: Arc::new(Mutex::new(peer_connection_rx)),
        }
    }

    pub async fn run(
        &self,
        p2p_external_tx: mpsc::Sender<ServerPeerMessage>,
        connection_p2p_external_tx: mpsc::Sender<PeerConnectionMessage>,
    ) -> Result<(), ServerError> {
        // lock rx_receiver channel
        // this also ensures only one run can be executed at a time
        let mut peer_p2p_rx = self.peer_p2p_rx.lock().await;
        let mut peer_connection_rx = self.peer_connection_rx.lock().await;

        // handle peer message passing
        loop {
            tokio::select! {
                // accept new incomming messages
                res = self.listener.accept() => {
                    let (stream, addr) = match res {
                        Ok(res) => res,
                        Err(e) => {
                            error!("run: failed to connect with listener: {:?}", e);
                            continue;
                        }
                    };

                    info!("run: incoming connection from {}", addr);
                    self.accept(stream).await;
                }

                // handle messages from peers after a peer is active
                in_msg = peer_p2p_rx.recv() => {
                    let (msg, identity, peer_addr) = match in_msg {
                        Some(data) => data,
                        None => {
                            error!("run: received empty message");
                            continue;
                        }
                    };


                    match msg {
                        message::envelope::Msg::Pull(_) |
                        message::envelope::Msg::Data(_) |
                        message::envelope::Msg::PullResponse(_) |
                        message::envelope::Msg::Rumor(_)  => {
                            match p2p_external_tx.send((msg, identity, peer_addr)).await {
                                Ok(_) => (),
                                Err(err) => return Err(ServerError::RxDataChannelError),
                            };
                        }
                        _ => {
                            unreachable!("run: got internal message {:?}", msg)

                        }
                    }
                }
                // handle peer removals in case of errors
                // does not include challenge errors (invalid peers will not be added to active pool)
                peer_connection_update = peer_connection_rx.recv() => {
                    match peer_connection_update {
                        Some((identity, status, peer)) => {
                            // handle peer removal
                            match &status {
                                PeerConnectionStatus::Closed(reason) => {
                                    self.remove_connection(identity).await;
                                    info!("peer removed {} due to {}", peer_into_str(identity), reason)
                                }
                                _ => ()
                            }
                            info!("peer status changed {}", peer_into_str(identity));
                            _ = connection_p2p_external_tx.send((identity, status, peer)).await;
                        },
                        _ => debug!("peer removing"),
                    }
                }
            }
        }
    }

    async fn accept(&self, stream: TcpStream) -> Result<(), ServerError> {
        let state = self.state.read().await;
        // Close incomming connection if server has reached its maximum connection counter
        if state.active_peers.keys().len() == state.max_parallel_connections {
            debug!("p2p/server/accept_connection: max number connections reached closing");
            return Err(ServerError::PeerLimitReached);
        }
        drop(state);

        let host_identity = self.get_identity();
        let host_pub_key = self.host_pub_key.clone();

        let mut peer = Peer::new(stream);

        match peer.challenge(host_identity, host_pub_key).await {
            Ok(_) => {
                let addr = peer.remote_addr();
                let identity = peer.identity.clone();
                info!(
                    "accept: completed challenge addr={}, id={}",
                    addr,
                    peer_into_str(peer.identity)
                );

                self.add_connection(identity, peer).await
            }
            Err(err) => Err(ServerError::PeerConnectionError(err)),
        }
    }

    // connect to remote peer‚
    pub async fn connect<T: ToSocketAddrs>(&self, addr: T) -> Result<(), ServerError> {
        let mut peer = match Peer::new_from_addr(addr).await {
            Err(err) => return Err(ServerError::PeerConnectionError(err)),
            Ok(peer) => peer,
        };

        let state = self.state.read().await;
        // Close incomming connection if server has reached its maximum connection counter
        if state.active_peers.keys().len() == state.max_parallel_connections {
            debug!("p2p/server/accept_connection: max number connections reached closing");
            return Err(ServerError::DuplicatePeer);
        }
        drop(state);

        let identity = match peer
            .connect(
                self.host_identity.clone(),
                self.addr.port(),
                self.host_pub_key.clone(),
            )
            .await
        {
            Err(err) => return Err(ServerError::PeerConnectionError(err)),
            Ok(id) => id,
        };

        self.add_connection(identity, peer).await
    }

    pub async fn disconnect(&self, identity: PeerIdentity) -> Option<message::Peer> {
        self.remove_connection(identity)
            .await
            .map_or(None, |p| Some(p.peer.get_peer_description()))
    }

    async fn add_connection(&self, identity: PeerIdentity, peer: Peer) -> Result<(), ServerError> {
        let description = peer.get_peer_description();
        let addr = peer.get_peer_addr();
        let mut state = self.state.write().await;
        let handler = PeerHandler::new(
            peer,
            self.peer_p2p_tx.clone(),
            self.peer_connection_tx.clone(),
        );

        // if connection with this identity already exists drop the new connection
        if let Some(_) = state.active_peers.get(&identity) {
            return Err(ServerError::DuplicatePeer);
        }

        // store peer state
        if let Some(_) = state.active_peers.insert(identity, handler) {
            unreachable!("peers should never be replaced with newly connected peers")
        }
        info!("peer {:?} added to active pool", addr);
        _ = self
            .peer_connection_tx
            .clone()
            .send((identity, PeerConnectionStatus::Connected, description))
            .await;

        Ok(())
    }

    async fn remove_connection(&self, identity: PeerIdentity) -> Option<PeerHandler> {
        let mut state = self.state.write().await;
        state.active_peers.remove(&identity)
    }

    pub async fn send_to_peer(
        &self,
        identity: PeerIdentity,
        msg: message::envelope::Msg,
    ) -> Result<(), ServerError> {
        let state = self.state.read().await;
        if let Some(peer) = state.active_peers.get(&identity) {
            // TODO: make sure actual transmission has happened
            match peer.msg_tx.send(msg).await {
                Ok(_) => return Ok(()),
                Err(e) => return Err(ServerError::PeerSendError),
            };
        }
        Err(ServerError::PeerNotFound {
            identity: peer_into_str(identity),
        })
    }

    pub async fn broadcast(
        &self,
        data: message::Data,
        exclude_addrs: Vec<PeerIdentity>,
    ) -> Result<Vec<PeerIdentity>, tokio::sync::broadcast::error::SendError<message::envelope::Msg>>
    {
        // get a list of peers excluding peers listed in exclude_addrs
        let connected_peers = self.get_peer_list(None).await;
        let peers = connected_peers.iter().filter(|p| {
            match exclude_addrs
                .iter()
                .find(|&&exclude_peer| exclude_peer.to_vec() == p.identity)
            {
                Some(_) => false,
                None => true,
            }
        });

        let msg = message::envelope::Msg::Data(data);

        let mut send_to: Vec<PeerIdentity> = Vec::new();

        for peer in peers {
            let identity: PeerIdentity = parse_identity(&peer.identity).unwrap();
            match self.send_to_peer(identity, msg.clone()).await {
                Ok(_) => send_to.push(identity),
                Err(_) => (),
            };
        }

        Ok(send_to)
    }

    // select a random peer using
    // cryptographically random number generator
    async fn random_active_peer(&self) -> Result<PeerIdentity, ServerError> {
        use rand::prelude::IteratorRandom;

        let state = self.state.read().await;
        if let Some(key) = state.active_peers.keys().choose(&mut rand::thread_rng()) {
            Ok(key.clone())
        } else {
            Err(ServerError::PeerSamplingFailed)
        }
    }

    pub async fn push(&self, rumor: message::Rumor) -> Result<(), ServerError> {
        let random_identity = match self.random_active_peer().await {
            Ok(addr) => addr,
            Err(err) => return Err(err),
        };

        // construct peer rumor packet
        let payload = message::envelope::Msg::Rumor(rumor);

        debug!("pushing to {:?}", random_identity);

        match self.send_to_peer(random_identity, payload).await {
            Ok(_) => Ok(()),
            Err(err) => Err(ServerError::PeerPushError),
        }
    }

    pub async fn pull(&self) -> Result<(), ServerError> {
        let random_identity = match self.random_active_peer().await {
            Ok(addr) => addr,
            Err(err) => return Err(err),
        };

        // request peer for its knowledge base
        // response will be handled by normal handler routine
        // FIXME: use correct ttl
        let pull_req = message::envelope::Msg::Pull(message::PullRequest {
            id: 1,
            signature: vec![],
        });

        debug!("pulling from {}", peer_into_str(random_identity));

        match self.send_to_peer(random_identity, pull_req).await {
            Ok(_) => Ok(()),
            Err(err) => Err(ServerError::PeerPushError),
        }
    }

    async fn get_peer_list(&self, exclude_addr: Option<PeerIdentity>) -> Vec<message::Peer> {
        let state = self.state.read().await;

        // IDEA: preallocate capacity since its always peers.length() - 1
        let mut peers: Vec<message::Peer> = Vec::new();

        for (identitiy, peer_handler) in state.active_peers.iter() {
            // skip over exclude_addr if provided
            if let Some(exclude) = exclude_addr {
                if exclude == *identitiy {
                    continue;
                }
            }
            peers.push(peer_handler.peer.get_peer_description())
        }

        peers
    }

    pub fn get_identity(&self) -> PeerIdentity {
        self.host_identity.clone()
    }

    pub async fn get_active_peer_identities(&self) -> Vec<PeerIdentity> {
        let state = self.state.read().await;
        state.active_peers.iter().map(|(id, _)| *id).collect()
    }

    pub async fn print_conns(&self) {
        let state = self.state.read().await;

        for (identity, peer) in &state.active_peers {
            println!(
                "connected to identity={} addr={:?}",
                peer_into_str(*identity),
                peer.peer.get_peer_addr()
            )
        }
    }
}
