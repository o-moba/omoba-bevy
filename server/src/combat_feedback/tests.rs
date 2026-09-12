use super::*;

fn fixture() -> (ServerRuntime, SocketAddr, SocketAddr, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    rt.targeting_qa = true;
    let now = Instant::now();
    let a = "127.0.0.1:58201".parse().unwrap();
    let b = "127.0.0.1:58202".parse().unwrap();
    for (addr, team, x) in [(a, Team::Green, 0.0), (b, Team::Blue, 4.0)] {
        rt.handle_packet(
            addr,
            ClientPacket::Join {
                team,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
                session_id: None,
                passport_ticket: None,
            },
            now,
        );
        let player = rt.players.get_mut(&addr).unwrap();
        player.state.x = x;
        player.state.z = 0.0;
    }
    (rt, a, b, now)
}

fn projectiles(rt: &mut ServerRuntime, now: Instant, dt: f32) -> Vec<CombatEvent> {
    simulate_projectiles(
        &mut rt.players,
        &mut rt.minions,
        &mut rt.structures,
        &mut rt.neutrals,
        &mut rt.team_buffs,
        &mut rt.projectiles,
        &mut rt.game_state,
        dt,
        now,
    )
}

#[test]
fn every_class_keeps_basic_and_skill_travel_with_confirmed_source_style_and_slot() {
    for class in HeroClass::ALL {
        for slot in [None, Some(0)] {
            let (mut rt, a, b, now) = fixture();
            rt.players.get_mut(&a).unwrap().state.hero_class = class;
            let target = TargetId {
                kind: TargetKind::Player,
                id: rt.players[&b].state.id,
            };
            let packet = if let Some(slot) = slot {
                ClientPacket::Cast { target, slot }
            } else {
                ClientPacket::BasicAttack {
                    target,
                    server_epoch: rt.server_epoch,
                    match_id: rt.match_id,
                    request_id: 1,
                }
            };
            rt.handle_packet(a, packet, now);
            let projectile = rt.projectiles.values().next().expect("accepted attack");
            assert_eq!(projectile.state.source_kind, CombatEntityKind::Player);
            assert_eq!(projectile.state.style, ProjectileStyle::for_class(class));
            assert_eq!(
                projectile.state.action_slot,
                Some(slot.unwrap_or(shared::BASIC_ATTACK_ACTION_SLOT))
            );
            assert!(projectile.state.direction.iter().all(|v| v.is_finite()));
            assert!(projectile.state.direction[0] > 0.99);
            let expected_damage = projectile.damage;
            assert_eq!(rt.players[&b].state.hp, MAX_HP);
            assert!(projectiles(&mut rt, now + Duration::from_millis(10), 0.01).is_empty());
            assert_eq!(
                rt.players[&b].state.hp, MAX_HP,
                "even Warrior retains travel"
            );
            let events = projectiles(&mut rt, now + Duration::from_millis(250), 0.24);
            assert_eq!(events.len(), 1);
            let event = &events[0];
            assert_eq!(
                event.source,
                CombatEntity {
                    kind: CombatEntityKind::Player,
                    id: rt.players[&a].state.id
                }
            );
            assert_eq!(
                event.target,
                CombatEntity {
                    kind: CombatEntityKind::Player,
                    id: target.id
                }
            );
            assert_eq!(event.style, ProjectileStyle::for_class(class));
            assert!((event.amount - expected_damage).abs() < 0.001);
            assert_eq!(
                event.action_slot,
                Some(slot.unwrap_or(shared::BASIC_ATTACK_ACTION_SLOT))
            );
            assert!(!event.killed);
            assert!(projectiles(&mut rt, now + Duration::from_millis(300), 0.05).is_empty());
        }
    }
}

