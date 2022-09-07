use ini::Ini;
use log::error;
use std::{collections::HashMap, fs, io, net::SocketAddr, path::PathBuf};
use thiserror::Error;
use url::Url;

const GOSSIP_SECTION_NAME: &str = "gossip";

const HOST_KEY_PATH_FIELD: &str = "hostkey";
const CACHE_SIZE_CONFIG_FIELD: &str = "cache_size";
const DEGREE_CONFIG_FIELD: &str = "degree";
const BOOTSTRAPPER_CONFIG_FIELD: &str = "bootstrapper";
const P2P_ADDRESS_CONFIG_FIELD: &str = "p2p_address";
const API_ADDRESS_CONFIG_FIELD: &str = "api_address";

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
    #[error("Failed to open pem file: {0}")]
    PemOpenError(#[from] io::Error),
}

/// Configuration loaded from ini file
pub struct Config {
    host_key_path: PathBuf,
    host_priv_key_pem: pem::Pem,
    host_pub_key_pem: pem::Pem,

    cache_size: usize,
    degree: usize,
    bootstrapper: Url,
    p2p_address: SocketAddr,
    api_address: SocketAddr,
}

/// Opens and parses a pem file at location path
fn parse_pem(path: PathBuf) -> Result<pem::Pem, ConfigError> {
    let file = match fs::read_to_string(path.clone()) {
        Ok(f) => f,
        Err(err) => {
            println!(
                "failed to read pem at: {:?}",
                path.into_os_string().into_string()
            );
            return Err(ConfigError::PemOpenError(err));
        }
    };
    match pem::parse(file) {
        Ok(p) => Ok(p),
        Err(err) => Err(ConfigError::PemParseError(err)),
    }
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

        let mut host_pub_key_path = host_key_path.clone();
        host_pub_key_path.set_extension("pub");
        let pub_key = match parse_pem(host_pub_key_path) {
            Ok(key) => key,
            Err(err) => return Err(err),
        };

        let priv_key = match parse_pem(host_key_path.clone()) {
            Ok(key) => key,
            Err(err) => return Err(err),
        };

        let gossip_section =
            conf.section(Some(GOSSIP_SECTION_NAME))
                .ok_or(ConfigError::SectionMissing {
                    section_name: GOSSIP_SECTION_NAME.to_string(),
                })?;

        let parsed_usize_keys: Vec<String> = vec![
            CACHE_SIZE_CONFIG_FIELD.to_string(),
            DEGREE_CONFIG_FIELD.to_string(),
        ];
        let mut parsed_usize_fields: HashMap<String, usize> = HashMap::new();

        let parsed_url_keys: Vec<String> = vec![BOOTSTRAPPER_CONFIG_FIELD.to_string()];
        let mut parsed_url_fields: HashMap<String, Url> = HashMap::new();

        let parsed_address_keys: Vec<String> = vec![
            P2P_ADDRESS_CONFIG_FIELD.to_string(),
            API_ADDRESS_CONFIG_FIELD.to_string(),
        ];
        let mut parsed_address_fields: HashMap<String, SocketAddr> = HashMap::new();

        for field_key in [
            parsed_usize_keys.clone(),
            parsed_url_keys.clone(),
            parsed_address_keys.clone(),
        ]
        .concat()
        {
            let field_value = gossip_section
                .get(&field_key)
                .ok_or(ConfigError::FieldMissing {
                    field_name: field_key.clone(),
                })?;

            if parsed_usize_keys.contains(&field_key) {
                let usize_parsed_value: usize =
                    field_value
                        .parse::<usize>()
                        .map_err(|e| ConfigError::ParsingError {
                            field_name: field_key.clone(),
                            err_msg: e.to_string(),
                        })?;
                let _ = parsed_usize_fields.insert(field_key.clone(), usize_parsed_value);
            } else if parsed_url_keys.contains(&field_key) {
                let url_parsed_value: Url =
                    field_value
                        .parse::<Url>()
                        .map_err(|e| ConfigError::ParsingError {
                            field_name: field_key.clone(),
                            err_msg: e.to_string(),
                        })?;
                let _ = parsed_url_fields.insert(field_key.clone(), url_parsed_value);
            } else if parsed_address_keys.contains(&field_key) {
                let socketaddr_parsed_value: SocketAddr = field_value
                    .parse::<SocketAddr>()
                    .map_err(|e| ConfigError::ParsingError {
                        field_name: field_key.clone(),
                        err_msg: e.to_string(),
                    })?;
                let _ = parsed_address_fields.insert(field_key.clone(), socketaddr_parsed_value);
            } else {
                panic!("invalid state: invalid config key");
            }
        }

        Ok(Self {
            host_key_path,
            host_priv_key_pem: priv_key,
            host_pub_key_pem: pub_key,
            cache_size: parsed_usize_fields[CACHE_SIZE_CONFIG_FIELD],
            degree: parsed_usize_fields[DEGREE_CONFIG_FIELD],
            bootstrapper: parsed_url_fields[BOOTSTRAPPER_CONFIG_FIELD].clone(),
            p2p_address: parsed_address_fields[P2P_ADDRESS_CONFIG_FIELD],
            api_address: parsed_address_fields[API_ADDRESS_CONFIG_FIELD],
        })
    }

    pub fn get_host_key_path(&self) -> &PathBuf {
        &self.host_key_path
    }

    pub fn get_host_pub_key(&self) -> bytes::Bytes {
        bytes::Bytes::from(self.host_pub_key_pem.contents.clone())
    }

    pub fn get_cache_size(&self) -> usize {
        self.cache_size
    }

    pub fn get_degree(&self) -> usize {
        self.degree
    }

    pub fn get_bootstrapper(&self) -> &Url {
        &self.bootstrapper
    }

    pub fn get_p2p_address(&self) -> SocketAddr {
        self.p2p_address
    }

    pub fn get_api_address(&self) -> SocketAddr {
        self.api_address
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Config, API_ADDRESS_CONFIG_FIELD, BOOTSTRAPPER_CONFIG_FIELD, CACHE_SIZE_CONFIG_FIELD,
        DEGREE_CONFIG_FIELD, GOSSIP_SECTION_NAME, HOST_KEY_PATH_FIELD, P2P_ADDRESS_CONFIG_FIELD,
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
            &gossip_conf
                .get(BOOTSTRAPPER_CONFIG_FIELD)
                .unwrap()
                .parse::<Url>()
                .unwrap(),
            generated_config.get_bootstrapper()
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
    }
}
