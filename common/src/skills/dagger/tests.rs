use super::*;
use shared::loadout::{BuildRecipe, CoreId};
use shared::wire::{CharacterChoice, ClientPacket};
use shared::{HeroClass, PlayerActionKind};

fn addr(n: u16) -> SocketAddr {
    format!("127.0.0.1:{}", 57500 + n).parse().unwrap()
}
fn target(id: u64) -> TargetId {
    TargetId {
        kind: TargetKind::Player,
        id,
    }
}
fn fixture() -> (GameWorld, Instant, u64) {
    let now = Instant::now();
    let mut w = GameWorld::empty();
    for (n, class, team, x) in [
        (1, HeroClass::Adventurer, Team::Green, 0.0),
        (2, HeroClass::Warrior, Team::Blue, 2.0),
    ] {
        w.ensure_connected(addr(n), now);
        crate::session::handle_join_request(
            w.players.get_mut(&addr(n)).unwrap(),
            team,
            CharacterChoice::Ipfs,
            class,
            None,
            &w.map_layout,
            now,
        );
        let p = w.players.get_mut(&addr(n)).unwrap();
        p.hero.x = x;
        p.hero.z = 0.0;
        p.hero.hp = 1000.0;
        p.hero.max_hp = 1000.0;
        p.hero.mana = 500.0;
        p.hero.max_mana = 500.0;
        p.modifiers.unlock_all = true;
        p.hero.yaw = shared::math::hero_yaw_towards(1.0, 0.0);
    }
    let victim = w.players[&addr(2)].hero.identity.id;
    w.skill_runtime.dagger_chance.forced = Some(false);
    (w, now, victim)
}
fn attack(w: &mut GameWorld, slot: u8, request: u64, now: Instant) {
    super::super::cast(w, addr(1), slot, [2.0, 0.0], request, now);
}
fn pending(w: &mut GameWorld) -> Vec<CombatEvent> {
    std::mem::take(&mut w.skill_runtime.pending)
}

#[test]
fn rear_cone_uses_victim_forward_and_rejects_side_front_overlap_and_nan() {
    let yaw = shared::math::hero_yaw_towards(1.0, 0.0);
    for degrees in [120.1_f32, 150.0, 180.0, 210.0, 239.9] {
        let r = degrees.to_radians();
        assert!(in_rear([r.cos(), r.sin()], [0.0, 0.0], yaw));
    }
    for degrees in [0.0_f32, 90.0, 119.9, 240.1, 270.0] {
        let r = degrees.to_radians();
        assert!(!in_rear([r.cos(), r.sin()], [0.0, 0.0], yaw));
    }
    assert!(!in_rear([0.0, 0.0], [0.0, 0.0], yaw));
    assert!(!in_rear([f32::NAN, 0.0], [0.0, 0.0], yaw));
    assert!(!in_rear([-1.0, 0.0], [0.0, 0.0], f32::NAN));
}

