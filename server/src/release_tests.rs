//! Release regressions exercise the actual decoded-packet handler and UDP receiver.
use super::*;

fn runtime(config: MatchConfig) -> ServerRuntime {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    ServerRuntime::new(socket, config)
}

fn addr(port: u16) -> SocketAddr {
    format!("127.0.0.1:{port}").parse().unwrap()
}

fn join(session: &str, team: Team) -> ClientPacket {
    ClientPacket::Join {
        team,
        character: CharacterChoice::Ipfs,
        hero_class: HeroClass::default(),
        avatar: Some(shared::avatar_roster()[0].slug.clone()),
        sprite_character: None,
        session_id: Some(session.to_owned()),
    }
}

fn progress(player: &mut ConnectedPlayer, now: Instant, dead: bool) {
    for _ in 0..5 {
        grant_player_xp(&mut player.state, 180);
    }
    player.state.hp = if dead { 0.0 } else { 31.0 };
    player.state.mana = 17.0;
    player.state.gold = 311;
    player.state.x = 11.0;
    player.state.z = -12.0;
    player.state.yaw = 1.2;
    player.state.ranks = [3, 2, 1, 1];
    player.state.skill_points = 2;
    player.state.action_sequence = 73;
    player.state.action_kind = PlayerActionKind::Cast;
    player.state.action_slot = 1;
    player.last_cast_at = [Some(now); 4];
    player.respawn_at = dead.then_some(now + RESPAWN_DELAY);
}

fn assert_gameplay_same(
    before: &PlayerState,
    casts: [Option<Instant>; 4],
    respawn: Option<Instant>,
    after: &ConnectedPlayer,
) {
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(&after.state).unwrap()
    );
    assert_eq!(casts, after.last_cast_at);
    assert_eq!(respawn, after.respawn_at);
}

#[test]
fn handler_duplicate_and_reclaim_preserve_wounded_and_dead_full_state() {
    for dead in [false, true] {
        let mut rt = runtime(MatchConfig::release(1));
        let now = Instant::now();
        rt.handle_packet(addr(55101), join("original", Team::Blue), now);
        rt.handle_packet(addr(55102), join("opponent", Team::Blue), now);
        let player = rt.players.get_mut(&addr(55101)).unwrap();
        progress(player, now, dead);
        let before = player.state.clone();
        let casts = player.last_cast_at;
        let respawn = player.respawn_at;
        let mut duplicate = join("changed-session", Team::Blue);
        if let ClientPacket::Join {
            character,
            hero_class,
            avatar,
            ..
        } = &mut duplicate
        {
            *character = CharacterChoice::Cube;
            *hero_class = HeroClass::ALL[1];
            *avatar = None;
        }
        rt.handle_packet(addr(55101), duplicate, now + Duration::from_millis(100));
        let player = &rt.players[&addr(55101)];
        assert_gameplay_same(&before, casts, respawn, player);
        assert_eq!(player.session_id.as_deref(), Some("original"));
        // Cross-endpoint reuse while live is rejected and visible.
        rt.handle_packet(
            addr(55103),
            join("original", Team::Blue),
            now + Duration::from_secs(1),
        );
        assert_eq!(
            rt.players[&addr(55103)].join_error,
            Some(shared::protocol::JoinRejection::SessionActive)
        );
        assert!(!rt.players[&addr(55103)].joined);
        let reclaim_at = now + PLAYER_TIMEOUT + Duration::from_secs(1);
        rt.players.get_mut(&addr(55102)).unwrap().last_seen = reclaim_at;
        rt.handle_packet(
            addr(55104),
            ClientPacket::Hello {
                protocol_version: shared::protocol::PROTOCOL_VERSION,
            },
            reclaim_at,
        );
        rt.handle_packet(addr(55104), join("original", Team::Blue), reclaim_at);
        assert_gameplay_same(&before, casts, respawn, &rt.players[&addr(55104)]);
        assert!(rt.players[&addr(55104)].framed_snapshots);
        assert!(!rt.players.contains_key(&addr(55101)));
        assert_eq!(joined_count(&rt.players), 2);
    }
}

