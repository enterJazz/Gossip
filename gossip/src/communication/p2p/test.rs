use crate::communication::p2p::server::Server;

#[cfg(test)]
#[tokio::test]
async fn p2p_comm_test() {
    let mut s1 = Server::new("127.0.0.1:1333".parse().unwrap()).await;
    let mut s2 = Server::new("127.0.0.1:1334".parse().unwrap()).await;
    let mut s3 = Server::new("127.0.0.1:1335".parse().unwrap()).await;

    tokio::spawn(async move {
        _ = s1.run().await;
    });

    _ = tokio::spawn(async move {
        _ = match s2.connect("127.0.0.1:1333".parse().unwrap()).await {
            Ok(()) => (),
            Err(e) => {
                panic!("failed to connect {}", e);
            }
        }
    })
    .await;

    _ = tokio::spawn(async move {
        _ = s3.connect("127.0.0.1:1333".parse().unwrap()).await;
    })
    .await;
}
//     let s1_payload_raw = "1. Hello world";
//     let s2_payload_raw = "2. Hello world";
//     let s3_payload_raw = "3. Hello world";
//     let message_count = 100;

//     // test s1 -> s2
//     let task = tokio::spawn(async move {
//         let mut counter = 1;
//         loop {
//             let (buf, src_addr) = rx_s2.recv().await.unwrap();
//             let s = String::from_utf8_lossy(&buf);

//             assert_eq!(src_addr, s1_addr);
//             assert_eq!(s, s1_payload_raw);
//             if counter == message_count {
//                 return;
//             }
//             counter += 1;
//         }
//     });

//     // test s2,s3 -> s1
//     let task = tokio::spawn(async move {
//         let mut s2_counter = 1;
//         let mut s3_counter = 1;
//         loop {
//             let (buf, src_addr) = rx_s1.recv().await.unwrap();
//             let s = String::from_utf8_lossy(&buf);

//             if s2_counter == message_count && s3_counter == message_count {
//                 return;
//             }
//             if src_addr == s2_addr {
//                 assert_eq!(s, s2_payload_raw);
//                 s2_counter += 1;
//             }
//             if src_addr == s3_addr {
//                 assert_eq!(s, s3_payload_raw);
//                 s3_counter += 1;
//             }
//         }
//     });

//     // send out messages
//     for _ in 1..=message_count {
//         _ = s1
//             .send_to(s2_addr, s1_payload_raw.as_bytes().to_vec().clone())
//             .await;
//         _ = s2
//             .send_to(s1_addr, s2_payload_raw.as_bytes().to_vec().clone())
//             .await;
//         _ = s3
//             .send_to(s1_addr, s3_payload_raw.as_bytes().to_vec().clone())
//             .await;
//     }

//     task.await;
// }
