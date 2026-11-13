use super::*;

fn queued_backend(count: u16, now: Instant) -> MemoryCareer {
    let mut backend = MemoryCareer::test_backend(9);
    for index in 0..count {
        let addr = SocketAddr::from(([127, 0, 0, 1], 30000 + index));
        backend.test_authenticated(
            addr,
            ProfileSummary::new(format!("{index:064x}"), "Fixture".into()),
            &format!("s{index}"),
        );
        let c = backend.clients.get_mut(&addr).unwrap();
        c.key_checked = Some(now - Duration::from_secs(15));
        c.supporter_checked = c.key_checked;
    }
    backend
}

#[test]
fn a_thousand_clients_get_fair_bounded_refresh_before_authorization_expires() {
    let now = Instant::now();
    let mut backend = queued_backend(1000, now);
    let mut seen = HashSet::new();
    for step in 0..70 {
        backend.refresh_access(now + ACCESS_REFRESH_STEP * step);
        assert!(backend.link.jobs.len() <= ACCESS_REFRESH_BATCH);
        while let Some(job) = backend.link.jobs.pop_front() {
            if let Job::RefreshAccess { addr, .. } = &job {
                assert!(
                    seen.insert(*addr),
                    "refreshed client must yield to due peers"
                );
            } else {
                panic!("unexpected job");
            }
            backend.link.acknowledge(job);
        }
        backend.poll();
    }
    assert_eq!(seen.len(), 1000);
    assert!(backend.clients.values().all(|c| {
        c.auth.is_some() && c.key_checked.unwrap() >= now && c.supporter_checked.unwrap() >= now
    }));
    // The oldest initial check is at most22s old on the final paced round,
    // strictly inside the unchanged30s fail-closed authorization boundary.
    assert!(Duration::from_secs(15) + ACCESS_REFRESH_STEP * 70 < COSMETIC_CACHE_TTL);
}

#[test]
fn a_full_link_retries_without_claiming_a_successful_refresh() {
    let now = Instant::now();
    let mut backend = queued_backend(32, now);
    for i in 0..MAX_JOBS {
        backend.link.jobs.push_back(Job::Flush(i as u64));
    }
    backend.refresh_access(now);
    assert!(
        backend
            .clients
            .values()
            .all(|c| c.access_refresh_sent.is_none())
    );
    backend.link.jobs.clear();
    backend.refresh_access(now + ACCESS_REFRESH_STEP);
    assert_eq!(backend.link.jobs.len(), ACCESS_REFRESH_BATCH);
    assert_eq!(
        backend
            .clients
            .values()
            .filter(|c| c.access_refresh_sent.is_some())
            .count(),
        ACCESS_REFRESH_BATCH
    );
    assert!(
        backend
            .clients
            .values()
            .all(|c| c.key_checked.unwrap() < now)
    );
}

#[test]
fn missing_replies_are_retried_boundedly_and_never_extend_authorization() {
    let now = Instant::now();
    let mut backend = queued_backend(1, now);
    let addr = *backend.clients.keys().next().unwrap();
    backend.refresh_access(now);
    assert_eq!(backend.link.jobs.len(), 1);
    backend.link.jobs.clear();
    backend.refresh_access(now + Duration::from_secs(1));
    assert!(backend.link.jobs.is_empty());
    backend.refresh_access(now + ACCESS_REFRESH_RETRY);
    assert_eq!(backend.link.jobs.len(), 1);
    backend.clients.get_mut(&addr).unwrap().key_checked = Some(Instant::now() - COSMETIC_CACHE_TTL);
    backend.poll();
    assert!(backend.authenticated_session(addr).is_none());
    let late = backend.link.jobs.pop_front().unwrap();
    backend.link.acknowledge(late);
    backend.poll();
    assert!(backend.authenticated_session(addr).is_none());
}

