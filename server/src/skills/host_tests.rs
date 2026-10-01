use crate::session::handle_join_request;
use crate::skills::*;
use common::game_world::{GameWorld, TickCtx};
use shared::combat::CombatEvent;
use shared::{HeroClass, loadout::*, map::Team, wire::*};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
#[test]
fn cast_protocol_rejects_old_round_and_requires_hello_for_standard_admission() {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = crate::runtime::ServerRuntime::new(socket, crate::match_rules::MatchConfig::dev());
    let now = Instant::now();
    let join = shared::wire::ClientPacket::Join {
        handheld: Default::default(),
        prematch: false,
        team: Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: HeroClass::Dawnweaver,
        avatar: None,
        sprite_character: None,
        session_id: Some("standard-test".into()),
        passport_ticket: None,
    };
    rt.handle_packet(addr(1), join.clone(), now);
    assert!(!rt.world.players[&addr(1)].joined);
    assert_eq!(
        rt.world.players[&addr(1)].join_error,
        Some(shared::protocol::JoinRejection::ProtocolMismatch)
    );
    rt.handle_packet(
        addr(1),
        shared::wire::ClientPacket::Hello {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
        },
        now,
    );
    rt.handle_packet(addr(1), join, now);
    assert!(rt.world.players[&addr(1)].joined);
    rt.world.game_state = GameState::Running;
    rt.handle_packet(
        addr(1),
        shared::wire::ClientPacket::CastSkill {
            slot: 0,
            aim: [0.0, 10.0],
            server_epoch: rt.server_epoch,
            match_id: rt.match_id + 1,
            request_id: 999,
        },
        now,
    );
    assert_eq!(rt.world.players[&addr(1)].hero.skills.request_id, 0);
    rt.handle_packet(
        addr(1),
        shared::wire::ClientPacket::CastSkill {
            slot: 0,
            aim: [0.0, 10.0],
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            request_id: 1,
        },
        now,
    );
    assert_eq!(rt.world.players[&addr(1)].hero.skills.request_id, 1);
}

#[test]
fn practice_controllers_cast_standard_skills_through_live_runtime() {
    use crate::bots::BotKind;
    use crate::match_rules::{MatchConfig, MatchMode};
    for class in [HeroClass::Dawnweaver, HeroClass::Wildspark] {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = crate::runtime::ServerRuntime::new(
            socket,
            MatchConfig {
                mode: MatchMode::Practice,
                team_size: 1,
            },
        );
        let now = Instant::now();
        let bot = rt
            .spawn_bot(Team::Green, Some(class), BotKind::Duelist, now)
            .unwrap();
        let p = rt.world.players.get_mut(&bot).unwrap();
        p.hero.x = 0.0;
        p.hero.z = 0.0;
        p.hero.progress.level = 6;
        p.hero.mana = 500.0;
        rt.world.structures.clear();
        rt.world.neutrals.clear();
        add_player(
            &mut rt.world,
            1,
            HeroClass::Warrior,
            Team::Blue,
            [6.0, 0.0],
            now,
        );
        rt.world.game_state = GameState::Running;
        rt.simulate_bots(now, 0.05);
        assert!(
            !crate::skills::effects(&rt.world, now).is_empty(),
            "{class:?}"
        );
        assert!(rt.world.players[&bot].hero.mana < 500.0);
        advance(&mut rt.world, now, 0.5);
        assert!(rt.world.players[&addr(1)].hero.hp < 1000.0, "{class:?}");
        if class == HeroClass::Wildspark {
            assert_eq!(rt.world.players[&bot].hero.skills.mode, WeaponMode::Rockets);
        }
    }
}

fn addr(n: u16) -> SocketAddr {
    format!("127.0.0.1:{}", 56000 + n).parse().unwrap()
}
fn add_player(
    w: &mut GameWorld,
    n: u16,
    class: HeroClass,
    team: Team,
    pos: [f32; 2],
    now: Instant,
) -> u64 {
    w.ensure_connected(addr(n), now);
    handle_join_request(
        w.players.get_mut(&addr(n)).unwrap(),
        team,
        CharacterChoice::Ipfs,
        class,
        None,
        &w.map_layout,
        now,
    );
    let p = w.players.get_mut(&addr(n)).unwrap();
    p.hero.x = pos[0];
    p.hero.z = pos[1];
    p.hero.hp = 1000.0;
    p.hero.max_hp = 1000.0;
    p.hero.mana = 500.0;
    p.hero.max_mana = 500.0;
    p.modifiers.unlock_all = true;
    p.hero.identity.id
}
fn advance(w: &mut GameWorld, now: Instant, seconds: f32) -> Vec<CombatEvent> {
    let mut result = Vec::new();
    let count = (seconds / 0.05).ceil() as usize;
    for i in 1..=count {
        let at = now + Duration::from_secs_f32((i as f32 * 0.05).min(seconds));
        let events = tick(w, TickCtx { now: at, dt: 0.05 });
        observe(w, &events, at);
        normalize(w, at);
        result.extend(events);
    }
    result
}
