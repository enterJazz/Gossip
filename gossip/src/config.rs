use ini::{Ini, Properties};
use log::{error, info};
use openssl::{
    pkey::{Private, Public},
    rsa::{self, Rsa},
};
use std::{collections::HashMap, fs, io, net::SocketAddr, path::PathBuf, str::FromStr};
use thiserror::Error;

const GOSSIP_SECTION_NAME: &str = "gossip";
const RPS_SECTION_NAME: &str = "rps";

const HOST_KEY_PATH_FIELD: &str = "hostkey";
const CACHE_SIZE_CONFIG_FIELD: &str = "cache_size";
const DEGREE_CONFIG_FIELD: &str = "degree";
const BOOTSTRAPPER_CONFIG_FIELD: &str = "bootstrapper";
const P2P_ADDRESS_CONFIG_FIELD: &str = "p2p_address";
const API_ADDRESS_CONFIG_FIELD: &str = "api_address";

const RPS_ADDRESS_CONFIG_FIELD: &str = "rps_address";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Error loading config {}: {}", path.display(), err_msg)]
    LoadingError { path: PathBuf, err_msg: String },
    #[error("Config section messing: {section_name}")]
    SectionMissing { section_name: String },
    #[error("Config field missing: {field_name}")]
    FieldMissing { field_name: String },
    #[error("Failed to parse field {field_name}: {err_msg}")]
    ParsingError { field_name: String, err_msg: String },
    #[error("Failed to parse pem file: {0}")]
    PemParseError(#[from] pem::PemError),
    #[error("Failed to process RSA key")]
    RsaKeyError,
}

/// Configuration loaded from ini file
#[derive(Debug, Clone)]
pub struct Config {
    host_key_path: PathBuf,
    host_priv_key: Rsa<Private>,
    host_pub_key: Rsa<Public>,

    cache_size: usize,
    degree: usize,
    bootstrapper: Option<String>,
    p2p_address: SocketAddr,
    api_address: SocketAddr,
    rps_address: SocketAddr,
}

impl Config {
    /// Loads config from a given path
    pub fn load_config(path_to_config: PathBuf) -> Result<Self, ConfigError> {
        let conf =
            Ini::load_from_file(path_to_config.clone()).map_err(|e| ConfigError::LoadingError {
                path: path_to_config,
                err_msg: e.to_string(),
            })?;
        let host_key_path = conf
            .general_section()
            .get(HOST_KEY_PATH_FIELD)
            .ok_or(ConfigError::FieldMissing {
                field_name: HOST_KEY_PATH_FIELD.to_string(),
            })?
            .parse::<PathBuf>()
            .map_err(|e| ConfigError::ParsingError {
                field_name: HOST_KEY_PATH_FIELD.to_string(),
                err_msg: e.to_string(),
            })?;

        println!("key at {}", host_key_path.display());
        let mut host_pub_key_path = host_key_path.clone();
        host_pub_key_path.set_extension("pub");
        let pub_key = match fs::read(host_pub_key_path) {
            Ok(key) => {
                rsa::Rsa::public_key_from_pem(&key).expect("failed to parse host public key")
            }
            Err(err) => panic!("failed to read host public key: {}", err),
        };

        println!("key at {}", host_key_path.display());
        let priv_key = match fs::read(host_key_path.clone()) {
            Ok(key) => {
                rsa::Rsa::private_key_from_pem(&key).expect("failed to parse host private key")
            }
            Err(err) => panic!("failed to read host private key: {}", err),
        };

        let gossip_section =
            conf.section(Some(GOSSIP_SECTION_NAME))
                .ok_or(ConfigError::SectionMissing {
                    section_name: GOSSIP_SECTION_NAME.to_string(),
                })?;

        // parse bootstrapping address
        let mut bootstrapping_addr: Option<String> = None;
        if let Some(addr) = gossip_section.get(BOOTSTRAPPER_CONFIG_FIELD.clone()) {
            bootstrapping_addr = Some(addr.to_string())
        }

        // parse usized values
        let parsed_usize_keys: Vec<String> = vec![
            CACHE_SIZE_CONFIG_FIELD.to_string(),
            DEGREE_CONFIG_FIELD.to_string(),
        ];

        let mut parsed_usize_fields: HashMap<String, usize> = HashMap::new();
        for field_key in parsed_usize_keys {
            if let Some(field_value) = gossip_section.get(&field_key) {
                let usize_parsed_value: usize =
                    field_value
                        .parse::<usize>()
                        .map_err(|e| ConfigError::ParsingError {
                            field_name: field_key.clone(),
                            err_msg: e.to_string(),
                        })?;
                let _ = parsed_usize_fields.insert(field_key.clone(), usize_parsed_value);
            } else {
                panic!("invalid state: invalid config key");
            }
        }

        // parse addresses
        let parsed_address_keys: Vec<String> = vec![
            P2P_ADDRESS_CONFIG_FIELD.to_string(),
            API_ADDRESS_CONFIG_FIELD.to_string(),
        ];
        let mut parsed_address_fields: HashMap<String, SocketAddr> = HashMap::new();
        for field_key in parsed_address_keys {
            if let Some(field_value) = gossip_section.get(&field_key) {
                let socketaddr_parsed_value: SocketAddr = field_value
                    .parse::<SocketAddr>()
                    .map_err(|e| ConfigError::ParsingError {
                        field_name: field_key.clone(),
                        err_msg: e.to_string(),
                    })?;
                let _ = parsed_address_fields.insert(field_key.clone(), socketaddr_parsed_value);
            } else {
                panic!("invalid state: invalid config key: {}", field_key);
            }
        }

        let rps_section =
            conf.section(Some(RPS_SECTION_NAME))
                .ok_or(ConfigError::SectionMissing {
                    section_name: RPS_SECTION_NAME.to_string(),
                })?;

        let rps_address = rps_section
            .get(RPS_ADDRESS_CONFIG_FIELD)
            .ok_or(ConfigError::FieldMissing {
                field_name: RPS_ADDRESS_CONFIG_FIELD.to_string(),
            })?
            .parse::<SocketAddr>()
            .map_err(|e| ConfigError::ParsingError {
                field_name: RPS_ADDRESS_CONFIG_FIELD.to_string(),
                err_msg: e.to_string(),
            })?;

        Ok(Self {
            host_key_path,
            host_priv_key: priv_key,
            host_pub_key: pub_key,
            cache_size: parsed_usize_fields[CACHE_SIZE_CONFIG_FIELD],
            degree: parsed_usize_fields[DEGREE_CONFIG_FIELD],
            bootstrapper: bootstrapping_addr,
            p2p_address: parsed_address_fields[P2P_ADDRESS_CONFIG_FIELD],
            api_address: parsed_address_fields[API_ADDRESS_CONFIG_FIELD],
            rps_address,
        })
    }

