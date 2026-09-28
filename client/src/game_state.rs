//! Full-screen game state card: matchmaking progress (lobby, forming,
//! starting) and the career queue. The round result is the result screen's
//! (`frontend/postmatch.rs`, DECISIONS R2.2).
// i18n-strict
use bevy::prelude::*;

use crate::i18n::{tr, trf};
use crate::net::{ClientSession, GameState, GameStateSnapshot};

const OVERLAY_ALPHA: f32 = 0.55;
const LOBBY_COLOR: Color = Color::srgba(0.10, 0.10, 0.35, OVERLAY_ALPHA);

pub struct GameStateUiPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GameStateUiSet;

impl Plugin for GameStateUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_game_state_ui).add_systems(
            Update,
            update_game_state_ui
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .in_set(GameStateUiSet),
        );
    }
}

#[derive(Component)]
struct GameStateOverlay;

#[derive(Component)]
struct GameStateLabel;

fn setup_game_state_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
            Visibility::Hidden,
            ZIndex(10),
            // Purely informational overlay: it must never swallow pointer
            // events meant for UI underneath (e.g. the pre-join select screen).
            Pickable::IGNORE,
            GameStateOverlay,
            Name::new("GameStateOverlay"),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Percent(88.0),
                        max_width: Val::Px(660.0),
                        padding: UiRect::all(Val::Px(28.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(crate::ui::theme::PANEL),
                    BorderColor::all(crate::ui::theme::GOLD),
                    Pickable::IGNORE,
                    Name::new("GameStateCard"),
                ))
                .with_children(|card| {
                    card.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: 28.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Pickable::IGNORE,
                        GameStateLabel,
                        Name::new("GameStateLabel"),
                    ));
                });
        });
}

/// Matchmaking overlay text for the pre-match states. `None` = keep the
/// overlay hidden (join not committed yet - the select screen is visible).
fn matchmaking_status_text(state: &GameState, join_committed: bool) -> Option<String> {
    if !join_committed {
        return None;
    }
    match state {
        GameState::Lobby => Some(tr("state.lobby").to_owned()),
        GameState::Forming { ready, needed } => Some(trf(
            "state.forming",
            &[("ready", ready), ("needed", needed)],
        )),
        GameState::Starting { countdown_ms } => Some(trf(
            "state.starting",
            &[("seconds", &countdown_ms.div_ceil(1000).max(1))],
        )),
        GameState::Running | GameState::Victory { .. } => None,
    }
}

fn update_game_state_ui(
    game_state: Res<GameStateSnapshot>,
    client_session: Res<ClientSession>,
    career: Option<Res<crate::career::CareerClient>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut overlay_query: Query<(&mut Visibility, &mut BackgroundColor), With<GameStateOverlay>>,
    mut text_query: Query<&mut Text, With<GameStateLabel>>,
) {
    let Ok((mut visibility, mut background)) = overlay_query.single_mut() else {
        return;
    };
    let Ok(mut label) = text_query.single_mut() else {
        return;
    };

    // The result screen owns the end of a round (DECISIONS R2.2): the card
    // keeps only the lobby and queue texts and never sits under it.
    let result_screen = screen
        .as_ref()
        .is_some_and(|screen| *screen.get() == crate::frontend::AppScreen::PostMatch);
    if career.as_ref().is_some_and(|career| career.modal_open())
        || result_screen
        || !client_session.is_connected()
    {
        hide(&mut visibility, &mut background, &mut label);
        return;
    }

    if let Some(status) = career
        .as_ref()
        .and_then(|career| crate::career::queue_text(&career.view.queue))
    {
        *visibility = Visibility::Visible;
        *background = BackgroundColor(LOBBY_COLOR);
        label.0 = status;
        return;
    }
    // Before the local join is committed the select screen is up; keep the
    // lobby overlay hidden so it never obscures that flow. A running or
    // finished round shows nothing here.
    match matchmaking_status_text(&game_state.state, client_session.join_in_flight()) {
        Some(text) => {
            *visibility = Visibility::Visible;
            *background = BackgroundColor(LOBBY_COLOR);
            label.0 = text;
        }
        None => hide(&mut visibility, &mut background, &mut label),
    }
}

