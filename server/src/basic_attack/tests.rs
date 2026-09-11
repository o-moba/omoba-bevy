use super::*;

fn fixture() -> (ServerRuntime, SocketAddr, SocketAddr, TargetId, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut runtime = ServerRuntime::new(socket, MatchConfig::dev());
    runtime.targeting_qa = false;
    let now = Instant::now();
    let a: SocketAddr = "127.0.0.1:58001".parse().unwrap();
    let b: SocketAddr = "127.0.0.1:58002".parse().unwrap();
    for (addr, team, x) in [(a, Team::Green, 0.0), (b, Team::Blue, 2.0)] {
        runtime.handle_packet(
            addr,
            ClientPacket::Join {
                team,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
                session_id: Some(format!("basic-{}", addr.port())),
            },
            now,
        );
        let state = &mut runtime.players.get_mut(&addr).unwrap().state;
        state.x = x;
        state.z = 0.0;
    }
    let target = TargetId {
        kind: TargetKind::Player,
        id: runtime.players[&b].state.id,
    };
    (runtime, a, b, target, now)
}

fn packet(runtime: &ServerRuntime, target: TargetId, request_id: u64) -> ClientPacket {
    ClientPacket::BasicAttack {
        target,
        server_epoch: runtime.server_epoch,
        match_id: runtime.match_id,
        request_id,
    }
}

fn strike(runtime: &mut ServerRuntime, addr: SocketAddr, target: TargetId, id: u64, now: Instant) {
    runtime.handle_packet(addr, packet(runtime, target, id), now);
}

#[test]
fn zero_mana_basic_attack_and_q_have_independent_costs_and_clocks() {
    let (mut rt, a, _, target, now) = fixture();
    let attacker = rt.players.get_mut(&a).unwrap();
    attacker.state.mana = 0.0;
    attacker.last_cast_at = [Some(now); 4];
    strike(&mut rt, a, target, 1, now);
    assert_eq!(rt.projectiles.len(), 1);
    let player = &rt.players[&a];
    assert_eq!(player.state.mana, 0.0);
    assert_eq!(player.last_cast_at, [Some(now); 4]);
    assert_eq!(player.state.action_slot, BASIC_ATTACK_ACTION_SLOT);
    assert_eq!(player.state.action_kind, PlayerActionKind::Attack);
    assert_eq!(player.state.ranks, [1; 4]);
    let player = rt.players.get_mut(&a).unwrap();
    player.state.mana = MAX_MANA;
    player.last_cast_at = [None; 4];
    rt.handle_packet(a, ClientPacket::Cast { target, slot: 0 }, now);
    assert_eq!(
        rt.projectiles.len(),
        2,
        "Q remains available while basic attack cools down"
    );
    assert_eq!(rt.players[&a].last_basic_attack_at, Some(now));
    assert_eq!(rt.players[&a].state.mana, MAX_MANA - 10.0);
    strike(&mut rt, a, target, 2, now + Duration::from_millis(899));
    assert_eq!(rt.projectiles.len(), 2);
    strike(&mut rt, a, target, 3, now + Duration::from_millis(901));
    assert_eq!(rt.projectiles.len(), 3);
}

#[test]
fn packet_replay_and_wrong_round_cannot_attack_or_poison_fresh_sequence() {
    let (mut rt, a, b, target, now) = fixture();
    for (epoch, round) in [
        (rt.server_epoch + 1, rt.match_id),
        (rt.server_epoch, rt.match_id + 1),
    ] {
        rt.handle_packet(
            a,
            ClientPacket::BasicAttack {
                target,
                server_epoch: epoch,
                match_id: round,
                request_id: 999,
            },
            now,
        );
        assert_eq!(rt.players[&a].state.basic_attack_request_id, 0);
    }
    strike(&mut rt, a, target, 1, now);
    // Advance beyond the attack cooldown while keeping both ordinary peers
    // alive; stale strike packets deliberately do not refresh their liveness.
    for addr in [a, b] {
        rt.handle_packet(addr, ClientPacket::Ping, now + Duration::from_secs(3));
    }
    strike(&mut rt, a, target, 1, now + Duration::from_secs(5));
    strike(&mut rt, a, target, 0, now + Duration::from_secs(5));
    assert_eq!(rt.projectiles.len(), 1);
    let old_packet = packet(&rt, target, 100);
    rt.restart_round(now + Duration::from_secs(6));
    rt.handle_packet(a, old_packet, now + Duration::from_secs(7));
    assert_eq!(rt.players[&a].state.basic_attack_request_id, 0);
    assert!(rt.players[&a].last_basic_attack_at.is_none());
    assert_eq!(rt.players[&a].state.basic_attack_remaining_secs, 0.0);
}

