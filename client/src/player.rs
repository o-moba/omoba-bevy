use bevy::{
    input::mouse::MouseButton,
    math::{Dir3, primitives::InfinitePlane3d},
    prelude::*,
    window::PrimaryWindow,
};
use std::f32::consts::PI;

use crate::camera::{CameraState, MainCamera};

pub const PLAYER_SPEED: f32 = 5.0;
pub const PLAYER_SIZE: f32 = 1.0;
pub const JUMP_HEIGHT: f32 = 1.5;
pub const JUMP_DURATION: f32 = 0.6;
const GRAVITY: f32 = 20.0;
const GROUND_EPSILON: f32 = 0.001;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (handle_player_input, animate_jump, move_player).chain())
            .add_systems(PostUpdate, apply_gravity);
    }
}

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct PlayerBody;

#[derive(Component, Default)]
pub struct VerticalVelocity(pub f32);

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
    player_query: Query<Entity, With<Player>>,
    cam_state: Res<CameraState>,
) {
    if !cam_state.locked {
        return;
    }
    let Ok(window) = window_query.get_single() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else {
        return;
    };

    if mouse_button_input.just_pressed(MouseButton::Left) {
        if let Some(cursor_pos) = window.cursor_position() {
            if let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) {
                if let Ok(plane_normal) = Dir3::new(Vec3::Y) {
                    let plane_origin = Vec3::ZERO;
                    let infinite_plane = InfinitePlane3d::new(plane_normal);
                    if let Some(distance) = ray.intersect_plane(plane_origin, infinite_plane) {
                        if distance >= 0.0 {
                            let target_pos = ray.get_point(distance);
                            if let Ok(player_entity) = player_query.get_single() {
                                commands
                                    .entity(player_entity)
                                    .insert(MovementTarget { target: target_pos });
                                commands.entity(player_entity).insert(Jumping {
                                    timer: Timer::from_seconds(JUMP_DURATION, TimerMode::Repeating),
                                    start_y: PLAYER_SIZE / 2.0,
                                });
                            }
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
    mut query: Query<(Entity, &mut Transform, &MovementTarget), With<Player>>,
    other_players: Query<&Transform, (With<PlayerBody>, Without<Player>)>,
) {
    for (entity, mut transform, movement_target) in query.iter_mut() {
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
            desired = resolve_player_collisions(desired, &other_players);
            transform.translation.x = desired.x;
            transform.translation.z = desired.z;
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
        } else {
            let mut desired = current_pos + direction * move_delta;
            desired = resolve_player_collisions(desired, &other_players);
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

fn resolve_player_collisions(
    desired: Vec3,
    other_players: &Query<&Transform, (With<PlayerBody>, Without<Player>)>,
) -> Vec3 {
    let mut resolved = desired;
    let min_distance = PLAYER_SIZE;

    for other in other_players.iter() {
        let other_pos = other.translation;
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

    resolved
}