#[test]
fn handler_capacity_counts_reservations_and_expired_claim_is_fresh() {
    let mut rt = runtime(MatchConfig::release(1));
    let now = Instant::now();
    rt.handle_packet(addr(55201), join("a", Team::Blue), now);
    rt.handle_packet(addr(55202), join("b", Team::Blue), now);
    let old_id = rt.players[&addr(55201)].state.id;
    let later = now + PLAYER_TIMEOUT + Duration::from_secs(1);
    rt.players.get_mut(&addr(55202)).unwrap().last_seen = later;
    rt.handle_packet(addr(55203), join("replacement", Team::Blue), later);
    assert_eq!(rt.disconnected_sessions.len(), 1);
    assert_eq!(
        rt.players[&addr(55203)].join_error,
        Some(shared::protocol::JoinRejection::MatchFull)
    );
    rt.handle_packet(addr(55204), join("a", Team::Blue), later);
    assert_eq!(rt.players[&addr(55204)].state.id, old_id);
    assert_eq!(joined_count(&rt.players), 2);
    let expired = later + PLAYER_TIMEOUT + SESSION_RECLAIM_WINDOW + Duration::from_secs(1);
    rt.players.get_mut(&addr(55202)).unwrap().last_seen = expired;
    rt.handle_packet(addr(55205), join("replacement", Team::Blue), expired);
    assert!(rt.players[&addr(55205)].joined);
    rt.handle_packet(addr(55206), join("a", Team::Green), expired);
    assert_eq!(
        rt.players[&addr(55206)].join_error,
        Some(shared::protocol::JoinRejection::MatchFull)
    );
    assert!(!rt.players.values().any(|player| player.state.id == old_id));
}

#[test]
fn handler_release_rejects_debug_and_bad_protocol_while_explicit_dev_accepts_debug() {
    for config in [MatchConfig::release(1), MatchConfig::dev()] {
        let mut rt = runtime(config);
        let now = Instant::now();
        rt.handle_packet(addr(55301), join("a", Team::Green), now);
        progress(rt.players.get_mut(&addr(55301)).unwrap(), now, true);
        rt.handle_packet(addr(55301), ClientPacket::SetGodMode { enabled: true }, now);
        rt.handle_packet(
            addr(55301),
            ClientPacket::SetSpeedBoost { enabled: true },
            now,
        );
        let player = &rt.players[&addr(55301)];
        assert_eq!(player.god_mode, config.mode == MatchMode::Dev);
        if config.mode == MatchMode::Release {
            assert_eq!(player.state.hp, 0.0);
            assert!(player.respawn_at.is_some());
            assert_eq!(player.speed_mult, 1.0);
        } else {
            assert_eq!(player.state.hp, player.state.max_hp);
            assert!(player.speed_mult > 1.0);
        }
        rt.handle_packet(
            addr(55302),
            ClientPacket::Hello {
                protocol_version: u16::MAX,
            },
            now,
        );
        rt.handle_packet(addr(55302), join("bad", Team::Blue), now);
        assert!(!rt.players[&addr(55302)].joined);
        assert_eq!(
            rt.players[&addr(55302)].join_error,
            Some(shared::protocol::JoinRejection::ProtocolMismatch)
        );
        rt.handle_packet(
            addr(55302),
            ClientPacket::Hello {
                protocol_version: shared::protocol::PROTOCOL_VERSION,
            },
            now,
        );
        rt.handle_packet(addr(55302), join("bad", Team::Blue), now);
        assert!(rt.players[&addr(55302)].joined);
        assert_eq!(rt.players[&addr(55302)].join_error, None);
    }
}

