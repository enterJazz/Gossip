use bytes::{Buf, Bytes};
use log::debug;
use std::fmt::{Debug, Display};
use std::io::Cursor;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("not enough data available to parse message")]
    Incomplete,

    #[error("unknown value for field {}: {}", field_name, value)]
    Unknown { field_name: String, value: String },
}

pub fn get_u8(src: &mut Cursor<&[u8]>) -> Result<u8, Error> {
    if !src.has_remaining() {
        return Err(Error::Incomplete);
    }

    Ok(src.get_u8())
}

pub fn get_u16(src: &mut Cursor<&[u8]>) -> Result<u16, Error> {
    if !(src.remaining() >= 2) {
        debug!("PARSE: not enough bytes for skip for 2");
        return Err(Error::Incomplete);
    }

    Ok(src.get_u16())
}

pub fn get_size(src: &mut Cursor<&[u8]>, size: usize) -> Result<Bytes, Error> {
    if src.remaining() < size {
        return Err(Error::Incomplete);
    }

    let data = Bytes::copy_from_slice(&src.chunk()[..size]);
    skip(src, size as usize)?;
    Ok(data)
}

pub fn skip(src: &mut Cursor<&[u8]>, n: usize) -> Result<(), Error> {
    if src.remaining() < n {
        debug!(
            "PARSE: not enough bytes for skip {n}; remaining: {}",
            src.remaining()
        );
        return Err(Error::Incomplete);
    }

    src.advance(n);
    Ok(())
}
