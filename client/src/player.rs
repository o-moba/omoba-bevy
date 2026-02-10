use bevy::{
    input::mouse::MouseButton,
    math::{Dir3, primitives::InfinitePlane3d},
    prelude::*,
    window::PrimaryWindow,
};
use std::f32::consts::PI;

use crate::camera::{CameraState, MainCamera};
use crate::combat::{CombatStats, MAX_HP};
use crate::debug_console::DebugConsole;
use crate::maps::MapLayout;
use crate::net::{GameState, GameStateSnapshot, NetworkStructure, StructureKind};
use crate::team::Team;

pub const PLAYER_SPEED: f32 = 5.0;
pub const PLAYER_SIZE: f32 = 1.0;
pub const JUMP_HEIGHT: f32 = 1.5;
pub const JUMP_DURATION: f32 = 0.6;
const GRAVITY: f32 = 20.0;
const GROUND_EPSILON: f32 = 0.001;
const RESPAWN_DELAY_SECONDS: f32 = 5.0;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (handle_player_input, animate_jump, move_player).chain(),
        )
        .add_systems(PostUpdate, apply_gravity)
        .init_resource::<RespawnCountdown>()
        .add_systems(Startup, setup_respawn_ui)
        .add_systems(Update, respawn_countdown_system);
    }
}

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct PlayerBody;

#[derive(Component, Default)]
pub struct VerticalVelocity(pub f32);

#[derive(Resource)]
struct RespawnCountdown {
    end_time: Option<f32>,
    last_shown: i32,
    last_hp: f32,
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
struct RespawnCountdownText;

#[derive(Component)]
struct MovementTarget {
    target: Vec3,
}

#[derive(Component)]
struct Jumping {
    timer: Timer,
    start_y: f32,
}

fn handle_player_input(
    mut commands: Commands,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    player_query: Query<(Entity, &CombatStats), With<Player>>,
    cam_state: Res<CameraState>,
    map_layout: Option<Res<MapLayout>>,
    game_state: Option<Res<GameStateSnapshot>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    if !cam_state.locked {
        return;
    }
    let Ok(window) = window_query.get_single() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else {
        return;
    };

    let Ok((player_entity, stats)) = player_query.get_single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }

    if mouse_button_input.just_pressed(MouseButton::Left) {
        if let Some(cursor_pos) = window.cursor_position() {
            if let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) {
                if let Ok(plane_normal) = Dir3::new(Vec3::Y) {
                    let plane_origin = Vec3::ZERO;
                    let infinite_plane = InfinitePlane3d::new(plane_normal);
                    if let Some(distance) = ray.intersect_plane(plane_origin, infinite_plane) {
                        if distance >= 0.0 {
                            let mut target_pos = ray.get_point(distance);
                            if let Some(map_layout) = map_layout.as_ref() {
                                target_pos = map_layout.clamp_position(target_pos);
                            }
                            commands
                                .entity(player_entity)
                                .insert(MovementTarget { target: target_pos });
                            commands.entity(player_entity).insert(Jumping {
                                timer: Timer::from_seconds(JUMP_DURATION, TimerMode::Repeating),
                                start_y: PLAYER_SIZE / 2.0,
                            });
                        }
                    }
                } else {
                    warn!("Plane normal is zero, cannot raycast");
                }
            }
        }
    }
}

fn move_player(
    mut commands: Commands,
    time: Res<Time>,
    mut transform_sets: ParamSet<(
        Query<(Entity, &mut Transform, &MovementTarget, &CombatStats), With<Player>>,
        Query<&Transform, (With<PlayerBody>, Without<Player>)>,
        Query<(&Transform, &StructureKind), With<NetworkStructure>>,
    )>,
    map_layout: Option<Res<MapLayout>>,
    game_state: Option<Res<GameStateSnapshot>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let other_players = transform_sets
        .p1()
        .iter()
        .map(|transform| transform.translation)
        .collect::<Vec<_>>();
    let structures = transform_sets
        .p2()
        .iter()
        .map(|(transform, kind)| (transform.translation, *kind))
        .collect::<Vec<_>>();

    let mut player_query = transform_sets.p0();
    for (entity, mut transform, movement_target, stats) in player_query.iter_mut() {
        if !stats.is_alive() {
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
            continue;
        }
        let current_pos = transform.translation;

        let target_pos_flat = Vec3::new(
            movement_target.target.x,
            current_pos.y,
            movement_target.target.z,
        );
        let direction = (target_pos_flat - current_pos).normalize_or_zero();
        let distance = current_pos.xz().distance(target_pos_flat.xz());
        let move_delta = PLAYER_SPEED * time.delta_secs();

        if distance < move_delta || distance < 0.01 {
            let mut desired = Vec3::new(
                movement_target.target.x,
                current_pos.y,
                movement_target.target.z,
            );
            desired = resolve_player_collisions(desired, &other_players, &structures);
            if let Some(map_layout) = map_layout.as_ref() {
                desired = map_layout.clamp_position(desired);
            }
            transform.translation.x = desired.x;
            transform.translation.z = desired.z;
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
        } else {
            let mut desired = current_pos + direction * move_delta;
            desired = resolve_player_collisions(desired, &other_players, &structures);
            if let Some(map_layout) = map_layout.as_ref() {
                desired = map_layout.clamp_position(desired);
            }
            transform.translation.x = desired.x;
            transform.translation.z = desired.z;

            if direction.length_squared() > 0.001 {
                let target_y_angle = direction.x.atan2(direction.z);
                let target_rotation = Quat::from_rotation_y(target_y_angle);

                transform.rotation = transform
                    .rotation
                    .slerp(target_rotation, time.delta_secs() * 10.0);
            }
        }
    }
}