fn contaminate_round(rt: &mut ServerRuntime, now: Instant) {
    for player in rt.players.values_mut() {
        progress(player, now, true);
    }
    for structure in rt.structures.values_mut() {
        structure.state.hp = 0.0;
        structure.last_attack_at = Some(now);
    }
    for neutral in rt.neutrals.values_mut() {
        neutral.state.hp = 0.0;
        neutral.state.x += 20.0;
        neutral.target_player_id = Some(1);
        neutral.last_attack_at = Some(now);
        neutral.dead_until = Some(now + Duration::from_secs(900));
    }
    rt.team_buffs
        .grant(Team::Green, TeamBuffKind::MutatioMight, now);
    spawn_minion_wave_for_team_lane(
        &rt.map_layout,
        &mut rt.minions,
        &mut rt.next_minion_id,
        Team::Green,
        Lane::Mid,
    );
    rt.projectiles.insert(
        99,
        Projectile {
            state: ProjectileState {
                id: 99,
                owner_id: 1,
                owner_team: Team::Green,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            target: TargetId {
                kind: TargetKind::Player,
                id: 1,
            },
            velocity: Vec3f::new(1.0, 0.0, 0.0),
            homing: true,
            guaranteed_hit: true,
            damage: 3.0,
            radius: 1.0,
            expires_at: now + Duration::from_secs(100),
        },
    );
    rt.game_state = GameState::Victory {
        winner: Team::Green,
    };
    rt.victory_at = Some(now);
}

fn assert_clean_round(rt: &ServerRuntime) {
    for player in rt.players.values().filter(|player| player.joined) {
        assert_eq!((player.state.hp, player.state.max_hp), (MAX_HP, MAX_HP));
        assert_eq!(
            (player.state.mana, player.state.max_mana),
            (MAX_MANA, MAX_MANA)
        );
        assert_eq!(
            (
                player.state.xp,
                player.state.level,
                player.state.next_level_xp
            ),
            (0, STARTING_LEVEL, xp_threshold_for_level(STARTING_LEVEL))
        );
        assert_eq!((player.state.gold, player.state.skill_points), (0, 0));
        assert_eq!(player.state.ranks, [1; 4]);
        assert_eq!(player.state.action_sequence, 0);
        assert_eq!(player.state.action_kind, PlayerActionKind::None);
        assert_eq!(player.state.action_slot, 0);
        assert_eq!(player.last_cast_at, [None; 4]);
        assert_eq!(player.respawn_at, None);
        assert_eq!(player.speed_mult, 1.0);
        assert!(!player.god_mode);
        let spawn = spawn_position_for_team(&rt.map_layout, player.state.team);
        assert_eq!(
            (player.state.x, player.state.z, player.state.yaw),
            (spawn.x, spawn.z, 0.0)
        );
    }
    assert!(rt.projectiles.is_empty() && rt.minions.is_empty());
    assert!(rt.disconnected_sessions.is_empty());
    assert!(rt.team_buffs.snapshot(Instant::now()).is_empty());
    assert!(
        rt.structures
            .values()
            .all(|s| s.state.hp == s.state.max_hp && s.last_attack_at.is_none())
    );
    for neutral in rt.neutrals.values() {
        assert_eq!(neutral.state.x, neutral.anchor.x);
        assert_eq!(neutral.last_attack_at, None);
        assert_eq!(neutral.target_player_id, None);
        if neutral.state.camp_type.is_boss() {
            assert_eq!(neutral.state.hp, 0.0);
        } else {
            assert_eq!(neutral.state.hp, neutral.state.max_hp);
            assert_eq!(neutral.dead_until, None);
        }
    }
}

#[test]
fn canonical_rematch_resets_every_system_and_reforms_underfilled_release_roster() {
    let mut rt = runtime(MatchConfig::release(1));
    let now = Instant::now();
    rt.handle_packet(addr(55401), join("a", Team::Green), now);
    rt.handle_packet(addr(55402), join("b", Team::Blue), now);
    contaminate_round(&mut rt, now);
    let old_match = rt.match_id;
    let player = rt.players.remove(&addr(55402)).unwrap();
    rt.disconnected_sessions.insert(
        "b".into(),
        DisconnectedSession {
            player,
            disconnected_at: now,
        },
    );
    rt.handle_packet(addr(55401), ClientPacket::RequestRematch, now);
    assert_eq!(rt.match_id, old_match + 1);
    assert_clean_round(&rt);
    assert_eq!(
        rt.game_state,
        GameState::Forming {
            ready: 1,
            needed: 2
        }
    );
    assert_eq!(rt.match_started_at, None);
    rt.handle_packet(addr(55403), join("b", Team::Blue), now);
    assert_eq!(rt.players[&addr(55403)].state.level, STARTING_LEVEL);
    rt.simulate_after_mana(now, 0.01);
    assert!(matches!(rt.game_state, GameState::Starting { .. }));
    rt.simulate_after_mana(now + Duration::from_secs(3), 3.0);
    assert_eq!(rt.game_state, GameState::Running);
    assert!(rt.minions.is_empty());
    let started = rt.match_started_at.unwrap();
    for player in rt.players.values_mut() {
        player.last_seen = started + FIRST_MINION_WAVE_DELAY;
    }
    rt.simulate_after_mana(
        started + FIRST_MINION_WAVE_DELAY - Duration::from_millis(1),
        0.0,
    );
    assert!(rt.minions.is_empty());
    rt.simulate_after_mana(started + FIRST_MINION_WAVE_DELAY, 0.0);
    assert_eq!(rt.minions.len(), MINIONS_PER_WAVE * 6);
    for neutral in rt.neutrals.values().filter(|n| n.state.camp_type.is_boss()) {
        assert_eq!(
            neutral.dead_until,
            Some(started + boss_spawn_delay(neutral.state.camp_type).unwrap())
        );
    }
}

#[test]
fn empty_roster_grace_clears_reservations_and_next_group_gets_clean_match() {
    let mut rt = runtime(MatchConfig::dev());
    let now = Instant::now();
    rt.handle_packet(addr(55501), join("old", Team::Green), now);
    let old_id = rt.players[&addr(55501)].state.id;
    contaminate_round(&mut rt, now);
    rt.game_state = GameState::Running;
    rt.victory_at = None;
    let empty_at = now + PLAYER_TIMEOUT + Duration::from_millis(1);
    rt.maintain_roster(empty_at);
    assert_eq!(rt.disconnected_sessions.len(), 1);
    rt.maintain_roster(empty_at + EMPTY_ROSTER_GRACE - Duration::from_millis(1));
    assert_eq!(rt.match_id, 1);
    // Pings cannot keep an empty match alive.
    rt.handle_packet(
        addr(55502),
        ClientPacket::Ping,
        empty_at + EMPTY_ROSTER_GRACE,
    );
    assert_eq!(rt.match_id, 2);
    assert_eq!(rt.game_state, GameState::Lobby);
    assert_clean_round(&rt);
    rt.handle_packet(
        addr(55502),
        join("old", Team::Blue),
        empty_at + EMPTY_ROSTER_GRACE,
    );
    assert_ne!(rt.players[&addr(55502)].state.id, old_id);
    assert_eq!(rt.players[&addr(55502)].state.level, 1);
    assert_eq!(rt.game_state, GameState::Running);
    assert_clean_round(&rt);
}

#[test]
fn siege_blocks_cast_and_damage_until_own_lane_falls_and_resets() {
    let mut rt = runtime(MatchConfig::dev());
    let now = Instant::now();
    rt.handle_packet(addr(55601), join("siege", Team::Green), now);
    let base = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::BaseTower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .clone();
    let own_lane = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .id;
    let other_lane = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Green)
        .unwrap()
        .state
        .id;
    let player = rt.players.get_mut(&addr(55601)).unwrap();
    player.state.x = base.x - 2.0;
    player.state.z = base.z;
    rt.handle_packet(
        addr(55601),
        ClientPacket::Cast {
            target: TargetId {
                kind: TargetKind::Structure,
                id: base.id,
            },
            slot: 0,
        },
        now,
    );
    assert!(rt.projectiles.is_empty());
    assert_eq!(rt.players[&addr(55601)].state.mana, MAX_MANA);
    apply_structure_damage(
        &mut rt.structures,
        base.id,
        50.0,
        Team::Green,
        &mut rt.game_state,
    );
    assert_eq!(rt.structures[&base.id].state.hp, base.hp);
    rt.structures.get_mut(&other_lane).unwrap().state.hp = 0.0;
    assert!(structure_is_protected(&rt.structures, base.id));
    apply_structure_damage(
        &mut rt.structures,
        own_lane,
        999.0,
        Team::Green,
        &mut rt.game_state,
    );
    assert!(!structure_is_protected(&rt.structures, base.id));
    rt.handle_packet(
        addr(55601),
        ClientPacket::Cast {
            target: TargetId {
                kind: TargetKind::Structure,
                id: base.id,
            },
            slot: 0,
        },
        now,
    );
    assert_eq!(rt.projectiles.len(), 1);
    apply_structure_damage(
        &mut rt.structures,
        base.id,
        50.0,
        Team::Green,
        &mut rt.game_state,
    );
    assert_eq!(rt.structures[&base.id].state.hp, base.hp - 50.0);
    rt.restart_round(now);
    assert!(structure_is_protected(&rt.structures, base.id));
}

