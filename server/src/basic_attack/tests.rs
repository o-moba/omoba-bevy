use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use shared::map::{Lane, Team};
use shared::shop::ItemId;
use shared::wire::{CharacterChoice, ClientPacket, GameState, StructureKind, TargetId, TargetKind};
use shared::{HeroClass, PlayerActionKind, SkillSlot, ability_for_class_slot, scaled_mana_cost};

use super::*;
use crate::balance::{
    MAX_HP, MAX_MANA, MINION_RADIUS, MOVEMENT_POSITION_TOLERANCE, NEUTRAL_RADIUS, PLAYER_GROUND_Y,
    PLAYER_HIT_RADIUS, PLAYER_SPEED,
};
use crate::game_world::TickCtx;
use crate::hero_stats::StatModifiers;
use crate::match_rules::MatchConfig;
use crate::progression::{apply_level_up, grant_player_xp};
use crate::runtime::ServerRuntime;
use crate::session::{handle_respawns, handle_transform_request, reset_player_round};
use crate::sim::projectiles::simulate_projectiles;
use crate::world::spawn_minion_wave_for_team_lane;
use crate::{hero_stats, hero_timers};

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
                prematch: false,
                team,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
                session_id: Some(format!("basic-{}", addr.port())),
                passport_ticket: None,
            },
            now,
        );
        let state = &mut runtime.world.players.get_mut(&addr).unwrap().hero;
        state.x = x;
        state.z = 0.0;
    }
    let target = TargetId {
        kind: TargetKind::Player,
        id: runtime.world.players[&b].hero.identity.id,
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
    let attacker = rt.world.players.get_mut(&a).unwrap();
    attacker.hero.mana = 0.0;
    attacker.timers.last_cast_at = [Some(now); 4];
    strike(&mut rt, a, target, 1, now);
    assert_eq!(rt.world.projectiles.len(), 1);
    let player = &rt.world.players[&a];
    assert_eq!(player.hero.mana, 0.0);
    assert_eq!(player.timers.last_cast_at, [Some(now); 4]);
    assert_eq!(player.hero.last_action.slot, BASIC_ATTACK_ACTION_SLOT);
    assert_eq!(player.hero.last_action.kind, PlayerActionKind::Attack);
    assert_eq!(player.hero.progress.ranks, [1; 4]);
    let player = rt.world.players.get_mut(&a).unwrap();
    player.hero.mana = MAX_MANA;
    player.timers.last_cast_at = [None; 4];
    rt.handle_packet(a, ClientPacket::Cast { target, slot: 0 }, now);
    assert_eq!(
        rt.world.projectiles.len(),
        2,
        "Q remains available while basic attack cools down"
    );
    assert_eq!(rt.world.players[&a].timers.last_basic_attack_at, Some(now));
    assert_eq!(rt.world.players[&a].hero.mana, MAX_MANA - 10.0);
    strike(&mut rt, a, target, 2, now + Duration::from_millis(899));
    assert_eq!(rt.world.projectiles.len(), 2);
    strike(&mut rt, a, target, 3, now + Duration::from_millis(901));
    assert_eq!(rt.world.projectiles.len(), 3);
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
        assert_eq!(rt.world.players[&a].economy.basic_attack_request_id, 0);
    }
    strike(&mut rt, a, target, 1, now);
    // Advance beyond the attack cooldown while keeping both ordinary peers
    // alive; stale strike packets deliberately do not refresh their liveness.
    for addr in [a, b] {
        rt.handle_packet(addr, ClientPacket::Ping, now + Duration::from_secs(3));
    }
    strike(&mut rt, a, target, 1, now + Duration::from_secs(5));
    strike(&mut rt, a, target, 0, now + Duration::from_secs(5));
    assert_eq!(rt.world.projectiles.len(), 1);
    let old_packet = packet(&rt, target, 100);
    rt.restart_round(now + Duration::from_secs(6));
    rt.handle_packet(a, old_packet, now + Duration::from_secs(7));
    assert_eq!(rt.world.players[&a].economy.basic_attack_request_id, 0);
    assert!(rt.world.players[&a].timers.last_basic_attack_at.is_none());
    assert_eq!(
        rt.player_view(a, now + Duration::from_secs(7))
            .basic_attack_remaining_secs,
        0.0
    );
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
            "unjoined" => rt.world.players.get_mut(&a).unwrap().joined = false,
            "dead attacker" => rt.world.players.get_mut(&a).unwrap().hero.hp = 0.0,
            "dead target" => rt.world.players.get_mut(&b).unwrap().hero.hp = 0.0,
            "friendly" => rt.world.players.get_mut(&b).unwrap().hero.identity.team = Team::Green,
            "missing" => target.id = u64::MAX,
            "far" => rt.world.players.get_mut(&b).unwrap().hero.x = 100.0,
            "lobby" => rt.world.game_state = GameState::Lobby,
            _ => unreachable!(),
        }
        strike(&mut rt, a, target, 1, now);
        assert!(rt.world.projectiles.is_empty(), "{condition}");
        assert!(
            rt.world.players[&a].timers.last_basic_attack_at.is_none(),
            "{condition}"
        );
        assert_eq!(rt.world.players[&a].hero.mana, MAX_MANA, "{condition}");
        assert_eq!(
            rt.world.players[&a].hero.last_action.sequence, 0,
            "{condition}"
        );
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
                rt.world.players.get_mut(&b).unwrap().hero.x = range + PLAYER_HIT_RADIUS - 0.01;
                PLAYER_HIT_RADIUS
            }
            TargetKind::Minion => {
                spawn_minion_wave_for_team_lane(
                    &rt.world.map_layout,
                    &mut rt.world.minions,
                    &mut rt.world.next_minion_id,
                    Team::Blue,
                    Lane::Mid,
                );
                let minion = rt.world.minions.values_mut().next().unwrap();
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
                    .world
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
                    .world
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
            rt.world.projectiles.len(),
            1,
            "surface within {range}+{radius} for {kind:?}"
        );
        rt.world.players.get_mut(&a).unwrap().hero.x = -0.1;
        strike(&mut rt, a, target, 2, now + Duration::from_secs(2));
        assert_eq!(
            rt.world.projectiles.len(),
            1,
            "outside surface range for {kind:?}"
        );
    }
    let (mut rt, a, _, _, now) = fixture();
    let base = rt
        .world
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
        rt.world.projectiles.is_empty(),
        "protected base cannot be attacked"
    );
}

