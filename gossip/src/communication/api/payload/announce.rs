use crate::communication::api::parse::{get_size, get_u16, get_u8, skip, Error};
use bytes::Bytes;
use std::io::Cursor;

/// size of fixed length fields of announce message
const ANNOUNCE_PREAMBLE_SIZE: u16 = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Announce {
    pub ttl: u8,
    pub data_type: u16,
    pub data: Bytes,
}

impl Announce {
    pub fn parse(src: &mut Cursor<&[u8]>, size: u16) -> Result<Announce, Error> {
        let ttl = get_u8(src)?;
        // reserved
        skip(src, 1)?;
        let data_type = get_u16(src)?;
        let data_size = size - ANNOUNCE_PREAMBLE_SIZE;
        let data = get_size(src, data_size as usize)?;
        Ok(Announce {
            ttl,
            data_type,
            data,
        })
    }
}
