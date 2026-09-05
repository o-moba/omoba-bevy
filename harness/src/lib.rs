//! Headless gameplay test harness for the omoba authoritative server.
//!
//! This crate spins up the **real** UDP server as a child process and drives it
//! with scripted bot clients that speak the server's JSON wire protocol. It
//! asserts true gameplay rules (god mode, movement clamps, skill-point gating)
//! without a GPU, a renderer, or a human in the loop. It supersedes the
//! ad-hoc Python flow in `scripts/verify_task_02_multiplayer_session_flow.py`
//! for Rust-typed gameplay-rule coverage.
//!
//! # Layers
//! - [`protocol`] — a typed, minimal **test mirror** of the server's UDP wire
//!   format. Must be kept in sync with `server/src/main.rs`.
//! - [`server`] — [`ServerProcess`], an RAII handle that launches the server on
//!   a unique loopback port and kills it on drop.
//! - [`bot`] — [`Bot`], a connected client with typed packet senders and
//!   snapshot-polling helpers.
//!
//! # Adding a scenario
//! Integration scenarios live in `tests/gameplay.rs`. To add one:
//! 1. Create a fresh [`ServerProcess`] (its own port; cleaned up on drop).
//! 2. Connect one or more [`Bot`]s with [`Bot::connect`] and `join` them.
//! 3. Drive the bots with the typed helpers and poll snapshots with
//!    [`Bot::wait_for_player`] / [`Bot::recv_snapshot`] using timeouts (never
//!    fixed sleeps for assertions) so the scenario stays deterministic.
//! 4. Assert on the returned [`protocol::PlayerState`].
//!
//! Keep scenarios few and rock-solid. Rules that need real long-form leveling,
//! pixel-perfect rendering, or UI interaction belong in server unit tests or a
//! human QA pass, respectively.

pub mod bot;
pub mod bot_ai;
pub mod protocol;
pub mod server;

pub use bot::Bot;
pub use protocol::{
    Character, ClientPacket, GameState, HeroClass, NeutralCampType, NeutralState, PlayerActionKind,
    PlayerState, ServerPacket, TargetId, TargetKind, Team, TeamBuffKind, TeamBuffState,
};
pub use server::ServerProcess;
