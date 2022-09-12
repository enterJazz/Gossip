//! API server implementation
//!
//! Provides an async `run` function that listens for inbound connections,
//! spawning a task per connection.

use std::collections::HashMap;
use std::net::SocketAddr;

use crate::communication::api::connection::Connection;
use crate::communication::api::message::ApiMessage;
use log::{debug, error, info, warn};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use thiserror::Error;
use tokio::sync::mpsc::{Receiver, Sender};

/// time to wait in secs before polling server for availability
const RECONNECT_WAIT: u64 = 1;

#[derive(Error, Debug)]
pub enum Error {
    #[error("no subscribers for topic {topic}")]
    NoSubscribers { topic: u16 },
}

/// Server listener state. Created in the `run` call. It includes a `run` method
/// which performs the TCP listening and initialization of per-connection state.
#[derive(Debug)]
struct Listener {
    /// Shared database handle.
    ///
    /// Contains the key / value store as well as the broadcast channels for
    /// pub/sub.
    db_holder: Arc<Mutex<HashMap<u16, Vec<mpsc::Sender<ApiMessage>>>>>,

    /// TCP listener supplied by the `run` caller.
    listener: TcpListener,

    /// Receives payloads from handlers
    handler_api_rx: mpsc::Receiver<ApiMessage>,

    /// Handler sends results to API through this tx
    /// Copied per send.
    handler_api_tx: mpsc::Sender<ApiMessage>,

    /// API to RPS Connection Handler Transmitter.
    /// Connected to by API server upon initiation.
    api_rps_tx: mpsc::Sender<ApiMessage>,

    /// RPS to API Server Receiver.
    rps_api_rx: mpsc::Receiver<ApiMessage>,
}

/// Per-connection handler. Reads requests from `connection` and applies the
/// commands to `db`.
#[derive(Debug)]
struct Handler {
    /// Shared database handle.
    ///
    /// When a command is received from `connection`, it is applied with `db`.
    /// The implementation of the command is in the `cmd` module. Each command
    /// will need to interact with `db` in order to complete the work.
    db: Arc<Mutex<HashMap<u16, Vec<mpsc::Sender<ApiMessage>>>>>,

    /// The TCP connection decorated with the redis protocol encoder / decoder
    /// implemented using a buffered `TcpStream`.
    ///
    /// When `Listener` receives an inbound connection, the `TcpStream` is
    /// passed to `Connection::new`, which initializes the associated buffers.
    /// `Connection` allows the handler to operate at the "message" level and keep
    /// the byte level protocol parsing details encapsulated in `Connection`.
    connection: Connection,

    /// The original connection address; used for reconnecting
    conn_addr: SocketAddr,

    /// If set, the handler attempts to reconnect after a closed connection
    attempt_reconnect: bool,

    /// Sender for API interaction.
    /// API fetches txs per topic to send notifications.
    api_handler_tx: mpsc::Sender<ApiMessage>,

    /// Receiver for API interaction.
    /// Handler pushes tx to topic of the HashMap db.
    api_handler_rx: mpsc::Receiver<ApiMessage>,

    /// Contains subscribed topics of connection.
    /// Only used to ensure that subscriber cannot perform multiple subscriptions to same topic.
    subscribed_topics: Vec<u16>,

    /// Handler sends results to API through this tx
    handler_api_tx: mpsc::Sender<ApiMessage>,
}