#[test]
fn a_reply_for_an_old_nonce_cannot_complete_the_current_refresh() {
    let now = Instant::now();
    let mut backend = queued_backend(1, now);
    backend.refresh_access(now);
    let Job::RefreshAccess {
        addr,
        public_key,
        profile_id,
        ..
    } = backend.link.jobs.pop_front().unwrap()
    else {
        panic!("refresh expected")
    };
    backend.link.acknowledge(Job::RefreshAccess {
        addr,
        nonce: "old".into(),
        public_key,
        profile_id,
    });
    backend.poll();
    let c = &backend.clients[&addr];
    assert_eq!(c.access_refresh_sent, Some(now));
    assert_eq!(c.key_checked, Some(now - Duration::from_secs(15)));
}

#[test]
fn a_thousand_presence_profiles_are_deduplicated_with_live_seats_preserved() {
    let now = Instant::now();
    let mut backend = queued_backend(1000, now);
    let first = SocketAddr::from(([127, 0, 0, 1], 30000));
    let profile = backend.clients[&first].view.profile.clone().unwrap();
    let second = SocketAddr::from(([127, 0, 0, 1], 32000));
    backend.test_authenticated(second, profile.clone(), "other-device");
    backend.clients.get_mut(&first).unwrap().playing = true;
    backend
        .matches
        .insert(profile.profile_id.clone(), "live-allocation".into());
    backend.last_presence = now - Duration::from_secs(10);
    backend.poll();
    let presence = backend
        .link
        .jobs
        .iter()
        .find_map(|job| match job {
            Job::Presence(rows) => Some(rows),
            _ => None,
        })
        .unwrap();
    assert_eq!(presence.len(), 1000);
    assert_eq!(
        presence
            .iter()
            .find(|(id, _)| *id == profile.profile_id)
            .unwrap()
            .1
            .as_deref(),
        Some("live-allocation")
    );
    assert_eq!(
        presence
            .chunks(PRESENCE_BATCH_SIZE)
            .map(<[_]>::len)
            .collect::<Vec<_>>(),
        vec![512, 488]
    );
    assert_eq!(
        presence
            .iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>()
            .len(),
        1000
    );
}

fn queue_read(index: u16) -> Job {
    Job::QueueAccess {
        request: crate::career_port::MatchRequest {
            addr: SocketAddr::from(([127, 0, 0, 1], 30000 + index)),
            request_id: u64::from(index),
            preference: shared::match_service::MatchPreference::Casual,
            also_casual: false,
        },
        nonce: format!("nonce-{index}"),
        sequence: u64::from(index) + 1,
        public_key: "a".repeat(64),
        profile_id: "b".repeat(64),
    }
}

#[test]
fn read_batches_stop_before_barriers_and_leave_the_next_read_in_order() {
    for barrier in [Job::Flush(77), Job::Presence(Vec::new())] {
        let (sender, receiver) = mpsc::sync_channel(MAX_JOBS);
        assert!(sender.send(queue_read(1)).is_ok());
        assert!(sender.send(barrier).is_ok());
        assert!(sender.send(queue_read(2)).is_ok());
        let mut deferred = None;
        let batch = collect_access_reads(queue_read(0), &receiver, &mut deferred);
        assert_eq!(batch.len(), 2);
        assert!(matches!(deferred, Some(Job::Flush(77) | Job::Presence(_))));
        assert!(matches!(
            receiver.try_recv(),
            Ok(Job::QueueAccess { sequence: 3, .. })
        ));
    }
}

#[test]
fn read_batches_never_take_more_than_the_existing_lobby_connection_budget() {
    let (sender, receiver) = mpsc::sync_channel(MAX_JOBS);
    for index in 1..10 {
        assert!(sender.send(queue_read(index)).is_ok());
    }
    let mut deferred = None;
    let batch = collect_access_reads(queue_read(0), &receiver, &mut deferred);
    assert_eq!(batch.len(), ACCESS_READ_BATCH);
    assert!(deferred.is_none());
    assert!(matches!(
        receiver.try_recv(),
        Ok(Job::QueueAccess { sequence: 5, .. })
    ));
}

