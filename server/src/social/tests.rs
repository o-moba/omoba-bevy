use std::io;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use shared::wire::ServerPacket;

use super::*;
use crate::match_rules::MatchConfig;
use crate::runtime::ServerRuntime;
use crate::runtime::ports::{ManualClock, MemoryTransport};

fn sender(id: u64, team: SocialTeam) -> Sender {
    Sender {
        id,
        nickname: format!("Player {id}"),
        team,
        session: format!("s{id}"),
        rate_key: format!("s{id}"),
        alive: true,
        requires_signature: false,
        entitlements: Entitlements::default(),
        mute: None,
    }
}
fn chat(id: u64, channel: SocialChannel, text: &str) -> SocialRequest {
    SocialRequest {
        request_id: id,
        server_epoch: 10,
        match_id: 1,
        session_id: "s1".into(),
        command: SocialCommand::Chat {
            channel,
            text: text.into(),
        },
    }
}
fn state(now: Instant) -> SocialRuntime {
    let mut state = SocialRuntime::default();
    state.sync_round(10, 1, now);
    state
}

#[test]
fn team_privacy_server_identity_unicode_ack_and_deduplication() {
    let now = Instant::now();
    let mut state = state(now);
    let request = chat(1, SocialChannel::Team, "  Привет 小明  ");
    state.submit(sender(1, SocialTeam::Green), request.clone(), false, now);
    state.submit(sender(1, SocialTeam::Green), request, false, now);
    let green = state.view(&sender(2, SocialTeam::Green), now);
    assert_eq!(green.events.len(), 1);
    assert_eq!(green.events[0].nickname, "Player 1");
    assert!(
        matches!(&green.events[0].kind, SocialEventKind::Chat { text, .. } if text == "Привет 小明")
    );
    assert!(
        state
            .view(&sender(3, SocialTeam::Blue), now)
            .events
            .is_empty()
    );
    assert_eq!(
        state.view(&sender(1, SocialTeam::Green), now).request_id,
        Some(1)
    );
    state.submit(
        sender(1, SocialTeam::Green),
        chat(2, SocialChannel::Match, "All"),
        false,
        now,
    );
    assert_eq!(
        state.view(&sender(3, SocialTeam::Blue), now).events.len(),
        1
    );
}

#[test]
fn chat_and_reactions_share_budget_across_reconnect_and_rematch() {
    let now = Instant::now();
    let mut state = state(now);
    for id in 1..=3 {
        state.submit(
            sender(1, SocialTeam::Green),
            chat(id, SocialChannel::Match, "Hi"),
            false,
            now,
        );
    }
    let mut reaction = chat(4, SocialChannel::Match, "");
    reaction.command = SocialCommand::Reaction {
        reaction_id: "thumbs_up".into(),
    };
    state.submit(sender(1, SocialTeam::Green), reaction.clone(), false, now);
    assert!(
        state
            .view(&sender(1, SocialTeam::Green), now)
            .error
            .unwrap()
            .contains("cooldown")
    );
    assert_eq!(state.events.len(), 3);
    reaction.request_id = 5;
    state.submit(
        sender(1, SocialTeam::Green),
        reaction,
        false,
        now + Duration::from_secs(2),
    );
    assert_eq!(state.events.len(), 4);
    state.sync_round(10, 2, now + Duration::from_secs(2));
    assert!(state.events.is_empty());
    let mut request = chat(1, SocialChannel::Match, "After rematch");
    request.match_id = 2;
    let mut reconnected = sender(99, SocialTeam::Green);
    reconnected.rate_key = "s1".into();
    reconnected.session = "s1".into();
    state.submit(reconnected, request, false, now + Duration::from_secs(2));
    assert!(
        state.events.is_empty(),
        "changing an actor/round cannot refill the account/session budget"
    );
}