/// Run the server.
pub async fn run(
    listener: TcpListener,
    pub_api_rx: mpsc::Receiver<ApiMessage>,
    api_pub_tx: mpsc::Sender<Result<ApiMessage, Error>>,
    broadcaster_api_rx: mpsc::Receiver<ApiMessage>,
    api_broadcaster_tx: mpsc::Sender<Result<ApiMessage, Error>>,
    rps_address: SocketAddr,
) {
    let db = Arc::new(Mutex::new(HashMap::new()));

    // Initialize the listener state
    let (rps_api_tx, rps_api_rx) = mpsc::channel(512);
    let (api_rps_tx, api_rps_rx) = mpsc::channel(512);

    // connect to the RPS
    // block until RPS is found
    let rps_conn: Connection;
    loop {
        rps_conn = match TcpStream::connect(rps_address).await {
            Ok(sock) => {
                Connection::new(sock)
            },
            Err(e) => {
                error!("API SERVER: failed to connect to RPS server: {} waiting and trying again...", e.to_string());
                std::thread::sleep(std::time::Duration::from_secs(RECONNECT_WAIT));
                continue
            },
        };
        break
    }
    let mut rps_handler = Handler {
        db: db.clone(),
        connection: rps_conn,
        api_handler_tx: api_rps_tx.clone(),
        api_handler_rx: api_rps_rx,
        subscribed_topics: vec![],
        handler_api_tx: rps_api_tx,
        conn_addr: rps_address.clone(),
        attempt_reconnect: true,
    };

    tokio::spawn(async move {
        rps_handler.run().await;
    });

    let (handler_api_tx, handler_api_rx) = mpsc::channel(512);
    let mut server = Listener {
        listener,
        handler_api_rx,
        db_holder: db.clone(),
        handler_api_tx,
        api_rps_tx: api_rps_tx.clone(),
        rps_api_rx,
    };

    // transmitter must be given to publisher
    // idea: publisher passes receiver upon run
    server
        .run(
            pub_api_rx,
            api_pub_tx,
            broadcaster_api_rx,
            api_broadcaster_tx,
        )
        .await;
}

