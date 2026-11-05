use super::*;
use std::net::SocketAddr;

fn clock() -> Instant {
    Instant::now() + Duration::from_secs(1000)
}
fn waiter(
    id: u64,
    preference: MatchPreference,
    rating: i32,
    newcomer: bool,
    age: u64,
    now: Instant,
) -> Waiting {
    Waiting {
        profile: ProfileSummary {
            completed_public_matches: 10,
            rating,
            rated_matches: if newcomer { 0 } else { 20 },
            ..ProfileSummary::new(format!("{id:064x}"), "Fixture".into())
        },
        session: format!("s{id}"),
        preference,
        queued: now - Duration::from_secs(age),
        touched: now,
        order: id,
        party: None,
        casual_since: (preference == MatchPreference::Casual)
            .then_some(now - Duration::from_secs(age)),
    }
}
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

#[test]
fn shared_views_match_the_independent_legacy_counters_for_mixed_queues_and_parties() {
    let now = clock();
    let mut rng = Rng(0x719abc);
    for case in 0..128 {
        let mut waiting = Vec::new();
        for group in 0..1 + rng.next() as usize % 10 {
            let size = 1 + rng.next() as usize % 5;
            let preference = [
                MatchPreference::Ranked,
                MatchPreference::Casual,
                MatchPreference::BotPractice,
            ][rng.next() as usize % 3];
            let rating = 700 + (rng.next() % 1800) as i32;
            let newcomer = rng.next().is_multiple_of(2);
            let age =
                [0, 29, 30, 59, 60, 119, 120, 180, 240, 299, 300, 600][rng.next() as usize % 12];
            for member in 0..size {
                let id = waiting.len() as u64 + 1;
                let mut entry = waiter(
                    id,
                    preference,
                    rating + member as i32 * 20,
                    newcomer,
                    age,
                    now,
                );
                if preference == MatchPreference::Ranked
                    && size > 1
                    && member + 1 == size
                    && case % 7 == 0
                {
                    entry.profile.rated_matches = if newcomer { 20 } else { 0 };
                }
                if size > 1 {
                    entry.party = Some((
                        group as u64 + 1,
                        size + usize::from(case % 4 == 0 && size < 5),
                    ));
                }
                if preference == MatchPreference::Ranked && rng.next().is_multiple_of(2) {
                    entry.casual_since = Some(now - Duration::from_secs(10));
                }
                waiting.push(entry);
            }
        }
        let snapshot = Snapshot::new(&waiting, now);
        assert!(snapshot.windows.len() <= 9);
        for index in 0..waiting.len() {
            let line = Some(CapacityLine {
                position: index as u32 + 1,
                eta_secs: Some(5),
            });
            assert_eq!(
                snapshot.view(&waiting, index, line, now),
                super::super::waiting_view(&waiting, index, line, now),
                "case {case}, entry {index}"
            );
        }
    }
}

#[test]
fn cached_windows_keep_elapsed_and_widening_boundaries_dynamic() {
    let now = clock();
    let waiting: Vec<_> = (0..40)
        .map(|i| {
            waiter(
                i + 1,
                MatchPreference::Ranked,
                700 + i as i32 * 25,
                i % 2 == 0,
                0,
                now,
            )
        })
        .collect();
    let snapshot = Snapshot::new(&waiting, now);
    for age in [0, 119, 120, 179, 180, 239, 240, 299, 300, 600] {
        let at = now + Duration::from_secs(age);
        for index in 0..waiting.len() {
            assert_eq!(
                snapshot.view(&waiting, index, None, at),
                super::super::waiting_view(&waiting, index, None, at),
                "age {age}, index {index}"
            );
        }
    }
}