#[test]
fn malformed_stale_unsigned_and_locked_reactions_never_emit() {
    let now = Instant::now();
    let mut state = state(now);
    let mut request = chat(1, SocialChannel::Match, "Hi");
    request.session_id = "wrong".into();
    state.submit(sender(1, SocialTeam::Green), request, false, now);
    let mut request = chat(2, SocialChannel::Match, "Hi");
    request.match_id = 9;
    state.submit(sender(1, SocialTeam::Green), request, false, now);
    state.submit(
        sender(1, SocialTeam::Green),
        chat(3, SocialChannel::Match, "bad\nline"),
        false,
        now,
    );
    let mut signed = sender(1, SocialTeam::Green);
    signed.requires_signature = true;
    state.submit(signed, chat(4, SocialChannel::Match, "forged"), false, now);
    let mut request = chat(5, SocialChannel::Match, "");
    request.command = SocialCommand::Reaction {
        reaction_id: "https://untrusted.test/image.png".into(),
    };
    state.submit(sender(1, SocialTeam::Green), request, false, now);
    assert!(state.events.is_empty());
    let mut request = chat(6, SocialChannel::Match, "");
    request.command = SocialCommand::Reaction {
        reaction_id: "thumbs_down".into(),
    };
    let mut dead = sender(1, SocialTeam::Green);
    dead.alive = false;
    state.submit(dead, request, false, now);
    assert!(state.events.is_empty());
    let mut signed = sender(1, SocialTeam::Green);
    signed.requires_signature = true;
    state.submit(signed, chat(7, SocialChannel::Match, "signed"), true, now);
    assert_eq!(state.events.len(), 1);
}

#[test]
fn unsigned_high_request_id_cannot_poison_signed_sequence_or_subscribe() {
    let now = Instant::now();
    let mut state = state(now);
    let mut signed = sender(1, SocialTeam::Green);
    signed.requires_signature = true;
    state.submit(
        signed,
        chat(u64::MAX, SocialChannel::Match, "forged"),
        false,
        now,
    );
    assert!(state.receipts.is_empty());
    let mut signed = sender(1, SocialTeam::Green);
    signed.requires_signature = true;
    state.submit(signed, chat(1, SocialChannel::Match, "valid"), true, now);
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        state.view(&sender(1, SocialTeam::Green), now).request_id,
        Some(1)
    );
}

#[test]
fn event_history_and_sender_maps_remain_bounded_and_age_does_not_restart() {
    let now = Instant::now();
    let mut state = state(now);
    for id in 1..=100 {
        let mut request = chat(1, SocialChannel::Match, "Hi");
        request.session_id = format!("s{id}");
        state.submit(sender(id, SocialTeam::Green), request, false, now);
    }
    assert_eq!(state.events.len(), MAX_EVENTS);
    let view = state.view(&sender(1, SocialTeam::Green), now + Duration::from_secs(2));
    assert!(view.events.iter().all(|event| event.age_ms == 2000));
    for id in 101..=600 {
        let mut request = chat(1, SocialChannel::Match, "Hi");
        request.session_id = format!("s{id}");
        state.submit(sender(id, SocialTeam::Green), request, false, now);
    }
    assert_eq!(state.receipts.len(), MAX_SENDERS);
    assert!(state.limits.len() <= MAX_SENDERS);
    state.sync_round(10, 1, now + HISTORY_TTL);
    assert!(state.events.is_empty());
}

