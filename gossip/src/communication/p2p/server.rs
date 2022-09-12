use crate::communication::p2p::peer::{parse_identity, peer_into_str};
use crate::communication::p2p::peer::{Peer, PeerError};
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
use super::peer::{compute_identity, PeerConnectionStatus, PeerIdentity};
use super::peer_handler::PeerHandler;

// message type used by messages originating from server
pub type ServerPeerMessage = (message::envelope::Msg, PeerIdentity, SocketAddr);
pub type PeerConnectionMessage = (PeerIdentity, PeerConnectionStatus, message::Peer);

type PeerReceiver = mpsc::Receiver<message::envelope::Msg>;
pub type PeerSender = mpsc::Sender<message::envelope::Msg>;
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

/// State maintained by the P2P Server
struct ServerState {
    max_parallel_connections: usize,
    active_peers: HashMap<PeerIdentity, PeerHandler>,
}

impl ServerState {
    /// Create a new, empty, instance of `ServerState`.
    fn new() -> Self {
        ServerState {
            active_peers: HashMap::new(),
            max_parallel_connections: 100,
        }
    }
}

/// Server instance handling all P2P connections and communications
pub struct Server {
    /// Underlying TCP Listener for incoming connections
    listener: TcpListener,
    /// Current server state storing active peer statuses
    state: Arc<RwLock<ServerState>>,

    /// Address on which the server is running
    addr: SocketAddr,

    /// Blake3 Hash of the DER encoded host public key
    host_identity: PeerIdentity,

    /// DER encoded public key of the server
    host_pub_key: bytes::Bytes,

    /// Channel for transmitting messeages from peers to main loop for processing
    peer_p2p_tx: mpsc::Sender<ServerPeerMessage>,

    /// Channel for receiving messeages from peers on the main loop for processing
    peer_p2p_rx: Arc<Mutex<mpsc::Receiver<ServerPeerMessage>>>,

    /// Channels for sending peer connection updates
    peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
    /// Channels for receiving peer connection updates
    peer_connection_rx: Arc<Mutex<mpsc::Receiver<PeerConnectionMessage>>>,
}

/// Runs the P2P server on a string encoded local address (Ip+Port)
///
/// # Arguments
///
/// * `addr_str` - IP:port of the P2P server on the local system e.g. 127.0.0.1:5005
/// * `host_pub_key` - Public key of the host machine in DER format
/// * `tx` - mpsc channel on which incoming messages will be received
/// * `peer_connection_tx` - mpsc channel on connection status updates will be transmitted
pub async fn run_from_str_addr(
    addr_str: &str,
    host_pub_key: bytes::Bytes,
    tx: mpsc::Sender<ServerPeerMessage>,
    peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
) -> Result<Arc<Server>, AddrParseError> {
    // parse address
    let addr: SocketAddr = match addr_str.parse() {
        Ok(a) => a,
        Err(err) => return Err(err),
    };

    Ok(run(addr, host_pub_key, tx, peer_connection_tx).await)
}

/// Runs the P2P server on a SocketAddr
///
/// # Arguments
///
/// * `addr` - run target for p2p server as SocketAddr instance
/// * `host_pub_key` - Public key of the host machine in DER format
/// * `tx` - mpsc channel on which incoming messages will be received
/// * `peer_connection_tx` - mpsc channel on connection status updates will be transmitted
pub async fn run(
    addr: SocketAddr,
    host_pub_key: bytes::Bytes,
    tx: mpsc::Sender<ServerPeerMessage>,
    peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
) -> Arc<Server> {
    let server = Arc::new(Server::new(addr, host_pub_key).await);

    let s = server.clone();
    tokio::spawn(async move {
        match s.run(tx, peer_connection_tx).await {
            Ok(()) => info!("p2p/server: executed finished"),
            Err(err) => error!("p2p/server: execution aborted due to error {}", err),
        }
    });

    server
}

