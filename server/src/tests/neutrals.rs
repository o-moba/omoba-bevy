use shared::HeroClass;
use shared::map::Team;
use crate::game_world::GameWorld;
use crate::sim::towers::simulate_tower_attacks;
use crate::entities::TeamBuffs;
use crate::balance::SKIRMISHER_ATTACK_RANGE;
use shared::wire::NeutralAiState;
use crate::balance::SKIRMISHER_KILL_XP;
use crate::balance::SKIRMISHER_ATTACK_DAMAGE;
use crate::game_world::TickCtx;
use crate::balance::BRUISER_KILL_GOLD;
use std::time::Instant;
use crate::world::build_structures;
use shared::wire::StructureKind;
use shared::wire::CharacterChoice;
use crate::balance::BRUISER_MAX_HP;
use crate::world::spawn_minion_waves_if_due;
use std::net::SocketAddr;
use crate::session::handle_join_request;
use crate::balance::NEUTRAL_RESPAWN_COOLDOWN;
use crate::balance::SKIRMISHER_MAX_HP;
use crate::balance::SPITTER_ATTACK_RANGE;
use shared::shop::STARTING_GOLD;
use shared::wire::GameState;
use std::time::Duration;
use crate::balance::MINION_WAVE_INTERVAL;
use crate::sim::minions::simulate_minions;
use crate::balance::SPITTER_KILL_XP;
use crate::sim::neutrals::apply_neutral_damage;
use crate::sim::neutrals::simulate_neutrals;
use crate::balance::TOWER_COOLDOWN;
use crate::balance::NEUTRAL_LEASH_DISTANCE;
use crate::neutrals::neutral_template;
use crate::neutrals::build_neutral_camps;
use shared::wire::NeutralCampType;
use crate::balance::SKIRMISHER_KILL_GOLD;
use super::*;

#[test]
fn neutral_template_matches_balance_constants() {
    let sk = neutral_template(NeutralCampType::Skirmisher);
    assert!((sk.max_hp - SKIRMISHER_MAX_HP).abs() < EPSILON);
    assert!((sk.attack_damage - SKIRMISHER_ATTACK_DAMAGE).abs() < EPSILON);
    assert!((sk.attack_range - SKIRMISHER_ATTACK_RANGE).abs() < EPSILON);
    assert_eq!(sk.kill_gold, SKIRMISHER_KILL_GOLD);
    assert_eq!(sk.kill_xp, SKIRMISHER_KILL_XP);

    let br = neutral_template(NeutralCampType::Bruiser);
    assert!((br.max_hp - BRUISER_MAX_HP).abs() < EPSILON);
    assert_eq!(br.kill_gold, BRUISER_KILL_GOLD);

    let sp = neutral_template(NeutralCampType::Spitter);
    assert!((sp.attack_range - SPITTER_ATTACK_RANGE).abs() < EPSILON);
    assert_eq!(sp.kill_xp, SPITTER_KILL_XP);
}

#[test]
fn neutral_camps_spawn_alive_with_distinct_templates() {
    let mut next_neutral_id = 9_001;
    let neutrals = build_neutral_camps(&mut next_neutral_id);

    assert_eq!(neutrals.len(), 6);

    let mut camp_types = Vec::new();
    for neutral in neutrals.values() {
        let template = neutral_template(neutral.state.camp_type);
        camp_types.push(neutral.state.camp_type);
        assert!((neutral.state.x - neutral.anchor.x).abs() < EPSILON);
        assert!((neutral.state.y - neutral.anchor.y).abs() < EPSILON);
        assert!((neutral.state.z - neutral.anchor.z).abs() < EPSILON);
        assert!((neutral.state.hp - template.max_hp).abs() < EPSILON);
        assert!((neutral.state.max_hp - template.max_hp).abs() < EPSILON);
        assert_eq!(neutral.state.ai_state, NeutralAiState::Idle);
        assert!(neutral.dead_until.is_none());
    }
    for kind in [
        NeutralCampType::Skirmisher,
        NeutralCampType::Bruiser,
        NeutralCampType::Spitter,
    ] {
        assert_eq!(camp_types.iter().filter(|&&value| value == kind).count(), 2);
    }
}

#[test]
fn neutral_kills_award_rewards_and_respawn_on_cooldown() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:45678".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    let killer_id = world.players.get(&addr).unwrap().hero.identity.id;

    let mut next_neutral_id = 9_001;
    world.neutrals = build_neutral_camps(&mut next_neutral_id);
    let neutral_id = *world.neutrals.keys().next().unwrap();
    let camp_type = world.neutrals.get(&neutral_id).unwrap().state.camp_type;
    let template = neutral_template(camp_type);
    let kill_damage = world.neutrals.get(&neutral_id).unwrap().state.hp + 1.0;

    let mut team_buffs = TeamBuffs::default();
    apply_neutral_damage(
        &mut world.players,
        &mut world.neutrals,
        &mut team_buffs,
        neutral_id,
        kill_damage,
        killer_id,
        now,
    );

    let killer = world.players.get(&addr).unwrap();
    assert_eq!(killer.economy.gold, STARTING_GOLD + template.kill_gold);
    assert_eq!(killer.hero.progress.xp, template.kill_xp);

    let neutral = world.neutrals.get(&neutral_id).unwrap();
    assert_eq!(neutral.state.hp, 0.0);
    assert_eq!(neutral.state.ai_state, NeutralAiState::Idle);
    assert!(neutral.dead_until.is_some());
    assert!(neutral.target_player_id.is_none());

    simulate_neutrals(
        &mut world,
        TickCtx {
            now: now + NEUTRAL_RESPAWN_COOLDOWN + Duration::from_millis(1),
            dt: 0.1,
        },
    );

    let respawned = world.neutrals.get(&neutral_id).unwrap();
    assert!(respawned.dead_until.is_none());
    assert!((respawned.state.hp - template.max_hp).abs() < EPSILON);
    assert!((respawned.state.x - respawned.anchor.x).abs() < EPSILON);
    assert!((respawned.state.z - respawned.anchor.z).abs() < EPSILON);
    assert_eq!(respawned.state.ai_state, NeutralAiState::Idle);
}