#[test]
fn quick_rematches_do_not_reuse_completed_or_pending_social_assembly_keys() {
    let now = Instant::now();
    let mut state = state(now);
    let frames = |state: &mut SocialRuntime| {
        state.sequence += 1;
        let mut social = state.view(&sender(1, SocialTeam::Green), now);
        // A realistic multi-fragment chat history, without touching the limiter.
        social.events = (1..=12)
            .map(|id| SocialEvent {
                id,
                player_id: 1,
                nickname: "Player 1".into(),
                team: SocialTeam::Green,
                age_ms: 0,
                kind: SocialEventKind::Chat {
                    channel: SocialChannel::Match,
                    text: "你好世界".repeat(30),
                },
            })
            .collect();
        let packet = ServerPacket::Social {
            server_epoch: state.epoch,
            match_id: state.round,
            sequence: state.sequence,
            social,
        };
        let payload = serde_json::to_vec(&packet).unwrap();
        shared::transport::encode_snapshot(
            &payload,
            state.epoch,
            SOCIAL_FRAME_NAMESPACE | state.sequence,
        )
        .unwrap()
    };
    let completed_old = frames(&mut state);
    let pending_old = frames(&mut state);
    assert!(completed_old.len() > 1);
    let mut assembler = shared::transport::SnapshotAssembler::default();
    for frame in &completed_old {
        assembler.push(frame, now).unwrap();
    }
    assert!(assembler.push(&pending_old[0], now).unwrap().is_none());
    state.sync_round(10, 2, now);
    let new_round = frames(&mut state);
    let mut decoded = Vec::new();
    for index in 0..new_round.len().max(pending_old.len()) {
        for frame in [
            new_round.get(index),
            pending_old.get(index),
            completed_old.get(index),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(payload) = assembler.push(frame, now).unwrap() {
                let ServerPacket::Social { match_id, .. } =
                    serde_json::from_slice(&payload).unwrap()
                else {
                    panic!("social packet")
                };
                decoded.push(match_id);
            }
        }
    }
    decoded.sort_unstable();
    assert_eq!(
        decoded,
        vec![1, 2],
        "both delayed old data and fresh round data must assemble independently"
    );
}

