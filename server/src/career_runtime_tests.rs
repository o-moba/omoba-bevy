//! Runtime/UDP integration with explicit authenticated worker fixtures. Crypto
//! verification still runs for signed account commands; worker I/O is controlled.
use super::*;
use shared::career::{CareerAction, CareerRequest, MatchOutcome, ProfileSummary, QueueView};

fn runtime(config: MatchConfig, career: bool) -> ServerRuntime {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, config);
    if career {
        rt.career.backend = career_backend::CareerBackend::test_backend(rt.server_epoch);
    }
    rt
}
fn addr(port: u16) -> SocketAddr {
    format!("127.0.0.1:{port}").parse().unwrap()
}
fn profile(id: u64, rating: i32, experienced: bool) -> ProfileSummary {
    let mut p = ProfileSummary::new(format!("{id:064x}"), format!("Hero {id}"));
    p.rating = rating;
    p.rated_matches = if experienced { 25 } else { 0 };
    p
}
fn join(session: &str) -> ClientPacket {
    ClientPacket::Join {
        prematch: false,
        team: Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: HeroClass::Mage,
        avatar: None,
        sprite_character: None,
        session_id: Some(session.into()),
        passport_ticket: None,
    }
}
fn authenticated_join(
    rt: &mut ServerRuntime,
    address: SocketAddr,
    id: u64,
    rating: i32,
    experienced: bool,
    now: Instant,
) {
    let session = format!("session-{id}");
    rt.career
        .backend
        .test_authenticated(address, profile(id, rating, experienced), &session);
    rt.handle_packet(address, join(&session), now);
    rt.world.players.get_mut(&address).unwrap().career_capable = true;
}
fn start(rt: &mut ServerRuntime, now: Instant) {
    rt.tick(now, 0.0);
    rt.tick(now, 3.0);
    assert!(matches!(
        rt.world.game_state,
        GameState::Starting { countdown_ms: 0 }
    ));
    let allocation = rt.career_allocation_for_test().unwrap();
    rt.career.backend.test_ack_start(&allocation.result_id);
    rt.tick(now, 0.0);
    assert_eq!(rt.world.game_state, GameState::Running);
}

#[test]
fn invalidated_account_releases_waiting_and_reserved_seats_without_stalling_the_roster() {
    for selected in [false, true] {
        let mut rt = runtime(MatchConfig::release(1), true);
        let now = Instant::now();
        let revoked = addr(59431);
        authenticated_join(&mut rt, revoked, 431, 1000, false, now);
        if selected {
            authenticated_join(&mut rt, addr(59432), 432, 1000, false, now);
            assert!(rt.career.queue.selection().is_some());
            assert_eq!(joined_count(&rt.world.players), 2);
        }
        // Worker revocation/expiry clears cached authority. This runtime test
        // exercises the exact post-invalidation cleanup, independent of SQL.
        rt.career.backend.forget(revoked);
        rt.poll_career(now);
        assert!(!rt.world.players[&revoked].joined);
        assert!(rt.career.queue.selection().is_none());
        assert_eq!(rt.career.queue.len(), usize::from(selected));
        assert_eq!(joined_count(&rt.world.players), 0);
        if selected {
            authenticated_join(&mut rt, addr(59433), 433, 1000, false, now);
            assert!(rt.career.queue.selection().is_some());
            assert_eq!(joined_count(&rt.world.players), 2);
            assert!(!rt.world.players[&revoked].joined);
        }
    }
}

#[test]
fn release_queue_uses_saved_skill_and_experience_and_waits_for_durable_allocation() {
    let mut rt = runtime(MatchConfig::release(1), true);
    let now = Instant::now();
    authenticated_join(&mut rt, addr(59301), 1, 1000, false, now);
    authenticated_join(&mut rt, addr(59302), 2, 1000, true, now);
    authenticated_join(&mut rt, addr(59303), 3, 2500, false, now);
    assert_eq!(joined_count(&rt.world.players), 0);
    assert_eq!(rt.world.game_state, GameState::Lobby);
    assert!(matches!(
        rt.career_view(addr(59301), now).queue,
        QueueView::Waiting { compatible: 1, .. }
    ));
    authenticated_join(&mut rt, addr(59304), 4, 1050, false, now);
    assert_eq!(joined_count(&rt.world.players), 2);
    assert!(!rt.world.players[&addr(59302)].joined);
    assert!(!rt.world.players[&addr(59303)].joined);
    rt.tick(now, 0.0);
    rt.tick(now, 3.0);
    let allocation = rt.career_allocation_for_test().unwrap();
    assert!(allocation.rated);
    assert_eq!(allocation.ruleset, career_runtime::RATED_RULESET);
    let before = rt.world.players[&addr(59301)].state.gold;
    for i in 1..=3 {
        rt.tick(now + Duration::from_millis(i), 0.1);
        assert_eq!(rt.world.game_state, GameState::Starting { countdown_ms: 0 });
        assert_eq!(rt.world.players[&addr(59301)].state.gold, before);
        assert!(!rt.combat_log.ledger.is_started());
        assert!(rt.world.minions.is_empty());
        assert_eq!(
            rt.career_allocation_for_test().unwrap().result_id,
            allocation.result_id
        );
    }
    rt.career.backend.test_ack_start(&allocation.result_id);
    rt.tick(now + Duration::from_millis(4), 0.0);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert_eq!(rt.combat_log.ledger.snapshot().len(), 2);
    assert_eq!(rt.career.queue.len(), 2);
    assert_eq!(rt.career_view(addr(59301), now).queue, QueueView::Playing);
}

