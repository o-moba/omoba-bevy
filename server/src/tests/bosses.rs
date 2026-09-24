use crate::balance::BOTTOM_BOSS_SPAWN_DELAY;
use crate::balance::BOTTOM_BOSS_BUFF_DAMAGE_MULT;
use crate::balance::TOP_BOSS_BUFF_HP_REGEN_PER_SECOND;
use crate::balance::MUTATIO_KILL_XP;
use shared::wire::ServerPacket;
use crate::entities::TeamBuffs;
use shared::wire::NeutralAiState;
use shared::wire::TeamBuffState;
use crate::game_world::TickCtx;
use shared::wire::TeamBuffKind;
use crate::balance::BOTTOM_BOSS_BUFF_DURATION;
use crate::sim::regenerate_team_buff_hp;
use std::time::Instant;
use crate::world::build_structures;
use shared::wire::TargetKind;
use crate::balance::WENDIGO_MAX_HP;
use crate::balance::WENDIGO_ATTACK_RANGE;
use crate::balance::WENDIGO_ATTACK_DAMAGE;
use crate::balance::TOP_BOSS_BUFF_DURATION;
use std::net::SocketAddr;
use shared::scaled_cooldown;
use crate::balance::NEUTRAL_RESPAWN_COOLDOWN;
use crate::balance::WENDIGO_KILL_GOLD;
use crate::balance::WENDIGO_KILL_XP;
use crate::balance::MUTATIO_ATTACK_RANGE;
use shared::wire::NeutralCampType;
use shared::shop::STARTING_GOLD;
use shared::wire::GameState;
use std::time::Duration;
use shared::SkillSlot;
use crate::balance::BOSS_LEASH_DISTANCE;
use crate::neutrals::jungle_camp_blueprints;
use crate::balance::TOP_BOSS_SPAWN_DELAY;
use std::collections::HashMap;
use crate::neutrals::schedule_boss_spawns;
use crate::neutrals::build_boss_neutrals;
use crate::sim::neutrals::apply_neutral_damage;
use crate::sim::neutrals::simulate_neutrals;
use crate::balance::MUTATIO_ATTACK_DAMAGE;
use crate::balance::TOP_BOSS_BUFF_DAMAGE_MULT;
use crate::balance::NEUTRAL_LEASH_DISTANCE;
use crate::neutrals::boss_blueprints;
use crate::neutrals::neutral_template;
use crate::neutrals::build_neutral_camps;
use shared::wire::TargetId;
use crate::balance::MUTATIO_MAX_HP;
use crate::balance::BOSS_RESPAWN_COOLDOWN;
use crate::entities::Neutral;
use shared::HeroClass;
use shared::map::Team;
use shared::ability_for_class_slot;
use crate::game_world::GameWorld;
use crate::balance::MUTATIO_KILL_GOLD;
use super::*;

// --- TASK-19 raid bosses -------------------------------------------------

/// Camp types visible under the snapshot filter (alive and not respawn-gated).
fn visible_camp_types(neutrals: &HashMap<u64, Neutral>) -> Vec<NeutralCampType> {
    neutrals
        .values()
        .filter(|neutral| neutral.dead_until.is_none() && neutral.state.hp > 0.0)
        .map(|neutral| neutral.state.camp_type)
        .collect()
}

fn build_camps_and_bosses() -> HashMap<u64, Neutral> {
    let mut next_neutral_id = 9_001;
    let mut neutrals = build_neutral_camps(&mut next_neutral_id);
    neutrals.extend(build_boss_neutrals(&mut next_neutral_id));
    neutrals
}

fn find_boss_id(neutrals: &HashMap<u64, Neutral>, camp_type: NeutralCampType) -> u64 {
    neutrals
        .values()
        .find(|neutral| neutral.state.camp_type == camp_type)
        .map(|neutral| neutral.state.id)
        .expect("boss neutral must exist")
}