#[test]
fn full_roster_progression_baseline_is_reproducible_and_conserves_rewards() {
    for team_size in [1, 2, 5] {
        let mut rt = runtime(MatchConfig::dev());
        let now = Instant::now();
        for i in 0..team_size {
            rt.handle_packet(addr(55700 + i), join(&format!("p{i}"), Team::Green), now);
        }
        let mut milestones = HashMap::new();
        for wave in 1..=20 {
            for _ in 0..MINIONS_PER_WAVE * 3 {
                award_minion_kill_rewards(&mut rt.players, Team::Green);
            }
            for level in [2, 4, 6] {
                let count = rt
                    .players
                    .values()
                    .filter(|p| p.state.level >= level)
                    .count();
                if count > 0 {
                    milestones.entry((level, "first")).or_insert(wave);
                }
                if count == team_size as usize {
                    milestones.entry((level, "all")).or_insert(wave);
                }
            }
            let expected_gold = wave as u32 * MINIONS_PER_WAVE as u32 * 3 * MINION_KILL_GOLD;
            assert_eq!(
                rt.players.values().map(|p| p.state.gold).sum::<u32>(),
                expected_gold
            );
            for p in rt.players.values() {
                assert_eq!(p.state.skill_points, p.state.level - 1);
                assert_eq!(p.state.ranks, [1; 4]);
            }
        }
        if team_size == 5 {
            assert_eq!((milestones[&(2, "first")], milestones[&(2, "all")]), (2, 2));
            assert_eq!((milestones[&(4, "first")], milestones[&(4, "all")]), (7, 8));
            assert_eq!(
                (milestones[&(6, "first")], milestones[&(6, "all")]),
                (15, 17)
            );
            for level in [2, 4, 6] {
                let seconds = |key| {
                    FIRST_MINION_WAVE_DELAY.as_secs()
                        + (milestones[&(level, key)] as u64 - 1) * MINION_WAVE_INTERVAL.as_secs()
                };
                println!(
                    "PROGRESSION_ESTIMATE roster=5v5 level={level} first_secs={} all_secs={} combat_travel_excluded=true",
                    seconds("first"),
                    seconds("all")
                );
            }
        }
    }
}