impl Listener {
    /// Run the server
    ///
    /// Listen for inbound connections. For each inbound connection, spawn a
    /// task to process that connection.
    async fn run(
        &mut self,
        mut pub_api_rx: mpsc::Receiver<ApiMessage>,
        mut api_pub_tx: mpsc::Sender<Result<ApiMessage, Error>>,
        mut broadcaster_api_rx: mpsc::Receiver<ApiMessage>,
        mut api_broadcaster_tx: mpsc::Sender<Result<ApiMessage, Error>>,
    ) {
        info!("API SERVER: accepting inbound connections");

        loop {
            tokio::select! {
                res = self.listener.accept() => {
                    let (socket, addr) = match res {
                        Ok(res) => res,
                        Err(e) => {
                            error!("API SERVER: failed to connect with listener: {:?}", e);
                            continue;
                        }
                    };
                    info!("API SERVER: received connection from {:?}", addr.clone());
                    let connection = Connection::new(socket);

                    // Create the necessary per-connection handler state.
                    let (tx, rx) = mpsc::channel(512);
                    let mut handler = Handler {
                        // Get a handle to the shared database.
                        db: self.db_holder.clone(),

                        // Initialize the connection state
                        connection,
                        api_handler_tx: tx,
                        api_handler_rx: rx,
                        subscribed_topics: vec![],
                        handler_api_tx: self.handler_api_tx.clone(),
                        conn_addr: addr,
                        attempt_reconnect: false,
                    };

                    // Spawn a new task to process the connections
                    tokio::spawn(async move {
                        handler.run().await;
                    });
                }

                message = pub_api_rx.recv() => {
                    info!("API SERVER: received message from publisher");
                    let message = match message {
                        Some(message) => message,
                        None => {
                            error!("API Server: received NONE message from publisher! Ignoring...");
                            continue;
                        }
                    };
                    match &message {
                        ApiMessage::Notification(m_sub) => {
                            let mut tx_vec_cp;
                            let mut api_pub_tx_send = None;
                            {
                                let db = self.db_holder.lock().unwrap();
                                let tx_vec = db.get(&m_sub.data_type);
                                tx_vec_cp = match tx_vec {
                                    Some(tx_vec) => tx_vec.clone(),
                                    None => {
                                        warn!("no subscribers found for topic {}", &m_sub.data_type);
                                        api_pub_tx_send = Some(api_pub_tx.send(Err(Error::NoSubscribers {topic: m_sub.data_type})));
                                        vec![]
                                    }
                                };
                            }
                            // had to move this here due to issues with awaiting a send during DB lock
                            match api_pub_tx_send {
                                Some(send) => {
                                    send.await.unwrap();
                                    return
                                },
                                None => {}
                            };

                            for i in 0..tx_vec_cp.len() {
                                let mut tx = tx_vec_cp.get_mut(i).unwrap();
                                match tx.send(message.clone()).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        warn!("sending to connection tx {:?} failed; removing tx", tx);
                                        let mut db = self.db_holder.lock().unwrap();
                                        db.get_mut(&m_sub.data_type).unwrap().remove(i);
                                    }
                                }
                            }

                            {
                                let db = self.db_holder.lock().unwrap();
                                let mut tx_vec = db.get(&m_sub.data_type).unwrap();
                                tx_vec_cp = tx_vec.clone();
                            }


                            // check that we did not remove all txs
                            if tx_vec_cp.is_empty() {
                                warn!("no subscribers found for topic {}", &m_sub.data_type);
                                api_pub_tx.send(Err(Error::NoSubscribers {topic: m_sub.data_type})).await.unwrap();
                                return;
                            }


                        }
                        _ => panic!("message {:?} may not be sent by server", message),
                    }
                }

                message = self.handler_api_rx.recv() => {
                    info!("API SERVER: received message from handler");
                     let message = match message {
                        Some(message) => message,
                        None => {
                            error!("API Server: received NONE from handler");
                            continue;
                        }
                    };
                    match &message {
                        // validation messages: return to publisher
                        ApiMessage::Validation(_) => {
                            api_pub_tx.send(Ok(message)).await.unwrap();
                        },
                        // announce messages: send to broadcaster
                        ApiMessage::Announce(_) => {
                            api_broadcaster_tx.send(Ok(message)).await.unwrap();
                        },

                        _ => panic!("message {:?} not meant to be handled by API", message),
                    }
                }

                // RPS communication
                // Broadcaster -> API -> RPS Handler
                message = broadcaster_api_rx.recv() => {
                    info!("API SERVER: received message from broadcaster");
                    let message = match message {
                        Some(message) => message,
                        None => {
                            error!("API Server: received NONE from broadcaster");
                            continue;
                        }
                    };
                    match &message {
                        ApiMessage::RPSQuery => self.api_rps_tx.send(message).await.unwrap(),
                        _ => panic!("message {:?} not meant to be handled by API", message),
                    }
                }
                // RPS Handler -> API -> broadcaster
                message = self.rps_api_rx.recv() => {
                    info!("API SERVER: received message from RPS handler");
                    let message = match message {
                        Some(message) => message,
                        None => {
                            error!("API Server: received NONE from RPS handler");
                            continue;
                        }
                    };
                    match &message {
                        ApiMessage::RPSPeer(_) => api_broadcaster_tx.send(Ok(message)).await.unwrap(),
                        _ => panic!("message {:?} not meant to be handled by API", message),
                    }
                }
            }
        }
    }
}

impl Handler {
    /// Process a single connection.
    ///
    /// Request messages are read from the socket and processed. Responses are
    /// written back to the socket.
    async fn run(&mut self) {
        loop {
            // handler either reads a message or receives a notification from the api
            tokio::select! {
                result_message = self.connection.read_message() => {
                    let maybe_message = match result_message {
                        Ok(opt_m) => opt_m,
                        Err(e) => {
                            debug!("error while reading message from connection: {}", e);
                            continue
                        },
                    };
                    let message = match maybe_message {
                        Some(m) => m,
                        None => {
                            info!("API HANDLER: Connection closed gracefully");
                            if self.attempt_reconnect {
                                self.reconnect().await;
                                continue
                            } else {
                                break
                            }
                        }
                    };
                    // if message is None, connection closed gracefully
                    debug!("API HANDLER: handling connection message {:?}", &message);
                    self.handle_incoming(message).await;
                }
                notification = self.api_handler_rx.recv() => {
                    let notification = notification.unwrap();
                    debug!("API HANDLER: handling api notification {:?}", &notification);
                    self.handle_outgoing(notification).await;
                }
            };
        }
    }

