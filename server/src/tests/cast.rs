use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::combat::MinionKind;
use shared::map::{Lane, Team};
use shared::wire::{
    CharacterChoice, MinionBrainState, MinionState, StructureKind, TargetId, TargetKind,
};
use shared::{
    HeroClass, PlayerActionKind, SkillSlot, TargetingMode, ability_for_class_slot,
    rank_effect_scale, scaled_cooldown, scaled_mana_cost,
};

use super::*;
use crate::balance::{MAX_MANA, MINION_MAX_HP, MINION_SPAWN_HEIGHT, PLAYER_HIT_RADIUS};
use crate::entities::{Minion, StructureRole, Vec3f};
use crate::game_world::GameWorld;
use crate::neutrals::build_neutral_camps;
use crate::progression::grant_player_xp;
use crate::session::handle_join_request;
use crate::sim::cast::{apply_skill_upgrade, handle_cast_request};
use crate::world::add_structure;

#[test]
fn cast_drains_mana_respects_cooldown_and_blocks_empty_mana() {
    let mut world = GameWorld::empty();
    let addr_a: SocketAddr = "127.0.0.1:51001".parse().unwrap();
    let addr_b: SocketAddr = "127.0.0.1:51002".parse().unwrap();
    let now = Instant::now();
    let q = ability_for_class_slot(HeroClass::Warrior, SkillSlot::Q);
    let b_id = setup_caster_and_target(
        &mut world,
        addr_a,
        addr_b,
        HeroClass::Warrior,
        q.cast_range * 0.5,
        now,
    );
    let target = TargetId {
        kind: TargetKind::Player,
        id: b_id,
    };
    let a_mana_before = world.players.get(&addr_a).unwrap().hero.mana;

    cast_slot(&mut world, addr_a, target, 0, now);
    assert_eq!(world.projectiles.len(), 1);
    {
        let action = world.players.get(&addr_a).unwrap().hero.last_action;
        assert_eq!(action.sequence, 1);
        assert_eq!(action.kind, PlayerActionKind::Attack);
        assert_eq!(action.slot, 0);
    }
    let mana_after_first = world.players.get(&addr_a).unwrap().hero.mana;
    assert!((mana_after_first - (a_mana_before - scaled_mana_cost(q, 1))).abs() < EPSILON);

    cast_slot(&mut world, addr_a, target, 0, now);
    assert_eq!(
        world.projectiles.len(),
        1,
        "second cast at same instant must be cooldown-blocked"
    );
    assert_eq!(
        world
            .players
            .get(&addr_a)
            .unwrap()
            .hero
            .last_action
            .sequence,
        1,
        "rejected casts must not advance cosmetic action state"
    );

    let later = now + scaled_cooldown(q, 1) + Duration::from_millis(1);
    cast_slot(&mut world, addr_a, target, 0, later);
    assert_eq!(
        world.projectiles.len(),
        2,
        "cast after cooldown should spawn another projectile"
    );
    assert_eq!(
        world
            .players
            .get(&addr_a)
            .unwrap()
            .hero
            .last_action
            .sequence,
        2
    );

    world.players.get_mut(&addr_a).unwrap().hero.mana = 0.0;
    cast_slot(&mut world, addr_a, target, 0, later + scaled_cooldown(q, 1));
    assert_eq!(
        world.projectiles.len(),
        2,
        "zero mana must not create a projectile"
    );
    assert_eq!(
        world
            .players
            .get(&addr_a)
            .unwrap()
            .hero
            .last_action
            .sequence,
        2
    );
}

#[test]
fn join_applies_class_and_normalizes_avatar() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:51050".parse().unwrap();
    let now = Instant::now();
    world.ensure_connected(addr, now);

    let valid_slug = omoba_passport::avatars::avatar_roster()[0].slug.clone();
    handle_join_request(
        world.players.get_mut(&addr).unwrap(),
        Team::Blue,
        CharacterChoice::Ipfs,
        HeroClass::Cleric,
        Some(valid_slug.as_str()),
        &world.map_layout,
        now,
    );
    {
        let state = &world.players.get(&addr).unwrap().hero;
        assert_eq!(state.identity.hero_class, HeroClass::Cleric);
        assert_eq!(state.identity.avatar.as_deref(), Some(valid_slug.as_str()));
    }

    // Unknown or malicious avatar slugs on a fresh admission fall back safely.
    world.players.remove(&addr);
    world.ensure_connected(addr, now);
    handle_join_request(
        world.players.get_mut(&addr).unwrap(),
        Team::Blue,
        CharacterChoice::Ipfs,
        HeroClass::Mage,
        Some("../../etc/passwd"),
        &world.map_layout,
        now,
    );
    let state = &world.players.get(&addr).unwrap().hero;
    assert_eq!(state.identity.hero_class, HeroClass::Mage);
    assert_eq!(state.identity.avatar, None);
}

