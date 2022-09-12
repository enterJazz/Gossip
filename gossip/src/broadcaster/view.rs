use crate::communication::p2p;
use log::{debug, info};
use std::{
    collections::HashMap,
    fmt::{self, Display},
    net::SocketAddr,
};

#[derive(Clone)]
pub struct PeerViewItem {
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
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn get_addr(&self) -> Option<SocketAddr> {
        if let Some(addr) = &self.info.address {
            p2p::peer::from_addr(addr)
        } else {
            None
        }
    }
}

impl Display for PeerViewItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}|{}] [active:{}] ",
            p2p::peer::from_addr(self.info.address.as_ref().unwrap()).unwrap(),
            p2p::peer::peer_into_str(p2p::peer::parse_identity(&self.info.identity).unwrap()),
            self.is_active,
        )
    }
}

pub struct View {
    cache_size: usize,

    pub known_peers: HashMap<p2p::peer::PeerIdentity, PeerViewItem>,
}

impl View {
    pub fn new(cache_size: usize) -> Self {
        View {
            cache_size,
            known_peers: HashMap::new(),
        }
    }

    /// Merge combines the current view with incoming information about peers
    pub fn merge(&mut self, rumor: p2p::message::Rumor) {
        // find peers we know nothing about yet
        let new_unknown_peers: Vec<(p2p::peer::PeerIdentity, p2p::message::Peer)> = rumor
            .peers
            .iter()
            .filter_map(|peer| {
                if let Some(identity) = p2p::peer::parse_identity(&peer.identity) {
                    // only return item if we don't already know about it
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

            info!(
                "adding peer with identity {:?}",
                p2p::peer::peer_into_str(identity)
            );

            self.known_peers.insert(identity, PeerViewItem::from(peer));
        }
    }

    pub fn remove_peer(&mut self, identity: &p2p::peer::PeerIdentity) -> Option<PeerViewItem> {
        self.known_peers.remove(identity)
    }

    pub fn process_peer_update(
        &mut self,
        identity: p2p::peer::PeerIdentity,
        status: p2p::peer::PeerConnectionStatus,
        peer: p2p::message::Peer,
    ) {
        let is_active = match status {
            p2p::peer::PeerConnectionStatus::Connected => true,
            p2p::peer::PeerConnectionStatus::Closed(_) => false,
            _ => false,
        };

        let item = PeerViewItem {
            info: peer,
            is_active: is_active,
        };

        self.known_peers.insert(identity, item);
    }

    pub fn into_rumor(self) -> p2p::message::Rumor {
        p2p::message::Rumor {
            signature: vec![],
            peers: self
                .known_peers
                .into_values()
                .map(|peer| peer.info)
                .collect(),
        }
    }
}

impl Display for View {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        _ = writeln!(f, "");
        _ = writeln!(
            f,
            "----------------------------------------------------------------------------"
        );
        _ = writeln!(
            f,
            "                                  peer view                                 "
        );
        _ = writeln!(
            f,
            "----------------------------------------------------------------------------"
        );
        for (_, peer) in &self.known_peers {
            _ = writeln!(f, "{}", peer);
        }
        writeln!(
            f,
            "----------------------------------------------------------------------------"
        )
    }
}

impl Clone for View {
    fn clone(&self) -> Self {
        Self {
            cache_size: self.cache_size,
            known_peers: self.known_peers.clone(),
        }
    }
}