fn animate_jump(
    time: Res<Time>,
    mut query: Query<(&mut Transform, &mut Jumping), (With<Player>, With<MovementTarget>)>,
) {
    for (mut transform, mut jumping) in query.iter_mut() {
        jumping.timer.tick(time.delta());

        let progress = jumping.timer.fraction();

        let jump_offset = (progress * PI).sin() * JUMP_HEIGHT;

        transform.translation.y = jumping.start_y + jump_offset;
    }
}

fn apply_gravity(
    time: Res<Time>,
    mut query: Query<(&mut Transform, &mut VerticalVelocity, Option<&Jumping>), With<Player>>,
) {
    let ground_y = PLAYER_SIZE / 2.0;
    let dt = time.delta_secs();

    for (mut transform, mut velocity, jumping) in query.iter_mut() {
        if jumping.is_some() {
            velocity.0 = 0.0;
            continue;
        }

        if transform.translation.y <= ground_y + GROUND_EPSILON {
            transform.translation.y = ground_y;
            velocity.0 = 0.0;
            continue;
        }

        velocity.0 -= GRAVITY * dt;
        transform.translation.y += velocity.0 * dt;

        if transform.translation.y <= ground_y {
            transform.translation.y = ground_y;
            velocity.0 = 0.0;
        }
    }
}

fn setup_respawn_ui(mut commands: Commands) {
    commands.spawn((
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

fn respawn_countdown_system(
    time: Res<Time>,
    map_layout: Res<MapLayout>,
    mut commands: Commands,
    mut state: ResMut<RespawnCountdown>,
    mut player_query: Query<(Entity, &mut Transform, &CombatStats, &Team, &mut VerticalVelocity), With<Player>>,
    mut text_query: Query<(&mut Text, &mut Visibility), With<RespawnCountdownText>>,
    mut console: ResMut<DebugConsole>,
    game_state: Option<Res<GameStateSnapshot>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Ok((entity, mut transform, stats, team, mut velocity)) =
        player_query.get_single_mut()
    else {
        return;
    };

    let Ok((mut text, mut visibility)) = text_query.get_single_mut() else {
        return;
    };

    if stats.is_alive() {
        if state.end_time.is_some() {
            state.end_time = None;
            state.last_shown = -1;
            *visibility = Visibility::Hidden;
            text.0.clear();
            let spawn = match team {
                Team::Green => map_layout.home_spawn,
                Team::Blue => map_layout.away_spawn,
            };
            transform.translation = spawn;
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
        state.end_time = Some(time.elapsed_secs() + RESPAWN_DELAY_SECONDS);
        state.last_shown = -1;
    }
    let end_time = state
        .end_time
        .get_or_insert_with(|| time.elapsed_secs() + RESPAWN_DELAY_SECONDS);
    let remaining = (*end_time - time.elapsed_secs()).ceil().max(0.0) as i32;

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

fn resolve_player_collisions(
    desired: Vec3,
    other_players: &[Vec3],
    structures: &[(Vec3, StructureKind)],
) -> Vec3 {
    let mut resolved = desired;
    let min_distance = PLAYER_SIZE;
    let player_radius = PLAYER_SIZE * 0.5;

    for &other_pos in other_players.iter() {
        let delta = Vec3::new(resolved.x - other_pos.x, 0.0, resolved.z - other_pos.z);
        let distance = delta.length();
        if distance < min_distance {
            let push_dir = if distance > 0.0001 {
                delta / distance
            } else {
                Vec3::X
            };
            resolved.x = other_pos.x + push_dir.x * min_distance;
            resolved.z = other_pos.z + push_dir.z * min_distance;
        }
    }

    for &(obstacle_pos, kind) in structures.iter() {
        let obstacle_radius = match kind {
            StructureKind::Tower => 1.3,
            StructureKind::Nexus => 4.0,
        };
        let min_distance = player_radius + obstacle_radius;
        let delta =
            Vec3::new(resolved.x - obstacle_pos.x, 0.0, resolved.z - obstacle_pos.z);
        let distance = delta.length();
        if distance < min_distance {
            let push_dir = if distance > 0.0001 {
                delta / distance
            } else {
                Vec3::X
            };
            resolved.x = obstacle_pos.x + push_dir.x * min_distance;
            resolved.z = obstacle_pos.z + push_dir.z * min_distance;
        }
    }

    resolved
}
