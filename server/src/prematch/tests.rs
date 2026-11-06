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
            handheld: Default::default(),
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
        handheld: Default::default(),
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
    assert_eq!(view(&rt, address(1), now).remaining_ms, DRAFT_SELECTION_MS);
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
    assert_eq!(view(&rt, address(1), now).phase, PrematchPhase::Countdown);
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
fn selection_expiry_freezes_accepted_choices_and_defaults_but_waits_for_assets() {
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
    let expired = now + Duration::from_millis(DRAFT_SELECTION_MS.into());
    rt.tick_prematch(expired - Duration::from_millis(1));
    assert_eq!(
        view(&rt, address(1), expired - Duration::from_millis(1)).remaining_ms,
        1
    );
    assert!(!rt.world.players[&address(1)].draft.locked);
    rt.tick_prematch(expired);
    let state = view(&rt, address(1), expired);
    assert_eq!(state.phase, PrematchPhase::Countdown);
    assert_eq!(state.remaining_ms, COUNTDOWN_MS);
    assert!(state.players.iter().all(|p| p.locked));
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Mage
    );
    assert_eq!(rt.world.players[&address(1)].draft.role, Role::Jungle);
    assert_eq!(
        rt.world.players[&address(2)].hero.identity.character,
        CharacterChoice::Cube
    );
    assert_eq!(
        rt.world.players[&address(2)].hero.identity.hero_class,
        HeroClass::Warrior
    );
    assert!(rt.match_started_at.is_none());

    // A Loaded message during countdown does not acknowledge the asset barrier.
    for player in rt.world.players.values_mut() {
        player.last_seen = expired;
    }
    send(&mut rt, address(1), 2, PrematchAction::Loaded, expired);
    assert!(!rt.world.players[&address(1)].draft.loaded);
    let loading = expired + Duration::from_millis(COUNTDOWN_MS.into());
    rt.tick_prematch(loading);
    assert_eq!(view(&rt, address(1), loading).phase, PrematchPhase::Loading);
    send(&mut rt, address(1), 3, PrematchAction::Loaded, loading);
    assert!(rt.match_started_at.is_none());
    send(&mut rt, address(2), 1, PrematchAction::Loaded, loading);
    assert_eq!(rt.world.game_state, GameState::Running);
}

#[test]
fn incomplete_roster_does_not_consume_the_selection_window() {
    let (mut rt, now) = fixture(MatchMode::Release, 1);
    join(&mut rt, address(1), "one", now, true);
    assert_eq!(view(&rt, address(1), now).remaining_ms, 0);
    let complete = now + Duration::from_secs(45);
    rt.tick_prematch(complete);
    assert_eq!(view(&rt, address(1), complete).remaining_ms, 0);
    assert!(!rt.world.players[&address(1)].draft.locked);
    rt.world.players.get_mut(&address(1)).unwrap().last_seen = complete;
    join(&mut rt, address(2), "two", complete, true);
    let state = view(&rt, address(1), complete);
    assert_eq!(state.phase, PrematchPhase::Draft);
    assert_eq!(state.remaining_ms, DRAFT_SELECTION_MS);
}

#[test]
fn party_gathering_finishes_before_the_selection_window_starts() {
    use shared::match_service::MatchPreference;
    use shared::party::PartyCommand;

    let (mut rt, now) = fixture(MatchMode::Practice, 5);
    for i in 1..=2 {
        rt.handle_packet(
            address(i),
            ClientPacket::Party {
                command: PartyCommand::Presence {
                    nickname: format!("Party {i}"),
                    avatar: None,
                },
            },
            now,
        );
    }
    let leader = rt.world.players[&address(1)].hero.identity.id;
    let friend = rt.world.players[&address(2)].hero.identity.id;
    let party_id = rt.party.invite(leader, friend, now).unwrap();
    rt.party.accept(friend, party_id, now).unwrap();
    rt.party
        .launch(leader, MatchPreference::BotPractice, now)
        .unwrap();
    join(&mut rt, address(1), "leader", now, true);
    assert!(rt.party_gathering(now));
    assert_eq!(view(&rt, address(1), now).remaining_ms, 0);
    let halfway = now + Duration::from_secs(30);
    rt.tick_prematch(halfway);
    assert_eq!(view(&rt, address(1), halfway).phase, PrematchPhase::Draft);
    assert_eq!(view(&rt, address(1), halfway).remaining_ms, 0);
    assert!(!rt.world.players[&address(1)].draft.locked);

    // The existing bounded party gather expires first; the full draft window
    // then belongs to the remaining roster, rather than expiring retroactively.
    let gathered = now + crate::party::LAUNCH_GATHER;
    rt.tick_prematch(gathered);
    assert_eq!(
        view(&rt, address(1), gathered).remaining_ms,
        DRAFT_SELECTION_MS
    );
    rt.tick_prematch(gathered + Duration::from_millis(DRAFT_SELECTION_MS.into()));
    assert_eq!(
        view(&rt, address(1), gathered).phase,
        PrematchPhase::Countdown
    );
}

