use crate::communication::p2p::peer::{Peer, PeerError, PeerResult};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use std::{io, net::SocketAddr};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

struct ServerState {
    max_parallel_connections: usize,
    peers: HashMap<SocketAddr, Peer>,
}

pub struct Server {
    listener: TcpListener,
    state: Arc<Mutex<ServerState>>,
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

async fn accept_connection(state_ref: Arc<Mutex<ServerState>>, stream: TcpStream) {
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
                info!("p2p/server/accept: completed challenge; result={:?}", resp);
                let mut state = state_ref.lock().await;

                // store peer state
                state.peers.insert(peer.get_addr(), peer);
            }
            Err(err) => error!("p2p/server/accept: challenge failed {}", err),
        }
    });
}

impl Server {
    pub async fn new(addr: SocketAddr) -> Self {
        let listener = TcpListener::bind(addr).await.unwrap();

        Server {
            state: Arc::new(Mutex::new(ServerState::new())),
            listener: listener,
        }
    }

    // connect to remote peer‚
    pub async fn connect(&mut self, addr: SocketAddr) -> PeerResult<()> {
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
        state.peers.insert(peer.get_addr(), peer);

        info!("p2p/server/connect: peer {} added to active pool", addr);

        Ok({})
    }

    pub async fn run(&mut self) -> io::Result<()> {
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
                    accept_connection(self.state.clone(), stream).await;
                }
            }
        }
    }
}
