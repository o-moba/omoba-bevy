use crate::combat::CombatStats;
use crate::domain::{MovementRoute, MovementTarget, Player, PlayerBody, VerticalVelocity};
use crate::maps::MapLayout;
use crate::model_scale::NormalizeModelScale;
use crate::net::{GameState, GameStateSnapshot, NetworkStructure, StructureKind};
use crate::sprite::PlayerVisualMode;
use bevy::prelude::*;
use shared::hero_balance::{DEBUG_SPEED_MULTIPLIER, PLAYER_SPEED};
use std::f32::consts::PI;

use super::{GRAVITY, GROUND_EPSILON, JUMP_HEIGHT, PLAYER_SIZE, ground_origin_y};
use crate::debug::DebugToggles;

#[derive(Component)]
pub(super) struct Jumping {
    pub(super) timer: Timer,
}

pub(super) fn move_player(
    mut commands: Commands,
    time: Res<Time>,
    mut transform_sets: ParamSet<(
        Query<
            (
                Entity,
                &mut Transform,
                &mut MovementRoute,
                &CombatStats,
                Option<&crate::net::PlayerEquipment>,
                Option<&crate::net::PlayerUtility>,
                Option<&crate::net::NetworkHeroClass>,
                Option<&crate::net::PlayerProgression>,
            ),
            (With<Player>, With<MovementTarget>),
        >,
        Query<&Transform, (With<PlayerBody>, Without<Player>)>,
        Query<(&Transform, &StructureKind, Option<&CombatStats>), With<NetworkStructure>>,
    )>,
    debug: Res<DebugToggles>,
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
        .filter(|(_, _, stats)| stats.is_none_or(|stats| stats.is_alive()))
        .map(|(transform, kind, _)| (transform.translation, *kind))
        .collect::<Vec<_>>();

    let mut player_query = transform_sets.p0();
    for (entity, mut transform, mut route, stats, equipment, utility, class, progression) in
        player_query.iter_mut()
    {
        if !stats.is_alive() {
            commands
                .entity(entity)
                .remove::<(MovementTarget, MovementRoute, Jumping)>();
            continue;
        }
        let Some(waypoint) = route.waypoints.first().copied() else {
            commands
                .entity(entity)
                .remove::<(MovementTarget, MovementRoute, Jumping)>();
            continue;
        };
        let current_pos = transform.translation;

        let target_pos_flat = Vec3::new(waypoint.x, current_pos.y, waypoint.z);
        let direction = (target_pos_flat - current_pos).normalize_or_zero();
        let distance = current_pos.xz().distance(target_pos_flat.xz());
        let speed = if debug.speed_boost {
            PLAYER_SPEED * DEBUG_SPEED_MULTIPLIER
        } else {
            PLAYER_SPEED
        };
        let speed = crate::sandbox::movement_speed(
            game_state.as_deref(),
            speed
                * equipment.map_or(1.0, |equipment| {
                    equipment.item_bonuses.move_speed_multiplier
                })
                * hero_movement_multiplier(class, progression),
        ) * utility.map_or(1.0, |u| u.state.movement_multiplier());
        let move_delta =
            speed * time.delta_secs() * crate::sandbox::time_scale(game_state.as_deref());

        if distance < move_delta || distance < 0.01 {
            let mut desired = target_pos_flat;
            desired = resolve_player_collisions(current_pos, desired, &other_players, &structures);
            if let Some(map_layout) = map_layout.as_ref() {
                desired = map_layout.clamp_position(desired);
            }
            desired = clip_static_movement(current_pos, desired);
            transform.translation.x = desired.x;
            transform.translation.z = desired.z;
            // Do not cut corners by advancing before the actual collision-
            // resolved position reaches this waypoint.
            if desired.xz().distance(waypoint.xz()) <= 0.01 {
                route.waypoints.remove(0);
                if route.waypoints.is_empty() {
                    commands
                        .entity(entity)
                        .remove::<(MovementTarget, MovementRoute, Jumping)>();
                }
            }
        } else {
            let mut desired = current_pos + direction * move_delta;
            desired = resolve_player_collisions(current_pos, desired, &other_players, &structures);
            if let Some(map_layout) = map_layout.as_ref() {
                desired = map_layout.clamp_position(desired);
            }
            desired = clip_static_movement(current_pos, desired);
            transform.translation.x = desired.x;
            transform.translation.z = desired.z;

            if direction.length_squared() > 0.001 {
                let target_y_angle = shared::math::hero_yaw_towards(direction.x, direction.z);
                let target_rotation = Quat::from_rotation_y(target_y_angle);

                transform.rotation = transform
                    .rotation
                    .slerp(target_rotation, time.delta_secs() * 10.0);
            }
        }
    }
}

/// A local presentation clock: pause/frame steps follow authority, never global Time.
#[derive(Default)]
pub(super) struct SandboxVisualClock {
    last: Option<((u64, u64), f64)>,
    paused: bool,
}
impl SandboxVisualClock {
    pub(super) fn delta(&mut self, time: &Time, game: Option<&GameStateSnapshot>) -> f32 {
        let Some((game, snapshot)) = game.and_then(|g| g.sandbox.as_ref().map(|s| (g, s))) else {
            self.last = None;
            self.paused = false;
            return time.delta_secs();
        };
        let identity = (game.meta.server_epoch, game.meta.match_id);
        let paused = snapshot.config.environment.paused;
        let delta = if paused {
            if self.paused {
                self.last
                    .filter(|(old, _)| *old == identity)
                    .map_or(0.0, |(_, previous)| {
                        (snapshot.simulation_secs - previous).max(0.0) as f32
                    })
            } else {
                0.0
            }
        } else {
            time.delta_secs() * snapshot.config.environment.time_scale
        };
        self.last = Some((identity, snapshot.simulation_secs));
        self.paused = paused;
        delta
    }
}

