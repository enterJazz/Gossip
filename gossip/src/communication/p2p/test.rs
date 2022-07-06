use crate::communication::p2p::com::Communicator;
use log::info;
use std::{io, net::SocketAddr, sync::Arc};
use tokio::{net::UdpSocket, sync::mpsc};

#[cfg(test)]
#[tokio::test]
async fn p2p_comm_test() {
    let (s1, mut rx_s1) = Communicator::new("127.0.0.1:0".to_string(), 1)
        .await
        .unwrap();
    let (s2, mut rx_s2) = Communicator::new("127.0.0.1:0".to_string(), 1)
        .await
        .unwrap();
    let (s3, _) = Communicator::new("127.0.0.1:0".to_string(), 1)
        .await
        .unwrap();

    let s1_addr = s1.local_addr().unwrap();
    let s2_addr = s2.local_addr().unwrap();
    let s3_addr = s3.local_addr().unwrap();

    println!("s1 ({}) <-> s2 ({})", s1_addr, s2_addr);

    let s1_payload_raw = "1. Hello world";
    let s2_payload_raw = "2. Hello world";
    let s3_payload_raw = "3. Hello world";
    let message_count = 100;

    // test s1 -> s2
    let task = tokio::spawn(async move {
        let mut counter = 1;
        loop {
            let (buf, src_addr) = rx_s2.recv().await.unwrap();
            let s = String::from_utf8_lossy(&buf);

            assert_eq!(src_addr, s1_addr);
            assert_eq!(s, s1_payload_raw);
            if counter == message_count {
                return;
            }
            counter += 1;
        }
    });

    // test s2,s3 -> s1
    let task = tokio::spawn(async move {
        let mut s2_counter = 1;
        let mut s3_counter = 1;
        loop {
            let (buf, src_addr) = rx_s1.recv().await.unwrap();
            let s = String::from_utf8_lossy(&buf);

            if s2_counter == message_count && s3_counter == message_count {
                return;
            }
            if src_addr == s2_addr {
                assert_eq!(s, s2_payload_raw);
                s2_counter += 1;
            }
            if src_addr == s3_addr {
                assert_eq!(s, s3_payload_raw);
                s3_counter += 1;
            }
        }
    });

    // send out messages
    for _ in 1..=message_count {
        _ = s1
            .send_to(s2_addr, s1_payload_raw.as_bytes().to_vec().clone())
            .await;
        _ = s2
            .send_to(s1_addr, s2_payload_raw.as_bytes().to_vec().clone())
            .await;
        _ = s3
            .send_to(s1_addr, s3_payload_raw.as_bytes().to_vec().clone())
            .await;
    }

    task.await;
}
