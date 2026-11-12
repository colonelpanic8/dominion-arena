//! The serde types `dominion-server` speaks: agent-view@2 ([`view`], re-exported here)
//! and its own requests and responses ([`api`]).

pub mod api;
pub mod view;

pub use view::*;