#[test]
fn roster_change_restarts_a_complete_selection_window() {
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    join(&mut rt, address(1), "one", now, true);
    let generation = rt.prematch.generation;
    let changed = now + Duration::from_secs(29);
    rt.world.players.get_mut(&address(1)).unwrap().last_seen = changed;
    join(&mut rt, address(2), "two", changed, true);
    let state = view(&rt, address(1), changed);
    assert!(state.generation > generation);
    assert_eq!(state.remaining_ms, DRAFT_SELECTION_MS);
    let old_expiry = now + Duration::from_millis(DRAFT_SELECTION_MS.into());
    rt.tick_prematch(old_expiry);
    assert_eq!(
        view(&rt, address(1), old_expiry).phase,
        PrematchPhase::Draft
    );
    assert_eq!(
        view(&rt, address(1), old_expiry).remaining_ms,
        DRAFT_SELECTION_MS - 1_000
    );
    rt.tick_prematch(changed + Duration::from_millis(DRAFT_SELECTION_MS.into()));
    assert_eq!(
        view(&rt, address(1), changed).phase,
        PrematchPhase::Countdown
    );
}

#[test]
fn selection_expiry_waits_for_pending_avatar_admission_before_freezing() {
    for allowed in [false, true] {
        let (mut rt, now) = fixture(MatchMode::Dev, 1);
        join(&mut rt, address(1), "one", now, true);
        let request = PrematchRequest {
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            generation: rt.prematch.generation,
            request_id: 1,
            action: select(Role::Support, HeroClass::Cleric),
        };
        // Deterministic admission completion seam, as in the generation test.
        // Registry verification itself is covered in passport_admission tests.
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
        let after_expiry = now + Duration::from_secs(45);
        rt.tick_prematch(after_expiry);
        let waiting = view(&rt, address(1), after_expiry);
        assert_eq!(waiting.phase, PrematchPhase::Draft);
        assert_eq!(waiting.remaining_ms, 0);
        assert_eq!(waiting.last_request_id, 0);
        assert!(!rt.world.players[&address(1)].draft.locked);
        assert!(rt.match_started_at.is_none());

        rt.complete_prematch_admission(address(1), request, allowed, after_expiry);
        rt.tick_prematch(after_expiry);
        let state = view(&rt, address(1), after_expiry);
        assert_eq!(state.phase, PrematchPhase::Countdown);
        assert_eq!(state.remaining_ms, COUNTDOWN_MS);
        assert_eq!(state.last_request_id, 1);
        assert!(rt.world.players[&address(1)].draft.locked);
        assert_eq!(
            rt.world.players[&address(1)].hero.identity.hero_class,
            if allowed {
                HeroClass::Cleric
            } else {
                HeroClass::Warrior
            }
        );
        assert_eq!(
            rt.world.players[&address(1)].draft.role,
            if allowed { Role::Support } else { Role::Mid }
        );
        assert_eq!(state.error.is_some(), !allowed);
        assert!(rt.match_started_at.is_none());
    }
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
    assert_eq!(state.error.as_deref(), Some("loading_timeout"));
    assert!(!rt.world.players[&address(1)].draft.locked);
    let retry = now + Duration::from_secs(35);
    rt.tick_prematch(retry);
    assert_eq!(
        view(&rt, address(1), retry).remaining_ms,
        DRAFT_SELECTION_MS
    );
    assert!(!rt.world.players[&address(1)].draft.loaded);
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
            shared::career::ProfileSummary {
                ranked: None,
                completed_public_matches: 10,
                ..shared::career::ProfileSummary::new(format!("{:064x}", i), format!("Hero {i}"))
            },
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
    assert_eq!(view(&rt, address(1), loading).phase, PrematchPhase::Loading);
    assert!(rt.match_started_at.is_none());
    rt.career.backend.test_ack_gameplay(&allocation.result_id);
    rt.tick_prematch(loading + Duration::from_millis(2));
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
    assert_eq!(state.error.as_deref(), Some("start_timeout"));
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

fn allocated_loading_roster(
    preference: shared::match_service::MatchPreference,
) -> (ServerRuntime, Instant, std::path::PathBuf) {
    use crate::match_allocation::{AllocatedHuman, Manifest, Worker, unix_ms};
    let mode = if preference.rated() {
        MatchMode::Release
    } else {
        MatchMode::Practice
    };
    let (mut rt, now) = fixture(mode, 5);
    rt.career.backend = Box::new(crate::career_backend::MemoryCareer::test_backend(
        rt.server_epoch,
    ));
    let directory = std::env::temp_dir().join(format!("omoba-loading-product-{}", rt.server_epoch));
    std::fs::create_dir_all(&directory).unwrap();
    let humans: Vec<_> = (1..=10)
        .map(|i| AllocatedHuman {
            profile_id: format!("{i:064x}"),
            session_id: format!("seat-{i}"),
            team: if i <= 5 { Team::Green } else { Team::Blue },
        })
        .collect();
    rt.match_service = crate::match_service::MatchService::Worker(Worker {
        manifest: Manifest {
            ranked_season_id: None,
            version: 1,
            allocation_id: "e".repeat(32),
            endpoint: "127.0.0.1:41000".into(),
            bind: "127.0.0.1:41000".into(),
            preference,
            humans: humans.clone(),
            join_deadline_ms: unix_ms() + 60_000,
            cohort: None,
        },
        directory: directory.clone(),
        recovery: false,
        origin_epoch: 0,
        aborted: false,
        booted_at: now,
        last_status: None,
        terminal_at: None,
        saw_human: true,
        released: Vec::new(),
    });
    for (i, human) in humans.iter().enumerate() {
        let addr = address(i as u16 + 1);
        rt.career.backend.test_authenticated(
            addr,
            shared::career::ProfileSummary::new(human.profile_id.clone(), format!("Player {i}")),
            &human.session_id,
        );
        join(&mut rt, addr, &human.session_id, now, true);
    }
    let mut roster: Vec<_> = rt
        .world
        .players
        .iter()
        .filter(|(_, p)| !p.hero.identity.is_bot)
        .map(|(addr, p)| (p.hero.identity.id, *addr))
        .collect();
    roster.sort_unstable();
    let mut next = [0, 0];
    for (_, addr) in roster {
        let p = rt.world.players.get_mut(&addr).unwrap();
        let team = if p.hero.identity.team == Team::Green {
            0
        } else {
            1
        };
        // Valid committed choices under the new per-team uniqueness rule.
        p.hero.identity.hero_class = HeroClass::ALL[next[team]];
        next[team] += 1;
        reset_player_round(p, &rt.world.map_layout, now);
        p.draft.locked = true;
        p.draft.loaded = addr != address(10);
    }
    rt.prematch.phase = Some(PrematchPhase::Loading);
    rt.prematch.deadline = Some(now + Duration::from_secs(1));
    (rt, now, directory)
}

#[test]
fn casual_loading_timeout_replaces_only_the_missing_human_and_keeps_every_other_lock() {
    let (mut rt, now, directory) =
        allocated_loading_roster(shared::match_service::MatchPreference::Casual);
    rt.tick_prematch(now + Duration::from_secs(2));
    let humans: Vec<_> = rt
        .world
        .players
        .values()
        .filter(|p| p.joined && !p.hero.identity.is_bot)
        .collect();
    assert_eq!(humans.len(), 9);
    assert!(humans.iter().all(|p| p.draft.locked && p.draft.loaded));
    assert_eq!(
        rt.world
            .players
            .values()
            .filter(|p| p.joined && p.hero.identity.is_bot)
            .count(),
        1
    );
    assert_eq!(rt.prematch.phase, Some(PrematchPhase::Loading));
    assert!(
        rt.match_service
            .worker()
            .unwrap()
            .released
            .contains(&"seat-10".into())
    );
    assert!(rt.allocated_humans_ready());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn ranked_loading_timeout_aborts_for_requeue_without_resetting_the_other_nine_picks() {
    let (mut rt, now, directory) =
        allocated_loading_roster(shared::match_service::MatchPreference::Ranked);
    rt.tick_prematch(now + Duration::from_secs(2));
    assert!(rt.match_service.worker().unwrap().aborted);
    let left: Vec<String> =
        serde_json::from_slice(&std::fs::read(directory.join("cancel.json")).unwrap()).unwrap();
    assert_eq!(left, vec!["seat-10"]);
    assert!(
        rt.world
            .players
            .values()
            .filter(|p| p.joined)
            .all(|p| p.draft.locked
                && p.draft.error.as_deref() == Some("allocation_cancelled_or_expired"))
    );
    assert_eq!(rt.match_started_at, None);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn locked_teammate_class_rejects_select_and_lock_but_opponent_can_mirror() {
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    for i in 1..=4 {
        join(&mut rt, address(i), &format!("unique-{i}"), now, true);
    }
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.team,
        rt.world.players[&address(3)].hero.identity.team
    );
    send(
        &mut rt,
        address(1),
        1,
        PrematchAction::Lock { locked: true },
        now,
    );
    send(
        &mut rt,
        address(3),
        1,
        PrematchAction::Lock { locked: true },
        now,
    );
    assert!(!rt.world.players[&address(3)].draft.locked);
    assert_eq!(
        rt.world.players[&address(3)].draft.error.as_deref(),
        Some("hero_taken")
    );
    send(
        &mut rt,
        address(3),
        2,
        select(Role::Mid, HeroClass::Warrior),
        now,
    );
    assert_eq!(
        rt.world.players[&address(3)].draft.error.as_deref(),
        Some("hero_taken")
    );
    send(
        &mut rt,
        address(3),
        3,
        select(Role::Mid, HeroClass::Mage),
        now,
    );
    send(
        &mut rt,
        address(3),
        4,
        PrematchAction::Lock { locked: true },
        now,
    );
    send(
        &mut rt,
        address(2),
        1,
        PrematchAction::Lock { locked: true },
        now,
    );
    assert!(
        rt.world.players[&address(2)].draft.locked,
        "opposing team may use Warrior"
    );
    send(
        &mut rt,
        address(1),
        2,
        PrematchAction::Lock { locked: false },
        now,
    );
    send(
        &mut rt,
        address(3),
        5,
        PrematchAction::Lock { locked: false },
        now,
    );
    send(
        &mut rt,
        address(3),
        6,
        select(Role::Solo, HeroClass::Warrior),
        now,
    );
    send(
        &mut rt,
        address(3),
        7,
        PrematchAction::Lock { locked: true },
        now,
    );
    assert!(
        rt.world.players[&address(3)].draft.locked,
        "unlock frees the class for a new request"
    );
    send(
        &mut rt,
        address(1),
        3,
        PrematchAction::Lock { locked: true },
        now,
    );
    assert_eq!(
        rt.world.players[&address(1)].draft.error.as_deref(),
        Some("hero_taken")
    );
}

#[test]
fn draft_expiry_resolves_collisions_and_preserves_a_valid_human_lock() {
    let (mut rt, now) = fixture(MatchMode::Dev, 3);
    for i in 1..=6 {
        join(&mut rt, address(i), &format!("expiry-{i}"), now, true);
    }
    send(
        &mut rt,
        address(1),
        1,
        select(Role::Mid, HeroClass::Mage),
        now,
    );
    send(
        &mut rt,
        address(1),
        2,
        PrematchAction::Lock { locked: true },
        now,
    );
    rt.world
        .players
        .get_mut(&address(5))
        .unwrap()
        .hero
        .identity
        .avatar = Some("not-a-verified-fallback".into());
    rt.tick_prematch(now + Duration::from_millis(u64::from(DRAFT_SELECTION_MS) + 1));
    assert_eq!(rt.prematch.phase, Some(PrematchPhase::Countdown));
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Mage
    );
    for team in [Team::Green, Team::Blue] {
        let mut classes = Vec::new();
        for p in rt
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.identity.team == team)
        {
            assert!(
                !classes.contains(&p.hero.identity.hero_class),
                "timeout must not bypass uniqueness"
            );
            classes.push(p.hero.identity.hero_class);
            assert!(p.draft.locked);
        }
        assert_eq!(classes.len(), 3);
    }
    let fallback = &rt.world.players[&address(5)];
    assert_ne!(fallback.hero.identity.hero_class, HeroClass::Warrior);
    assert_eq!(fallback.hero.identity.avatar, None);
    assert!(fallback.hero.identity.handheld.is_default());
    assert_eq!(fallback.draft.error.as_deref(), Some("hero_auto_picked"));
}

