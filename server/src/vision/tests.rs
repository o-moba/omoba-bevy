use super::*;

fn fixture() -> (ServerRuntime, SocketAddr, SocketAddr, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    rt.targeting_qa = false;
    let now = Instant::now();
    let a = "127.0.0.1:58901".parse().unwrap();
    let b = "127.0.0.1:58902".parse().unwrap();
    for (addr, team, x) in [(a, Team::Green, -18.0), (b, Team::Blue, -22.0)] {
        rt.handle_packet(
            addr,
            ClientPacket::Join {
                prematch: false,
                team,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Mage,
                avatar: None,
                sprite_character: None,
                session_id: None,
                passport_ticket: None,
            },
            now,
        );
        let p = rt.world.players.get_mut(&addr).unwrap();
        p.hero.x = x;
        p.hero.z = -8.0;
    }
    rt.world.structures.clear();
    rt.world.minions.clear();
    rt.world.neutrals.clear();
    (rt, a, b, now)
}
fn target(rt: &ServerRuntime, addr: SocketAddr) -> TargetId {
    TargetId {
        kind: TargetKind::Player,
        id: rt.world.players[&addr].hero.identity.id,
    }
}
fn snapshot(rt: &mut ServerRuntime, addr: SocketAddr, now: Instant) -> ServerPacket {
    let mut packet = ServerPacket::Snapshot {
        vision: None,
        sandbox: None,
        match_mode: "dev".into(),
        geometry_id: shared::map::GEOMETRY_ID.into(),
        map_profile: "verdant_default".into(),
        meta: Default::default(),
        join_error: None,
        your_id: rt.world.players[&addr].hero.identity.id,
        players: build_players_snapshot(&rt.world, now),
        scoreboard: rt.combat_log.ledger.live_scoreboard(),
        prematch: None,
        projectiles: rt
            .world
            .projectiles
            .values()
            .map(|p| p.state.clone())
            .collect(),
        combat_events: rt.combat_log.snapshot(now),
        structures: rt
            .world
            .structures
            .values()
            .filter(|p| p.state.hp > 0.0)
            .map(|p| p.state.clone())
            .collect(),
        minions: rt
            .world
            .minions
            .values()
            .filter(|p| p.state.hp > 0.0)
            .map(|p| p.state.clone())
            .collect(),
        neutrals: rt
            .world
            .neutrals
            .values()
            .filter(|p| p.state.hp > 0.0 && p.dead_until.is_none())
            .map(|p| p.state.clone())
            .collect(),
        team_buffs: vec![],
        forest_pickups: vec![],
        game_state: GameState::Running,
        rematch_in_secs: None,
    };
    filter_snapshot(&mut packet, &rt.world.players[&addr], &rt.world, now);
    packet
}
fn strike(rt: &mut ServerRuntime, a: SocketAddr, t: TargetId, now: Instant) {
    handle_basic_attack_request(&mut rt.world, a, t, 1, now);
}
fn cast(rt: &mut ServerRuntime, a: SocketAddr, t: TargetId, now: Instant) {
    handle_cast_request(&mut rt.world, a, t, 0, now);
}
fn minion(id: u64, team: Team, pos: [f32; 2]) -> Minion {
    Minion {
        state: MinionState {
            kind: MinionKind::Melee,
            attack_sequence: 0,
            id,
            team,
            lane: Lane::Mid,
            x: pos[0],
            y: MINION_SPAWN_HEIGHT,
            z: pos[1],
            yaw: 0.0,
            hp: 100.0,
            max_hp: 100.0,
            state: MinionBrainState::Marching,
            target_kind: None,
            target_id: None,
        },
        path: vec![Vec3f::new(pos[0], MINION_SPAWN_HEIGHT, pos[1])],
        next_waypoint: 0,
        last_attack_at: None,
        aggro_target: None,
    }
}

