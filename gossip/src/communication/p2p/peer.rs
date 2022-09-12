use crate::communication::p2p::{
    message::{envelope, Envelope, VerificationRequest, VerificationResponse},
    pow,
};
use bytes::BytesMut;
use log::{debug, error, info};
use prost::Message;
use std::time::Duration;
use std::{
    convert::TryInto,
    fmt::Display,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
};
use std::{fmt, sync::Arc};
use std::{io, net::SocketAddr};
use thiserror::Error;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{tcp, TcpStream, ToSocketAddrs},
    sync::{mpsc, Mutex},
    time::timeout,
};

use super::{
    message::{self},
    pow::CHALLENGE_LEN,
};

const POW_DIFFICULTY: u32 = 1;
const POW_TIMEOUT: u64 = 20;

/// Peer identity is a Blake3 hahs of a peers public key
pub type PeerIdentity = [u8; 32];

/// Convert a peer identity into a human readable base64 representation
pub fn peer_into_str(peer: PeerIdentity) -> String {
    base64::encode(peer)
}

/// Parse PeerIdentity from a u8 slice
pub fn parse_identity(buf: &[u8]) -> Option<PeerIdentity> {
    match buf.try_into() {
        Ok(id) => Some(id),
        Err(_) => None,
    }
}

/// Convert a socket address into a p2p::message::Addr
/// respecting IPv4 and IPv6 types
pub fn into_addr(addr: SocketAddr) -> message::Addr {
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

/// Convert a p2p::message::Addr into a SocketAddr instance
/// with IPv4 and IPv6 handling
pub fn from_addr(addr: &message::Addr) -> Option<SocketAddr> {
    let port = addr.port.try_into();
    if let Err(_) = port {
        return None;
    }

    match addr.ip.clone() {
        // IPv4 handling
        Some(message::addr::Ip::V4(ip)) => Some(std::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::from(ip),
            port.unwrap(),
        ))),
        // IPv6 handling
        Some(message::addr::Ip::V6(ip)) => {
            let ip_buf: [u8; 16] = match ip.try_into() {
                Ok(b) => b,
                Err(_) => return None,
            };

            Some(std::net::SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(ip_buf),
                addr.port.try_into().unwrap(),
                0,
                0,
            )))
        }
        _ => return None,
    }
}

/// Shorthand for the receive half of the message channel.
type RxStreamHalf = Arc<Mutex<tcp::OwnedReadHalf>>;
type TxStreamHalf = Arc<Mutex<tcp::OwnedWriteHalf>>;

#[derive(Error, Debug)]
pub enum ChallengeError {
    #[error("challenge exechange error")]
    ExchangeError,
    #[error("remote solution check failed")]
    CheckFailed,
    #[error("challenge timeout exceeded")]
    Timeout,
}

#[derive(Error, Debug)]
pub enum PeerError {
    #[error("peer error: peer connection already exists")]
    Duplicate,
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
    #[error("protoc decode error {0}")]
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

/// Enum representing the current state of a P2P connection
#[derive(PartialEq, Clone)]
pub enum PeerConnectionStatus {
    // indicates peer was newly created and has not begun pairing process
    Unset,
    // peer is in the connection process
    Connecting,
    // peer was connected to successfully
    Connected,
    // peer connection was aborted
    Closed(String),
}

impl Display for PeerConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PeerConnectionStatus::Unset => write!(f, "unknown"),
            PeerConnectionStatus::Connecting => write!(f, "connecting"),
            PeerConnectionStatus::Connected => write!(f, "connected"),
            PeerConnectionStatus::Closed(reason) => write!(f, "closed({})", reason),
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

    /// Hash of public key and ip address and port of a peer (only set after completed handshake)
    pub identity: PeerIdentity,
    /// Public key of remote peer (only set after completed handshake)
    pub pub_key: bytes::Bytes,

    /// Connection Port used by the Peer server for incoming connections
    pub connection_port: u16,

    /// Remote peer address
    remote_addr: SocketAddr,
}

/// Compute Blake3 based identity given a DER key as a byte slice
pub fn compute_identity(pub_key: &[u8]) -> PeerIdentity {
    *blake3::hash(pub_key).as_bytes()
}