fn send_udp(socket: &UdpSocket, rt: &mut ServerRuntime, packet: ClientPacket) {
    let addr = socket.local_addr().unwrap();
    let before = rt.players.get(&addr).map(|p| p.last_seen);
    socket
        .send_to(
            &serde_json::to_vec(&packet).unwrap(),
            rt.socket.local_addr().unwrap(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        rt.receive_packets();
        if rt.players.get(&addr).map(|p| p.last_seen) != before {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "UDP handler did not receive the request"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn udp_snapshot(
    rt: &mut ServerRuntime,
    socket: &UdpSocket,
    assembler: &mut shared::transport::SnapshotAssembler,
) -> ServerPacket {
    rt.last_snapshot_at = Instant::now() - SNAPSHOT_INTERVAL;
    rt.simulate_after_mana(Instant::now(), 0.0);
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut buf = vec![0; 65_536];
    loop {
        let (len, _) = socket.recv_from(&mut buf).unwrap();
        assert!(len <= shared::transport::MAX_DATAGRAM_BYTES);
        if let Some(payload) = assembler.push(&buf[..len], Instant::now()).unwrap() {
            return serde_json::from_slice(&payload).unwrap();
        }
    }
}

#[test]
fn live_udp_victory_rematch_uses_real_cast_receiver_and_framed_snapshots() {
    // Test-only accelerated fixture: normal production handlers, transport and
    // damage rules, with a nearly completed siege. No production cheat flags.
    let mut rt = runtime(MatchConfig::release(1));
    let first = UdpSocket::bind("127.0.0.1:0").unwrap();
    let second = UdpSocket::bind("127.0.0.1:0").unwrap();
    let first_addr = first.local_addr().unwrap();
    let mut assembler = shared::transport::SnapshotAssembler::default();
    send_udp(
        &first,
        &mut rt,
        ClientPacket::Hello {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
        },
    );
    send_udp(&first, &mut rt, join("udp-a", Team::Green));
    send_udp(&second, &mut rt, join("udp-b", Team::Blue));
    let now = Instant::now();
    rt.simulate_after_mana(now, 0.0);
    rt.simulate_after_mana(now, 3.0);
    assert_eq!(rt.game_state, GameState::Running);
    let base = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::BaseTower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .clone();
    let lane = rt
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .id;
    for s in rt.structures.values_mut() {
        s.attack_range = 0.0;
    }
    rt.structures.get_mut(&lane).unwrap().state.hp = 0.0;
    rt.structures.get_mut(&base.id).unwrap().state.hp = 1.0;
    let player = rt.players.get_mut(&first_addr).unwrap();
    progress(player, now, false);
    player.state.x = base.x - 2.0;
    player.state.z = base.z;
    player.state.mana = player.state.max_mana;
    player.last_cast_at = [None; 4];
    let camp = rt
        .neutrals
        .values_mut()
        .find(|n| !n.state.camp_type.is_boss())
        .unwrap();
    camp.state.hp = 0.0;
    camp.dead_until = Some(now + Duration::from_secs(40));
    rt.team_buffs
        .grant(Team::Green, TeamBuffKind::WendigoFavor, now);
    let before = rt.players[&first_addr].state.clone();
    send_udp(&first, &mut rt, join("udp-changed", Team::Blue));
    assert_gameplay_same(&before, [None; 4], None, &rt.players[&first_addr]);
    send_udp(
        &first,
        &mut rt,
        ClientPacket::Cast {
            target: TargetId {
                kind: TargetKind::Structure,
                id: base.id,
            },
            slot: 0,
        },
    );
    assert_eq!(rt.projectiles.len(), 1);
    rt.simulate_after_mana(Instant::now(), 1.0);
    assert_eq!(
        rt.game_state,
        GameState::Victory {
            winner: Team::Green
        }
    );
    let ServerPacket::Snapshot {
        meta: first_meta,
        game_state,
        ..
    } = udp_snapshot(&mut rt, &first, &mut assembler);
    assert_eq!(
        game_state,
        GameState::Victory {
            winner: Team::Green
        }
    );
    assert!(first_meta.server_epoch > 0 && first_meta.snapshot_tick > 0);
    send_udp(&first, &mut rt, ClientPacket::RequestRematch);
    assert_clean_round(&rt);
    let ServerPacket::Snapshot {
        meta: second_meta,
        game_state,
        ..
    } = udp_snapshot(&mut rt, &first, &mut assembler);
    assert_eq!(second_meta.server_epoch, first_meta.server_epoch);
    assert_eq!(second_meta.match_id, first_meta.match_id + 1);
    assert!(second_meta.snapshot_tick > first_meta.snapshot_tick);
    assert!(matches!(game_state, GameState::Starting { .. }));
    rt.simulate_after_mana(Instant::now(), 3.0);
    assert_eq!(rt.game_state, GameState::Running);
    assert_clean_round(&rt);
    assert!(structure_is_protected(&rt.structures, base.id));
    println!(
        "LIVE_UDP_LIFECYCLE victory=green match_before={} match_after={} framed_max_bytes={} clean_second_running=true",
        first_meta.match_id,
        second_meta.match_id,
        shared::transport::MAX_DATAGRAM_BYTES
    );
}
