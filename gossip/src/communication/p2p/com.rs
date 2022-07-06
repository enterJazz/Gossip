use log::{debug, error, info};
use std::{error::Error, io, net::SocketAddr, sync::Arc};
use tokio::{net::UdpSocket, sync::mpsc};

pub struct Communicator {
    socket: Arc<UdpSocket>,
    send_chan: mpsc::Sender<(Vec<u8>, SocketAddr)>,
}

impl Communicator {
    // FIXME: currently there is no way to close the connection
    pub async fn new(
        addr: String,
        channel_capacity: usize,
    ) -> Result<(Self, mpsc::Receiver<(Vec<u8>, SocketAddr)>), Box<dyn Error>> {
        let parsed_addr = addr.parse::<SocketAddr>();
        if let Some(err) = parsed_addr.err() {
            // TODO: is boxing like this okay?
            return Err(Box::new(err));
        }

        let socket = Arc::new(UdpSocket::bind(addr).await?);

        info!("listening on: {}", socket.local_addr()?);

        // create receiving and sending channels
        let (tx, mut rx) = mpsc::channel::<(Vec<u8>, SocketAddr)>(channel_capacity);
        let (recv_tx, recv_rx) = mpsc::channel::<(Vec<u8>, SocketAddr)>(channel_capacity);

        debug!("starting P2PServer");

        // clone socket to pass to tokio thread
        let tx_socket = socket.clone();
        tokio::spawn(async move {
            // read bytes + addr from send/tx channel
            loop {
                while let Some((bytes, addr)) = rx.recv().await {
                    let len = tx_socket.send_to(&bytes, &addr).await.unwrap();
                    debug!("{:?} bytes sent", len);
                }
            }
        });

        // receiver task
        let rx_socket = socket.clone();
        tokio::spawn(async move {
            // FIXME: increase buffer size if needed
            let mut buf = [0; 1024];
            loop {
                let (num_bytes, src_addr) = rx_socket.recv_from(&mut buf).await.unwrap();
                // pass received data to the recv channel for processing
                if let Err(msg) = recv_tx.send((buf[..num_bytes].to_vec(), src_addr)).await {
                    error!("failed to process incomming data: {}", msg);
                    continue;
                }
            }
        });

        Ok((
            Communicator {
                socket,
                send_chan: tx,
            },
            recv_rx,
        ))
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.as_ref().local_addr()
    }

    pub async fn send_to(
        &self,
        addr: SocketAddr,
        data: Vec<u8>,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<(std::vec::Vec<u8>, std::net::SocketAddr)>>
    {
        let tx_chan_tx = &self.send_chan;

        match tx_chan_tx.send((data, addr)).await {
            Ok(_) => Ok(()),
            Err(e) => return Err(e),
        }
    }
}
