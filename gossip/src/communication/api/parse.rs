//! utility module to read and parse data from a connection
use bytes::{Buf, Bytes};
use log::debug;
use std::fmt::{Debug, Display};
use std::io::Cursor;
use thiserror::Error;

/// errors returned by the parse module
#[derive(Error, Debug)]
pub enum Error {
    /// indicates that not enough data is available on given buffer to complety parse message
    #[error("not enough data available to parse message")]
    Incomplete,

    /// indicates that a message contains an unexpected value
    #[error("unknown value for field {}: {}", field_name, value)]
    Unknown { field_name: String, value: String },
}

/// reads a u8 (single byte) from the connection
pub fn get_u8(src: &mut Cursor<&[u8]>) -> Result<u8, Error> {
    if !src.has_remaining() {
        return Err(Error::Incomplete);
    }

    Ok(src.get_u8())
}

/// reads a u16 (2 bytes) from the connection
pub fn get_u16(src: &mut Cursor<&[u8]>) -> Result<u16, Error> {
    if !(src.remaining() >= 2) {
        debug!("PARSE: not enough bytes for skip for 2");
        return Err(Error::Incomplete);
    }

    Ok(src.get_u16())
}

/// reads a variable length number of bytes from the connection. required for variable length payloads.
pub fn get_size(src: &mut Cursor<&[u8]>, size: usize) -> Result<Bytes, Error> {
    if src.remaining() < size {
        return Err(Error::Incomplete);
    }

    let data = Bytes::copy_from_slice(&src.chunk()[..size]);
    skip(src, size as usize)?;
    Ok(data)
}

/// skips over a variable length of bytes. required to skip over reserved fields
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