fn hide(visibility: &mut Visibility, background: &mut BackgroundColor, label: &mut Text) {
    *visibility = Visibility::Hidden;
    *background = BackgroundColor(Color::NONE);
    label.0.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::ClientConnectionState;

    fn spawn_ui_app() -> App {
        let mut app = App::new();
        app.init_resource::<GameStateSnapshot>();
        app.init_resource::<ClientSession>();
        app.add_systems(Startup, setup_game_state_ui);
        app.add_systems(
            Update,
            update_game_state_ui
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .in_set(GameStateUiSet),
        );
        app
    }

    fn overlay_entity(app: &mut App) -> Entity {
        app.world_mut()
            .query_filtered::<Entity, With<GameStateOverlay>>()
            .single(app.world())
            .expect("overlay spawned")
    }

    #[test]
    fn overlay_never_blocks_pointer_picking() {
        let mut app = spawn_ui_app();
        app.update();

        let overlay = overlay_entity(&mut app);
        assert_eq!(
            app.world().entity(overlay).get::<Pickable>(),
            Some(&Pickable::IGNORE),
            "overlay root must ignore picking"
        );

        let label = app
            .world_mut()
            .query_filtered::<Entity, With<GameStateLabel>>()
            .single(app.world())
            .expect("label spawned");
        assert_eq!(
            app.world().entity(label).get::<Pickable>(),
            Some(&Pickable::IGNORE),
            "overlay label must ignore picking"
        );
    }

    #[test]
    fn lobby_overlay_stays_hidden_until_join_is_committed() {
        let mut app = spawn_ui_app();
        {
            let mut session = app.world_mut().resource_mut::<ClientSession>();
            session.set_state_for_test(ClientConnectionState::Connected);
            session.set_join_in_flight_for_test(false);
        }
        app.update();

        let overlay = overlay_entity(&mut app);
        assert_eq!(
            *app.world().entity(overlay).get::<Visibility>().unwrap(),
            Visibility::Hidden,
            "pre-join lobby must not cover the select screen"
        );

        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_join_in_flight_for_test(true);
        app.update();
        assert_eq!(
            *app.world().entity(overlay).get::<Visibility>().unwrap(),
            Visibility::Visible,
            "committed join in lobby shows the waiting overlay"
        );
    }

    #[test]
    fn matchmaking_text_covers_all_search_states() {
        // Not committed: overlay stays hidden regardless of state.
        assert_eq!(matchmaking_status_text(&GameState::Lobby, false), None);
        assert_eq!(
            matchmaking_status_text(
                &GameState::Forming {
                    ready: 3,
                    needed: 10
                },
                false
            ),
            None
        );
        // Committed: every pre-match state has a distinct, readable message.
        assert_eq!(
            matchmaking_status_text(&GameState::Lobby, true).unwrap(),
            "Waiting for players..."
        );
        let forming = matchmaking_status_text(
            &GameState::Forming {
                ready: 3,
                needed: 10,
            },
            true,
        )
        .unwrap();
        assert!(
            forming.contains("3/10"),
            "forming shows progress: {forming}"
        );
        let starting =
            matchmaking_status_text(&GameState::Starting { countdown_ms: 2400 }, true).unwrap();
        assert!(
            starting.contains("Starting in 3"),
            "countdown rounds up: {starting}"
        );
        // In-match states render no matchmaking overlay.
        assert_eq!(matchmaking_status_text(&GameState::Running, true), None);
    }
    /// DECISIONS R2.2: the round result is the result screen's; the card
    /// never shows it (it replaced "Victory! … Stay connected …") and comes
    /// back for the next round's countdown on its own.
    #[test]
    fn victory_is_left_to_the_result_screen_and_the_countdown_returns() {
        let mut app = spawn_ui_app();
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(ClientConnectionState::Connected);
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_join_in_flight_for_test(true);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Victory {
            winner: shared::map::Team::Green,
        };
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .rematch_in_secs = Some(10);
        app.update();
        let overlay = overlay_entity(&mut app);
        let label = app
            .world_mut()
            .query_filtered::<Entity, With<GameStateLabel>>()
            .single(app.world())
            .unwrap();
        assert_eq!(
            *app.world().get::<Visibility>(overlay).unwrap(),
            Visibility::Hidden
        );
        assert!(app.world().get::<Text>(label).unwrap().0.is_empty());
        app.world_mut().resource_mut::<GameStateSnapshot>().state =
            GameState::Starting { countdown_ms: 3000 };
        app.update();
        assert!(
            app.world()
                .get::<Text>(label)
                .unwrap()
                .0
                .contains("Starting in 3")
        );
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(overlay).unwrap(),
            Visibility::Hidden
        );
    }

    /// The card also stays out from under the result screen, whatever it
    /// would print (a queue line from the career view).
    #[test]
    fn the_result_screen_hides_the_card() {
        use crate::frontend::AppScreen;
        let mut app = spawn_ui_app();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>();
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(ClientConnectionState::Connected);
        let mut career = crate::career::CareerClient::default();
        career.view.queue = shared::career::QueueView::Waiting {
            compatible: 3,
            needed: 10,
            elapsed_secs: 12,
            newcomer: false,
        };
        app.insert_resource(career);
        app.update();
        let overlay = overlay_entity(&mut app);
        assert_eq!(
            *app.world().get::<Visibility>(overlay).unwrap(),
            Visibility::Visible
        );
        app.world_mut()
            .resource_mut::<NextState<AppScreen>>()
            .set(AppScreen::PostMatch);
        app.update();
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(overlay).unwrap(),
            Visibility::Hidden
        );
    }

    #[test]
    fn authoritative_queue_wait_is_visible_while_another_match_is_running() {
        let mut app = spawn_ui_app();
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(ClientConnectionState::Connected);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        let mut career = crate::career::CareerClient::default();
        career.view.queue = shared::career::QueueView::Waiting {
            compatible: 3,
            needed: 10,
            elapsed_secs: 12,
            newcomer: true,
        };
        app.insert_resource(career);
        app.update();
        let text = &app
            .world_mut()
            .query_filtered::<&Text, With<GameStateLabel>>()
            .single(app.world())
            .unwrap()
            .0;
        assert!(text.contains("3/10 compatible players"));
        assert!(text.contains("New players"));
    }

    #[test]
    fn lobby_text_follows_the_language() {
        if crate::i18n::testing::isolated("game_state::tests::lobby_text_follows_the_language") {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let mut app = spawn_ui_app();
        app.add_plugins(I18nPlugin::default());
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(ClientConnectionState::Connected);
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_join_in_flight_for_test(true);
        app.update();
        let label = app
            .world_mut()
            .query_filtered::<Entity, With<GameStateLabel>>()
            .single(app.world())
            .unwrap();
        let text = |app: &App| app.world().get::<Text>(label).unwrap().0.clone();
        assert_eq!(text(&app), "Waiting for players...");
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("zh-Hans").unwrap());
        app.update();
        assert_eq!(text(&app), "等待玩家加入…");
        assert_eq!(
            matchmaking_status_text(&GameState::Lobby, true).as_deref(),
            Some("等待玩家加入…")
        );
    }
}
