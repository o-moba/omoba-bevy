use crate::game_world::GameWorld;
use crate::session::handle_respawns;
use crate::balance::LEVEL_UP_HP_BONUS;
use std::time::Instant;
use crate::world::build_structures;
use crate::balance::STARTING_LEVEL;
use std::net::SocketAddr;
use crate::balance::MAX_HP;
use std::time::Duration;
use crate::progression::grant_player_xp;
use crate::balance::MAX_MANA;
use crate::progression::xp_threshold_for_level;
use crate::balance::LEVEL_UP_MANA_BONUS;
use super::*;

#[test]
fn progression_levels_up_and_scales_stats() {
    let mut world = GameWorld::empty();
    let addr: SocketAddr = "127.0.0.1:34568".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    let player = world.players.get_mut(&addr).unwrap();
    let first_threshold = player.hero.progress.next_level_xp;
    let second_threshold = xp_threshold_for_level(STARTING_LEVEL + 1);

    grant_player_xp(&mut player.hero, first_threshold + second_threshold + 17);

    assert_eq!(player.hero.progress.level, STARTING_LEVEL + 2);
    assert_eq!(player.hero.progress.skill_points, 2);
    assert_eq!(player.hero.progress.xp, 17);
    assert_eq!(
        player.hero.progress.next_level_xp,
        xp_threshold_for_level(STARTING_LEVEL + 2)
    );
    assert!((player.hero.max_hp - (MAX_HP + LEVEL_UP_HP_BONUS * 2.0)).abs() < EPSILON);
    assert!((player.hero.max_mana - (MAX_MANA + LEVEL_UP_MANA_BONUS * 2.0)).abs() < EPSILON);
}

#[test]
fn respawn_restores_scaled_maximums() {
    let mut world = GameWorld::empty();
    world.structures = build_structures(&world.map_layout);
    let addr: SocketAddr = "127.0.0.1:34569".parse().unwrap();
    let now = Instant::now();

    world.ensure_connected(addr, now);
    let player = world.players.get_mut(&addr).unwrap();
    let first_threshold = player.hero.progress.next_level_xp;
    let second_threshold = xp_threshold_for_level(STARTING_LEVEL + 1);
    grant_player_xp(&mut player.hero, first_threshold + second_threshold);
    player.hero.hp = 0.0;
    player.hero.mana = 0.0;
    player.timers.respawn_at = Some(now - Duration::from_millis(1));

    handle_respawns(&mut world, now);

    let player = world.players.get(&addr).unwrap();
    assert_eq!(player.hero.progress.level, STARTING_LEVEL + 2);
    assert!(player.hero.max_hp > MAX_HP);
    assert!(player.hero.max_mana > MAX_MANA);
    assert!((player.hero.hp - player.hero.max_hp).abs() < EPSILON);
    assert!((player.hero.mana - player.hero.max_mana).abs() < EPSILON);
}