#[test]
fn tuned_default_label_is_unrated_and_running_roster_rejects_new_players() {
    let mut rt = runtime(MatchConfig::release(1), true);
    rt.world.map_config.structures[0].stats.max_hp += 1.0;
    let now = Instant::now();
    authenticated_join(&mut rt, addr(59311), 11, 1000, false, now);
    authenticated_join(&mut rt, addr(59312), 12, 1000, false, now);
    start(&mut rt, now);
    let allocation = rt.career_allocation_for_test().unwrap();
    assert!(!allocation.rated);
    assert!(allocation.unrated_reason.unwrap().contains("Custom"));
    let original = rt.combat_log.ledger.snapshot();
    authenticated_join(&mut rt, addr(59313), 13, 1000, false, now);
    assert!(!rt.world.players[&addr(59313)].joined);
    assert_eq!(rt.combat_log.ledger.snapshot(), original);
    assert!(matches!(
        rt.career_view(addr(59313), now).queue,
        QueueView::Waiting { .. }
    ));
}

#[test]
fn frozen_result_contains_offline_totals_and_survives_round_reset_until_ack() {
    let mut rt = runtime(MatchConfig::release(1), true);
    let now = Instant::now();
    authenticated_join(&mut rt, addr(59321), 21, 1000, false, now);
    authenticated_join(&mut rt, addr(59322), 22, 1000, false, now);
    start(&mut rt, now);
    let id = rt.world.players[&addr(59321)].state.id;
    let other_id = rt.world.players[&addr(59322)].state.id;
    rt.world.players.get_mut(&addr(59321)).unwrap().state.level = 5;
    rt.combat_log.extend(
        now,
        [CombatEvent {
            source: CombatEntity {
                kind: CombatEntityKind::Player,
                id,
            },
            target: CombatEntity {
                kind: CombatEntityKind::Player,
                id: other_id,
            },
            amount: 3.0,
            killed: true,
            ..Default::default()
        }],
    );
    let later = now + PLAYER_TIMEOUT + Duration::from_secs(1);
    rt.world.players.get_mut(&addr(59322)).unwrap().last_seen = later;
    rt.maintain_roster(later);
    assert!(!rt.world.players.contains_key(&addr(59321)));
    rt.world.game_state = GameState::Victory {
        winner: Team::Green,
    };
    rt.record_match_metrics(later);
    let result = rt.career_view(addr(59322), later).last_result.unwrap();
    let offline = result
        .participants
        .iter()
        .find(|p| p.player_id == id)
        .unwrap();
    assert_eq!(
        (
            offline.stats.kills,
            offline.stats.damage_to_heroes,
            offline.stats.final_level
        ),
        (1, 3.0, 5)
    );
    assert!(offline.disconnected);
    assert!(!result.saved);
    rt.record_match_metrics(later + Duration::from_secs(1));
    assert_eq!(
        rt.career_view(addr(59322), later).last_result,
        Some(result.clone())
    );
    rt.restart_round(later);
    assert_eq!(
        rt.career_view(addr(59322), later).last_result,
        Some(result.clone())
    );
    assert!(!rt.combat_log.ledger.is_started());
    let mut acknowledged = result.clone();
    acknowledged.saved = true;
    rt.career.backend.test_ack_settle(acknowledged.clone());
    rt.poll_career(later);
    assert_eq!(
        rt.career_view(addr(59322), later).last_result,
        Some(acknowledged)
    );
}

