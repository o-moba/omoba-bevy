//! Regressions for side-lane corner spurs, using real wave spawn and movement.
use super::*;

/// Lane corridors stop at the base entry coordinate. The decorative corner
/// beyond that entry is not a destination a marching wave needs to visit.
fn forward_corridor(layout: &MapLayoutState, lane: Lane, team: Team) -> Vec<Vec3f> {
    let point = |x, z| Vec3f::new(x, MINION_SPAWN_HEIGHT, z);
    let home = point(layout.home.x, layout.home.z);
    let away = point(layout.away.x, layout.away.z);
    let mut points = match lane {
        Lane::Mid => vec![home, away],
        Lane::Top => vec![
            home,
            point(layout.left_x, layout.home.z),
            point(layout.left_x, layout.top_z),
            point(layout.away.x, layout.top_z),
            away,
        ],
        Lane::Bot => vec![
            home,
            point(layout.home.x, layout.bottom_z),
            point(layout.right_x, layout.bottom_z),
            point(layout.right_x, layout.away.z),
            away,
        ],
    };
    if team == Team::Blue {
        points.reverse();
    }
    points
}

fn planar_distance(a: Vec3f, b: Vec3f) -> f32 {
    (a.x - b.x).hypot(a.z - b.z)
}

/// Return distance from the expected corridor and progress along it. This
/// measures actual simulated positions independently of next_waypoint/path.
fn corridor_progress(points: &[Vec3f], position: Vec3f) -> (f32, f32) {
    let mut along = 0.0;
    let mut best = (f32::INFINITY, 0.0);
    for pair in points.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let dx = b.x - a.x;
        let dz = b.z - a.z;
        let length = dx.hypot(dz);
        if length <= 0.0001 {
            continue;
        }
        let t = (((position.x - a.x) * dx + (position.z - a.z) * dz) / (length * length))
            .clamp(0.0, 1.0);
        let projected = Vec3f::new(a.x + dx * t, position.y, a.z + dz * t);
        let distance = planar_distance(position, projected);
        if distance < best.0 {
            best = (distance, along + length * t);
        }
        along += length;
    }
    best
}

#[test]
fn every_wave_member_marches_forward_through_departure_and_far_base_approach() {
    let layout = build_map_layout();
    for team in [Team::Green, Team::Blue] {
        for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
            let mut minions = HashMap::new();
            spawn_minion_wave_for_team_lane(&layout, &mut minions, &mut 1, team, lane);
            assert_eq!(minions.len(), MINIONS_PER_WAVE);
            let corridor = forward_corridor(&layout, lane, team);
            let mut traces = minions
                .values()
                .map(|minion| {
                    let start = Vec3f::new(minion.state.x, minion.state.y, minion.state.z);
                    // Rear formation members begin behind the first corridor point.
                    let mut member_corridor = vec![start];
                    member_corridor.extend_from_slice(&corridor);
                    (minion.state.id, member_corridor, start, 0.0)
                })
                .collect::<Vec<_>>();
            let mut players = HashMap::new();
            let mut structures = HashMap::new();
            let mut state = GameState::Running;
            let started = Instant::now();
            let mut finished = false;
            for step in 1..=3000 {
                simulate_minions(
                    &mut players,
                    &mut minions,
                    &mut structures,
                    &mut state,
                    0.05,
                    started + Duration::from_millis(step * 50),
                );
                finished = true;
                for (id, expected, previous, previous_progress) in &mut traces {
                    let minion = &minions[&*id];
                    let current = Vec3f::new(minion.state.x, minion.state.y, minion.state.z);
                    let (distance, progress) = corridor_progress(expected, current);
                    // The existing waypoint arrival tolerance is 0.1 units.
                    assert!(
                        distance <= 0.12,
                        "{team:?}/{lane:?} member {id} step {step}: left the forward lane by {distance:.3} at ({:.3},{:.3}); waypoint {}",
                        current.x,
                        current.z,
                        minion.next_waypoint
                    );
                    assert!(
                        progress + 0.001 >= *previous_progress,
                        "{team:?}/{lane:?} member {id}: retraced progress {previous_progress:.3} -> {progress:.3}"
                    );
                    assert!(planar_distance(*previous, current) <= MINION_SPEED * 0.05 + 0.0001);
                    assert_eq!(minion.state.hp, MINION_MAX_HP);
                    assert_eq!(minion.state.y, MINION_SPAWN_HEIGHT);
                    assert_eq!(minion.state.state, MinionBrainState::Marching);
                    *previous = current;
                    *previous_progress = progress;
                    finished &= minion.next_waypoint == minion.path.len();
                }
                if finished {
                    break;
                }
            }
            assert!(
                finished,
                "{team:?}/{lane:?}: wave never reached the far base"
            );
            for minion in minions.values() {
                let current = Vec3f::new(minion.state.x, minion.state.y, minion.state.z);
                assert!(planar_distance(current, *corridor.last().unwrap()) <= 0.11);
            }
        }
    }
}

