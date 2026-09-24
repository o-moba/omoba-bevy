use crate::combat::{CombatStats, MAX_HP};
use crate::debug::DebugConsole;
use crate::domain::{MovementTarget, Player, VerticalVelocity};
use crate::maps::MapLayout;
use crate::net::{GameState, GameStateSnapshot};
use crate::team::Team;
use bevy::prelude::*;

use super::RESPAWN_DELAY_SECONDS;
use super::motion::Jumping;

#[derive(Resource)]
pub(super) struct RespawnCountdown {
    pub(super) end_time: Option<f32>,
    pub(super) last_shown: i32,
    pub(super) last_hp: f32,
}

impl Default for RespawnCountdown {
    fn default() -> Self {
        Self {
            end_time: None,
            last_shown: -1,
            last_hp: MAX_HP,
        }
    }
}

#[derive(Component)]
pub(super) struct RespawnCountdownText;

pub(super) fn setup_respawn_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(20.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Name::new("RespawnCountdown"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: 36.0,
                    ..default()
                },
                TextColor::WHITE,
                Visibility::Hidden,
                RespawnCountdownText,
            ));
        });
}

pub(super) fn respawn_countdown_system(
    time: Res<Time>,
    map_layout: Res<MapLayout>,
    mut commands: Commands,
    mut state: ResMut<RespawnCountdown>,
    mut player_query: Query<
        (
            Entity,
            &mut Transform,
            &CombatStats,
            &Team,
            &mut VerticalVelocity,
        ),
        With<Player>,
    >,
    mut text_query: Query<(&mut Text, &mut Visibility), With<RespawnCountdownText>>,
    mut console: ResMut<DebugConsole>,
    game_state: Option<Res<GameStateSnapshot>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Ok((entity, mut transform, stats, team, mut velocity)) = player_query.single_mut() else {
        return;
    };

    let Ok((mut text, mut visibility)) = text_query.single_mut() else {
        return;
    };

    let display_time = game_state
        .as_ref()
        .and_then(|g| g.sandbox.as_ref())
        .map_or(time.elapsed_secs(), |s| s.simulation_secs as f32);
    if stats.is_alive() {
        if state.end_time.is_some() {
            state.end_time = None;
            state.last_shown = -1;
            *visibility = Visibility::Hidden;
            text.0.clear();
            if game_state.as_ref().is_none_or(|g| g.sandbox.is_none()) {
                transform.translation = map_layout.team_spawn(*team);
            }
            velocity.0 = 0.0;
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
            console.push_line("Respawned.");
            info!("Respawned.");
        }
        state.last_hp = stats.hp;
        return;
    }

    if state.last_hp > 0.0 && stats.hp <= 0.0 {
        state.end_time = Some(display_time + RESPAWN_DELAY_SECONDS);
        state.last_shown = -1;
    }
    let end_time = state
        .end_time
        .get_or_insert(display_time + RESPAWN_DELAY_SECONDS);
    let remaining = (*end_time - display_time).ceil().max(0.0) as i32;

    *visibility = Visibility::Visible;
    if remaining != state.last_shown {
        state.last_shown = remaining;
        text.0 = remaining.to_string();
        let message = format!("Respawn in {remaining}");
        console.push_line(message.clone());
        info!("{message}");
    }
    state.last_hp = stats.hp;
}
