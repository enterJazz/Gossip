use crate::{
    communication::{api, p2p, p2p::message},
    config::Config,
};
use log::{error, info};
use std::str;
use std::sync::Arc;
use tokio::sync::mpsc;

async fn create_peer(
    port: u16,
) -> (
    Arc<p2p::server::Server>,
    mpsc::Receiver<p2p::server::ServerPeerMessage>,
    mpsc::Receiver<p2p::server::PeerConnectionMessage>,
) {
    let (tx, rx) = mpsc::channel(512);
    let (conn_tx, conn_rx) = mpsc::channel(512);

    let key = bytes::Bytes::from(format!("this is not a real key {}", port));

    let s = match p2p::server::run_from_str_addr(
        &format!("127.0.0.1:{}", port).to_string(),
        key,
        tx,
        conn_tx,
    )
    .await
    {
        Ok(s) => s,
        Err(err) => panic!("failed to start server {}", err),
    };

    (s, rx, conn_rx)
}

#[cfg(test)]
#[tokio::test]
async fn p2p_unique_connection_test() {
    use core::panic;

    let (s1, _, _) = create_peer(1333).await;
    let (s2, mut rx_2, _) = create_peer(1334).await;

    // connect peers for test case
    match s2.connect("127.0.0.1:1333").await {
        Ok(()) => (),
        Err(e) => {
            panic!("failed to connect {}", e);
        }
    };

    match s1.connect("127.0.0.1:1334").await {
        Ok(()) => panic!("duplicate connection created"),
        Err(e) => match e {
            p2p::peer::PeerError::Duplicate => (),
            _ => panic!("got unexpected error"),
        },
    };
}

#[cfg(test)]
#[tokio::test]
async fn p2p_send_to_test() {
    let (s1, _, _) = create_peer(1333).await;
    let (s2, mut rx_2, _) = create_peer(1334).await;

    // connect peers for test case
    match s2.connect("127.0.0.1:1333").await {
        Ok(()) => (),
        Err(e) => {
            panic!("failed to connect {}", e);
        }
    };

    let payload = "hello there";
    let msg = message::Data {
        ttl: 1,
        data_type: 2,
        payload: payload.as_bytes().to_vec(),
    };
    _ = s1
        .send_to_peer(s2.get_identity(), p2p::message::envelope::Msg::Data(msg))
        .await;

    match rx_2.recv().await {
        Some((p2p::message::envelope::Msg::Data(msg), identity, _)) => {
            assert_eq!(identity, s1.get_identity());
            assert_eq!(str::from_utf8(&msg.payload.to_vec()).unwrap(), payload);
        }
        None => panic!("failed to receive message"),
        _ => panic!("received unexpected message"),
    };
}

#[cfg(test)]
#[tokio::test]
async fn p2p_broadcast_test() {
    let (s1, _, _) = create_peer(1333).await;
    let (s2, rx_2, _) = create_peer(1334).await;
    let (s3, rx_3, _) = create_peer(1335).await;
    let (s4, rx_4, _) = create_peer(1336).await;

    // connect peers for test case
    match s2.connect("127.0.0.1:1333").await {
        Ok(()) => (),
        Err(e) => {
            panic!("failed to connect {}", e);
        }
    };

    for s in [s2, s3, s4] {
        match s.connect("127.0.0.1:1333").await {
            Ok(()) => (),
            Err(e) => {
                panic!("failed to connect {}", e);
            }
        };
    }

    let payload = "hello there";
    let msg = message::Data {
        ttl: 1,
        data_type: 2,
        payload: payload.as_bytes().to_vec(),
    };
    _ = s1.broadcast(msg, Vec::new()).await;

    for mut rx in [rx_2, rx_3, rx_4] {
        match rx.recv().await {
            Some((p2p::message::envelope::Msg::Data(msg), identity, _)) => {
                assert_eq!(identity, s1.get_identity());
                assert_eq!(str::from_utf8(&msg.payload.to_vec()).unwrap(), payload);
            }
            None => panic!("failed to receive message"),
            _ => panic!("received unexpected message"),
        };
    }
}
