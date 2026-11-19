//! Browser-free client for the version-pinned official Dominion server protocol.
pub mod catalog;
mod knowledge;
pub mod model;
pub mod questions;
pub mod record;
pub mod reducer;
pub mod server;
pub mod transport;
pub mod wire;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("unsupported official-server feature: {0}")]
    Unsupported(String),
    #[error("REJECTED: {0}")]
    Rejected(String),
    #[error("official-server connection: {0}")]
    Connection(String),
}