#[test]
fn receipts_use_actual_damage_and_reject_overkill_repeats_protection_and_immunity() {
    let (mut rt, a, b, now) = fixture();
    let player_id = rt.players[&b].state.id;
    rt.players.get_mut(&b).unwrap().god_mode = true;
    assert!(apply_player_damage(&mut rt.players, player_id, 500.0, now).is_none());
    rt.players.get_mut(&b).unwrap().god_mode = false;
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(apply_player_damage(&mut rt.players, player_id, bad, now).is_none());
    }
    rt.players.get_mut(&b).unwrap().state.hp = 3.0;
    let event = apply_player_damage(&mut rt.players, player_id, 500.0, now).unwrap();
    assert_eq!(event.amount, 3.0);
    assert!(event.killed);
    assert!(apply_player_damage(&mut rt.players, player_id, 500.0, now).is_none());

    let base = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::BaseTower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .id;
    assert!(
        apply_structure_damage(
            &mut rt.structures,
            base,
            999.0,
            Team::Green,
            &mut rt.game_state
        )
        .is_none()
    );
    let tower = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .id;
    assert!(
        apply_structure_damage(
            &mut rt.structures,
            tower,
            1.0,
            Team::Blue,
            &mut rt.game_state
        )
        .is_none()
    );
    rt.structures.get_mut(&tower).unwrap().state.hp = 2.0;
    let event = apply_structure_damage(
        &mut rt.structures,
        tower,
        999.0,
        Team::Green,
        &mut rt.game_state,
    )
    .unwrap();
    assert_eq!(
        (event.target.kind, event.amount, event.killed),
        (CombatEntityKind::Structure, 2.0, true)
    );

    spawn_minion_wave_for_team_lane(
        &rt.map_layout,
        &mut rt.minions,
        &mut rt.next_minion_id,
        Team::Blue,
        Lane::Mid,
    );
    let id = *rt.minions.keys().min().unwrap();
    rt.minions.get_mut(&id).unwrap().state.hp = 1.0;
    assert!(apply_minion_damage(&mut rt.players, &mut rt.minions, id, 999.0, Team::Blue).is_none());
    let event =
        apply_minion_damage(&mut rt.players, &mut rt.minions, id, 999.0, Team::Green).unwrap();
    assert_eq!(
        (event.target.kind, event.amount, event.killed),
        (CombatEntityKind::Minion, 1.0, true)
    );
    let rewards = (rt.players[&a].state.gold, rt.players[&a].state.xp);
    assert!(
        apply_minion_damage(&mut rt.players, &mut rt.minions, id, 999.0, Team::Green).is_none()
    );
    assert_eq!(
        rewards,
        (rt.players[&a].state.gold, rt.players[&a].state.xp)
    );

    let id = *rt.neutrals.keys().min().unwrap();
    rt.neutrals.get_mut(&id).unwrap().state.hp = 4.0;
    let attacker = rt.players[&a].state.id;
    let event = apply_neutral_damage(
        &mut rt.players,
        &mut rt.neutrals,
        &mut rt.team_buffs,
        id,
        999.0,
        attacker,
        now,
    )
    .unwrap();
    assert_eq!(
        (event.target.kind, event.amount, event.killed),
        (CombatEntityKind::Neutral, 4.0, true)
    );
    assert!(
        apply_neutral_damage(
            &mut rt.players,
            &mut rt.neutrals,
            &mut rt.team_buffs,
            id,
            999.0,
            attacker,
            now
        )
        .is_none()
    );
}

#[test]
fn mixed_waves_are_symmetric_and_caster_uses_range_cooldown_and_real_impact() {
    let (mut rt, a, b, now) = fixture();
    rt.players.get_mut(&b).unwrap().state.x = 40.0;
    for team in [Team::Green, Team::Blue] {
        for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
            let mut wave = HashMap::new();
            spawn_minion_wave_for_team_lane(&rt.map_layout, &mut wave, &mut 1, team, lane);
            assert_eq!(wave.len(), 3);
            assert_eq!(
                wave.values()
                    .filter(|m| m.state.kind == MinionKind::Melee)
                    .count(),
                2
            );
            let caster = wave
                .values()
                .find(|m| m.state.kind == MinionKind::Caster)
                .unwrap();
            assert_eq!(caster.state.hp, 45.0);
            assert_eq!(caster.state.id, 3, "caster follows two melee members");
        }
    }
    spawn_minion_wave_for_team_lane(
        &rt.map_layout,
        &mut rt.minions,
        &mut rt.next_minion_id,
        Team::Blue,
        Lane::Mid,
    );
    rt.minions.retain(|_, m| m.state.kind == MinionKind::Caster);
    let caster = rt.minions.values_mut().next().unwrap();
    caster.state.x = 7.0;
    caster.state.z = 0.0;
    let id = caster.state.id;
    rt.structures.clear();
    let tick = |rt: &mut ServerRuntime, time| {
        simulate_minions(
            &mut rt.players,
            &mut rt.minions,
            &mut rt.structures,
            &mut rt.projectiles,
            &mut rt.next_projectile_id,
            &mut rt.game_state,
            0.01,
            time,
        )
    };
    assert!(
        tick(&mut rt, now).is_empty(),
        "caster release does not cause instant damage"
    );
    assert_eq!(rt.minions[&id].state.x, 7.0, "caster holds firing range");
    assert_eq!(rt.minions[&id].state.attack_sequence, 1);
    assert_eq!(rt.projectiles.len(), 1);
    assert_eq!(
        rt.projectiles.values().next().unwrap().state.source_kind,
        CombatEntityKind::Minion
    );
    assert_eq!(rt.players[&a].state.hp, MAX_HP);
    assert!(projectiles(&mut rt, now + Duration::from_millis(10), 0.01).is_empty());
    let events = projectiles(&mut rt, now + Duration::from_millis(400), 0.39);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].source,
        CombatEntity {
            kind: CombatEntityKind::Minion,
            id
        }
    );
    assert_eq!(events[0].style, ProjectileStyle::CasterBolt);
    assert_eq!(events[0].amount, 7.0);
    tick(&mut rt, now + Duration::from_millis(1199));
    assert!(rt.projectiles.is_empty());
    tick(&mut rt, now + Duration::from_millis(1200));
    assert_eq!(rt.projectiles.len(), 1);
    assert_eq!(rt.minions[&id].state.attack_sequence, 2);
}

