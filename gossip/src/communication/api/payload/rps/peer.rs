use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use bytes::Bytes;
use log::{debug, warn};
use num_traits::FromPrimitive;
use num_derive::FromPrimitive;
use crate::communication::api::parse;
use crate::communication::api::parse::{Error, get_size, get_u16, get_u8, skip};

#[derive(Debug, Clone, PartialEq, FromPrimitive)]
pub enum Module {
    Gossip = 500,
    NSE = 520,
    Onion = 560,
    DHT = 650,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub(crate) address: std::net::SocketAddr,
    pub(crate) port_map_records: Vec<PortMapRecord>,
    pub(crate) host_key: Bytes,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortMapRecord {
    pub(crate) module: Module,
    pub(crate) port: u16,
}

impl Peer {
    pub fn parse(src: &mut Cursor<&[u8]>, size: u16) -> Result<Self, parse::Error> {
        let port = get_u16(src)?;
        let no_port_map_records = get_u8(src)?;
        let ip_addr_version = match get_u8(src)? & 0b00000001 {
            0 => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            1 => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            _ => panic!("unexpected"),
        };

        // parse port maps
        let mut port_map_records: Vec<PortMapRecord> = vec![];
        for _ in 0..no_port_map_records {

            let module_code = get_u16(src)?;
            let module: Module =
                FromPrimitive::from_u16(module_code).ok_or(Error::Unknown { field_name: "module code".to_string(), value: module_code.to_string() })?;
            let module_port = get_u16(src)?;

            port_map_records.push(
                PortMapRecord {
                    module,
                    port: module_port,
                }
            )
        }

        // parse IP addr
        let ip_addr: IpAddr = match &ip_addr_version {
            IpAddr::V4(_) => {
                let mut data = [0u8; 4];
                data.copy_from_slice(&get_size(src, 4)?.to_vec()[0..4]);
                IpAddr::V4(Ipv4Addr::from(data))
            }
            IpAddr::V6(_) => {
                let mut data = [0u8; 16];
                data.copy_from_slice(&get_size(src, 16)?.to_vec()[0..8]);
                IpAddr::V6(Ipv6Addr::from(data))
            }
        };

        let address = SocketAddr::new(ip_addr, port);

        // determine size of peer's host key
        // size -
        // header = 2
        // port + portmap + reserved + V = 4
        // no_port_maps * 4
        // IpAddr: 4 OR 16
        let host_key_size = match &ip_addr_version {
            IpAddr::V4(_) => {
                (size as usize) - (&port_map_records.len() * 4 + 10)
            }
            IpAddr::V6(_) => {
                (size as usize) - (&port_map_records.len() * 4 + 22)
            }
        };

        let host_key = get_size(src, host_key_size)?;

        Ok(
            Self {
                address,
                port_map_records,
                host_key,
            }
        )
    }
}
