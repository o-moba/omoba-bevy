//! Both real hosting entry paths run the same fixtures and explicit simulated clock.
use crate::{
    match_rules::MatchConfig,
    runtime::{
        ServerRuntime,
        ports::{ManualClock, MemoryTransport},
    },
};
use common::offline::{EPOCH, LOCAL_ADDR, PracticeSession};
use shared::{
    HeroClass,
    map::Team,
    wire::{CharacterChoice, ClientPacket, GameState, TargetId, TargetKind},
};
use std::time::{Duration, Instant};
fn join(class: HeroClass) -> ClientPacket {
    ClientPacket::Join {
        handheld: Default::default(),
        prematch: false,
        team: Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: class,
        avatar: None,
        sprite_character: None,
        session_id: None,
        passport_ticket: None,
    }
}
fn fixture(class: HeroClass, now: Instant) -> PracticeSession {
    let mut p = PracticeSession::new(now);
    p.command(join(class));
    p.command(ClientPacket::Practice {
        command: shared::practice::PracticeCommand::ClearBots,
    });
    p.world.structures.clear();
    p.world.minions.clear();
    p.world.neutrals.clear();
    p.world.last_wave_spawn_at = now + Duration::from_secs(3600);
    let enemy = "127.0.0.1:58001".parse().unwrap();
    p.world.ensure_connected(enemy, now);
    common::session::handle_join_request(
        p.world.players.get_mut(&enemy).unwrap(),
        Team::Blue,
        CharacterChoice::Ipfs,
        HeroClass::Warrior,
        None,
        &p.world.map_layout,
        now,
    );
    for (addr, player) in &mut p.world.players {
        player.hero.x = if *addr == LOCAL_ADDR { 10.0 } else { 13.0 };
        player.hero.z = 0.0;
        player.hero.hp = 2000.0;
        player.hero.max_hp = 2000.0;
    }
    p
}
fn state(w: &common::game_world::GameWorld, now: Instant) -> serde_json::Value {
    let mut minions: Vec<_> = w.minions.values().map(|m| m.state.clone()).collect();
    minions.sort_by_key(|m| m.id);
    let mut structures: Vec<_> = w.structures.values().map(|m| m.state.clone()).collect();
    structures.sort_by_key(|m| m.id);
    let mut neutrals: Vec<_> = w.neutrals.values().map(|m| m.state.clone()).collect();
    neutrals.sort_by_key(|m| m.id);
    let mut projectiles: Vec<_> = w.projectiles.values().map(|m| m.state.clone()).collect();
    projectiles.sort_by_key(|m| m.id);
    serde_json::json!({"players":common::snapshot::build_players_snapshot(w,Some(1),now),"effects":common::skills::effects(w,now),"minions":minions,"structures":structures,"neutrals":neutrals,"projectiles":projectiles})
}
#[test]
fn all_sixteen_classes_match_online_command_and_tick_outcomes_offline() {
    for class in HeroClass::ALL {
        let now = Instant::now();
        let clock = ManualClock::new(now);
        let mut online = ServerRuntime::for_test(
            MemoryTransport::new("127.0.0.1:4100".parse().unwrap()),
            clock.clone(),
            crate::career_backend::MemoryCareer::disabled(55000),
            MatchConfig::dev(),
        );
        let mut offline = fixture(class, now);
        online.world = fixture(class, now).world;
        online.server_epoch = EPOCH;
        online.match_id = 1;
        online.match_started_at = Some(now);
        online.world.game_state = GameState::Running;
        let target = TargetId {
            kind: TargetKind::Player,
            id: online.world.players[&"127.0.0.1:58001".parse().unwrap()]
                .hero
                .identity
                .id,
        };
        for step in 0..100 {
            if step % 20 == 0 && step < 80 {
                let slot = (step / 20) as u8;
                let packet = if class.is_standard() {
                    ClientPacket::CastSkill {
                        slot,
                        aim: [13.0, 0.0],
                        server_epoch: EPOCH,
                        match_id: 1,
                        request_id: slot as u64 + 1,
                    }
                } else {
                    ClientPacket::Cast { slot, target }
                };
                online.handle_packet_authorized(LOCAL_ADDR, packet.clone(), offline.now);
                offline.command(packet);
            }
            if step % 15 == 0 {
                let packet = ClientPacket::BasicAttack {
                    target,
                    server_epoch: EPOCH,
                    match_id: 1,
                    request_id: step + 1,
                };
                online.handle_packet_authorized(LOCAL_ADDR, packet.clone(), offline.now);
                offline.command(packet);
            }
            clock.advance(Duration::from_millis(50));
            offline.advance(0.05);
            for p in online.world.players.values_mut() {
                p.last_seen = offline.now;
            }
            online.tick(offline.now, 0.05);
            assert_eq!(
                state(&online.world, offline.now),
                state(&offline.world, offline.now),
                "{class:?} step {step}"
            );
        }
        assert!(
            offline.world.players.values().any(|p| p.hero.hp < 2000.0),
            "{class:?} actual damage"
        );
    }
}

