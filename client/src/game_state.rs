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

fn setup_game_state_ui(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
        Visibility::Hidden,
        ZIndex(10),
        GameStateOverlay,
        Name::new("GameStateOverlay"),
    ));
}

fn update_game_state_ui(
    game_state: Res<GameStateSnapshot>,
    local_team: Query<&Team, With<Player>>,
    mut overlay_query: Query<(&mut Visibility, &mut BackgroundColor), With<GameStateOverlay>>,
) {
    let Ok((mut visibility, mut background)) = overlay_query.get_single_mut() else {
        return;
    };

    match game_state.state {
        GameState::Running => {
            *visibility = Visibility::Hidden;
            *background = BackgroundColor(Color::NONE);
        }
        GameState::Victory { winner } => {
            *visibility = Visibility::Visible;
            let is_winner = local_team.get_single().is_ok_and(|team| *team == winner);
            *background = BackgroundColor(if is_winner { WIN_COLOR } else { LOSE_COLOR });
        }
    }
}