#[test]
fn bluff_turns_stuns_and_blocks_transform_basic_cast_and_dash_without_escape() {
    let (mut w, now, victim) = fixture();
    w.players.get_mut(&addr(2)).unwrap().hero.yaw = shared::math::hero_yaw_towards(-1.0, 0.0);
    attack(&mut w, 1, 1, now);
    assert!(rear_hero(&w, [0.0, 0.0], target(victim)));
    assert_eq!(w.players[&addr(1)].hero.mana, 480.0);
    let before = (
        w.players[&addr(2)].hero.x,
        w.players[&addr(2)].hero.z,
        w.players[&addr(2)].hero.yaw,
    );
    let at = now + duration(0.5);
    let owner = w.players[&addr(1)].hero.identity.id;
    let v = w.players.get_mut(&addr(2)).unwrap();
    crate::session::handle_transform_request(v, &w.map_layout, 3.0, 0.0, 0.0, 0.0, at);
    crate::utility::handle_utility_request(
        v,
        &w.map_layout,
        &w.structures,
        &w.game_state,
        shared::utility::UtilityAction::Dash,
        [1.0, 0.0],
        1,
        at,
    );
    crate::basic_attack::handle_basic_attack_request(&mut w, addr(2), target(owner), 1, at);
    crate::sim::cast::handle_cast_request(&mut w, addr(2), target(owner), 0, at);
    assert_eq!(
        (
            w.players[&addr(2)].hero.x,
            w.players[&addr(2)].hero.z,
            w.players[&addr(2)].hero.yaw
        ),
        before
    );
    assert!(w.projectiles.is_empty());
    assert_eq!(w.players[&addr(2)].hero.utility.dash_sequence, 0);
    assert_eq!(w.players[&addr(2)].hero.skills.control.movement(at), 0.0);
    assert_eq!(
        w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(1.01)),
        1.0
    );
    // A modern technique-equipped victim cannot cast out of stun either.
    w.players.get_mut(&addr(2)).unwrap().hero.skills.loadout =
        Some(shared::loadout::resolve(&CoreId::Adventurer.preset()).unwrap());
    super::super::cast(&mut w, addr(2), 0, [0.0, 0.0], 1, at);
    assert_eq!(w.players[&addr(2)].hero.mana, 500.0);
    assert_eq!(
        w.players[&addr(2)].hero.last_action.kind,
        PlayerActionKind::None
    );
}

#[test]
fn invalid_bluff_and_strikes_spend_no_resources_or_rng() {
    for condition in 0..8 {
        let (mut w, now, _) = fixture();
        let victim = w.players.get_mut(&addr(2)).unwrap();
        match condition {
            0 => victim.hero.identity.team = Team::Green,
            1 => victim.hero.hp = 0.0,
            2 => victim.modifiers.god_mode = true,
            3 => victim.hero.skills.advanced.parry_until = Some(now + duration(2.0)),
            4 => victim.hero.skills.advanced.untargetable_until = Some(now + duration(2.0)),
            5 => victim.hero.skills.advanced.unstoppable_until = Some(now + duration(2.0)),
            6 => victim.hero.x = 9.0,
            _ => victim.joined = false,
        }
        attack(&mut w, 1, 1, now);
        assert_eq!(
            w.players[&addr(1)].hero.mana,
            500.0,
            "condition {condition}"
        );
        assert!(w.players[&addr(1)].timers.last_cast_at[1].is_none());
        assert_eq!(w.skill_runtime.dagger_chance.counter, 0);
        assert!(w.players[&addr(2)].hero.skills.control.stun_until.is_none());
    }
}

#[test]
fn three_strikes_have_distinct_normal_and_rear_damage_with_rank_scaling() {
    for (slot, damage, rear_multiplier) in [(0, 34.0, 1.0), (2, 24.0, 2.5), (3, 48.0, 1.5)] {
        for rear in [false, true] {
            let (mut w, now, _) = fixture();
            if !rear {
                w.players.get_mut(&addr(2)).unwrap().hero.yaw =
                    shared::math::hero_yaw_towards(-1.0, 0.0);
            }
            w.players.get_mut(&addr(1)).unwrap().hero.progress.ranks[slot as usize] = 2;
            attack(&mut w, slot, 1, now);
            let expected = damage * 1.1 * if rear { rear_multiplier } else { 1.0 };
            assert!((1000.0 - w.players[&addr(2)].hero.hp - expected).abs() < 0.0001);
            assert_eq!(pending(&mut w).len(), 1);
        }
    }
}

#[test]
fn vital_break_is_accepted_rear_only_leaves_one_hp_without_kill_reward_or_respawn() {
    let (mut w, now, victim) = fixture();
    w.skill_runtime.dagger_chance.forced = Some(true);
    let gold = w.players[&addr(1)].economy.gold;
    attack(&mut w, 2, 11, now);
    assert_eq!(w.players[&addr(2)].hero.hp, 1.0);
    assert_eq!(w.skill_runtime.dagger_chance.counter, 1);
    let events = pending(&mut w);
    assert_eq!(events.iter().filter(|e| e.near_lethal).count(), 1);
    assert_eq!(events.iter().map(|e| e.amount).sum::<f32>(), 999.0);
    assert!(events.iter().all(|e| !e.killed && e.target.id == victim));
    observe(&mut w, &events, now);
    assert_eq!(w.players[&addr(1)].economy.gold, gold);
    assert!(w.players[&addr(2)].timers.respawn_at.is_none());
    assert_eq!(w.players[&addr(2)].economy.death_streak, 0);
    // Same or lower accepted request can never roll again, even after cooldown.
    attack(&mut w, 2, 11, now + duration(20.0));
    attack(&mut w, 2, 10, now + duration(20.0));
    assert_eq!(w.skill_runtime.dagger_chance.counter, 1);
    assert!(pending(&mut w).is_empty());
}

