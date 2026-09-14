use super::*;

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
    while runtime.players.values().filter(|p| p.joined).count() < 3 && Instant::now() < deadline {
        runtime.receive_packets();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(runtime.players.values().filter(|p| p.joined).count(), 3);
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
        assert_eq!(view.events.len(), if index == 2 { 1 } else { 2 });
        assert!(
            view.events
                .iter()
                .all(|e| e.player_id == runtime.players[&peers[0].local_addr().unwrap()].state.id)
        );
    }
    assert!(now.elapsed() < Duration::from_secs(10));
}
