//! Front-end shell: the screens a player sees before and between matches.
//!
//! The client used to boot straight into the live map with the class/avatar
//! picker floating on top of it, so a player picked a hero inside a running
//! world. The shell introduces an explicit screen state machine:
//!
//! `Home -> HeroSelect -> Searching -> Loading -> InMatch -> PostMatch -> Home`
//!
//! Gameplay plugins keep running underneath; menu screens paint an opaque
//! full-screen root above them and [`crate::input_context`] treats every menu
//! screen as a modal so world input stays inert.

pub mod card;
pub mod collection;
pub mod home;
pub mod loading;
pub mod postmatch;
pub mod preview;
pub mod searching;
pub mod widgets;

use bevy::prelude::*;

use crate::net::{ClientSession, GameState, GameStateSnapshot};
use crate::player::Player;

/// The screen the player is looking at. Menus and the match are mutually
/// exclusive: exactly one screen is active at a time.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppScreen {
    /// Welcome screen: profile card, PLAY, navigation.
    #[default]
    Home,
    /// Profile card customization (main class, showcase avatar, accent).
    Card,
    /// Avatar collection with the 3D preview.
    Collection,
    /// Class + avatar + side pick. Locking in sends the join/queue packet.
    HeroSelect,
    /// Matchmaking search driven by `QueueView`.
    Searching,
    /// Match found: the map is coming up.
    Loading,
    /// Live match; the shell is out of the way.
    InMatch,
    /// Result screen shown after the match ends.
    PostMatch,
}

impl AppScreen {
    /// Menu screens hide the world and block gameplay input.
    pub fn is_menu(self) -> bool {
        !matches!(self, Self::InMatch | Self::PostMatch)
    }
}

/// Screen change requested from code that cannot hold `NextState` (network
/// teardown deep inside `net.rs`, for example).
#[derive(Resource, Default)]
pub struct PendingScreen(pub Option<AppScreen>);

/// Stops the session-driven screen transitions. Only the screenshot harness
/// sets this, so it can hold a screen that the live session would leave
/// immediately (the search screen without a real queue entry, for example).
#[derive(Resource, Default)]
pub struct ScreenDriverPaused(pub bool);

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrontendSet;

pub struct FrontendPlugin;

impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppScreen>()
            .init_resource::<PendingScreen>()
            .init_resource::<ScreenDriverPaused>()
            .init_resource::<JoinNotice>()
            .add_systems(Startup, bypass_shell_for_automation)
            .add_systems(Update, retry_connection_from_menus)
            .add_systems(Update, scale_menus_to_the_window)
            .add_systems(
                Update,
                (apply_pending_screen, drive_screen_from_session)
                    .chain()
                    .in_set(FrontendSet)
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(crate::input_context::InputContextSet::Resolve),
            )
            .add_plugins((
                widgets::FrontendWidgetsPlugin,
                home::HomeScreenPlugin,
                card::ProfileCardPlugin,
                collection::CollectionScreenPlugin,
                preview::AvatarPreviewPlugin,
                searching::SearchingScreenPlugin,
                loading::LoadingScreenPlugin,
                postmatch::PostMatchScreenPlugin,
            ));
    }
}

/// Headless evidence runs and the screenshot harnesses expect the old
/// "boot straight into the world" behaviour, so the shell steps aside for
/// them. Decided once: the environment does not change under a running client.
pub fn automation_bypass() -> bool {
    static BYPASS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *BYPASS.get_or_init(|| {
        bypass_for(
            std::env::vars_os()
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, _)| key.to_string_lossy().into_owned()),
        )
    })
}

/// The shell's own harness is the one QA run that must *not* bypass it.
const OWN_HARNESS: &str = "OMOBA_FRONTEND_QA_OUTPUT";

fn bypass_for(keys: impl Iterator<Item = String>) -> bool {
    keys.into_iter().any(|key| {
        key == "OMOBA_AUTOJOIN"
            || (key.starts_with("OMOBA_")
                && key != OWN_HARNESS
                && (key.ends_with("_QA_DIR") || key.ends_with("_QA_OUTPUT")))
    })
}

fn bypass_shell_for_automation(mut next: ResMut<NextState<AppScreen>>) {
    if automation_bypass() {
        next.set(AppScreen::InMatch);
    }
}

fn apply_pending_screen(
    mut pending: ResMut<PendingScreen>,
    mut next: ResMut<NextState<AppScreen>>,
) {
    if let Some(screen) = pending.0.take() {
        next.set(screen);
    }
}