#[test]
fn range_counts_are_wide_at_integer_limits_and_validate_stale_membership_indices() {
    let now = clock();
    let mut waiting: Vec<_> = (0..20)
        .map(|i| {
            waiter(
                i + 1,
                MatchPreference::Ranked,
                if i < 10 {
                    i32::MIN + i as i32
                } else {
                    i32::MAX - i as i32
                },
                true,
                300,
                now,
            )
        })
        .collect();
    let snapshot = Snapshot::new(&waiting, now);
    for i in 0..waiting.len() {
        assert_eq!(
            snapshot.view(&waiting, i, None, now),
            super::super::waiting_view(&waiting, i, None, now)
        );
    }
    let first = waiting[0].profile.profile_id.clone();
    waiting.swap(0, 1);
    assert_eq!(snapshot.index(&waiting, &first), Some(1));
    waiting.remove(1);
    assert_eq!(snapshot.index(&waiting, &first), None);
    waiting.push(waiter(99, MatchPreference::Ranked, 1000, true, 0, now));
    let id = &waiting.last().unwrap().profile.profile_id;
    assert_eq!(snapshot.index(&waiting, id), Some(waiting.len() - 1));
}

#[test]
fn gathering_names_other_queue_members_opt_outs_and_deadline_use_the_shared_party() {
    let now = clock();
    let mut leader = waiter(1, MatchPreference::BotPractice, 1000, true, 0, now);
    leader.party = Some((7, 3));
    let mut friend = waiter(2, MatchPreference::Casual, 1000, true, 0, now);
    friend.party = Some((7, 3));
    let waiting = vec![leader, friend];
    let roster = vec![
        (waiting[0].profile.profile_id.clone(), "Leader".into()),
        (waiting[1].profile.profile_id.clone(), "Other queue".into()),
        ("missing".into(), "Late friend".into()),
    ];
    let snapshot = Snapshot::new(&waiting, now);
    let gathering = snapshot
        .gathering(&waiting[0], Some(&roster), None, now)
        .unwrap();
    assert_eq!(gathering.missing, ["Late friend"]);
    assert_eq!(gathering.needed, 1);
    assert_eq!(gathering.remaining_secs, 60);
    let out = HashMap::from([("missing".into(), now)]);
    assert!(
        snapshot
            .gathering(&waiting[0], Some(&roster), Some(&out), now)
            .unwrap()
            .missing
            .is_empty()
    );
    assert!(
        snapshot
            .gathering(&waiting[0], Some(&roster), None, now + PARTY_GATHER_WINDOW)
            .is_none()
    );
}