#[test]
fn delayed_avatar_admission_cannot_take_a_class_locked_while_it_was_pending() {
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    for i in 1..=4 {
        join(&mut rt, address(i), &format!("pending-{i}"), now, true);
    }
    let id = rt.world.players[&address(3)].hero.identity.id;
    let request = PrematchRequest {
        server_epoch: rt.server_epoch,
        match_id: rt.match_id,
        generation: rt.prematch.generation,
        request_id: 7,
        action: select(Role::Mid, HeroClass::Mage),
    };
    rt.world
        .players
        .get_mut(&address(3))
        .unwrap()
        .draft
        .request_id = 7;
    rt.prematch.pending.insert(address(3), (id, 7));
    send(
        &mut rt,
        address(1),
        1,
        select(Role::Mid, HeroClass::Mage),
        now,
    );
    send(
        &mut rt,
        address(1),
        2,
        PrematchAction::Lock { locked: true },
        now,
    );
    rt.complete_prematch_admission(address(3), request, true, now);
    let p = &rt.world.players[&address(3)];
    assert_eq!(p.hero.identity.hero_class, HeroClass::Warrior);
    assert_eq!(p.draft.error.as_deref(), Some("hero_taken"));
    assert_eq!(p.draft.acknowledged_request_id, 7);
    assert!(!p.draft.locked);
}

