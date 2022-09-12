mod common;

use std::io::Write;
use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Output},
    thread,
};

use chrono::Local;
use env_logger::Builder;
use gossip::{broadcaster, config::Config};
use ini::Ini;
use log::LevelFilter;
use url::Url;

async fn setup_gossip() {
    // start RPS
    common::api::rps::start_rps().await;
    // start gossip
    common::common_gossip::start_gossip_bootstrapper().await;
}

mod api {
    use std::time::Duration;

    use crate::{common, setup_gossip};
    use gossip::broadcaster;
    use log::info;
    use test_log::test;

    #[test(tokio::test(flavor = "multi_thread", worker_threads = 8))]
    async fn test_gossip_api() {
        setup_gossip().await;

        common::api::gossip::client_notify_to_bootstrapper_peer().await;
        std::thread::sleep(Duration::from_secs(1));
        common::api::gossip::client_announce_to_bootstrapper_peer().await;
        std::thread::sleep(Duration::from_secs(3));
    }

    #[test(tokio::test(flavor = "multi_thread", worker_threads = 16))]
    async fn test_gossip_p2p_push_pull() {
        // start RPS
        common::api::rps::start_rps().await;

        let bootsrapper_config = common::utils::get_bootstrapper_config();
        let c1 = common::utils::get_peer_1_config();
        let c2 = common::utils::get_peer_2_config();
        let c3 = common::utils::get_peer_3_config();

        println!("p2p 1: {}", bootsrapper_config.get_p2p_address());

        // spawn client nodes
        for conf in [c1, c2, c3] {
            println!("spawning client {}", conf.get_p2p_address());
            tokio::spawn(async move {
                let b = broadcaster::Broadcaster::new(conf).await;

                match b.run().await {
                    Ok(_) => info!("stopping gossip"),
                    Err(err) => panic!("gossip failed {}", err),
                }
            });
        }

        let b = broadcaster::Broadcaster::new(bootsrapper_config.clone()).await;
        let payload = b.inser_random_item().await;

        match b.run().await {
            Ok(_) => info!("stopping gossip"),
            Err(err) => panic!("gossip failed {}", err),
        }
    }
}
