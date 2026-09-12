use super::*;

#[cfg(test)]
pub(crate) fn build_structures(_layout: &MapLayoutState) -> HashMap<u64, Structure> {
    build_configured_structures(&shared::map::ResolvedMap::default())
}

pub(crate) fn load_map_config(
    path: Option<&std::path::Path>,
) -> io::Result<shared::map::ResolvedMap> {
    let Some(path) = path else {
        return Ok(shared::map::ResolvedMap::default());
    };
    use std::io::Read;
    let mut source = String::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(shared::map::MAX_CONFIG_BYTES as u64 + 1)
                .read_to_string(&mut source)
        })
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("OMOBA_MAP_CONFIG {}: {error}", path.display()),
            )
        })?;
    shared::map::MapDefinition::from_json(&source)
        .and_then(|map| map.resolve())
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("OMOBA_MAP_CONFIG {}: {error}", path.display()),
            )
        })
}

pub(crate) fn build_configured_structures(
    config: &shared::map::ResolvedMap,
) -> HashMap<u64, Structure> {
    config
        .structures
        .iter()
        .map(|item| {
            let team = match item.team {
                shared::map::Team::Green => Team::Green,
                shared::map::Team::Blue => Team::Blue,
            };
            let (kind, role, y) = match item.lane {
                Some(lane) => (
                    StructureKind::Tower,
                    StructureRole::LaneTower {
                        lane: server_lane(lane),
                    },
                    3.0,
                ),
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
                    attack_cooldown: Duration::from_millis(item.stats.attack_cooldown_ms),
                },
            )
        })
        .collect()
}

fn server_lane(lane: shared::map::Lane) -> Lane {
    match lane {
        shared::map::Lane::Top => Lane::Top,
        shared::map::Lane::Mid => Lane::Mid,
        shared::map::Lane::Bot => Lane::Bot,
    }
}
fn shared_lane(lane: Lane) -> shared::map::Lane {
    match lane {
        Lane::Top => shared::map::Lane::Top,
        Lane::Mid => shared::map::Lane::Mid,
        Lane::Bot => shared::map::Lane::Bot,
    }
}

#[cfg(test)]
pub(crate) fn add_structure(
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
                    StructureRole::LaneTower { lane } => Some(shared_lane(lane)),
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
            attack_cooldown,
        },
    );
}

pub(crate) fn build_map_layout() -> MapLayoutState {
    let g = shared::map::geometry();
    MapLayoutState {
        home: Vec3f::new(g.home[0], 0.0, g.home[1]),
        away: Vec3f::new(g.away[0], 0.0, g.away[1]),
        min_x: g.bounds.min[0],
        max_x: g.bounds.max[0],
        min_z: g.bounds.min[1],
        max_z: g.bounds.max[1],
        #[cfg(test)]
        left_x: g.left_x,
        #[cfg(test)]
        right_x: g.right_x,
        #[cfg(test)]
        top_z: g.top_z,
        #[cfg(test)]
        bottom_z: g.bottom_z,
    }
}

pub(crate) fn spawn_position_for_team(map_layout: &MapLayoutState, team: Team) -> Vec3f {
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

pub(crate) fn spawn_position_for_team_from_base(
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

#[cfg(test)]
pub(crate) fn lane_control_points(_layout: &MapLayoutState, lane: Lane) -> Vec<Vec3f> {
    shared::map::lane_points(shared_lane(lane))
        .into_iter()
        .map(|point| Vec3f::new(point[0], 0.0, point[1]))
        .collect()
}

pub(crate) fn build_minion_path(_layout: &MapLayoutState, lane: Lane, team: Team) -> Vec<Vec3f> {
    let mut points: Vec<Vec3f> = shared::map::minion_lane_points(shared_lane(lane))
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

pub(crate) fn spawn_minion_waves_if_due(
    map_layout: &MapLayoutState,
    minions: &mut HashMap<u64, Minion>,
    next_minion_id: &mut u64,
    game_state: &GameState,
    now: Instant,
    last_wave_spawn_at: &mut Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    if now.duration_since(*last_wave_spawn_at) < MINION_WAVE_INTERVAL {
        return;
    }
    *last_wave_spawn_at = now;

    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
        spawn_minion_wave_for_team_lane(map_layout, minions, next_minion_id, Team::Green, lane);
        spawn_minion_wave_for_team_lane(map_layout, minions, next_minion_id, Team::Blue, lane);
    }
}

pub(crate) fn spawn_minion_wave_for_team_lane(
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
                yaw = dir_x.atan2(dir_z);
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

pub(crate) fn structure_radius(kind: StructureKind) -> f32 {
    match kind {
        StructureKind::Tower => TOWER_SIZE * 0.5,
        StructureKind::BaseTower => BASE_TOWER_SIZE * 0.5,
    }
}