#[test]
fn boss_templates_use_boss_constants_and_outclass_camps() {
    let wendigo = neutral_template(NeutralCampType::WendigoBoss);
    assert!((wendigo.max_hp - WENDIGO_MAX_HP).abs() < EPSILON);
    assert!((wendigo.attack_damage - WENDIGO_ATTACK_DAMAGE).abs() < EPSILON);
    assert!((wendigo.attack_range - WENDIGO_ATTACK_RANGE).abs() < EPSILON);
    assert_eq!(wendigo.kill_gold, WENDIGO_KILL_GOLD);
    assert_eq!(wendigo.kill_xp, WENDIGO_KILL_XP);

    let mutatio = neutral_template(NeutralCampType::KingMutatioBoss);
    assert!((mutatio.max_hp - MUTATIO_MAX_HP).abs() < EPSILON);
    assert!((mutatio.attack_damage - MUTATIO_ATTACK_DAMAGE).abs() < EPSILON);
    assert!((mutatio.attack_range - MUTATIO_ATTACK_RANGE).abs() < EPSILON);
    assert_eq!(mutatio.kill_gold, MUTATIO_KILL_GOLD);
    assert_eq!(mutatio.kill_xp, MUTATIO_KILL_XP);

    for camp in [
        NeutralCampType::Skirmisher,
        NeutralCampType::Bruiser,
        NeutralCampType::Spitter,
    ] {
        let template = neutral_template(camp);
        assert!(wendigo.max_hp > template.max_hp);
        assert!(mutatio.max_hp > template.max_hp);
        assert!(wendigo.attack_damage > template.attack_damage);
        assert!(!camp.is_boss());
    }
    assert!(NeutralCampType::WendigoBoss.is_boss());
    assert!(NeutralCampType::KingMutatioBoss.is_boss());
}

#[test]
fn boss_pits_are_point_symmetric_and_clear_of_camps() {
    let bosses = boss_blueprints();
    assert_eq!(bosses.len(), 2);
    let (wendigo_anchor, wendigo_type) = bosses[0];
    let (mutatio_anchor, mutatio_type) = bosses[1];
    assert_eq!(wendigo_type, NeutralCampType::WendigoBoss);
    assert_eq!(mutatio_type, NeutralCampType::KingMutatioBoss);

    // Bottom boss sits in negative-z (bottom-lane) territory, top boss in
    // positive-z; the pits are 180-degree rotationally symmetric.
    assert!(wendigo_anchor.z < 0.0 && mutatio_anchor.z > 0.0);
    assert!((wendigo_anchor.x + mutatio_anchor.x).abs() < EPSILON);
    assert!((wendigo_anchor.z + mutatio_anchor.z).abs() < EPSILON);

    // Pits stay clear of every jungle camp slot.
    for (camp_anchor, _) in jungle_camp_blueprints() {
        for (boss_anchor, _) in &bosses {
            assert!(
                camp_anchor.distance(*boss_anchor) > 5.0,
                "boss pit overlaps a camp"
            );
        }
    }
}

