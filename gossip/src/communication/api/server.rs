//! API server implementation
//!
//! Provides an async `run` function that listens for inbound connections,
//! spawning a task per connection.

use std::collections::HashMap;

use crate::communication::api::connection::Connection;
use crate::communication::api::message::ApiMessage;
use log::{debug, info, warn};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

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
    api_pub_tx: mpsc::Sender<Result<ApiMessage, ()>>,
) {
    // Initialize the listener state
    let (tx, rx) = mpsc::channel(512);
    let mut server = Listener {
        listener,
        handler_api_rx: rx,
        db_holder: Arc::new(Mutex::new(HashMap::new())),
        handler_api_tx: tx,
    };

    // transmitter must be given to publisher
    // idea: publisher passes receiver upon run
    server.run(pub_api_rx, api_pub_tx).await;
}

impl Listener {
    /// Run the server
    ///
    /// Listen for inbound connections. For each inbound connection, spawn a
    /// task to process that connection.
    async fn run(
        &mut self,
        mut pub_api_rx: mpsc::Receiver<ApiMessage>,
        mut api_pub_tx: mpsc::Sender<Result<ApiMessage, ()>>,
    ) {
        info!("API SERVER: accepting inbound connections");

        loop {
            tokio::select! {
                res = self.listener.accept() => {
                    let (socket, addr) = res.unwrap();
                    info!("API SERVER: received connection from {:?}", addr);
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
                    };

                    // Spawn a new task to process the connections
                    tokio::spawn(async move {
                        handler.run().await;
                    });
                }

                message = pub_api_rx.recv() => {
                    info!("API SERVER: received message from api");
                    let message = message.unwrap();
                    match &message {
                        ApiMessage::Notification(m_sub) => {
                            let mut tx_vec_cp;
                            {
                                let db = self.db_holder.lock().unwrap();
                                let mut tx_vec = match db.get(&m_sub.data_type) {
                                    Some(tx_vec) => tx_vec,
                                    None => {
                                        warn!("no subscribers found for topic {}", m_sub.data_type);
                                        api_pub_tx.send(Err(()));
                                        return;
                                    }
                                };
                                tx_vec_cp = tx_vec.clone();
                            }

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
                                warn!("no subscribers found for topic {}", m_sub.data_type);
                                api_pub_tx.send(Err(()));
                                return;
                            }


                        }
                        _ => panic!("message {:?} may not be sent by server", message),
                    }
                }

                message = self.handler_api_rx.recv() => {
                    info!("API SERVER: received message from handler");
                    let message = message.unwrap();
                    match &message {
                        ApiMessage::Announce(_) | ApiMessage::Validation(_) => {
                            api_pub_tx.send(Ok(message)).await.unwrap();
                        }
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
                maybe_message = self.connection.read_message() => {
                    let message = maybe_message.unwrap();
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

    async fn handle_incoming(&mut self, maybe_message: Option<ApiMessage>) {
        // If `None` is returned from `read_message()` then the peer closed
        // the socket. There is no further work to do and the task can be
        // terminated.
        // NOTE handler could be destroyed after connection is dropped
        let message = match maybe_message {
            Some(message) => message,
            None => return,
        };

        debug!("incoming: {:?}", message);

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
            ApiMessage::Validation(_) | ApiMessage::Announce(_) => {
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
    use std::thread;
    use std::time::Duration;
    use bytes::Bytes;
    use crate::communication::api::connection::Connection;
    use crate::communication::api::message::ApiMessage;
    use crate::communication::api::message::MessageType::GossipNotify;
    use crate::communication::api::payload::notification::Notification;
    use crate::communication::api::server::run;
    use log::{debug, info};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;
    use crate::communication::api::payload::notify::Notify;

    const TEST_ADDR: &str = "localhost:1337";

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_api_interaction() {
        env_logger::init();
        // run api
        let listener = TcpListener::bind(TEST_ADDR).await.unwrap();

        let (test_server_tx, test_server_rx) = mpsc::channel(512);
        let (server_test_tx, server_test_rx) = mpsc::channel(512);

        debug!("starting api server");
        tokio::spawn(async move {
            run(listener, test_server_rx, server_test_tx).await;
        });

        // serialized messages to send
        // subscription
        let gossip_notify: Vec<u8> = vec![0, 6, 0b00000001, 0b11110101, 0, 0, 0, 0];
        // TODO implement announce
        // let gossip_announce: Vec<u8> = vec![];

        // connect to api
        let mut stream = TcpStream::connect(TEST_ADDR).await.unwrap();

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

        // wait until notify arrives before sending notification
        thread::sleep(Duration::from_secs(1));

        info!("TEST: sending notification message to server to write back to test over socket");
        test_server_tx.send(msg.clone()).await.unwrap();

        info!("TEST: reading message from server");
        let mut conn = Connection::new(stream);
        let read_msg = conn.read_message().await.unwrap().unwrap();


        assert_eq!(msg, read_msg);
    }
}