#[test]
fn equipment_changes_basic_deadline_without_rescaling_elapsed_time() {
    let (mut rt, a, _, target, now) = fixture();
    strike(&mut rt, a, target, 1, now);
    let elapsed = Duration::from_millis(400);
    rt.world.players.get_mut(&a).unwrap().economy.item_bonuses =
        shared::shop::item_bonuses(&[ItemId::SwiftGrip, ItemId::EmberBlade]);
    let definition = basic_attack_for_class(HeroClass::Warrior);
    let duration = basic_attack_cooldown(definition, rt.world.players[&a].economy.item_bonuses);
    assert!(
        (rt.player_view(a, now + elapsed).basic_attack_remaining_secs
            - (duration - elapsed).as_secs_f32())
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
        .world
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
        &mut rt.world,
        TickCtx {
            now: now + Duration::from_millis(250),
            dt: 0.25,
        },
    );
    assert_eq!(rt.world.players[&b].hero.hp, MAX_HP - 12.0);
    assert!(rt.world.projectiles.is_empty());
    let attacker = rt.world.players.get_mut(&a).unwrap();
    attacker.hero.hp = 0.0;
    attacker.timers.respawn_at = Some(now + Duration::from_secs(1));
    hero_timers::normalize_hero_timers(&mut rt.world);
    assert!(rt.world.players[&a].timers.last_basic_attack_at.is_none());
    handle_respawns(&mut rt.world, now + Duration::from_secs(1));
    assert_eq!(rt.world.players[&a].economy.basic_attack_request_id, 7);
    assert_eq!(
        rt.player_view(a, now + Duration::from_secs(1))
            .basic_attack_remaining_secs,
        0.0
    );
    strike(&mut rt, a, target, 7, now + Duration::from_secs(2));
    assert!(rt.world.projectiles.is_empty());
}

#[test]
fn actual_udp_receiver_accepts_the_basic_wire_contract_once() {
    let (mut rt, _, b, target, _) = fixture();
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sender.local_addr().unwrap();
    rt.handle_packet(
        addr,
        ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Ipfs,
            hero_class: HeroClass::Warrior,
            avatar: None,
            sprite_character: None,
            session_id: Some("udp-basic".to_owned()),
            passport_ticket: None,
        },
        Instant::now(),
    );
    let attacker = rt.world.players.get_mut(&addr).unwrap();
    attacker.hero.x = 0.0;
    attacker.hero.z = 0.0;
    attacker.hero.mana = 0.0;
    let bytes = serde_json::to_vec(
        &serde_json::json!({"type":"basic_attack","target":{"kind":"player","id":target.id},
        "server_epoch":rt.server_epoch,"match_id":rt.match_id,"request_id":1}),
    )
    .unwrap();
    for _ in 0..2 {
        sender
            .send_to(&bytes, rt.transport.local_addr().unwrap())
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while rt.world.projectiles.is_empty() && Instant::now() < deadline {
        rt.receive_packets();
    }
    assert_eq!(rt.world.projectiles.len(), 1);
    assert_eq!(rt.world.players[&addr].economy.basic_attack_request_id, 1);
    simulate_projectiles(
        &mut rt.world,
        TickCtx {
            now: Instant::now(),
            dt: 0.25,
        },
    );
    assert_eq!(rt.world.players[&b].hero.hp, MAX_HP - 12.0);
}