#[test]
fn bosses_are_gated_before_spawn_delays_and_spawn_with_full_stats() {
    let mut world = GameWorld::empty();
    world.neutrals = build_camps_and_bosses();
    let now = Instant::now();

    // Before match start (Lobby): bosses dormant, only the six camps visible.
    let visible = visible_camp_types(&world.neutrals);
    assert_eq!(visible.len(), 6);
    assert!(visible.iter().all(|camp_type| !camp_type.is_boss()));

    schedule_boss_spawns(&mut world.neutrals, now);

    // Just before the bottom-boss delay: still no boss.
    let before_bottom = now + BOTTOM_BOSS_SPAWN_DELAY - Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: before_bottom,
            dt: 0.1,
        },
    );
    assert!(
        visible_camp_types(&world.neutrals)
            .iter()
            .all(|camp_type| !camp_type.is_boss())
    );

    // At/after the bottom delay: Wendigo up at its pit with full boss HP,
    // top boss still gated.
    let after_bottom = now + BOTTOM_BOSS_SPAWN_DELAY + Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: after_bottom,
            dt: 0.1,
        },
    );
    let visible = visible_camp_types(&world.neutrals);
    assert!(visible.contains(&NeutralCampType::WendigoBoss));
    assert!(!visible.contains(&NeutralCampType::KingMutatioBoss));
    let wendigo = world
        .neutrals
        .values()
        .find(|neutral| neutral.state.camp_type == NeutralCampType::WendigoBoss)
        .unwrap();
    assert!((wendigo.state.hp - WENDIGO_MAX_HP).abs() < EPSILON);
    assert!((wendigo.state.x - wendigo.anchor.x).abs() < EPSILON);
    assert!((wendigo.state.z - wendigo.anchor.z).abs() < EPSILON);

    // At/after the top delay: King Mutatio up as well.
    let after_top = now + TOP_BOSS_SPAWN_DELAY + Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: after_top,
            dt: 0.1,
        },
    );
    let visible = visible_camp_types(&world.neutrals);
    assert!(visible.contains(&NeutralCampType::KingMutatioBoss));
    let mutatio = world
        .neutrals
        .values()
        .find(|neutral| neutral.state.camp_type == NeutralCampType::KingMutatioBoss)
        .unwrap();
    assert!((mutatio.state.hp - MUTATIO_MAX_HP).abs() < EPSILON);
    assert!((mutatio.state.x - mutatio.anchor.x).abs() < EPSILON);
    assert!((mutatio.state.z - mutatio.anchor.z).abs() < EPSILON);

    // Camps were never gated by the boss schedule.
    let camp_count = visible
        .iter()
        .filter(|camp_type| !camp_type.is_boss())
        .count();
    assert_eq!(camp_count, 6);
}

#[test]
fn boss_kill_grants_team_buff_and_respawns_on_boss_cooldown() {
    let mut world = GameWorld::empty();
    let killer_addr: SocketAddr = "127.0.0.1:47001".parse().unwrap();
    let enemy_addr: SocketAddr = "127.0.0.1:47002".parse().unwrap();
    let now = Instant::now();
    world.ensure_connected(killer_addr, now);
    world.ensure_connected(enemy_addr, now);
    world
        .players
        .get_mut(&killer_addr)
        .unwrap()
        .hero
        .identity
        .team = Team::Green;
    world
        .players
        .get_mut(&enemy_addr)
        .unwrap()
        .hero
        .identity
        .team = Team::Blue;
    let killer_id = world.players.get(&killer_addr).unwrap().hero.identity.id;

    world.neutrals = build_camps_and_bosses();
    schedule_boss_spawns(&mut world.neutrals, now);
    let spawn_at = now + BOTTOM_BOSS_SPAWN_DELAY + Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: spawn_at,
            dt: 0.1,
        },
    );

    let wendigo_id = find_boss_id(&world.neutrals, NeutralCampType::WendigoBoss);
    let mut team_buffs = TeamBuffs::default();
    let kill_at = spawn_at + Duration::from_secs(5);
    apply_neutral_damage(
        &mut world.players,
        &mut world.neutrals,
        &mut team_buffs,
        wendigo_id,
        WENDIGO_MAX_HP + 1.0,
        killer_id,
        kill_at,
    );

    // Killer got the individual reward; the whole killing team got the buff.
    let killer = world.players.get(&killer_addr).unwrap();
    assert_eq!(killer.economy.gold, STARTING_GOLD + WENDIGO_KILL_GOLD);
    assert!(team_buffs.is_active(Team::Green, TeamBuffKind::WendigoFavor, kill_at));
    assert!(!team_buffs.is_active(Team::Blue, TeamBuffKind::WendigoFavor, kill_at));
    assert!(
        (team_buffs.damage_multiplier(Team::Green, kill_at) - BOTTOM_BOSS_BUFF_DAMAGE_MULT).abs()
            < EPSILON
    );
    assert!((team_buffs.damage_multiplier(Team::Blue, kill_at) - 1.0).abs() < EPSILON);

    // The boss stays down through the camp cooldown (40s) and until its own
    // 180s cooldown elapses, then respawns at the pit at full HP.
    let after_camp_cooldown = kill_at + NEUTRAL_RESPAWN_COOLDOWN + Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: after_camp_cooldown,
            dt: 0.1,
        },
    );
    assert!(
        !visible_camp_types(&world.neutrals).contains(&NeutralCampType::WendigoBoss),
        "boss must not reuse the camp respawn cooldown"
    );

    let before_boss_respawn = kill_at + BOSS_RESPAWN_COOLDOWN - Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: before_boss_respawn,
            dt: 0.1,
        },
    );
    assert!(!visible_camp_types(&world.neutrals).contains(&NeutralCampType::WendigoBoss));

    let after_boss_respawn = kill_at + BOSS_RESPAWN_COOLDOWN + Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: after_boss_respawn,
            dt: 0.1,
        },
    );
    let wendigo = world.neutrals.get(&wendigo_id).unwrap();
    assert!(wendigo.dead_until.is_none());
    assert!((wendigo.state.hp - WENDIGO_MAX_HP).abs() < EPSILON);
    assert!((wendigo.state.x - wendigo.anchor.x).abs() < EPSILON);
    assert!((wendigo.state.z - wendigo.anchor.z).abs() < EPSILON);
}

