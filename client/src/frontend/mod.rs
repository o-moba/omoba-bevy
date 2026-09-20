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
            .add_systems(Startup, bypass_shell_for_automation)
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
/// "boot straight into the world" behaviour. They set one of the QA
/// directories or `OMOBA_AUTOJOIN`, so the shell steps aside for them.
pub fn automation_bypass() -> bool {
    if std::env::var_os("OMOBA_AUTOJOIN").is_some() {
        return true;
    }
    std::env::vars_os().any(|(key, value)| {
        let key = key.to_string_lossy();
        key.starts_with("OMOBA_") && key.ends_with("_QA_DIR") && !value.is_empty()
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

/// Advances the shell from what the session and the server actually report.
/// Screens never guess: `Searching` leaves only once the server admits the
/// join, `Loading` leaves only once the local hero exists in a running match.
fn drive_screen_from_session(
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    paused: Res<ScreenDriverPaused>,
    local_player: Query<(), With<Player>>,
) {
    if paused.0 || automation_bypass() {
        return;
    }
    let current = *screen.get();
    let admitted = session.join_confirmed();
    let in_world = !local_player.is_empty();
    match current {
        AppScreen::Searching => {
            if session.join_blocked() || !session.join_flow_committed {
                // Rejected or cancelled: back to the picker with the reason on
                // screen (the connection status line renders it).
                next.set(AppScreen::HeroSelect);
            } else if admitted
                && matches!(
                    game.state,
                    GameState::Starting { .. } | GameState::Running | GameState::Victory { .. }
                )
            {
                next.set(AppScreen::Loading);
            }
        }
        AppScreen::Loading => {
            if !session.join_flow_committed && !admitted {
                next.set(AppScreen::Home);
            } else if in_world && matches!(game.state, GameState::Running) {
                next.set(AppScreen::InMatch);
            }
        }
        AppScreen::InMatch => {
            if matches!(game.state, GameState::Victory { .. }) {
                next.set(AppScreen::PostMatch);
            } else if !session.join_flow_committed && !admitted && !in_world {
                next.set(AppScreen::Home);
            }
        }
        AppScreen::PostMatch => {
            if matches!(game.state, GameState::Running) && in_world {
                next.set(AppScreen::InMatch);
            }
        }
        // Home, Card, Collection and HeroSelect: a player who ends up in a
        // live match anyway (a queue entry the server honoured after a cancel,
        // a reconnect that landed mid-round) must not be left in the menus.
        _ => {
            if admitted && in_world && matches!(game.state, GameState::Running) {
                next.set(AppScreen::InMatch);
            }
        }
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

    #[test]
    fn the_shell_only_leaves_the_search_when_the_server_admitted_the_join() {
        let mut app = driver_app();
        // Locking in commits the join and asks for the search screen in the
        // same frame, which is what the picker does.
        app.world_mut()
            .resource_mut::<ClientSession>()
            .join_flow_committed = true;
        app.world_mut().resource_mut::<PendingScreen>().0 = Some(AppScreen::Searching);
        app.update();
        app.update();
        // Queued, but no admission and no match start yet: the screen holds.
        assert_eq!(screen(&app), AppScreen::Searching);

        // Admitted and the match is starting: loading takes over.
        *app.world_mut().resource_mut::<ClientSession>() = ClientSession::admitted_for_test();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Starting {
            countdown_ms: 3_000,
        };
        app.update();
        app.update();
        assert_eq!(screen(&app), AppScreen::Loading);

        // The world is only entered once the local hero actually exists.
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.update();
        app.update();
        assert_eq!(screen(&app), AppScreen::Loading);
        app.world_mut().spawn(Player);
        app.update();
        app.update();
        assert_eq!(screen(&app), AppScreen::InMatch);

        // The result screen follows the server's verdict.
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Victory {
            winner: crate::team::Team::Green,
        };
        app.update();
        app.update();
        assert_eq!(screen(&app), AppScreen::PostMatch);
    }

    #[test]
    fn a_rejected_join_returns_to_hero_select() {
        let mut app = driver_app();
        app.world_mut().resource_mut::<PendingScreen>().0 = Some(AppScreen::Searching);
        app.update();
        app.update();
        let mut session = ClientSession::admitted_for_test();
        session.reject_for_test(shared::protocol::JoinRejection::MatchFull);
        *app.world_mut().resource_mut::<ClientSession>() = session;
        app.update();
        app.update();
        assert_eq!(screen(&app), AppScreen::HeroSelect);
    }

    #[test]
    fn a_paused_driver_never_moves_the_screen() {
        let mut app = driver_app();
        app.world_mut().resource_mut::<ScreenDriverPaused>().0 = true;
        app.world_mut().resource_mut::<PendingScreen>().0 = Some(AppScreen::Searching);
        *app.world_mut().resource_mut::<ClientSession>() = ClientSession::admitted_for_test();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.update();
        app.update();
        assert_eq!(screen(&app), AppScreen::Searching);
    }
}
