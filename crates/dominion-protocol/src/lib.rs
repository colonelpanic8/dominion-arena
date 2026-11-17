//! The serde types `dominion-server` speaks: agent-view@2 ([`view`], re-exported here),
//! events@1 ([`events`]), its own requests and responses ([`api`], [`lobby`]) and the asset
//! cache's manifest ([`assets`]).

pub mod api;
pub mod assets;
pub mod contract;
pub mod events;
pub mod lobby;
pub mod transcript;
pub mod view;

pub use view::*;