#[test]
fn camp_kills_do_not_grant_team_buffs() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:47003".parse().unwrap();
    let now = Instant::now();
    world.ensure_connected(addr, now);
    let killer_id = world.players.get(&addr).unwrap().hero.identity.id;

    let mut next_neutral_id = 9_001;
    world.neutrals = build_neutral_camps(&mut next_neutral_id);
    let camp_id = *world.neutrals.keys().next().unwrap();
    let mut team_buffs = TeamBuffs::default();
    apply_neutral_damage(
        &mut world.players,
        &mut world.neutrals,
        &mut team_buffs,
        camp_id,
        10_000.0,
        killer_id,
        now,
    );
    for team in [Team::Green, Team::Blue] {
        assert!((team_buffs.damage_multiplier(team, now) - 1.0).abs() < EPSILON);
        assert!(team_buffs.hp_regen_per_second(team, now) == 0.0);
    }
    assert!(team_buffs.snapshot(now).is_empty());
}

#[test]
fn team_buffs_expire_refresh_and_stack_multiplicatively() {
    let mut buffs = TeamBuffs::default();
    let now = Instant::now();
    buffs.grant(Team::Green, TeamBuffKind::WendigoFavor, now);

    let almost_expired = now + BOTTOM_BOSS_BUFF_DURATION - Duration::from_millis(1);
    assert!(buffs.is_active(Team::Green, TeamBuffKind::WendigoFavor, almost_expired));
    let expired = now + BOTTOM_BOSS_BUFF_DURATION;
    assert!(!buffs.is_active(Team::Green, TeamBuffKind::WendigoFavor, expired));
    assert!((buffs.damage_multiplier(Team::Green, expired) - 1.0).abs() < EPSILON);

    // Re-kill refreshes the expiry from the new kill instant.
    let rekill_at = now + Duration::from_secs(30);
    buffs.grant(Team::Green, TeamBuffKind::WendigoFavor, rekill_at);
    assert!(buffs.is_active(
        Team::Green,
        TeamBuffKind::WendigoFavor,
        rekill_at + BOTTOM_BOSS_BUFF_DURATION - Duration::from_millis(1)
    ));
    assert!(!buffs.is_active(
        Team::Green,
        TeamBuffKind::WendigoFavor,
        rekill_at + BOTTOM_BOSS_BUFF_DURATION
    ));

    // Both buffs active for one team combine multiplicatively.
    buffs.grant(Team::Green, TeamBuffKind::MutatioMight, rekill_at);
    let both_active = rekill_at + Duration::from_secs(1);
    let expected = BOTTOM_BOSS_BUFF_DAMAGE_MULT * TOP_BOSS_BUFF_DAMAGE_MULT;
    assert!((buffs.damage_multiplier(Team::Green, both_active) - expected).abs() < EPSILON);
    assert!(
        (buffs.hp_regen_per_second(Team::Green, both_active) - TOP_BOSS_BUFF_HP_REGEN_PER_SECOND)
            .abs()
            < EPSILON
    );
    // Enemy team remains unaffected throughout.
    assert!((buffs.damage_multiplier(Team::Blue, both_active) - 1.0).abs() < EPSILON);
    assert!(buffs.hp_regen_per_second(Team::Blue, both_active) == 0.0);

    // Snapshot carries both active entries with sane remaining times.
    let snapshot = buffs.snapshot(both_active);
    assert_eq!(snapshot.len(), 2);
    for entry in &snapshot {
        assert_eq!(entry.team, Team::Green);
        assert!(entry.remaining_secs > 0.0);
        assert!(entry.remaining_secs <= TOP_BOSS_BUFF_DURATION.as_secs_f32());
    }

    buffs.clear();
    assert!(buffs.snapshot(both_active).is_empty());
}