#[test]
fn forced_broadcasts_share_queue_data_independently_of_the_worker_poll_clock() {
    let now = clock();
    let pool = crate::match_pool::Pool::for_test();
    let directory = pool.test_directory().to_owned();
    let mut service = MatchService::Lobby(Lobby::new(pool, Limits::default()));
    if let MatchService::Lobby(lobby) = &mut service {
        lobby
            .waiting
            .push(waiter(1, MatchPreference::Ranked, 1000, true, 0, now));
    }
    service.refresh_handoff_views(now);
    if let MatchService::Lobby(lobby) = &mut service {
        assert_eq!(lobby.queue_views.ranked, 1);
        lobby
            .waiting
            .push(waiter(2, MatchPreference::Ranked, 1000, true, 0, now));
        lobby.last_handoff_refresh = Some(now + Duration::from_millis(200));
    }
    service.refresh_handoff_views(now + Duration::from_millis(250));
    if let MatchService::Lobby(lobby) = &mut service {
        assert_eq!(
            lobby.queue_views.ranked, 2,
            "a worker poll cannot delay the queue snapshot another250ms"
        );
        lobby
            .waiting
            .push(waiter(3, MatchPreference::Ranked, 1000, true, 0, now));
    }
    service.refresh_handoff_views(now + Duration::from_millis(251));
    let MatchService::Lobby(lobby) = &service else {
        unreachable!()
    };
    assert_eq!(
        lobby.queue_views.ranked, 2,
        "extra packets do not rebuild the snapshot"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn occupied_lobby(now: Instant) -> (Lobby, std::path::PathBuf, String) {
    let mut pool = crate::match_pool::Pool::for_test();
    let directory = pool.test_directory().to_owned();
    let id = pool
        .allocate(Manifest {
            version: 1,
            allocation_id: "a".repeat(32),
            endpoint: String::new(),
            bind: String::new(),
            preference: MatchPreference::BotPractice,
            humans: vec![AllocatedHuman {
                profile_id: "f".repeat(64),
                session_id: "owner".into(),
                team: shared::map::Team::Green,
            }],
            join_deadline_ms: unix_ms() + 60000,
            cohort: None,
        })
        .unwrap();
    pool.adopt_for_test(&id);
    let mut lobby = Lobby::new(pool, Limits::default());
    lobby.limits.arenas_per_ip = 1;
    lobby.order = 100;
    lobby.waiting = (1..=100)
        .map(|i| waiter(i, MatchPreference::BotPractice, 1000, true, 0, now))
        .collect();
    let ip = SourceIp::of("198.51.100.1:40000".parse().unwrap());
    for w in &lobby.waiting {
        lobby.sources.insert(w.profile.profile_id.clone(), ip);
    }
    (lobby, directory, id)
}

#[test]
fn exact_capacity_line_reuses_renewals_and_invalidates_membership_and_source_limits() {
    let now = clock();
    let (mut lobby, directory, allocation) = occupied_lobby(now);
    lobby.refresh_line(now);
    assert_eq!(lobby.line_rebuilds, 1);
    assert_eq!(lobby.line.len(), 100);
    for _ in 0..500 {
        for w in &mut lobby.waiting {
            w.touched = now + Duration::from_secs(1);
        }
        lobby.refresh_line(now + Duration::from_secs(1));
    }
    assert_eq!(
        lobby.line_rebuilds, 1,
        "heartbeat renewals must not simulate every roster again"
    );
    let extra = waiter(101, MatchPreference::BotPractice, 1000, true, 0, now);
    let id = extra.profile.profile_id.clone();
    let ip = SourceIp::of("198.51.100.1:40000".parse().unwrap());
    lobby.sources.insert(id.clone(), ip);
    lobby.waiting.push(extra);
    lobby.refresh_line(now + Duration::from_secs(1));
    assert_eq!(lobby.line_rebuilds, 2);
    assert_eq!(lobby.line[&id], 101);
    lobby.arena_sources.insert(allocation, vec![ip]);
    lobby.refresh_line(now + Duration::from_secs(1));
    assert_eq!(lobby.line_rebuilds, 3);
    assert!(lobby.line.is_empty());
    lobby.limits.arenas_per_ip = 2;
    lobby.refresh_line(now + Duration::from_secs(1));
    assert_eq!(lobby.line_rebuilds, 4);
    assert_eq!(lobby.line[&id], 101);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_fresh_search_never_reuses_its_cancelled_attempts_capacity_position() {
    let now = clock();
    let (mut lobby, directory, _) = occupied_lobby(now);
    lobby.refresh_line(now);
    let entry = lobby.waiting[0].clone();
    assert_eq!(lobby.line[&entry.profile.profile_id], 1);
    let mut service = MatchService::Lobby(lobby);
    service.cancel(&entry.profile.profile_id, now);
    service.enqueue(
        entry.profile.clone(),
        entry.session.clone(),
        2,
        MatchPreference::BotPractice.into(),
        None,
        SourceIp::of("198.51.100.1:40000".parse().unwrap()),
        now,
    );
    let Some(MatchServiceView::Waiting { capacity_line, .. }) =
        service.view(&entry.profile.profile_id, &entry.session, now)
    else {
        panic!("fresh search must wait")
    };
    assert!(
        capacity_line.is_none(),
        "position1 belongs to the cancelled attempt"
    );
    service.tick(now + Duration::from_secs(1));
    let Some(MatchServiceView::Waiting {
        capacity_line: Some(line),
        ..
    }) = service.view(
        &entry.profile.profile_id,
        &entry.session,
        now + Duration::from_secs(1),
    )
    else {
        panic!("position follows the scheduling pass")
    };
    assert_eq!(
        line.position, 100,
        "a fresh search joins the end of the line"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn exact_line_key_ignores_elapsed_seconds_but_tracks_real_policy_boundaries() {
    let now = clock();
    let pool = crate::match_pool::Pool::for_test();
    let directory = pool.test_directory().to_owned();
    let mut lobby = Lobby::new(pool, Limits::default());
    lobby
        .waiting
        .push(waiter(1, MatchPreference::Ranked, 1000, true, 0, now));
    assert_eq!(
        LineKey::new(&lobby, now),
        LineKey::new(&lobby, now + Duration::from_secs(119))
    );
    assert_ne!(
        LineKey::new(&lobby, now),
        LineKey::new(&lobby, now + Duration::from_secs(120))
    );
    assert_eq!(
        LineKey::new(&lobby, now + Duration::from_secs(120)),
        LineKey::new(&lobby, now + Duration::from_secs(179))
    );
    assert_ne!(
        LineKey::new(&lobby, now + Duration::from_secs(299)),
        LineKey::new(&lobby, now + Duration::from_secs(300))
    );
    lobby.waiting[0].preference = MatchPreference::Casual;
    lobby.waiting[0].casual_since = Some(now);
    assert_ne!(
        LineKey::new(&lobby, now + Duration::from_secs(29)),
        LineKey::new(&lobby, now + Duration::from_secs(30))
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn snapshot_and_all_view_queries_are_measured_at_1000_and_4096_waiters() {
    let now = clock();
    for players in [1000_usize, 4096] {
        let waiting: Vec<_> = (0..players)
            .map(|i| {
                waiter(
                    i as u64 + 1,
                    MatchPreference::Ranked,
                    ((i * 7919) % 5001) as i32,
                    i % 2 == 0,
                    [0, 120, 180, 240, 300][i % 5],
                    now,
                )
            })
            .collect();
        let start = Instant::now();
        let snapshot = Snapshot::new(&waiting, now);
        let build = start.elapsed();
        let queries = Instant::now();
        for i in 0..players {
            assert_eq!(
                snapshot.index(&waiting, &waiting[i].profile.profile_id),
                Some(i)
            );
            let MatchServiceView::Waiting {
                searching: Some(n),
                humans,
                ..
            } = snapshot.view(&waiting, i, None, now)
            else {
                panic!("expected a wait")
            };
            assert_eq!(n, players as u32);
            assert!((1..=10).contains(&humans));
        }
        assert!(snapshot.windows.len() <= 9);
        assert_eq!(snapshot.entries.len(), players);
        println!(
            "QUEUE_VIEW_BENCH {}",
            serde_json::json!({"players":players,"build_ms":build.as_secs_f64()*1000.0,"all_views_ms":queries.elapsed().as_secs_f64()*1000.0,"window_indexes":snapshot.windows.len()})
        );
    }
}

#[test]
fn lobby_account_requests_reply_only_to_the_sender_without_starving_periodic_views() {
    use crate::runtime::ports::{ManualClock, MemoryTransport};
    use shared::career::CareerRequest;
    let now = clock();
    for lobby_role in [true, false] {
        let transport = MemoryTransport::new("127.0.0.1:4000".parse().unwrap());
        let mut runtime = ServerRuntime::for_test(
            transport.clone(),
            ManualClock::new(now),
            crate::career_backend::MemoryCareer::disabled(7),
            crate::match_rules::MatchConfig::dev(),
        );
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_owned();
        if lobby_role {
            runtime.match_service = MatchService::Lobby(Lobby::new(pool, Limits::default()));
        }
        let a: SocketAddr = "127.0.0.1:60101".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:60102".parse().unwrap();
        for address in [a, b] {
            runtime.world.ensure_connected(address, now);
            runtime
                .world
                .players
                .get_mut(&address)
                .unwrap()
                .career_capable = true;
        }
        runtime.send_career_views(now);
        transport.take_outbound();
        for request_id in 1..=200 {
            runtime.handle_career_request(
                a,
                CareerRequest::Profile {
                    request_id,
                    profile_id: "f".repeat(64),
                },
                now + Duration::from_millis(1),
            );
        }
        let responses = transport.take_outbound();
        assert!(responses.iter().any(|(address, _)| *address == a));
        assert_eq!(
            responses.iter().any(|(address, _)| *address == b),
            !lobby_role,
            "only the lobby changes from broadcast to request-scoped replies"
        );
        runtime.send_career_views(now + Duration::from_millis(249));
        assert!(transport.take_outbound().is_empty());
        runtime.send_career_views(now + Duration::from_millis(251));
        let periodic = transport.take_outbound();
        assert!(periodic.iter().any(|(address, _)| *address == a));
        assert!(
            periodic.iter().any(|(address, _)| *address == b),
            "request traffic cannot starve other players' periodic replies"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