#[test]
fn recent_receipts_are_bounded_repeated_for_loss_and_cleared_on_round_reset() {
    let (mut rt, _, b, now) = fixture();
    let id = rt.players[&b].state.id;
    let event = apply_player_damage(&mut rt.players, id, 1.0, now).unwrap();
    for _ in 0..120 {
        rt.combat_log.extend(now, [event.clone()]);
    }
    let events = rt.combat_log.snapshot(now);
    assert_eq!(events.len(), 96);
    assert_eq!((events[0].id, events[95].id), (25, 120));
    assert_eq!(
        rt.combat_log.snapshot(now + Duration::from_millis(999)),
        events
    );
    assert!(
        rt.combat_log
            .snapshot(now + Duration::from_secs(1))
            .is_empty()
    );
    rt.combat_log.extend(now + Duration::from_secs(1), [event]);
    assert_eq!(
        rt.combat_log.snapshot(now + Duration::from_secs(1))[0].id,
        121
    );
    rt.restart_round(now + Duration::from_secs(2));
    assert!(
        rt.combat_log
            .snapshot(now + Duration::from_secs(2))
            .is_empty()
    );
}

#[test]
fn ecs_projectile_minion_receipts_preserve_nonplayer_identity_and_overkill_once() {
    let (mut rt, _, _, now) = fixture();
    spawn_minion_wave_for_team_lane(
        &rt.map_layout,
        &mut rt.minions,
        &mut rt.next_minion_id,
        Team::Green,
        Lane::Mid,
    );
    let mut caster = rt.minions.remove(&3).unwrap();
    caster.state.x = 0.0;
    caster.state.z = 0.0;
    let target = rt.minions.get_mut(&1).unwrap();
    target.state.team = Team::Blue;
    target.state.x = 4.0;
    target.state.z = 0.0;
    target.state.hp = 3.0;
    let position = Vec3f::new(4.0, target.state.y, 0.0);
    // Two in-flight hits must not duplicate a kill, reward, or receipt.
    for _ in 0..2 {
        spawn_caster_projectile(
            &caster,
            TargetId {
                kind: TargetKind::Minion,
                id: 1,
            },
            position,
            &mut rt.projectiles,
            &mut rt.next_projectile_id,
            now,
        );
    }
    let mut app = App::new();
    app.add_plugins(GameplayPlugin)
        .insert_resource(rt)
        .insert_resource(TickContext {
            now: Some(now),
            dt: 0.01,
        })
        .add_systems(
            Update,
            (
                gameplay::combat::sync_minions_into_ecs_system,
                gameplay::combat::collect_projectile_minion_damage_system,
                gameplay::combat::apply_projectile_minion_damage_system,
            )
                .chain(),
        );
    app.update();
    assert_eq!(
        app.world().resource::<ServerRuntime>().minions[&1].state.hp,
        3.0
    );
    app.world_mut().resource_mut::<TickContext>().dt = 0.3;
    app.update();
    let mut rt = app.world_mut().resource_mut::<ServerRuntime>();
    let events = rt.combat_log.snapshot(now);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].source,
        CombatEntity {
            kind: CombatEntityKind::Minion,
            id: caster.state.id
        }
    );
    assert_eq!(
        events[0].target,
        CombatEntity {
            kind: CombatEntityKind::Minion,
            id: 1
        }
    );
    assert_eq!(events[0].style, ProjectileStyle::CasterBolt);
    assert_eq!(events[0].amount, 3.0);
    assert!(events[0].killed);
}