#[test]
fn team_buff_multiplies_cast_damage_for_buffed_team_only() {
    let mut world = GameWorld::empty();
    let now = Instant::now();
    let caster_addr: SocketAddr = "127.0.0.1:47101".parse().unwrap();
    let target_addr: SocketAddr = "127.0.0.1:47102".parse().unwrap();
    let q = ability_for_class_slot(HeroClass::Warrior, SkillSlot::Q);
    let target_id = setup_caster_and_target(
        &mut world,
        caster_addr,
        target_addr,
        HeroClass::Warrior,
        q.cast_range * 0.5,
        now,
    );
    let caster_id = world.players.get(&caster_addr).unwrap().hero.identity.id;
    let base_damage = q.projectile_damage.unwrap();

    // Buff the caster's team (Green): outgoing damage is multiplied.
    world
        .team_buffs
        .grant(Team::Green, TeamBuffKind::WendigoFavor, now);
    cast_slot(
        &mut world,
        caster_addr,
        TargetId {
            kind: TargetKind::Player,
            id: target_id,
        },
        0,
        now,
    );
    let buffed = world
        .projectiles
        .values()
        .next()
        .expect("buffed cast fires");
    assert!(
        (buffed.damage - base_damage * BOTTOM_BOSS_BUFF_DAMAGE_MULT).abs() < EPSILON,
        "buffed team damage must include the boss multiplier"
    );

    // The enemy (Blue) caster gets no multiplier from Green's buff.
    world.projectiles.clear();
    cast_slot(
        &mut world,
        target_addr,
        TargetId {
            kind: TargetKind::Player,
            id: caster_id,
        },
        0,
        now,
    );
    let unbuffed = world.projectiles.values().next().expect("enemy cast fires");
    assert!((unbuffed.damage - base_damage).abs() < EPSILON);

    // Both buffs active: multiplicative stacking on the buffed team.
    world
        .team_buffs
        .grant(Team::Green, TeamBuffKind::MutatioMight, now);
    world.projectiles.clear();
    let later = now + scaled_cooldown(q, 1) + Duration::from_millis(1);
    cast_slot(
        &mut world,
        caster_addr,
        TargetId {
            kind: TargetKind::Player,
            id: target_id,
        },
        0,
        later,
    );
    let double_buffed = world
        .projectiles
        .values()
        .next()
        .expect("double-buffed cast fires");
    let expected = base_damage * BOTTOM_BOSS_BUFF_DAMAGE_MULT * TOP_BOSS_BUFF_DAMAGE_MULT;
    assert!((double_buffed.damage - expected).abs() < EPSILON);

    // After expiry the multiplier is gone.
    world.projectiles.clear();
    let after_expiry = now + TOP_BOSS_BUFF_DURATION + scaled_cooldown(q, 1);
    cast_slot(
        &mut world,
        caster_addr,
        TargetId {
            kind: TargetKind::Player,
            id: target_id,
        },
        0,
        after_expiry,
    );
    let expired = world
        .projectiles
        .values()
        .next()
        .expect("post-expiry cast fires");
    assert!((expired.damage - base_damage).abs() < EPSILON);
}