#[test]
fn shields_immunity_infinite_hp_and_nonrear_hits_never_roll() {
    for condition in 0..5 {
        let (mut w, now, _) = fixture();
        w.skill_runtime.dagger_chance.forced = Some(true);
        let victim = w.players.get_mut(&addr(2)).unwrap();
        match condition {
            0 => victim.hero.skills.shields.push(Shield {
                source: 77,
                amount: 100.0,
                expires: now + duration(2.0),
            }),
            1 => victim.hero.skills.advanced.untargetable_until = Some(now + duration(2.0)),
            2 => victim.modifiers.infinite_hp = true,
            3 => victim.hero.yaw = shared::math::hero_yaw_towards(-1.0, 0.0),
            _ => victim.hero.hp = 10.0,
        }
        attack(&mut w, 2, 1, now);
        assert_eq!(
            w.skill_runtime.dagger_chance.counter, 0,
            "condition {condition}"
        );
        assert!(pending(&mut w).iter().all(|e| !e.near_lethal));
    }
    // A partial shield must be paid in full by normal damage before HP qualifies.
    let (mut w, now, _) = fixture();
    w.skill_runtime.dagger_chance.forced = Some(true);
    w.players
        .get_mut(&addr(2))
        .unwrap()
        .hero
        .skills
        .shields
        .push(Shield {
            source: 77,
            amount: 20.0,
            expires: now + duration(2.0),
        });
    attack(&mut w, 2, 1, now);
    assert_eq!(w.players[&addr(2)].hero.hp, 1.0);
    assert!(
        w.players[&addr(2)].hero.skills.shields.is_empty(),
        "the second damage pass removes the fully consumed shield"
    );
    let events = pending(&mut w);
    assert_eq!(events.iter().map(|event| event.amount).sum::<f32>(), 999.0);
    assert_eq!(events.iter().filter(|event| event.near_lethal).count(), 1);
}

#[test]
fn resource_cooldown_level_and_rejected_request_highwater_are_authoritative() {
    let (mut w, now, _) = fixture();
    w.players.get_mut(&addr(1)).unwrap().hero.mana = 10.0;
    attack(&mut w, 2, 1, now);
    assert_eq!(w.players[&addr(2)].hero.hp, 1000.0);
    w.players.get_mut(&addr(1)).unwrap().hero.mana = 500.0;
    attack(&mut w, 2, 1, now);
    assert_eq!(
        w.players[&addr(2)].hero.hp,
        1000.0,
        "rejected id stays consumed"
    );
    attack(&mut w, 2, 2, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 482.0);
    attack(&mut w, 2, 3, now + duration(0.5));
    assert_eq!(w.players[&addr(1)].hero.mana, 482.0);
    assert_eq!(w.skill_runtime.dagger_chance.counter, 1);
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.modifiers.unlock_all = false;
    p.hero.progress.level = 1;
    attack(&mut w, 3, 4, now + duration(1.0));
    assert!(w.players[&addr(1)].timers.last_cast_at[3].is_none());
}

#[test]
fn dagger_mastery_applies_ten_percent_to_rear_primary_basic_only() {
    for rear in [false, true] {
        let (mut w, now, victim) = fixture();
        if !rear {
            w.players.get_mut(&addr(2)).unwrap().hero.yaw =
                shared::math::hero_yaw_towards(-1.0, 0.0);
        }
        let owner = w.players[&addr(1)].hero.identity.id;
        let events = basic_impact(
            &mut w,
            target(victim),
            20.0,
            source(owner, shared::BASIC_ATTACK_ACTION_SLOT),
            Team::Green,
            0,
            now,
        );
        assert_eq!(events[0].amount, if rear { 22.0 } else { 20.0 });
        assert_eq!(w.skill_runtime.dagger_chance.counter, 0);
    }
}

