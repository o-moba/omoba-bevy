use shared::vision::*;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use shared::HeroClass;
use shared::combat::{CombatEntity, CombatEntityKind, CombatEvent, MinionKind};
use shared::map::{Lane, Team};
use shared::shop::{ItemBonuses, ItemId, PurchaseReceipt};
use shared::wire::{
    CharacterChoice, ClientPacket, GameState, MinionBrainState, MinionState, MinionTargetKind,
    PlayerState, ServerPacket, TargetId, TargetKind,
};

use super::*;
use crate::balance::MINION_SPAWN_HEIGHT;
use crate::basic_attack::handle_basic_attack_request;
use crate::entities::{Minion, MinionAggroTarget, Vec3f};
use crate::game_world::TickCtx;
use crate::match_rules::MatchConfig;
use crate::neutrals::build_neutral_camps;
use crate::runtime::ServerRuntime;
use crate::sim::cast::handle_cast_request;
use crate::sim::minions::simulate_minions;
use crate::sim::projectiles::simulate_projectiles;
use crate::sim::towers::simulate_tower_attacks;
use crate::snapshot::build_players_snapshot;
use crate::world::build_structures;

// Convert authored fixture positions alongside the shipped arena. Sight
// radii, combat ranges and all resource assertions stay in gameplay units.
fn map_coordinate(value: f32) -> f32 {
    value * shared::map::WORLD_SCALE
}