    async fn reconnect(&mut self) {
        let conn: Connection;
        info!("API HANDLER: attempting to reconnect to {}", self.conn_addr);
        loop {
            conn = match TcpStream::connect(self.conn_addr).await {
                Ok(sock) => {
                    Connection::new(sock)
                },
                Err(e) => {
                    error!("API HANDLER: failed to connect to server: {} waiting and trying again...", e.to_string());
                    std::thread::sleep(std::time::Duration::from_secs(RECONNECT_WAIT));
                    continue
                },
            };
            break
        }
        self.connection = conn;
    }

    async fn handle_incoming(&mut self, message: ApiMessage) {
        // If `None` is returned from `read_message()` then the peer closed
        // the socket. There is no further work to do and the task can be
        // terminated.
        // NOTE handler could be destroyed after connection is dropped
        match &message {
            ApiMessage::Notify(m_sub) => {
                // subscription message
                let mut topic_conn_list_map = self.db.lock().unwrap();
                let conn_list = topic_conn_list_map.entry(m_sub.data_type).or_insert(vec![]);
                if !self.subscribed_topics.contains(&m_sub.data_type) {
                    info!(
                        "subscribing {:?} to topic {}",
                        self.connection, m_sub.data_type
                    );
                    conn_list.push(self.api_handler_tx.clone());
                    self.subscribed_topics.push(m_sub.data_type);
                }
            }
            ApiMessage::Validation(_) | ApiMessage::Announce(_) | ApiMessage::RPSPeer(_) => {
                self.handler_api_tx.send(message).await.unwrap();
            }
            _ => panic!(
                "message of type {:?} is not expected to be received by gossip",
                message
            ),
        }
    }

    async fn handle_outgoing(&mut self, payload: ApiMessage) {
        debug!("outgoing: {:?}", payload);
        self.connection.write_message(payload).await.unwrap();
    }
}

#[cfg(test)]
mod tests {
    use crate::communication::api::connection::Connection;
    use crate::communication::api::message::ApiMessage;
    use crate::communication::api::message::MessageType::GossipNotify;
    use crate::communication::api::payload::notification::Notification;
    use crate::communication::api::payload::notify::Notify;
    use crate::communication::api::payload::rps::peer::{Module, Peer, PortMapRecord};
    use crate::communication::api::server::run;
    use bytes::Bytes;
    use log::{debug, info, warn};
    use num_traits::FromPrimitive;
    use std::net::SocketAddr;
    use std::thread;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;

    const API_TEST_ADDR: &str = "0.0.0.0:1337";
    const RPS_TEST_ADDR: &str = "0.0.0.0:1338";