#[test]
fn mixed_core_recipe_and_host_packet_entry_execute_same_dagger_kit() {
    let (mut w, now, victim) = fixture();
    let recipe = BuildRecipe {
        skills: CoreId::Adventurer.preset().skills,
        ..CoreId::Dawnweaver.preset()
    };
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.hero.identity.hero_class = HeroClass::Dawnweaver;
    p.hero.skills.loadout = Some(shared::loadout::resolve(&recipe).unwrap());
    let packet = ClientPacket::CastSkill {
        server_epoch: 12,
        match_id: 34,
        slot: 1,
        aim: [2.0, 0.0],
        request_id: 1,
    };
    crate::command::apply(&mut w, addr(1), &packet, 99, 34, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
    crate::command::apply(&mut w, addr(1), &packet, 12, 34, now);
    assert!(rear_hero(&w, [0.0, 0.0], target(victim)));
    attack(&mut w, 2, 2, now + duration(0.3));
    assert!(w.players[&addr(2)].hero.hp < 1000.0);
    assert!(w.players[&addr(1)].timers.last_cast_at[2].is_some());
}

#[test]
fn nonlethal_damage_floor_does_not_execute_even_when_f32_cannot_represent_hp_minus_one() {
    let (mut w, now, victim) = fixture();
    let hp = 1.0e20;
    w.players.get_mut(&addr(2)).unwrap().hero.hp = hp;
    let event = crate::combat_feedback::apply_player_nonlethal_damage(
        &mut w.players,
        victim,
        hp - 1.0,
        now,
    )
    .unwrap();
    assert_eq!(w.players[&addr(2)].hero.hp, 1.0);
    assert!(!event.killed);
    assert!(w.players[&addr(2)].timers.respawn_at.is_none());
}

#[test]
fn nonhero_strike_hits_normally_but_bluff_and_vital_break_exclude_it() {
    let (mut w, now, _) = fixture();
    w.players.remove(&addr(2));
    crate::world::spawn_minion_wave_for_team_lane(
        &w.map_layout,
        &mut w.minions,
        &mut w.next_minion_id,
        Team::Blue,
        shared::map::Lane::Mid,
    );
    let first = *w.minions.keys().min().unwrap();
    w.minions.retain(|id, _| *id == first);
    let minion = w.minions.get_mut(&first).unwrap();
    minion.state.x = 2.0;
    minion.state.z = 0.0;
    minion.state.hp = 1000.0;
    w.skill_runtime.dagger_chance.forced = Some(true);
    attack(&mut w, 1, 1, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
    attack(&mut w, 2, 2, now);
    assert_eq!(w.minions[&first].state.hp, 976.0);
    assert_eq!(w.skill_runtime.dagger_chance.counter, 0);
    assert!(pending(&mut w).iter().all(|e| !e.near_lethal));
}

#[test]
fn brush_hidden_enemy_cannot_be_selected_by_forged_melee_point() {
    let (mut w, now, victim) = fixture();
    let zone = shared::vision::brush_layout()[0];
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.hero.x = zone.center[0] + zone.radius + 0.1;
    p.hero.z = zone.center[1];
    let p = w.players.get_mut(&addr(2)).unwrap();
    p.hero.x = zone.center[0] + zone.radius - 1.0;
    p.hero.z = zone.center[1];
    let aim = [p.hero.x, p.hero.z];
    assert!(!crate::vision::target_visible(
        Team::Green,
        target(victim),
        &w,
        now
    ));
    super::super::cast(&mut w, addr(1), 1, aim, 1, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
    assert!(w.players[&addr(2)].hero.skills.control.stun_until.is_none());
}

#[test]
fn offline_practice_and_network_command_entry_produce_the_same_bluff_backstab_outcome() {
    let (mut online, now, _) = fixture();
    let (offline_world, _, _) = fixture();
    let mut offline = crate::offline::PracticeSession::new(now);
    offline.world = offline_world;
    offline.now = now;
    let player = offline.world.players.remove(&addr(1)).unwrap();
    offline
        .world
        .players
        .insert(crate::offline::LOCAL_ADDR, player);
    for (request, slot, elapsed) in [(1, 1, 0.0), (2, 2, 0.3)] {
        let packet = ClientPacket::CastSkill {
            server_epoch: crate::offline::EPOCH,
            match_id: 1,
            slot,
            aim: [2.0, 0.0],
            request_id: request,
        };
        let at = now + duration(elapsed);
        crate::command::apply(&mut online, addr(1), &packet, crate::offline::EPOCH, 1, at);
        offline.now = at;
        offline.command(packet);
    }
    assert_eq!(
        offline.world.players[&addr(2)].hero.hp,
        online.players[&addr(2)].hero.hp
    );
    assert_eq!(
        offline.world.players[&addr(2)].hero.yaw,
        online.players[&addr(2)].hero.yaw
    );
    assert_eq!(
        offline.world.players[&crate::offline::LOCAL_ADDR].hero.mana,
        online.players[&addr(1)].hero.mana
    );
    assert_eq!(offline.world.skill_runtime.dagger_chance.counter, 1);
}

#[test]
fn mixed_flow_recipe_primes_only_accepted_dagger_casts_and_restores_energy_on_followup_basics() {
    for slot in 0..4 {
        let (mut w, now, victim) = fixture();
        let recipe = BuildRecipe {
            skills: CoreId::Adventurer.preset().skills,
            ..CoreId::Stormfist.preset()
        };
        let p = w.players.get_mut(&addr(1)).unwrap();
        p.hero.identity.hero_class = HeroClass::Stormfist;
        p.hero.skills.loadout = Some(shared::loadout::resolve(&recipe).unwrap());
        p.hero.mana = 200.0;
        let owner = p.hero.identity.id;
        let cost = p
            .hero
            .skills
            .loadout
            .unwrap()
            .skill(SkillSlot::from_index(slot).unwrap())
            .ability
            .base_mana_cost;
        // Invalid aim does not prime a passive or reset the forge/combat lifecycle.
        super::super::cast(&mut w, addr(1), slot, [20.0, 0.0], 1, now);
        assert_eq!(w.players[&addr(1)].hero.skills.advanced.flow_attacks, 0);
        assert_eq!(w.players[&addr(1)].hero.skills.advanced.flow_until, None);
        assert_eq!(w.players[&addr(1)].hero.mana, 200.0);
        attack(&mut w, slot, 2, now);
        assert_eq!(w.players[&addr(1)].hero.mana, 200.0 - cost);
        assert_eq!(w.players[&addr(1)].hero.skills.advanced.flow_attacks, 2);
        assert_eq!(
            w.players[&addr(1)].hero.skills.advanced.flow_until,
            Some(now + duration(3.0))
        );
        assert_eq!(
            w.players[&addr(1)].hero.skills.advanced.attack_rate(now),
            1.4
        );
        basic_impact(
            &mut w,
            target(victim),
            10.0,
            source(owner, shared::BASIC_ATTACK_ACTION_SLOT),
            Team::Green,
            0,
            now + duration(0.1),
        );
        assert_eq!(w.players[&addr(1)].hero.mana, 215.0 - cost);
        assert_eq!(w.players[&addr(1)].hero.skills.advanced.flow_attacks, 1);
        // A fresh request rejected on cooldown cannot refill/extend an active proc.
        attack(&mut w, slot, 3, now + duration(0.3));
        assert_eq!(w.players[&addr(1)].hero.skills.advanced.flow_attacks, 1);
        assert_eq!(
            w.players[&addr(1)].hero.skills.advanced.flow_until,
            Some(now + duration(3.0))
        );
        basic_impact(
            &mut w,
            target(victim),
            10.0,
            source(owner, shared::BASIC_ATTACK_ACTION_SLOT),
            Team::Green,
            0,
            now + duration(0.4),
        );
        assert_eq!(w.players[&addr(1)].hero.mana, 230.0 - cost);
        assert_eq!(w.players[&addr(1)].hero.skills.advanced.flow_attacks, 0);
        assert_eq!(
            w.players[&addr(1)]
                .hero
                .skills
                .advanced
                .attack_rate(now + duration(0.4)),
            1.0
        );
        // Neither replay nor an insufficient-resource request can prime it later.
        attack(&mut w, slot, 2, now + duration(30.0));
        w.players.get_mut(&addr(1)).unwrap().hero.mana = 0.0;
        attack(&mut w, slot, 4, now + duration(30.0));
        assert_eq!(w.players[&addr(1)].hero.skills.advanced.flow_attacks, 0);
    }
}

#[test]
fn dagger_drag_hits_close_and_overlapping_enemies_without_throwing_a_projectile() {
    for x in [0.0, 0.2, 1.0, 2.6, 2.9] {
        let (mut w, now, _) = fixture();
        w.players.get_mut(&addr(2)).unwrap().hero.x = x;
        // Thumb drag sets the maximum-range point, not the victim centre.
        super::super::cast(&mut w, addr(1), 0, [2.6, 0.0], 1, now);
        let events = pending(&mut w);
        assert_eq!(events.len(), 1, "x={x}");
        assert!(events[0].amount >= 34.0);
        assert_eq!(events[0].style, ProjectileStyle::Crescent);
        assert!(w.projectiles.is_empty());
    }
    let (mut w, now, _) = fixture();
    w.players.get_mut(&addr(2)).unwrap().hero.x = 5.0;
    super::super::cast(&mut w, addr(1), 0, [2.6, 0.0], 1, now);
    assert!(pending(&mut w).is_empty());
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
}

#[test]
fn dagger_basic_uses_melee_contact_and_shared_hit_receipts() {
    let (mut w, now, victim) = fixture();
    let before = w.players[&addr(2)].hero.hp;
    crate::basic_attack::handle_basic_attack_request(&mut w, addr(1), target(victim), 1, now);
    assert!(w.projectiles.is_empty(), "dagger must stay in the hand");
    let events = pending(&mut w);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].action_slot,
        Some(shared::BASIC_ATTACK_ACTION_SLOT)
    );
    assert!((before - w.players[&addr(2)].hero.hp - 26.4).abs() < 0.001);
    crate::basic_attack::handle_basic_attack_request(&mut w, addr(1), target(victim), 2, now);
    assert!(
        pending(&mut w).is_empty(),
        "contact does not bypass cooldown"
    );
}

#[test]
fn winning_melee_contact_keeps_the_terminal_damage_receipt() {
    let (mut w, now, _) = fixture();
    w.structures = crate::world::build_configured_structures(&w.map_config);
    w.structures.retain(|_, s| {
        s.state.kind == shared::wire::StructureKind::BaseTower && s.state.team == Team::Blue
    });
    let (&id, tower) = w.structures.iter_mut().next().unwrap();
    tower.state.x = 2.0;
    tower.state.z = 0.0;
    tower.state.hp = 1.0;
    w.players.get_mut(&addr(1)).unwrap().modifiers.bypass_vision = true;
    crate::basic_attack::handle_basic_attack_request(
        &mut w,
        addr(1),
        TargetId {
            kind: TargetKind::Structure,
            id,
        },
        1,
        now,
    );
    assert!(matches!(
        w.game_state,
        shared::wire::GameState::Victory {
            winner: Team::Green
        }
    ));
    let mut log = crate::combat_feedback::CombatLog::default();
    crate::tick::skills(&mut w, &mut log, now, 0.0);
    assert_eq!(log.snapshot(now).len(), 1);
    assert!(log.snapshot(now)[0].killed);
    crate::tick::skills(&mut w, &mut log, now, 0.1);
    assert_eq!(
        log.snapshot(now).len(),
        1,
        "a retained final receipt is never replayed"
    );
}
