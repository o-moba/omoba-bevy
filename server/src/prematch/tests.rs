use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use shared::HeroClass;
use shared::map::Team;
use shared::wire::{CharacterChoice, ClientPacket, GameState, ServerPacket};

use super::*;
use crate::career_backend;
use crate::formation::joined_count;
use crate::match_rules::{MatchConfig, MatchMode};
use crate::runtime::ServerRuntime;
fn fixture(mode: MatchMode, size: u32) -> (ServerRuntime, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    (
        ServerRuntime::new(
            socket,
            MatchConfig {
                mode,
                team_size: size,
            },
        ),
        Instant::now(),
    )
}
fn address(index: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 59000 + index))
}
fn join(rt: &mut ServerRuntime, addr: SocketAddr, session: &str, now: Instant, enabled: bool) {
    rt.handle_packet(
        addr,
        ClientPacket::Join {
            prematch: enabled,
            team: Team::Blue,
            character: CharacterChoice::Cube,
            hero_class: HeroClass::Warrior,
            avatar: None,
            sprite_character: None,
            session_id: Some(session.into()),
            passport_ticket: None,
        },
        now,
    );
}
fn send(rt: &mut ServerRuntime, addr: SocketAddr, id: u64, action: PrematchAction, now: Instant) {
    let request = PrematchRequest {
        server_epoch: rt.server_epoch,
        match_id: rt.match_id,
        generation: rt.prematch.generation,
        request_id: id,
        action,
    };
    rt.handle_packet(addr, ClientPacket::Prematch { request }, now);
}
fn select(role: Role, class: HeroClass) -> PrematchAction {
    PrematchAction::Select {
        character: CharacterChoice::Ipfs,
        hero_class: class,
        avatar: None,
        sprite_character: None,
        role,
        passport_ticket: None,
    }
}
fn view(rt: &ServerRuntime, addr: SocketAddr, now: Instant) -> PrematchSnapshot {
    snapshot(
        &rt.prematch,
        &rt.world.players,
        &rt.world.players[&addr],
        rt.rules,
        now,
    )
    .unwrap()
}
#[test]
fn automatic_teams_and_opt_in_draft_work_in_all_modes_legacy_remains_inert() {
    for mode in [MatchMode::Dev, MatchMode::Release, MatchMode::Practice] {
        let (mut rt, now) = fixture(mode, 2);
        join(&mut rt, address(1), "one", now, true);
        join(&mut rt, address(2), "two", now, true);
        assert_eq!(
            rt.world.players[&address(1)].hero.identity.team,
            Team::Green
        );
        assert_eq!(rt.world.players[&address(2)].hero.identity.team, Team::Blue);
        assert_eq!(view(&rt, address(1), now).phase, PrematchPhase::Draft);
        assert!(rt.match_started_at.is_none());
    }
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    join(&mut rt, address(1), "old", now, false);
    assert_eq!(rt.world.players[&address(1)].hero.identity.team, Team::Blue);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(
        snapshot(
            &rt.prematch,
            &rt.world.players,
            &rt.world.players[&address(1)],
            rt.rules,
            now
        )
        .is_none()
    );
    let legacy: ClientPacket = serde_json::from_str(r#"{"type":"join","team":"green"}"#).unwrap();
    assert!(matches!(
        legacy,
        ClientPacket::Join {
            prematch: false,
            ..
        }
    ));
}
#[test]
fn two_peers_share_selection_countdown_and_wait_for_every_asset_ack() {
    let (mut rt, now) = fixture(MatchMode::Release, 1);
    for i in 1..=2 {
        join(&mut rt, address(i), &format!("peer-{i}"), now, true);
    }
    send(
        &mut rt,
        address(1),
        1,
        select(Role::Jungle, HeroClass::Mage),
        now,
    );
    let peer = view(&rt, address(2), now);
    let row = peer
        .players
        .iter()
        .find(|p| p.player_id == rt.world.players[&address(1)].hero.identity.id)
        .unwrap();
    assert_eq!(row.role, Role::Jungle);
    assert_eq!(row.hero_class, HeroClass::Mage);
    send(
        &mut rt,
        address(1),
        2,
        PrematchAction::Lock { locked: true },
        now,
    );
    assert_eq!(view(&rt, address(2), now).phase, PrematchPhase::Draft);
    send(
        &mut rt,
        address(2),
        1,
        PrematchAction::Lock { locked: true },
        now,
    );
    assert_eq!(view(&rt, address(1), now).remaining_ms, 3000);
    assert_eq!(
        view(&rt, address(1), now).remaining_ms,
        view(&rt, address(2), now).remaining_ms
    );
    rt.tick_prematch(now + Duration::from_secs(3));
    assert_eq!(
        view(&rt, address(1), now + Duration::from_secs(3)).phase,
        PrematchPhase::Loading
    );
    send(
        &mut rt,
        address(1),
        3,
        PrematchAction::Loaded,
        now + Duration::from_secs(3),
    );
    assert!(rt.match_started_at.is_none());
    send(
        &mut rt,
        address(2),
        2,
        PrematchAction::Loaded,
        now + Duration::from_secs(3),
    );
    assert!(matches!(rt.world.game_state, GameState::Running));
    assert!(rt.match_started_at.is_some());
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Mage
    );
    send(
        &mut rt,
        address(1),
        4,
        select(Role::Solo, HeroClass::Warrior),
        now + Duration::from_secs(3),
    );
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Mage
    );
}
#[test]
fn stale_namespace_sequence_generation_and_unknown_avatar_preserve_choice() {
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    join(&mut rt, address(1), "one", now, true);
    let generation = rt.prematch.generation;
    send(
        &mut rt,
        address(1),
        1,
        select(Role::Jungle, HeroClass::Mage),
        now,
    );
    send(
        &mut rt,
        address(1),
        1,
        select(Role::Solo, HeroClass::Warrior),
        now,
    );
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Mage
    );
    for (epoch, mat, requested_generation) in [
        (rt.server_epoch + 1, rt.match_id, generation),
        (rt.server_epoch, rt.match_id + 1, generation),
        (rt.server_epoch, rt.match_id, generation + 1),
    ] {
        rt.handle_packet(
            address(1),
            ClientPacket::Prematch {
                request: PrematchRequest {
                    server_epoch: epoch,
                    match_id: mat,
                    generation: requested_generation,
                    request_id: 90,
                    action: select(Role::Solo, HeroClass::Warrior),
                },
            },
            now,
        );
    }
    assert_eq!(rt.world.players[&address(1)].draft.request_id, 1);
    let mut action = select(Role::Solo, HeroClass::Warrior);
    if let PrematchAction::Select { avatar, .. } = &mut action {
        *avatar = Some("unregistered-invalid-model".into());
    }
    send(&mut rt, address(1), 2, action, now);
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Mage
    );
    assert!(view(&rt, address(1), now).error.is_some());
}
#[test]
fn dropout_cancels_countdown_and_reconnect_preserves_identity_but_requires_lock_again() {
    let (mut rt, now) = fixture(MatchMode::Release, 1);
    join(&mut rt, address(1), "one", now, true);
    join(&mut rt, address(2), "two", now, true);
    let identity = rt.world.players[&address(2)].hero.identity.id;
    for i in 1..=2 {
        send(
            &mut rt,
            address(i),
            1,
            PrematchAction::Lock { locked: true },
            now,
        );
    }
    let generation = rt.prematch.generation;
    rt.world.players.get_mut(&address(1)).unwrap().last_seen = now + Duration::from_secs(6);
    rt.maintain_roster(now + Duration::from_secs(6));
    rt.tick_prematch(now + Duration::from_secs(6));
    assert_eq!(
        view(&rt, address(1), now + Duration::from_secs(6)).phase,
        PrematchPhase::Draft
    );
    assert!(rt.prematch.generation > generation);
    join(
        &mut rt,
        address(3),
        "two",
        now + Duration::from_secs(6),
        true,
    );
    rt.tick_prematch(now + Duration::from_secs(6));
    assert_eq!(rt.world.players[&address(3)].hero.identity.id, identity);
    assert!(!rt.world.players[&address(1)].draft.locked);
    assert!(!rt.world.players[&address(3)].draft.locked);
}
#[test]
fn practice_bots_auto_ready_and_loading_timeout_returns_to_draft() {
    let (mut rt, now) = fixture(MatchMode::Practice, 5);
    join(&mut rt, address(1), "solo", now, true);
    assert_eq!(view(&rt, address(1), now).players.len(), 10);
    assert!(
        view(&rt, address(1), now)
            .players
            .iter()
            .filter(|p| p.is_bot)
            .all(|p| p.locked && p.loaded)
    );
    send(
        &mut rt,
        address(1),
        1,
        PrematchAction::Lock { locked: true },
        now,
    );
    rt.tick_prematch(now + Duration::from_secs(3));
    let generation = rt.prematch.generation;
    rt.tick_prematch(now + Duration::from_secs(34));
    let state = view(&rt, address(1), now + Duration::from_secs(34));
    assert_eq!(state.phase, PrematchPhase::Draft);
    assert!(state.generation > generation);
    assert!(state.error.unwrap().contains("timed out"));
    assert!(!rt.world.players[&address(1)].draft.locked);
}
#[test]
fn real_udp_peers_negotiate_and_replicate_draft_without_client_fixtures() {
    let (mut rt, now) = fixture(MatchMode::Release, 1);
    let peers: Vec<_> = (0..2)
        .map(|_| {
            let s = UdpSocket::bind("127.0.0.1:0").unwrap();
            s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            s
        })
        .collect();
    for (i, peer) in peers.iter().enumerate() {
        let bytes=serde_json::to_vec(&serde_json::json!({"type":"join","prematch":true,"team":"blue","character":"cube","session_id":format!("wire-{i}")})).unwrap();
        peer.send_to(&bytes, rt.transport.local_addr().unwrap())
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while joined_count(&rt.world.players) < 2 && Instant::now() < deadline {
        rt.receive_packets();
    }
    assert_eq!(joined_count(&rt.world.players), 2);
    let a = peers[0].local_addr().unwrap();
    let b = peers[1].local_addr().unwrap();
    let req = PrematchRequest {
        server_epoch: rt.server_epoch,
        match_id: rt.match_id,
        generation: rt.prematch.generation,
        request_id: 1,
        action: select(Role::Support, HeroClass::Cleric),
    };
    peers[0]
        .send_to(
            &serde_json::to_vec(&ClientPacket::Prematch { request: req }).unwrap(),
            rt.transport.local_addr().unwrap(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while rt.world.players[&a].draft.request_id < 1 && Instant::now() < deadline {
        rt.receive_packets();
    }
    rt.last_snapshot_at = now - Duration::from_secs(1);
    rt.tick(Instant::now(), 0.01);
    let mut buffer = vec![0u8; 65536];
    let (len, _) = peers[1].recv_from(&mut buffer).unwrap();
    let packet: ServerPacket = serde_json::from_slice(&buffer[..len]).unwrap();
    let ServerPacket::Snapshot {
        your_id,
        prematch: Some(draft),
        ..
    } = packet
    else {
        panic!("authoritative draft snapshot missing")
    };
    assert_eq!(your_id, rt.world.players[&b].hero.identity.id);
    assert!(
        draft
            .players
            .iter()
            .any(|p| p.player_id == rt.world.players[&a].hero.identity.id
                && p.role == Role::Support
                && p.hero_class == HeroClass::Cleric)
    );
    // Continue through the actual wire receiver, including a countdown dropout.
    fn wire(rt: &mut ServerRuntime, peer: &UdpSocket, id: u64, action: PrematchAction) {
        let addr = peer.local_addr().unwrap();
        let packet = ClientPacket::Prematch {
            request: PrematchRequest {
                server_epoch: rt.server_epoch,
                match_id: rt.match_id,
                generation: rt.prematch.generation,
                request_id: id,
                action,
            },
        };
        peer.send_to(
            &serde_json::to_vec(&packet).unwrap(),
            rt.transport.local_addr().unwrap(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while rt.world.players[&addr].draft.request_id < id && Instant::now() < deadline {
            rt.receive_packets();
        }
        assert_eq!(rt.world.players[&addr].draft.request_id, id);
    }
    wire(&mut rt, &peers[0], 2, PrematchAction::Lock { locked: true });
    wire(&mut rt, &peers[1], 1, PrematchAction::Lock { locked: true });
    assert_eq!(view(&rt, a, now).phase, PrematchPhase::Countdown);
    let old_generation = rt.prematch.generation;
    let returning_id = rt.world.players[&b].hero.identity.id;
    rt.world.players.get_mut(&b).unwrap().last_seen = Instant::now() - Duration::from_secs(6);
    rt.maintain_roster(Instant::now());
    rt.tick_prematch(Instant::now());
    assert!(rt.prematch.generation > old_generation);
    assert_eq!(view(&rt, a, now).phase, PrematchPhase::Draft);
    let returning = UdpSocket::bind("127.0.0.1:0").unwrap();
    returning
        .send_to(
            br#"{"type":"join","prematch":true,"team":"green","session_id":"wire-1"}"#,
            rt.transport.local_addr().unwrap(),
        )
        .unwrap();
    let c = returning.local_addr().unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !rt.world.players.get(&c).is_some_and(|p| p.joined) && Instant::now() < deadline {
        rt.receive_packets();
    }
    rt.tick_prematch(Instant::now());
    assert_eq!(rt.world.players[&c].hero.identity.id, returning_id);
    wire(&mut rt, &peers[0], 3, PrematchAction::Lock { locked: true });
    wire(
        &mut rt,
        &returning,
        2,
        PrematchAction::Lock { locked: true },
    );
    rt.tick_prematch(Instant::now() + Duration::from_secs(3));
    assert_eq!(view(&rt, a, now).phase, PrematchPhase::Loading);
    wire(&mut rt, &peers[0], 4, PrematchAction::Loaded);
    assert!(rt.match_started_at.is_none());
    wire(&mut rt, &returning, 3, PrematchAction::Loaded);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(rt.match_started_at.is_some());
    let first_id = rt.world.players[&a].hero.identity.id;
    rt.world.players.get_mut(&a).unwrap().last_seen = Instant::now() - Duration::from_secs(6);
    rt.maintain_roster(Instant::now());
    let running_rejoin = UdpSocket::bind("127.0.0.1:0").unwrap();
    running_rejoin
        .send_to(
            br#"{"type":"join","prematch":true,"team":"blue","session_id":"wire-0"}"#,
            rt.transport.local_addr().unwrap(),
        )
        .unwrap();
    let d = running_rejoin.local_addr().unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !rt.world.players.get(&d).is_some_and(|p| p.joined) && Instant::now() < deadline {
        rt.receive_packets();
    }
    assert_eq!(rt.world.players[&d].hero.identity.id, first_id);
    assert_eq!(
        rt.world.players[&d].hero.identity.hero_class,
        HeroClass::Cleric
    );
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(
        snapshot(
            &rt.prematch,
            &rt.world.players,
            &rt.world.players[&d],
            rt.rules,
            Instant::now()
        )
        .is_none()
    );
}

#[test]
fn loaded_peers_still_wait_for_durable_career_ack_and_frozen_loadout_is_recorded() {
    let (mut rt, now) = fixture(MatchMode::Release, 1);
    rt.career.backend = Box::new(career_backend::MemoryCareer::test_backend(rt.server_epoch));
    for i in 1..=2 {
        let session = format!("auth-{i}");
        rt.career.backend.test_authenticated(
            address(i),
            shared::career::ProfileSummary::new(format!("{:064x}", i), format!("Hero {i}")),
            &session,
        );
        join(&mut rt, address(i), &session, now, true);
    }
    rt.tick_prematch(now);
    send(
        &mut rt,
        address(1),
        1,
        select(Role::Support, HeroClass::Cleric),
        now,
    );
    for i in 1..=2 {
        send(
            &mut rt,
            address(i),
            2,
            PrematchAction::Lock { locked: true },
            now,
        );
    }
    let loading = now + Duration::from_secs(3);
    rt.tick_prematch(loading);
    for i in 1..=2 {
        send(&mut rt, address(i), 3, PrematchAction::Loaded, loading);
    }
    assert!(rt.match_started_at.is_none());
    assert_eq!(view(&rt, address(1), loading).phase, PrematchPhase::Loading);
    let allocation = rt.career_allocation_for_test().unwrap();
    assert!(allocation.participants.iter().any(|p| p.player_id
        == rt.world.players[&address(1)].hero.identity.id
        && p.hero_class == HeroClass::Cleric));
    assert!(!rt.combat_log.ledger.is_started());
    let gold = rt.world.players[&address(1)].economy.gold;
    rt.tick(loading, 0.1);
    assert_eq!(rt.world.players[&address(1)].economy.gold, gold);
    assert!(rt.world.minions.is_empty());
    rt.career.backend.test_ack_start(&allocation.result_id);
    rt.tick_prematch(loading + Duration::from_millis(1));
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(rt.match_started_at.is_some());
}

#[test]
fn running_reconnect_bypasses_draft_and_preserves_class_role_identity() {
    let (mut rt, now) = fixture(MatchMode::Practice, 1);
    join(&mut rt, address(1), "returning", now, true);
    send(
        &mut rt,
        address(1),
        1,
        select(Role::Jungle, HeroClass::Mage),
        now,
    );
    send(
        &mut rt,
        address(1),
        2,
        PrematchAction::Lock { locked: true },
        now,
    );
    rt.tick_prematch(now + Duration::from_secs(3));
    send(
        &mut rt,
        address(1),
        3,
        PrematchAction::Loaded,
        now + Duration::from_secs(3),
    );
    let identity = rt.world.players[&address(1)].hero.identity.id;
    join(
        &mut rt,
        address(2),
        "returning",
        now + Duration::from_secs(9),
        true,
    );
    assert_eq!(rt.world.players[&address(2)].hero.identity.id, identity);
    assert_eq!(
        rt.world.players[&address(2)].hero.identity.hero_class,
        HeroClass::Mage
    );
    assert_eq!(rt.world.players[&address(2)].draft.role, Role::Jungle);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(
        snapshot(
            &rt.prematch,
            &rt.world.players,
            &rt.world.players[&address(2)],
            rt.rules,
            now
        )
        .is_none()
    );
}

#[test]
fn loading_leave_rejoin_invalidates_ready_even_if_same_endpoint_and_identity_return() {
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    join(&mut rt, address(1), "one", now, true);
    join(&mut rt, address(2), "two", now, true);
    for i in 1..=2 {
        send(
            &mut rt,
            address(i),
            1,
            PrematchAction::Lock { locked: true },
            now,
        );
    }
    rt.tick_prematch(now + Duration::from_secs(3));
    send(
        &mut rt,
        address(1),
        2,
        PrematchAction::Loaded,
        now + Duration::from_secs(3),
    );
    let generation = rt.prematch.generation;
    rt.handle_packet(
        address(1),
        ClientPacket::Leave,
        now + Duration::from_secs(3),
    );
    join(
        &mut rt,
        address(1),
        "one",
        now + Duration::from_secs(3),
        true,
    );
    assert!(rt.prematch.generation > generation);
    assert_eq!(view(&rt, address(1), now).phase, PrematchPhase::Draft);
    assert!(
        rt.world
            .players
            .values()
            .filter(|p| p.joined)
            .all(|p| !p.draft.loaded && !p.draft.locked)
    );
}

#[test]
fn incapable_legacy_peer_is_ready_without_being_silently_required_to_ack() {
    let (mut rt, now) = fixture(MatchMode::Release, 1);
    join(&mut rt, address(1), "modern", now, true);
    join(&mut rt, address(2), "legacy", now, false);
    let legacy = view(&rt, address(1), now)
        .players
        .into_iter()
        .find(|p| p.player_id == rt.world.players[&address(2)].hero.identity.id)
        .unwrap();
    assert!(legacy.locked && legacy.loaded);
    send(
        &mut rt,
        address(1),
        1,
        PrematchAction::Lock { locked: true },
        now,
    );
    rt.tick_prematch(now + Duration::from_secs(3));
    send(
        &mut rt,
        address(1),
        2,
        PrematchAction::Loaded,
        now + Duration::from_secs(3),
    );
    assert_eq!(rt.world.game_state, GameState::Running);
}

#[test]
fn durable_allocation_timeout_is_bounded_and_retires_the_pending_roster() {
    let (mut rt, now) = fixture(MatchMode::Dev, 1);
    rt.career.backend = Box::new(career_backend::MemoryCareer::test_backend(rt.server_epoch));
    join(&mut rt, address(1), "service-timeout", now, true);
    send(
        &mut rt,
        address(1),
        1,
        PrematchAction::Lock { locked: true },
        now,
    );
    let loading = now + Duration::from_secs(3);
    rt.tick_prematch(loading);
    send(&mut rt, address(1), 2, PrematchAction::Loaded, loading);
    assert!(rt.career_allocation_for_test().is_some());
    assert!(rt.match_started_at.is_none());
    rt.tick_prematch(loading + Duration::from_secs(30));
    let state = view(&rt, address(1), loading + Duration::from_secs(30));
    assert_eq!(state.phase, PrematchPhase::Draft);
    assert!(state.error.unwrap().contains("Match start took too long"));
    assert!(rt.career_allocation_for_test().is_none());
    assert!(!rt.world.players[&address(1)].draft.loaded);
    assert!(!rt.combat_log.ledger.is_started());
}

#[test]
fn late_avatar_authorization_cannot_modify_a_new_roster_generation() {
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    join(&mut rt, address(1), "one", now, true);
    let request = PrematchRequest {
        server_epoch: rt.server_epoch,
        match_id: rt.match_id,
        generation: rt.prematch.generation,
        request_id: 1,
        action: select(Role::Support, HeroClass::Cleric),
    };
    // Deterministic worker-completion seam; actual async registry entry is
    // covered independently by passport_admission's draft selection test.
    rt.prematch.pending.insert(
        address(1),
        (rt.world.players[&address(1)].hero.identity.id, 1),
    );
    rt.world
        .players
        .get_mut(&address(1))
        .unwrap()
        .draft
        .request_id = 1;
    join(&mut rt, address(2), "two", now, true);
    assert!(rt.prematch.generation > request.generation);
    assert_eq!(view(&rt, address(1), now).last_request_id, 1);
    rt.complete_prematch_admission(address(1), request, true, now);
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Warrior
    );
}