#[test]
fn rejects_unjoined_dead_friendly_missing_and_out_of_range_targets() {
    for condition in [
        "unjoined",
        "dead attacker",
        "dead target",
        "friendly",
        "missing",
        "far",
        "lobby",
    ] {
        let (mut rt, a, b, mut target, now) = fixture();
        match condition {
            "unjoined" => rt.players.get_mut(&a).unwrap().joined = false,
            "dead attacker" => rt.players.get_mut(&a).unwrap().state.hp = 0.0,
            "dead target" => rt.players.get_mut(&b).unwrap().state.hp = 0.0,
            "friendly" => rt.players.get_mut(&b).unwrap().state.team = Team::Green,
            "missing" => target.id = u64::MAX,
            "far" => rt.players.get_mut(&b).unwrap().state.x = 100.0,
            "lobby" => rt.game_state = GameState::Lobby,
            _ => unreachable!(),
        }
        strike(&mut rt, a, target, 1, now);
        assert!(rt.projectiles.is_empty(), "{condition}");
        assert!(rt.players[&a].last_basic_attack_at.is_none(), "{condition}");
        assert_eq!(rt.players[&a].state.mana, MAX_MANA, "{condition}");
        assert_eq!(rt.players[&a].state.action_sequence, 0, "{condition}");
    }
}

#[test]
fn target_surface_range_and_base_protection_are_authoritative_for_all_kinds() {
    for kind in [
        TargetKind::Player,
        TargetKind::Minion,
        TargetKind::Structure,
        TargetKind::Neutral,
    ] {
        let (mut rt, a, b, mut target, now) = fixture();
        let range = basic_attack_for_class(HeroClass::Warrior).range;
        let radius = match kind {
            TargetKind::Player => {
                rt.players.get_mut(&b).unwrap().state.x = range + PLAYER_HIT_RADIUS - 0.01;
                PLAYER_HIT_RADIUS
            }
            TargetKind::Minion => {
                spawn_minion_wave_for_team_lane(
                    &rt.map_layout,
                    &mut rt.minions,
                    &mut rt.next_minion_id,
                    Team::Blue,
                    Lane::Mid,
                );
                let minion = rt.minions.values_mut().next().unwrap();
                minion.state.x = range + MINION_RADIUS - 0.01;
                minion.state.z = 0.0;
                target = TargetId {
                    kind,
                    id: minion.state.id,
                };
                MINION_RADIUS
            }
            TargetKind::Structure => {
                let structure = rt
                    .structures
                    .values_mut()
                    .find(|s| s.state.team == Team::Blue && s.state.kind == StructureKind::Tower)
                    .unwrap();
                structure.state.x = range + shared::TOWER_TARGET_RADIUS - 0.01;
                structure.state.z = 0.0;
                target = TargetId {
                    kind,
                    id: structure.state.id,
                };
                shared::TOWER_TARGET_RADIUS
            }
            TargetKind::Neutral => {
                let neutral = rt
                    .neutrals
                    .values_mut()
                    .find(|n| n.dead_until.is_none() && n.state.hp > 0.0)
                    .unwrap();
                neutral.state.x = range + NEUTRAL_RADIUS - 0.01;
                neutral.state.z = 0.0;
                target = TargetId {
                    kind,
                    id: neutral.state.id,
                };
                NEUTRAL_RADIUS
            }
        };
        strike(&mut rt, a, target, 1, now);
        assert_eq!(
            rt.projectiles.len(),
            1,
            "surface within {range}+{radius} for {kind:?}"
        );
        rt.players.get_mut(&a).unwrap().state.x = -0.1;
        strike(&mut rt, a, target, 2, now + Duration::from_secs(2));
        assert_eq!(
            rt.projectiles.len(),
            1,
            "outside surface range for {kind:?}"
        );
    }
    let (mut rt, a, _, _, now) = fixture();
    let base = rt
        .structures
        .values_mut()
        .find(|s| s.state.team == Team::Blue && s.state.kind == StructureKind::BaseTower)
        .unwrap();
    base.state.x = 2.0;
    base.state.z = 0.0;
    let target = TargetId {
        kind: TargetKind::Structure,
        id: base.state.id,
    };
    strike(&mut rt, a, target, 1, now);
    assert!(
        rt.projectiles.is_empty(),
        "protected base cannot be attacked"
    );
}

