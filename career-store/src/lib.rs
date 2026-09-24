//! Shared trusted career persistence and the bounded queue policy.
//!
//! The game server settles matches through this crate and the account API
//! serves profiles from it; neither the Bevy engine nor the game simulation is
//! needed to link it. HTTP callers must authorize an actor before invoking
//! the store; match settlement remains owned by game processes.
pub mod career_store;
pub mod matchmaking;
