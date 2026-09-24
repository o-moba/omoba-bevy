//! Release regressions exercise the actual decoded-packet handler and UDP receiver.

use shared::wire::TargetKind;

use crate::balance::RESPAWN_DELAY;

use crate::neutrals::boss_spawn_delay;

use shared::shop::STARTING_GOLD;

use shared::wire::ProjectileState;

use crate::hero::Hero;

use shared::wire::CharacterChoice;

use shared::HeroClass;

use crate::entities::Projectile;

use crate::session::handle_respawns;

use crate::entities::Vec3f;

use shared::shop::ItemBonuses;

use shared::wire::TeamBuffKind;

use std::net::SocketAddr;

use crate::world::spawn_position_for_team_from_base;

use crate::sim::minions::award_minion_kill_rewards;

use crate::formation::joined_count;

use shared::wire::StructureKind;

use shared::wire::ServerPacket;

use crate::balance::MAX_HP;

use shared::wire::ClientPacket;

use crate::entities::ConnectedPlayer;

use crate::sim::towers::structure_is_protected;

use crate::match_rules::MatchMode;

use crate::hero::HeroEconomy;

use crate::progression::grant_player_xp;

use crate::balance::FIRST_MINION_WAVE_DELAY;

use crate::world::spawn_position_for_team;

use crate::balance::EMPTY_ROSTER_GRACE;

use shared::wire::GameState;

use shared::combat::CombatEntityKind;

use crate::balance::MINION_WAVE_INTERVAL;

use crate::runtime::PLAYER_TIMEOUT;

use crate::balance::STARTING_LEVEL;

use crate::match_rules::MatchConfig;

use crate::progression::xp_threshold_for_level;

use crate::world::spawn_minion_wave_for_team_lane;

use shared::combat::ProjectileStyle;

use std::time::Duration;

use std::collections::HashMap;

use crate::balance::MINIONS_PER_WAVE;

use shared::map::Lane;

use shared::PlayerActionKind;

use crate::balance::LEVEL_UP_HP_BONUS;

use crate::balance::MINION_KILL_GOLD;

use std::net::UdpSocket;

use shared::wire::TargetId;

use crate::runtime::ServerRuntime;

use crate::balance::MAX_MANA;

use crate::entities::DisconnectedSession;

use crate::balance::SESSION_RECLAIM_WINDOW;

use shared::map::Team;

use crate::sim::towers::apply_structure_damage;

use std::time::Instant;

use crate::snapshot::SNAPSHOT_INTERVAL;

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
        prematch: false,
        team,
        character: CharacterChoice::Ipfs,
        hero_class: HeroClass::default(),
        avatar: Some(shared::avatar_roster()[0].slug.clone()),
        sprite_character: None,
        session_id: Some(session.to_owned()),
        passport_ticket: None,
    }
}

fn progress(player: &mut ConnectedPlayer, now: Instant, dead: bool) {
    for _ in 0..5 {
        grant_player_xp(&mut player.hero, 180);
    }
    player.hero.hp = if dead { 0.0 } else { 31.0 };
    player.hero.mana = 17.0;
    player.economy.gold = 311;
    player.hero.x = 11.0;
    player.hero.z = -12.0;
    player.hero.yaw = 1.2;
    player.hero.progress.ranks = [3, 2, 1, 1];
    player.hero.progress.skill_points = 2;
    player.hero.last_action.sequence = 73;
    player.hero.last_action.kind = PlayerActionKind::Cast;
    player.hero.last_action.slot = 1;
    player.timers.last_cast_at = [Some(now); 4];
    player.timers.respawn_at = dead.then_some(now + RESPAWN_DELAY);
}

fn assert_gameplay_same(
    before: &(Hero, HeroEconomy),
    casts: [Option<Instant>; 4],
    respawn: Option<Instant>,
    after: &ConnectedPlayer,
) {
    assert_eq!(before.0, after.hero);
    assert_eq!(before.1, after.economy);
    assert_eq!(casts, after.timers.last_cast_at);
    assert_eq!(respawn, after.timers.respawn_at);
}

