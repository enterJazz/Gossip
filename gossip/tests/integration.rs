mod common;

use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Output},
    thread,
};

use gossip::{broadcaster, config::Config};
use ini::Ini;
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
}
