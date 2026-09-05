use bevy::prelude::*;

use crate::net::{ClientSession, GameState, GameStateSnapshot};
use crate::player::Player;
use crate::team::Team;

const OVERLAY_ALPHA: f32 = 0.55;
const WIN_COLOR: Color = Color::srgba(0.12, 0.55, 0.22, OVERLAY_ALPHA);
const LOSE_COLOR: Color = Color::srgba(0.55, 0.12, 0.12, OVERLAY_ALPHA);
const LOBBY_COLOR: Color = Color::srgba(0.10, 0.10, 0.35, OVERLAY_ALPHA);

pub struct GameStateUiPlugin;

impl Plugin for GameStateUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_game_state_ui)
            .add_systems(Update, update_game_state_ui);
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
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                Pickable::IGNORE,
                GameStateLabel,
                Name::new("GameStateLabel"),
            ));
        });
}

/// Matchmaking overlay text for the pre-match states. `None` = keep the
/// overlay hidden (join not committed yet - the select screen is visible).
fn matchmaking_status_text(state: &GameState, join_committed: bool) -> Option<String> {
    if !join_committed {
        return None;
    }
    match state {
        GameState::Lobby => Some("Searching for match...".to_owned()),
        GameState::Forming { ready, needed } => Some(format!(
            "Searching for match...\nWaiting for players - {ready}/{needed}"
        )),
        GameState::Starting { countdown_ms } => Some(format!(
            "Match found!\nStarting in {}...",
            countdown_ms.div_ceil(1000).max(1)
        )),
        GameState::Running | GameState::Victory { .. } => None,
    }
}

fn update_game_state_ui(
    game_state: Res<GameStateSnapshot>,
    client_session: Res<ClientSession>,
    local_team: Query<&Team, With<Player>>,
    mut overlay_query: Query<(&mut Visibility, &mut BackgroundColor), With<GameStateOverlay>>,
    mut text_query: Query<&mut Text, With<GameStateLabel>>,
) {
    let Ok((mut visibility, mut background)) = overlay_query.single_mut() else {
        return;
    };
    let Ok(mut label) = text_query.single_mut() else {
        return;
    };

    if !client_session.is_connected() {
        *visibility = Visibility::Hidden;
        *background = BackgroundColor(Color::NONE);
        label.0.clear();
        return;
    }

    match game_state.state {
        GameState::Lobby | GameState::Forming { .. } | GameState::Starting { .. } => {
            // Before the local join is committed the select screen is up;
            // keep the lobby overlay hidden so it never obscures that flow.
            match matchmaking_status_text(&game_state.state, client_session.join_flow_committed) {
                Some(text) => {
                    *visibility = Visibility::Visible;
                    *background = BackgroundColor(LOBBY_COLOR);
                    label.0 = text;
                }
                None => {
                    *visibility = Visibility::Hidden;
                    *background = BackgroundColor(Color::NONE);
                    label.0.clear();
                }
            }
        }
        GameState::Running => {
            *visibility = Visibility::Hidden;
            *background = BackgroundColor(Color::NONE);
            label.0.clear();
        }
        GameState::Victory { winner } => {
            *visibility = Visibility::Visible;
            let is_winner = local_team.iter().next().is_some_and(|team| *team == winner);
            *background = BackgroundColor(if is_winner { WIN_COLOR } else { LOSE_COLOR });
            let base_msg = if is_winner {
                format!(
                    "Victory! Team {} destroyed the enemy base tower.",
                    winner.as_str()
                )
            } else {
                format!(
                    "Defeat. Team {} destroyed your base tower.",
                    winner.as_str()
                )
            };
            let next_steps = "\nPress Escape for the menu. Wait for an automatic rematch countdown when shown, or restart the client from the pause menu if needed.";
            label.0 = if let Some(secs) = game_state.rematch_in_secs {
                format!("{base_msg}\nRematch in {secs}s...{next_steps}")
            } else {
                format!("{base_msg}{next_steps}")
            };
        }
    }
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
        app.add_systems(Update, update_game_state_ui);
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
            session.state = ClientConnectionState::Connected;
            session.join_flow_committed = false;
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
            .join_flow_committed = true;
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
            "Searching for match..."
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
}
