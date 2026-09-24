use shared::HeroClass;
use shared::map::Team;
use crate::balance::MINION_SPAWN_HEIGHT;
use crate::game_world::GameWorld;
use crate::game_world::TickCtx;
use shared::wire::MinionTargetKind;
use std::time::Instant;
use shared::wire::CharacterChoice;
use std::collections::HashMap;
use shared::wire::MinionState;
use std::net::SocketAddr;
use shared::wire::MinionBrainState;
use crate::session::handle_join_request;
use crate::balance::MINION_ATTACK_DAMAGE;
use crate::entities::Minion;
use crate::balance::MINION_MAX_HP;
use crate::balance::MINION_VISION_RANGE;
use shared::combat::MinionKind;
use shared::map::Lane;
use crate::sim::minions::simulate_minions;
use super::*;

#[test]
fn minion_prefers_enemy_minion_over_closer_player() {
    let mut world = GameWorld::empty();
    let now = Instant::now();
    let enemy_addr: SocketAddr = "127.0.0.1:34570".parse().unwrap();
    world.ensure_connected(enemy_addr, now);
    handle_join_request(
        world.players.get_mut(&enemy_addr).unwrap(),
        Team::Blue,
        CharacterChoice::Ipfs,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );
    {
        // Enemy player right next to the acting green minion at the origin.
        let p = world.players.get_mut(&enemy_addr).unwrap();
        p.hero.x = 1.0;
        p.hero.z = 0.0;
    }

    let make_minion = |id: u64, team: Team, x: f32| Minion {
        state: MinionState {
            kind: MinionKind::Melee,
            attack_sequence: 0,
            id,
            team,
            lane: Lane::Mid,
            x,
            y: MINION_SPAWN_HEIGHT,
            z: 0.0,
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
    };

    world.minions.insert(1, make_minion(1, Team::Green, 0.0));
    // Enemy minion farther than the player but still within vision.
    world
        .minions
        .insert(2, make_minion(2, Team::Blue, MINION_VISION_RANGE * 0.5));

    simulate_minions(&mut world, TickCtx { now, dt: 0.1 });

    let green = world.minions.get(&1).unwrap();
    assert_eq!(
        green.state.target_kind,
        Some(MinionTargetKind::Minion),
        "enemy minion must take priority over the closer player"
    );
    assert_eq!(green.state.target_id, Some(2));
}

#[test]
fn opposing_minions_engage_take_damage_die_and_leave_the_next_snapshot_set() {
    let make_minion = |id: u64, team: Team, x: f32, hp: f32| Minion {
        state: MinionState {
            kind: MinionKind::Melee,
            attack_sequence: 0,
            id,
            team,
            lane: Lane::Mid,
            x,
            y: MINION_SPAWN_HEIGHT,
            z: 0.0,
            yaw: 0.0,
            hp,
            max_hp: MINION_MAX_HP,
            state: MinionBrainState::Marching,
            target_kind: None,
            target_id: None,
        },
        path: Vec::new(),
        next_waypoint: 0,
        last_attack_at: None,
        aggro_target: None,
    };

    let mut world = GameWorld::empty();
    world.minions = HashMap::from([
        (1, make_minion(1, Team::Green, 0.0, MINION_MAX_HP)),
        (2, make_minion(2, Team::Blue, 1.0, MINION_ATTACK_DAMAGE)),
    ]);

    simulate_minions(
        &mut world,
        TickCtx {
            now: Instant::now(),
            dt: 0.1,
        },
    );

    let survivor = world.minions.get(&1).expect("green minion remains live");
    assert_eq!(survivor.state.state, MinionBrainState::Attacking);
    assert_eq!(survivor.state.target_kind, Some(MinionTargetKind::Minion));
    assert_eq!(survivor.state.target_id, Some(2));
    assert_eq!(survivor.state.hp, MINION_MAX_HP - MINION_ATTACK_DAMAGE);
    let defeated = world
        .minions
        .get(&2)
        .expect("dead state is observable this tick");
    assert_eq!(defeated.state.hp, 0.0);
    assert_eq!(defeated.state.state, MinionBrainState::Dead);
    assert_eq!(defeated.state.target_kind, None);
    assert_eq!(defeated.state.target_id, None);

    // The production tick applies this retention before serializing the
    // next snapshot, which in turn drives client owner/proxy cleanup.
    world.minions.retain(|_, minion| minion.state.hp > 0.0);
    assert_eq!(world.minions.keys().copied().collect::<Vec<_>>(), [1]);
}
