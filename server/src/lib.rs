//! Shared trusted career persistence. HTTP callers must authorize an actor before
//! invoking this library; match settlement remains owned by game processes.
pub mod career_store;

pub mod matchmaking;