#[test]
fn class_kits_apply_distinct_authoritative_numbers() {
    let now = Instant::now();
    let mut results = Vec::new();
    for (index, class) in [HeroClass::Warrior, HeroClass::Mage]
        .into_iter()
        .enumerate()
    {
        let mut world = GameWorld::empty();
        let caster: SocketAddr = format!("127.0.0.1:5210{index}").parse().unwrap();
        let victim: SocketAddr = format!("127.0.0.1:5220{index}").parse().unwrap();
        let q = ability_for_class_slot(class, SkillSlot::Q);
        let victim_id =
            setup_caster_and_target(&mut world, caster, victim, class, q.cast_range * 0.5, now);
        cast_slot(
            &mut world,
            caster,
            TargetId {
                kind: TargetKind::Player,
                id: victim_id,
            },
            0,
            now,
        );
        assert_eq!(world.projectiles.len(), 1);
        let projectile = world.projectiles.values().next().unwrap();
        assert!((projectile.damage - q.projectile_damage.unwrap()).abs() < EPSILON);
        let mana_spent = MAX_MANA - world.players.get(&caster).unwrap().hero.mana;
        assert!((mana_spent - q.base_mana_cost).abs() < EPSILON);
        results.push((projectile.damage, mana_spent));
    }
    assert!(
        (results[0].0 - results[1].0).abs() > EPSILON,
        "warrior and mage Q damage must differ"
    );
    assert!(
        (results[0].1 - results[1].1).abs() > EPSILON,
        "warrior and mage Q mana cost must differ"
    );
}

#[test]
fn self_target_abilities_apply_heal_and_respect_unlock_gates() {
    let mut world = GameWorld::empty();
    let now = Instant::now();
    let caster: SocketAddr = "127.0.0.1:52301".parse().unwrap();
    let other: SocketAddr = "127.0.0.1:52302".parse().unwrap();
    setup_caster_and_target(&mut world, caster, other, HeroClass::Cleric, 5.0, now);
    let self_target = TargetId {
        kind: TargetKind::Player,
        id: world.players.get(&caster).unwrap().hero.identity.id,
    };
    let w = ability_for_class_slot(HeroClass::Cleric, SkillSlot::W);
    assert_eq!(w.targeting, TargetingMode::SelfTarget);

    // Level 1: W is locked -> cast must be a complete no-op.
    {
        let state = &mut world.players.get_mut(&caster).unwrap().hero;
        state.hp = 40.0;
    }
    cast_slot(&mut world, caster, self_target, 1, now);
    {
        let player = world.players.get(&caster).unwrap();
        let hero = &player.hero;
        assert!((hero.hp - 40.0).abs() < EPSILON, "locked W must not heal");
        assert!((hero.mana - MAX_MANA).abs() < EPSILON);
        assert!(world.projectiles.is_empty());
        assert_eq!(player.hero.last_action.sequence, 0);
    }

    // Level 2 unlocks W: heal appears, mana is drained, still no projectile.
    {
        let player = world.players.get_mut(&caster).unwrap();
        let needed = player.hero.progress.next_level_xp;
        grant_player_xp(&mut player.hero, needed);
        assert_eq!(player.hero.progress.level, 2);
        player.hero.hp = 40.0;
        player.hero.mana = player.hero.max_mana;
    }
    cast_slot(&mut world, caster, self_target, 1, now);
    let player = world.players.get(&caster).unwrap();
    let hero = &player.hero;
    assert!(
        (hero.hp
            - (40.0
                + w.self_heal.unwrap()
                    * shared::hero_balance::ability_power_multiplier(
                        hero.identity.hero_class,
                        player.hero.progress.level
                    )))
        .abs()
            < EPSILON,
        "unlocked W must heal by the kit amount"
    );
    assert!((hero.mana - (hero.max_mana - w.base_mana_cost)).abs() < EPSILON);
    assert_eq!(player.hero.last_action.sequence, 1);
    assert_eq!(player.hero.last_action.kind, PlayerActionKind::Cast);
    assert_eq!(player.hero.last_action.slot, 1);
    assert!(
        world.projectiles.is_empty(),
        "self ability must not spawn a projectile"
    );
}

#[test]
fn rank_scaling_boosts_damage_and_upgrades_cap_at_max_rank() {
    let mut world = GameWorld::empty();
    let now = Instant::now();
    let caster: SocketAddr = "127.0.0.1:52401".parse().unwrap();
    let victim: SocketAddr = "127.0.0.1:52402".parse().unwrap();
    let q = ability_for_class_slot(HeroClass::Ranger, SkillSlot::Q);
    let victim_id = setup_caster_and_target(
        &mut world,
        caster,
        victim,
        HeroClass::Ranger,
        q.cast_range * 0.5,
        now,
    );

    // Upgrades consume points and cap at the shared max rank (3).
    {
        let player = world.players.get_mut(&caster).unwrap();
        player.hero.progress.skill_points = 5;
        for _ in 0..5 {
            apply_skill_upgrade(player, 0);
        }
        assert_eq!(player.hero.progress.ranks[0], q.max_rank);
        assert_eq!(
            player.hero.progress.skill_points,
            5 - u32::from(q.max_rank - 1),
            "only rank-raising upgrades may consume points"
        );
    }

    cast_slot(
        &mut world,
        caster,
        TargetId {
            kind: TargetKind::Player,
            id: victim_id,
        },
        0,
        now,
    );
    let projectile = world
        .projectiles
        .values()
        .next()
        .expect("rank-3 cast fires");
    let expected = q.projectile_damage.unwrap() * rank_effect_scale(q.max_rank);
    assert!((projectile.damage - expected).abs() < EPSILON);
    let mana_spent = MAX_MANA - world.players.get(&caster).unwrap().hero.mana;
    assert!((mana_spent - scaled_mana_cost(q, q.max_rank)).abs() < EPSILON);
}

