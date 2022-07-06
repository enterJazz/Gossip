use crate::communication::p2p::peer::{Peer, PeerConnectionStatus, PeerError, PeerResult};
use log::{debug, error, info};
use std::collections::HashMap;
use std::sync::Arc;
use std::{io, net::SocketAddr};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

/// Shorthand for the transmit half of the message channel.
type Tx = mpsc::UnboundedSender<String>;

struct ServerState {
    peers: HashMap<SocketAddr, (Peer, Rx)>,
}

pub struct Server {
    //  com: Communicator,
    state: Arc<Mutex<ServerState>>,
}

impl ServerState {
    /// Create a new, empty, instance of `Shared`.
    fn new() -> Self {
        ServerState {
            peers: HashMap::new(),
        }
    }
}

impl Server {
    pub async fn new() -> Result<Self, io::Error> {
        Ok(Server {
            state: Arc::new(Mutex::new(ServerState::new())),
        })
    }

    pub async fn run(&mut self, addr: String) -> io::Result<()> {
        let listener = TcpListener::bind(addr).await?;

        loop {
            let (socket, _) = listener.accept().await?;

            // Clone a handle to the `Shared` state for the new connection.
            let state = Arc::clone(&self.state);

            tokio::spawn(async move {
                Server::process_connection(state, socket);
            });
        }
    }

    pub async fn connect_with_addr(&mut self, addr: SocketAddr) -> PeerResult<()> {
        // Clone a handle to the `Shared` state for the new connection.
        let state = Arc::clone(&self.state);
        match Peer::new_from_addr(addr).await {
            Ok(peer) => Server::register_peer(state, peer).await,
            Err(err) => Err(err),
        }
    }

    async fn process_connection(state: Arc<Mutex<ServerState>>, socket: TcpStream) {
        let mut peer = Peer::new(socket).await;

        if let Err(err) = Server::register_peer(state, peer).await {
            error!("failed to register peer");
            return;
        }
    }

    async fn register_peer(state: Arc<Mutex<ServerState>>, mut peer: Peer) -> PeerResult<()> {
        let mut state = state.lock().await;
        let addr = peer.get_addr();
        // check if peer is already connected
        if let Some((old_peer, rx)) = state.peers.get(&addr) {
            if old_peer.is_active() {
                return Err(PeerError::err_with_peer(
                    "trying to register an existing peer".to_string(),
                    old_peer,
                ));
            }

            // if peer is invalid remove and restart connection
            state.peers.remove(&addr);
        }

        peer.connect().await;

        state.peers.insert(addr, peer);
        Ok(())
    }

    pub async fn remove_peer(state: Arc<Mutex<ServerState>>, addr: SocketAddr) -> PeerResult<()> {
        let mut state = state.lock().await;
        if let Some(mut peer) = state.peers.remove(&addr) {
            // peer.disconnect();
        } else {
            return Err(PeerError("remove failed peer not found".to_string()));
        }

        // TODO: add additional teardown

        Ok(())
    }
}
