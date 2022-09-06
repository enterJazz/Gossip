use crate::communication::p2p::message::{
    envelope, Envelope, VerificationRequest, VerificationResponse,
};
use bytes::BytesMut;
use log::{debug, error, info, log_enabled, warn, Level};
use prost::Message;
use std::net::{IpAddr, Ipv4Addr};
use std::str;
use std::sync::Arc;
use std::{io, net::SocketAddr};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::{tcp, TcpStream};
use tokio::sync::{mpsc, Mutex};

use super::message;

/// Shorthand for the receive half of the message channel.
type RxStreamHalf = Arc<Mutex<tcp::OwnedReadHalf>>;
type TxStreamHalf = Arc<Mutex<tcp::OwnedWriteHalf>>;

#[derive(Error, Debug)]
pub enum ChallengeError {
    #[error("challenge exechange error")]
    ExchangeError,
    #[error("remote solution check failed")]
    CheckFailed,
}

#[derive(Error, Debug)]
pub enum PeerError {
    #[error("peer error: connection failed {0}")]
    Connection(#[from] io::Error),
    #[error("peer error: challenge failed {0}")]
    Challenge(#[from] ChallengeError),
    #[error("peer error: could not read message")]
    Read(#[from] RecvError),
    #[error("peer error: could not send message")]
    Write(#[from] SendError),
}

#[derive(Error, Debug)]
pub enum RecvError {
    #[error("protoc decode error")]
    Decode(#[from] prost::DecodeError),
    #[error("received empty envelope")]
    EmptyEnvelope,
    #[error("stream read error")]
    Read(#[from] io::Error),
    #[error("read invalid length {0}")]
    InvalidLength(usize),
}

#[derive(Error, Debug)]
pub enum SendError {
    #[error("protoc encode error")]
    Encode(#[from] prost::EncodeError),
    #[error("stream write error")]
    Write(#[from] io::Error),
    #[error("write invalid length {0}")]
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

/// A connection to a remote Peer on the network
pub struct Peer {
    /// TCP connection read half
    pub rx: RxStreamHalf,
    /// TCP connection write half
    pub tx: TxStreamHalf,

    /// Connection status indicating verification state
    pub status: PeerConnectionStatus,

    /// Remote peer address
    remote_addr: SocketAddr,

    /// Hash of public key and ip address and port of a peer (only set after completed handshake)
    pub identity: [u8; 256],
    /// Public key of remote peer (only set after completed handshake)
    pub pub_key: [u8; 256],

    /// Internal buffer for reading incomming messages
    read_buffer: BytesMut,
}

impl Peer {
    /// Returns a new Peer instance given a TCP connection
    /// No handshake is performed at this step
    ///
    /// # Arguments
    ///
    /// * `stream` - TCP stream inteded for all communications to peer
    pub fn new(stream: TcpStream) -> Self {
        let remote_addr = stream.peer_addr().unwrap();

        let (rx, tx) = TcpStream::into_split(stream);
        return Peer {
            rx: Arc::new(Mutex::new(rx)),
            tx: Arc::new(Mutex::new(tx)),
            remote_addr,
            // TODO: handle
            status: PeerConnectionStatus::Unknown,
            read_buffer: BytesMut::with_capacity(8 * 1024),
            identity: [0; 256],
            pub_key: [0; 256],
        };
    }

    pub async fn new_from_addr(addr: SocketAddr) -> PeerResult<Self> {
        match TcpStream::connect(addr).await {
            Ok(socket) => Ok(Peer::new(socket)),
            Err(e) => Err(PeerError::Connection(e)),
        }
    }

    pub fn disconnect(&mut self) -> PeerResult<()> {
        // TODO: implement teardown if required
        self.status = PeerConnectionStatus::Closed("graceful shutdown".to_string());
        Ok(())
    }

    // initiate a connection process to the remote defined by the underlying TCP Stream
    pub async fn connect(&mut self) -> PeerResult<()> {
        let rx = self.rx.clone();

        debug!(
            "p2p/peer/connect: reading message from {}",
            self.remote_addr()
        );

        debug!("p2p/peer/connect: finished reading message");

        let req = match self.read_and_unwrap_msg().await? {
            envelope::Msg::VerificationRequest(req) => req.challenge,
            _ => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
        };

        info!(
            "P2P peer: got challenge value: {}",
            str::from_utf8(&req.to_vec()).unwrap()
        );

        // TODO: use actual validation here
        // for now just send some placeholder

        let resp = envelope::Msg::VerificationResponse(message::VerificationResponse {
            challenge: req.to_vec(),
            nonce: [0; 256].to_vec(),
            pub_key: [0; 256].to_vec(),
            signature: [0; 256].to_vec(),
        });

        // send challenge result
        match self.send_and_wrap_msg(resp).await {
            Ok(()) => (),
            Err(err) => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
        }

        // wait for connection status
        let challenge_status = match self.read_and_unwrap_msg().await? {
            envelope::Msg::VerificationValidationResponse(resp) => resp.status,
            _ => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
        };

        // verify remote response
        match message::VerificationResponseStatus::from_i32(challenge_status) {
            Some(message::VerificationResponseStatus::Ok) => Ok(()),
            _ => Err(PeerError::Challenge(ChallengeError::CheckFailed)),
        }
    }

    // initiate the connection process by publishing challenge to the other party defined by the underlying
    // TCP Stream
    pub async fn challenge(&mut self) -> PeerResult<()> {
        let challenge_req = envelope::Msg::VerificationRequest(VerificationRequest {
            challenge: "test".as_bytes().to_vec(),
            pub_key: "bla".as_bytes().to_vec(),
        });

        if let Err(err) = self.send_and_wrap_msg(challenge_req).await {
            error!("P2P peer: failed to send challenge {}", err.to_string());
            return Err(PeerError::Challenge(ChallengeError::ExchangeError));
        }

        info!("P2P peer: challenge send");

        let resp = match self.read_and_unwrap_msg().await {
            Ok(msg) => match msg {
                message::envelope::Msg::VerificationResponse(resp) => resp,
                _ => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
            },
            Err(_) => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
        };

        // TODO: validate response

        let resp_status_msg = envelope::Msg::VerificationValidationResponse(
            message::VerificationValidationResponse {
                status: message::VerificationResponseStatus::Ok as i32,
            },
        );
        // send challenge status result
        match self.send_and_wrap_msg(resp_status_msg).await {
            Ok(()) => (),
            Err(err) => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
        }

        Ok(())
    }

    pub async fn send_and_wrap_msg(&self, msg: envelope::Msg) -> Result<(), SendError> {
        let envelope = Envelope { msg: Some(msg) };
        self.send_msg(envelope).await
    }

    pub async fn read_and_unwrap_msg(&self) -> Result<envelope::Msg, RecvError> {
        match self.read_msg().await {
            Ok(envelope) => match envelope.msg {
                Some(msg) => Ok(msg),
                None => Err(RecvError::EmptyEnvelope),
            },
            Err(e) => Err(e),
        }
    }

    pub async fn send_msg(&self, envelope: Envelope) -> Result<(), SendError> {
        let mut tx = self.tx.lock().await;
        let buf = envelope.encode_to_vec();
        match tx.write(&buf).await {
            Ok(0) => Err(SendError::InvalidLength(0)),
            Ok(n) => Ok(()),
            Err(e) => Err(SendError::Write(e)),
        }
    }

    pub async fn read_msg(&self) -> Result<Envelope, RecvError> {
        let mut s = self.rx.lock().await;
        // FIXME: @wlad use shared buffer!!!
        let mut buf = BytesMut::with_capacity(8 * 1024);

        match s.read_buf(&mut buf).await {
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

    pub fn remote_addr(&self) -> SocketAddr {
        return self.remote_addr;
    }

    pub fn get_peer_addr(&self) -> message::Addr {
        let addr = self.remote_addr;

        let mut msg_addr = message::Addr {
            ip: None,
            port: addr.port() as u32,
        };

        match addr.ip() {
            IpAddr::V4(ip) => msg_addr.ip = Some(message::addr::Ip::V4(ip.into())),
            IpAddr::V6(ip) => msg_addr.ip = Some(message::addr::Ip::V6(ip.octets().to_vec())),
        }

        msg_addr
    }

    pub fn get_peer_description(&self) -> message::Peer {
        message::Peer {
            identity: self.identity.to_vec(),
            address: Some(self.get_peer_addr()),
        }
    }

    pub async fn run(
        &self,
        mut tx: mpsc::Receiver<envelope::Msg>,
        rx: mpsc::Sender<envelope::Msg>,
    ) -> Result<(), PeerError> {
        loop {
            tokio::select! {
                rx_msg = self.read_and_unwrap_msg() => {
                    debug!("peer: recv {:?}", rx_msg);
                    match rx_msg {
                        // TODO: remove optional wrapping aroung msg in envelope
                        Ok(msg) => rx.send(msg).await.unwrap(),
                        Err(err) => {
                            return Err(PeerError::Read(err))
                        },
                    }
                }

                tx_msg = tx.recv() => {
                    if let None = tx_msg {
                        // debug!("got empty message");
                        continue;
                    }

                    match self.send_and_wrap_msg(tx_msg.unwrap()).await {
                        Ok(_) => (),
                        Err(err) => {
                            return Err(PeerError::Write(err))
                        }
                    }
                }
            }
        }
    }
}
