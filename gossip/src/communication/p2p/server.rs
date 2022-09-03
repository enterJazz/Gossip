use crate::communication::p2p::peer::{Peer, PeerError, PeerResult};
use log::{debug, error, info, warn};
use prost::Message;
use std::collections::HashMap;
use std::sync::Arc;
use std::{io, net::SocketAddr};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex};

use super::message::envelope::Msg;

struct ServerState {
    max_parallel_connections: usize,
    peers: HashMap<SocketAddr, PeerHandler>,
}

pub struct Server {
    listener: TcpListener,
    state: Arc<Mutex<ServerState>>,

    tx_sender: mpsc::Sender<(Msg, SocketAddr)>,
    tx_receiver: mpsc::Receiver<(Msg, SocketAddr)>,

    broadcast_sender: broadcast::Sender<Msg>,
    broadcast_receiver: broadcast::Receiver<Msg>,
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

struct PeerHandler {
    msg_rx: mpsc::Receiver<Msg>,
    msg_tx: mpsc::Sender<Msg>,
    peer: Arc<Peer>,
}

impl PeerHandler {
    fn new(peer: Peer, mut broadcast_receiver: broadcast::Receiver<Msg>) -> PeerHandler {
        let (rx_write, rx_read) = mpsc::channel(512);
        let (tx_write, tx_read) = mpsc::channel(512);

        let p = Arc::new(peer);
        let peer_run = p.clone();

        tokio::spawn(async move {
            peer_run.run(tx_read, rx_write).await;
        });

        let peer_broadcast = p.clone();
        tokio::spawn(async move {
            loop {
                match broadcast_receiver.recv().await {
                    Ok(msg) => match peer_broadcast.send_and_wrap_msg(msg).await {
                        Ok(()) => debug!("broadcast completed"),
                        Err(e) => {
                            error!("failed to broadcast {}", e);
                            todo!();
                        }
                    },
                    Err(e) => {
                        error!("failed to broadcast {}", e);
                        todo!();
                    }
                };
            }
        });

        let p = PeerHandler {
            msg_rx: rx_read,
            msg_tx: tx_write,
            peer: p,
        };

        return p;
    }
}

async fn accept_connection(
    state_ref: Arc<Mutex<ServerState>>,
    stream: TcpStream,
    mut broadcast_receiver: broadcast::Receiver<Msg>,
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
                let handler = PeerHandler::new(peer, broadcast_receiver);
                // store peer state
                state.peers.insert(addr, handler);
            }
            Err(err) => error!("p2p/server/accept: challenge failed {}", err),
        }
    });
}

impl Server {
    pub async fn new(addr: SocketAddr) -> Self {
        let listener = TcpListener::bind(addr).await.unwrap();

        let (tx_sender, tx_receiver) = mpsc::channel(512);
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(512);

        Server {
            state: Arc::new(Mutex::new(ServerState::new())),
            listener,
            broadcast_receiver,
            broadcast_sender,
            tx_receiver,
            tx_sender,
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
        let handler = PeerHandler::new(peer, self.broadcast_sender.subscribe());
        // store peer state
        state.peers.insert(addr, handler);

        info!("p2p/server/connect: peer {} added to active pool", addr);

        Ok({})
    }

    pub async fn run(&self) -> io::Result<()> {
        loop {
            tokio::select! {
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
                        self.broadcast_sender.subscribe(),
                    ).await;
                }
            }
        }
    }

    pub async fn print_conns(&self) {
        let mut state = self.state.lock().await;

        for (addr, peer) in &state.peers {
            println!("connected to {}", addr)
        }
    }

    pub async fn broadcast(
        &self,
        msg: Msg,
    ) -> Result<usize, tokio::sync::broadcast::error::SendError<Msg>> {
        self.broadcast_sender.send(msg)
    }
}