/// Peer represents a connection to a peer on the network
/// peers are only deemed connected after a PoW challenge is completed by the connecting party
/// TODO: add reverse connection
impl Peer {
    /// Returns a new Peer instance given a TCP connection
    /// No handshake is performed at this step
    ///
    /// # Arguments
    ///
    /// * `stream` - TCP stream inteded for all communications to peer
    pub fn new(stream: TcpStream) -> Self {
        Peer::new_with_identity(stream, [0; 32], bytes::Bytes::new())
    }

    /// Creates a Peer with an underlying TCP-Steam connection
    /// No handshake is performed at this step
    ///
    /// # Arguments
    ///
    /// * `addr` - any type that confirms to the ToSocketAddrs trait
    pub async fn new_from_addr<T: ToSocketAddrs>(addr: T) -> PeerResult<Self> {
        match TcpStream::connect(addr).await {
            Ok(socket) => {
                let port = match socket.peer_addr() {
                    Ok(addr) => addr.port(),
                    Err(err) => {
                        unreachable!("connection created without valid peer address {}", err)
                    }
                };
                let mut peer = Peer::new(socket);
                peer.connection_port = port;

                Ok(peer)
            }
            Err(e) => Err(PeerError::Connection(e)),
        }
    }

    /// Creates a new Peer instance given a TCP connection and additional
    /// indentity information
    ///
    /// # Arguments
    ///
    /// * `stream` - any type that confirms to the ToSocketAddrs trait
    /// * `ìdentity` - Blake3 Hash of the DER encoded public key
    /// * `pub_key` - DER encoded public key
    pub fn new_with_identity(
        stream: TcpStream,
        identity: PeerIdentity,
        pub_key: bytes::Bytes,
    ) -> Self {
        let remote_addr = stream.peer_addr().unwrap();

        let (rx, tx) = TcpStream::into_split(stream);
        return Peer {
            rx: Arc::new(Mutex::new(rx)),
            tx: Arc::new(Mutex::new(tx)),
            remote_addr,
            connection_port: 0,
            status: PeerConnectionStatus::Unset,
            identity: identity,
            pub_key: pub_key,
        };
    }

    /// Peer internal shutdown handling
    /// TCP-Connection shutdown is handled externally
    pub fn shutdown(&mut self) -> PeerResult<()> {
        // NOTE: intended for additional cleanup
        // will be called by peer handler in case of external closure
        // TCP connection is cleaned when all references are removed e.g.
        self.status = PeerConnectionStatus::Closed("graceful shutdown".to_string());
        Ok(())
    }

    // initiate a connection process to the remote defined by the underlying TCP Stream
    pub async fn connect(
        &mut self,
        host_identity: PeerIdentity,
        host_connection_port: u16,
        host_pub_key: bytes::Bytes,
    ) -> PeerResult<PeerIdentity> {
        self.status = PeerConnectionStatus::Connecting;

        let peer_identity = match self
            .solve_pow(host_identity, host_connection_port, &host_pub_key)
            .await
        {
            Ok(id) => id,
            Err(err) => return Err(err),
        };

        debug!("pow: completed 1 of 2");

        match self.request_pow(host_identity, host_pub_key).await {
            Ok(_) => {
                debug!("pow: completed 2 of 2");
                Ok(peer_identity)
            }
            Err(err) => Err(err),
        }
    }

    // initiate the connection process by publishing challenge to the other party defined by the underlying
    // TCP Stream
    pub async fn challenge(
        &mut self,
        host_identity: PeerIdentity,
        host_pub_key: bytes::Bytes,
    ) -> PeerResult<PeerIdentity> {
        match self.request_pow(host_identity, host_pub_key.clone()).await {
            Ok(Some(resp)) => {
                // update values on peer
                self.pub_key = bytes::Bytes::from(resp.pub_key.clone());
                self.identity = compute_identity(&resp.pub_key);
                self.connection_port = resp.server_port as u16;
            }
            Ok(None) => {
                unreachable!()
            }
            Err(err) => return Err(err),
        };
        debug!("pow: completed 1 of 2");

        match self
            .solve_pow(host_identity, self.connection_port, &host_pub_key)
            .await
        {
            Ok(peer_identity) => {
                debug!("pow: completed 2 of 2");
                Ok(peer_identity)
            }
            Err(err) => Err(err),
        }
    }

