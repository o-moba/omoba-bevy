//! Client debug tooling: the development toggles and the practice sandbox
//! page, sent as [`shared::debug::DebugCommand`]s.
//!
//! - [`ClientDebugAccess`] is what the tools may do right now (step 11e): the
//!   server's `Snapshot.debug_access` when it sends one (step 11f), else
//!   `DebugAccess::for_match_mode(match_mode)` for an older server; nothing
//!   until the join is confirmed. In Combat Test the sandbox actor config owns
//!   god mode and the speed boost, so the toggles are not offered there.
//! - [`DebugToggles`] is the one client copy of god mode and the speed boost.
//!   The HUD buttons and hotkeys (`hud`), the tools page (`tools_page`),
//!   local movement (`player`) and the local-player snapshot stage (`net`)
//!   all read it. While the toggles are not allowed it is held at "both off"
//!   ([`sync_debug_access`]), so leaving practice or joining a worker round
//!   drops them.
//! - [`resend_debug_toggles`] re-sends the toggles every 0.5 s wherever they
//!   are allowed (before 11e: only with `OMOBA_DEBUG_UI`).
//! - `OMOBA_DEBUG_UI` still enables the extras: the bottom-left HUD buttons
//!   and F2/F3 hotkeys (shown only where the toggles are allowed), the
//!   on-screen log (`console`, which sends nothing) and F8 debug flight.
//!
//! The Combat Test panel (`crate::sandbox`) is a separate protocol and stays
//! outside this module; the tools page only links to it.

pub(crate) mod console;
pub(crate) mod hud;
pub(crate) mod tools_page;

use bevy::prelude::*;
use shared::debug::{DebugAccess, DebugCommand};

use crate::net::{ClientSession, GameStateSnapshot, NetworkCommand};

pub use console::DebugConsole;
pub(crate) use console::DebugConsolePlugin;
pub(crate) use hud::GodModePlugin;
pub(crate) use tools_page::PracticeSandboxPlugin;

/// Last requested god mode and speed boost. The server owns the real flags;
/// this is what the client asks for (and re-sends while the toggles are
/// allowed). The speed boost also makes local movement faster and widens the
/// snapshot snap threshold. Both reset to off whenever the toggles are not
/// allowed ([`sync_debug_access`]): the next match starts without them on the
/// server too.
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

/// Debug access as the client applies it; updated every frame by
/// [`sync_debug_access`] in [`DebugAccessSet`].
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientDebugAccess {
    /// What the server accepts from us (see [`server_debug_access`]).
    pub server: DebugAccess,
    /// A Combat Test session: the sandbox owns the toggles.
    pub combat_test: bool,
}

impl ClientDebugAccess {
    /// God mode and the speed boost may be shown, sent and re-sent.
    pub fn toggles(self) -> bool {
        self.server.toggles && !self.combat_test
    }

    /// The practice bot commands may be shown and sent.
    pub fn practice(self) -> bool {
        self.server.practice
    }

    /// The tools page has something to show.
    pub fn any(self) -> bool {
        self.toggles() || self.practice() || self.combat_test
    }
}

/// What the server accepts from this client: nothing before the join is
/// confirmed; then the snapshot's `debug_access` (servers since step 11f),
/// else the table for its `match_mode` (older servers, which cannot tell a
/// worker round apart; they refuse the commands there on their own).
pub(crate) fn server_debug_access(
    snapshot: Option<&GameStateSnapshot>,
    session: &ClientSession,
) -> DebugAccess {
    match snapshot {
        Some(snapshot) if session.join_confirmed() => snapshot
            .debug_access
            .unwrap_or_else(|| DebugAccess::for_match_mode(&snapshot.match_mode)),
        _ => DebugAccess::default(),
    }
}

/// Runs before every system that reads [`ClientDebugAccess`].
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DebugAccessSet;

/// The access state and the periodic re-send; part of `DebugPlugins`.
pub(crate) struct DebugAccessPlugin;

impl Plugin for DebugAccessPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientDebugAccess>()
            .init_resource::<DebugToggles>()
            .add_systems(
                Update,
                (
                    sync_debug_access,
                    resend_debug_toggles.run_if(debug_toggles_allowed),
                )
                    .chain()
                    .in_set(DebugAccessSet),
            );
    }
}

pub(crate) fn debug_toggles_allowed(access: Option<Res<ClientDebugAccess>>) -> bool {
    access.is_some_and(|access| access.toggles())
}

/// Recomputes [`ClientDebugAccess`] and holds [`DebugToggles`] at "both off"
/// while the toggles are not allowed (not joined yet, a release match, a
/// worker round, Combat Test), so nothing requested in one match leaks into
/// the next or keeps local movement boosted.
pub(crate) fn sync_debug_access(
    snapshot: Option<Res<GameStateSnapshot>>,
    session: Res<ClientSession>,
    mut access: ResMut<ClientDebugAccess>,
    mut toggles: ResMut<DebugToggles>,
) {
    let next = ClientDebugAccess {
        server: server_debug_access(snapshot.as_deref(), &session),
        combat_test: crate::sandbox::requested()
            || snapshot.as_ref().is_some_and(|s| s.sandbox.is_some()),
    };
    if *access != next {
        *access = next;
    }
    if !next.toggles() && *toggles != DebugToggles::default() {
        info!("[debug] toggles not allowed here; god mode and speed boost off");
        *toggles = DebugToggles::default();
    }
}

