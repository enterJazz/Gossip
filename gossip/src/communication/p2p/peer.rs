use crate::communication::p2p::message::{envelope, Envelope, VerificationRequest};
use log::{debug, error, info, log_enabled, Level};
use std::sync::Arc;
use std::{io, net::SocketAddr};
use thiserror::Error;
use tokio::net::{tcp, TcpStream};
use tokio::sync::{mpsc, Mutex};

use prost::Message;
#[derive(Debug, Clone)]
pub struct PeerError(pub String);

/// Shorthand for the receive half of the message channel.
type Rx = mpsc::UnboundedReceiver<String>;

impl PeerError {
    // utility method to log peer information
    pub fn err_with_peer(msg: String, peer: &Peer) -> Self {
        return PeerError(format!(
            "peer error: {} (addr:{}, status: {}) ",
            msg,
            peer.get_addr(),
            peer.status.to_string()
        ));
    }
}

#[derive(Error, Debug)]
pub enum RecvError {
    #[error("protoc decode error")]
    Decode(#[from] prost::DecodeError),
    #[error("stream read error")]
    Read(#[from] io::Error),
    #[error("read invalid length {0}")]
    InvalidLength(usize),
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
    fn to_string(&self) -> String {
        match self {
            PeerConnectionStatus::Unknown => "unknown".to_string(),
            PeerConnectionStatus::Connecting => "connecting".to_string(),
            PeerConnectionStatus::Connected => "connected".to_string(),
            PeerConnectionStatus::Closed(reason) => format!("closed({})", reason),
        }
    }
}

pub struct Peer {
    pub socket: Arc<Mutex<TcpStream>>,
    pub status: PeerConnectionStatus,
    addr: SocketAddr,
}

impl Peer {
    pub async fn new(socket: TcpStream) -> Self {
        let addr = socket.local_addr().unwrap();

        return Peer {
            socket: Arc::new(Mutex::new(socket)),
            addr,
            // TODO: handle
            status: PeerConnectionStatus::Unknown,
        };
    }

    pub async fn new_from_addr(addr: SocketAddr) -> PeerResult<Self> {
        match TcpStream::connect(addr).await {
            Ok(socket) => Ok(Peer::new(socket).await),
            Err(e) => Err(PeerError("could not connect".to_string())),
        }
    }

    pub fn disconnect(&mut self) -> PeerResult<()> {
        // TODO: implement teardown if required
        self.status = PeerConnectionStatus::Closed("graceful shutdown".to_string());
        Ok(())
    }

    pub async fn connect(&mut self) -> PeerResult<()> {
        let challenge_req = Envelope {
            msg: Some(envelope::Msg::VerificationRequest(VerificationRequest {
                challenge: "test".as_bytes().to_vec(),
                pub_key: "bla".as_bytes().to_vec(),
            })),
        };
        if let Err(err) = Peer::send_msg(self.socket.clone(), challenge_req).await {
            error!("failed to send challenge {}", err.to_string());
            return Err(PeerError("challenge failed".to_string()));
        }

        let socket = self.socket.clone();
        // wait for response
        // TODO: add delay
        tokio::spawn(async move { return Peer::read_msg(socket).await }).await;

        Ok(())
    }

    pub async fn send_msg(socket: Arc<Mutex<TcpStream>>, msg: Envelope) -> io::Result<usize> {
        let s = socket.lock().await;
        let buf = msg.encode_to_vec();
        return s.try_write(&buf);
    }

    pub async fn read_msg(socket: Arc<Mutex<TcpStream>>) -> Result<Envelope, RecvError> {
        let mut buf = [0; 8192];
        let s = socket.lock().await;

        match s.try_read(&mut buf) {
            Ok(0) => Err(RecvError::InvalidLength(0)),
            Ok(n) => match Envelope::decode(&buf[..n]) {
                Ok(msg) => Ok(msg),
                Err(err) => Err(RecvError::Decode(err)),
            },
            // Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Err(RecvError::Read(e)),
            Err(e) => {
                return Err(e.into());
            }
        }
    }

    pub fn get_addr(&self) -> SocketAddr {
        return self.addr;
    }

    pub fn is_active(&self) -> bool {
        self.status == PeerConnectionStatus::Connected
            || self.status == PeerConnectionStatus::Connecting
    }
}