#[test]
fn top_buff_regenerates_hp_for_alive_buffed_players_only() {
    let mut world = GameWorld::empty();
    let green_addr: SocketAddr = "127.0.0.1:47201".parse().unwrap();
    let blue_addr: SocketAddr = "127.0.0.1:47202".parse().unwrap();
    let dead_addr: SocketAddr = "127.0.0.1:47203".parse().unwrap();
    let now = Instant::now();
    for addr in [green_addr, blue_addr, dead_addr] {
        world.ensure_connected(addr, now);
        // Buff regen only applies to joined players.
        world.players.get_mut(&addr).unwrap().joined = true;
    }
    world
        .players
        .get_mut(&green_addr)
        .unwrap()
        .hero
        .identity
        .team = Team::Green;
    world
        .players
        .get_mut(&blue_addr)
        .unwrap()
        .hero
        .identity
        .team = Team::Blue;
    world
        .players
        .get_mut(&dead_addr)
        .unwrap()
        .hero
        .identity
        .team = Team::Green;
    world.players.get_mut(&green_addr).unwrap().hero.hp = 50.0;
    world.players.get_mut(&blue_addr).unwrap().hero.hp = 50.0;
    world.players.get_mut(&dead_addr).unwrap().hero.hp = 0.0;

    world
        .team_buffs
        .grant(Team::Green, TeamBuffKind::MutatioMight, now);

    regenerate_team_buff_hp(&mut world, TickCtx { now, dt: 1.0 });
    let expected = 50.0 + TOP_BOSS_BUFF_HP_REGEN_PER_SECOND;
    assert!((world.players.get(&green_addr).unwrap().hero.hp - expected).abs() < EPSILON);
    assert!((world.players.get(&blue_addr).unwrap().hero.hp - 50.0).abs() < EPSILON);
    assert!(world.players.get(&dead_addr).unwrap().hero.hp == 0.0);

    // Regen clamps to max HP.
    regenerate_team_buff_hp(&mut world, TickCtx { now, dt: 10_000.0 });
    let green = world.players.get(&green_addr).unwrap();
    assert!((green.hero.hp - green.hero.max_hp).abs() < EPSILON);

    // No regen after the buff expires.
    world.players.get_mut(&green_addr).unwrap().hero.hp = 50.0;
    let expired = now + TOP_BOSS_BUFF_DURATION;
    regenerate_team_buff_hp(
        &mut world,
        TickCtx {
            now: expired,
            dt: 1.0,
        },
    );
    assert!((world.players.get(&green_addr).unwrap().hero.hp - 50.0).abs() < EPSILON);
}

#[test]
fn reset_match_clears_buffs_and_restarts_boss_schedule() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:47301".parse().unwrap();
    let now = Instant::now();
    world.ensure_connected(addr, now);

    world.structures = build_structures(&world.map_layout);
    world.neutrals = build_camps_and_bosses();
    schedule_boss_spawns(&mut world.neutrals, now);
    // Bring both bosses up, then grant a buff as if one was killed.
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: now + TOP_BOSS_SPAWN_DELAY + Duration::from_millis(1),
            dt: 0.1,
        },
    );
    world
        .team_buffs
        .grant(Team::Green, TeamBuffKind::WendigoFavor, now);

    world.game_state = GameState::Victory {
        winner: Team::Green,
    };
    world.reset_round(now);

    assert!(matches!(world.game_state, GameState::Lobby));
    // Buffs cleared for both teams.
    let later = now + Duration::from_secs(1);
    for team in [Team::Green, Team::Blue] {
        assert!((world.team_buffs.damage_multiplier(team, later) - 1.0).abs() < EPSILON);
    }
    // Bosses dormant until the fresh Running transition; ordinary camps restored.
    for neutral in world.neutrals.values() {
        if neutral.state.camp_type.is_boss() {
            assert!(neutral.dead_until.is_none());
            assert!(neutral.state.hp <= 0.0);
        } else {
            assert!(neutral.dead_until.is_none());
            assert!(neutral.state.hp > 0.0);
        }
    }
}