#[test]
fn online_and_offline_projectiles_match_for_minions_structures_and_neutrals() {
    fn with_target(now: Instant, kind: TargetKind) -> (PracticeSession, TargetId) {
        let mut p = fixture(HeroClass::Ranger, now);
        p.world.players.retain(|a, _| *a == LOCAL_ADDR);
        let id = match kind {
            TargetKind::Minion => {
                common::world::spawn_minion_wave_for_team_lane(
                    &p.world.map_layout,
                    &mut p.world.minions,
                    &mut p.world.next_minion_id,
                    Team::Blue,
                    shared::map::Lane::Mid,
                );
                let id = *p.world.minions.keys().min().unwrap();
                p.world.minions.retain(|key, _| *key == id);
                let m = p.world.minions.get_mut(&id).unwrap();
                m.state.x = 13.0;
                m.state.z = 0.0;
                m.state.hp = 1.0;
                id
            }
            TargetKind::Structure => {
                p.world.structures =
                    common::world::build_configured_structures(&p.world.map_config);
                let id = p
                    .world
                    .structures
                    .values()
                    .filter(|s| {
                        s.state.team == Team::Blue
                            && s.state.kind == shared::wire::StructureKind::Tower
                    })
                    .map(|s| s.state.id)
                    .min()
                    .unwrap();
                p.world.structures.retain(|key, _| *key == id);
                let s = p.world.structures.get_mut(&id).unwrap();
                s.state.x = 13.0;
                s.state.z = 0.0;
                s.state.hp = 1.0;
                id
            }
            TargetKind::Neutral => {
                p.world.neutrals = common::neutrals::build_neutral_camps(&mut 9001);
                let id = *p.world.neutrals.keys().min().unwrap();
                p.world.neutrals.retain(|key, _| *key == id);
                let n = p.world.neutrals.get_mut(&id).unwrap();
                n.state.x = 13.0;
                n.state.z = 0.0;
                n.anchor.x = 13.0;
                n.anchor.z = 0.0;
                n.state.hp = 1.0;
                id
            }
            _ => unreachable!(),
        };
        (p, TargetId { kind, id })
    }
    for kind in [
        TargetKind::Minion,
        TargetKind::Structure,
        TargetKind::Neutral,
    ] {
        let now = Instant::now();
        let clock = ManualClock::new(now);
        let mut online = ServerRuntime::for_test(
            MemoryTransport::new("127.0.0.1:4100".parse().unwrap()),
            clock.clone(),
            crate::career_backend::MemoryCareer::disabled(55000),
            MatchConfig::dev(),
        );
        let (mut offline, target) = with_target(now, kind);
        online.world = with_target(now, kind).0.world;
        online.server_epoch = EPOCH;
        online.match_id = 1;
        online.match_started_at = Some(now);
        let packet = ClientPacket::BasicAttack {
            target,
            server_epoch: EPOCH,
            match_id: 1,
            request_id: 1,
        };
        online.handle_packet_authorized(LOCAL_ADDR, packet.clone(), now);
        offline.command(packet);
        for step in 0..15 {
            clock.advance(Duration::from_millis(50));
            offline.advance(0.05);
            online.tick(offline.now, 0.05);
            assert_eq!(
                state(&online.world, offline.now),
                state(&offline.world, offline.now),
                "{kind:?} step {step}"
            );
        }
        assert!(
            offline
                .combat_log
                .snapshot(offline.now)
                .iter()
                .any(|e| e.target.id == target.id && e.killed),
            "{kind:?} actual lethal hit"
        );
    }
}