#[test]
fn authenticated_reclaim_requires_profile_and_signed_session_preserving_round_id() {
    let mut rt = runtime(MatchConfig::release(1), true);
    let now = Instant::now();
    authenticated_join(&mut rt, addr(59331), 31, 1000, false, now);
    authenticated_join(&mut rt, addr(59332), 32, 1000, false, now);
    start(&mut rt, now);
    let id = rt.world.players[&addr(59331)].state.id;
    rt.world.players.get_mut(&addr(59331)).unwrap().state.hp = 17.0;
    let later = now + PLAYER_TIMEOUT + Duration::from_secs(1);
    rt.world.players.get_mut(&addr(59332)).unwrap().last_seen = later;
    rt.maintain_roster(later);
    rt.career
        .backend
        .test_authenticated(addr(59333), profile(33, 1000, false), "session-31");
    rt.handle_packet(addr(59333), join("session-31"), later);
    assert!(!rt.world.players[&addr(59333)].joined);
    assert!(
        rt.career_view(addr(59333), later)
            .error
            .unwrap()
            .contains("another authenticated profile")
    );
    rt.career.backend.test_authenticated(
        addr(59334),
        profile(31, 1000, false),
        "different-session",
    );
    rt.handle_packet(addr(59334), join("session-31"), later);
    assert!(!rt.world.players[&addr(59334)].joined);
    rt.career
        .backend
        .test_authenticated(addr(59335), profile(31, 1000, false), "session-31");
    rt.handle_packet(addr(59335), join("session-31"), later);
    assert_eq!(rt.world.players[&addr(59335)].state.id, id);
    assert_eq!(rt.world.players[&addr(59335)].state.hp, 17.0);
    assert_eq!(rt.combat_log.ledger.snapshot().len(), 2);
    assert!(
        !rt.combat_log
            .ledger
            .snapshot()
            .iter()
            .find(|p| p.player_id == id)
            .unwrap()
            .disconnected
    );
}

#[test]
fn practice_results_are_explicitly_unrated_and_play_again_preserves_result_for_other_player() {
    let mut rt = runtime(MatchConfig::dev(), false);
    let now = Instant::now();
    rt.handle_packet(addr(59341), join("practice-1"), now);
    rt.handle_packet(addr(59342), join("practice-2"), now);
    rt.world
        .players
        .get_mut(&addr(59341))
        .unwrap()
        .career_capable = true;
    rt.world
        .players
        .get_mut(&addr(59342))
        .unwrap()
        .career_capable = true;
    rt.world.game_state = GameState::Victory {
        winner: Team::Green,
    };
    rt.record_match_metrics(now);
    let result = rt.career_view(addr(59342), now).last_result.unwrap();
    assert_eq!(result.participants.len(), 2);
    assert!(!result.rated);
    assert!(!result.saved);
    assert!(result.participants.iter().all(|p| p.profile_id.is_none()));
    let later = now + VICTORY_REMATCH_DELAY + Duration::from_secs(1);
    for player in rt.world.players.values_mut() {
        player.last_seen = later;
    }
    rt.tick(later, 0.1);
    assert!(matches!(rt.world.game_state, GameState::Victory { .. }));
    rt.handle_packet(addr(59341), ClientPacket::RequestRematch, later);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(rt.world.players[&addr(59341)].joined);
    assert!(!rt.world.players[&addr(59342)].joined);
    assert!(rt.career_view(addr(59341), later).last_result.is_none());
    assert_eq!(rt.career_view(addr(59342), later).last_result, Some(result));
}

