//! Configuration is exercised through the actual simulation and packet handler.
use super::*;
use shared::map::{MapDefinition, Placement, ResolvedMap};

fn example_definition() -> MapDefinition {
    MapDefinition::from_json(include_str!("../../examples/maps/two-tier.json")).unwrap()
}
fn example() -> ResolvedMap {
    example_definition().resolve().unwrap()
}
fn runtime(config: ResolvedMap) -> ServerRuntime {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    ServerRuntime::new_with_map(socket, MatchConfig::dev(), config)
}
fn mid_only() -> HashMap<u64, Structure> {
    let mut definition = example_definition();
    definition.structures.retain(|s| {
        matches!(
            s.placement,
            Placement::Base
                | Placement::Tower {
                    lane: shared::map::Lane::Mid,
                    ..
                }
        )
    });
    build_configured_structures(&definition.resolve().unwrap())
}

#[test]
fn default_and_example_preserve_ids_stats_and_reachable_placement() {
    let layout = build_map_layout();
    let defaults = build_structures(&layout);
    assert_eq!(defaults.len(), 8);
    for id in 1..=8 {
        let s = &defaults[&id];
        assert_eq!(s.state.hp, if id <= 6 { 240.0 } else { 650.0 });
        assert_eq!(s.attack_range, if id <= 6 { 20.0 } else { 24.0 });
        assert_eq!(s.attack_damage, if id <= 6 { 14.0 } else { 18.0 });
        assert_eq!(
            s.attack_cooldown,
            Duration::from_millis(if id <= 6 { 900 } else { 850 })
        );
    }
    let config = example();
    assert_eq!(config.structures.len(), 10);
    let configured = build_configured_structures(&config);
    assert_eq!(configured[&3].state.hp, 300.0);
    assert_eq!(configured[&9].state.hp, 420.0);
    assert_eq!(
        (configured[&3].state.tier, configured[&9].state.tier),
        (0, 1)
    );
    assert_eq!(configured[&3].state.lane, Some(shared::map::Lane::Mid));
    let expected = shared::map::sample_lane(shared::map::Lane::Mid, 0.36);
    assert_eq!([configured[&3].state.x, configured[&3].state.z], expected);
    for config in [ResolvedMap::default(), config] {
        let discs: Vec<_> = config
            .structures
            .iter()
            .map(|s| shared::navigation::Disc {
                center: s.position,
                radius: if s.lane.is_some() { 1.3 } else { 3.2 },
            })
            .collect();
        for structure in &config.structures {
            if structure.lane.is_none() {
                continue;
            }
            let spawn = spawn_position_for_team(
                &layout,
                if structure.team == shared::map::Team::Green {
                    Team::Green
                } else {
                    Team::Blue
                },
            );
            let reachable = (0..8).any(|index| {
                let angle = index as f32 * std::f32::consts::FRAC_PI_4;
                let goal = [
                    structure.position[0] + angle.cos() * 4.0,
                    structure.position[1] + angle.sin() * 4.0,
                ];
                shared::navigation::world_navigation()
                    .plan_route([spawn.x, spawn.z], goal, &discs)
                    .is_some()
            });
            assert!(
                reachable,
                "{} has no reachable attack approach",
                structure.key
            );
        }
    }
}

#[test]
fn configured_tiers_gate_damage_then_base_and_ignore_empty_lanes() {
    for (outer, inner, base, attacker) in [(3, 9, 7, Team::Blue), (4, 10, 8, Team::Green)] {
        let mut structures = mid_only();
        let mut state = GameState::Running;
        assert!(!structure_is_protected(&structures, outer));
        assert!(structure_is_protected(&structures, inner));
        assert!(
            structure_is_protected(&structures, base),
            "empty side lanes cannot unlock a defended map"
        );
        assert!(
            apply_structure_damage(&mut structures, inner, 1000.0, attacker, &mut state).is_none()
        );
        assert!(
            apply_structure_damage(&mut structures, base, 1000.0, attacker, &mut state).is_none()
        );
        let receipt =
            apply_structure_damage(&mut structures, outer, 1000.0, attacker, &mut state).unwrap();
        assert_eq!(receipt.amount, 300.0);
        assert!(!structure_is_protected(&structures, inner));
        assert!(structure_is_protected(&structures, base));
        assert!(
            apply_structure_damage(&mut structures, inner, 1000.0, attacker, &mut state)
                .unwrap()
                .killed
        );
        assert!(!structure_is_protected(&structures, base));
        assert!(
            apply_structure_damage(&mut structures, base, 1000.0, attacker, &mut state)
                .unwrap()
                .killed
        );
        assert!(matches!(state,GameState::Victory{winner} if winner==attacker));
    }
    let mut config = example_definition();
    config
        .structures
        .retain(|s| matches!(s.placement, Placement::Base));
    let structures = build_configured_structures(&config.resolve().unwrap());
    assert!(!structure_is_protected(&structures, 7));
    assert!(!structure_is_protected(&structures, 8));
    // With all lanes present, clearing just the single-tower top lane suffices.
    let mut structures = build_configured_structures(&example());
    structures.get_mut(&1).unwrap().state.hp = 0.0;
    assert!(!structure_is_protected(&structures, 7));
    assert!(structure_is_protected(&structures, 8));
}

