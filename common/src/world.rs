use std::collections::HashMap;
use std::time::{Duration, Instant};

use shared::combat::MinionKind;
use shared::map::{Lane, Team};
use shared::wire::{GameState, MinionBrainState, MinionState, StructureKind, StructureState};

use crate::balance::{
    BASE_TOWER_SIZE, MINION_RADIUS, MINION_SPAWN_HEIGHT, MINION_WAVE_INTERVAL, MINIONS_PER_WAVE,
    PLAYER_SPAWN_OFFSET, TOWER_SIZE,
};
use crate::combat_feedback::minion_stats;
use crate::entities::{MapLayoutState, Minion, Structure, StructureRole, Vec3f};
use crate::game_world::GameWorld;

#[cfg(any(test, feature = "test-support"))]
use crate::balance::{
    BASE_TOWER_COOLDOWN, BASE_TOWER_DAMAGE, BASE_TOWER_MAX_HP, BASE_TOWER_RANGE, TOWER_COOLDOWN,
    TOWER_DAMAGE, TOWER_MAX_HP, TOWER_RANGE,
};

#[cfg(any(test, feature = "test-support"))]
pub fn build_structures(_layout: &MapLayoutState) -> HashMap<u64, Structure> {
    build_configured_structures(&shared::map::ResolvedMap::default())
}

