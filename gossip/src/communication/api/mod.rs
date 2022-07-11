mod connection;
pub mod message;
mod parse;
pub mod payload;
pub(crate) mod server;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
