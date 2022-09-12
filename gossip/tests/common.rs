use std::{process::Command, path::PathBuf};

// path to mockups
const RPS_MOCKUP_PATH: &str = "tests/mockups/rps_mockup.py";

// path to client
const GOSSIP_CLIENT_PATH: &str = "tests/clients/gossip_client.py";

// python command
const PYTHON3: &str = "python3";

// test config paths
const TEST_CONFIG_PATH_BOOTSTRAPPER: &str = "tests/resources/integration_config_bootstrapper.ini";
const TEST_CONFIG_PATH_PEER_1: &str = "tests/resources/integration_config_peer_1.ini";
const TEST_CONFIG_PATH_PEER_2: &str = "tests/resources/integration_config_peer_2.ini";
const TEST_CONFIG_PATH_PEER_3: &str = "tests/resources/integration_config_peer_3.ini";

// test config templates
const TEST_CONFIG_PATH_BOOTSTRAPPER_TEMPLATE: &str = "tests/resources/configs/integration_config_bootstrapper_template.ini";
const TEST_CONFIG_PATH_PEER_1_TEMPLATE: &str = "tests/resources/configs/integration_config_peer_1_template.ini";
const TEST_CONFIG_PATH_PEER_2_TEMPLATE: &str = "tests/resources/configs/integration_config_peer_2_template.ini";
const TEST_CONFIG_PATH_PEER_3_TEMPLATE: &str = "tests/resources/configs/integration_config_peer_3_template.ini";

// config ini sections
const GOSSIP_SECTION: &str = "gossip";
const API_FIELD: &str = "api_address";

// config ini fields
const GENERAL_HOST_KEY: &str = "hostkey";

// timeout duration in secs
const SERVICE_TIMEOUT_DURATION_SECS: u64 = 5;

pub mod api {
    use std::{net::SocketAddr, process::Output};

    use ini::Ini;

    use super::*;

    pub mod gossip {
        use super::*;

        pub async fn client_announce_to_bootstrapper_peer() {
            let test_config = setup(Action::Announce);
            run_gossip_api_client(test_config).await;
        }

        pub async fn client_notify_to_bootstrapper_peer() {
            let test_config = setup(Action::Notify);
            run_gossip_api_client(test_config).await;
        }

        #[derive(Debug)]
        enum Action {
            Announce,
            Notify,
        }

        struct GossipClientConfig {
            destination: String,
            port: u16,
            action: Action,
        }

        fn setup(action: Action) ->  GossipClientConfig {
            let mut config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            config_path.push(TEST_CONFIG_PATH_BOOTSTRAPPER);
            let ini_conf = Ini::load_from_file(config_path.clone()).unwrap();
            let gossip_conf = ini_conf.section(Some(GOSSIP_SECTION)).unwrap();

            let target_addr = match action {
                // case: API action
                Action::Announce | Action::Notify => {
                    gossip_conf.get(API_FIELD).unwrap().parse::<SocketAddr>().unwrap()
                },
                _ => todo!(),
            };

            let destination = String::from(target_addr.ip().to_string());
            let port = target_addr.port();

            // start gossip module

            GossipClientConfig {
                destination,
                port,
                action,
            }
        }

        async fn run_gossip_api_client(conf: GossipClientConfig) {
            let mut client_cmd = Command::new(PYTHON3);
            
            client_cmd
                .arg(GOSSIP_CLIENT_PATH);

            match &conf.action {
                Action::Announce => client_cmd.arg("--announce"),
                Action::Notify => client_cmd.arg("--notify"),
            };

            client_cmd
                .args(["--destination", &conf.destination])
                .args(["--port", &conf.port.to_string()]);

            tokio::spawn(async move {
                let out = client_cmd.output().unwrap();
                utils::assert_cmd_success(out);
            });
        }
    }

    pub mod rps {
        use std::{thread, time::Duration};

        use log::debug;

        use super::*;

        pub async fn start_rps() {
            let rps_config = setup_rps_config();
            start_rps_with_config(rps_config.clone()).await;
            utils::wait_for_service(format!("{}:{}", &rps_config.server_address, &rps_config.server_port), Duration::from_secs(SERVICE_TIMEOUT_DURATION_SECS));
        }

        #[derive(Debug, Clone)]
        struct RPSMockupConfig {
            server_address: String,
            server_port: u16,
            first_peer: String,
            first_ports: String,
            first_key: PathBuf,
            second_peer: String,
            second_ports: String,
            second_key: PathBuf,
        }

        fn setup_rps_config() -> RPSMockupConfig {
            let bootstrapper_config = utils::get_bootstrapper_config();
            let peer_1_config = utils::get_peer_1_config();
            let peer_2_config = utils::get_peer_2_config();

            let rps_addr = bootstrapper_config.get_rps_address();

            RPSMockupConfig {
                server_address: rps_addr.clone().ip().to_string(),
                server_port: rps_addr.clone().port(),
                first_peer: peer_1_config.get_p2p_address().to_string(),
                first_ports: "650:1234,500:4567".to_string(),
                first_key: peer_1_config.get_host_key_path().to_owned(),
                second_peer: peer_2_config.get_p2p_address().to_string(),
                second_ports: "520:4444,560:1111".to_string(),
                second_key: peer_2_config.get_host_key_path().to_owned(),
            }
        }