/// Continuously re-assert the current debug toggle state to the server (~2x/sec).
/// A single edge-triggered send can be dropped (UDP, connection races, or a fresh
/// server session resetting the flags); periodic idempotent re-assertion guarantees
/// the server's `god_mode`/`speed_mult` eventually match the local toggles.
/// Runs only while [`ClientDebugAccess::toggles`] allows them.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(350),
            ))
            .init_resource::<ClientSession>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_plugins(DebugAccessPlugin);
        app
    }

    fn set(app: &mut App, mode: &str, access: Option<DebugAccess>) {
        let mut snapshot = app.world_mut().resource_mut::<GameStateSnapshot>();
        snapshot.match_mode = mode.into();
        snapshot.debug_access = access;
    }

    fn access(app: &App) -> ClientDebugAccess {
        *app.world().resource::<ClientDebugAccess>()
    }

    fn toggles(app: &App) -> DebugToggles {
        *app.world().resource::<DebugToggles>()
    }

    const ON: DebugToggles = DebugToggles {
        god_mode: true,
        speed_boost: true,
    };

    #[test]
    fn access_prefers_the_server_value_and_falls_back_to_the_match_mode() {
        let mut app = app();
        set(&mut app, "practice", None);
        app.update();
        assert_eq!(access(&app), ClientDebugAccess::default(), "not joined");

        app.insert_resource(ClientSession::admitted_for_test());
        app.update();
        let all = DebugAccess {
            toggles: true,
            practice: true,
        };
        assert_eq!(access(&app).server, all, "old server: the mode table");

        // A new server in a worker round: "practice", but nothing allowed.
        set(&mut app, "practice", Some(DebugAccess::default()));
        app.update();
        assert!(!access(&app).any());

        set(
            &mut app,
            "dev",
            Some(DebugAccess {
                toggles: true,
                practice: false,
            }),
        );
        app.update();
        assert!(access(&app).toggles() && !access(&app).practice());
        set(&mut app, "dev", None);
        app.update();
        assert!(access(&app).toggles() && !access(&app).practice());

        // Combat Test: the sandbox owns the toggles.
        app.world_mut().resource_mut::<GameStateSnapshot>().sandbox =
            Some(shared::sandbox::SandboxSnapshot {
                config: Default::default(),
                ack: None,
                last_request_id: 0,
                actors: vec![],
                analytics: Default::default(),
                simulation_secs: 0.0,
                frame: 1,
            });
        app.update();
        assert!(access(&app).combat_test && !access(&app).toggles() && access(&app).any());
    }

    /// Step 11e: the toggles reset to off locally when access drops, both on
    /// leaving practice and on joining a worker round; a dev match keeps them.
    #[test]
    fn toggles_reset_when_access_drops() {
        let mut app = app();
        app.insert_resource(ClientSession::admitted_for_test());
        set(&mut app, "dev", None);
        app.update();
        app.insert_resource(ON);
        app.update();
        app.update();
        assert_eq!(toggles(&app), ON, "dev keeps them");

        set(&mut app, "practice", None);
        app.update();
        assert_eq!(toggles(&app), ON, "practice keeps them");
        set(&mut app, "release", None);
        app.update();
        assert_eq!(toggles(&app), DebugToggles::default(), "leaving practice");

        set(&mut app, "practice", None);
        app.update();
        app.insert_resource(ON);
        app.update();
        assert_eq!(toggles(&app), ON);
        set(&mut app, "practice", Some(DebugAccess::default()));
        app.update();
        assert_eq!(toggles(&app), DebugToggles::default(), "worker round");

        // Held off while not allowed: a stray write does not survive a frame.
        app.insert_resource(ON);
        app.update();
        assert_eq!(toggles(&app), DebugToggles::default());
    }

    /// The re-send runs wherever the toggles are allowed, without
    /// `OMOBA_DEBUG_UI`, and not elsewhere.
    #[test]
    fn toggles_are_re_sent_only_where_allowed() {
        let mut app = app();
        app.insert_resource(ClientSession::admitted_for_test());
        let sent = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .collect::<Vec<_>>()
        };
        let run_one_second = |app: &mut App| {
            for _ in 0..3 {
                app.update();
            }
        };
        set(&mut app, "release", None);
        app.update();
        run_one_second(&mut app);
        assert!(sent(&mut app).is_empty(), "release");

        set(&mut app, "practice", None);
        app.update();
        app.world_mut().resource_mut::<DebugToggles>().god_mode = true;
        run_one_second(&mut app);
        let commands = sent(&mut app);
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, NetworkCommand::Debug(DebugCommand::GodMode(true)))),
            "{commands:?}"
        );
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, NetworkCommand::Debug(DebugCommand::SpeedBoost(false))))
        );
    }
}