    pub fn get_host_key_path(&self) -> &PathBuf {
        &self.host_key_path
    }

    pub fn get_host_pub_key(&self) -> openssl::rsa::Rsa<Public> {
        self.host_pub_key.clone()
    }

    pub fn get_host_pub_key_der(&self) -> bytes::Bytes {
        bytes::Bytes::from(
            self.host_pub_key
                .public_key_to_der()
                .expect("failed to encode public key to pem"),
        )
    }

    pub fn get_cache_size(&self) -> usize {
        self.cache_size
    }

    pub fn get_degree(&self) -> usize {
        self.degree
    }

    pub fn get_bootstrapper(&self) -> Option<String> {
        self.bootstrapper.clone()
    }

    pub fn get_p2p_address(&self) -> SocketAddr {
        self.p2p_address
    }

    pub fn get_api_address(&self) -> SocketAddr {
        self.api_address
    }

    pub fn get_rps_address(&self) -> SocketAddr {
        self.rps_address
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Config, API_ADDRESS_CONFIG_FIELD, BOOTSTRAPPER_CONFIG_FIELD, CACHE_SIZE_CONFIG_FIELD,
        DEGREE_CONFIG_FIELD, GOSSIP_SECTION_NAME, HOST_KEY_PATH_FIELD, P2P_ADDRESS_CONFIG_FIELD,
        RPS_ADDRESS_CONFIG_FIELD, RPS_SECTION_NAME,
    };
    use ini::Ini;
    use std::{net::SocketAddr, path::PathBuf};
    use url::Url;
    const TEST_CONFIG_PATH: &str = "tests/resources/config.ini";

    #[test]
    fn test_load_config() {
        let mut config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        config_path.push(TEST_CONFIG_PATH);

        let ini_conf = Ini::load_from_file(config_path.clone()).unwrap();
        let generated_config = Config::load_config(config_path).unwrap();

        assert_eq!(
            &ini_conf
                .general_section()
                .get(HOST_KEY_PATH_FIELD)
                .unwrap()
                .parse::<PathBuf>()
                .unwrap(),
            generated_config.get_host_key_path()
        );

        let gossip_conf = ini_conf.section(Some(GOSSIP_SECTION_NAME)).unwrap();
        assert_eq!(
            gossip_conf
                .get(CACHE_SIZE_CONFIG_FIELD)
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            generated_config.get_cache_size()
        );
        assert_eq!(
            gossip_conf
                .get(DEGREE_CONFIG_FIELD)
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            generated_config.get_degree()
        );
        assert_eq!(
            gossip_conf.get(BOOTSTRAPPER_CONFIG_FIELD).unwrap(),
            generated_config.get_bootstrapper().unwrap()
        );
        assert_eq!(
            gossip_conf
                .get(P2P_ADDRESS_CONFIG_FIELD)
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap(),
            generated_config.get_p2p_address()
        );
        assert_eq!(
            gossip_conf
                .get(API_ADDRESS_CONFIG_FIELD)
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap(),
            generated_config.get_api_address()
        );
        assert_eq!(
            ini_conf
                .section(Some(RPS_SECTION_NAME))
                .unwrap()
                .get(RPS_ADDRESS_CONFIG_FIELD)
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap(),
            generated_config.get_rps_address()
        )
    }
}
