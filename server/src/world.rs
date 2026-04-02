use super::*;

pub(crate) fn build_structures(layout: &MapLayoutState) -> HashMap<u64, Structure> {
    let mut structures = HashMap::new();
    let mut next_id: u64 = 1;

    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
        let lane_points = lane_control_points(layout, lane);
        let green_tower = sample_polyline_position(&lane_points, 0.30);
        let blue_tower = sample_polyline_position(&lane_points, 0.70);
        add_structure(
            &mut structures,
            &mut next_id,
            StructureKind::Tower,
            StructureRole::LaneTower { lane },
            Team::Green,
            Vec3f::new(green_tower.x, 3.0, green_tower.z),
        );
        add_structure(
            &mut structures,
            &mut next_id,
            StructureKind::Tower,
            StructureRole::LaneTower { lane },
            Team::Blue,
            Vec3f::new(blue_tower.x, 3.0, blue_tower.z),
        );
    }

    let home_nexus = Vec3f::new(layout.home.x, 4.0, layout.home.z);
    let away_nexus = Vec3f::new(layout.away.x, 4.0, layout.away.z);
    add_structure(
        &mut structures,
        &mut next_id,
        StructureKind::BaseTower,
        StructureRole::BaseTower,
        Team::Green,
        home_nexus,
    );
    add_structure(
        &mut structures,
        &mut next_id,
        StructureKind::BaseTower,
        StructureRole::BaseTower,
        Team::Blue,
        away_nexus,
    );

    structures
}

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
    let inner_side = TARGET_BASE_DISTANCE / 2.0_f32.sqrt();
    let half_inner_side = inner_side * 0.5;
    let base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
    let half_map_size = half_inner_side + base_padding;
    let home = Vec3f::new(-half_inner_side, 0.0, -half_inner_side);
    let away = Vec3f::new(half_inner_side, 0.0, half_inner_side);

    let lane_edge_offset = LANE_EDGE_PADDING + LANE_WIDTH * 0.5;
    let left_x = -half_map_size + lane_edge_offset;
    let right_x = half_map_size - lane_edge_offset;
    let top_z = half_map_size - lane_edge_offset;
    let bottom_z = -half_map_size + lane_edge_offset;

    MapLayoutState {
        home,
        away,
        left_x,
        right_x,
        top_z,
        bottom_z,
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

pub(crate) fn lane_control_points(layout: &MapLayoutState, lane: Lane) -> Vec<Vec3f> {
    match lane {
        Lane::Mid => vec![layout.home, layout.away],
        Lane::Top => vec![
            layout.home,
            Vec3f::new(layout.left_x, 0.0, layout.home.z),
            Vec3f::new(layout.left_x, 0.0, layout.top_z),
            Vec3f::new(layout.right_x, 0.0, layout.top_z),
            Vec3f::new(layout.away.x, 0.0, layout.top_z),
            layout.away,
        ],
        Lane::Bot => vec![
            layout.home,
            Vec3f::new(layout.home.x, 0.0, layout.bottom_z),
            Vec3f::new(layout.left_x, 0.0, layout.bottom_z),
            Vec3f::new(layout.right_x, 0.0, layout.bottom_z),
            Vec3f::new(layout.right_x, 0.0, layout.away.z),
            layout.away,
        ],
    }
}

pub(crate) fn sample_polyline_position(points: &[Vec3f], t: f32) -> Vec3f {
    if points.len() <= 1 {
        return points.first().copied().unwrap_or(Vec3f::new(0.0, 0.0, 0.0));
    }

    let segment_lengths = points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .collect::<Vec<_>>();
    let total_length: f32 = segment_lengths.iter().sum();
    if total_length <= 0.0001 {
        return points[0];
    }

    let mut remaining = total_length * t.clamp(0.0, 1.0);
    for (index, length) in segment_lengths.into_iter().enumerate() {
        if remaining <= length {
            let local_t = if length <= 0.0001 {
                0.0
            } else {
                remaining / length
            };
            return points[index].lerp(points[index + 1], local_t);
        }
        remaining -= length;
    }

    points.last().copied().unwrap_or(points[0])
}

pub(crate) fn build_minion_path(layout: &MapLayoutState, lane: Lane, team: Team) -> Vec<Vec3f> {
    let mut points = lane_control_points(layout, lane);
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
                    id: minion_id,
                    team,
                    lane,
                    x: spawn_x,
                    y: MINION_SPAWN_HEIGHT,
                    z: spawn_z,
                    yaw,
                    hp: MINION_MAX_HP,
                    max_hp: MINION_MAX_HP,
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
