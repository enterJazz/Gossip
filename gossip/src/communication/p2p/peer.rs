use crate::communication::p2p::message::{envelope, Envelope, VerificationRequest};
use bytes::BytesMut;
use log::{debug, error, info, log_enabled, warn, Level};
use prost::Message;
use std::error::Error;
use std::str;
use std::sync::Arc;
use std::{io, net::SocketAddr};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::{tcp, TcpStream};
use tokio::sync::{mpsc, Mutex};

/// Shorthand for the receive half of the message channel.
type RxStreamHalf = Arc<Mutex<tcp::OwnedReadHalf>>;
type TxStreamHalf = Arc<Mutex<tcp::OwnedWriteHalf>>;

#[derive(Error, Debug)]
pub enum PeerError {
    #[error("peer error: connection failed {0}")]
    Connection(#[from] io::Error),
    #[error("peer error: challenge failed")]
    Challenge(),
    #[error("peer error: could not read message")]
    Read(#[from] RecvError),
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

pub struct Peer {
    pub rx: RxStreamHalf,
    pub tx: TxStreamHalf,
    pub status: PeerConnectionStatus,
    addr: SocketAddr,

    // buffer for reading messages.
    read_buffer: BytesMut,
}

impl Peer {
    pub fn new(stream: TcpStream) -> Self {
        let addr = stream.peer_addr().unwrap();

        let (mut rx, mut tx) = TcpStream::into_split(stream);

        return Peer {
            rx: Arc::new(Mutex::new(rx)),
            tx: Arc::new(Mutex::new(tx)),
            addr,
            // TODO: handle
            status: PeerConnectionStatus::Unknown,
            read_buffer: BytesMut::with_capacity(8 * 1024),
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

        debug!("p2p/peer/connect: reading message from {}", self.get_addr());

        // TODO: handle errors
        let env = self.read_msg().await?;

        debug!("p2p/peer/connect: finished reading message");

        let msg = match env.msg {
            Some(envelope::Msg::VerificationRequest(req)) => req.challenge,
            _ => return Err(PeerError::Challenge()),
        };

        info!(
            "P2P peer: got challenge value: {}",
            str::from_utf8(&msg.to_vec()).unwrap()
        );

        Ok(())
    }

    // initiate the connection process by publishing challenge to the other party defined by the underlying
    // TCP Stream
    pub async fn challenge(&mut self) -> PeerResult<()> {
        let challenge_req = Envelope {
            msg: Some(envelope::Msg::VerificationRequest(VerificationRequest {
                challenge: "test".as_bytes().to_vec(),
                pub_key: "bla".as_bytes().to_vec(),
            })),
        };

        if let Err(err) = self.send_msg(challenge_req).await {
            error!("P2P peer: failed to send challenge {}", err.to_string());
            return Err(PeerError::Challenge());
        }

        info!("P2P peer: challenge send");

        // TODO: handle responses
        // let rx = self.rx.clone();
        // // wait for response  // TODO: add delay
        // tokio::spawn(async move { return Peer::read_msg(rx).await }).await;

        Ok(())
    }

    pub async fn send_msg(&self, msg: Envelope) -> Result<(), SendError> {
        let mut tx = self.tx.lock().await;
        let buf = msg.encode_to_vec();
        match tx.write(&buf).await {
            Ok(0) => Err(SendError::InvalidLength(0)),
            Ok(n) => Ok(()),
            Err(e) => Err(SendError::Write(e)),
        }
    }

    pub async fn read_msg(&self) -> Result<Envelope, RecvError> {
        let mut s = self.rx.lock().await;
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

    pub fn get_addr(&self) -> SocketAddr {
        return self.addr;
    }

    pub fn is_active(&self) -> bool {
        self.status == PeerConnectionStatus::Connected
            || self.status == PeerConnectionStatus::Connecting
    }

    pub async fn run(
        &self,
        mut tx: mpsc::Receiver<envelope::Msg>,
        rx: mpsc::Sender<envelope::Msg>,
    ) -> bool {
        loop {
            tokio::select! {
                rx_envelope = self.read_msg() => {
                    match rx_envelope {
                        // TODO: remove optional wrapping aroung msg in envelope
                        Ok(envelope) => rx.send(envelope.msg.unwrap()).await.unwrap(),
                        Err(err) => {
                            warn!("failed to receive msg {}", err);
                            continue;
                        },
                    }
                }

                tx_msg = tx.recv() => {
                    if let None = tx_msg {
                        // debug!("got empty message");
                        continue;
                    }

                    debug!("sending data");
                    let env = Envelope{
                        msg: tx_msg,
                    };
                    match self.send_msg(env).await {
                        Ok(()) => debug!("msg send"),
                        Err(err) => error!("failed to send {}", err)
                    }
                }
            }
        }
    }
}
