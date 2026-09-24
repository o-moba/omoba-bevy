//! Client-side game model shared by `net`, gameplay and presentation.
//!
//! Only the types several modules agree on live here: the round identity, the
//! team, the combat pools and the hero markers with their movement intents.
//! The old paths (`crate::team::Team`, `crate::combat::CombatStats`,
//! `crate::player::Player`, `crate::net::RemotePlayer`, ...) re-export these,
//! so callers can move over gradually.

mod actors;
mod round;
mod stats;
mod team;

pub(crate) use actors::{MovementRoute, MovementTarget};
pub use actors::{Player, PlayerBody, RemotePlayer, VerticalVelocity};
pub use round::RoundId;
pub use stats::{CombatStats, MAX_HP};
pub use team::Team;
