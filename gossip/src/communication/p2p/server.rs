use crate::communication::p2p::peer::peer_into_str;
use crate::communication::p2p::peer::{Peer, PeerError, PeerResult};
use log::{debug, error, info, warn};
use rand::seq::IteratorRandom;
use std::collections::HashMap;
use std::net::AddrParseError;
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::{broadcast, mpsc, Mutex};

use super::message;
use super::peer::PeerIdentity;

// message type used by messages originating from server
type ServerPeerMessage = (message::envelope::Msg, SocketAddr);

type PeerReceiver = mpsc::Receiver<message::envelope::Msg>;
type PeerSender = mpsc::Sender<message::envelope::Msg>;
type PeerBroadcastReceiver = broadcast::Receiver<message::envelope::Msg>;
type PeerBroadcastSender = broadcast::Sender<message::envelope::Msg>;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("server peer close error")]
    PeerCloseError,
    #[error("peer not found {0}")]
    PeerNotFound(SocketAddr),
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
    active_peers: HashMap<SocketAddr, PeerHandler>,
}

pub struct Server {
    listener: TcpListener,
    state: Arc<Mutex<ServerState>>,
    host_identity: PeerIdentity,
    host_pub_key: bytes::Bytes,

    tx_sender: mpsc::Sender<ServerPeerMessage>,
    tx_receiver: mpsc::Receiver<ServerPeerMessage>,
    rx_sender: mpsc::Sender<ServerPeerMessage>,
    rx_receiver: Arc<Mutex<mpsc::Receiver<ServerPeerMessage>>>,

    broadcast_sender: broadcast::Sender<message::envelope::Msg>,
    broadcast_receiver: broadcast::Receiver<message::envelope::Msg>,

