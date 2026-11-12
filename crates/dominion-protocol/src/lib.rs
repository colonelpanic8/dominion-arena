//! The serde types `dominion-server` speaks: agent-view@2 ([`view`], re-exported here),
//! its own requests and responses ([`api`], [`lobby`]) and the asset cache's manifest ([`assets`]).

pub mod api;
pub mod assets;
pub mod lobby;
pub mod view;

pub use view::*;