#[test]
fn ordered_paths_have_no_corner_reversal_and_remain_team_symmetric() {
    let layout = build_map_layout();
    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
        let green = build_minion_path(&layout, lane, Team::Green);
        let blue = build_minion_path(&layout, lane, Team::Blue);
        assert_eq!(green.len(), blue.len());
        for (a, b) in green.iter().zip(blue.iter().rev()) {
            assert!(planar_distance(*a, *b) < 0.0001);
        }
        for (team, path) in [(Team::Green, &green), (Team::Blue, &blue)] {
            for triple in path.windows(3) {
                let [a, b, c] = [triple[0], triple[1], triple[2]];
                let dot = (b.x - a.x) * (c.x - b.x) + (b.z - a.z) * (c.z - b.z);
                assert!(
                    dot >= 0.0,
                    "{team:?}/{lane:?}: a waypoint reverses the preceding leg"
                );
            }
        }
    }
    let top = build_minion_path(&layout, Lane::Top, Team::Green);
    let mirrored_bot = build_minion_path(&layout, Lane::Bot, Team::Blue);
    assert_eq!(top.len(), mirrored_bot.len());
    for (a, b) in top.iter().zip(mirrored_bot.iter()) {
        assert!((a.x + b.x).abs() < 0.0001 && (a.z + b.z).abs() < 0.0001);
    }
    let mid = build_minion_path(&layout, Lane::Mid, Team::Green);
    assert_eq!(mid.len(), 2);
    assert!((planar_distance(mid[0], mid[1]) - TARGET_BASE_DISTANCE).abs() < 0.001);
}

#[test]
fn spawn_formation_wave_cadence_and_authored_structure_anchors_are_preserved() {
    let layout = build_map_layout();
    for team in [Team::Green, Team::Blue] {
        for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
            let expected = forward_corridor(&layout, lane, team);
            let (base, first) = (expected[0], expected[1]);
            let length = planar_distance(base, first);
            let direction = ((first.x - base.x) / length, (first.z - base.z) / length);
            let mut minions = HashMap::new();
            spawn_minion_wave_for_team_lane(&layout, &mut minions, &mut 1, team, lane);
            for wave_index in 0..3 {
                let minion = &minions[&(wave_index + 1)];
                let offset = wave_index as f32 * 1.5;
                assert!((minion.state.x - (base.x - direction.0 * offset)).abs() < 0.001);
                assert!((minion.state.z - (base.z - direction.1 * offset)).abs() < 0.001);
                assert!((minion.state.yaw - direction.0.atan2(direction.1)).abs() < 0.001);
                assert_eq!(minion.next_waypoint, 1);
            }
        }
    }
    assert_eq!(MINIONS_PER_WAVE, 3);
    assert_eq!(MINION_SPEED, 3.1);
    assert_eq!(FIRST_MINION_WAVE_DELAY, Duration::from_secs(10));
    let now = Instant::now();
    let mut last_wave = now;
    let mut minions = HashMap::new();
    let mut next_id = 1;
    for (millis, expected_count) in [(59_999, 0), (60_000, 18), (119_999, 18), (120_000, 36)] {
        spawn_minion_waves_if_due(
            &layout,
            &mut minions,
            &mut next_id,
            &GameState::Running,
            now + Duration::from_millis(millis),
            &mut last_wave,
        );
        assert_eq!(minions.len(), expected_count);
    }

    // Baseline positions from the authored six-point outer lanes at fab5f23.
    // Towers must not move when the minion-only route drops its dead-end spur.
    let structures = build_structures(&layout);
    for (id, x, z) in [
        (1, -96.54951, 29.5099),
        (2, 4.49010, 96.54951),
        (3, -31.8198, -31.8198),
        (4, 31.8198, 31.8198),
        (5, -4.49010, -96.54951),
        (6, 96.54951, -29.5099),
        (7, -79.54951, -79.54951),
        (8, 79.54951, 79.54951),
    ] {
        let structure = &structures[&id].state;
        assert!((structure.x - x).abs() < 0.001 && (structure.z - z).abs() < 0.001);
    }
}