#[test]
fn living_joined_sources_share_move_die_disconnect_and_exclude_opponents() {
    let (mut rt, a, b, now) = fixture();
    assert_eq!(sources(Team::Green, &rt.world).len(), 1);
    let mut allies = vec![];
    for (i, x) in [(3, 48.0), (4, 88.0)] {
        let addr = SocketAddr::from(([127, 0, 0, 1], 58900 + i));
        rt.world.ensure_connected(addr, now);
        let p = rt.world.players.get_mut(&addr).unwrap();
        p.joined = true;
        p.hero.identity.team = Team::Green;
        p.hero.x = x;
        p.hero.z = 0.0;
        allies.push(addr);
    }
    assert!(point_visible(
        &sources(Team::Green, &rt.world),
        [100.0, 0.0],
        false
    ));
    rt.world.players.get_mut(&allies[1]).unwrap().hero.hp = 0.0;
    assert!(!point_visible(
        &sources(Team::Green, &rt.world),
        [100.0, 0.0],
        false
    ));
    rt.world.players.get_mut(&allies[0]).unwrap().joined = false;
    rt.world.players.get_mut(&a).unwrap().hero.hp = 0.0;
    assert!(sources(Team::Green, &rt.world).is_empty());
    rt.world.players.remove(&allies[0]);
    rt.world
        .minions
        .insert(501, minion(501, Team::Green, [0.0, 0.0]));
    assert_eq!(
        sources(Team::Green, &rt.world)[0].radius,
        MINION_SIGHT_RADIUS
    );
    rt.world.minions.get_mut(&501).unwrap().state.x = 60.0;
    assert!(point_visible(
        &sources(Team::Green, &rt.world),
        [80.0, 0.0],
        false
    ));
    rt.world.minions.get_mut(&501).unwrap().state.hp = 0.0;
    assert!(sources(Team::Green, &rt.world).is_empty());
    assert_eq!(sources(Team::Blue, &rt.world).len(), 1);
    rt.world.players.remove(&b);
    assert!(sources(Team::Blue, &rt.world).is_empty());
    rt.world.structures = build_structures(&rt.world.map_layout);
    let sight = sources(Team::Green, &rt.world);
    assert!(sight.iter().any(|s| s.radius == TOWER_SIGHT_RADIUS));
    assert!(sight.iter().any(|s| s.radius == BASE_SIGHT_RADIUS));
    for s in rt.world.structures.values_mut() {
        s.state.hp = 0.0;
    }
    assert!(sources(Team::Green, &rt.world).is_empty());
}
#[test]
fn unseen_basic_and_cast_reject_without_resources_or_reveal_but_same_brush_accepts() {
    let (mut rt, a, b, now) = fixture();
    let t = target(&rt, b);
    let mana = rt.world.players[&a].hero.mana;
    strike(&mut rt, a, t, now);
    cast(&mut rt, a, t, now);
    assert!(rt.world.projectiles.is_empty());
    assert_eq!(rt.world.players[&a].hero.mana, mana);
    assert_eq!(rt.world.players[&a].timers.last_basic_attack_at, None);
    assert_eq!(rt.world.players[&a].timers.last_cast_at, [None; 4]);
    assert!(!revealed(&rt.world.players[&a], now));
    rt.world.players.get_mut(&a).unwrap().hero.x = -20.0;
    cast(&mut rt, a, t, now);
    assert_eq!(rt.world.projectiles.len(), 1);
    assert!(revealed(&rt.world.players[&a], now));
}
#[test]
fn hostile_action_reveal_expires_and_self_cast_does_not_reveal() {
    let (mut rt, a, b, now) = fixture();
    let t = target(&rt, a);
    assert!(!target_visible(Team::Green, target(&rt, b), &rt.world, now));
    cast(&mut rt, b, t, now);
    assert!(revealed(&rt.world.players[&b], now));
    assert!(target_visible(
        Team::Green,
        target(&rt, b),
        &rt.world,
        now + Duration::from_millis(1999)
    ));
    assert!(!target_visible(
        Team::Green,
        target(&rt, b),
        &rt.world,
        now + Duration::from_secs(2)
    ));
    let p = rt.world.players.get_mut(&b).unwrap();
    p.timers.last_cast_at = [None; 4];
    p.timers.last_cast_at[1] = Some(now);
    assert!(!revealed(p, now));
}
#[test]
fn recipient_payloads_hide_actors_projectiles_events_and_pickup_receipts() {
    let (mut rt, a, b, now) = fixture();
    let ta = target(&rt, a);
    let tb = target(&rt, b);
    // Launch while visible, then hide target before replication.
    rt.world.players.get_mut(&b).unwrap().hero.x = -18.5;
    cast(&mut rt, a, tb, now);
    rt.world.players.get_mut(&b).unwrap().hero.x = -22.0;
    rt.combat_log.extend(
        now,
        [CombatEvent {
            source: CombatEntity {
                kind: CombatEntityKind::Player,
                id: ta.id,
            },
            target: CombatEntity {
                kind: CombatEntityKind::Player,
                id: tb.id,
            },
            x: -22.0,
            z: -8.0,
            ..Default::default()
        }],
    );
    rt.world
        .minions
        .insert(501, minion(501, Team::Blue, [101.125, 99.375]));
    let mut packet = snapshot(&mut rt, a, now);
    let ServerPacket::Snapshot {
        players,
        projectiles,
        combat_events,
        minions,
        ..
    } = &packet
    else {
        panic!()
    };
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].id, ta.id);
    assert!(projectiles.is_empty());
    assert!(combat_events.is_empty());
    assert!(minions.is_empty());
    let encoded = serde_json::to_string(&packet).unwrap();
    assert!(!encoded.contains("101.125"));
    assert!(!encoded.contains("99.375"));
    assert!(!encoded.contains("\"x\":-22.0"));
    if let ServerPacket::Snapshot { forest_pickups, .. } = &mut packet {
        forest_pickups.push(shared::forest_pickups::ForestPickupState {
            id: 99,
            position: [-18.0, -8.0],
            available: false,
            collection_sequence: 9,
            last_collector_id: Some(tb.id),
            healed_amount: 5.0,
        });
    }
    filter_snapshot(&mut packet, &rt.world.players[&a], &rt.world, now);
    let ServerPacket::Snapshot { forest_pickups, .. } = packet else {
        panic!()
    };
    assert_eq!(forest_pickups[0].last_collector_id, None);
    assert_eq!(forest_pickups[0].collection_sequence, 0);
    let ServerPacket::Snapshot { players, .. } = snapshot(&mut rt, b, now) else {
        panic!()
    };
    assert_eq!(players.len(), 2);
    rt.world.players.get_mut(&a).unwrap().joined = false;
    let ServerPacket::Snapshot {
        players,
        vision,
        projectiles,
        ..
    } = snapshot(&mut rt, a, now)
    else {
        panic!()
    };
    assert!(players.is_empty());
    assert!(vision.unwrap().sources.is_empty());
    assert!(projectiles.is_empty());
}
#[test]
fn launched_homing_hits_after_concealment_without_replication_leak() {
    let (mut rt, a, b, now) = fixture();
    let t = target(&rt, b);
    rt.world.players.get_mut(&b).unwrap().hero.x = -18.5;
    cast(&mut rt, a, t, now);
    assert_eq!(rt.world.projectiles.len(), 1);
    rt.world.players.get_mut(&b).unwrap().hero.x = -22.0;
    let ServerPacket::Snapshot { projectiles, .. } = snapshot(&mut rt, a, now) else {
        panic!()
    };
    assert!(projectiles.is_empty());
    let before = rt.world.players[&b].hero.hp;
    let events = simulate_projectiles(
        &mut rt.world,
        TickCtx {
            now: now + Duration::from_millis(500),
            dt: 0.5,
        },
    );
    assert!(rt.world.players[&b].hero.hp < before);
    assert_eq!(events.len(), 1);
    rt.combat_log.extend(now, events);
    let ServerPacket::Snapshot { combat_events, .. } = snapshot(&mut rt, a, now) else {
        panic!()
    };
    assert!(combat_events.is_empty());
}
#[test]
fn bots_minions_and_towers_do_not_acquire_or_keep_concealed_heroes() {
    let (mut rt, a, b, now) = fixture();
    assert!(
        rt.bot_target(Team::Green, [-18.0, -8.0], Lane::Mid, now)
            .is_none()
    );
    rt.world
        .minions
        .insert(501, minion(501, Team::Green, [-18.0, -8.0]));
    rt.world.minions.get_mut(&501).unwrap().aggro_target = Some(MinionAggroTarget::Player(
        rt.world.players[&b].hero.identity.id,
    ));
    simulate_minions(&mut rt.world, TickCtx { now, dt: 0.0 });
    assert_eq!(rt.world.minions[&501].state.target_id, None);
    assert_eq!(rt.world.minions[&501].last_attack_at, None);
    rt.world.structures = build_structures(&rt.world.map_layout);
    rt.world
        .structures
        .retain(|_, s| s.state.team == Team::Green);
    for s in rt.world.structures.values_mut() {
        s.state.x = -18.0;
        s.state.z = -8.0;
    }
    simulate_tower_attacks(&mut rt.world, now);
    assert!(rt.world.projectiles.is_empty());
    assert!(
        rt.world
            .structures
            .values()
            .all(|s| s.last_attack_at.is_none())
    );
    rt.world.players.get_mut(&a).unwrap().hero.x = -20.0;
    assert!(
        rt.bot_target(Team::Green, [-20.0, -8.0], Lane::Mid, now)
            .is_some()
    );
    simulate_tower_attacks(&mut rt.world, now);
    assert!(!rt.world.projectiles.is_empty());
}