fn fixture() -> (ServerRuntime, SocketAddr, SocketAddr, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    rt.targeting_qa = false;
    let now = Instant::now();
    let a = "127.0.0.1:58901".parse().unwrap();
    let b = "127.0.0.1:58902".parse().unwrap();
    for (addr, team, x) in [
        (a, Team::Green, map_coordinate(-18.0)),
        (b, Team::Blue, map_coordinate(-22.0)),
    ] {
        rt.handle_packet(
            addr,
            ClientPacket::Join {
                handheld: Default::default(),
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
        p.hero.z = map_coordinate(-8.0);
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
        debug_access: None,
        match_mode: "dev".into(),
        geometry_id: shared::map::GEOMETRY_ID.into(),
        map_profile: "verdant_default".into(),
        meta: Default::default(),
        join_error: None,
        your_id: rt.world.players[&addr].hero.identity.id,
        players: build_players_snapshot(
            &rt.world,
            Some(rt.world.players[&addr].hero.identity.id),
            now,
        ),
        scoreboard: rt.combat_log.ledger.live_scoreboard(),
        prematch: None,
        skill_effects: crate::skills::effects(&rt.world, now),
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
fn shockline_reveals_hit_npcs_for_its_team_until_expiry() {
    for kind in [TargetKind::Minion, TargetKind::Neutral] {
        let (mut rt, caster, opponent, now) = fixture();
        let p = rt.world.players.get_mut(&caster).unwrap();
        p.hero.x = 0.0;
        p.hero.z = 0.0;
        p.hero.skills.loadout = shared::loadout::preset_for_class(HeroClass::Wildspark);
        p.modifiers.unlock_all = true;
        let p = rt.world.players.get_mut(&opponent).unwrap();
        p.hero.x = 120.0;
        p.hero.z = 0.0;
        let id = if kind == TargetKind::Minion {
            rt.world
                .minions
                .insert(501, minion(501, Team::Blue, [6.0, 0.0]));
            501
        } else {
            let mut camps = build_neutral_camps(&mut 700);
            let id = *camps.keys().min().unwrap();
            let mut neutral = camps.remove(&id).unwrap();
            neutral.state.x = 6.0;
            neutral.state.z = 0.0;
            rt.world.neutrals.insert(id, neutral);
            id
        };
        crate::skills::cast(&mut rt.world, caster, 1, [24.0, 0.0], 1, now);
        let at = now + Duration::from_millis(300);
        crate::skills::tick(&mut rt.world, TickCtx { now: at, dt: 0.3 });
        if kind == TargetKind::Minion {
            rt.world.minions.get_mut(&id).unwrap().state.x = 40.0;
        } else {
            rt.world.neutrals.get_mut(&id).unwrap().state.x = 40.0;
        }
        let target = TargetId { kind, id };
        assert!(target_visible(Team::Green, target, &rt.world, at));
        if kind == TargetKind::Neutral {
            assert!(!target_visible(Team::Blue, target, &rt.world, at));
        }
        let ServerPacket::Snapshot {
            minions, neutrals, ..
        } = snapshot(&mut rt, caster, at)
        else {
            unreachable!()
        };
        assert_eq!(minions.len() + neutrals.len(), 1);

        let expired = now + Duration::from_secs(3);
        assert!(!target_visible(Team::Green, target, &rt.world, expired));
        let ServerPacket::Snapshot {
            minions, neutrals, ..
        } = snapshot(&mut rt, caster, expired)
        else {
            unreachable!()
        };
        assert!(minions.is_empty() && neutrals.is_empty());
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
    rt.world.players.get_mut(&a).unwrap().hero.x = map_coordinate(-20.0);
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
fn weapon_toggle_in_brush_does_not_reveal_but_a_hostile_cast_does() {
    let (mut rt, a, b, now) = fixture();
    let p = rt.world.players.get_mut(&b).unwrap();
    p.hero.skills.loadout = shared::loadout::preset_for_class(HeroClass::Wildspark);
    p.modifiers.unlock_all = true;
    let hidden = |rt: &mut ServerRuntime, at: Instant| {
        let ServerPacket::Snapshot { vision, .. } = snapshot(rt, b, at) else {
            panic!()
        };
        vision.unwrap().local_hidden
    };
    assert!(hidden(&mut rt, now));

    // Wild Switch is accepted and stamps its slot like any other cast.
    let origin = [rt.world.players[&b].hero.x, rt.world.players[&b].hero.z];
    crate::skills::cast(&mut rt.world, b, 0, origin, 1, now);
    let p = &rt.world.players[&b];
    assert_eq!(p.hero.skills.mode, shared::loadout::WeaponMode::Rockets);
    assert_eq!(p.timers.last_cast_at[0], Some(now));
    assert!(!revealed(p, now));
    assert!(!target_visible(Team::Green, target(&rt, b), &rt.world, now));
    assert!(hidden(&mut rt, now));

    // An aimed skill from the same brush still reveals for the usual window.
    let at = now + Duration::from_millis(400);
    let aim = [rt.world.players[&a].hero.x, rt.world.players[&a].hero.z];
    crate::skills::cast(&mut rt.world, b, 1, aim, 2, at);
    assert_eq!(rt.world.players[&b].timers.last_cast_at[1], Some(at));
    assert!(revealed(&rt.world.players[&b], at));
    assert!(target_visible(Team::Green, target(&rt, b), &rt.world, at));
    assert!(!hidden(&mut rt, at));
    assert!(!target_visible(
        Team::Green,
        target(&rt, b),
        &rt.world,
        at + Duration::from_secs(2)
    ));
}
#[test]
fn receipt_from_a_hidden_hero_reaches_the_victim_with_the_source_withheld() {
    let (mut rt, a, b, now) = fixture();
    let (ta, tb) = (target(&rt, a), target(&rt, b));
    // B is concealed in brush; A stands in the open and takes the hit.
    let victim = &rt.world.players[&a].hero;
    let at = [victim.x, victim.z];
    let hit = CombatEvent {
        source: CombatEntity {
            kind: CombatEntityKind::Player,
            id: tb.id,
        },
        target: CombatEntity {
            kind: CombatEntityKind::Player,
            id: ta.id,
        },
        x: victim.x,
        y: victim.y,
        z: victim.z,
        amount: 20.0,
        action_slot: Some(2),
        trap_triggered: true,
        ..Default::default()
    };
    // A hidden unit that is not a hero keeps the old rule: no receipt.
    rt.world
        .minions
        .insert(501, minion(501, Team::Blue, [101.125, 99.375]));
    let from_minion = CombatEvent {
        source: CombatEntity {
            kind: CombatEntityKind::Minion,
            id: 501,
        },
        action_slot: None,
        trap_triggered: false,
        ..hit.clone()
    };
    rt.combat_log.extend(now, [hit, from_minion]);

    let ServerPacket::Snapshot {
        players,
        combat_events,
        ..
    } = snapshot(&mut rt, a, now)
    else {
        panic!()
    };
    assert_eq!(players.len(), 1, "the attacker itself stays hidden");
    assert_eq!(combat_events.len(), 1);
    let seen = combat_events[0].clone();
    assert_eq!(seen.source, CombatEntity::default());
    assert_eq!(seen.target.id, ta.id);
    assert_eq!(
        (seen.amount, seen.action_slot, seen.trap_triggered),
        (20.0, Some(2), true)
    );
    assert_eq!([seen.x, seen.z], at);

    // The attacker's own view names the attacker.
    let ServerPacket::Snapshot { combat_events, .. } = snapshot(&mut rt, b, now) else {
        panic!()
    };
    let own = combat_events
        .iter()
        .find(|event| event.id == seen.id)
        .unwrap();
    assert_eq!(own.source.id, tb.id);

    // Once the victim sees the attacker, the same retained receipt names it too.
    rt.world.players.get_mut(&a).unwrap().hero.x = map_coordinate(-20.0);
    let ServerPacket::Snapshot { combat_events, .. } = snapshot(&mut rt, a, now) else {
        panic!()
    };
    let named = combat_events
        .iter()
        .find(|event| event.id == seen.id)
        .unwrap();
    assert_eq!(named.source.id, tb.id);
    assert_eq!(named.source.kind, CombatEntityKind::Player);
}
#[test]
fn recipient_payloads_hide_actors_projectiles_events_and_pickup_receipts() {
    let (mut rt, a, b, now) = fixture();
    let ta = target(&rt, a);
    let tb = target(&rt, b);
    // Launch while visible, then hide target before replication.
    rt.world.players.get_mut(&b).unwrap().hero.x = map_coordinate(-18.5);
    cast(&mut rt, a, tb, now);
    rt.world.players.get_mut(&b).unwrap().hero.x = map_coordinate(-22.0);
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
            x: map_coordinate(-22.0),
            z: map_coordinate(-8.0),
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
    assert!(!encoded.contains(&format!("\"x\":{}", brush_layout()[0].center[0])));
    if let ServerPacket::Snapshot { forest_pickups, .. } = &mut packet {
        forest_pickups.push(shared::forest_pickups::ForestPickupState {
            id: 99,
            position: [map_coordinate(-18.0), map_coordinate(-8.0)],
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
    rt.world.players.get_mut(&b).unwrap().hero.x = map_coordinate(-18.5);
    cast(&mut rt, a, t, now);
    assert_eq!(rt.world.projectiles.len(), 1);
    rt.world.players.get_mut(&b).unwrap().hero.x = map_coordinate(-22.0);
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
fn zero_damage_trap_activation_respects_authoritative_snapshot_visibility() {
    let (mut rt, a, b, now) = fixture();
    let target = &rt.world.players[&b].hero;
    rt.combat_log.extend(
        now,
        [CombatEvent {
            source: CombatEntity {
                kind: CombatEntityKind::Player,
                id: rt.world.players[&a].hero.identity.id,
            },
            target: CombatEntity {
                kind: CombatEntityKind::Player,
                id: target.identity.id,
            },
            x: target.x,
            y: target.y,
            z: target.z,
            amount: 0.0,
            action_slot: Some(2),
            trap_triggered: true,
            ..Default::default()
        }],
    );
    // Sharing the brush makes both the victim and receipt position visible.
    rt.world.players.get_mut(&a).unwrap().hero.x = map_coordinate(-20.0);
    let ServerPacket::Snapshot { combat_events, .. } = snapshot(&mut rt, a, now) else {
        panic!()
    };
    assert_eq!(combat_events.len(), 1);
    assert!(combat_events[0].id > 0 && combat_events[0].trap_triggered);
    assert_eq!(combat_events[0].amount, 0.0);
    let receipt = combat_events[0].clone();
    // Leaving the brush hides the same retained event; the explicit trigger
    // flag must not become a position/audio side channel through concealment.
    rt.world.players.get_mut(&a).unwrap().hero.x = map_coordinate(-18.0);
    let ServerPacket::Snapshot { combat_events, .. } = snapshot(&mut rt, a, now) else {
        panic!()
    };
    assert!(combat_events.is_empty());
    rt.world.players.get_mut(&a).unwrap().hero.x = map_coordinate(-20.0);
    let ServerPacket::Snapshot { combat_events, .. } = snapshot(&mut rt, a, now) else {
        panic!()
    };
    assert_eq!(combat_events, [receipt]);
}

#[test]
fn bots_minions_and_towers_do_not_acquire_or_keep_concealed_heroes() {
    let (mut rt, a, b, now) = fixture();
    assert!(
        rt.combat_host()
            .bot_target(
                Team::Green,
                [map_coordinate(-18.0), map_coordinate(-8.0)],
                Lane::Mid,
                now
            )
            .is_none()
    );
    rt.world.minions.insert(
        501,
        minion(
            501,
            Team::Green,
            [map_coordinate(-18.0), map_coordinate(-8.0)],
        ),
    );
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
        s.state.x = map_coordinate(-18.0);
        s.state.z = map_coordinate(-8.0);
    }
    simulate_tower_attacks(&mut rt.world, now);
    assert!(rt.world.projectiles.is_empty());
    assert!(
        rt.world
            .structures
            .values()
            .all(|s| s.last_attack_at.is_none())
    );
    rt.world.players.get_mut(&a).unwrap().hero.x = map_coordinate(-20.0);
    assert!(
        rt.combat_host()
            .bot_target(
                Team::Green,
                [map_coordinate(-20.0), map_coordinate(-8.0)],
                Lane::Mid,
                now
            )
            .is_some()
    );
    simulate_tower_attacks(&mut rt.world, now);
    assert!(!rt.world.projectiles.is_empty());
}

#[test]
fn native_qa_route_is_walkable_and_final_destination_is_outside_green_sight() {
    let (mut rt, a, _, _) = fixture();
    rt.world.players.get_mut(&a).unwrap().hero.x = map_coordinate(-14.0);
    rt.world.structures = build_structures(&rt.world.map_layout);
    // Keep a world-unit distance beyond hero sight. The old endpoint falls
    // against a scaled trunk; ordinary move orders use the navigation route.
    let destination = [23.0, 0.0];
    assert!(!point_visible(
        &sources(Team::Green, &rt.world),
        destination,
        false
    ));
    let navigation = shared::navigation::world_navigation();
    let mut previous = [map_coordinate(-18.0), map_coordinate(-8.0)];
    let route = navigation.plan_route(previous, destination, &[]).unwrap();
    assert_eq!(route.last(), Some(&destination));
    for next in route {
        assert!(navigation.segment_clear(previous, next));
        previous = next;
    }
}

#[test]
fn both_teams_hide_other_brush_and_all_hidden_dynamic_channels() {
    let (mut rt, a, b, now) = fixture();
    let pa = rt.world.players.get_mut(&a).unwrap();
    pa.hero.x = brush_layout()[1].center[0];
    pa.hero.z = brush_layout()[1].center[1];
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
    let mut dead_minion = minion(
        501,
        Team::Blue,
        [map_coordinate(-18.0), map_coordinate(-8.0)],
    );
    dead_minion.state.hp = 0.0;
    rt.world.minions.insert(501, dead_minion);
    let mut camps = build_neutral_camps(&mut 502);
    let mut dead_neutral = camps.remove(&502).unwrap();
    dead_neutral.state.hp = 0.0;
    dead_neutral.dead_until = Some(now + Duration::from_secs(30));
    dead_neutral.state.x = map_coordinate(-18.0);
    dead_neutral.state.z = map_coordinate(-8.0);
    rt.world.neutrals.insert(502, dead_neutral);
    let mut authored = build_structures(&rt.world.map_layout);
    let mut dead_tower = authored.remove(&1).unwrap();
    dead_tower.state.id = 503;
    dead_tower.state.hp = 0.0;
    dead_tower.state.x = map_coordinate(-18.0);
    dead_tower.state.z = map_coordinate(-8.0);
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
                x: map_coordinate(-18.0),
                z: map_coordinate(-8.0),
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
            x: map_coordinate(-18.0),
            z: map_coordinate(-8.0),
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

/// Gives `addr` a private economy and request marks worth redacting.
fn enrich(rt: &mut ServerRuntime, addr: SocketAddr) {
    let p = rt.world.players.get_mut(&addr).unwrap();
    p.economy.gold = 345;
    p.economy.earned_gold = 265;
    p.economy.inventory = vec![ItemId::EmberBlade, ItemId::TrailBoots];
    p.economy.item_bonuses = shared::shop::item_bonuses(&p.economy.inventory);
    p.economy.last_purchase = Some(PurchaseReceipt {
        request_id: 4,
        match_id: rt.match_id,
        item_id: Some(ItemId::TrailBoots),
        error: None,
    });
    p.economy.basic_attack_request_id = 9;
    p.hero.utility.last_request_id = 6;
    p.hero.utility.dash_sequence = 2;
    p.hero.progress.level = 4;
    p.hero.progress.xp = 33;
    p.hero.progress.ranks = [2, 1, 2, 1];
    p.hero.hp = 123.0;
}

#[test]
fn visible_enemy_is_replicated_with_its_private_economy_blanked() {
    let (mut rt, a, b, now) = fixture();
    // Out of the brush, inside green sight.
    rt.world.players.get_mut(&b).unwrap().hero.x = map_coordinate(-18.5);
    enrich(&mut rt, b);
    let packet = snapshot(&mut rt, a, now);
    let ServerPacket::Snapshot { players, .. } = &packet else {
        panic!()
    };
    let live = &rt.world.players[&b];
    let enemy = players
        .iter()
        .find(|p| p.id == live.hero.identity.id)
        .expect("visible enemy is replicated");
    assert_eq!(enemy.gold, 0);
    assert_eq!(enemy.earned_gold, 0);
    assert!(enemy.inventory.is_empty());
    assert_eq!(enemy.item_bonuses, ItemBonuses::default());
    assert_eq!(enemy.last_purchase, None);
    assert_eq!(enemy.basic_attack_request_id, 0);
    assert_eq!(enemy.utility.last_request_id, 0);
    // Everything public is intact.
    assert_eq!(
        (enemy.x, enemy.y, enemy.z),
        (live.hero.x, live.hero.y, live.hero.z)
    );
    assert_eq!((enemy.hp, enemy.max_hp), (live.hero.hp, live.hero.max_hp));
    assert_eq!((enemy.level, enemy.xp), (4, 33));
    assert_eq!(enemy.ranks, [2, 1, 2, 1]);
    assert_eq!(enemy.utility.dash_sequence, 2);
    assert_eq!(
        serde_json::to_vec(enemy).unwrap(),
        serde_json::to_vec(&live.public_view(now, &rt.world.map_layout, &rt.world.game_state))
            .unwrap()
    );
    // The redaction is the only difference from the owner view.
    let owner = live.owner_view(now, &rt.world.map_layout, &rt.world.game_state);
    assert_eq!(owner.gold, 345);
    assert_eq!(owner.inventory.len(), 2);
    let restored = PlayerState {
        gold: owner.gold,
        earned_gold: owner.earned_gold,
        inventory: owner.inventory.clone(),
        item_bonuses: owner.item_bonuses,
        last_purchase: owner.last_purchase.clone(),
        basic_attack_request_id: owner.basic_attack_request_id,
        utility: owner.utility,
        ..enemy.clone()
    };
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(&owner).unwrap()
    );
}

#[test]
fn recipient_gets_its_own_owner_view_and_teammates_are_redacted() {
    let (mut rt, a, _b, now) = fixture();
    let c: SocketAddr = "127.0.0.1:58903".parse().unwrap();
    rt.world.ensure_connected(c, now);
    let mate = rt.world.players.get_mut(&c).unwrap();
    mate.joined = true;
    mate.hero.identity.team = Team::Green;
    mate.hero.x = map_coordinate(-16.0);
    mate.hero.z = map_coordinate(-8.0);
    enrich(&mut rt, a);
    enrich(&mut rt, c);
    let packet = snapshot(&mut rt, a, now);
    let ServerPacket::Snapshot { players, .. } = &packet else {
        panic!()
    };
    let own_live = &rt.world.players[&a];
    let own = players
        .iter()
        .find(|p| p.id == own_live.hero.identity.id)
        .unwrap();
    assert_eq!(own.gold, 345);
    assert_eq!(own.basic_attack_request_id, 9);
    assert_eq!(own.utility.last_request_id, 6);
    assert_eq!(
        serde_json::to_vec(own).unwrap(),
        serde_json::to_vec(&own_live.owner_view(now, &rt.world.map_layout, &rt.world.game_state))
            .unwrap(),
        "the recipient's own entry is the owner view"
    );
    let mate_live = &rt.world.players[&c];
    let mate = players
        .iter()
        .find(|p| p.id == mate_live.hero.identity.id)
        .expect("teammates are always replicated");
    assert_eq!(mate.team, Team::Green);
    assert_eq!(mate.gold, 0);
    assert_eq!(mate.earned_gold, 0);
    assert!(mate.inventory.is_empty());
    assert_eq!(mate.last_purchase, None);
    assert_eq!(mate.basic_attack_request_id, 0);
    assert_eq!(mate.utility.last_request_id, 0);
    assert_eq!((mate.level, mate.ranks), (4, [2, 1, 2, 1]));
    assert_eq!(
        serde_json::to_vec(mate).unwrap(),
        serde_json::to_vec(&mate_live.public_view(now, &rt.world.map_layout, &rt.world.game_state))
            .unwrap(),
        "a teammate is a non-owner"
    );
    // The sandbox broadcast skips the vision filter but builds the player
    // list the same way, so it is redacted too.
    let sandboxed = build_players_snapshot(&rt.world, Some(own_live.hero.identity.id), now);
    let mate = sandboxed
        .iter()
        .find(|p| p.id == mate_live.hero.identity.id)
        .unwrap();
    assert_eq!((mate.gold, mate.inventory.len()), (0, 0));
    assert_eq!(
        sandboxed
            .iter()
            .find(|p| p.id == own_live.hero.identity.id)
            .unwrap()
            .gold,
        345
    );
}