/// Why the last lock-in did not become a match. The picker shows it until the
/// player locks in again.
#[derive(Resource, Default)]
pub struct JoinNotice(pub Option<String>);

/// Frames a screen may sit in `Searching` without a committed join before the
/// shell treats the lock-in as lost. The picker is ordered before the network
/// send, so this is only a guard against a future ordering change.
const UNCOMMITTED_GRACE_FRAMES: u8 = 3;

/// Seconds between connection retries while the player is in the menus with
/// nothing committed. A joined session has its own reconnect cadence.
const MENU_RETRY_SECS: f32 = 5.0;

/// Advances the shell from what the session and the server actually report.
/// Screens never guess: `Searching` leaves only once the server admits the
/// join, `Loading` leaves only once the local hero exists in a running match.
///
/// "Still mine" is [`ClientSession::has_committed_join`], not
/// `join_flow_committed`: a transport teardown clears the latter while the
/// session reconnects, and a reconnect must not look like leaving the match.
fn drive_screen_from_session(
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut session: ResMut<ClientSession>,
    game: Res<GameStateSnapshot>,
    paused: Res<ScreenDriverPaused>,
    mut notice: ResMut<JoinNotice>,
    local_player: Query<(), With<Player>>,
    mut uncommitted_frames: Local<u8>,
) {
    if paused.0 || automation_bypass() {
        return;
    }
    let current = *screen.get();
    let admitted = session.join_confirmed();
    let committed = session.has_committed_join();
    let in_world = !local_player.is_empty();
    if current != AppScreen::Searching {
        *uncommitted_frames = 0;
    }
    match current {
        AppScreen::Searching => {
            if session.join_blocked() {
                // Keep the reason, drop the dead join: the picker must be usable
                // again and the player must know why they are back on it.
                notice.0 = Some(session.join_rejection().map_or_else(
                    || {
                        "The server did not answer the join. Check the connection and lock in again."
                            .to_owned()
                    },
                    |rejection| rejection.message().to_owned(),
                ));
                session.abandon_join();
                next.set(AppScreen::HeroSelect);
            } else if !committed {
                *uncommitted_frames = uncommitted_frames.saturating_add(1);
                if *uncommitted_frames > UNCOMMITTED_GRACE_FRAMES {
                    next.set(AppScreen::HeroSelect);
                }
            } else {
                *uncommitted_frames = 0;
                if admitted
                    && matches!(
                        game.state,
                        GameState::Starting { .. } | GameState::Running | GameState::Victory { .. }
                    )
                {
                    next.set(AppScreen::Loading);
                }
            }
        }
        AppScreen::Loading => {
            if !committed && !admitted {
                next.set(AppScreen::Home);
            } else if in_world && matches!(game.state, GameState::Running) {
                next.set(AppScreen::InMatch);
            }
        }
        AppScreen::InMatch => {
            if matches!(game.state, GameState::Victory { .. }) {
                next.set(AppScreen::PostMatch);
            } else if !committed && !admitted && !in_world {
                next.set(AppScreen::Home);
            }
        }
        AppScreen::PostMatch => {
            if matches!(game.state, GameState::Running) && in_world {
                next.set(AppScreen::InMatch);
            }
        }
        // Home, Card, Collection and HeroSelect: a player whose committed join
        // lands in a live match anyway (a reconnect that completed mid-round)
        // must not be left in the menus. Without a committed join nothing here
        // may pull them in: that is what "back to menu" and "cancel" mean.
        _ => {
            if committed && admitted && in_world && matches!(game.state, GameState::Running) {
                next.set(AppScreen::InMatch);
            }
        }
    }
}

/// Height the front-end screens are laid out for. Everything was checked at
/// 1280x720 and 1024x640; a phone in landscape is ~400 logical pixels tall.
const MENU_DESIGN_HEIGHT: f32 = 640.0;
/// Below this the text would stop being readable on a phone.
const MIN_MENU_SCALE: f32 = 0.55;

/// Scale for the front-end screens on a window this tall. Never above 1: a
/// big window gets more room, not bigger buttons.
pub fn menu_scale(logical_height: f32) -> f32 {
    if logical_height <= 0.0 {
        return 1.0;
    }
    (logical_height / MENU_DESIGN_HEIGHT).clamp(MIN_MENU_SCALE, 1.0)
}