#[test]
fn real_udp_filters_team_messages_and_preserves_fragmented_social_payloads() {
    let host = UdpSocket::bind("127.0.0.1:0").unwrap();
    host.set_nonblocking(true).unwrap();
    let address = host.local_addr().unwrap();
    let mut runtime = ServerRuntime::new(host, MatchConfig::dev());
    let peers: Vec<_> = (0..3)
        .map(|_| {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.connect(address).unwrap();
            socket.set_nonblocking(true).unwrap();
            socket
        })
        .collect();
    let now = Instant::now();
    for (index, peer) in peers.iter().enumerate() {
        for packet in [
            serde_json::json!({"type":"hello","protocol_version":shared::protocol::PROTOCOL_VERSION}),
            serde_json::json!({"type":"join","team":if index==2 {"blue"} else {"green"},"session_id":format!("s{index}")}),
        ] {
            peer.send(&serde_json::to_vec(&packet).unwrap()).unwrap();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.world.players.values().filter(|p| p.joined).count() < 3
        && Instant::now() < deadline
    {
        runtime.receive_packets();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        runtime.world.players.values().filter(|p| p.joined).count(),
        3
    );
    // Explicit opt-in preserves the world-only protocol of older clients.
    for (index, peer) in peers.iter().enumerate() {
        peer.send(&serde_json::to_vec(&serde_json::json!({"type":"social","request":{
            "request_id":1,"server_epoch":runtime.server_epoch,"match_id":runtime.match_id,"session_id":format!("s{index}"),
            "command":{"kind":"subscribe"}
        }})).unwrap()).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.social.receipts.len() < 3 && Instant::now() < deadline {
        runtime.receive_packets();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(runtime.social.receipts.len(), 3);
    for (id, channel) in [(2, "team"), (3, "match")] {
        peers[0].send(&serde_json::to_vec(&serde_json::json!({"type":"social","request":{
            "request_id":id,"server_epoch":runtime.server_epoch,"match_id":runtime.match_id,"session_id":"s0",
            "command":{"kind":"chat","channel":channel,"text":"你好世界".repeat(35)}
        }})).unwrap()).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.social.events.len() < 2 && Instant::now() < deadline {
        runtime.receive_packets();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(runtime.social.events.len(), 2);
    peers[0].send(&serde_json::to_vec(&serde_json::json!({"type":"social","request":{
        "request_id":4,"server_epoch":runtime.server_epoch,"match_id":runtime.match_id,"session_id":"s0",
        "command":{"kind":"reaction","reaction_id":"thumbs_up"}
    }})).unwrap()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.social.events.len() < 3 && Instant::now() < deadline {
        runtime.receive_packets();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(runtime.social.events.len(), 3);
    runtime.send_social_views(Instant::now());
    for (index, peer) in peers.iter().enumerate() {
        let mut assembler = shared::transport::SnapshotAssembler::default();
        let mut received = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut buf = [0u8; 65536];
        while received.is_none() && Instant::now() < deadline {
            match peer.recv(&mut buf) {
                Ok(size) => {
                    assert!(size <= 1200);
                    if let Some(body) = assembler.push(&buf[..size], Instant::now()).unwrap() {
                        if let Ok(ServerPacket::Social { social, .. }) =
                            serde_json::from_slice(&body)
                        {
                            received = Some(social);
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(e) => panic!("{e}"),
            }
        }
        let view = received.expect("real Social datagram");
        assert_eq!(view.events.len(), if index == 2 { 2 } else { 3 });
        assert!(view.events.iter().any(|e| matches!(&e.kind, SocialEventKind::Reaction { reaction_id } if reaction_id == "thumbs_up")), "server echoes the reaction to every eligible peer");
        assert!(view.events.iter().all(|e| {
            e.player_id
                == runtime.world.players[&peers[0].local_addr().unwrap()]
                    .hero
                    .identity
                    .id
        }));
    }
    assert!(now.elapsed() < Duration::from_secs(10));
}

/// A standalone runtime whose `seats` are joined, framed and subscribed to
/// Social, as a client's scoped Subscribe request leaves them.
fn subscribed_runtime(
    seats: &[(SocketAddr, Team)],
    now: Instant,
) -> (ServerRuntime, MemoryTransport) {
    let transport = MemoryTransport::new("127.0.0.1:4000".parse().unwrap());
    let mut rt = ServerRuntime::for_test(
        transport.clone(),
        ManualClock::new(now),
        crate::career_backend::MemoryCareer::disabled(7),
        MatchConfig::dev(),
    );
    for (index, &(addr, team)) in seats.iter().enumerate() {
        rt.world.ensure_connected(addr, now);
        let player = rt.world.players.get_mut(&addr).unwrap();
        crate::session::handle_join_request(
            player,
            team,
            shared::wire::CharacterChoice::Ipfs,
            shared::HeroClass::default(),
            None,
            &rt.world.map_layout,
            now,
        );
        player.framed_snapshots = true;
        player.career_capable = true;
        player.session_id = Some(format!("seat-{index}"));
    }
    rt.world.game_state = GameState::Running;
    for (index, &(addr, _)) in seats.iter().enumerate() {
        let request = SocialRequest {
            request_id: 1,
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            session_id: format!("seat-{index}"),
            command: SocialCommand::Subscribe,
        };
        rt.handle_social_request(addr, request, false, now);
    }
    assert_eq!(rt.social.receipts.len(), seats.len());
    (rt, transport)
}

#[test]
fn match_streams_never_target_internal_actor_addresses() {
    let now = Instant::now();
    let human: SocketAddr = "127.0.0.1:60500".parse().unwrap();
    // Where a worker parks a human seat once a bot takes it over
    // (`session_recovery`). No role may address it: the IPv4 socket fails
    // every such datagram with EAFNOSUPPORT.
    let parked: SocketAddr = "[::]:1024".parse().unwrap();
    assert!(crate::bots::is_bot_address(parked));
    let (mut rt, transport) =
        subscribed_runtime(&[(human, Team::Green), (parked, Team::Blue)], now);
    let later = now + crate::snapshot::SNAPSHOT_INTERVAL;
    rt.broadcast_snapshots(later, false);
    rt.send_career_views(later);
    rt.send_social_views(later);
    let sent = transport.take_outbound();
    let stray: Vec<_> = sent
        .iter()
        .map(|(to, _)| *to)
        .filter(|to| *to != human)
        .collect();
    assert!(
        stray.is_empty(),
        "internal actor addresses were targeted: {stray:?}"
    );
    let mut assembler = shared::transport::SnapshotAssembler::default();
    let mut streams = Vec::new();
    for (_, datagram) in &sent {
        if let Some(body) = assembler.push(datagram, later).unwrap() {
            streams.push(
                match serde_json::from_slice::<ServerPacket>(&body).unwrap() {
                    ServerPacket::Snapshot { .. } => "snapshot",
                    ServerPacket::Social { .. } => "social",
                    ServerPacket::Career { .. } => "career",
                    ServerPacket::Party { .. } => "party",
                },
            );
        }
    }
    for stream in ["snapshot", "career", "social"] {
        assert!(
            streams.contains(&stream),
            "{stream} must still reach the connected human: {streams:?}"
        );
    }
}

/// Refuses every destination, as an IPv4 socket refuses `[::]:N`.
struct RefusingTransport;
impl crate::runtime::ports::Transport for RefusingTransport {
    fn recv(&mut self, _: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        Err(io::ErrorKind::WouldBlock.into())
    }
    fn send_to(&self, _: &[u8], _: SocketAddr) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "address family not supported by protocol",
        ))
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok("127.0.0.1:4000".parse().unwrap())
    }
    fn peek(&self, _: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        Err(io::ErrorKind::WouldBlock.into())
    }
}

#[test]
fn social_send_failures_are_counted_on_their_own_limiter() {
    let now = Instant::now();
    let human: SocketAddr = "127.0.0.1:60501".parse().unwrap();
    let (mut rt, _) = subscribed_runtime(&[(human, Team::Green)], now);
    rt.transport = Box::new(RefusingTransport);
    // Four Social intervals inside one diagnostic interval: one line, then a
    // count, never one line per failed datagram.
    for step in 0..4 {
        rt.send_social_views(now + SEND_INTERVAL * step);
    }
    assert!(rt.social.send_diagnostic.last_emitted_at.is_some());
    assert_eq!(rt.social.send_diagnostic.suppressed, 3);
    assert!(
        rt.snapshot_send_diagnostic.last_emitted_at.is_none()
            && rt.snapshot_send_diagnostic.suppressed == 0,
        "Social failures must not spend the world-snapshot diagnostic budget"
    );
}

#[test]
fn a_muted_sender_is_refused_with_its_end_and_spends_no_cooldown() {
    use shared::moderation::{Sanction, SanctionKind, code};
    let now = Instant::now();
    let mut state = state(now);
    let mute = Sanction {
        kind: SanctionKind::ChatMute,
        until: Some(1_900_000_000),
    };
    let mut muted = sender(1, SocialTeam::Green);
    muted.mute = Some(mute);
    for id in 1..=5 {
        state.submit(
            muted.clone(),
            chat(id, SocialChannel::Match, "Buy gold"),
            false,
            now,
        );
    }
    let own = state.view(&muted, now);
    assert_eq!(own.error_code.as_deref(), Some(code::CHAT_MUTED));
    assert_eq!(own.mute, Some(mute), "the client says until when");
    assert!(state.events.is_empty());
    assert!(
        state
            .view(&sender(2, SocialTeam::Green), now)
            .events
            .is_empty()
    );
    // Reactions stay, and the refusals spent no shared cooldown token.
    let mut reaction = chat(6, SocialChannel::Match, "");
    reaction.command = SocialCommand::Reaction {
        reaction_id: "thumbs_up".into(),
    };
    state.submit(muted.clone(), reaction, false, now);
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.view(&muted, now).error, None);
}

/// A request from `sender(2, …)`, whose session is `s2`.
fn from_two(mut request: SocialRequest) -> SocialRequest {
    request.session_id = "s2".into();
    request
}

#[test]
fn the_chat_switch_and_a_mute_drop_held_chat_but_keep_reactions() {
    use shared::moderation::code;
    let now = Instant::now();
    let mut state = state(now);
    state.submit(
        sender(1, SocialTeam::Green),
        chat(1, SocialChannel::Match, "Before the switch"),
        false,
        now,
    );
    let mut reaction = chat(2, SocialChannel::Match, "");
    reaction.command = SocialCommand::Reaction {
        reaction_id: "heart".into(),
    };
    state.submit(sender(1, SocialTeam::Green), reaction, false, now);
    assert_eq!(state.events.len(), 2);
    // Off on this server: held chat is gone for everyone, reactions stay.
    state.set_env_chat(false);
    state.apply_moderation(false, HashSet::new());
    let view = state.view(&sender(2, SocialTeam::Blue), now);
    assert!(view.chat_disabled);
    assert_eq!(view.events.len(), 1);
    assert!(matches!(
        view.events[0].kind,
        SocialEventKind::Reaction { .. }
    ));
    state.submit(
        sender(2, SocialTeam::Blue),
        from_two(chat(1, SocialChannel::Match, "Anyone?")),
        false,
        now,
    );
    let own = state.view(&sender(2, SocialTeam::Blue), now);
    assert_eq!(own.error_code.as_deref(), Some(code::CHAT_DISABLED));
    assert_eq!(state.events.len(), 1);
    // Back on: the dropped message does not come back; new chat flows.
    state.set_env_chat(true);
    state.apply_moderation(false, HashSet::new());
    assert!(!state.view(&sender(2, SocialTeam::Blue), now).chat_disabled);
    state.submit(
        sender(2, SocialTeam::Blue),
        from_two(chat(2, SocialChannel::Match, "Back")),
        false,
        now,
    );
    assert_eq!(
        state.view(&sender(1, SocialTeam::Green), now).events.len(),
        2
    );
    // A mute drops what the player sent before it.
    state.apply_moderation(false, HashSet::from([2]));
    let events = state.view(&sender(1, SocialTeam::Green), now).events;
    assert_eq!(events.len(), 1);
    assert!(events.iter().all(|e| e.player_id != 2));
}

#[test]
fn a_moderated_runtime_delivers_no_muted_or_switched_off_chat() {
    use shared::moderation::{Sanction, SanctionKind, code};
    let now = Instant::now();
    let speaker: SocketAddr = "127.0.0.1:60610".parse().unwrap();
    let listener: SocketAddr = "127.0.0.1:60611".parse().unwrap();
    let (mut rt, _) = subscribed_runtime(&[(speaker, Team::Green), (listener, Team::Green)], now);
    // The speaker signs in to a profile, as on a public worker.
    rt.career.backend.test_authenticated(
        speaker,
        shared::career::ProfileSummary::new("a".repeat(64), "Rude#0001".into()),
        "seat-0",
    );
    let message = |rt: &ServerRuntime, id: u64, text: &str| SocialRequest {
        request_id: id,
        server_epoch: rt.server_epoch,
        match_id: rt.match_id,
        session_id: "seat-0".into(),
        command: SocialCommand::Chat {
            channel: SocialChannel::Match,
            text: text.into(),
        },
    };
    let request = message(&rt, 2, "first");
    rt.handle_social_request(speaker, request, true, now);
    assert_eq!(rt.social.events.len(), 1);
    let mute = Sanction {
        kind: SanctionKind::ChatMute,
        until: Some(crate::moderation::unix_now() + 3600),
    };
    rt.career.backend.test_set_sanctions(speaker, vec![mute]);
    let request = message(&rt, 3, "second");
    rt.handle_social_request(speaker, request, true, now);
    let heard = |rt: &ServerRuntime, addr| {
        let sender = rt.social_sender(addr).unwrap();
        rt.social.view(&sender, now)
    };
    assert!(heard(&rt, listener).events.is_empty(), "reaches no one");
    let own = heard(&rt, speaker);
    assert_eq!(own.error_code.as_deref(), Some(code::CHAT_MUTED));
    assert_eq!(own.mute, Some(mute));
    // Lifted (the next access refresh): chat flows again, no restart.
    rt.career.backend.test_set_sanctions(speaker, Vec::new());
    let request = message(&rt, 4, "third");
    rt.handle_social_request(speaker, request, true, now);
    assert_eq!(heard(&rt, listener).events.len(), 1);
    // The database switch turns chat off on a running server.
    rt.career.backend.test_set_chat_switch(Some(false));
    rt.send_social_views(now + Duration::from_secs(1));
    assert!(rt.social.events.is_empty());
    let request = message(&rt, 5, "fourth");
    rt.handle_social_request(speaker, request, true, now);
    assert!(rt.social.events.is_empty());
    assert!(heard(&rt, listener).chat_disabled);
    assert_eq!(
        heard(&rt, speaker).error_code.as_deref(),
        Some(code::CHAT_DISABLED)
    );
}
