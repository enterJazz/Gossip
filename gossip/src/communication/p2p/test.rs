use crate::{
    communication::{api, p2p, p2p::message},
    config::Config,
};
use log::{error, info};
use std::str;
use std::sync::Arc;

#[cfg(test)]
#[tokio::test]
async fn p2p_broadcast_test() {
    use tokio::sync::mpsc;

    let (tx_1, mut rx_1) = mpsc::channel(512);
    let (tx_2, mut rx_2) = mpsc::channel(512);
    let (tx_3, mut rx_3) = mpsc::channel(512);
    let (conn_tx_1, mut conn_rx_1) = mpsc::channel(512);
    let (conn_tx_2, mut conn_rx_2) = mpsc::channel(512);
    let (conn_tx_3, mut conn_rx_3) = mpsc::channel(512);

    // kreate some nonsense keys for peers
    let k1 = bytes::Bytes::from("this is not a real key 01");
    let k2 = bytes::Bytes::from("this is not a real key 02");
    let k3 = bytes::Bytes::from("this is not a real key 03");

    let s1 = match p2p::server::run_from_str_addr("127.0.0.1:1333", k1, tx_1, conn_tx_1).await {
        Ok(s) => s,
        Err(err) => panic!("failed to start server {}", err),
    };

    let s2 = match p2p::server::run_from_str_addr("127.0.0.1:1334", k2, tx_2, conn_tx_2).await {
        Ok(s) => s,
        Err(err) => panic!("failed to start server {}", err),
    };
    let s3 = match p2p::server::run_from_str_addr("127.0.0.1:1335", k3, tx_3, conn_tx_3).await {
        Ok(s) => s,
        Err(err) => panic!("failed to start server {}", err),
    };

    // connect peers for test case
    match s2.connect("127.0.0.1:1333").await {
        Ok(()) => (),
        Err(e) => {
            panic!("failed to connect {}", e);
        }
    };

    match s3.connect("127.0.0.1:1333").await {
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
    return todo!("fix @wlad");
    //    match s1.broadcast(msg).await {
    //        Ok(size) => info!("send okay {}", size),
    //        Err(e) => error!("err {}", e),
    //    };
    //
    //    loop {
    //        match tx_2.recv().await {
    //            Some((data, addr)) => {
    //                println!("s2: got message from {}", addr);
    //                assert_eq!(str::from_utf8(&data.payload.to_vec()).unwrap(), payload);
    //                break;
    //            }
    //            None => (),
    //        }
    //    }
    //
    //    loop {
    //        match tx_3.recv().await {
    //            Some((data, addr)) => {
    //                println!("s3: got message from {}", addr);
    //                assert_eq!(str::from_utf8(&data.payload.to_vec()).unwrap(), payload);
    //                break;
    //            }
    //            None => (),
    //        }
    //    }
}