    /// Read incoming Pow Challenge request and solve it
    /// Returns a peer identity if solution was found in time
    async fn solve_pow(
        &mut self,
        host_identity: PeerIdentity,
        host_connection_port: u16,
        host_pub_key: &bytes::Bytes,
    ) -> PeerResult<PeerIdentity> {
        let req = match self.read_and_unwrap_msg().await? {
            envelope::Msg::VerificationRequest(req) => req,
            _ => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
        };

        // compute remote identity
        let peer_identity: PeerIdentity = compute_identity(&req.pub_key);
        debug!(
            "received challenge, difficulty={} challenge={:?} peer={}",
            req.difficulty,
            req.challenge,
            peer_into_str(peer_identity)
        );

        // update peer values based on response
        self.pub_key = bytes::Bytes::from(req.pub_key);
        self.identity = peer_identity;

        // ensure challenge has the correct length as expected by PoW module
        let challenge: [u8; pow::CHALLENGE_LEN] = match req.challenge.as_slice().try_into() {
            Ok(c) => c,
            Err(err) => {
                self.status = PeerConnectionStatus::Closed(err.to_string());
                return Err(PeerError::Challenge(ChallengeError::CheckFailed));
            }
        };

        debug!(
            "searching for solution src_id={} target_id={} challenge={:?}",
            peer_into_str(peer_identity),
            peer_into_str(host_identity),
            challenge
        );

        // compute nonce based on self and remote identities
        let nonce = pow::generate_proof_of_work(
            peer_identity,
            host_identity,
            challenge,
            req.difficulty as u8,
        )
        .await;

        debug!("nonce found {:?}", nonce,);

        let resp = envelope::Msg::VerificationResponse(message::VerificationResponse {
            challenge: req.challenge,
            server_port: host_connection_port as u32,
            remote_identity: peer_identity.to_vec(),
            nonce: nonce.to_vec(),
            pub_key: host_pub_key.to_vec(),
        });

        // send challenge result
        match self.send_and_wrap_msg(resp).await {
            Ok(()) => (),
            Err(err) => {
                self.status = PeerConnectionStatus::Closed(err.to_string());
                return Err(PeerError::Challenge(ChallengeError::ExchangeError));
            }
        }

        // wait for connection status
        let challenge_status = match self.read_and_unwrap_msg().await? {
            envelope::Msg::VerificationValidationResponse(resp) => resp.status,
            _ => {
                return {
                    self.status = PeerConnectionStatus::Closed(
                        "received invalid verification response".to_string(),
                    );
                    Err(PeerError::Challenge(ChallengeError::ExchangeError))
                }
            }
        };

        debug!(
            "solution response-status received status={:?}",
            challenge_status
        );

        // verify remote response
        match message::VerificationResponseStatus::from_i32(challenge_status) {
            Some(message::VerificationResponseStatus::Ok) => (),
            Some(message::VerificationResponseStatus::Timeout) => {
                self.status =
                    PeerConnectionStatus::Closed("challenge was not solved in time".to_string());
                return Err(PeerError::Challenge(ChallengeError::Timeout));
            }
            _ => {
                self.status =
                    PeerConnectionStatus::Closed("received invalid challenge response".to_string());
                return Err(PeerError::Challenge(ChallengeError::CheckFailed));
            }
        };

        Ok(peer_identity)
    }

