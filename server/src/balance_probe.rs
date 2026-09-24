//! Controlled balance measurements through the ordinary authoritative combat path.
//! No sandbox/god mode/unlimited mana. See docs/balance-tuning.md for limitations.

use shared::wire::ClientPacket;

use crate::sim::cast::apply_skill_upgrade;

use shared::unlocked_slots_for_level;

use crate::hero_stats;

use crate::progression::grant_player_xp;

use crate::game_world::TickCtx;

use shared::wire::GameState;

use shared::scaled_mana_cost;

use crate::sim::cast::handle_cast_request;

use crate::match_rules::MatchConfig;

use crate::session::handle_transform_request_with_structures;

use std::time::Duration;

use crate::balance::MOVEMENT_POSITION_TOLERANCE;

use shared::TargetingMode;

use std::net::UdpSocket;

use crate::sim::regenerate_mana;

use shared::wire::TargetId;

use crate::runtime::ServerRuntime;

use shared::map::Team;

use std::time::Instant;

use crate::sim::projectiles::simulate_projectiles;

use shared::wire::TargetKind;

use shared::ability_for_class_slot;

use crate::basic_attack;

use shared::SkillSlot;

use shared::wire::CharacterChoice;

use shared::HeroClass;

use std::net::SocketAddr;

fn fixture(
    attacker: HeroClass,
    defender: HeroClass,
    level: u32,
) -> (ServerRuntime, SocketAddr, SocketAddr, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    let now = Instant::now();
    let a: SocketAddr = "127.0.0.1:59101".parse().unwrap();
    let b: SocketAddr = "127.0.0.1:59102".parse().unwrap();
    for (addr, team, hero, x) in [
        (a, Team::Green, attacker, 0.0),
        (b, Team::Blue, defender, 3.0),
    ] {
        rt.handle_packet(
            addr,
            ClientPacket::Join {
                prematch: false,
                team,
                character: CharacterChoice::Cube,
                hero_class: hero,
                avatar: None,
                sprite_character: None,
                session_id: None,
                passport_ticket: None,
            },
            now,
        );
        let p = rt.world.players.get_mut(&addr).unwrap();
        while p.hero.progress.level < level {
            let xp = p.hero.progress.next_level_xp;
            grant_player_xp(&mut p.hero, xp);
        }
        p.hero.x = x;
        p.hero.z = 0.0;
        for slot in [0, 0, 1, 1, 2, 2, 3, 3] {
            if unlocked_slots_for_level(level)[slot] {
                apply_skill_upgrade(p, slot as u8);
            }
        }
        p.hero.hp = p.hero.max_hp;
        p.hero.mana = p.hero.max_mana;
    }
    rt.world.structures.clear();
    rt.world.minions.clear();
    rt.world.neutrals.clear();
    rt.world.game_state = GameState::Running;
    (rt, a, b, now)
}
fn cast(rt: &mut ServerRuntime, a: SocketAddr, target: TargetId, slot: u8, now: Instant) {
    handle_cast_request(&mut rt.world, a, target, slot, now);
}
fn strike(rt: &mut ServerRuntime, a: SocketAddr, target: TargetId, id: u64, now: Instant) {
    basic_attack::handle_basic_attack_request(&mut rt.world, a, target, id, now);
}
fn tick(rt: &mut ServerRuntime, dt: f32, now: Instant) {
    regenerate_mana(&mut rt.world.players, dt);
    let events = simulate_projectiles(&mut rt.world, TickCtx { now, dt });
    rt.combat_log.extend(now, events);
}
fn measure(
    attacker: HeroClass,
    defender: HeroClass,
    level: u32,
    policy: &str,
    order: [u8; 3],
    reply_delay: Option<f32>,
) -> serde_json::Value {
    let (mut rt, a, b, start) = fixture(attacker, defender, level);
    let initial_a = rt.world.players[&a].hero.clone();
    let initial_b = rt.world.players[&b].hero.clone();
    let target = TargetId {
        kind: TargetKind::Player,
        id: initial_b.identity.id,
    };
    let reverse = TargetId {
        kind: TargetKind::Player,
        id: initial_a.identity.id,
    };
    let basic_damage = hero_stats::basic_attack_damage(&rt.world.players[&a]);
    let basic_cd = hero_stats::basic_attack_cooldown(&rt.world.players[&a]).as_secs_f32();
    // Measure accepted movement under a deliberately overlong request, removing
    // the documented positional tolerance. Start is an unobstructed lane point.
    handle_transform_request_with_structures(
        rt.world.players.get_mut(&a).unwrap(),
        &rt.world.map_layout,
        &rt.world.structures,
        100.0,
        0.5,
        0.0,
        0.0,
        start + Duration::from_millis(100),
    );
    let move_speed = (rt.world.players[&a].hero.x - MOVEMENT_POSITION_TOLERANCE) / 0.1;
    rt.world.players.get_mut(&a).unwrap().hero.x = 0.0;
    rt.world
        .players
        .get_mut(&a)
        .unwrap()
        .timers
        .last_movement_at = start;
    let dt = 1.0 / 120.0;
    let mut ttk = None;
    let mut spent = 0.0f32;
    let mut regen = 0.0f32;
    let mut starved = 0u32;
    let mut burst = 0.0f32;
    let mut actions = 0u64;
    for step in 0..7200u64 {
        let elapsed = step as f32 * dt;
        let now = start + Duration::from_secs_f32(elapsed);
        for (who, enemy, active) in [
            (a, target, true),
            (b, reverse, reply_delay.is_some_and(|d| elapsed >= d)),
        ] {
            if !active || rt.world.players[&who].hero.hp <= 0.0 {
                continue;
            }
            let before = rt.world.players[&who].hero.mana;
            if policy != "q_only" {
                strike(&mut rt, who, enemy, step + 1, now);
            }
            if policy != "basic_only" {
                for slot in order {
                    let p = &rt.world.players[&who];
                    let def = ability_for_class_slot(
                        p.hero.identity.hero_class,
                        SkillSlot::from_index(slot).unwrap(),
                    );
                    if (policy == "q_only" && slot != 0) || def.projectile_damage.is_none() {
                        continue;
                    }
                    if who == a
                        && unlocked_slots_for_level(level)[slot as usize]
                        && before < scaled_mana_cost(def, p.hero.progress.ranks[slot as usize])
                    {
                        starved += 1;
                    }
                    cast(&mut rt, who, enemy, slot, now);
                }
                // Mana restore is part of the real kit. Healing only in the
                // bidirectional scenarios, never turn the passive target active.
                for slot in [1, 2, 3] {
                    let p = &rt.world.players[&who];
                    let def = ability_for_class_slot(
                        p.hero.identity.hero_class,
                        SkillSlot::from_index(slot).unwrap(),
                    );
                    let restore = def
                        .self_mana_restore
                        .is_some_and(|m| p.hero.max_mana - p.hero.mana >= m);
                    let heal = reply_delay.is_some()
                        && def
                            .self_heal
                            .is_some_and(|h| p.hero.max_hp - p.hero.hp >= h);
                    if restore || heal {
                        cast(&mut rt, who, enemy, slot, now);
                    }
                }
            }
            if who == a {
                spent += (before - rt.world.players[&who].hero.mana).max(0.0);
            }
        }
        let mana = rt.world.players[&a].hero.mana;
        tick(&mut rt, dt, now);
        regen += (rt.world.players[&a].hero.mana - mana).max(0.0);
        if elapsed <= 1.0 {
            burst = initial_b.max_hp - rt.world.players[&b].hero.hp;
        }
        actions = rt.world.players[&a].hero.last_action.sequence;
        if rt.world.players[&b].hero.hp <= 0.0 || rt.world.players[&a].hero.hp <= 0.0 {
            ttk = Some(elapsed + dt);
            break;
        }
    }
    serde_json::json!({"attacker":attacker.id(),"defender":defender.id(),"level":level,"policy":policy,"order":order,"reply_delay":reply_delay,"attacker_hp":initial_a.max_hp,"defender_hp":initial_b.max_hp,"mana":initial_a.max_mana,"ranks":initial_a.progress.ranks,"basic_damage":basic_damage,"basic_interval":basic_cd,"move_speed":move_speed,"ttk":ttk,"attacker_alive":rt.world.players[&a].hero.hp>0.0,"defender_alive":rt.world.players[&b].hero.hp>0.0,"mana_net_spent":spent,"mana_regenerated":regen,"mana_short_attempts":starved,"damage_first_second":burst,"accepted_actions":actions})
}
fn sustain(class: HeroClass) -> serde_json::Value {
    let (mut rt, a, _, start) = fixture(class, class, 10);
    let max = rt.world.players[&a].hero.max_hp;
    let mana = rt.world.players[&a].hero.max_mana;
    rt.world.players.get_mut(&a).unwrap().hero.hp = max * 0.5;
    let mut healing = 0.0f32;
    let dt = 1.0 / 120.0;
    for step in 0..3600 {
        let now = start + Duration::from_secs_f32(step as f32 * dt);
        // Nonlethal damage creates a constant need for sustain; measure only
        // health actually restored, not attempted overheal.
        let p = rt.world.players.get_mut(&a).unwrap();
        p.hero.hp = (p.hero.hp - 10.0 * dt).max(1.0);
        let before = p.hero.hp;
        let target = TargetId {
            kind: TargetKind::Player,
            id: p.hero.identity.id,
        };
        for slot in [1, 2, 3] {
            let def = ability_for_class_slot(class, SkillSlot::from_index(slot).unwrap());
            if def.targeting == TargetingMode::SelfTarget {
                cast(&mut rt, a, target, slot, now);
            }
        }
        healing += rt.world.players[&a].hero.hp - before;
        regenerate_mana(&mut rt.world.players, dt);
        assert!(rt.world.players[&a].hero.hp <= max && rt.world.players[&a].hero.mana <= mana);
    }
    serde_json::json!({"class":class.id(),"seconds":30,"external_damage_per_second":10,"hp_restored":healing,"final_hp":rt.world.players[&a].hero.hp,"final_mana":rt.world.players[&a].hero.mana})
}
#[test]
fn measured_balance_matrix() {
    let mut rows = vec![];
    let mut supplements = vec![];
    for level in [1, 5, 10] {
        for a in HeroClass::ALL {
            for b in HeroClass::ALL {
                rows.push(measure(a, b, level, "all_in", [3, 2, 0], None));
            }
        }
    }
    for level in [1, 10] {
        for a in HeroClass::ALL {
            for policy in ["basic_only", "q_only"] {
                supplements.push(measure(a, a, level, policy, [3, 2, 0], None));
            }
            supplements.push(measure(a, a, level, "all_in", [0, 2, 3], None));
            supplements.push(measure(a, a, level, "all_in", [3, 2, 0], Some(0.5)));
        }
    }
    let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"setup":{"distance":3.0,"dt":1.0/120.0,"timeout":60,"items":[],"sandbox":false,"victim":"stationary; no healing except labeled bidirectional supplement","rank_policy":"spend normally available points in Q Q W W E E R R order, unlocked slots only","request_order":"basic then R E Q; retry every simulation step; self mana restore when meaningful","ttk_start":"first request, includes travel","repeatability_tolerance":1.0/120.0},"matrix":rows,"supplements":supplements,"sustain":HeroClass::ALL.map(sustain)});
    if let Ok(path) = std::env::var("OMOBA_BALANCE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(
        report["matrix"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["ttk"].is_number()),
        "finite kills required"
    );
}

/// Do not let the fixed benchmark priority hide a faster same-level rotation.
#[test]
fn every_damaging_skill_opening_preserves_response_windows() {
    let mut rows = Vec::new();
    for level in [1, 10] {
        for attacker in HeroClass::ALL {
            for defender in HeroClass::ALL {
                for order in [
                    [0, 2, 3],
                    [0, 3, 2],
                    [2, 0, 3],
                    [2, 3, 0],
                    [3, 0, 2],
                    [3, 2, 0],
                ] {
                    rows.push(measure(attacker, defender, level, "all_in", order, None));
                }
            }
        }
    }
    if let Ok(path) = std::env::var("OMOBA_BALANCE_OPENINGS") {
        std::fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
    for row in rows {
        let minimum = if row["level"] == 1 { 6.0 } else { 2.0 };
        assert!(
            row["ttk"].as_f64().is_some_and(|t| t >= minimum),
            "opening {row}"
        );
    }
}