#[test]
fn ambient_melee_and_tower_sources_are_recorded_at_their_actual_damage_sinks() {
    let (mut rt, a, b, now) = fixture();
    rt.players.get_mut(&b).unwrap().state.x = 100.0;
    spawn_minion_wave_for_team_lane(
        &rt.map_layout,
        &mut rt.minions,
        &mut rt.next_minion_id,
        Team::Blue,
        Lane::Mid,
    );
    rt.minions.retain(|id, _| *id == 1);
    let minion = rt.minions.get_mut(&1).unwrap();
    minion.state.x = 1.0;
    minion.state.z = 0.0;
    let events = simulate_minions(
        &mut rt.players,
        &mut rt.minions,
        &mut HashMap::new(),
        &mut rt.projectiles,
        &mut rt.next_projectile_id,
        &mut rt.game_state,
        0.01,
        now,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].source,
        CombatEntity {
            kind: CombatEntityKind::Minion,
            id: 1
        }
    );
    assert_eq!(events[0].amount, 8.0);

    let neutral_id = *rt.neutrals.keys().min().unwrap();
    rt.neutrals.retain(|id, _| *id == neutral_id);
    let neutral = rt.neutrals.get_mut(&neutral_id).unwrap();
    neutral.anchor = Vec3f::new(1.0, neutral.state.y, 0.0);
    reset_neutral_at_anchor(neutral);
    let events = simulate_neutrals(&mut rt.players, &mut rt.neutrals, &rt.game_state, 0.01, now);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].source,
        CombatEntity {
            kind: CombatEntityKind::Neutral,
            id: neutral_id
        }
    );
    assert_eq!(events[0].target.id, rt.players[&a].state.id);

    let tower_id = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Green)
        .unwrap()
        .state
        .id;
    rt.structures.retain(|id, _| *id == tower_id);
    let tower = &rt.structures[&tower_id];
    let minion = rt.minions.get_mut(&1).unwrap();
    minion.state.x = tower.state.x + 1.0;
    minion.state.z = tower.state.z;
    let events = simulate_tower_attacks(
        &mut rt.players,
        &mut rt.minions,
        &mut rt.projectiles,
        &mut rt.structures,
        &mut rt.next_projectile_id,
        &rt.game_state,
        now,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].source,
        CombatEntity {
            kind: CombatEntityKind::Structure,
            id: tower_id
        }
    );
    assert_eq!(events[0].target.kind, CombatEntityKind::Minion);
    assert_eq!(events[0].style, ProjectileStyle::TowerBolt);
}

#[test]
fn cosmetic_history_yields_space_to_gameplay_and_keeps_newest_receipts() {
    let (rt, a, _, _) = fixture();
    let mut player = rt.players[&a].state.clone();
    player.avatar = Some(String::new());
    let mut packet = ServerPacket::Snapshot {
        geometry_id: shared::map::GEOMETRY_ID.to_owned(),
        map_profile: "verdant_default".to_owned(),
        meta: shared::protocol::SnapshotMeta::new(1, 1, 1),
        join_error: None,
        your_id: player.id,
        players: vec![player],
        projectiles: vec![],
        combat_events: vec![],
        structures: vec![],
        minions: vec![],
        neutrals: vec![],
        team_buffs: vec![],
        game_state: GameState::Running,
        rematch_in_secs: None,
    };
    let base_size = serde_json::to_vec(&packet).unwrap().len();
    let ServerPacket::Snapshot {
        players,
        combat_events,
        ..
    } = &mut packet;
    players[0].avatar = Some("x".repeat(IPV4_UDP_MAX_PAYLOAD_BYTES - base_size - 400));
    let original_player = serde_json::to_value(&players[0]).unwrap();
    *combat_events = (1..=96)
        .map(|id| CombatEvent {
            id,
            amount: 7.0,
            ..Default::default()
        })
        .collect();
    assert!(serde_json::to_vec(&packet).unwrap().len() > IPV4_UDP_MAX_PAYLOAD_BYTES);
    let payload =
        serialize_snapshot_datagram(&packet).expect("events cannot crowd out legal gameplay");
    assert!(payload.len() <= IPV4_UDP_MAX_PAYLOAD_BYTES);
    let decoded: ServerPacket = serde_json::from_slice(&payload).unwrap();
    let ServerPacket::Snapshot {
        players,
        combat_events,
        ..
    } = decoded;
    assert_eq!(serde_json::to_value(&players[0]).unwrap(), original_player);
    assert!(!combat_events.is_empty() && combat_events.len() < 96);
    assert_eq!(combat_events.last().unwrap().id, 96);
    assert!(
        combat_events
            .windows(2)
            .all(|pair| pair[0].id + 1 == pair[1].id)
    );
}