/// Shrinks the whole UI while a menu is on screen and restores it for the
/// match, whose HUD and touch controls are laid out for the real size. On a
/// phone the menus are otherwise taller than the screen.
fn scale_menus_to_the_window(
    screen: Res<State<AppScreen>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut ui_scale: ResMut<UiScale>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    // The phone picker is laid out in real screen pixels by `mobile_ui`,
    // like the match HUD: scaling it would misplace every part.
    let phone_picker = *screen.get() == AppScreen::HeroSelect
        && mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    let wanted = if screen.get().is_menu() && !phone_picker {
        menu_scale(window.resolution.height())
    } else {
        1.0
    };
    if (ui_scale.0 - wanted).abs() > f32::EPSILON {
        ui_scale.0 = wanted;
    }
}

/// Before anything is committed nobody reconnects for the player, so the
/// menus do: the career profile and the PLAY path come back on their own when
/// the server does.
fn retry_connection_from_menus(
    time: Res<Time>,
    screen: Res<State<AppScreen>>,
    session: Res<ClientSession>,
    mut retry: MessageWriter<crate::net::SessionUiCommand>,
    mut since_last: Local<f32>,
) {
    if automation_bypass() {
        return;
    }
    let offline = session.state == crate::net::ClientConnectionState::Disconnected;
    if !screen.get().is_menu() || !offline || session.has_committed_join() {
        *since_last = 0.0;
        return;
    }
    *since_last += time.delta_secs();
    if *since_last >= MENU_RETRY_SECS {
        *since_last = 0.0;
        retry.write(crate::net::SessionUiCommand::Retry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_screens_hide_the_world_and_match_screens_do_not() {
        for screen in [
            AppScreen::Home,
            AppScreen::Card,
            AppScreen::Collection,
            AppScreen::HeroSelect,
            AppScreen::Searching,
            AppScreen::Loading,
        ] {
            assert!(screen.is_menu(), "{screen:?} must hide the world");
        }
        assert!(!AppScreen::InMatch.is_menu());
        assert!(!AppScreen::PostMatch.is_menu());
    }

    #[test]
    fn every_world_harness_bypasses_the_shell_and_the_shell_harness_does_not() {
        let keys = |names: &[&str]| {
            names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        for harness in [
            "OMOBA_AUTOJOIN",
            "OMOBA_VISUAL_QA_DIR",
            "OMOBA_CAREER_QA_OUTPUT",
            "OMOBA_AUDIO_QA_OUTPUT",
        ] {
            assert!(bypass_for(keys(&[harness]).into_iter()), "{harness}");
        }
        assert!(!bypass_for(
            keys(&["OMOBA_FRONTEND_QA_OUTPUT", "HOME"]).into_iter()
        ));
        assert!(!bypass_for(keys(&["OMOBA_QA_WIDTH"]).into_iter()));
    }

    #[test]
    fn menus_shrink_on_a_phone_and_never_grow_on_a_big_screen() {
        // iPhone 16 Pro in landscape is 874x402 logical pixels.
        let phone = menu_scale(402.0);
        assert!((MIN_MENU_SCALE..0.7).contains(&phone), "{phone}");
        assert_eq!(menu_scale(640.0), 1.0);
        // iPad Air 11" landscape and desktop windows keep the designed size.
        assert_eq!(menu_scale(820.0), 1.0);
        assert_eq!(menu_scale(1440.0), 1.0);
        assert_eq!(menu_scale(100.0), MIN_MENU_SCALE);
        assert_eq!(menu_scale(0.0), 1.0);
    }

    #[test]
    fn pending_screen_requests_are_applied_once() {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>()
            .init_resource::<PendingScreen>()
            .add_systems(Update, apply_pending_screen);
        app.world_mut().resource_mut::<PendingScreen>().0 = Some(AppScreen::HeroSelect);
        // The request is consumed in `Update`; the state machine applies it in
        // the following frame's `StateTransition`.
        app.update();
        assert!(app.world().resource::<PendingScreen>().0.is_none());
        app.update();
        assert_eq!(
            *app.world().resource::<State<AppScreen>>().get(),
            AppScreen::HeroSelect
        );
    }

    /// Builds the smallest app that can exercise the screen driver.
    fn driver_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>()
            .init_resource::<PendingScreen>()
            .init_resource::<ScreenDriverPaused>()
            .init_resource::<JoinNotice>()
            .init_resource::<ClientSession>()
            .init_resource::<GameStateSnapshot>()
            .add_systems(
                Update,
                (apply_pending_screen, drive_screen_from_session).chain(),
            );
        app
    }

    fn screen(app: &App) -> AppScreen {
        *app.world().resource::<State<AppScreen>>().get()
    }

    /// Puts the app on `wanted` with `session`, the way the shell gets there.
    fn enter(app: &mut App, wanted: AppScreen, session: ClientSession) {
        *app.world_mut().resource_mut::<ClientSession>() = session;
        app.world_mut().resource_mut::<PendingScreen>().0 = Some(wanted);
        app.update();
        app.update();
    }

    fn settle(app: &mut App) {
        for _ in 0..(UNCOMMITTED_GRACE_FRAMES as usize + 3) {
            app.update();
        }
    }

    #[test]
    fn the_shell_only_leaves_the_search_when_the_server_admitted_the_join() {
        let mut app = driver_app();
        enter(
            &mut app,
            AppScreen::Searching,
            ClientSession::queued_for_test(),
        );
        settle(&mut app);
        // Queued, but no admission and no match start yet: the screen holds.
        assert_eq!(screen(&app), AppScreen::Searching);

        // Admitted and the match is starting: loading takes over.
        *app.world_mut().resource_mut::<ClientSession>() = ClientSession::admitted_for_test();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Starting {
            countdown_ms: 3_000,
        };
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::Loading);

        // The world is only entered once the local hero actually exists.
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::Loading);
        app.world_mut().spawn(Player);
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::InMatch);

        // The result screen follows the server's verdict.
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Victory {
            winner: crate::team::Team::Green,
        };
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::PostMatch);
    }

    #[test]
    fn a_rejected_join_returns_to_a_working_picker_with_the_reason() {
        let mut app = driver_app();
        let mut session = ClientSession::queued_for_test();
        session.reject_for_test(shared::protocol::JoinRejection::MatchFull);
        enter(&mut app, AppScreen::Searching, session);
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::HeroSelect);
        // The reason survives for the picker...
        assert_eq!(
            app.world().resource::<JoinNotice>().0.as_deref(),
            Some(shared::protocol::JoinRejection::MatchFull.message())
        );
        // ...and the dead join does not: the lock-in must work again.
        let session = app.world().resource::<ClientSession>();
        assert!(!session.join_flow_committed);
        assert!(!session.has_committed_join());
        assert!(!session.join_blocked());
    }

    #[test]
    fn a_lost_lock_in_falls_back_to_the_picker_after_the_grace_frames() {
        let mut app = driver_app();
        enter(&mut app, AppScreen::Searching, ClientSession::default());
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::HeroSelect);
        assert!(app.world().resource::<JoinNotice>().0.is_none());
    }

    #[test]
    fn a_reconnect_in_the_middle_of_a_match_keeps_the_match_on_screen() {
        let mut app = driver_app();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.world_mut().spawn(Player);
        enter(
            &mut app,
            AppScreen::InMatch,
            ClientSession::admitted_for_test(),
        );
        assert_eq!(screen(&app), AppScreen::InMatch);

        // Transport teardown: hero despawned, snapshot reset, reconnect armed.
        let players: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<Player>>()
            .iter(app.world())
            .collect();
        for player in players {
            app.world_mut().despawn(player);
        }
        *app.world_mut().resource_mut::<GameStateSnapshot>() = GameStateSnapshot::default();
        *app.world_mut().resource_mut::<ClientSession>() = ClientSession::reconnecting_for_test();
        settle(&mut app);
        assert_eq!(
            screen(&app),
            AppScreen::InMatch,
            "a reconnect must not look like leaving the match"
        );

        // The same holds while still loading into the match.
        enter(
            &mut app,
            AppScreen::Loading,
            ClientSession::reconnecting_for_test(),
        );
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::Loading);
    }

    #[test]
    fn leaving_the_match_goes_home_and_stale_snapshots_cannot_pull_back() {
        let mut app = driver_app();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        enter(
            &mut app,
            AppScreen::InMatch,
            ClientSession::admitted_for_test(),
        );

        // "Back to menu": the join is abandoned, the hero is gone.
        let mut session = ClientSession::admitted_for_test();
        session.abandon_join();
        *app.world_mut().resource_mut::<ClientSession>() = session;
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::Home);

        // A snapshot still in flight re-lists the player and even a hero: with
        // nothing committed, the menus keep the player.
        let mut stale = ClientSession::admitted_for_test();
        stale.last_join = None;
        *app.world_mut().resource_mut::<ClientSession>() = stale;
        app.world_mut().spawn(Player);
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::Home);
    }

    #[test]
    fn a_paused_driver_never_moves_the_screen() {
        let mut app = driver_app();
        app.world_mut().resource_mut::<ScreenDriverPaused>().0 = true;
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        enter(
            &mut app,
            AppScreen::Searching,
            ClientSession::admitted_for_test(),
        );
        settle(&mut app);
        assert_eq!(screen(&app), AppScreen::Searching);
    }
}