#[test]
fn neutral_leash_reset_restores_anchor_and_full_hp() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:56789".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    let player_id = world.players.get(&addr).unwrap().hero.identity.id;

    let mut next_neutral_id = 9_001;
    world.neutrals = build_neutral_camps(&mut next_neutral_id);
    let neutral_id = *world.neutrals.keys().next().unwrap();
    let anchor = world.neutrals.get(&neutral_id).unwrap().anchor;

    {
        let neutral = world.neutrals.get_mut(&neutral_id).unwrap();
        neutral.state.hp -= 15.0;
        neutral.state.x = anchor.x + 2.0;
        neutral.state.z = anchor.z + 1.5;
        neutral.state.ai_state = NeutralAiState::Aggro;
        neutral.target_player_id = Some(player_id);
    }

    {
        let player = world.players.get_mut(&addr).unwrap();
        player.hero.x = anchor.x + NEUTRAL_LEASH_DISTANCE + 2.0;
        player.hero.z = anchor.z;
    }

    simulate_neutrals(&mut world, TickCtx { now, dt: 0.1 });

    let reset = world.neutrals.get(&neutral_id).unwrap();
    assert!((reset.state.x - anchor.x).abs() < EPSILON);
    assert!((reset.state.y - anchor.y).abs() < EPSILON);
    assert!((reset.state.z - anchor.z).abs() < EPSILON);
    assert!((reset.state.hp - reset.state.max_hp).abs() < EPSILON);
    assert_eq!(reset.state.ai_state, NeutralAiState::Idle);
    assert!(reset.target_player_id.is_none());
}

#[test]
fn neutrals_do_not_break_minion_waves_or_tower_attacks() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:60000".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    handle_join_request(
        world.players.get_mut(&addr).unwrap(),
        Team::Green,
        CharacterChoice::Ipfs,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );

    let mut next_neutral_id = 9_001;
    world.neutrals = build_neutral_camps(&mut next_neutral_id);
    let focus_neutral_id = *world.neutrals.keys().next().unwrap();
    let focus_anchor = world.neutrals.get(&focus_neutral_id).unwrap().anchor;
    {
        let player = world.players.get_mut(&addr).unwrap();
        player.hero.x = focus_anchor.x;
        player.hero.z = focus_anchor.z;
    }
    simulate_neutrals(&mut world, TickCtx { now, dt: 0.1 });
    assert_eq!(
        world
            .neutrals
            .get(&focus_neutral_id)
            .unwrap()
            .state
            .ai_state,
        NeutralAiState::Aggro
    );

    world.last_wave_spawn_at = now - MINION_WAVE_INTERVAL;
    spawn_minion_waves_if_due(&mut world, now);
    assert!(!world.minions.is_empty());

    let moving_minion_id = *world.minions.keys().next().unwrap();
    let before_move = {
        let minion = world.minions.get(&moving_minion_id).unwrap();
        (minion.state.x, minion.state.z)
    };

    world.structures = build_structures(&world.map_layout);
    let green_tower = world
        .structures
        .values()
        .find(|structure| {
            structure.state.team == Team::Green && structure.state.kind == StructureKind::Tower
        })
        .unwrap()
        .state
        .clone();

    let enemy_minion_id = world
        .minions
        .values()
        .find(|minion| minion.state.team == Team::Blue)
        .unwrap()
        .state
        .id;
    {
        let enemy_minion = world.minions.get_mut(&enemy_minion_id).unwrap();
        enemy_minion.state.x = green_tower.x + 1.0;
        enemy_minion.state.z = green_tower.z + 1.0;
    }
    let tower_target_hp = world.minions.get(&enemy_minion_id).unwrap().state.hp;

    simulate_minions(
        &mut world,
        TickCtx {
            now: now + Duration::from_millis(250),
            dt: 0.5,
        },
    );

    let after_move = {
        let minion = world.minions.get(&moving_minion_id).unwrap();
        (minion.state.x, minion.state.z)
    };
    assert!(before_move != after_move);

    simulate_tower_attacks(&mut world, now + TOWER_COOLDOWN);

    let damaged_hp = world.minions.get(&enemy_minion_id).unwrap().state.hp;
    assert!(damaged_hp < tower_target_hp);
    assert!(matches!(world.game_state, GameState::Running));
}
