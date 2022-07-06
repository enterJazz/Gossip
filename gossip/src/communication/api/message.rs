//! Provides a type representing an API Gossip protocol message as well as utilities for
//! parsing messages from a byte array.

use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use std::io::Cursor;
use std::mem;

use crate::communication::api::message::Error::Unexpected;
use crate::communication::api::parse::{self, get_u16, skip};
use thiserror::Error;

use super::payload::{
    announce::Announce, notification::Notification, notify::Notify, validation::Validation,
};

#[derive(Error, Debug)]
pub enum Error {
    #[error("unknown message type - received message type id {message_code}")]
    Unknown { message_code: u16 },

    #[error("unexpected message type: {:?}", message_type)]
    Unexpected { message_type: MessageType },

    #[error(transparent)]
    ParseError(#[from] parse::Error),
}

#[derive(Debug, FromPrimitive, Copy, Clone, PartialEq)]
pub enum MessageType {
    GossipAnnounce = 500,
    GossipNotify = 501,
    GossipNotification = 502,
    GossipValidation = 503,
}

/// An API Gossip message.
#[derive(Clone, Debug, PartialEq)]
pub enum ApiMessage {
    Announce(Announce),
    Notification(Notification),
    Notify(Notify),
    Validation(Validation),
}

#[derive(Clone, Debug)]
pub struct Header {
    size: u16,
    message_type: MessageType,
}

impl ApiMessage {
    /// Parses an Api message from the underlying stream
    pub fn parse(src: &mut Cursor<&[u8]>) -> Result<ApiMessage, Error> {
        let header = Header::parse(src)?;
        Ok(match header.message_type {
            MessageType::GossipAnnounce => ApiMessage::Announce(Announce::parse(src, header.size)?),
            MessageType::GossipNotify => ApiMessage::Notify(Notify::parse(src, header.size)?),
            MessageType::GossipValidation => {
                ApiMessage::Validation(Validation::parse(src, header.size)?)
            }
            MessageType::GossipNotification => {
                ApiMessage::Notification(Notification::parse(src, header.size)?)
            }
        })
    }

    /// Serializes a message for transmission over the wire.
    pub fn pack(self) -> Result<Vec<u8>, Error> {
        match self {
            ApiMessage::Notification(payload) => Ok(payload.pack()),
            ApiMessage::Announce(_) => Err(Error::Unexpected {
                message_type: MessageType::GossipAnnounce,
            }),
            ApiMessage::Notify(_) => Err(Error::Unexpected {
                message_type: MessageType::GossipNotify,
            }),
            ApiMessage::Validation(_) => Err(Error::Unexpected {
                message_type: MessageType::GossipValidation,
            }),
        }
    }

    /// Checks if an entire message can be decoded from `src`
    pub fn check(src: &mut Cursor<&[u8]>) -> Result<(), Error> {
        let size = get_u16(src)?;
        skip(src, size as usize)?;
        Ok(())
    }

    pub fn get_size(&self) -> Result<u16, Error> {
        match self {
            ApiMessage::Notification(n) => {
                Ok(n.get_size())
            }
            ApiMessage::Announce(_) => {
                Err(Unexpected { message_type: MessageType::GossipAnnounce })
            }
            ApiMessage::Notify(_) => {
                Err(Unexpected { message_type: MessageType::GossipNotify })
            }
            ApiMessage::Validation(_) => {
                Err(Unexpected { message_type: MessageType::GossipValidation })
            }
        }
    }
}

impl Header {
    pub fn new(payload: &ApiMessage) -> Result<Header, Error> {
        // size is always payload + header without size (2)
        let header_len = 2;
        let size: u16 = payload.get_size()?;
        // only Notification is sent by Gossip to other modules
        // so no other headers should be created
        let message_type = match payload {
            ApiMessage::Notification(_) => MessageType::GossipNotification,
            ApiMessage::Announce(_) => {
                return Err(Error::Unexpected {
                    message_type: MessageType::GossipAnnounce,
                })
            }
            ApiMessage::Notify(_) => {
                return Err(Error::Unexpected {
                    message_type: MessageType::GossipNotify,
                })
            }
            ApiMessage::Validation(_) => {
                return Err(Error::Unexpected {
                    message_type: MessageType::GossipValidation,
                })
            }
        };

        Ok(Header { size, message_type })
    }

    pub fn pack(self) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(self.size.to_be_bytes());
        bytes.extend((self.message_type as u16).to_be_bytes());
        bytes
    }

    fn parse(src: &mut Cursor<&[u8]>) -> Result<Header, Error> {
        let size = get_u16(src)?;
        let message_code = get_u16(src)?;
        let message_type: MessageType =
            FromPrimitive::from_u16(message_code).ok_or(Error::Unknown { message_code })?;

        Ok(Header { size, message_type })
    }
}