    async fn run_mock_rps() {
        let mut mock_rps_peer_msg: Vec<u8> = vec![];

        let mock_rps_peer_msg_payload: Vec<u8> = vec![
            0, 0, // port 0
            2, // # portmap
            0, // reserved + V (IpV4)
            0b00000010, 0b10001010, // App 1: DHT (650)
            0, 0, // App 1 port 0
            0b00000010, 0b00001000, // App 2: NSE (520)
            0, 0, // App 2 port 0
            0, 0, 0, 0, // Ipv4 Addr: 0.0.0.0
            0b00000001, 0b00000001, 0b00000001,
            0b00000001, // some bytes which could be a peer's DER host key
        ];

        let payload_size: u8 = u8::from_usize(*&mock_rps_peer_msg_payload.len()).unwrap();
        let mock_rps_peer_header: Vec<u8> = vec![
            0,
            (payload_size + 2), // size
            0b00000010,
            0b00011101, // message code: RPS PEER (541)
        ];

        mock_rps_peer_msg.extend(mock_rps_peer_header);
        mock_rps_peer_msg.extend(mock_rps_peer_msg_payload);

        let rps_listener = TcpListener::bind(RPS_TEST_ADDR).await.unwrap();

        loop {
            let (mut stream, _) = rps_listener.accept().await.unwrap();

            let peer_msg = mock_rps_peer_msg.clone();
            tokio::spawn(async move {
                // read query
                let mut buf = [0u8; 4];
                stream.read_exact(&mut buf).await.unwrap();

                // write response
                stream.write(&peer_msg).await.unwrap();
                stream.flush().await.unwrap();
            });
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_api_interaction() {
        env_logger::init();
        // run api
        let api_listener = TcpListener::bind(API_TEST_ADDR).await.unwrap();

        let (test_pub_server_tx, test_pub_server_rx) = mpsc::channel(512);
        let (server_test_pub_tx, server_test_pub_rx) = mpsc::channel(512);

        let (test_broadcaster_server_tx, test_broadcaster_server_rx) = mpsc::channel(512);
        let (server_test_broadcaster_tx, mut server_test_broadcaster_rx) = mpsc::channel(512);
        let rps_address: SocketAddr = RPS_TEST_ADDR.to_string().parse().unwrap();

        debug!("starting mock rps server");
        tokio::spawn(async {
            run_mock_rps().await;
        });

        // wait until server available
        let mut count = 0;
        loop {
            match TcpStream::connect(RPS_TEST_ADDR).await {
                Ok(_) => break,
                Err(_) => {
                    count += 1;
                    if count > 5 {
                        panic!("unable to connect to rps server");
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }

        debug!("starting api server");
        tokio::spawn(async move {
            run(
                api_listener,
                test_pub_server_rx,
                server_test_pub_tx,
                test_broadcaster_server_rx,
                server_test_broadcaster_tx,
                rps_address,
            )
            .await;
        });

        // serialized messages to send
        // subscription
        let gossip_notify: Vec<u8> = vec![0, 6, 0b00000001, 0b11110101, 0, 0, 0, 0];
        // TODO implement announce
        // let gossip_announce: Vec<u8> = vec![];

        // connect to api
        let mut stream = TcpStream::connect(API_TEST_ADDR).await.unwrap();

        // send notify (subscription) message
        info!("sending notify to test over socket");
        stream.write(&gossip_notify).await.unwrap();

        let data: Vec<u8> = vec![1, 2];
        let data = Bytes::copy_from_slice(&data);
        let msg = ApiMessage::Notification(Notification {
            message_id: 0,
            data_type: 0,
            data,
        });

        // send valid notification (publish) from test to server
        // check that socket receives notification

        thread::sleep(Duration::from_secs(1));

        info!("TEST: sending notification message to server to write back to test over socket");
        test_pub_server_tx.send(msg.clone()).await.unwrap();

        info!("TEST: reading message from server");
        let mut conn = Connection::new(stream);
        let read_msg = conn.read_message().await.unwrap().unwrap();

        info!("sending notify to test over socket");

        assert_eq!(msg, read_msg);

        // test RPS interaction
        let rps_query = ApiMessage::RPSQuery;
        test_broadcaster_server_tx.send(rps_query).await.unwrap();

        let recv_msg = server_test_broadcaster_rx.recv().await.unwrap().unwrap();

        // expected message:

        let bytes: Vec<u8> = vec![1, 1, 1, 1];
        let data = Bytes::from(bytes);
        let check_msg = ApiMessage::RPSPeer(Peer {
            address: "0.0.0.0:0".parse().unwrap(),
            port_map_records: vec![
                PortMapRecord {
                    module: Module::DHT,
                    port: 0,
                },
                PortMapRecord {
                    module: Module::NSE,
                    port: 0,
                },
            ],
            host_key: data,
        });

        assert_eq!(recv_msg, check_msg);
    }
}
