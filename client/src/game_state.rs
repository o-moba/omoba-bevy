use bevy::prelude::*;

use crate::net::{GameState, GameStateSnapshot};
use crate::player::Player;
use crate::team::Team;

const OVERLAY_ALPHA: f32 = 0.55;
const WIN_COLOR: Color = Color::srgba(0.12, 0.55, 0.22, OVERLAY_ALPHA);
const LOSE_COLOR: Color = Color::srgba(0.55, 0.12, 0.12, OVERLAY_ALPHA);

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
                GameStateLabel,
                Name::new("GameStateLabel"),
            ));
        });
}

fn update_game_state_ui(
    game_state: Res<GameStateSnapshot>,
    local_team: Query<&Team, With<Player>>,
    mut overlay_query: Query<(&mut Visibility, &mut BackgroundColor), With<GameStateOverlay>>,
    mut text_query: Query<&mut Text, With<GameStateLabel>>,
) {
    let Ok((mut visibility, mut background)) = overlay_query.get_single_mut() else {
        return;
    };
    let Ok(mut label) = text_query.get_single_mut() else {
        return;
    };

    match game_state.state {
        GameState::Running => {
            *visibility = Visibility::Hidden;
            *background = BackgroundColor(Color::NONE);
            label.0.clear();
        }
        GameState::Victory { winner } => {
            *visibility = Visibility::Visible;
            let is_winner = local_team.get_single().is_ok_and(|team| *team == winner);
            *background = BackgroundColor(if is_winner { WIN_COLOR } else { LOSE_COLOR });
            label.0 = if is_winner {
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
        }
    }
}