#[test]
fn equipment_changes_basic_deadline_without_rescaling_elapsed_time() {
    let (mut rt, a, _, target, now) = fixture();
    strike(&mut rt, a, target, 1, now);
    let elapsed = Duration::from_millis(400);
    rt.players.get_mut(&a).unwrap().state.item_bonuses =
        shared::shop::item_bonuses(&[ItemId::SwiftGrip, ItemId::EmberBlade]);
    refresh_basic_attack_cooldowns(&mut rt.players, now + elapsed);
    let definition = basic_attack_for_class(HeroClass::Warrior);
    let duration = basic_attack_cooldown(definition, rt.players[&a].state.item_bonuses);
    assert!(
        (rt.players[&a].state.basic_attack_remaining_secs - (duration - elapsed).as_secs_f32())
            .abs()
            < 0.00001
    );
    strike(
        &mut rt,
        a,
        target,
        2,
        now + duration + Duration::from_millis(1),
    );
    let damage = rt
        .projectiles
        .values()
        .find(|p| p.state.id == 2)
        .unwrap()
        .damage;
    assert!((damage - 12.0 * 1.12).abs() < 0.0001);
}

#[test]
fn accepted_strike_deals_real_projectile_damage_and_death_retains_replay_guard() {
    let (mut rt, a, b, target, now) = fixture();
    strike(&mut rt, a, target, 7, now);
    simulate_projectiles(
        &mut rt.players,
        &mut rt.minions,
        &mut rt.structures,
        &mut rt.neutrals,
        &mut rt.team_buffs,
        &mut rt.projectiles,
        &mut rt.game_state,
        0.25,
        now + Duration::from_millis(250),
    );
    assert_eq!(rt.players[&b].state.hp, MAX_HP - 12.0);
    assert!(rt.projectiles.is_empty());
    let attacker = rt.players.get_mut(&a).unwrap();
    attacker.state.hp = 0.0;
    attacker.respawn_at = Some(now + Duration::from_secs(1));
    refresh_basic_attack_cooldowns(&mut rt.players, now);
    assert!(rt.players[&a].last_basic_attack_at.is_none());
    handle_respawns(
        &mut rt.players,
        &rt.structures,
        &rt.map_layout,
        &rt.game_state,
        now + Duration::from_secs(1),
    );
    assert_eq!(rt.players[&a].state.basic_attack_request_id, 7);
    assert_eq!(rt.players[&a].state.basic_attack_remaining_secs, 0.0);
    strike(&mut rt, a, target, 7, now + Duration::from_secs(2));
    assert!(rt.projectiles.is_empty());
}

#[test]
fn actual_udp_receiver_accepts_the_basic_wire_contract_once() {
    let (mut rt, _, b, target, _) = fixture();
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sender.local_addr().unwrap();
    rt.handle_packet(
        addr,
        ClientPacket::Join {
            team: Team::Green,
            character: CharacterChoice::Ipfs,
            hero_class: HeroClass::Warrior,
            avatar: None,
            sprite_character: None,
            session_id: Some("udp-basic".to_owned()),
        },
        Instant::now(),
    );
    let attacker = rt.players.get_mut(&addr).unwrap();
    attacker.state.x = 0.0;
    attacker.state.z = 0.0;
    attacker.state.mana = 0.0;
    let bytes = serde_json::to_vec(
        &serde_json::json!({"type":"basic_attack","target":{"kind":"player","id":target.id},
        "server_epoch":rt.server_epoch,"match_id":rt.match_id,"request_id":1}),
    )
    .unwrap();
    for _ in 0..2 {
        sender
            .send_to(&bytes, rt.socket.local_addr().unwrap())
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while rt.projectiles.is_empty() && Instant::now() < deadline {
        rt.receive_packets();
    }
    assert_eq!(rt.projectiles.len(), 1);
    assert_eq!(rt.players[&addr].state.basic_attack_request_id, 1);
    simulate_projectiles(
        &mut rt.players,
        &mut rt.minions,
        &mut rt.structures,
        &mut rt.neutrals,
        &mut rt.team_buffs,
        &mut rt.projectiles,
        &mut rt.game_state,
        0.25,
        Instant::now(),
    );
    assert_eq!(rt.players[&b].state.hp, MAX_HP - 12.0);
}