#[test]
fn cast_range_validation_covers_target_types_and_rejects_far_targets() {
    let mut world = GameWorld::empty();
    let caster_addr: SocketAddr = "127.0.0.1:53001".parse().unwrap();
    let target_addr: SocketAddr = "127.0.0.1:53002".parse().unwrap();
    let now = Instant::now();
    // Mage Q has the longest basic range; targets below sit at half range.
    let q = ability_for_class_slot(HeroClass::Mage, SkillSlot::Q);
    let cast_range = q.cast_range;
    let cooldown = scaled_cooldown(q, 1);

    world.ensure_connected(caster_addr, now);
    world.ensure_connected(target_addr, now);
    handle_join_request(
        world.players.get_mut(&caster_addr).unwrap(),
        Team::Green,
        CharacterChoice::Ipfs,
        HeroClass::Mage,
        None,
        &world.map_layout,
        now,
    );
    handle_join_request(
        world.players.get_mut(&target_addr).unwrap(),
        Team::Blue,
        CharacterChoice::Wang,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );
    {
        let caster = world.players.get_mut(&caster_addr).unwrap();
        caster.hero.x = 0.0;
        caster.hero.z = 0.0;
    }
    {
        let target = world.players.get_mut(&target_addr).unwrap();
        target.hero.x = cast_range * 0.5;
        target.hero.z = 0.0;
    }

    world.minions.insert(
        10,
        Minion {
            state: MinionState {
                kind: MinionKind::Melee,
                attack_sequence: 0,
                id: 10,
                team: Team::Blue,
                lane: Lane::Mid,
                x: 0.0,
                y: MINION_SPAWN_HEIGHT,
                z: cast_range * 0.5,
                yaw: 0.0,
                hp: MINION_MAX_HP,
                max_hp: MINION_MAX_HP,
                state: MinionBrainState::Marching,
                target_kind: None,
                target_id: None,
            },
            path: Vec::new(),
            next_waypoint: 0,
            last_attack_at: None,
            aggro_target: None,
        },
    );

    let mut structure_id = 20;
    add_structure(
        &mut world.structures,
        &mut structure_id,
        StructureKind::Tower,
        StructureRole::LaneTower { lane: Lane::Mid },
        Team::Blue,
        Vec3f::new(cast_range * 0.5, 3.0, cast_range * 0.25),
    );

    let mut next_neutral_id = 9_001;
    world.neutrals = build_neutral_camps(&mut next_neutral_id);
    let neutral_id = *world.neutrals.keys().next().unwrap();
    {
        let neutral = world.neutrals.get_mut(&neutral_id).unwrap();
        neutral.state.x = cast_range * 0.25;
        neutral.state.z = cast_range * 0.5;
    }

    let target_player_id = world.players.get(&target_addr).unwrap().hero.identity.id;
    let targets = [
        TargetId {
            kind: TargetKind::Player,
            id: target_player_id,
        },
        TargetId {
            kind: TargetKind::Minion,
            id: 10,
        },
        TargetId {
            kind: TargetKind::Structure,
            id: 20,
        },
        TargetId {
            kind: TargetKind::Neutral,
            id: neutral_id,
        },
    ];

    for (index, target) in targets.into_iter().enumerate() {
        world.players.get_mut(&caster_addr).unwrap().hero.mana = MAX_MANA;
        handle_cast_request(
            &mut world,
            caster_addr,
            target,
            0,
            now + (cooldown + Duration::from_millis(1)) * index as u32,
        );
        assert_eq!(world.projectiles.len(), index + 1);
    }

    {
        let target = world.players.get_mut(&target_addr).unwrap();
        target.hero.x = cast_range + PLAYER_HIT_RADIUS + 10.0;
        target.hero.z = 0.0;
    }
    let mana_before = world.players.get(&caster_addr).unwrap().hero.mana;
    let projectile_count = world.projectiles.len();
    handle_cast_request(
        &mut world,
        caster_addr,
        TargetId {
            kind: TargetKind::Player,
            id: target_player_id,
        },
        0,
        now + (cooldown + Duration::from_millis(1)) * 5,
    );

    assert_eq!(world.projectiles.len(), projectile_count);
    assert!((world.players.get(&caster_addr).unwrap().hero.mana - mana_before).abs() < EPSILON);
}