#[test]
#[ignore = "requires isolated OMOBA_TEST_DATABASE_URL; exercises concurrent account reads on real PostgreSQL"]
fn postgres_access_read_batches_preserve_identity_errors_and_revocation() {
    let url = std::env::var("OMOBA_TEST_DATABASE_URL").expect("isolated test database");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = CareerStore::connect_runtime(&url).await.unwrap();
        let key = random_id::<32>();
        let profile = store.authenticate(&key,"ReadBatch").await.unwrap();
        let read = |index, public_key: String| {
            let Job::QueueAccess {request,nonce,sequence,..}=queue_read(index) else {unreachable!()};
            Job::QueueAccess {request,nonce,sequence,public_key,profile_id:profile.profile_id.clone()}
        };
        let batch=vec![read(0,key.clone()),read(1,"invalid".into()),read(2,"f".repeat(64)),Job::RefreshAccess {addr:SocketAddr::from(([127,0,0,1],30003)),nonce:"refresh".into(),public_key:key.clone(),profile_id:profile.profile_id.clone()}];
        let replies=answer_access_reads(&store,batch).await;
        assert_eq!(replies.len(),4);
        let mut seen=HashSet::new();
        for reply in replies {
            match reply {
                Reply::QueueAccess {request,nonce,sequence,access}=>{
                    assert_eq!(nonce,format!("nonce-{}",request.request_id));
                    assert_eq!(sequence,request.request_id+1);
                    assert!(seen.insert(request.request_id));
                    match request.request_id {
                        0=>{let access=access.unwrap().unwrap();assert!(access.key_active);assert_eq!(access.completed_public_matches,0);assert_eq!(access.profile,Some(profile.clone()));},
                        1=>assert!(access.is_err()),
                        2=>{let access=access.unwrap().unwrap();assert!(!access.key_active);assert!(access.profile.is_none());},
                        _=>panic!("unexpected identity"),
                    }
                }
                Reply::RefreshAccess {nonce,key_active,profile:fresh,..}=>{assert_eq!(nonce,"refresh");assert_eq!(key_active,Some(true));assert_eq!(fresh,Some(profile.clone()));},
                _=>panic!("unexpected reply"),
            }
        }
        assert_eq!(seen.len(),3);
        let pool=sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::query("UPDATE career_keys SET revoked_at=clock_timestamp() WHERE public_key=$1 AND profile_id=$2").bind(&key).bind(&profile.profile_id).execute(&pool).await.unwrap();
        let replies=answer_access_reads(&store,vec![read(0,key)]).await;
        assert!(matches!(&replies[0],Reply::QueueAccess {access:Ok(Some(access)),..} if !access.key_active));
        pool.close().await;
    });
}