        async fn start_rps_with_config(config: RPSMockupConfig) {
            let rps_path = utils::canonicalize_relative_path_to_repo_dir(RPS_MOCKUP_PATH);

            debug!("rps conf: {:?}", config.clone());
            tokio::spawn(async move {
                let out = Command::new(PYTHON3)
                    .arg(rps_path.to_str().unwrap())
                    .args(["--address", &config.server_address])
                    .args(["--port", &config.server_port.to_string()])
                    .args(["--firstpeer", &config.first_peer.to_string()])
                    .args(["--firstports", &config.first_ports.to_string()])
                    .args(["--firstkey", &config.first_key.to_str().unwrap()])
                    .args(["--secpeer", &config.second_peer.to_string()])
                    .args(["--secports", &config.second_ports.to_string()])
                    .args(["--seckey", &config.second_key.to_str().unwrap()])
                    .output().unwrap();
                    utils::assert_cmd_success(out);
            });
       }
    }
}

mod utils {
    use std::{time::{Instant, Duration}, net::TcpStream, path::PathBuf, thread, process::Output};

    use gossip::config;
    use ini::Ini;
    use log::{warn, info};

    use super::{TEST_CONFIG_PATH_BOOTSTRAPPER_TEMPLATE, TEST_CONFIG_PATH_PEER_1_TEMPLATE, TEST_CONFIG_PATH_PEER_2_TEMPLATE, TEST_CONFIG_PATH_PEER_3_TEMPLATE, GENERAL_HOST_KEY};

    use super::{TEST_CONFIG_PATH_BOOTSTRAPPER, TEST_CONFIG_PATH_PEER_1, TEST_CONFIG_PATH_PEER_2, TEST_CONFIG_PATH_PEER_3};

    // wait for a service to be connectable via tcp until timeout is reached
    pub fn wait_for_service(address: String, timeout: Duration) {
        let now = Instant::now();

        loop {
            match TcpStream::connect(address.clone()) {
                Ok(_) => {
                    info!("connected to {}", address.clone());
                    break
                },
                Err(e) => warn!("couldn't connect to service at {}: {}\nwaiting...", address.clone(), e.to_string()),
            }
            thread::sleep(Duration::from_secs(1));
            if now.elapsed() >= timeout {
                panic!("timeout on waiting for service at {}", address.clone());
            }
        }
    }

    pub fn get_all_peer_configs() -> Vec<config::Config> {
        Vec::from([get_bootstrapper_config(), get_peer_1_config(), get_peer_2_config(), get_peer_3_config()])
    }

    pub fn get_bootstrapper_config() -> config::Config {
        get_peer_x_config(0)
    }

    pub fn get_peer_1_config() -> config::Config {
        get_peer_x_config(1)
    }

    pub fn get_peer_2_config() -> config::Config {
        get_peer_x_config(2)
    }

    pub fn get_peer_3_config() -> config::Config {
        get_peer_x_config(3)
    }

    fn get_peer_x_config(x: usize) -> config::Config {
        let (peer_conf_path, peer_conf_template) = match x {
            0 => (TEST_CONFIG_PATH_BOOTSTRAPPER, TEST_CONFIG_PATH_BOOTSTRAPPER_TEMPLATE),
            1 => (TEST_CONFIG_PATH_PEER_1, TEST_CONFIG_PATH_PEER_1_TEMPLATE),
            2 => (TEST_CONFIG_PATH_PEER_2, TEST_CONFIG_PATH_PEER_2_TEMPLATE),
            3 => (TEST_CONFIG_PATH_PEER_3, TEST_CONFIG_PATH_PEER_3_TEMPLATE),
            _ => panic!("invalid config param peer id"),
        };

        let config_path = canonicalize_relative_path_to_repo_dir(peer_conf_path);

        // modify template
        init_config_ini(peer_conf_template, peer_conf_path);

        config::Config::load_config(config_path).unwrap()
    }

    fn init_config_ini(config_template_path: &str, config_target_path: &str) {
        let mut conf = Ini::load_from_file(config_template_path).unwrap();
        let adjusted_path = canonicalize_relative_path_to_repo_dir(conf.general_section().get(GENERAL_HOST_KEY).unwrap());
        conf.with_general_section().set(GENERAL_HOST_KEY, adjusted_path.to_str().unwrap());
        conf.write_to_file(config_target_path).unwrap();
    }

    pub fn canonicalize_relative_path_to_repo_dir(relative_path: &str) -> PathBuf {
        let base_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        base_path.join(relative_path)
    }

    pub fn assert_cmd_success(out: Output) {
        assert!(out.status.success(), "out:\ncode: {}\nstdout:{}\nstderr:{}",
            out.status,
            String::from_utf8(out.stdout).unwrap(),    
            String::from_utf8(out.stderr).unwrap(),    
        );

    }

}

pub mod common_gossip {
    use std::time::Duration;

    use super::*;
    use gossip::broadcaster;

    /// to start gossip: we start the bootstrapper node. This node needs an RPS instance to connect to, which we start beforehand.
    pub async fn start_gossip_bootstrapper() {
        let config_path = utils::canonicalize_relative_path_to_repo_dir(TEST_CONFIG_PATH_BOOTSTRAPPER);

        let config = gossip::config::Config::load_config(config_path).unwrap();
        let thread_config = config.clone();

        tokio::spawn(async move {
            let gossip = broadcaster::Broadcaster::new(thread_config).await;
            gossip.run().await.unwrap();
        });

        utils::wait_for_service(config.get_api_address().to_string(), Duration::from_secs(SERVICE_TIMEOUT_DURATION_SECS));
        utils::wait_for_service(config.get_p2p_address().to_string(), Duration::from_secs(SERVICE_TIMEOUT_DURATION_SECS));
    }
}