#[test]
fn skill_recovery_blocks_cross_slot_bursts_without_spending_and_basics_overlap() {
    for class in HeroClass::ALL {
        let (mut rt, a, _, target, now) = fixture();
        let player = rt.world.players.get_mut(&a).unwrap();
        player.hero.identity.hero_class = class;
        apply_level_up(&mut player.hero); // W legitimately unlocks at two.
        let mana = player.hero.mana;
        let cost = scaled_mana_cost(ability_for_class_slot(class, SkillSlot::Q), 1);
        rt.handle_packet(a, ClientPacket::Cast { target, slot: 0 }, now);
        assert_eq!(rt.world.projectiles.len(), 1, "{class:?}: first Q");
        assert!((rt.world.players[&a].hero.mana - (mana - cost)).abs() < 0.0001);
        let after_q = rt.world.players[&a].hero.mana;
        for slot in [0, 1, 1] {
            rt.handle_packet(a, ClientPacket::Cast { target, slot }, now);
        }
        assert_eq!(rt.world.players[&a].hero.mana, after_q);
        assert_eq!(rt.world.players[&a].timers.last_cast_at[1], None);
        assert_eq!(rt.world.players[&a].hero.last_action.sequence, 1);
        let view = rt.player_view(a, now);
        assert!(view.skill_recovery_remaining_secs > 0.0);
        assert_eq!(view.skill_cooldown_remaining_secs[1], 0.0);
        strike(&mut rt, a, target, 1, now);
        assert_eq!(
            rt.world.projectiles.len(),
            2,
            "{class:?}: basic overlaps skill"
        );
        let recovery = Duration::from_secs_f32(shared::hero_balance::skill_recovery_secs(2));
        rt.handle_packet(a, ClientPacket::Cast { target, slot: 1 }, now + recovery);
        let p = &rt.world.players[&a];
        assert_eq!(p.timers.last_cast_at[1], Some(now + recovery));
        assert!(p.hero.hp <= p.hero.max_hp && p.hero.mana <= p.hero.max_mana);
        assert!(
            rt.player_view(a, now + recovery)
                .skill_cooldown_remaining_secs[1]
                > 0.0
        );
    }
}

#[test]
fn normal_level_ten_movement_and_item_growth_are_authoritative_and_bounded() {
    for class in HeroClass::ALL {
        let (mut rt, a, _, _, now) = fixture();
        let p = rt.world.players.get_mut(&a).unwrap();
        p.hero.identity.hero_class = class;
        reset_player_round(p, &rt.world.map_layout, now);
        assert_eq!(p.hero.max_hp, shared::hero_balance::base_hp(class));
        grant_player_xp(&mut p.hero, u32::MAX);
        assert_eq!(p.hero.progress.level, 10);
        p.economy.item_bonuses =
            shared::shop::item_bonuses(&[ItemId::TrailBoots, ItemId::SwiftGrip]);
        p.hero.x = 0.0;
        p.hero.z = 0.0;
        let expected = PLAYER_SPEED * 1.24 * 1.08 * 0.1;
        handle_transform_request(
            p,
            &rt.world.map_layout,
            expected,
            PLAYER_GROUND_Y,
            0.0,
            0.0,
            now + Duration::from_millis(100),
        );
        assert!(
            (p.hero.x - expected).abs() < 0.0001,
            "{class:?}: legal movement"
        );
        handle_transform_request(
            p,
            &rt.world.map_layout,
            20.0,
            PLAYER_GROUND_Y,
            0.0,
            0.0,
            now + Duration::from_millis(200),
        );
        assert!(p.hero.x <= expected * 2.0 + MOVEMENT_POSITION_TOLERANCE + 0.0001);
        let cooldown = hero_stats::basic_attack_cooldown(p);
        assert!(
            (cooldown.as_secs_f32()
                - basic_attack_for_class(class).cooldown_secs
                    / shared::hero_balance::attack_rate_multiplier(class, 10)
                    / 1.12)
                .abs()
                < 0.0001
        );
        let max_hp = p.hero.max_hp;
        p.hero.hp = 0.0;
        p.timers.respawn_at = Some(now);
        handle_respawns(&mut rt.world, now);
        assert_eq!(rt.world.players[&a].hero.hp, max_hp);
        assert_eq!(
            hero_stats::basic_attack_cooldown(&rt.world.players[&a]),
            cooldown
        );
    }
}

#[test]
fn explicit_sandbox_no_cooldowns_bypasses_inter_skill_recovery() {
    let (mut rt, a, _, target, now) = fixture();
    let p = rt.world.players.get_mut(&a).unwrap();
    p.modifiers = StatModifiers {
        no_cooldowns: true,
        unlock_all: true,
        ..Default::default()
    };
    for slot in [0, 1, 0] {
        rt.handle_packet(a, ClientPacket::Cast { target, slot }, now);
    }
    assert_eq!(rt.world.players[&a].hero.last_action.sequence, 3);
    let view = rt.player_view(a, now);
    assert_eq!(view.skill_recovery_remaining_secs, 0.0);
    assert_eq!(view.skill_cooldown_remaining_secs, [0.0; 4]);
}
