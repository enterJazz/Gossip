//! Represents the notify message
//! 
//! Only supports unmarshaling this message
use crate::communication::api::parse::{get_u16, skip, Error};
use std::io::Cursor;

#[derive(Debug, Clone, PartialEq)]
pub struct Notify {
    pub data_type: u16,
}

impl Notify {
    pub fn parse(src: &mut Cursor<&[u8]>, size: u16) -> Result<Notify, Error> {
        // reserved
        skip(src, 2)?;
        let data_type = get_u16(src)?;
        Ok(Notify { data_type })
    }
}