#[test]
fn season_reset_refreshes_a_signed_session_without_reconnect_and_fences_late_replies() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 30401));
    let mut backend = MemoryCareer::test_backend(9);
    let mut old = ProfileSummary::new("a".repeat(64), "Returning".into());
    old.rating = 2300;
    old.rated_matches = 40;
    old.matches_played = 55;
    old.completed_public_matches = 10;
    old.progression_xp = 12000;
    old.ranked = Some(shared::ranked::SeasonProfile {
        season_id: 1,
        started_at_ms: 1000,
        ends_at_ms: 1000 + shared::ranked::SEASON_DURATION_MS,
        rated_matches: 40,
        wins: 20,
        losses: 20,
        completed_matches: 40,
        latest_badge: None,
    });
    let mut reset = old.clone();
    reset.rating = 1650;
    let season = reset.ranked.as_mut().unwrap();
    season.season_id = 2;
    season.started_at_ms = season.ends_at_ms;
    season.ends_at_ms += shared::ranked::SEASON_DURATION_MS;
    season.rated_matches = 0;
    season.wins = 0;
    season.losses = 0;
    season.completed_matches = 0;
    season.latest_badge = Some(shared::ranked::SeasonBadge {
        season_id: 1,
        ended_at_ms: season.started_at_ms,
        rating: 2300,
        rated_matches: 40,
        tier: Some(shared::ranked::Tier::Platinum),
    });
    backend.test_authenticated(addr, old.clone(), "same-session");
    let nonce = backend.clients[&addr].auth.as_ref().unwrap().nonce.clone();
    let refresh = |profile: ProfileSummary, nonce: String, active| Reply::RefreshAccess {
        completed_public_matches: Some(profile.completed_public_matches),
        profile: Some(profile),
        addr,
        nonce,
        key_active: Some(active),
        supporter: None,
        nickname: None,
        sanctions: Some(Vec::new()),
        play_cooldown: Some(None),
    };
    // An unrelated identity or a response from the previous sign-in cannot reset us.
    let mut foreign = reset.clone();
    foreign.profile_id = "b".repeat(64);
    for reply in [
        refresh(foreign, nonce.clone(), true),
        refresh(reset.clone(), "old-login".into(), true),
    ] {
        backend.link.replies.push_back(reply);
        backend.poll();
        assert_eq!(backend.profile(addr), Some(old.clone()));
    }
    backend
        .link
        .replies
        .push_back(refresh(reset.clone(), nonce.clone(), true));
    backend.poll();
    assert_eq!(backend.profile(addr), Some(reset.clone()));
    assert_eq!(
        backend.authenticated_session(addr).as_deref(),
        Some("same-session")
    );
    assert!(reset.ranked_unlocked());
    // A slow pre-reset read, own-profile action and settlement must not undo the reset.
    let mut stale_public = old.clone();
    stale_public.completed_public_matches = 9;
    backend
        .link
        .replies
        .push_back(refresh(stale_public, nonce.clone(), true));
    backend.link.replies.push_back(Reply::Action {
        addr,
        nonce: nonce.clone(),
        request_id: Some(7),
        result: Ok(shared::career::CareerView {
            profile: Some(old.clone()),
            ..Default::default()
        }),
    });
    let receipt = serde_json::from_value(serde_json::json!({
        "result_id": "old-award", "server_epoch": 9, "match_id": 1,
        "started_at_ms": 1, "ended_at_ms": 2, "duration_ms": 1,
        "map_profile": "verdant_default", "ruleset": "public-ranked-v1",
        "outcome": "completed", "winner": "green", "rated": true,
        "participants": [], "ranked_season_id": 1
    }))
    .unwrap();
    backend.link.replies.push_back(Reply::Settled {
        result: receipt,
        profiles: vec![old],
    });
    backend.poll();
    assert_eq!(backend.profile(addr), Some(reset.clone()));
    // A same-season in-flight access read cannot replace a newer settled award.
    backend
        .clients
        .get_mut(&addr)
        .unwrap()
        .view
        .profile
        .as_mut()
        .unwrap()
        .rating = 1670;
    backend
        .link
        .replies
        .push_back(refresh(reset.clone(), nonce.clone(), true));
    backend.poll();
    assert_eq!(backend.profile(addr).unwrap().rating, 1670);
    // Queue authorization refreshes the season before dispatching Find too.
    let mut third = reset.clone();
    third.ranked.as_mut().unwrap().season_id = 3;
    third.rating = 1325;
    let request = crate::career_port::MatchRequest {
        addr,
        request_id: 8,
        preference: shared::match_service::MatchPreference::Ranked,
        also_casual: false,
    };
    backend.clients.get_mut(&addr).unwrap().queue_sequence = Some(8);
    backend.link.replies.push_back(Reply::QueueAccess {
        request,
        nonce: nonce.clone(),
        sequence: 8,
        access: Ok(Some(omoba_career_store::career_store::moderation::Access {
            profile: Some(third.clone()),
            completed_public_matches: 10,
            key_active: true,
            nickname: None,
            sanctions: Vec::new(),
            play_cooldown: None,
        })),
    });
    backend.poll();
    assert_eq!(backend.take_match_requests(), [request]);
    assert_eq!(backend.profile(addr), Some(third.clone()));
    // Revocation still ends the session and cannot install the supplied snapshot.
    backend.link.replies.push_back(refresh(reset, nonce, false));
    backend.poll();
    assert!(backend.authenticated_session(addr).is_none());
    assert_eq!(backend.view(addr).profile, Some(third));
}
