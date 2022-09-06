use crate::communication::p2p::peer::{Peer, PeerError, PeerResult};
use log::{debug, error, info, warn};
use prost::Message;
use rand::seq::IteratorRandom;
use std::collections::HashMap;
use std::net::AddrParseError;
use std::sync::Arc;
use std::{io, net::SocketAddr};
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::{broadcast, mpsc, Mutex};

use super::message::{self, addr};

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("peer not found {0}")]
    PeerNotFound(SocketAddr),
    #[error("failed to sample random peer")]
    PeerSamplingFailed,
    #[error("push failed")]
    PeerPushError,
    #[error("pull failed")]
    PeerPullError,
}

struct ServerState {
    max_parallel_connections: usize,
    peers: HashMap<SocketAddr, PeerHandler>,
}

pub struct Server {
    listener: TcpListener,
    state: Arc<Mutex<ServerState>>,

    tx_sender: mpsc::Sender<(message::envelope::Msg, SocketAddr)>,
    tx_receiver: mpsc::Receiver<(message::envelope::Msg, SocketAddr)>,
    rx_sender: mpsc::Sender<(message::envelope::Msg, SocketAddr)>,
    rx_receiver: Arc<Mutex<mpsc::Receiver<(message::envelope::Msg, SocketAddr)>>>,

    broadcast_sender: broadcast::Sender<message::envelope::Msg>,
    broadcast_receiver: broadcast::Receiver<message::envelope::Msg>,
}

impl ServerState {
    /// Create a new, empty, instance of `Shared`.
    fn new() -> Self {
        ServerState {
            peers: HashMap::new(),
            max_parallel_connections: 10,
        }
    }
}

type PeerReceiver = mpsc::Receiver<message::envelope::Msg>;
type PeerSender = mpsc::Sender<message::envelope::Msg>;
type PeerBroadcastReceiver = broadcast::Receiver<message::envelope::Msg>;
type PeerBroadcastSender = broadcast::Sender<message::envelope::Msg>;

struct PeerHandler {
    msg_tx: PeerSender,
    peer: Arc<Peer>,
}