#[test]
fn boss_and_buff_wire_formats_are_snake_case_and_additive() {
    assert_eq!(
        serde_json::to_string(&NeutralCampType::WendigoBoss).unwrap(),
        "\"wendigo_boss\""
    );
    assert_eq!(
        serde_json::to_string(&NeutralCampType::KingMutatioBoss).unwrap(),
        "\"king_mutatio_boss\""
    );
    let entry = TeamBuffState {
        team: Team::Green,
        kind: TeamBuffKind::MutatioMight,
        remaining_secs: 12.5,
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert!(json.contains("\"mutatio_might\""));
    assert!(json.contains("\"green\""));

    // The snapshot stays decodable without the new field (serde default).
    let legacy = r#"{"type":"snapshot","your_id":1,"players":[],"projectiles":[],"structures":[],"minions":[],"game_state":{"type":"lobby"}}"#;
    let packet: ServerPacket = serde_json::from_str(legacy).expect("legacy snapshot decodes");
    let ServerPacket::Snapshot { team_buffs, .. } = packet else {
        panic!("expected snapshot")
    };
    assert!(team_buffs.is_empty());
}

#[test]
fn boss_leash_reset_uses_boss_distance_and_restores_full_hp() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:47401".parse().unwrap();
    let now = Instant::now();
    world.ensure_connected(addr, now);
    world.players.get_mut(&addr).unwrap().joined = true;
    let player_id = world.players.get(&addr).unwrap().hero.identity.id;

    world.neutrals = build_camps_and_bosses();
    schedule_boss_spawns(&mut world.neutrals, now);
    let spawn_at = now + BOTTOM_BOSS_SPAWN_DELAY + Duration::from_millis(1);
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: spawn_at,
            dt: 0.1,
        },
    );
    let wendigo_id = find_boss_id(&world.neutrals, NeutralCampType::WendigoBoss);
    let anchor = world.neutrals.get(&wendigo_id).unwrap().anchor;

    // Aggro the boss, then keep the target INSIDE the camp leash distance
    // but OUTSIDE the (larger) boss leash: the boss must keep chasing.
    {
        let boss = world.neutrals.get_mut(&wendigo_id).unwrap();
        boss.state.hp -= 50.0;
        boss.state.ai_state = NeutralAiState::Aggro;
        boss.target_player_id = Some(player_id);
    }
    {
        let player = world.players.get_mut(&addr).unwrap();
        player.hero.x = anchor.x + NEUTRAL_LEASH_DISTANCE + 2.0;
        player.hero.z = anchor.z;
    }
    const {
        assert!(NEUTRAL_LEASH_DISTANCE + 2.0 < BOSS_LEASH_DISTANCE);
    }
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: spawn_at + Duration::from_millis(100),
            dt: 0.1,
        },
    );
    {
        let boss = world.neutrals.get(&wendigo_id).unwrap();
        assert_eq!(boss.state.ai_state, NeutralAiState::Aggro);
        assert!(boss.state.hp < WENDIGO_MAX_HP, "no reset inside boss leash");
    }

    // Past the boss leash the boss resets to its pit at full HP.
    {
        let player = world.players.get_mut(&addr).unwrap();
        player.hero.x = anchor.x + BOSS_LEASH_DISTANCE + 2.0;
    }
    simulate_neutrals(
        &mut world,
        TickCtx {
            now: spawn_at + Duration::from_millis(200),
            dt: 0.1,
        },
    );
    let boss = world.neutrals.get(&wendigo_id).unwrap();
    assert_eq!(boss.state.ai_state, NeutralAiState::Idle);
    assert!((boss.state.hp - WENDIGO_MAX_HP).abs() < EPSILON);
    assert!((boss.state.x - anchor.x).abs() < EPSILON);
    assert!((boss.state.z - anchor.z).abs() < EPSILON);
    assert!(boss.target_player_id.is_none());
}