    peer_close_sender: mpsc::Sender<Option<PeerError>>,
    peer_close_receiver: Arc<Mutex<mpsc::Receiver<Option<PeerError>>>>,
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
        msg_rx_sender: mpsc::Sender<ServerPeerMessage>,
        mut broadcast_receiver: PeerBroadcastReceiver,
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
        tokio::spawn(async move {
            loop {
                tokio::select! {

                    // message comming in from peer
                    // receive message from peer and expand it by peer information
                    // so that in can easily be processes in the server level
                    peer_rx_msg = rx_read.recv() => {
                        match peer_rx_msg {
                            Some(msg) => match msg_rx_sender.send((msg, peer_addr)).await {
                                Ok(_) => (),
                                Err(e) => {
                                    error!("failed to receive message {}", e);
                                    todo!();
                                }
                            }
                            None => warn!("got empty message from peer channel")
                        }
                    }

                    // broadcast message to peer
                    broadcast_msg = broadcast_receiver.recv() => {
                        match broadcast_msg {
                            Ok(msg) => match tx_clone.send(msg).await {
                                Ok(()) => debug!("message send"),
                                Err(e) => {
                                    error!("failed to broadcast {}", e);
                                    todo!();
                                },
                            },
                            Err(e) => {
                                error!("failed to broadcast {}", e);
                                todo!();
                            }
                        }
                    }

                    close_err = close_receiver.recv() => {
                        match close_err {
                            Some(Some(err)) => {error!("peer_handler: closed with error {}", err); todo!();}
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

async fn accept_connection(
    state_ref: Arc<Mutex<ServerState>>,
    stream: TcpStream,
    host_identity: PeerIdentity,
    host_pub_key: bytes::Bytes,
    msg_rx_sender: mpsc::Sender<(message::envelope::Msg, SocketAddr)>,
    broadcast_receiver: broadcast::Receiver<message::envelope::Msg>,
) {
    let mut state = state_ref.lock().await;
    // Close incomming connection if server has reached its maximum connection counter
    if state.active_peers.keys().len() == state.max_parallel_connections {
        debug!("p2p/server/accept_connection: max number connections reached closing");
        return;
    }

    // TODO: don't forget to remove peers when they become inactive
    state.max_parallel_connections = state.max_parallel_connections + 1;

    // perform challenge acceptance in an async call to not block accept
    let state_ref = state_ref.clone();
    tokio::spawn(async move {
        let mut peer = Peer::new(stream);

        match peer.challenge(host_identity, host_pub_key).await {
            Ok(resp) => {
                let mut state = state_ref.lock().await;
                let addr = peer.remote_addr();
                info!(
                    "accept: completed challenge addr={}, id={}",
                    addr,
                    peer_into_str(peer.identity)
                );
                let handler = PeerHandler::new(peer, msg_rx_sender, broadcast_receiver);
                // store peer state
                state.active_peers.insert(addr, handler);
            }
            Err(err) => error!("p2p/server/accept: challenge failed {}", err),
        }
    });
}

async fn remove_connection(
    state_ref: Arc<Mutex<ServerState>>,
    addr: SocketAddr,
) -> Option<PeerHandler> {
    let mut state = state_ref.lock().await;

    // TODO: properly close connection

    state.active_peers.remove(&addr)
}

pub async fn run_from_str_addr(
    addr_str: &str,
    host_pub_key: bytes::Bytes,
    rx: mpsc::Sender<(message::Data, SocketAddr)>,
) -> Result<Arc<Server>, AddrParseError> {
    let addr: SocketAddr = match addr_str.parse() {
        Ok(a) => a,
        Err(err) => return Err(err),
    };

    Ok(run(addr, host_pub_key, rx).await)
}

pub async fn run(
    addr: SocketAddr,
    host_pub_key: bytes::Bytes,
    rx: mpsc::Sender<(message::Data, SocketAddr)>,
    // pub_key: [u8; 256],
) -> Arc<Server> {
    let server = Arc::new(Server::new(addr, host_pub_key).await);

    let s = server.clone();
    tokio::spawn(async move {
        match s.run(rx).await {
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
        let (rx_sender, rx_receiver) = mpsc::channel(512);
        let (tx_sender, tx_receiver) = mpsc::channel(512);
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(512);

        // channel for handling peer removal
        let (peer_close_sender, peer_close_receiver) = mpsc::channel(512);

        let identity = *blake3::hash(&host_pub_key.clone()).as_bytes();

        Server {
            state: Arc::new(Mutex::new(ServerState::new())),
            listener,
            host_pub_key: host_pub_key,
            host_identity: identity,

            broadcast_receiver,
            broadcast_sender,

            // channels for sending messages to a specific peer
            tx_receiver,
            tx_sender,

            // channel for receiving messages from peers
            rx_sender,
            rx_receiver: Arc::new(Mutex::new(rx_receiver)),

            peer_close_sender,
            peer_close_receiver: Arc::new(Mutex::new(peer_close_receiver)),
        }
    }

    pub async fn run(
        &self,
        rx: mpsc::Sender<(message::Data, SocketAddr)>,
    ) -> Result<(), ServerError> {
        // lock rx_receiver channel
        // this also ensures only one run can be executed at a time
        let mut rx_receiver = self.rx_receiver.lock().await;
        let mut peer_close_receiver = self.peer_close_receiver.lock().await;

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
                    accept_connection(
                        self.state.clone(),
                        stream,
                        self.host_identity,
                        self.host_pub_key.clone(),
                        self.rx_sender.clone(),
                        self.broadcast_sender.subscribe(),
                    ).await;
                }

                // handle messages from peers after a peer is active
                in_msg = rx_receiver.recv() => {
                    let (msg, peer_addr) = match in_msg {
                        Some(data) => data,
                        None => {
                            error!("run: received empty message");
                            continue;
                        }
                    };

                    match msg {
                        message::envelope::Msg::Data(data) => {
                            match rx.send((data, peer_addr)).await {
                                Ok(_) => (),
                                Err(err) => return Err(ServerError::RxDataChannelError),
                            };
                        }
                        _ => {
                            // TODO: handle incomming pull responses here
                            debug!("run: got internal message {:?}", msg)
                        }
                    }

                }

                 // handle peer removals in case of errors
                // does not include challenge errors (invalid peers will not be added to active pool)
                // TODO: remove peer from active pool
                peer_close = peer_close_receiver.recv() => {
                    match peer_close {
                        Some(Some(err)) => {
                            error!("peer closed with error {}", err);
                            return Err(ServerError::PeerCloseError);
                        },
                        _ => debug!("peer removing"),
                    }
                }
            }
        }
    }

    // connect to remote peer‚
    pub async fn connect(&self, addr: SocketAddr) -> PeerResult<()> {
        let mut peer = match Peer::new_from_addr(addr).await {
            Err(e) => return Err(e),
            Ok(peer) => peer,
        };

        let identity = match peer
            .connect(self.host_identity.clone(), self.host_pub_key.clone())
            .await
        {
            Err(e) => return Err(e),
            Ok(id) => id,
        };

        info!("connected: addr={} id={:?}", addr, peer_into_str(identity));

        // store connection on state
        let r = self.state.clone();
        let mut state = r.lock().await;
        let handler = PeerHandler::new(
            peer,
            self.rx_sender.clone(),
            self.broadcast_sender.subscribe(),
        );
        // store peer state
        state.active_peers.insert(addr, handler);

        info!("p2p/server/connect: peer {} added to active pool", addr);

        Ok({})
    }

    pub async fn broadcast(
        &self,
        data: message::Data,
    ) -> Result<usize, tokio::sync::broadcast::error::SendError<message::envelope::Msg>> {
        info!("broadcast sending...");
        self.broadcast_sender
            .send(message::envelope::Msg::Data(data))
    }

    // select a random peer using
    // cryptographically random number generator
    async fn sample_random_peer(&self) -> Result<SocketAddr, ServerError> {
        let state = self.state.lock().await;

        if let Some(key) = state.active_peers.keys().choose(&mut rand::thread_rng()) {
            Ok(key.clone())
        } else {
            Err(ServerError::PeerSamplingFailed)
        }
    }

    async fn push(&self) -> Result<(), ServerError> {
        let random_peer_addr = match self.sample_random_peer().await {
            Ok(addr) => addr,
            Err(err) => return Err(err),
        };

        let state = self.state.lock().await;
        let peer_handle = match state.active_peers.get(&random_peer_addr) {
            Some(peer) => peer,
            None => return Err(ServerError::PeerNotFound(random_peer_addr)),
        };

        let peers = self.get_peer_list(Some(random_peer_addr)).await;
        // construct peer rumor packet
        let rumor = message::envelope::Msg::Rumor(message::Rumor {
            // FIXME: use correct ttl
            ttl: 1,
            peers: peers,
        });

        match peer_handle.msg_tx.send(rumor).await {
            Ok(_) => Ok(()),
            Err(err) => Err(ServerError::PeerPushError),
        }
    }

    async fn pull(&self) -> Result<(), ServerError> {
        let random_peer_addr = match self.sample_random_peer().await {
            Ok(addr) => addr,
            Err(err) => return Err(err),
        };

        let state = self.state.lock().await;
        let peer_handle = match state.active_peers.get(&random_peer_addr) {
            Some(peer) => peer,
            None => return Err(ServerError::PeerNotFound(random_peer_addr)),
        };

        // request peer for its knowledge base
        // response will be handled by normal handler routine
        // FIXME: use correct ttl
        let pull_req = message::envelope::Msg::Pull(message::PullRequest { ttl: 1 });

        match peer_handle.msg_tx.send(pull_req).await {
            Ok(_) => Ok(()),
            Err(err) => Err(ServerError::PeerPullError),
        }
    }

    async fn get_peer_list(&self, exclude_addr: Option<SocketAddr>) -> Vec<message::Peer> {
        let state = self.state.lock().await;

        // IDEA: preallocate capacity since its always peers.length() - 1
        let mut peers: Vec<message::Peer> = Vec::new();

        for (addr, peer_handler) in state.active_peers.iter() {
            // skip over exclude_addr if provided
            if let Some(exclude) = exclude_addr {
                if exclude == *addr {
                    continue;
                }
            }
            peers.push(peer_handler.peer.get_peer_description())
        }

        peers
    }

    pub async fn print_conns(&self) {
        let state = self.state.lock().await;

        for (addr, peer) in &state.active_peers {
            println!("connected to {}", addr)
        }
    }
}