impl PeerHandler {
    fn new(
        peer: Peer,
        msg_rx_sender: mpsc::Sender<(message::envelope::Msg, SocketAddr)>,
        mut broadcast_receiver: PeerBroadcastReceiver,
    ) -> PeerHandler {
        let (rx_write, mut rx_read) = mpsc::channel(512);
        let (tx_write, tx_read) = mpsc::channel(512);

        let p = Arc::new(peer);
        let peer_run = p.clone();

        tokio::spawn(async move {
            peer_run.run(tx_read, rx_write).await;
        });

        let tx_clone = tx_write.clone();
        let peer_addr = p.get_addr();
        tokio::spawn(async move {
            loop {
                info!("running select");
                tokio::select! {

                    // message comming in from peer
                    // receive message from peer and expand it by peer information
                    // so that in can easily be processes in the server level
                    peer_rx_msg = rx_read.recv() => {
                        info!("rx_read.recv()");
                        match peer_rx_msg {
                            Some(msg) => match msg_rx_sender.send((msg, peer_addr)).await {
                                Ok(()) => debug!("recv completed"),
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
                        info!("got broadcast message");
                        match broadcast_msg {
                            Ok(msg) => match tx_clone.send(msg).await {
                                Ok(()) => debug!("message send"),
                                Err(e) => {
                                    error!("failed to broadcast {}", e);
                                },
                            },
                            Err(e) => {
                                error!("failed to broadcast {}", e);
                                todo!();
                            }
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
    msg_rx_sender: mpsc::Sender<(message::envelope::Msg, SocketAddr)>,
    broadcast_receiver: broadcast::Receiver<message::envelope::Msg>,
) {
    let mut state = state_ref.lock().await;
    // Close incomming connection if server has reached its maximum connection counter
    if state.peers.keys().len() == state.max_parallel_connections {
        debug!("p2p/server/accept_connection: max number connections reached closing");
        return;
    }
    state.max_parallel_connections = state.max_parallel_connections + 1;

    // perform challenge acceptance in an async call to not block accept
    let state_ref = state_ref.clone();
    tokio::spawn(async move {
        let mut peer = Peer::new(stream);

        match peer.challenge().await {
            Ok(resp) => {
                let mut state = state_ref.lock().await;
                let addr = peer.get_addr();
                info!(
                    "p2p/server/accept: completed challenge; result={:?} addr={}",
                    resp, addr
                );
                let handler = PeerHandler::new(peer, msg_rx_sender, broadcast_receiver);
                // store peer state
                state.peers.insert(addr, handler);
            }
            Err(err) => error!("p2p/server/accept: challenge failed {}", err),
        }
    });
}

pub async fn run_p2p_server(
    addr_str: &str,
    // tx: mpsc::Receiver<(message::envelope::Msg, SocketAddr)>,
    // rx: mpsc::Sender<(message::envelope::Msg, SocketAddr)>,
    // pub_key: [u8; 256],
) -> Result<Arc<Server>, AddrParseError> {
    let addr: SocketAddr = match addr_str.parse() {
        Ok(a) => a,
        Err(err) => return Err(err),
    };

    let server = Arc::new(Server::new(addr).await);

    let s = server.clone();
    tokio::spawn(async move {
        match s.run().await {
            Ok(()) => info!("p2p/server: executed finished"),
            Err(err) => error!("p2p/server: execution aborted due to error {}", err),
        }
    });

    Ok(server)
}

impl Server {
    pub async fn new(addr: SocketAddr) -> Self {
        let listener = TcpListener::bind(addr).await.unwrap();

        let (rx_sender, rx_receiver) = mpsc::channel(512);
        let (tx_sender, tx_receiver) = mpsc::channel(512);
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(512);

        Server {
            state: Arc::new(Mutex::new(ServerState::new())),
            listener,
            broadcast_receiver,
            broadcast_sender,

            // channels for sending messages to a specific peer
            tx_receiver,
            tx_sender,

            // channel for receiving messages from peers
            rx_sender,
            rx_receiver: Arc::new(Mutex::new(rx_receiver)),
        }
    }

    // connect to remote peer‚
    pub async fn connect(&self, addr: SocketAddr) -> PeerResult<()> {
        let mut peer = match Peer::new_from_addr(addr).await {
            Err(e) => return Err(e),
            Ok(peer) => peer,
        };

        match peer.connect().await {
            Err(e) => return Err(e),
            Ok(()) => info!("p2p/server/connect: connected to {}", addr),
        };

        // store connection on state
        let r = self.state.clone();
        let mut state = r.lock().await;
        let handler = PeerHandler::new(
            peer,
            self.rx_sender.clone(),
            self.broadcast_sender.subscribe(),
        );
        // store peer state
        state.peers.insert(addr, handler);

        info!("p2p/server/connect: peer {} added to active pool", addr);

        Ok({})
    }

    pub async fn run(&self) -> io::Result<()> {
        // lock rx_receiver channel
        // this also ensures only one run can be executed at a time
        let mut rx_receiver = self.rx_receiver.lock().await;

        loop {
            tokio::select! {
                // accept new incomming messages
                res = self.listener.accept() => {
                    let (stream, addr) = match res {
                        Ok(res) => res,
                        Err(e) => {
                            error!("p2p/server/accept: failed to connect with listener: {:?}", e);
                            continue;
                        }
                    };

                    info!("p2p/server/accept: incoming connection from {}", addr);
                    accept_connection(
                        self.state.clone(),
                        stream,
                        self.rx_sender.clone(),
                        self.broadcast_sender.subscribe(),
                    ).await;
                }

                // handle messages from peers after a peer is active
                in_msg = rx_receiver.recv() => {
                    let (msg, peer_addr) = match in_msg {
                        Some(data) => data,
                        None => {
                            error!("p2p/server/recv: received empty message");
                            continue;
                        }
                    };

                    todo!("peer messages not handled yet {:?} {}", msg, peer_addr)
                }
            }
        }
    }

    pub async fn broadcast(
        &self,
        msg: message::envelope::Msg,
    ) -> Result<usize, tokio::sync::broadcast::error::SendError<message::envelope::Msg>> {
        info!("broadcast sending...");
        self.broadcast_sender.send(msg)
    }

    async fn get_peer_list(&self, exclude_addr: Option<SocketAddr>) -> Vec<message::Peer> {
        let state = self.state.lock().await;

        // IDEA: preallocate capacity since its always peers.length() - 1
        let mut peers: Vec<message::Peer> = Vec::new();

        for (addr, peer_handler) in state.peers.iter() {
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

    // select a random peer using
    // cryptographically random number generator
    async fn sample_random_peer(&self) -> Result<SocketAddr, ServerError> {
        let state = self.state.lock().await;

        if let Some(key) = state.peers.keys().choose(&mut rand::thread_rng()) {
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
        let peer_handle = match state.peers.get(&random_peer_addr) {
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
        let peer_handle = match state.peers.get(&random_peer_addr) {
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

    pub async fn print_conns(&self) {
        let state = self.state.lock().await;

        for (addr, peer) in &state.peers {
            println!("connected to {}", addr)
        }
    }
}
