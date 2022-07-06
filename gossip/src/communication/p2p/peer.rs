use std::{io, net::SocketAddr};

#[derive(Debug, Clone)]
pub struct PeerError(pub String);

impl PeerError {
    // utility method to log peer information
    pub fn err_with_peer(msg: String, peer: &Peer) -> Self {
        return PeerError(format!(
            "peer error: {} (addr:{}, status: {}) ",
            msg,
            peer.addr,
            peer.status.as_str()
        ));
    }
}

pub type PeerResult<T> = std::result::Result<T, PeerError>;

#[derive(PartialEq, Clone)]
pub enum PeerConnectionStatus {
    // indicates peer was newly created and has not begun pairing process
    Unknown,
    // peer is in the connection process
    Connecting,
    // peer was connected to successfully
    Connected,
    // peer connection was aborted
    Closed(String),
}

impl PeerConnectionStatus {
    fn as_str(&self) -> &'static str {
        match self {
            PeerConnectionStatus::Unknown => "unknown",
            PeerConnectionStatus::Connecting => "connecting",
            PeerConnectionStatus::Connected => "connected",
            PeerConnectionStatus::Closed(reason) => format!("closed({})", reason),
        }
    }
}

#[derive(Clone)]
pub struct Peer {
    addr: SocketAddr,
    pub status: PeerConnectionStatus,
}

impl Peer {
    pub fn new(addr: SocketAddr) -> Self {
        return Peer {
            // TODO: handle parsing errors
            addr,
            status: PeerConnectionStatus::Unknown,
        };
    }

    pub fn get_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn is_active(&self) -> bool {
        self.status == PeerConnectionStatus::Connected
            || self.status == PeerConnectionStatus::Connecting
    }
}
