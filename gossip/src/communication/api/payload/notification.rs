use std::io::Cursor;
use std::mem;
use bytes::Bytes;
use crate::communication::api::parse::{Error, get_size, get_u16};

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// random number to identify this message and its corresponding response
    pub(crate) message_id: u16,
    pub data_type: u16,
    pub(crate) data: Bytes,
}

impl Notification {
    pub fn pack(self) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(self.message_id.to_be_bytes());
        bytes.extend(self.data_type.to_be_bytes());
        bytes.extend(self.data.to_vec());
        bytes
    }

    pub fn get_size(&self) -> u16 {
        (mem::size_of_val(&self.message_id) +
            mem::size_of_val(&self.message_id) +
            self.data.len()) as u16
    }

    pub fn parse(src: &mut Cursor<&[u8]>, size: u16) -> Result<Self, Error> {
        let message_id = get_u16(src)?;
        let data_type = get_u16(src)?;
        let bytes = get_size(src, (size - 4) as usize)?;
        Ok(Self {
            message_id,
            data_type,
            data: bytes,
        })
    }
}