    async fn request_pow(
        &mut self,
        host_identity: PeerIdentity,
        host_pub_key: bytes::Bytes,
    ) -> Result<Option<VerificationResponse>, PeerError> {
        let (challenge, challenge_request) =
            Self::gen_verification_challenge_request(host_pub_key).await;

        if let Err(err) = self
            .send_and_wrap_msg(envelope::Msg::VerificationRequest(challenge_request))
            .await
        {
            error!("failed to send challenge {}", err.to_string());
            return Err(PeerError::Challenge(ChallengeError::ExchangeError));
        }

        let challenge_timeout = Duration::from_secs(POW_TIMEOUT);
        // wait for response or timeout before returning result
        let mut response: Option<VerificationResponse> = None;
        let resp_future = self.read_and_unwrap_msg();

        match timeout(challenge_timeout, resp_future).await {
            // handle response incoming before timeout
            Ok(timeout_resp) => match timeout_resp {
                Ok(message::envelope::Msg::VerificationResponse(resp)) => response = Some(resp),
                // invalid response message
                Ok(_) => {
                    debug!("received unexpected result");
                    return Err(PeerError::Challenge(ChallengeError::ExchangeError));
                }
                // error during response read
                Err(err) => {
                    debug!("could not receive response message {}", err);
                    return Err(PeerError::Challenge(ChallengeError::ExchangeError));
                }
            },
            Err(err) => error!("challenge timeout error {}", err),
        };

        let mut status: message::VerificationResponseStatus =
            message::VerificationResponseStatus::Invalid;

        // only if we got a response in time will this case happen
        if let Some(resp) = &response {
            let remote_identity: PeerIdentity = compute_identity(&resp.pub_key);

            debug!(
                "received challenge solution with src_id={} target_id={} challenge={:?} nonce={:?}",
                peer_into_str(host_identity),
                peer_into_str(remote_identity),
                challenge,
                resp.nonce,
            );

            // make sure nonce has correct length
            let nonce: [u8; pow::NONCE_LEN] = match resp.nonce.as_slice().try_into() {
                Ok(c) => c,
                Err(err) => return Err(PeerError::Challenge(ChallengeError::CheckFailed)),
            };

            let valid = pow::validate_nonce(
                host_identity,
                remote_identity,
                challenge,
                nonce,
                POW_DIFFICULTY as u8,
            )
            .await;

            if valid {
                status = message::VerificationResponseStatus::Ok;
            }
        } else {
            debug!("challenge response timeout exceeded");
            status = message::VerificationResponseStatus::Timeout;
        }

        let resp_status_msg = envelope::Msg::VerificationValidationResponse(
            message::VerificationValidationResponse {
                status: status as i32,
            },
        );
        // send challenge status result
        match self.send_and_wrap_msg(resp_status_msg).await {
            Ok(()) => Ok(response),
            Err(_) => return Err(PeerError::Challenge(ChallengeError::ExchangeError)),
        }
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
        // IDEA: use shared buffer here instead of recreating one each read cycle
        let mut buf = BytesMut::with_capacity(8 * 1024);

        match s.read_buf(&mut buf).await {
            Ok(0) => Err(RecvError::InvalidLength(0)),
            Ok(n) => match Envelope::decode(&buf[..n]) {
                Ok(msg) => Ok(msg),
                Err(err) => Err(RecvError::Decode(err)),
            },
            Err(e) => {
                return Err(e.into());
            }
        }
    }

    pub fn remote_addr(&self) -> SocketAddr {
        return self.remote_addr;
    }

    pub fn get_peer_addr(&self) -> message::Addr {
        into_addr(self.remote_addr)
    }

    pub fn get_peer_p2p_addr(&self) -> message::Addr {
        let mut addr = into_addr(self.remote_addr);
        addr.port = self.connection_port as u32;
        addr
    }

    pub fn get_peer_description(&self) -> message::Peer {
        message::Peer {
            identity: self.identity.to_vec(),
            address: Some(self.get_peer_p2p_addr()),
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
                    match rx_msg {
                        Ok(msg) => rx.send(msg).await.unwrap(),
                        Err(err) => {
                            return Err(PeerError::Read(err))
                        },
                    }
                }

                tx_msg = tx.recv() => {
                    if let None = tx_msg {
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

    async fn gen_verification_challenge_request(
        host_pub_key: bytes::Bytes,
    ) -> ([u8; CHALLENGE_LEN], VerificationRequest) {
        let challenge = pow::generate_challenge().await;

        (
            challenge,
            VerificationRequest {
                challenge: challenge.to_vec(),
                difficulty: POW_DIFFICULTY,
                pub_key: host_pub_key.to_vec(),
            },
        )
    }
}