#[test]
fn native_qa_route_is_walkable_and_final_destination_is_outside_green_sight() {
    let (mut rt, a, _, _) = fixture();
    rt.world.players.get_mut(&a).unwrap().hero.x = -14.0;
    rt.world.structures = build_structures(&rt.world.map_layout);
    assert!(!point_visible(
        &sources(Team::Green, &rt.world),
        [22.0, -8.0],
        false
    ));
    assert!(shared::navigation::world_navigation().segment_clear([-18.0, -8.0], [22.0, -8.0]));
}

#[test]
fn both_teams_hide_other_brush_and_all_hidden_dynamic_channels() {
    let (mut rt, a, b, now) = fixture();
    let pa = rt.world.players.get_mut(&a).unwrap();
    pa.hero.x = 22.0;
    pa.hero.z = 8.0;
    rt.world.structures = build_structures(&rt.world.map_layout);
    rt.world.neutrals = build_neutral_camps(&mut 700);
    for n in rt.world.neutrals.values_mut() {
        n.state.x = 100.125;
        n.state.z = 99.875;
    }
    for s in rt.world.structures.values_mut() {
        s.state.x = 100.375;
        s.state.z = 99.625;
    }
    // Keep structures alive but outside the tested actors' source radius.
    for (viewer, hidden) in [(a, b), (b, a)] {
        let hidden_id = rt.world.players[&hidden].hero.identity.id;
        rt.world.minions.clear();
        let mut m = minion(
            601,
            rt.world.players[&viewer].hero.identity.team,
            [
                rt.world.players[&viewer].hero.x,
                rt.world.players[&viewer].hero.z,
            ],
        );
        m.state.target_id = Some(hidden_id);
        m.state.target_kind = Some(MinionTargetKind::Player);
        rt.world.minions.insert(601, m);
        let packet = snapshot(&mut rt, viewer, now);
        let ServerPacket::Snapshot {
            players,
            minions,
            neutrals,
            structures,
            ..
        } = &packet
        else {
            panic!()
        };
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].id, rt.world.players[&viewer].hero.identity.id);
        assert_eq!(minions[0].target_id, None);
        assert_eq!(minions[0].target_kind, None);
        // Nearby allied structures reveal this test's far location; mark all structures
        // dead below to test absence without contributing any sight.
        assert!(!structures.is_empty());
        assert!(!neutrals.is_empty());
    }
    for s in rt.world.structures.values_mut() {
        s.state.hp = 0.0;
    }
    // Retain only remote hostile records; their destroyed state cannot contribute sight.
    rt.world
        .structures
        .retain(|_, s| s.state.team == Team::Blue);
    rt.world.minions.clear();
    let packet = snapshot(&mut rt, a, now);
    let ServerPacket::Snapshot {
        structures,
        neutrals,
        ..
    } = &packet
    else {
        panic!()
    };
    assert!(structures.is_empty());
    assert!(neutrals.is_empty());
    let encoded = serde_json::to_string(&packet).unwrap();
    for hidden in ["100.125", "99.875", "100.375", "99.625"] {
        assert!(!encoded.contains(hidden), "{hidden}");
    }
}

