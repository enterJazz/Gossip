use crate::communication::p2p::message;
use crate::communication::p2p::server::run_p2p_server;
use log::{error, info};
use std::str;
use std::sync::Arc;

#[cfg(test)]
#[tokio::test]
async fn p2p_broadcast_test() {
    use tokio::sync::mpsc;

    let (rx_1, mut tx_1) = mpsc::channel(512);
    let (rx_2, mut tx_2) = mpsc::channel(512);
    let (rx_3, mut tx_3) = mpsc::channel(512);

    let s1 = match run_p2p_server("127.0.0.1:1333", rx_1).await {
        Ok(s) => s,
        Err(err) => panic!("failed to start server {}", err),
    };

    let s2 = match run_p2p_server("127.0.0.1:1334", rx_2).await {
        Ok(s) => s,
        Err(err) => panic!("failed to start server {}", err),
    };
    let s3 = match run_p2p_server("127.0.0.1:1335", rx_3).await {
        Ok(s) => s,
        Err(err) => panic!("failed to start server {}", err),
    };

    // connect peers for test case
    match s2.connect("127.0.0.1:1333".parse().unwrap()).await {
        Ok(()) => (),
        Err(e) => {
            panic!("failed to connect {}", e);
        }
    };

    match s3.connect("127.0.0.1:1333".parse().unwrap()).await {
        Ok(()) => (),
        Err(e) => {
            panic!("failed to connect {}", e);
        }
    };

    let payload = "hello there";

    let msg = message::envelope::Msg::Data(message::Data {
        ttl: 1,
        payload: payload.as_bytes().to_vec(),
    });
    match s1.broadcast(msg).await {
        Ok(size) => info!("send okay {}", size),
        Err(e) => error!("err {}", e),
    };

    loop {
        match tx_2.recv().await {
            Some((msg, addr)) => {
                println!("s2: got message from {}", addr);
                match msg {
                    message::envelope::Msg::Data(data) => {
                        assert_eq!(str::from_utf8(&data.payload.to_vec()).unwrap(), payload);
                        break;
                    }
                    _ => (),
                }
                break;
            }
            None => (),
        }
    }

    loop {
        match tx_3.recv().await {
            Some((msg, addr)) => {
                println!("s3: got message from {}", addr);
                match msg {
                    message::envelope::Msg::Data(data) => {
                        assert_eq!(str::from_utf8(&data.payload.to_vec()).unwrap(), payload);

                        break;
                    }
                    _ => panic!("got unexpected message {:?}", msg),
                }
            }
            None => (),
        }
    }
}