#[test]
fn handler_duplicate_and_reclaim_preserve_wounded_and_dead_full_state() {
    for dead in [false, true] {
        let mut rt = runtime(MatchConfig::release(1));
        let now = Instant::now();
        rt.handle_packet(addr(55101), join("original", Team::Blue), now);
        rt.handle_packet(addr(55102), join("opponent", Team::Blue), now);
        let player = rt.world.players.get_mut(&addr(55101)).unwrap();
        progress(player, now, dead);
        let before = (player.hero.clone(), player.economy.clone());
        let casts = player.timers.last_cast_at;
        let respawn = player.timers.respawn_at;
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
        let player = &rt.world.players[&addr(55101)];
        assert_gameplay_same(&before, casts, respawn, player);
        assert_eq!(player.session_id.as_deref(), Some("original"));
        // Cross-endpoint reuse while live is rejected and visible.
        rt.handle_packet(
            addr(55103),
            join("original", Team::Blue),
            now + Duration::from_secs(1),
        );
        assert_eq!(
            rt.world.players[&addr(55103)].join_error,
            Some(shared::protocol::JoinRejection::SessionActive)
        );
        assert!(!rt.world.players[&addr(55103)].joined);
        let reclaim_at = now + PLAYER_TIMEOUT + Duration::from_secs(1);
        rt.world.players.get_mut(&addr(55102)).unwrap().last_seen = reclaim_at;
        rt.handle_packet(
            addr(55104),
            ClientPacket::Hello {
                protocol_version: shared::protocol::PROTOCOL_VERSION,
            },
            reclaim_at,
        );
        rt.handle_packet(addr(55104), join("original", Team::Blue), reclaim_at);
        assert_gameplay_same(&before, casts, respawn, &rt.world.players[&addr(55104)]);
        assert!(rt.world.players[&addr(55104)].framed_snapshots);
        assert!(!rt.world.players.contains_key(&addr(55101)));
        assert_eq!(joined_count(&rt.world.players), 2);
    }
}

#[test]
fn handler_capacity_counts_reservations_and_expired_claim_is_fresh() {
    let mut rt = runtime(MatchConfig::release(1));
    let now = Instant::now();
    rt.handle_packet(addr(55201), join("a", Team::Blue), now);
    rt.handle_packet(addr(55202), join("b", Team::Blue), now);
    let old_id = rt.world.players[&addr(55201)].hero.identity.id;
    let later = now + PLAYER_TIMEOUT + Duration::from_secs(1);
    rt.world.players.get_mut(&addr(55202)).unwrap().last_seen = later;
    rt.handle_packet(addr(55203), join("replacement", Team::Blue), later);
    assert_eq!(rt.world.disconnected_sessions.len(), 1);
    assert_eq!(
        rt.world.players[&addr(55203)].join_error,
        Some(shared::protocol::JoinRejection::MatchFull)
    );
    rt.handle_packet(addr(55204), join("a", Team::Blue), later);
    assert_eq!(rt.world.players[&addr(55204)].hero.identity.id, old_id);
    assert_eq!(joined_count(&rt.world.players), 2);
    let expired = later + PLAYER_TIMEOUT + SESSION_RECLAIM_WINDOW + Duration::from_secs(1);
    rt.world.players.get_mut(&addr(55202)).unwrap().last_seen = expired;
    rt.handle_packet(addr(55205), join("replacement", Team::Blue), expired);
    assert!(rt.world.players[&addr(55205)].joined);
    rt.handle_packet(addr(55206), join("a", Team::Green), expired);
    assert_eq!(
        rt.world.players[&addr(55206)].join_error,
        Some(shared::protocol::JoinRejection::MatchFull)
    );
    assert!(
        !rt.world
            .players
            .values()
            .any(|player| player.hero.identity.id == old_id)
    );
}