#[test]
fn allied_dead_viewer_keeps_team_sight_and_round_reset_clears_reveal() {
    let (mut rt, a, b, now) = fixture();
    rt.world.players.get_mut(&b).unwrap().hero.identity.team = Team::Green;
    let p = rt.world.players.get_mut(&a).unwrap();
    p.hero.hp = 0.0;
    p.hero.x = shared::vision::brush_layout()[0].center[0];
    p.hero.z = shared::vision::brush_layout()[0].center[1];
    p.timers.last_basic_attack_at = Some(now);
    let ServerPacket::Snapshot {
        players, vision, ..
    } = snapshot(&mut rt, a, now)
    else {
        panic!()
    };
    assert_eq!(players.len(), 2);
    let vision = vision.unwrap();
    assert_eq!(vision.sources.len(), 1);
    assert_eq!(vision.local_brush, None);
    assert!(!vision.local_hidden);
    rt.restart_round(now + Duration::from_secs(1));
    assert!(!revealed(
        &rt.world.players[&a],
        now + Duration::from_secs(1)
    ));
    assert_eq!(
        sources(Team::Green, &rt.world)
            .iter()
            .filter(|s| s.radius == HERO_SIGHT_RADIUS)
            .count(),
        2
    );
}

#[test]
fn lethal_nonhero_receipts_survive_removal_only_for_visible_impacts() {
    let (mut rt, a, b, now) = fixture();
    let mut dead_minion = minion(501, Team::Blue, [-18.0, -8.0]);
    dead_minion.state.hp = 0.0;
    rt.world.minions.insert(501, dead_minion);
    let mut camps = build_neutral_camps(&mut 502);
    let mut dead_neutral = camps.remove(&502).unwrap();
    dead_neutral.state.hp = 0.0;
    dead_neutral.dead_until = Some(now + Duration::from_secs(30));
    dead_neutral.state.x = -18.0;
    dead_neutral.state.z = -8.0;
    rt.world.neutrals.insert(502, dead_neutral);
    let mut authored = build_structures(&rt.world.map_layout);
    let mut dead_tower = authored.remove(&1).unwrap();
    dead_tower.state.id = 503;
    dead_tower.state.hp = 0.0;
    dead_tower.state.x = -18.0;
    dead_tower.state.z = -8.0;
    rt.world.structures.insert(503, dead_tower);
    let source = CombatEntity {
        kind: CombatEntityKind::Player,
        id: rt.world.players[&a].hero.identity.id,
    };
    for (kind, id) in [
        (CombatEntityKind::Minion, 501),
        (CombatEntityKind::Neutral, 502),
        (CombatEntityKind::Structure, 503),
    ] {
        rt.combat_log.extend(
            now,
            [CombatEvent {
                source,
                target: CombatEntity { kind, id },
                x: -18.0,
                z: -8.0,
                amount: 7.0,
                killed: true,
                ..Default::default()
            }],
        );
    }
    let ServerPacket::Snapshot { combat_events, .. } = snapshot(&mut rt, a, now) else {
        panic!()
    };
    assert_eq!(
        combat_events.len(),
        3,
        "removed minion, neutral, and structure final hits remain visible"
    );
    rt.combat_log.extend(
        now,
        [CombatEvent {
            source,
            target: CombatEntity {
                kind: CombatEntityKind::Minion,
                id: 504,
            },
            x: 101.25,
            z: 98.5,
            amount: 7.0,
            killed: true,
            ..Default::default()
        }],
    );
    // An old lethal receipt cannot reveal a living, now unseen respawn.
    rt.world
        .minions
        .insert(505, minion(505, Team::Blue, [101.25, 98.5]));
    rt.combat_log.extend(
        now,
        [CombatEvent {
            source,
            target: CombatEntity {
                kind: CombatEntityKind::Minion,
                id: 505,
            },
            x: -18.0,
            z: -8.0,
            amount: 7.0,
            killed: true,
            ..Default::default()
        }],
    );
    // Heroes retain strict brush visibility even for a lethal impact.
    let hidden = rt.world.players.get_mut(&b).unwrap();
    hidden.hero.hp = 0.0;
    rt.combat_log.extend(
        now,
        [CombatEvent {
            source,
            target: CombatEntity {
                kind: CombatEntityKind::Player,
                id: hidden.hero.identity.id,
            },
            x: hidden.hero.x,
            z: hidden.hero.z,
            amount: 7.0,
            killed: true,
            ..Default::default()
        }],
    );
    let packet = snapshot(&mut rt, a, now);
    let ServerPacket::Snapshot { combat_events, .. } = &packet else {
        panic!()
    };
    assert_eq!(combat_events.len(), 3);
    assert!(!serde_json::to_string(&packet).unwrap().contains("101.25"));
}