#[test]
fn unique_loading_replacement_invalidates_only_its_changed_asset_readiness() {
    let (mut rt, now) = fixture(MatchMode::Dev, 2);
    for i in 1..=4 {
        join(&mut rt, address(i), &format!("replacement-{i}"), now, true);
    }
    for i in [1, 2] {
        let p = rt.world.players.get_mut(&address(i)).unwrap();
        p.draft.locked = true;
        p.draft.loaded = true;
    }
    let old = rt.world.players[&address(1)].hero.identity.hero_class;
    rt.prematch.phase = Some(PrematchPhase::Loading);
    assert!(rt.finalize_unique_draft(now));
    assert_eq!(rt.world.players[&address(1)].hero.identity.hero_class, old);
    assert!(rt.world.players[&address(1)].draft.loaded);
    let replacement = &rt.world.players[&address(3)];
    assert_ne!(replacement.hero.identity.hero_class, old);
    assert!(!replacement.draft.loaded);
    assert!(replacement.draft.locked);
}

#[test]
fn bots_yield_their_class_to_the_human_and_keep_development_modifiers_visible() {
    let (mut rt, now) = fixture(MatchMode::Practice, 3);
    join(&mut rt, address(1), "human-first", now, true);
    rt.fill_practice_bots(now);
    let human_team = rt.world.players[&address(1)].hero.identity.team;
    for p in rt.world.players.values_mut().filter(|p| p.joined) {
        p.hero.identity.hero_class = HeroClass::Warrior;
        p.draft.locked = true;
    }
    rt.world
        .players
        .get_mut(&address(1))
        .unwrap()
        .modifiers
        .god_mode = true;
    assert!(!rt.prematch_class_taken(address(1), &PrematchAction::Lock { locked: true }));
    assert!(rt.finalize_unique_draft(now));
    assert_eq!(
        rt.world.players[&address(1)].hero.identity.hero_class,
        HeroClass::Warrior
    );
    assert!(rt.world.players[&address(1)].modifiers.god_mode);
    let mut classes = Vec::new();
    for p in rt
        .world
        .players
        .values()
        .filter(|p| p.joined && p.hero.identity.team == human_team)
    {
        assert!(!classes.contains(&p.hero.identity.hero_class));
        classes.push(p.hero.identity.hero_class);
    }
    assert_eq!(classes.len(), 3);
}