#[test]
fn minions_target_front_tier_even_when_nearer_to_inner_or_base() {
    let layout = build_map_layout();
    let mut structures = mid_only();
    let mut minions = HashMap::new();
    spawn_minion_wave_for_team_lane(&layout, &mut minions, &mut 1, Team::Blue, Lane::Mid);
    minions.retain(|&id, _| id == 1);
    let now = Instant::now();
    for (dead, expected) in [(None, 3), (Some(3), 9), (Some(9), 7)] {
        if let Some(id) = dead {
            structures.get_mut(&id).unwrap().state.hp = 0.0;
        }
        let minion = minions.get_mut(&1).unwrap();
        minion.state.x = layout.home.x + 3.0;
        minion.state.z = layout.home.z + 3.0;
        simulate_minions(
            &mut HashMap::new(),
            &mut minions,
            &mut structures,
            &mut HashMap::new(),
            &mut 1,
            &mut GameState::Running,
            0.0,
            now,
        );
        assert_eq!(minions[&1].state.target_id, Some(expected));
    }
}

#[test]
fn instance_threat_overrides_drive_actual_range_damage_and_cooldown() {
    let mut definition = example_definition();
    let outer = definition
        .structures
        .iter_mut()
        .find(|s| s.id == 3)
        .unwrap();
    outer.overrides.attack_range = Some(6.0);
    outer.overrides.attack_damage = Some(17.0);
    outer.overrides.attack_cooldown_ms = Some(1250);
    let mut structures = build_configured_structures(&definition.resolve().unwrap());
    structures.retain(|&id, _| id == 3);
    let position = structures[&3].state.clone();
    let mut minions = HashMap::new();
    spawn_minion_wave_for_team_lane(
        &build_map_layout(),
        &mut minions,
        &mut 1,
        Team::Blue,
        Lane::Mid,
    );
    minions.retain(|&id, _| id == 1);
    let target = minions.get_mut(&1).unwrap();
    target.state.x = position.x + 7.0;
    target.state.z = position.z;
    let hp = target.state.hp;
    let now = Instant::now();
    let mut players = HashMap::new();
    let mut projectiles = HashMap::new();
    let mut next = 1;
    assert!(
        simulate_tower_attacks(
            &mut players,
            &mut minions,
            &mut projectiles,
            &mut structures,
            &mut next,
            &GameState::Running,
            now
        )
        .is_empty()
    );
    minions.get_mut(&1).unwrap().state.x = position.x + 3.0;
    let receipts = simulate_tower_attacks(
        &mut players,
        &mut minions,
        &mut projectiles,
        &mut structures,
        &mut next,
        &GameState::Running,
        now,
    );
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].amount, 17.0);
    assert_eq!(minions[&1].state.hp, hp - 17.0);
    assert!(
        simulate_tower_attacks(
            &mut players,
            &mut minions,
            &mut projectiles,
            &mut structures,
            &mut next,
            &GameState::Running,
            now + Duration::from_millis(1249)
        )
        .is_empty()
    );
    assert_eq!(
        simulate_tower_attacks(
            &mut players,
            &mut minions,
            &mut projectiles,
            &mut structures,
            &mut next,
            &GameState::Running,
            now + Duration::from_millis(1250)
        )
        .len(),
        1
    );
}

#[test]
fn packet_transform_respects_configured_live_discs_and_death_removes_blocker() {
    let mut rt = runtime(example());
    rt.game_state = GameState::Running;
    let address: SocketAddr = "127.0.0.1:57931".parse().unwrap();
    let mut now = Instant::now();
    ensure_player_connected(
        &mut rt.players,
        &rt.map_layout,
        address,
        &mut rt.next_player_id,
        now,
    );
    let center = [rt.structures[&3].state.x, rt.structures[&3].state.z];
    let start = [center[0] - 5.0, center[1]];
    let end = [center[0] + 5.0, center[1]];
    assert!(shared::navigation::world_navigation().segment_clear(start, end));
    let player = rt.players.get_mut(&address).unwrap();
    player.joined = true;
    player.state.x = start[0];
    player.state.z = start[1];
    player.speed_mult = 100.0;
    for _ in 0..4 {
        now += Duration::from_millis(100);
        rt.handle_packet(
            address,
            ClientPacket::Transform {
                x: end[0],
                y: 0.0,
                z: end[1],
                yaw: 0.0,
            },
            now,
        );
        let p = &rt.players[&address].state;
        assert!(p.x < center[0] - 1.79);
        assert!(
            p.x > start[0] + 1.0,
            "test movement must reach the actual tower surface"
        );
    }
    rt.structures.get_mut(&3).unwrap().state.hp = 0.0;
    now += Duration::from_millis(100);
    rt.handle_packet(
        address,
        ClientPacket::Transform {
            x: end[0],
            y: 0.0,
            z: end[1],
            yaw: 0.0,
        },
        now,
    );
    assert!((rt.players[&address].state.x - end[0]).abs() < 0.001);
}

