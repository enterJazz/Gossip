use crate::communication::p2p::com::Communicator;
use crate::communication::p2p::peer::{Peer, PeerConnectionStatus, PeerError, PeerResult};
use log::{debug, error, info};
use std::collections::HashMap;
use std::error::Error;
use std::{io, net::SocketAddr};
use tokio::sync::mpsc;

pub struct Server {
    peers: HashMap<SocketAddr, Peer>,
    com: Communicator,
}

impl Server {
    pub async fn new(addr: String) -> Result<Self, io::Error> {
        // create base communicator instance to handle unknown incomming messages
        let (com, rx) = Communicator::new(addr, 10).await.unwrap();

        Ok(Server {
            com,
            peers: HashMap::new(),
        })
    }

    pub async fn connect_peer(&mut self, peer: &mut Peer) -> PeerResult<()> {
        // TODO: there must be a nicer way to compare enums by type and not value
        if peer.status == PeerConnectionStatus::Unknown
            && std::mem::discriminant(&peer.status)
                == std::mem::discriminant(&PeerConnectionStatus::Closed("bla".to_string()))
        {
            return Err(PeerError::err_with_peer("invalid status".to_string(), peer));
        }

        Ok(())
    }

    pub async fn register_peer(&mut self, addr: String) -> PeerResult<()> {
        let mut peer = Peer::new(addr.parse::<SocketAddr>().unwrap());

        // check if peer is already connected
        if let Some(old_peer) = self.peers.get(&peer.get_addr()) {
            if old_peer.is_active() {
                return Err(PeerError::err_with_peer(
                    "trying to register an existing peer".to_string(),
                    old_peer,
                ));
            }

            // if peer is invalid remove and continue
            self.peers.remove(&peer.get_addr());
        }

        // sanity check: there should be no collisions here
        if let Some(old_peer) = self.peers.insert(peer.get_addr(), peer.clone()) {
            return Err(PeerError(
                "invalid peer map state, no peer name collision should be possible".to_string(),
            ));
        }

        match self.connect_peer(&mut peer).await {
            Ok(()) => (),
            Err(e) => return Err(e),
        }

        Ok(())
    }

    pub async fn remove_peer(&mut self, peer: Peer) -> PeerResult<()> {
        self.peers.remove(&peer.get_addr());
        // TODO: add additional teardown

        Ok(())
    }
}
