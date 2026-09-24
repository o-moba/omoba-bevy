//! Client debug tooling: the development toggles and the practice sandbox
//! page, sent as [`shared::debug::DebugCommand`]s.
//!
//! - [`DebugToggles`] is the one client copy of god mode and the speed boost.
//!   The HUD buttons and hotkeys (`hud`), the practice page (`tools_page`),
//!   local movement (`player`) and the local-player snapshot stage (`net`)
//!   all read it.
//! - `hud` holds the one re-send system ([`resend_debug_toggles`]): the toggles
//!   are re-sent every 0.5 s, only while the HUD controls are enabled
//!   (`OMOBA_DEBUG_UI`, not in Combat Test).
//! - `console` is the on-screen log (`OMOBA_DEBUG_UI`); it sends nothing.
//!
//! The Combat Test panel (`crate::sandbox`) is a separate protocol and stays
//! outside this module.

pub(crate) mod console;
pub(crate) mod hud;
pub(crate) mod tools_page;

use bevy::prelude::*;
use shared::debug::DebugCommand;

use crate::net::{ClientSession, NetworkCommand};

pub use console::DebugConsole;
pub(crate) use console::DebugConsolePlugin;
pub(crate) use hud::GodModePlugin;
pub(crate) use tools_page::PracticeSandboxPlugin;

/// Last requested god mode and speed boost. The server owns the real flags;
/// this is what the client asks for (and re-sends while the HUD is enabled).
/// The speed boost also makes local movement faster and widens the snapshot
/// snap threshold. Leaving a practice match clears `god_mode`
/// (`tools_page`), because the next match starts without it on the server.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugToggles {
    pub god_mode: bool,
    pub speed_boost: bool,
}

impl DebugToggles {
    /// Both toggles as commands, god mode first (the re-send order).
    pub(crate) fn commands(self) -> [DebugCommand; 2] {
        [
            DebugCommand::GodMode(self.god_mode),
            DebugCommand::SpeedBoost(self.speed_boost),
        ]
    }
}

/// Continuously re-assert the current debug toggle state to the server (~2x/sec).
/// A single edge-triggered send can be dropped (UDP, connection races, or a fresh
/// server session resetting the flags); periodic idempotent re-assertion guarantees
/// the server's `god_mode`/`speed_mult` eventually match the local toggles.
pub(crate) fn resend_debug_toggles(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    toggles: Res<DebugToggles>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    if !client_session.is_connected() {
        return;
    }
    *elapsed += time.delta_secs();
    if *elapsed < 0.5 {
        return;
    }
    *elapsed = 0.0;
    for command in toggles.commands() {
        command_writer.write(NetworkCommand::Debug(command));
    }
}