impl Server {
    /// Creates a new Server instance without starting the run loop
    ///
    /// # Arguments
    ///
    /// * `addr` - run target for p2p server as SocketAddr instance
    /// * `host_pub_key` - Public key of the host machine in DER format
    pub async fn new(addr: SocketAddr, host_pub_key: bytes::Bytes) -> Self {
        // construct underlaying TCP connection
        // forcefully unwrap here, it is expected that server dies if tcp listener cannot be started
        let listener = TcpListener::bind(addr).await.unwrap();

        // create channels for peer orchestration and handling
        let (peer_p2p_tx, peer_p2p_rx) = mpsc::channel(512);

        // channel for handling peer removal
        let (peer_connection_tx, peer_connection_rx) = mpsc::channel(512);

        let identity = compute_identity(&host_pub_key.clone());

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

    /// Run main server loop. Handles incoming connections to peers and formwards
    /// a filtered subset of messages to the external p2p_external_tx channel.
    /// Connection status updates are forwarded to connection_p2p_external_tx when
    /// a peer connects or disconnects from the server.
    /// NOTE: run cannot be called multiple times, if called multiple times this method
    /// will block until the first call terminates.
    ///
    /// # Arguments
    ///
    /// * `p2p_external_tx` - Channel for all incoming messages from other peers
    /// * `connection_p2p_external_tx` - Channel for connection update notifications
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
                    match self.accept(stream).await {
                        Ok(_) => debug!("peer connection accepted"),
                        Err(err) => warn!("failed to accept peer connection: {}", err)
                    }
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
                        // forward selected subset of messages to upper layer
                        // - Pull, PullResponse, Rumor are used for Gossip Push&Pull
                        // - Data is for message passing between peers
                        message::envelope::Msg::Pull(_) |
                        message::envelope::Msg::Data(_) |
                        message::envelope::Msg::PullResponse(_) |
                        message::envelope::Msg::Rumor(_)  => {
                            match p2p_external_tx.send((msg, identity, peer_addr)).await {
                                Ok(_) => (),
                                Err(_) => return Err(ServerError::RxDataChannelError),
                            };
                        }
                        // peers should not pass connection related messages
                        _ => {
                            unreachable!("peer send internal message over external communication channel: {:?}", msg)
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

    /// Accept handles incomming connections and initiates the PoW connection process.
    /// The challenge is always stated by the accepting party.
    ///
    /// # Arguments
    ///
    /// * `stream` - Incoming TCP stream
    async fn accept(&self, stream: TcpStream) -> Result<(), ServerError> {
        if !self.can_accept_more_peers().await {
            return Err(ServerError::PeerLimitReached);
        }

        let host_identity = self.get_identity();
        let host_pub_key = self.host_pub_key.clone();
        let mut peer = Peer::new(stream);

        // execute peer challenge
        // only add connection after successful PoW challenge process
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

    /// Connect is the counterpart to accept and initiates a connection to a remove P2P server.
    /// This will trigger accept on the remote server and start the PoW connection process.
    ///
    /// # Arguments
    ///
    /// * `addr` - Address of the P2P server to which to establish a connection
    pub async fn connect<T: ToSocketAddrs>(&self, addr: T) -> Result<(), ServerError> {
        if !self.can_accept_more_peers().await {
            return Err(ServerError::PeerLimitReached);
        }

        let mut peer = match Peer::new_from_addr(addr).await {
            Err(err) => return Err(ServerError::PeerConnectionError(err)),
            Ok(peer) => peer,
        };

        // process the PoW connection hanshake
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

    /// Remove a peer given its Identity from the active peer pool
    pub async fn disconnect(&self, identity: PeerIdentity) -> Option<message::Peer> {
        self.remove_connection(identity)
            .await
            .map_or(None, |p| Some(p.get_description()))
    }

    /// Add a previosly established Peer connection (after PoW) to the active connection pool
    ///
    /// # Arguments
    ///
    /// * `identity` - Blake3 Hash of peer DER encoded public key
    /// * `peer` - Peer instance wrapping the underlying TCP Stream
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

    // Remove a connection from the active pool and call internal cleanup methods
    async fn remove_connection(&self, identity: PeerIdentity) -> Option<PeerHandler> {
        let mut state = self.state.write().await;
        if let Some(mut peer) = state.active_peers.remove(&identity) {
            peer.shutdown().await;
            return Some(peer);
        }
        None
    }

    /// Transmit a Message to an individual peer given its Identity
    ///
    /// # Arguments
    ///
    /// * `identity` - Blake3 Hash of peer DER encoded public key
    /// * `msg` - Message to be transmitted to the peer
    pub async fn send_to_peer(
        &self,
        identity: PeerIdentity,
        msg: message::envelope::Msg,
    ) -> Result<(), ServerError> {
        let state = self.state.read().await;
        if let Some(peer) = state.active_peers.get(&identity) {
            match peer.msg_tx.send(msg).await {
                Ok(_) => return Ok(()),
                Err(e) => return Err(ServerError::PeerSendError),
            };
        }
        Err(ServerError::PeerNotFound {
            identity: peer_into_str(identity),
        })
    }

    /// Broadcasts the message to all active peers with identities matching the exclude_identities list
    ///
    /// # Arguments
    ///
    /// * `data` - Data to be transmitted to all active peers not included in exclude_identities list
    /// * `exclude_identities` - Slice of identities that will be ignored during broadcast, these identities will also be omitted in the sulting PeerIdentity list in the result
    pub async fn broadcast(
        &self,
        data: message::Data,
        exclude_identities: Vec<PeerIdentity>,
    ) -> Result<Vec<PeerIdentity>, tokio::sync::broadcast::error::SendError<message::envelope::Msg>>
    {
        // get a list of peers excluding peers listed in exclude_identities
        let connected_peers = self.get_peer_list(None).await;
        let peers = connected_peers.iter().filter(|p| {
            match exclude_identities
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

    /// Naive Gossip Push implementation
    /// - select a random peer from the active peer list
    /// - pass rumor/view to the selected peer
    pub async fn push(
        &self,
        identity: Option<PeerIdentity>,
        rumor: message::Rumor,
    ) -> Result<(), ServerError> {
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

    /// Naive Gossip Pull implementation
    /// - sample a random peer
    /// - send pull request with random identifier
    /// - wait for response in incomming messages
    pub async fn pull(&self, identity: Option<PeerIdentity>) -> Result<(), ServerError> {
        let random_identity = match self.random_active_peer().await {
            Ok(addr) => addr,
            Err(err) => return Err(err),
        };

        // request peer for its knowledge base
        // response will be handled by normal handler routine
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

    /// Returns a list of currently conencted peers with extended information
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
            peers.push(peer_handler.get_description())
        }

        peers
    }

    /// Returns the identity of the host running the server
    pub fn get_identity(&self) -> PeerIdentity {
        self.host_identity.clone()
    }

    /// Returns a list of currently conencted peer identifiers
    pub async fn get_active_peer_identities(&self) -> Vec<PeerIdentity> {
        let state = self.state.read().await;
        state.active_peers.iter().map(|(id, _)| *id).collect()
    }

    async fn can_accept_more_peers(&self) -> bool {
        let state = self.state.read().await;
        // Close incomming connection if server has reached its maximum connection counter
        if state.active_peers.keys().len() == state.max_parallel_connections {
            debug!("p2p/server/accept_connection: max number connections reached closing");
            return false;
        }
        return true;
    }
}