pub fn build_configured_structures(config: &shared::map::ResolvedMap) -> HashMap<u64, Structure> {
    config
        .structures
        .iter()
        .map(|item| {
            let team = item.team;
            let (kind, role, y) = match item.lane {
                Some(lane) => (StructureKind::Tower, StructureRole::LaneTower { lane }, 3.0),
                None => (StructureKind::BaseTower, StructureRole::BaseTower, 4.0),
            };
            (
                item.id,
                Structure {
                    state: StructureState {
                        protected: false,
                        id: item.id,
                        kind,
                        team,
                        map_key: item.key.clone(),
                        visual_profile: item.visual_profile.clone(),
                        lane: item.lane,
                        tier: item.tier,
                        x: item.position[0],
                        y,
                        z: item.position[1],
                        hp: item.stats.max_hp,
                        max_hp: item.stats.max_hp,
                    },
                    role,
                    last_attack_at: None,
                    attack_range: item.stats.attack_range,
                    attack_damage: item.stats.attack_damage,
                    hero_damage_multiplier: item.stats.hero_damage_multiplier,
                    attack_cooldown: Duration::from_millis(item.stats.attack_cooldown_ms),
                },
            )
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub fn add_structure(
    structures: &mut HashMap<u64, Structure>,
    next_id: &mut u64,
    kind: StructureKind,
    role: StructureRole,
    team: Team,
    position: Vec3f,
) {
    let (max_hp, attack_range, attack_damage, attack_cooldown) = match kind {
        StructureKind::Tower => (TOWER_MAX_HP, TOWER_RANGE, TOWER_DAMAGE, TOWER_COOLDOWN),
        StructureKind::BaseTower => (
            BASE_TOWER_MAX_HP,
            BASE_TOWER_RANGE,
            BASE_TOWER_DAMAGE,
            BASE_TOWER_COOLDOWN,
        ),
    };
    let id = *next_id;
    *next_id += 1;
    structures.insert(
        id,
        Structure {
            state: StructureState {
                protected: false,
                map_key: format!("test_{id}"),
                visual_profile: String::new(),
                lane: match role {
                    StructureRole::LaneTower { lane } => Some(lane),
                    StructureRole::BaseTower => None,
                },
                tier: 0,
                id,
                kind,
                team,
                x: position.x,
                y: position.y,
                z: position.z,
                hp: max_hp,
                max_hp,
            },
            role,
            last_attack_at: None,
            attack_range,
            attack_damage,
            hero_damage_multiplier: 2.0,
            attack_cooldown,
        },
    );
}

pub fn build_map_layout() -> MapLayoutState {
    let g = shared::map::geometry();
    MapLayoutState {
        home: Vec3f::new(g.home[0], 0.0, g.home[1]),
        away: Vec3f::new(g.away[0], 0.0, g.away[1]),
        min_x: g.bounds.min[0],
        max_x: g.bounds.max[0],
        min_z: g.bounds.min[1],
        max_z: g.bounds.max[1],
        #[cfg(any(test, feature = "test-support"))]
        left_x: g.left_x,
        #[cfg(any(test, feature = "test-support"))]
        right_x: g.right_x,
        #[cfg(any(test, feature = "test-support"))]
        top_z: g.top_z,
        #[cfg(any(test, feature = "test-support"))]
        bottom_z: g.bottom_z,
    }
}

pub fn spawn_position_for_team(map_layout: &MapLayoutState, team: Team) -> Vec3f {
    let base = match team {
        Team::Green => map_layout.home,
        Team::Blue => map_layout.away,
    };
    let dir = Vec3f::new(-base.x, 0.0, -base.z).normalize_or_zero();
    Vec3f::new(
        base.x + dir.x * PLAYER_SPAWN_OFFSET,
        base.y,
        base.z + dir.z * PLAYER_SPAWN_OFFSET,
    )
}

pub fn spawn_position_for_team_from_base(
    structures: &HashMap<u64, Structure>,
    map_layout: &MapLayoutState,
    team: Team,
) -> Vec3f {
    let Some(base_tower) = structures.values().find(|structure| {
        structure.state.team == team
            && structure.state.kind == StructureKind::BaseTower
            && structure.state.hp > 0.0
    }) else {
        return spawn_position_for_team(map_layout, team);
    };

    let base = Vec3f::new(base_tower.state.x, 0.0, base_tower.state.z);
    let dir = Vec3f::new(-base.x, 0.0, -base.z).normalize_or_zero();
    Vec3f::new(
        base.x + dir.x * PLAYER_SPAWN_OFFSET,
        0.0,
        base.z + dir.z * PLAYER_SPAWN_OFFSET,
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn lane_control_points(_layout: &MapLayoutState, lane: Lane) -> Vec<Vec3f> {
    shared::map::lane_points(lane)
        .into_iter()
        .map(|point| Vec3f::new(point[0], 0.0, point[1]))
        .collect()
}

pub fn build_minion_path(_layout: &MapLayoutState, lane: Lane, team: Team) -> Vec<Vec3f> {
    let mut points: Vec<Vec3f> = shared::map::minion_lane_points(lane)
        .into_iter()
        .map(|point| Vec3f::new(point[0], 0.0, point[1]))
        .collect();
    if team == Team::Blue {
        points.reverse();
    }
    for point in &mut points {
        point.y = MINION_SPAWN_HEIGHT;
    }
    points
}

pub fn spawn_minion_waves_if_due(world: &mut GameWorld, now: Instant) {
    if !matches!(world.game_state, GameState::Running) {
        return;
    }
    if now.duration_since(world.last_wave_spawn_at) < MINION_WAVE_INTERVAL {
        return;
    }
    world.last_wave_spawn_at = now;
    let GameWorld {
        map_layout,
        minions,
        next_minion_id,
        ..
    } = world;

    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
        spawn_minion_wave_for_team_lane(map_layout, minions, next_minion_id, Team::Green, lane);
        spawn_minion_wave_for_team_lane(map_layout, minions, next_minion_id, Team::Blue, lane);
    }
}

pub fn spawn_minion_wave_for_team_lane(
    map_layout: &MapLayoutState,
    minions: &mut HashMap<u64, Minion>,
    next_minion_id: &mut u64,
    team: Team,
    lane: Lane,
) {
    let path = build_minion_path(map_layout, lane, team);
    if path.is_empty() {
        return;
    }
    let spawn = path[0];

    for wave_index in 0..MINIONS_PER_WAVE {
        let kind = if wave_index == 2 {
            MinionKind::Caster
        } else {
            MinionKind::Melee
        };
        let max_hp = minion_stats(kind).0;
        let minion_id = *next_minion_id;
        *next_minion_id += 1;

        let offset = wave_index as f32 * (MINION_RADIUS * 2.0 + 0.4);
        let mut spawn_x = spawn.x;
        let mut spawn_z = spawn.z;
        let mut yaw = 0.0;
        if let Some(next_point) = path.get(1) {
            let dir_x = next_point.x - spawn.x;
            let dir_z = next_point.z - spawn.z;
            let len_sq = dir_x * dir_x + dir_z * dir_z;
            if len_sq > 0.0001 {
                let inv_len = len_sq.sqrt().recip();
                spawn_x -= dir_x * inv_len * offset;
                spawn_z -= dir_z * inv_len * offset;
                yaw = shared::math::unit_yaw_towards(dir_x, dir_z);
            }
        }

        minions.insert(
            minion_id,
            Minion {
                state: MinionState {
                    kind,
                    attack_sequence: 0,
                    id: minion_id,
                    team,
                    lane,
                    x: spawn_x,
                    y: MINION_SPAWN_HEIGHT,
                    z: spawn_z,
                    yaw,
                    hp: max_hp,
                    max_hp,
                    state: MinionBrainState::Marching,
                    target_kind: None,
                    target_id: None,
                },
                path: path.clone(),
                next_waypoint: 1,
                last_attack_at: None,
                aggro_target: None,
            },
        );
    }
}

pub fn structure_radius(kind: StructureKind) -> f32 {
    match kind {
        StructureKind::Tower => TOWER_SIZE * 0.5,
        StructureKind::BaseTower => BASE_TOWER_SIZE * 0.5,
    }
}

/// Movement footprint, not attack-target reach.
pub fn structure_collision_radius(kind: StructureKind) -> f32 {
    match kind {
        StructureKind::Tower => shared::TOWER_TARGET_RADIUS,
        StructureKind::BaseTower => shared::navigation::BASE_COLLISION_RADIUS,
    }
}