#[test]
fn public_draft_opt_out_is_normalized_before_the_first_durable_roster() {
    let (mut rt, now, directory) =
        allocated_loading_roster(shared::match_service::MatchPreference::Ranked);
    rt.prematch.phase = None;
    for p in rt.world.players.values_mut().filter(|p| p.joined) {
        p.draft.capable = false;
        p.draft.locked = false;
        p.hero.identity.hero_class = HeroClass::Warrior;
        p.framed_snapshots = true;
        p.protocol_compatible = true;
    }
    assert!(rt.prepare_public_unique_loadouts(now));
    for team in [Team::Green, Team::Blue] {
        let mut classes = Vec::new();
        for p in rt
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.identity.team == team)
        {
            assert!(!classes.contains(&p.hero.identity.hero_class));
            classes.push(p.hero.identity.hero_class);
        }
        assert_eq!(classes.len(), 5);
    }
    // The actual first-allocation boundary invokes the guard, even without draft opt-in.
    for p in rt.world.players.values_mut().filter(|p| p.joined) {
        p.hero.identity.hero_class = HeroClass::Warrior;
    }
    assert!(!rt.begin_career_round(now)); // waits for normal durable ACK
    for team in [Team::Green, Team::Blue] {
        let mut classes = Vec::new();
        for p in rt
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.identity.team == team)
        {
            assert!(!classes.contains(&p.hero.identity.hero_class));
            classes.push(p.hero.identity.hero_class);
        }
        assert_eq!(classes.len(), 5);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