#[test]
fn pinned_configuration_reconstructs_same_round_objects_and_identity() {
    let config = example();
    let mut rt = runtime(config);
    let before: Vec<_> = rt
        .map_config
        .structures
        .iter()
        .map(|s| {
            (
                s.id,
                serde_json::to_value(&rt.structures[&s.id].state).unwrap(),
            )
        })
        .collect();
    for s in rt.structures.values_mut() {
        s.state.hp = 0.0;
        s.state.x = 0.0;
        s.attack_range = 1.0;
        s.last_attack_at = Some(Instant::now());
    }
    rt.restart_round(Instant::now());
    assert_eq!(rt.map_config.map_profile, "verdant_two_tier_example");
    assert_eq!(rt.map_config.geometry_id, shared::map::GEOMETRY_ID);
    assert_eq!(rt.structures.len(), 10);
    for (id, state) in before {
        assert_eq!(
            serde_json::to_value(&rt.structures[&id].state).unwrap(),
            state
        );
        assert!(rt.structures[&id].last_attack_at.is_none());
    }
    assert_eq!(rt.structures[&9].attack_range, 20.0);
    assert_eq!(rt.match_id, 2);
}

#[test]
fn startup_loader_fails_explicit_invalid_path_and_resolves_example() {
    assert!(
        load_map_config(Some(std::path::Path::new("/no-such-omoba-map.json")))
            .unwrap_err()
            .to_string()
            .contains("OMOBA_MAP_CONFIG")
    );
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/maps/two-tier.json");
    assert_eq!(load_map_config(Some(&path)).unwrap().structures.len(), 10);
}

#[test]
fn melee_march_hits_maximum_lateral_offset_towers_on_both_teams() {
    let layout = build_map_layout();
    for (attacker, target_id) in [(Team::Green, 4), (Team::Blue, 3)] {
        let mut definition = MapDefinition::from_json(shared::map::DEFAULT_JSON).unwrap();
        let tower = definition
            .structures
            .iter_mut()
            .find(|s| s.id == target_id)
            .unwrap();
        let Placement::Tower { offset, .. } = &mut tower.placement else {
            unreachable!()
        };
        // Exactly three meters perpendicular to the diagonal lane. Before
        // horizontal reach, the 2.5m box-center height made this unreachable.
        *offset = [2.12132, -2.12132];
        let mut structures = build_configured_structures(&definition.resolve().unwrap());
        let tower_position = structures[&target_id].state.clone();
        let mut minions = HashMap::new();
        spawn_minion_wave_for_team_lane(&layout, &mut minions, &mut 1, attacker, Lane::Mid);
        minions.retain(|&id, _| id == 1);
        assert_eq!(minions[&1].state.kind, MinionKind::Melee);
        let mut players = HashMap::new();
        let mut projectiles = HashMap::new();
        let mut next_projectile = 1;
        let mut game_state = GameState::Running;
        let now = Instant::now();
        let mut struck = false;
        for step in 1..=1000 {
            let receipts = simulate_minions(
                &mut players,
                &mut minions,
                &mut structures,
                &mut projectiles,
                &mut next_projectile,
                &mut game_state,
                0.1,
                now + Duration::from_millis(step * 100),
            );
            if let Some(receipt) = receipts
                .iter()
                .find(|r| r.target.kind == CombatEntityKind::Structure && r.target.id == target_id)
            {
                assert_eq!(receipt.source.kind, CombatEntityKind::Minion);
                assert_eq!(receipt.source.id, 1);
                assert_eq!(receipt.amount, MINION_ATTACK_DAMAGE);
                struck = true;
                break;
            }
        }
        assert!(
            struck,
            "{attacker:?} melee marched past the offset tower without striking"
        );
        assert_eq!(
            structures[&target_id].state.hp,
            tower_position.hp - MINION_ATTACK_DAMAGE
        );
        assert!(projectiles.is_empty(), "melee damage remains direct");
        assert!(minions[&1].state.attack_sequence > 0);
        // Caster keeps its longer range and still emits a travelling projectile
        // aimed at the target's Y, although range is measured on the ground.
        minions.clear();
        spawn_minion_wave_for_team_lane(&layout, &mut minions, &mut 1, attacker, Lane::Mid);
        minions.retain(|&id, _| id == 3);
        let caster = minions.get_mut(&3).unwrap();
        caster.state.x = tower_position.x + 6.0;
        caster.state.z = tower_position.z;
        let receipts = simulate_minions(
            &mut players,
            &mut minions,
            &mut structures,
            &mut projectiles,
            &mut next_projectile,
            &mut game_state,
            0.0,
            now,
        );
        assert!(
            receipts.is_empty(),
            "caster damage must await projectile impact"
        );
        assert_eq!(projectiles.len(), 1);
        let projectile = projectiles.values().next().unwrap();
        assert_eq!(projectile.state.style, ProjectileStyle::CasterBolt);
        assert_eq!(projectile.target.id, target_id);
        assert_eq!(projectile.target.kind, TargetKind::Structure);
    }
}
