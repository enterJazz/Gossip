//! Module to manage API server connections
//!
//! Provides utility for reading and writing APIMessages over a TCP connection
use crate::communication::api::message::{self, ApiMessage};

use bytes::{Buf, BytesMut};
use log::{debug, error, info};
use std::io::{self, Cursor, ErrorKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;

/// Send and receive `api messages` from submodules.
///
/// The purpose of `Connection` is to read and write messages on the underlying `TcpStream`.
///
/// To read messages, the `Connection` uses an internal buffer, which is filled
/// up until there are enough bytes to create a full message. Once this happens,
/// the `Connection` creates the message and returns it to the caller.
///
/// When sending messages, the message is first encoded into the write buffer.
/// The contents of the write buffer are then written to the socket.
#[derive(Debug)]
pub struct Connection {
    // The `TcpStream`. It is decorated with a `BufWriter`, which provides write
    // level buffering. The `BufWriter` implementation provided by Tokio is
    // sufficient for our needs.
    stream: BufWriter<TcpStream>,

    // The buffer for reading messages.
    buffer: BytesMut,
}

impl Connection {
    /// Create a new `Connection`, backed by `socket`. Read and write buffers
    /// are initialized.
    pub fn new(socket: TcpStream) -> Connection {
        Connection {
            stream: BufWriter::new(socket),
            buffer: BytesMut::with_capacity(4 * 1024),
        }
    }

    /// Read a single `message` value from the underlying stream.
    ///
    /// The function waits until it has retrieved enough data to parse a message.
    /// Any data remaining in the read buffer after the message has been parsed is
    /// kept there for the next call to `read_message`.
    ///
    /// # Returns
    ///
    /// On success, the received message is returned. If the `TcpStream`
    /// is closed in a way that doesn't break a message in half, it returns
    /// `None`. Otherwise, an error is returned.
    pub async fn read_message(&mut self) -> super::Result<Option<ApiMessage>> {
        debug!("CONNECTION: reading message...");
        loop {
            // Attempt to parse a message from the buffered data. If enough data
            // has been buffered, the message is returned.
            if let Some(message) = self.parse_message()? {
                debug!("CONNECTION: read message {:?}", &message);
                return Ok(Some(message));
            }

            // There is not enough buffered data to read a message. Attempt to
            // read more data from the socket.
            //
            // On success, the number of bytes is returned. `0` indicates "end
            // of stream".
            if 0 == self.stream.read_buf(&mut self.buffer).await? {
                // The remote closed the connection. For this to be a clean
                // shutdown, there should be no data in the read buffer. If
                // there is, this means that the peer closed the socket while
                // sending a message.
                if self.buffer.is_empty() {
                    info!("CONNECTION: closed");
                    return Ok(None);
                } else {
                    debug!("CONNECTION: reset by peer");
                    return Err("connection reset by peer".into());
                }
            }
        }
    }

    /// Tries to parse a message from the buffer. If the buffer contains enough
    /// data, the message is returned and the data removed from the buffer. If not
    /// enough data has been buffered yet, `Ok(None)` is returned. If the
    /// buffered data does not represent a valid message, `Err` is returned.
    fn parse_message(&mut self) -> super::Result<Option<ApiMessage>> {
        debug!("CONNECTION: parsing message...");
        // Cursor is used to track the "current" location in the
        // buffer. Cursor also implements `Buf` from the `bytes` crate
        // which provides a number of helpful utilities for working
        // with bytes.
        let mut buf = Cursor::new(&self.buffer[..]);

        // The first step is to check if enough data has been buffered to parse
        // a single message. This step is usually much faster than doing a full
        // parse of the message, and allows us to skip allocating data structures
        // to hold the message data unless we know the full message has been
        // received.
        match ApiMessage::check(&mut buf) {
            Ok(_) => {
                debug!("CONNECTION: message check successful, parsing message");
                // The `check` function will have advanced the cursor until the
                // end of the message. Since the cursor had position set to zero
                // before `ApiMessage::check` was called, we obtain the length of the
                // message by checking the cursor position.
                let len = buf.position() as usize;

                // Reset the position to zero before passing the cursor to
                // `ApiMessage::parse`.
                buf.set_position(0);

                // Parse the message from the buffer. This allocates the necessary
                // structures to represent the message and returns the message
                // value.
                //
                // If the encoded message representation is invalid, an error is
                // returned. This should terminate the **current** connection
                // but should not impact any other connected client.
                let message = ApiMessage::parse(&mut buf)?;

                // Discard the parsed data from the read buffer.
                //
                // When `advance` is called on the read buffer, all of the data
                // up to `len` is discarded. The details of how this works is
                // left to `BytesMut`. This is often done by moving an internal
                // cursor, but it may be done by reallocating and copying data.
                self.buffer.advance(len);

                // Return the parsed message to the caller.
                Ok(Some(message))
            }
            // There is not enough data present in the read buffer to parse a
            // single message. We must wait for more data to be received from the
            // socket. Reading from the socket will be done in the statement
            // after this `match`.
            //
            // We do not want to return `Err` from here as this "error" is an
            // expected runtime condition.
            Err(_) => {
                debug!("CONNECTION: not enough data present to parse message");
                Ok(None)
            }
        }
    }

    /// Write a single `ApiMessage` value to the underlying stream.
    pub async fn write_message(&mut self, payload: ApiMessage) -> io::Result<()> {
        // create the header
        let header = message::Header::new(&payload)
            .map_err(|e| io::Error::new(ErrorKind::Unsupported, e))?;

        // write in buffer
        self.stream.write_all(header.pack().as_slice()).await?;
        self.stream
            .write_all(
                payload
                    .pack()
                    .map_err(|e| io::Error::new(ErrorKind::Unsupported, e))?
                    .as_slice(),
            )
            .await?;

        self.stream.flush().await
    }
}