#[test]
fn handler_release_rejects_debug_and_bad_protocol_while_explicit_dev_accepts_debug() {
    for config in [MatchConfig::release(1), MatchConfig::dev()] {
        let mut rt = runtime(config);
        let now = Instant::now();
        rt.handle_packet(addr(55301), join("a", Team::Green), now);
        progress(rt.world.players.get_mut(&addr(55301)).unwrap(), now, true);
        rt.handle_packet(addr(55301), ClientPacket::SetGodMode { enabled: true }, now);
        rt.handle_packet(
            addr(55301),
            ClientPacket::SetSpeedBoost { enabled: true },
            now,
        );
        let player = &rt.world.players[&addr(55301)];
        assert_eq!(player.modifiers.god_mode, config.mode == MatchMode::Dev);
        if config.mode == MatchMode::Release {
            assert_eq!(player.hero.hp, 0.0);
            assert!(player.timers.respawn_at.is_some());
            assert_eq!(player.modifiers.move_speed_mult, 1.0);
        } else {
            assert_eq!(player.hero.hp, player.hero.max_hp);
            assert!(player.modifiers.move_speed_mult > 1.0);
        }
        rt.handle_packet(
            addr(55302),
            ClientPacket::Hello {
                protocol_version: u16::MAX,
            },
            now,
        );
        rt.handle_packet(addr(55302), join("bad", Team::Blue), now);
        assert!(!rt.world.players[&addr(55302)].joined);
        assert_eq!(
            rt.world.players[&addr(55302)].join_error,
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
        assert!(rt.world.players[&addr(55302)].joined);
        assert_eq!(rt.world.players[&addr(55302)].join_error, None);
    }
}

fn contaminate_round(rt: &mut ServerRuntime, now: Instant) {
    for player in rt.world.players.values_mut() {
        progress(player, now, true);
    }
    for structure in rt.world.structures.values_mut() {
        structure.state.hp = 0.0;
        structure.last_attack_at = Some(now);
    }
    for neutral in rt.world.neutrals.values_mut() {
        neutral.state.hp = 0.0;
        neutral.state.x += 20.0;
        neutral.target_player_id = Some(1);
        neutral.last_attack_at = Some(now);
        neutral.dead_until = Some(now + Duration::from_secs(900));
    }
    rt.world
        .team_buffs
        .grant(Team::Green, TeamBuffKind::MutatioMight, now);
    spawn_minion_wave_for_team_lane(
        &rt.world.map_layout,
        &mut rt.world.minions,
        &mut rt.world.next_minion_id,
        Team::Green,
        Lane::Mid,
    );
    rt.world.projectiles.insert(
        99,
        Projectile {
            state: ProjectileState {
                source_kind: CombatEntityKind::Unknown,
                style: ProjectileStyle::Standard,
                action_slot: None,
                direction: [0.0; 3],
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
    rt.world.game_state = GameState::Victory {
        winner: Team::Green,
    };
    rt.victory_at = Some(now);
}

fn assert_clean_round(rt: &ServerRuntime) {
    for player in rt.world.players.values().filter(|player| player.joined) {
        assert_eq!((player.hero.hp, player.hero.max_hp), (MAX_HP, MAX_HP));
        assert_eq!(
            (player.hero.mana, player.hero.max_mana),
            (MAX_MANA, MAX_MANA)
        );
        assert_eq!(
            (
                player.hero.progress.xp,
                player.hero.progress.level,
                player.hero.progress.next_level_xp
            ),
            (0, STARTING_LEVEL, xp_threshold_for_level(STARTING_LEVEL))
        );
        assert_eq!(
            (player.economy.gold, player.hero.progress.skill_points),
            (STARTING_GOLD, 0)
        );
        assert!(player.economy.inventory.is_empty());
        assert_eq!(player.economy.item_bonuses, ItemBonuses::NONE);
        assert!(player.economy.last_purchase.is_none());
        assert_eq!(player.economy.purchase_sequence, 0);
        assert_eq!(player.hero.progress.ranks, [1; 4]);
        assert_eq!(player.hero.last_action.sequence, 0);
        assert_eq!(player.hero.last_action.kind, PlayerActionKind::None);
        assert_eq!(player.hero.last_action.slot, 0);
        assert_eq!(player.timers.last_cast_at, [None; 4]);
        assert_eq!(player.timers.respawn_at, None);
        assert_eq!(player.modifiers.move_speed_mult, 1.0);
        assert!(!player.modifiers.god_mode);
        let spawn = spawn_position_for_team(&rt.world.map_layout, player.hero.identity.team);
        assert_eq!(
            (player.hero.x, player.hero.z, player.hero.yaw),
            (spawn.x, spawn.z, 0.0)
        );
    }
    assert!(rt.world.projectiles.is_empty() && rt.world.minions.is_empty());
    assert!(rt.world.disconnected_sessions.is_empty());
    assert!(rt.world.team_buffs.snapshot(Instant::now()).is_empty());
    assert!(
        rt.world
            .structures
            .values()
            .all(|s| s.state.hp == s.state.max_hp && s.last_attack_at.is_none())
    );
    for neutral in rt.world.neutrals.values() {
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
    let player = rt.world.players.remove(&addr(55402)).unwrap();
    rt.world.disconnected_sessions.insert(
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
        rt.world.game_state,
        GameState::Forming {
            ready: 1,
            needed: 2
        }
    );
    assert_eq!(rt.match_started_at, None);
    rt.handle_packet(addr(55403), join("b", Team::Blue), now);
    assert_eq!(
        rt.world.players[&addr(55403)].hero.progress.level,
        STARTING_LEVEL
    );
    rt.tick(now, 0.01);
    assert!(matches!(rt.world.game_state, GameState::Starting { .. }));
    rt.tick(now + Duration::from_secs(3), 3.0);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(rt.world.minions.is_empty());
    let started = rt.match_started_at.unwrap();
    for player in rt.world.players.values_mut() {
        player.last_seen = started + FIRST_MINION_WAVE_DELAY;
    }
    rt.tick(
        started + FIRST_MINION_WAVE_DELAY - Duration::from_millis(1),
        0.0,
    );
    assert!(rt.world.minions.is_empty());
    rt.tick(started + FIRST_MINION_WAVE_DELAY, 0.0);
    assert_eq!(rt.world.minions.len(), MINIONS_PER_WAVE * 6);
    for neutral in rt
        .world
        .neutrals
        .values()
        .filter(|n| n.state.camp_type.is_boss())
    {
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
    let old_id = rt.world.players[&addr(55501)].hero.identity.id;
    contaminate_round(&mut rt, now);
    rt.world.game_state = GameState::Running;
    rt.victory_at = None;
    let empty_at = now + PLAYER_TIMEOUT + Duration::from_millis(1);
    rt.maintain_roster(empty_at);
    assert_eq!(rt.world.disconnected_sessions.len(), 1);
    rt.maintain_roster(empty_at + EMPTY_ROSTER_GRACE - Duration::from_millis(1));
    assert_eq!(rt.match_id, 1);
    // Pings cannot keep an empty match alive.
    rt.handle_packet(
        addr(55502),
        ClientPacket::Ping,
        empty_at + EMPTY_ROSTER_GRACE,
    );
    assert_eq!(rt.match_id, 2);
    assert_eq!(rt.world.game_state, GameState::Lobby);
    assert_clean_round(&rt);
    rt.handle_packet(
        addr(55502),
        join("old", Team::Blue),
        empty_at + EMPTY_ROSTER_GRACE,
    );
    assert_ne!(rt.world.players[&addr(55502)].hero.identity.id, old_id);
    assert_eq!(rt.world.players[&addr(55502)].hero.progress.level, 1);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert_clean_round(&rt);
}

#[test]
fn siege_blocks_cast_and_damage_until_own_lane_falls_and_resets() {
    let mut rt = runtime(MatchConfig::dev());
    let now = Instant::now();
    rt.handle_packet(addr(55601), join("siege", Team::Green), now);
    let base = rt
        .world
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::BaseTower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .clone();
    let own_lane = rt
        .world
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .id;
    let other_lane = rt
        .world
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Green)
        .unwrap()
        .state
        .id;
    let player = rt.world.players.get_mut(&addr(55601)).unwrap();
    player.hero.x = base.x - 2.0;
    player.hero.z = base.z;
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
    assert!(rt.world.projectiles.is_empty());
    assert_eq!(rt.world.players[&addr(55601)].hero.mana, MAX_MANA);
    apply_structure_damage(
        &mut rt.world.structures,
        base.id,
        50.0,
        Team::Green,
        &mut rt.world.game_state,
    );
    assert_eq!(rt.world.structures[&base.id].state.hp, base.hp);
    rt.world.structures.get_mut(&other_lane).unwrap().state.hp = 0.0;
    assert!(structure_is_protected(&rt.world.structures, base.id));
    apply_structure_damage(
        &mut rt.world.structures,
        own_lane,
        999.0,
        Team::Green,
        &mut rt.world.game_state,
    );
    assert!(!structure_is_protected(&rt.world.structures, base.id));
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
    assert_eq!(rt.world.projectiles.len(), 1);
    apply_structure_damage(
        &mut rt.world.structures,
        base.id,
        50.0,
        Team::Green,
        &mut rt.world.game_state,
    );
    assert_eq!(rt.world.structures[&base.id].state.hp, base.hp - 50.0);
    rt.restart_round(now);
    assert!(structure_is_protected(&rt.world.structures, base.id));
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
                award_minion_kill_rewards(&mut rt.world.players, Team::Green);
            }
            for level in [2, 4, 6] {
                let count = rt
                    .world
                    .players
                    .values()
                    .filter(|p| p.hero.progress.level >= level)
                    .count();
                if count > 0 {
                    milestones.entry((level, "first")).or_insert(wave);
                }
                if count == team_size as usize {
                    milestones.entry((level, "all")).or_insert(wave);
                }
            }
            let expected_gold = u32::from(team_size) * STARTING_GOLD
                + wave as u32 * MINIONS_PER_WAVE as u32 * 3 * MINION_KILL_GOLD;
            assert_eq!(
                rt.world
                    .players
                    .values()
                    .map(|p| p.economy.gold)
                    .sum::<u32>(),
                expected_gold
            );
            for p in rt.world.players.values() {
                assert_eq!(p.hero.progress.skill_points, p.hero.progress.level - 1);
                assert_eq!(p.hero.progress.ranks, [1; 4]);
            }
        }
        if team_size == 5 {
            assert_eq!((milestones[&(2, "first")], milestones[&(2, "all")]), (1, 1));
            assert_eq!((milestones[&(4, "first")], milestones[&(4, "all")]), (3, 3));
            assert_eq!((milestones[&(6, "first")], milestones[&(6, "all")]), (6, 6));
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
    let before = rt.world.players.get(&addr).map(|p| p.last_seen);
    socket
        .send_to(
            &serde_json::to_vec(&packet).unwrap(),
            rt.transport.local_addr().unwrap(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        rt.receive_packets();
        if rt.world.players.get(&addr).map(|p| p.last_seen) != before {
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
    rt.tick(Instant::now(), 0.0);
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
    rt.tick(now, 0.0);
    rt.tick(now, 3.0);
    assert_eq!(rt.world.game_state, GameState::Running);
    let base = rt
        .world
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::BaseTower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .clone();
    let lane = rt
        .world
        .structures
        .values()
        .find(|s| s.state.kind == StructureKind::Tower && s.state.team == Team::Blue)
        .unwrap()
        .state
        .id;
    for s in rt.world.structures.values_mut() {
        s.attack_range = 0.0;
    }
    rt.world.structures.get_mut(&lane).unwrap().state.hp = 0.0;
    rt.world.structures.get_mut(&base.id).unwrap().state.hp = 1.0;
    let player = rt.world.players.get_mut(&first_addr).unwrap();
    progress(player, now, false);
    player.hero.x = base.x - 2.0;
    player.hero.z = base.z;
    player.hero.mana = player.hero.max_mana;
    player.timers.last_cast_at = [None; 4];
    let camp = rt
        .world
        .neutrals
        .values_mut()
        .find(|n| !n.state.camp_type.is_boss())
        .unwrap();
    camp.state.hp = 0.0;
    camp.dead_until = Some(now + Duration::from_secs(40));
    rt.world
        .team_buffs
        .grant(Team::Green, TeamBuffKind::WendigoFavor, now);
    let before = (
        rt.world.players[&first_addr].hero.clone(),
        rt.world.players[&first_addr].economy.clone(),
    );
    send_udp(&first, &mut rt, join("udp-changed", Team::Blue));
    assert_gameplay_same(&before, [None; 4], None, &rt.world.players[&first_addr]);
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
    assert_eq!(rt.world.projectiles.len(), 1);
    rt.tick(Instant::now(), 1.0);
    assert_eq!(
        rt.world.game_state,
        GameState::Victory {
            winner: Team::Green
        }
    );
    let ServerPacket::Snapshot {
        meta: first_meta,
        game_state,
        ..
    } = udp_snapshot(&mut rt, &first, &mut assembler)
    else {
        panic!("expected snapshot")
    };
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
    } = udp_snapshot(&mut rt, &first, &mut assembler)
    else {
        panic!("expected snapshot")
    };
    assert_eq!(second_meta.server_epoch, first_meta.server_epoch);
    assert_eq!(second_meta.match_id, first_meta.match_id + 1);
    assert!(second_meta.snapshot_tick > first_meta.snapshot_tick);
    assert!(matches!(game_state, GameState::Starting { .. }));
    rt.tick(Instant::now(), 3.0);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert_clean_round(&rt);
    assert!(structure_is_protected(&rt.world.structures, base.id));
    println!(
        "LIVE_UDP_LIFECYCLE victory=green match_before={} match_after={} framed_max_bytes={} clean_second_running=true",
        first_meta.match_id,
        second_meta.match_id,
        shared::transport::MAX_DATAGRAM_BYTES
    );
}

#[test]
fn shared_xp_level_up_preserves_death_until_the_scheduled_respawn() {
    let mut rt = runtime(MatchConfig::dev());
    let now = Instant::now();
    rt.handle_packet(addr(55800), join("dead", Team::Green), now);
    rt.handle_packet(addr(55801), join("alive", Team::Green), now);
    let respawn_at = now + RESPAWN_DELAY;
    for player in rt.world.players.values_mut() {
        player.hero.progress.xp = player.hero.progress.next_level_xp - 1;
        player.hero.hp = 0.0;
        player.timers.respawn_at = Some(respawn_at);
    }
    rt.world.players.get_mut(&addr(55801)).unwrap().hero.hp = 40.0;
    rt.world
        .players
        .get_mut(&addr(55801))
        .unwrap()
        .timers
        .respawn_at = None;
    award_minion_kill_rewards(&mut rt.world.players, Team::Green);
    let dead = &rt.world.players[&addr(55800)];
    assert_eq!(dead.hero.progress.level, 2);
    assert_eq!(dead.hero.hp, 0.0);
    assert_eq!(dead.timers.respawn_at, Some(respawn_at));
    let alive = &rt.world.players[&addr(55801)];
    assert_eq!(alive.hero.progress.level, 2);
    assert_eq!(alive.hero.hp, 40.0 + LEVEL_UP_HP_BONUS);
    handle_respawns(&mut rt.world, respawn_at - Duration::from_millis(1));
    assert_eq!(rt.world.players[&addr(55800)].hero.hp, 0.0);
    handle_respawns(&mut rt.world, respawn_at);
    let dead = &rt.world.players[&addr(55800)];
    assert_eq!(dead.hero.hp, MAX_HP + LEVEL_UP_HP_BONUS);
    assert_eq!(dead.hero.hp, dead.hero.max_hp);
    assert_eq!(dead.timers.respawn_at, None);
    let spawn =
        spawn_position_for_team_from_base(&rt.world.structures, &rt.world.map_layout, Team::Green);
    assert_eq!((dead.hero.x, dead.hero.z), (spawn.x, spawn.z));
}