fn send(socket: &UdpSocket, rt: &mut ServerRuntime, packet: ClientPacket) {
    socket
        .send_to(
            &serde_json::to_vec(&packet).unwrap(),
            rt.socket.local_addr().unwrap(),
        )
        .unwrap();
    // Nonblocking receive may observe WouldBlock immediately after send_to,
    // even on loopback. Wait for kernel delivery without consuming or injecting
    // the datagram; the production decoder/handler still processes it exactly once.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut ready = [0_u8; MAX_CLIENT_REQUEST_PAYLOAD_BYTES];
    loop {
        match rt.socket.peek_from(&mut ready) {
            Ok((_, sender)) => {
                assert_eq!(sender, socket.local_addr().unwrap());
                rt.receive_packets();
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "UDP request was not delivered before the deadline"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("UDP readiness check failed: {error}"),
        }
    }
}

#[test]
fn live_udp_signed_cancellation_and_result_packet_order_use_real_receiver() {
    use ed25519_dalek::Signer;
    let mut rt = runtime(MatchConfig::release(1), true);
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    rt.career
        .backend
        .test_authenticated(address, profile(50, 1000, false), "signed-queue");
    send(
        &socket,
        &mut rt,
        ClientPacket::Hello {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
        },
    );
    send(&socket, &mut rt, join("signed-queue"));
    let id = rt.world.players[&address].state.id;
    assert_eq!(rt.career.queue.len(), 1);
    // A forged unsigned cancellation cannot mutate even a correctly addressed queue entry.
    send(
        &socket,
        &mut rt,
        ClientPacket::Career {
            request: CareerRequest::CancelQueue,
        },
    );
    assert_eq!(rt.career.queue.len(), 1);
    // Reset only the test worker's request throttle, retaining the same authenticated identity.
    rt.career
        .backend
        .test_authenticated(address, profile(50, 1000, false), "signed-queue");
    let nonce = "b".repeat(64);
    let bytes = shared::career::authorized_signing_bytes(
        rt.server_epoch,
        &nonce,
        1,
        &CareerAction::CancelQueue,
    );
    let signature = ed25519_dalek::SigningKey::from_bytes(&[7; 32])
        .sign(&bytes)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    send(
        &socket,
        &mut rt,
        ClientPacket::Career {
            request: CareerRequest::Authorized {
                session_nonce: nonce,
                sequence: 1,
                action: CareerAction::CancelQueue,
                signature,
            },
        },
    );
    assert_eq!(rt.career.queue.len(), 0);
    assert_eq!(
        rt.career_view(address, Instant::now()).queue,
        QueueView::Idle
    );
    assert!(!rt.world.players[&address].joined);
    assert_eq!(rt.world.players[&address].state.id, id);
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut buf = [0; 65_507];
    let mut sequences = Vec::new();
    let mut assembler = shared::transport::SnapshotAssembler::default();
    for _ in 0..2 {
        let payload = loop {
            let (len, _) = socket.recv_from(&mut buf).unwrap();
            assert!(len <= shared::transport::MAX_DATAGRAM_BYTES);
            if let Some(payload) = assembler.push(&buf[..len], Instant::now()).unwrap() {
                break payload;
            }
        };
        let ServerPacket::Career {
            server_epoch,
            sequence,
            career,
        } = serde_json::from_slice(&payload).unwrap()
        else {
            panic!("career response")
        };
        assert_eq!(server_epoch, rt.server_epoch);
        assert_eq!(career.profile.unwrap().profile_id, format!("{:064x}", 50));
        sequences.push(sequence);
    }
    assert!(sequences[1] > sequences[0]);
}

#[test]
fn empty_roster_finalizes_abandoned_before_reset_without_ranked_winner() {
    let mut rt = runtime(MatchConfig::dev(), false);
    let now = Instant::now();
    rt.handle_packet(addr(59361), join("alone"), now);
    let id = rt.world.players[&addr(59361)].state.id;
    let later = now + PLAYER_TIMEOUT + Duration::from_secs(1);
    rt.maintain_roster(later);
    let ended = later + EMPTY_ROSTER_GRACE;
    rt.maintain_roster(ended);
    assert_eq!(rt.match_id, 2);
    assert_eq!(rt.world.game_state, GameState::Lobby);
    // Retained final DTO is available by the old participant's retained identity.
    rt.world.ensure_connected(addr(59362), ended);
    rt.world.players.get_mut(&addr(59362)).unwrap().state.id = id;
    let result = rt.career_view(addr(59362), ended).last_result.unwrap();
    assert_eq!(result.outcome, MatchOutcome::Abandoned);
    assert_eq!(result.winner, None);
    assert!(!result.rated);
    assert!(result.participants[0].disconnected);
}

#[test]
fn postgres_live_udp_signed_profiles_queue_real_cast_and_durable_history() {
    use ed25519_dalek::Signer;
    let Ok(url) = std::env::var("OMOBA_TEST_DATABASE_URL") else {
        eprintln!("SKIP postgres_live_udp: OMOBA_TEST_DATABASE_URL is not configured");
        return;
    };
    // Fixture administration precedes the runtime, which intentionally cannot
    // create schema. This makes the test independent of other test ordering.
    let admin = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let prepared = admin
        .block_on(career_store::CareerStore::connect(&url))
        .expect("prepare isolated career schema");
    drop(prepared);
    drop(admin);
    let mut rt = runtime(MatchConfig::release(1), false);
    let outbox = std::env::temp_dir().join(format!(
        "omoba-career-udp-{}-{}",
        std::process::id(),
        rt.server_epoch
    ));
    rt.career.backend =
        career_backend::CareerBackend::test_with_database(rt.server_epoch, url, outbox);
    let sockets = [
        UdpSocket::bind("127.0.0.1:0").unwrap(),
        UdpSocket::bind("127.0.0.1:0").unwrap(),
    ];
    let mut keys = Vec::new();
    for (index, socket) in sockets.iter().enumerate() {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let public_key: String = key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let address = socket.local_addr().unwrap();
        let session = format!("live-signed-{index}");
        send(
            socket,
            &mut rt,
            ClientPacket::Career {
                request: CareerRequest::Challenge {
                    public_key,
                    nickname: format!("UDP Hero {index}"),
                    session_id: session,
                },
            },
        );
        let challenge = rt.career.backend.view(address).challenge.unwrap();
        std::thread::sleep(Duration::from_millis(110));
        let signature = key
            .sign(&challenge.signing_bytes())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        send(
            socket,
            &mut rt,
            ClientPacket::Career {
                request: CareerRequest::Authenticate {
                    challenge,
                    signature,
                },
            },
        );
        pump_worker(&mut rt, &sockets, |rt| {
            rt.career.backend.profile(address).is_some()
        });
        keys.push(key);
    }
    for (index, socket) in sockets.iter().enumerate() {
        send(socket, &mut rt, join(&format!("live-signed-{index}")));
    }
    let now = Instant::now();
    rt.tick(now, 0.0);
    rt.tick(now, 3.0);
    let allocation = rt.career_allocation_for_test().unwrap();
    assert!(allocation.rated);
    assert!(matches!(
        rt.world.game_state,
        GameState::Starting { countdown_ms: 0 }
    ));
    pump_worker(&mut rt, &sockets, |rt| {
        rt.career.backend.started(&allocation.result_id)
    });
    rt.tick(Instant::now(), 0.0);
    assert_eq!(rt.world.game_state, GameState::Running);
    let (attacker_addr, attacker_id) = rt
        .world
        .players
        .iter()
        .find(|(_, p)| p.joined && p.state.team == Team::Green)
        .map(|(addr, p)| (*addr, p.state.id))
        .unwrap();
    let attacker_index = sockets
        .iter()
        .position(|s| s.local_addr().unwrap() == attacker_addr)
        .unwrap();
    let base = rt
        .world
        .structures
        .values()
        .find(|s| s.state.team == Team::Blue && s.state.kind == StructureKind::BaseTower)
        .unwrap()
        .state
        .clone();
    // Accelerated siege fixture: a legal real cast delivers the final accepted
    // HP loss through receiver -> projectile simulation -> ledger -> database.
    for structure in rt.world.structures.values_mut() {
        structure.attack_range = 0.0;
        if structure.state.kind == StructureKind::Tower {
            structure.state.hp = 0.0;
        }
    }
    rt.world.structures.get_mut(&base.id).unwrap().state.hp = 1.0;
    let attacker = rt.world.players.get_mut(&attacker_addr).unwrap();
    attacker.state.x = base.x - 2.0;
    attacker.state.z = base.z;
    send(
        &sockets[attacker_index],
        &mut rt,
        ClientPacket::Cast {
            target: TargetId {
                kind: TargetKind::Structure,
                id: base.id,
            },
            slot: 0,
        },
    );
    assert_eq!(rt.world.projectiles.len(), 1);
    std::thread::sleep(Duration::from_millis(300));
    rt.tick(Instant::now(), 0.3);
    assert_eq!(
        rt.world.game_state,
        GameState::Victory {
            winner: Team::Green
        }
    );
    let result = rt
        .career_view(attacker_addr, Instant::now())
        .last_result
        .unwrap();
    assert!(!result.saved);
    let stats = &result
        .participants
        .iter()
        .find(|p| p.player_id == attacker_id)
        .unwrap()
        .stats;
    assert_eq!(
        (stats.damage_to_structures, stats.structures_destroyed),
        (1.0, 1)
    );
    pump_worker(&mut rt, &sockets, |rt| {
        rt.career_view(attacker_addr, Instant::now())
            .last_result
            .is_some_and(|r| r.saved)
    });
    let saved = rt
        .career_view(attacker_addr, Instant::now())
        .last_result
        .unwrap();
    assert_eq!(saved.result_id, result.result_id);
    assert!(saved.participants.iter().all(|p| p.rating.is_some()));
    let view = rt.career.backend.view(attacker_addr);
    let nonce = view.auth_nonce.unwrap();
    let action = CareerAction::History {
        request_id: 77,
        before: None,
    };
    let bytes = shared::career::authorized_signing_bytes(rt.server_epoch, &nonce, 1, &action);
    let signature = keys[attacker_index]
        .sign(&bytes)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    send(
        &sockets[attacker_index],
        &mut rt,
        ClientPacket::Career {
            request: CareerRequest::Authorized {
                session_nonce: nonce,
                sequence: 1,
                action,
                signature,
            },
        },
    );
    pump_worker(&mut rt, &sockets, |rt| {
        rt.career.backend.view(attacker_addr).history_loaded
    });
    let history = rt.career.backend.view(attacker_addr);
    assert_eq!(history.response_id, Some(77));
    assert!(
        history
            .history
            .iter()
            .any(|entry| entry.result_id == saved.result_id)
    );
    let target = rt
        .career
        .backend
        .profile(sockets[1 - attacker_index].local_addr().unwrap())
        .unwrap();
    let nonce = rt.career.backend.view(attacker_addr).auth_nonce.unwrap();
    let action = CareerAction::LookupPlayer {
        request_id: 78,
        handle: target.nickname.to_uppercase(),
    };
    // Separate signed account actions must respect the worker's 100 ms throttle.
    std::thread::sleep(Duration::from_millis(110));
    let bytes = shared::career::authorized_signing_bytes(rt.server_epoch, &nonce, 2, &action);
    let signature = keys[attacker_index]
        .sign(&bytes)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    send(
        &sockets[attacker_index],
        &mut rt,
        ClientPacket::Career {
            request: CareerRequest::Authorized {
                session_nonce: nonce,
                sequence: 2,
                action,
                signature,
            },
        },
    );
    pump_worker(&mut rt, &sockets, |rt| {
        rt.career.backend.view(attacker_addr).response_id == Some(78)
    });
    let found = rt.career.backend.view(attacker_addr).found_player.unwrap();
    assert_eq!(found.profile_id, target.profile_id);
    assert_eq!(found.nickname, target.nickname);
    for socket in &sockets {
        let address = socket.local_addr().unwrap();
        let profile = rt.career.backend.profile(address).unwrap();
        let settled = saved
            .participants
            .iter()
            .find(|p| p.profile_id.as_deref() == Some(profile.profile_id.as_str()))
            .unwrap();
        assert_eq!(profile.rating, settled.rating.as_ref().unwrap().after);
        assert_eq!(
            profile.rated_matches, 1,
            "saved ACK includes the new experience count"
        );
        send(socket, &mut rt, ClientPacket::RequestRematch);
    }
    let next = rt.career.queue.selection().unwrap();
    for selected in &next.participants {
        let settled = saved
            .participants
            .iter()
            .find(|p| p.profile_id.as_deref() == Some(selected.waiting.profile.profile_id.as_str()))
            .unwrap();
        assert_eq!(
            selected.waiting.profile.rating,
            settled.rating.as_ref().unwrap().after
        );
        assert_eq!(selected.waiting.profile.rated_matches, 1);
    }
    println!(
        "LIVE_CAREER_UDP signed_auth=2 queue=balanced allocation=durable real_cast_damage=1 terminal=green saved_ack=true history=true next_queue=fresh_rating_and_experience"
    );
}

fn pump_worker(
    rt: &mut ServerRuntime,
    sockets: &[UdpSocket],
    ready: impl Fn(&ServerRuntime) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        for socket in sockets {
            send(socket, rt, ClientPacket::Ping);
        }
        rt.poll_career(Instant::now());
        if ready(rt) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "career worker did not finish: {:?}",
            sockets
                .iter()
                .map(|socket| rt.career.backend.view(socket.local_addr().unwrap()).error)
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn permanent_allocation_conflict_releases_roster_and_requires_signed_retry_gesture() {
    use ed25519_dalek::Signer;
    let mut rt = runtime(MatchConfig::release(1), true);
    let now = Instant::now();
    let first = addr(59371);
    authenticated_join(&mut rt, first, 71, 1000, false, now);
    authenticated_join(&mut rt, addr(59372), 72, 1000, false, now);
    rt.tick(now, 0.0);
    rt.tick(now, 3.0);
    let allocation = rt.career_allocation_for_test().unwrap();
    rt.career.backend.test_reject_start(
        &allocation.result_id,
        "This profile already has an active assignment.",
    );
    rt.poll_career(now);
    assert_eq!(rt.world.game_state, GameState::Lobby);
    assert_eq!(joined_count(&rt.world.players), 0);
    assert_eq!(rt.career.queue.len(), 0);
    assert!(rt.career_allocation_for_test().is_none());
    assert!(
        rt.career_view(first, now)
            .error
            .unwrap()
            .contains("Leave queue")
    );
    assert!(rt.career_view(first, now).last_result.is_none());
    let match_id = rt.match_id;
    for _ in 0..20 {
        rt.handle_packet(first, join("session-71"), now);
    }
    assert_eq!(rt.career.queue.len(), 0);
    assert_eq!(rt.match_id, match_id);
    let nonce = "b".repeat(64);
    let bytes = shared::career::authorized_signing_bytes(
        rt.server_epoch,
        &nonce,
        1,
        &CareerAction::CancelQueue,
    );
    let signature = ed25519_dalek::SigningKey::from_bytes(&[7; 32])
        .sign(&bytes)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    rt.handle_packet(
        first,
        ClientPacket::Career {
            request: CareerRequest::Authorized {
                session_nonce: nonce,
                sequence: 1,
                action: CareerAction::CancelQueue,
                signature,
            },
        },
        now,
    );
    assert!(rt.career_view(first, now).error.is_none());
    rt.handle_packet(first, join("session-71"), now);
    assert_eq!(rt.career.queue.len(), 1);
    assert!(matches!(
        rt.career_view(first, now).queue,
        QueueView::Waiting { .. }
    ));
}

fn transport_result() -> shared::career::MatchResult {
    let mut rt = runtime(MatchConfig::dev(), false);
    let now = Instant::now();
    rt.handle_packet(addr(59490), join("transport-template"), now);
    let mut result = rt.career_allocation_for_test().unwrap();
    result.outcome = MatchOutcome::Completed;
    result.winner = Some(shared::map::Team::Green);
    result.ended_at_ms = result.started_at_ms + 1000;
    result.duration_ms = 1000;
    result.saved = true;
    let template = result.participants[0].clone();
    result.participants = (1..=32)
        .map(|id| {
            let mut p = template.clone();
            p.player_id = id;
            p.profile_id = Some(format!("{id:064x}"));
            p.nickname = "𠀀".repeat(20);
            p.team = if id <= 16 {
                shared::map::Team::Green
            } else {
                shared::map::Team::Blue
            };
            p.stats.damage_to_heroes = f64::MAX;
            p.stats.damage_to_creeps = f64::MAX;
            p.stats.damage_to_structures = f64::MAX;
            p.stats.damage_taken = f64::MAX;
            p.stats.kills = u32::MAX;
            p.stats.deaths = u32::MAX;
            p.stats.assists = u32::MAX;
            p
        })
        .collect();
    result
}

fn transport_friends(count: usize) -> shared::career::FriendsView {
    shared::career::FriendsView {
        friends: (0..count)
            .map(|index| {
                let mut p = profile(100 + index as u64, 5000, true);
                p.nickname = "𠀀".repeat(20);
                p.rated_matches = u32::MAX;
                p.matches_played = u32::MAX;
                p.wins = u32::MAX;
                p.losses = u32::MAX;
                p.progression_xp = u64::MAX;
                shared::career::FriendProfile {
                    profile: p,
                    presence: shared::career::FriendPresence::Playing,
                }
            })
            .collect(),
        ..Default::default()
    }
}

fn decoded_career_frames(frames: &[Vec<u8>]) -> (usize, shared::career::CareerView) {
    let mut assembler = shared::transport::SnapshotAssembler::default();
    let mut complete = None;
    for frame in frames.iter().rev() {
        assert!(frame.len() <= shared::transport::MAX_DATAGRAM_BYTES);
        if let Some(payload) = assembler.push(frame, Instant::now()).unwrap() {
            assert!(complete.is_none());
            complete = Some(payload);
        }
    }
    let payload = complete.expect("all career fragments reassemble");
    assert!(payload.len() <= shared::transport::MAX_SNAPSHOT_BYTES);
    let ServerPacket::Career { career, .. } = serde_json::from_slice(&payload).unwrap() else {
        panic!("career envelope")
    };
    (payload.len(), career)
}

#[test]
fn live_udp_full_roster_career_result_exceeds_9kb_and_arrives_in_small_fragments() {
    let mut rt = runtime(
        MatchConfig {
            mode: MatchMode::Dev,
            team_size: 16,
        },
        false,
    );
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = client.local_addr().unwrap();
    let now = Instant::now();
    for index in 0..32 {
        let endpoint = if index == 0 {
            address
        } else {
            format!("127.0.0.2:{}", 59400 + index).parse().unwrap()
        };
        rt.handle_packet(endpoint, join(&format!("full-roster-{index}")), now);
    }
    rt.world.players.get_mut(&address).unwrap().career_capable = true;
    rt.world.game_state = GameState::Victory {
        winner: Team::Green,
    };
    rt.record_match_metrics(now);
    let expected = rt.career_view(address, now).last_result.unwrap();
    assert_eq!(expected.participants.len(), 32);
    let epoch = rt.server_epoch;
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
    let receive = std::thread::spawn(move || {
        let mut assembler = shared::transport::SnapshotAssembler::default();
        let mut buf = [0_u8; shared::transport::MAX_DATAGRAM_BYTES + 1];
        let mut fragments = 0;
        ready_tx.send(()).unwrap();
        loop {
            let (len, _) = client.recv_from(&mut buf).unwrap();
            assert!(len <= shared::transport::MAX_DATAGRAM_BYTES);
            fragments += 1;
            if let Some(payload) = assembler.push(&buf[..len], Instant::now()).unwrap() {
                assert!(
                    payload.len() > 9 * 1024,
                    "exercise the platform's large-datagram failure boundary"
                );
                assert!(fragments > 1);
                let ServerPacket::Career {
                    server_epoch,
                    sequence,
                    career,
                } = serde_json::from_slice(&payload).unwrap()
                else {
                    panic!("career response")
                };
                assert_eq!(server_epoch, epoch);
                assert_eq!(
                    sequence, 1,
                    "framing namespace does not alter logical ordering"
                );
                return career.last_result.unwrap();
            }
        }
    });
    ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    rt.send_career_views(now);
    assert_eq!(receive.join().unwrap(), expected);
}

#[test]
fn career_and_world_fragments_with_equal_logical_sequence_do_not_collide() {
    let world = vec![b'w'; 12_000];
    let world_frames = shared::transport::encode_snapshot(&world, 19, 1).unwrap();
    let career_frames = career_runtime::encode_career_datagrams(
        shared::career::CareerView {
            last_result: Some(transport_result()),
            ..Default::default()
        },
        19,
        1,
    )
    .unwrap();
    let mut assembler = shared::transport::SnapshotAssembler::default();
    let mut outputs = Vec::new();
    for index in (0..world_frames.len().max(career_frames.len())).rev() {
        for frame in [world_frames.get(index), career_frames.get(index)]
            .into_iter()
            .flatten()
        {
            if let Some(payload) = assembler.push(frame, Instant::now()).unwrap() {
                outputs.push(payload);
            }
        }
    }
    assert_eq!(outputs.len(), 2);
    assert!(outputs.contains(&world));
    let career = outputs.iter().find(|payload| *payload != &world).unwrap();
    let ServerPacket::Career {
        sequence, career, ..
    } = serde_json::from_slice(career).unwrap()
    else {
        panic!("career response")
    };
    assert_eq!(sequence, 1);
    assert_eq!(career.last_result.unwrap().participants.len(), 32);
}

#[test]
fn normative_32_player_results_64_full_names_and_history_page_fit_the_transport_budget() {
    use shared::career::{CareerView, MatchSummary};
    let result = transport_result();
    let history: Vec<_> = (0..shared::career::HISTORY_PAGE_SIZE)
        .map(|index| MatchSummary {
            result_id: format!("history-{index}"),
            ended_at_ms: result.ended_at_ms,
            duration_ms: u64::MAX,
            outcome: MatchOutcome::Completed,
            won: Some(true),
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            kills: u32::MAX,
            deaths: u32::MAX,
            assists: u32::MAX,
            damage_to_heroes: f64::MAX,
            rating: None,
        })
        .collect();
    for view in [
        CareerView {
            response_id: Some(81),
            friends: Some(transport_friends(64)),
            ..Default::default()
        },
        CareerView {
            response_id: Some(82),
            history,
            history_loaded: true,
            ..Default::default()
        },
        CareerView {
            response_id: Some(83),
            detail: Some(result.clone()),
            ..Default::default()
        },
    ] {
        let (_, single) = decoded_career_frames(
            &career_runtime::encode_career_datagrams(view.clone(), 19, 1).unwrap(),
        );
        assert_eq!(
            single, view,
            "an individual normative query is never truncated"
        );
        let mut combined = view.clone();
        combined.last_result = Some(result.clone());
        let (_, sent) = decoded_career_frames(
            &career_runtime::encode_career_datagrams(combined.clone(), 19, 2).unwrap(),
        );
        assert_eq!(
            sent, combined,
            "normal query plus current result fits together"
        );
    }
}

#[test]
fn oversized_combined_career_views_alternate_without_losing_rows_and_single_oversize_is_an_error() {
    let mut current = transport_result();
    // Maximum accepted stored cosmetic string lengths; escaped bytes exercise
    // serialization expansion beyond ordinary shipped cosmetic slugs.
    for p in &mut current.participants {
        p.avatar = Some("\"".repeat(256));
        p.sprite_character = Some("\"".repeat(256));
    }
    let mut detail = current.clone();
    detail.result_id = "another-match".into();
    let view = shared::career::CareerView {
        response_id: Some(90),
        detail: Some(detail.clone()),
        last_result: Some(current.clone()),
        ..Default::default()
    };
    assert!(serde_json::to_vec(&view).unwrap().len() > shared::transport::MAX_SNAPSHOT_BYTES);
    let (_, first) = decoded_career_frames(
        &career_runtime::encode_career_datagrams(view.clone(), 19, 1).unwrap(),
    );
    let (_, second) =
        decoded_career_frames(&career_runtime::encode_career_datagrams(view, 19, 2).unwrap());
    assert_eq!(first.last_result, Some(current));
    assert!(first.detail.is_none());
    assert_eq!(first.response_id, None);
    assert_eq!(second.detail, Some(detail));
    assert!(second.last_result.is_none());
    assert_eq!(second.response_id, Some(90));
    assert!(first.error.is_none() && second.error.is_none());
    let (_, error) = decoded_career_frames(
        &career_runtime::encode_career_datagrams(
            shared::career::CareerView {
                response_id: Some(91),
                friends: Some(transport_friends(256)),
                ..Default::default()
            },
            19,
            2,
        )
        .unwrap(),
    );
    assert_eq!(error.response_id, Some(91));
    assert!(error.friends.is_none());
    assert!(!error.loading);
    assert!(error.error.unwrap().contains("response size"));
}
