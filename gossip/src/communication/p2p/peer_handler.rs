use log::{debug, error, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendError;

use super::message;
use super::peer::{Peer, PeerConnectionStatus, PeerError};
use super::server::{PeerConnectionMessage, PeerSender, ServerPeerMessage};

/// PeerHandler is a wrapper object handling message forwaring,
/// close handling and state management for indivual peers
pub struct PeerHandler {
    /// Channel for sending messages to the peer managed by PeerHandler
    pub msg_tx: PeerSender,

    /// Underlying peer instance
    peer: Arc<Peer>,
}

impl PeerHandler {
    /// Create a new PeerHandler instance given a peer and connection channels
    pub fn new(
        peer: Peer,
        peer_p2p_tx: mpsc::Sender<ServerPeerMessage>,
        peer_connection_tx: mpsc::Sender<PeerConnectionMessage>,
    ) -> PeerHandler {
        let (rx_write, mut rx_read) = mpsc::channel(512);
        let (tx_write, tx_read) = mpsc::channel(512);

        let (close_sender, mut close_receiver) = mpsc::channel::<Option<PeerError>>(1);

        let p = Arc::new(peer);
        let peer_run = p.clone();

        // run peer recv and send methods
        // on transmission error write into close channel
        tokio::spawn(async move {
            match peer_run.run(tx_read, rx_write).await {
                Ok(_) => close_sender.send(None).await,
                Err(err) => close_sender.send(Some(err)).await,
            }
        });

        let peer_addr = p.remote_addr();
        let peer_identity = p.identity.clone();
        let peer_description = p.get_peer_description();
        tokio::spawn(async move {
            loop {
                tokio::select! {

                    // message comming in from peer
                    // receive message from peer and expand it by peer information
                    // so that in can easily be processes in the server level
                    peer_rx_msg = rx_read.recv() => {
                        match peer_rx_msg {
                            Some(msg) => match peer_p2p_tx.send((msg, peer_identity, peer_addr)).await {
                                Ok(_) => (),
                                Err(e) => {
                                    error!("failed to receive message {}", e);
                                    todo!();
                                }
                            }
                            None => warn!("got empty message from peer channel")
                        }
                    }

                    close_err = close_receiver.recv() => {
                        match close_err {
                            Some(Some(err)) => {
                                error!("peer_handler: closed with error {}", err);
                                match peer_connection_tx.send((
                                    peer_identity,
                                    PeerConnectionStatus::Closed("closed with error".to_string()), peer_description.clone()
                                )).await {
                                    Ok(_) => (),
                                    Err(err) => {
                                        panic!("failed to send peer connection update {}", err)
                                    }
                                }
                            }
                            _ => {debug!("peer_handler: closed without errors"); todo!();}
                        }
                    }
                }
            }
        });

        PeerHandler {
            msg_tx: tx_write,
            peer: p,
        }
    }

    /// Send message to individual peer
    async fn send_msg(
        &self,
        msg: message::envelope::Msg,
    ) -> Result<(), SendError<message::envelope::Msg>> {
        self.msg_tx.send(msg).await
    }

    /// Get a purely info representation of the underlying peer in p2p::messages::Peer format
    pub fn get_description(&self) -> message::Peer {
        self.peer.get_peer_description()
    }

    /// Perform graceful shutdown on peer
    pub async fn shutdown(&mut self) {
        // NOTE: add when needed
    }
}