pub(super) fn animate_jump(
    time: Res<Time>,
    game: Option<Res<GameStateSnapshot>>,
    mut clock: Local<SandboxVisualClock>,
    map_layout: Res<MapLayout>,
    visual_mode: Res<PlayerVisualMode>,
    mut query: Query<
        (&mut Transform, &mut Jumping, Option<&NormalizeModelScale>),
        (With<Player>, With<MovementTarget>),
    >,
) {
    let dt = clock.delta(&time, game.as_deref());
    for (mut transform, mut jumping, normalization) in query.iter_mut() {
        jumping.timer.tick(std::time::Duration::from_secs_f32(dt));

        let progress = jumping.timer.fraction();

        let jump_offset = (progress * PI).sin() * JUMP_HEIGHT;

        // Base follows the terrain so hops track ramps instead of clipping.
        let base_y = ground_origin_y(
            &map_layout,
            *visual_mode,
            normalization,
            transform.translation.x,
            transform.translation.z,
        );
        transform.translation.y = base_y + jump_offset;
    }
}

pub(super) fn apply_gravity(
    time: Res<Time>,
    game: Option<Res<GameStateSnapshot>>,
    mut clock: Local<SandboxVisualClock>,
    map_layout: Res<MapLayout>,
    visual_mode: Res<PlayerVisualMode>,
    mut query: Query<
        (
            &mut Transform,
            &mut VerticalVelocity,
            Option<&Jumping>,
            Option<&NormalizeModelScale>,
        ),
        With<Player>,
    >,
) {
    let dt = clock.delta(&time, game.as_deref());
    if dt <= 0.0 {
        return;
    }

    for (mut transform, mut velocity, jumping, normalization) in query.iter_mut() {
        if jumping.is_some() {
            velocity.0 = 0.0;
            continue;
        }

        let ground_y = ground_origin_y(
            &map_layout,
            *visual_mode,
            normalization,
            transform.translation.x,
            transform.translation.z,
        );

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

/// Shared swept collision is applied after local crowd/structure resolution,
/// so a push or long frame cannot move the hero through static forest geometry.
pub(super) fn clip_static_movement(from: Vec3, desired: Vec3) -> Vec3 {
    let [x, z] = shared::navigation::world_navigation()
        .clip_movement([from.x, from.z], [desired.x, desired.z]);
    Vec3::new(x, desired.y, z)
}

pub(super) fn structure_revision(structures: &[(Vec3, StructureKind)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut keys: Vec<_> = structures
        .iter()
        .map(|(position, kind)| {
            (
                position.x.to_bits(),
                position.z.to_bits(),
                matches!(kind, StructureKind::BaseTower),
            )
        })
        .collect();
    keys.sort_unstable();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    keys.hash(&mut hash);
    hash.finish()
}

pub(super) fn resolve_player_collisions(
    from: Vec3,
    desired: Vec3,
    other_players: &[Vec3],
    structures: &[(Vec3, StructureKind)],
) -> Vec3 {
    let mut resolved = desired;
    let min_distance = PLAYER_SIZE;

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
        // Keep the same clearance as planned routes. Otherwise successive
        // thumb frames on the physical boundary form a chord through the
        // structure when sampled by the network, and authority rejects it.
        let min_distance = crate::navigation::structure_collision_radius(kind)
            + shared::navigation::PLANNING_CLEARANCE;
        let delta = Vec3::new(
            resolved.x - obstacle_pos.x,
            0.0,
            resolved.z - obstacle_pos.z,
        );
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

    let discs: Vec<_> = structures
        .iter()
        .map(|(p, kind)| shared::navigation::Disc {
            center: [p.x, p.z],
            radius: crate::navigation::structure_collision_radius(*kind)
                - shared::navigation::HERO_RADIUS,
        })
        .collect();
    let [x, z] = shared::navigation::clip_discs([from.x, from.z], [resolved.x, resolved.z], &discs);
    Vec3::new(x, resolved.y, z)
}

pub(super) fn resolve_player_structure_overlap(
    mut player_query: Query<&mut Transform, With<Player>>,
    structures: Query<
        (&Transform, &StructureKind, Option<&CombatStats>),
        (With<NetworkStructure>, Without<Player>),
    >,
) {
    let Ok(mut player_transform) = player_query.single_mut() else {
        return;
    };
    let mut resolved = player_transform.translation;

    for (structure_transform, kind, stats) in structures.iter() {
        if stats.is_some_and(|stats| !stats.is_alive()) {
            continue;
        }
        let min_distance = crate::navigation::structure_collision_radius(*kind);
        let delta = Vec3::new(
            resolved.x - structure_transform.translation.x,
            0.0,
            resolved.z - structure_transform.translation.z,
        );
        let distance = delta.length();
        if distance < min_distance {
            let push_dir = if distance > 0.0001 {
                delta / distance
            } else {
                Vec3::X
            };
            resolved.x = structure_transform.translation.x + push_dir.x * min_distance;
            resolved.z = structure_transform.translation.z + push_dir.z * min_distance;
        }
    }

    resolved = clip_static_movement(player_transform.translation, resolved);
    player_transform.translation.x = resolved.x;
    player_transform.translation.z = resolved.z;
}

/// Apply innate progression before the explicit sandbox speed override.
pub(super) fn hero_movement_multiplier(
    class: Option<&crate::net::NetworkHeroClass>,
    progression: Option<&crate::net::PlayerProgression>,
) -> f32 {
    shared::hero_balance::movement_multiplier(
        class.map_or(shared::HeroClass::default(), |c| c.0),
        progression.map_or(1, |p| p.level),
    )
}
