//! Actual worker/PostgreSQL failure-path tests. Never substitutes a mock DB.
use std::thread::JoinHandle;

use shared::HeroClass;
use shared::map::Team;

use super::*;

struct OwnedWorker {
    jobs: Option<SyncSender<Job>>,
    replies: Receiver<Reply>,
    handle: Option<JoinHandle<()>>,
    outbox: PathBuf,
}
impl OwnedWorker {
    fn spawn(url: String, full_replies: bool) -> Self {
        let outbox = std::env::temp_dir().join(format!("omoba-resilience-{}", random_id::<16>()));
        Self::spawn_at(url, full_replies, outbox)
    }
    fn spawn_at(url: String, full_replies: bool, outbox: PathBuf) -> Self {
        Self::spawn_with_admission(
            url,
            full_replies,
            outbox,
            RecoveryScope::Global,
            Admission::for_test(),
        )
    }
    fn spawn_public(url: String, outbox: PathBuf) -> Self {
        let scope = RecoveryScope::Disabled;
        let admission = Admission {
            require_ready_consent: scope.requires_ready_consent(),
            ..Admission::for_test()
        };
        Self::spawn_with_admission(url, false, outbox, scope, admission)
    }
    fn spawn_with_admission(
        url: String,
        full_replies: bool,
        outbox: PathBuf,
        scope: RecoveryScope,
        admission: Admission,
    ) -> Self {
        let (jobs, receiver) = mpsc::sync_channel(MAX_JOBS);
        let (sender, replies) = mpsc::sync_channel(MAX_JOBS * 2);
        if full_replies {
            for _ in 0..MAX_JOBS * 2 {
                sender
                    .try_send(Reply::RecordError(
                        "filler".into(),
                        "prefilled fixture".into(),
                    ))
                    .unwrap();
            }
        }
        let path = outbox.clone();
        let handle =
            std::thread::spawn(move || worker(url, path, receiver, sender, scope, admission));
        Self {
            jobs: Some(jobs),
            replies,
            handle: Some(handle),
            outbox,
        }
    }
    fn send(&self, job: Job) {
        self.jobs
            .as_ref()
            .unwrap()
            .send(job)
            .expect("live fixture worker");
    }
    fn wait_for(&self, mut accept: impl FnMut(Reply) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline {
            match self.replies.recv_timeout(Duration::from_millis(100)) {
                Ok(reply) => {
                    if accept(reply) {
                        return;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("worker exited before expected reply")
                }
            }
        }
        panic!("timed out waiting for worker reply");
    }
}
impl Drop for OwnedWorker {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(handle) = self.handle.take() {
            let result = handle.join();
            if !std::thread::panicking() {
                result.expect("fixture worker exits cleanly");
            }
        }
        if self.outbox.exists() {
            // Only this fixture's randomly named temporary directory is removed.
            let _ = fs::remove_dir_all(&self.outbox);
        }
    }
}

fn database() -> (String, tokio::runtime::Runtime, CareerStore, sqlx::PgPool) {
    let url =
        std::env::var("OMOBA_TEST_DATABASE_URL").expect("set isolated OMOBA_TEST_DATABASE_URL");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = runtime
        .block_on(CareerStore::connect(&url))
        .expect("connect real PostgreSQL");
    let pool = runtime.block_on(sqlx::PgPool::connect(&url)).unwrap();
    (url, runtime, store, pool)
}
// These new cases retain settled immutable rows, so isolate the entire real
// schema instead of attempting forbidden per-match cleanup in the shared DB.
fn isolated_public_database() -> (
    String,
    tokio::runtime::Runtime,
    CareerStore,
    sqlx::PgPool,
    sqlx::PgPool,
    String,
) {
    let base =
        std::env::var("OMOBA_TEST_DATABASE_URL").expect("set isolated OMOBA_TEST_DATABASE_URL");
    assert!(
        !base.contains("options="),
        "fixture owns its search_path option"
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let admin = runtime
        .block_on(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .connect(&base),
        )
        .unwrap();
    let schema = format!("career_public_consent_{}", random_id::<16>());
    runtime
        .block_on(sqlx::query(&format!("CREATE SCHEMA {schema}")).execute(&admin))
        .unwrap();
    let url = format!(
        "{base}{}options=-csearch_path%3D{schema}",
        if base.contains('?') { "&" } else { "?" }
    );
    let store = runtime.block_on(CareerStore::connect(&url)).unwrap();
    let pool = runtime
        .block_on(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .connect(&url),
        )
        .unwrap();
    (url, runtime, store, pool, admin, schema)
}
fn close_public_database(
    runtime: &tokio::runtime::Runtime,
    store: CareerStore,
    pool: sqlx::PgPool,
    admin: sqlx::PgPool,
    schema: &str,
) {
    assert!(
        schema.starts_with("career_public_consent_")
            && schema
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    );
    runtime.block_on(pool.close());
    drop(store);
    runtime
        .block_on(sqlx::query(&format!("DROP SCHEMA {schema} CASCADE")).execute(&admin))
        .unwrap();
    runtime.block_on(admin.close());
}
fn erase_owned_restore_fixture_allocation(
    runtime: &tokio::runtime::Runtime,
    pool: &sqlx::PgPool,
    schema: &str,
    id: &str,
) {
    runtime.block_on(async {
        let mut tx=pool.begin().await.unwrap();
        let actual:String=sqlx::query_scalar("SELECT current_schema()").fetch_one(&mut *tx).await.unwrap();
        assert_eq!(actual,schema);assert!(actual.starts_with("career_public_consent_"));
        // Test-only partial-restore simulation inside this disposable schema.
        // Both append-only guards are restored in the same transaction.
        sqlx::raw_sql("ALTER TABLE career_matches DISABLE TRIGGER career_match_guard; ALTER TABLE career_participants DISABLE TRIGGER career_participant_guard;").execute(&mut *tx).await.unwrap();
        sqlx::query("DELETE FROM career_presence WHERE result_id=$1").bind(id).execute(&mut *tx).await.unwrap();
        sqlx::query("DELETE FROM career_active_profiles WHERE result_id=$1").bind(id).execute(&mut *tx).await.unwrap();
        sqlx::query("DELETE FROM career_participants WHERE result_id=$1").bind(id).execute(&mut *tx).await.unwrap();
        sqlx::query("DELETE FROM career_matches WHERE result_id=$1").bind(id).execute(&mut *tx).await.unwrap();
        sqlx::raw_sql("ALTER TABLE career_matches ENABLE TRIGGER career_match_guard; ALTER TABLE career_participants ENABLE TRIGGER career_participant_guard;").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    });
}

fn allocation(profile: &ProfileSummary) -> MatchResult {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    MatchResult {
        ranked_season_id: None,
        gameplay_started: true,
        end_reason: None,
        result_id: random_id::<16>(),
        server_epoch: now,
        match_id: 1,
        started_at_ms: now - 1000,
        ended_at_ms: 0,
        duration_ms: 0,
        map_profile: "resilience-fixture".into(),
        ruleset: "test-unranked".into(),
        outcome: MatchOutcome::Interrupted,
        winner: None,
        rated: false,
        unrated_reason: Some("resilience fixture".into()),
        saved: false,
        participants: vec![ParticipantResult {
            input_kind: Default::default(),
            participation: None,
            is_bot: false,
            player_id: 1,
            profile_id: Some(profile.profile_id.clone()),
            nickname: profile.nickname.clone(),
            team: Team::Green,
            hero_class: HeroClass::Warrior,
            character: "warrior".into(),
            avatar: None,
            sprite_character: None,
            stats: MatchStats {
                final_level: 1,
                ..Default::default()
            },
            disconnected: false,
            rating: None,
            progression_xp_gained: 0,
        }],
    }
}
fn terminal(start: &MatchResult) -> MatchResult {
    let mut result = start.clone();
    result.outcome = MatchOutcome::Completed;
    result.winner = Some(Team::Green);
    result.ended_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    result.duration_ms = result.ended_at_ms.saturating_sub(result.started_at_ms);
    result.participants[0].stats.damage_to_heroes = 73.5;
    result
}
fn record(kind: RecordKind, result: MatchResult) -> Job {
    Job::Record(Box::new(PendingRecord {
        kind,
        result,
        recovery_allocation: None,
        recovered_live: false,
    }))
}
fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("fixture condition timed out");
}

#[test]
#[ignore = "requires isolated OMOBA_TEST_DATABASE_URL; real worker heartbeat/backpressure"]
fn postgres_worker_renews_lease_with_full_reply_buffer_and_retries_critical_acks() {
    let (url, runtime, store, pool) = database();
    let profile = runtime
        .block_on(store.authenticate(&random_id::<32>(), "Backpressure tester"))
        .unwrap();
    let start = allocation(&profile);
    let fixture = OwnedWorker::spawn(url, true);
    fixture.send(record(RecordKind::Start, start.clone()));
    wait_until(|| {
        runtime
            .block_on(
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM career_matches WHERE result_id=$1)",
                )
                .bind(&start.result_id)
                .fetch_one(&pool),
            )
            .unwrap()
    });
    assert!(
        spool_path(&fixture.outbox, &start.result_id)
            .unwrap()
            .exists(),
        "unaccepted Start ACK remains durable"
    );
    runtime.block_on(sqlx::query("UPDATE career_matches SET lease_until=clock_timestamp()+interval '15 seconds' WHERE result_id=$1")
        .bind(&start.result_id).execute(&pool)).unwrap();
    // Leave the reply channel entirely full. The former SyncSender::send
    // implementation blocks its only runtime thread here and cannot heartbeat.
    std::thread::sleep(Duration::from_secs(12));
    let seconds: f64 = runtime.block_on(sqlx::query_scalar(
        "SELECT extract(epoch FROM lease_until-clock_timestamp())::double precision FROM career_matches WHERE result_id=$1",
    ).bind(&start.result_id).fetch_one(&pool)).unwrap();
    assert!(
        seconds > 60.0,
        "full reply buffer stalled lease renewal: {seconds}s remaining"
    );
    fixture.wait_for(|reply| matches!(reply, Reply::Started(ref id) if id == &start.result_id));
    let finished = terminal(&start);
    fixture.send(record(RecordKind::Settle, finished.clone()));
    fixture.wait_for(|reply| match reply {
        Reply::Settled { result, profiles } if result.result_id == start.result_id => {
            assert!(result.saved);
            assert_eq!(result.participants[0].stats.damage_to_heroes, 73.5);
            assert_eq!(profiles[0].matches_played, 1);
            true
        }
        _ => false,
    });
    wait_until(|| {
        !spool_path(&fixture.outbox, &start.result_id)
            .unwrap()
            .exists()
    });
    assert!(
        runtime
            .block_on(store.settled_result(&start.result_id))
            .unwrap()
            .unwrap()
            .saved
    );
    drop(fixture);
    runtime.block_on(pool.close());
}

#[test]
#[ignore = "requires isolated OMOBA_TEST_DATABASE_URL; real worker rejection tombstone"]
fn postgres_worker_rejected_start_discards_already_queued_terminal_without_resurrection() {
    let (url, runtime, store, pool) = database();
    let public_key = random_id::<32>();
    let profile = runtime
        .block_on(store.authenticate(&public_key, "Tombstone tester"))
        .unwrap();
    let holding = allocation(&profile);
    runtime.block_on(store.start(holding.clone())).unwrap();
    let rejected = allocation(&profile);
    let fixture = OwnedWorker::spawn(url, false);
    fixture.send(record(RecordKind::Start, rejected.clone()));
    // Queue a terminal job before receiving any parent-side rejection reply.
    fixture.send(record(RecordKind::Settle, terminal(&rejected)));
    // This action is an in-order barrier proving the terminal job was consumed.
    fixture.send(Job::Action {
        addr: "127.0.0.1:39101".parse().unwrap(),
        nonce: "fixture-barrier".into(),
        public_key,
        profile_id: profile.profile_id.clone(),
        action: CareerAction::Friends { request_id: 991 },
    });
    let mut rejected_ack = false;
    let mut barrier = false;
    fixture.wait_for(|reply| {
        match reply {
            Reply::Rejected(id, _) if id == rejected.result_id => rejected_ack = true,
            Reply::Action {
                request_id: Some(991),
                result,
                ..
            } => {
                result.unwrap();
                barrier = true;
            }
            Reply::Started(id) if id == rejected.result_id => panic!("rejected allocation started"),
            Reply::Settled { result, .. } if result.result_id == rejected.result_id => {
                panic!("rejected allocation settled")
            }
            _ => {}
        }
        rejected_ack && barrier
    });
    let count: i64 = runtime
        .block_on(
            sqlx::query_scalar("SELECT count(*) FROM career_matches WHERE result_id=$1")
                .bind(&rejected.result_id)
                .fetch_one(&pool),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "queued terminal resurrected a rejected allocation"
    );
    assert!(
        !spool_path(&fixture.outbox, &rejected.result_id)
            .unwrap()
            .exists()
    );
    // Shutdown flushes any erroneously retained pending terminal to disk before
    // exiting, so inspect the directory after the thread has stopped as well.
    let mut fixture = fixture;
    fixture.jobs.take();
    fixture.handle.take().unwrap().join().unwrap();
    assert!(
        !spool_path(&fixture.outbox, &rejected.result_id)
            .unwrap()
            .exists(),
        "terminal remained pending after rejection"
    );
    runtime.block_on(store.settle(terminal(&holding))).unwrap();
    drop(fixture);
    runtime.block_on(pool.close());
}

fn queue_profile(
    runtime: &tokio::runtime::Runtime,
    store: &CareerStore,
) -> (String, ProfileSummary, queue_dodges::Event) {
    let key = random_id::<32>();
    let profile = runtime
        .block_on(store.authenticate(&key, &format!("Queue{}", &key[..8])))
        .unwrap();
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let event = queue_dodges::Event::new(&profile.profile_id, at).unwrap();
    (key, profile, event)
}
fn clear_queue_profile(
    runtime: &tokio::runtime::Runtime,
    pool: &sqlx::PgPool,
    profile: &ProfileSummary,
) {
    runtime.block_on(async {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM career_keys WHERE profile_id=$1")
            .bind(&profile.profile_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("DELETE FROM career_profiles WHERE profile_id=$1")
            .bind(&profile.profile_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    });
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn queue_dodge_offline_flush_restart_and_duplicate_delivery_are_exactly_once() {
    let (url, runtime, store, pool) = database();
    let (_, profile, event) = queue_profile(&runtime, &store);
    let mut offline =
        OwnedWorker::spawn("postgres://unused:unused@127.0.0.1:9/missing".into(), false);
    offline.send(Job::QueueDodge(event.clone()));
    offline.send(Job::Flush(901));
    offline.wait_for(|reply| matches!(reply, Reply::Flushed(901, true)));
    let file = offline
        .outbox
        .join("queue-dodges")
        .join(format!("{}.json", event.event_id));
    assert!(file.is_file());
    assert!(
        !offline
            .outbox
            .join(format!("{}.json", event.event_id))
            .exists()
    );
    assert!(
        runtime
            .block_on(store.queue_dodge_cooldown(&profile.profile_id))
            .unwrap()
            .is_none()
    );
    offline.jobs.take();
    offline.handle.take().unwrap().join().unwrap();
    let restarted = OwnedWorker::spawn_at(url, false, offline.outbox.clone());
    restarted.wait_for(
        |reply| matches!(reply, Reply::QueueDodgeCommitted(committed) if committed==event),
    );
    let first = runtime
        .block_on(store.queue_dodge_cooldown(&profile.profile_id))
        .unwrap()
        .unwrap();
    assert_eq!(first.strikes, 1);
    restarted.send(Job::QueueDodge(event.clone()));
    restarted.wait_for(
        |reply| matches!(reply, Reply::QueueDodgeCommitted(committed) if committed==event),
    );
    restarted.send(Job::Flush(902));
    restarted.wait_for(|reply| matches!(reply, Reply::Flushed(902, true)));
    assert!(!file.exists());
    assert_eq!(
        runtime
            .block_on(store.queue_dodge_cooldown(&profile.profile_id))
            .unwrap(),
        Some(first)
    );
    assert_eq!(
        runtime
            .block_on(store.profile(&profile.profile_id))
            .unwrap(),
        profile
    );
    let count: i64 = runtime
        .block_on(
            sqlx::query_scalar(
                "SELECT count(*) FROM career_queue_dodge_events WHERE profile_id=$1",
            )
            .bind(&profile.profile_id)
            .fetch_one(&pool),
        )
        .unwrap();
    assert_eq!(count, 1);
    let matches: i64 = runtime
        .block_on(
            sqlx::query_scalar("SELECT count(*) FROM career_participants WHERE profile_id=$1")
                .bind(&profile.profile_id)
                .fetch_one(&pool),
        )
        .unwrap();
    assert_eq!(
        matches, 0,
        "a queue receipt must never become an Interrupted match"
    );
    drop(restarted);
    drop(offline);
    clear_queue_profile(&runtime, &pool, &profile);
    runtime.block_on(pool.close());
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn queue_dodge_full_reply_channel_retains_receipt_and_retries_the_commit_ack() {
    let (url, runtime, store, pool) = database();
    let (_, profile, event) = queue_profile(&runtime, &store);
    let worker = OwnedWorker::spawn(url, true);
    worker.send(Job::QueueDodge(event.clone()));
    worker.send(Job::Flush(903));
    let file = worker
        .outbox
        .join("queue-dodges")
        .join(format!("{}.json", event.event_id));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        file.is_file(),
        "receipt must reach disk before a blocked flush reply"
    );
    let mut committed = false;
    let mut flushed = false;
    worker.wait_for(|reply| {
        match reply {
            Reply::QueueDodgeCommitted(receipt) if receipt == event => committed = true,
            Reply::Flushed(903, true) => flushed = true,
            _ => {}
        }
        committed && flushed
    });
    worker.send(Job::Flush(904));
    worker.wait_for(|reply| matches!(reply, Reply::Flushed(904, true)));
    let count: i64 = runtime
        .block_on(
            sqlx::query_scalar(
                "SELECT count(*) FROM career_queue_dodge_events WHERE profile_id=$1",
            )
            .bind(&profile.profile_id)
            .fetch_one(&pool),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        runtime
            .block_on(store.queue_dodge_cooldown(&profile.profile_id))
            .unwrap()
            .unwrap()
            .strikes,
        1
    );
    assert!(!file.exists());
    drop(worker);
    clear_queue_profile(&runtime, &pool, &profile);
    runtime.block_on(pool.close());
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn corrupt_queue_dodge_journal_refuses_search_and_flush_but_keeps_access_refresh() {
    let (url, runtime, store, pool) = database();
    let (public_key, profile, event) = queue_profile(&runtime, &store);
    let outbox = std::env::temp_dir().join(format!("omoba-resilience-{}", random_id::<16>()));
    let root = outbox.join("queue-dodges");
    fs::create_dir_all(&root).unwrap();
    let corrupt = root.join(format!("{}.json", event.event_id));
    fs::write(&corrupt, b"{broken").unwrap();
    let worker = OwnedWorker::spawn_at(url, false, outbox);
    worker.send(Job::QueueAccess {
        request: crate::career_port::MatchRequest {
            addr: "127.0.0.1:39001".parse().unwrap(),
            request_id: 1,
            preference: shared::match_service::MatchPreference::Casual,
            also_casual: false,
        },
        nonce: "queue-fixture".into(),
        sequence: 1,
        public_key: public_key.clone(),
        profile_id: profile.profile_id.clone(),
    });
    worker.send(Job::RefreshAccess {
        addr: "127.0.0.1:39001".parse().unwrap(),
        nonce: "refresh-fixture".into(),
        public_key,
        profile_id: profile.profile_id.clone(),
    });
    worker.send(Job::Flush(905));
    let mut denied = false;
    let mut refreshed = false;
    let mut flush_failed = false;
    worker.wait_for(|reply| {
        match reply {
            Reply::QueueAccess { access, .. } => {
                assert!(access.is_err());
                denied = true;
            }
            Reply::RefreshAccess {
                key_active: Some(true),
                ..
            } => refreshed = true,
            Reply::Flushed(905, false) => flush_failed = true,
            _ => {}
        }
        denied && refreshed && flush_failed
    });
    assert_eq!(fs::read(corrupt).unwrap(), b"{broken");
    drop(worker);
    clear_queue_profile(&runtime, &pool, &profile);
    runtime.block_on(pool.close());
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn queue_dodge_shared_access_checks_both_worker_paths_without_revoking_the_session() {
    let (url, runtime, store, pool) = database();
    let (key, profile, event) = queue_profile(&runtime, &store);
    runtime
        .block_on(store.record_queue_dodge(
            &event.event_id,
            &event.profile_id,
            event.observed_at_ms,
        ))
        .unwrap();
    let request = crate::career_port::MatchRequest {
        addr: "127.0.0.1:51802".parse().unwrap(),
        request_id: 701,
        preference: shared::match_service::MatchPreference::Ranked,
        also_casual: false,
    };
    let job = |sequence| Job::QueueAccess {
        request,
        nonce: "owned-access-fixture".into(),
        sequence,
        public_key: key.clone(),
        profile_id: profile.profile_id.clone(),
    };
    assert!(
        matches!(answer(&runtime, &store, job(1), &Admission::for_test()),
        Some(Reply::QueueAccess { access: Err(error), .. }) if error == shared::career::error_code::DODGE_COOLDOWN)
    );
    let worker = OwnedWorker::spawn(url, false);
    worker.send(job(2));
    worker.send(Job::RefreshAccess {
        addr: request.addr,
        nonce: "owned-refresh-fixture".into(),
        public_key: key.clone(),
        profile_id: profile.profile_id.clone(),
    });
    let (mut refused, mut refreshed) = (false, false);
    worker.wait_for(|reply| {
        match reply {
            Reply::QueueAccess {
                sequence: 2,
                access,
                ..
            } => {
                assert_eq!(
                    access.unwrap_err(),
                    shared::career::error_code::DODGE_COOLDOWN
                );
                refused = true;
            }
            Reply::RefreshAccess {
                key_active: Some(true),
                ..
            } => refreshed = true,
            _ => {}
        }
        refused && refreshed
    });
    runtime.block_on(sqlx::query("UPDATE career_queue_penalties SET cooldown_until=clock_timestamp()-interval '1 second' WHERE profile_id=$1")
        .bind(&profile.profile_id).execute(&pool)).unwrap();
    worker.send(job(3));
    worker.wait_for(|reply| match reply {
        Reply::QueueAccess {
            sequence: 3,
            access,
            ..
        } => {
            assert!(access.unwrap().unwrap().key_active);
            true
        }
        _ => false,
    });
    drop(worker);
    clear_queue_profile(&runtime, &pool, &profile);
    runtime.block_on(pool.close());
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn queue_dodge_rejected_start_retires_the_attempt_instead_of_waiting_for_penalty_expiry() {
    let (url, runtime, store, pool) = database();
    let (_, profile, event) = queue_profile(&runtime, &store);
    runtime
        .block_on(store.record_queue_dodge(
            &event.event_id,
            &event.profile_id,
            event.observed_at_ms,
        ))
        .unwrap();
    let rejected = allocation(&profile);
    let worker = OwnedWorker::spawn(url, false);
    worker.send(record(RecordKind::Start, rejected.clone()));
    worker.send(record(RecordKind::Settle, terminal(&rejected)));
    worker.wait_for(|reply| match reply {
        Reply::Rejected(id, error) if id == rejected.result_id => {
            assert_eq!(error, shared::career::error_code::DODGE_COOLDOWN);
            true
        }
        Reply::Started(id) if id == rejected.result_id => panic!("blocked roster started"),
        Reply::Settled { result, .. } if result.result_id == rejected.result_id => {
            panic!("blocked roster settled")
        }
        _ => false,
    });
    runtime.block_on(sqlx::query("UPDATE career_queue_penalties SET cooldown_until=clock_timestamp()-interval '1 second' WHERE profile_id=$1")
        .bind(&profile.profile_id).execute(&pool)).unwrap();
    // A late terminal retry after expiry cannot resurrect the refused attempt.
    worker.send(record(RecordKind::Settle, terminal(&rejected)));
    worker.send(Job::Flush(905));
    worker.wait_for(|reply| matches!(reply, Reply::Flushed(905, true)));
    let count: i64 = runtime
        .block_on(
            sqlx::query_scalar("SELECT count(*) FROM career_matches WHERE result_id=$1")
                .bind(&rejected.result_id)
                .fetch_one(&pool),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        !spool_path(&worker.outbox, &rejected.result_id)
            .unwrap()
            .exists()
    );
    assert_eq!(
        runtime
            .block_on(store.profile(&profile.profile_id))
            .unwrap(),
        profile
    );
    drop(worker);
    clear_queue_profile(&runtime, &pool, &profile);
    runtime.block_on(pool.close());
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn ready_ownership_worker_retires_a_start_with_foreign_unconfirmed_consent() {
    use omoba_career_store::career_store::ready::{Action, OWNED_ELSEWHERE, Seat};
    let (url, runtime, store, pool) = database();
    let (_, profile, _) = queue_profile(&runtime, &store);
    let id = random_id::<16>();
    runtime
        .block_on(store.ready_reserve(
            &id,
            shared::match_service::MatchPreference::Casual,
            vec![Seat {
                profile_id: profile.profile_id.clone(),
                session_id: "consent-fixture".into(),
            }],
        ))
        .unwrap();
    let initial = allocation(&profile);
    let worker = OwnedWorker::spawn(url, false);
    worker.send(record(RecordKind::Start, initial.clone()));
    worker.wait_for(|reply| match reply {
        Reply::Rejected(result_id, error) if result_id == initial.result_id => {
            assert_eq!(error, OWNED_ELSEWHERE);
            true
        }
        Reply::Started(result_id) if result_id == initial.result_id => {
            panic!("unconfirmed roster started")
        }
        _ => false,
    });
    runtime
        .block_on(store.ready_update(&id, Action::Cancel))
        .unwrap();
    worker.send(record(RecordKind::Settle, terminal(&initial)));
    worker.send(Job::Flush(906));
    worker.wait_for(|reply| matches!(reply, Reply::Flushed(906, true)));
    let count: i64 = runtime
        .block_on(
            sqlx::query_scalar("SELECT count(*) FROM career_matches WHERE result_id=$1")
                .bind(&initial.result_id)
                .fetch_one(&pool),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        !spool_path(&worker.outbox, &initial.result_id)
            .unwrap()
            .exists()
    );
    drop(worker);
    runtime
        .block_on(
            sqlx::query("DELETE FROM career_ready_checks WHERE check_id=$1")
                .bind(&id)
                .execute(&pool),
        )
        .unwrap();
    clear_queue_profile(&runtime, &pool, &profile);
    runtime.block_on(pool.close());
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn public_consent_worker_refuses_unknown_start_and_already_queued_terminal() {
    let (url, runtime, store, pool, admin, schema) = isolated_public_database();
    let (_, profile, _) = queue_profile(&runtime, &store);
    let initial = allocation(&profile);
    let outbox = std::env::temp_dir().join(format!("omoba-public-consent-{}", random_id::<16>()));
    let worker = OwnedWorker::spawn_public(url, outbox);
    worker.send(record(RecordKind::Start, initial.clone()));
    worker.send(record(RecordKind::Settle, terminal(&initial)));
    worker.wait_for(|reply| match reply {
        Reply::Rejected(id, error) if id == initial.result_id => {
            assert_eq!(error, omoba_career_store::career_store::ready::CLOSED);
            true
        }
        Reply::Started(id) if id == initial.result_id => {
            panic!("public worker admitted a roster without SQL consent")
        }
        Reply::Settled { result, .. } if result.result_id == initial.result_id => {
            panic!("unconfirmed public roster got a saved result")
        }
        _ => false,
    });
    worker.send(record(RecordKind::Settle, terminal(&initial)));
    worker.send(Job::Flush(910));
    worker.wait_for(|r| matches!(r, Reply::Flushed(910, true)));
    let count: i64 = runtime
        .block_on(
            sqlx::query_scalar("SELECT count(*) FROM career_matches WHERE result_id=$1")
                .bind(&initial.result_id)
                .fetch_one(&pool),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        runtime
            .block_on(store.profile(&profile.profile_id))
            .unwrap(),
        profile
    );
    assert!(
        !spool_path(&worker.outbox, &initial.result_id)
            .unwrap()
            .exists()
    );
    drop(worker);
    close_public_database(&runtime, store, pool, admin, &schema);
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn public_consent_unknown_recovery_is_retained_until_known_sql_admission_is_restored() {
    use omoba_career_store::career_store::ready::{Action, Seat};
    for kind in [
        RecordKind::Start,
        RecordKind::Checkpoint,
        RecordKind::Settle,
    ] {
        let (url, runtime, store, pool, admin, schema) = isolated_public_database();
        let (_, profile, _) = queue_profile(&runtime, &store);
        let initial = allocation(&profile);
        runtime
            .block_on(store.ready_reserve(
                &initial.result_id,
                shared::match_service::MatchPreference::BotPractice,
                vec![Seat {
                    profile_id: profile.profile_id.clone(),
                    session_id: "restore-fixture".into(),
                }],
            ))
            .unwrap();
        runtime
            .block_on(store.ready_update(&initial.result_id, Action::Publish))
            .unwrap();
        runtime
            .block_on(store.ready_update(
                &initial.result_id,
                Action::Answer {
                    profile_id: profile.profile_id.clone(),
                    session_id: "restore-fixture".into(),
                    accepted: true,
                },
            ))
            .unwrap();
        runtime
            .block_on(store.start_public(initial.clone()))
            .unwrap();
        // Owned fixture models a restore missing the committed match row.
        // Consumed consent alone must not recreate its admission automatically.
        erase_owned_restore_fixture_allocation(&runtime, &pool, &schema, &initial.result_id);
        let outbox =
            std::env::temp_dir().join(format!("omoba-public-consent-{}", random_id::<16>()));
        let is_terminal = matches!(kind, RecordKind::Settle);
        let mut snapshot = if is_terminal {
            terminal(&initial)
        } else {
            initial.clone()
        };
        snapshot.participants[0].stats.damage_to_heroes = 73.5;
        write_record(
            &outbox,
            &PendingRecord {
                kind,
                result: snapshot.clone(),
                recovery_allocation: is_terminal.then(|| initial.clone()),
                recovered_live: false,
            },
        )
        .unwrap();
        let worker = OwnedWorker::spawn_public(url, outbox);
        worker.wait_for(|reply| match reply {
            Reply::RecordError(id, error) if id == initial.result_id => {
                assert_eq!(error, omoba_career_store::career_store::ready::CLOSED);
                true
            }
            Reply::Rejected(id, _) if id == initial.result_id => {
                panic!("restore evidence was discarded")
            }
            Reply::Settled { result, .. } if result.result_id == initial.result_id => {
                panic!("spool invented an admission missing from SQL")
            }
            _ => false,
        });
        worker.send(Job::Flush(911));
        worker.wait_for(|r| matches!(r, Reply::Flushed(911, true)));
        let path = spool_path(&worker.outbox, &initial.result_id).unwrap();
        let preserved: PendingRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(preserved.recovered_live, !is_terminal);
        assert_eq!(
            preserved.recovery_allocation.as_ref().unwrap(),
            if is_terminal { &initial } else { &snapshot }
        );
        assert_eq!(
            runtime
                .block_on(store.profile(&profile.profile_id))
                .unwrap(),
            profile
        );
        let count: i64 = runtime
            .block_on(
                sqlx::query_scalar("SELECT count(*) FROM career_matches WHERE result_id=$1")
                    .bind(&initial.result_id)
                    .fetch_one(&pool),
            )
            .unwrap();
        assert_eq!(count, 0);
        // Explicit test-only restoration of the exact previously committed
        // immutable allocation; production public workers never call this API.
        runtime
            .block_on(store.ensure_allocation(initial.clone()))
            .unwrap();
        runtime.block_on(sqlx::query("UPDATE career_matches SET lease_until=clock_timestamp()-interval '1 second' WHERE result_id=$1")
            .bind(&initial.result_id).execute(&pool)).unwrap();
        worker.wait_for(|reply| match reply {
            Reply::Settled { result, .. } if result.result_id == initial.result_id => {
                assert_eq!(
                    result.outcome,
                    if is_terminal {
                        MatchOutcome::Completed
                    } else {
                        MatchOutcome::Interrupted
                    }
                );
                assert_eq!(result.participants[0].stats.damage_to_heroes, 73.5);
                true
            }
            Reply::Rejected(id, error) if id == initial.result_id => {
                panic!("restored admission refused: {error}")
            }
            _ => false,
        });
        wait_until(|| !path.exists());
        let saved = runtime
            .block_on(store.profile(&profile.profile_id))
            .unwrap();
        assert_eq!(saved.matches_played, profile.matches_played + 1);
        assert_eq!(saved.rating, profile.rating);
        assert_eq!(saved.progression_xp, profile.progression_xp);
        drop(worker);
        close_public_database(&runtime, store, pool, admin, &schema);
    }
}

#[test]
#[ignore = "requires real isolated PostgreSQL"]
fn public_consent_recovered_known_allocation_keeps_the_existing_resume_authority() {
    use omoba_career_store::career_store::ready::{Action, Seat};
    let (url, runtime, store, pool, admin, schema) = isolated_public_database();
    let (_, profile, _) = queue_profile(&runtime, &store);
    let mut initial = allocation(&profile);
    initial.gameplay_started = false;
    runtime
        .block_on(store.ready_reserve(
            &initial.result_id,
            shared::match_service::MatchPreference::BotPractice,
            vec![Seat {
                profile_id: profile.profile_id.clone(),
                session_id: "public-consent-fixture".into(),
            }],
        ))
        .unwrap();
    runtime
        .block_on(store.ready_update(&initial.result_id, Action::Publish))
        .unwrap();
    runtime
        .block_on(store.ready_update(
            &initial.result_id,
            Action::Answer {
                profile_id: profile.profile_id.clone(),
                session_id: "public-consent-fixture".into(),
                accepted: true,
            },
        ))
        .unwrap();
    runtime
        .block_on(store.start_public(initial.clone()))
        .unwrap();
    runtime.block_on(sqlx::query("UPDATE career_matches SET lease_until=clock_timestamp()-interval '1 second' WHERE result_id=$1").bind(&initial.result_id).execute(&pool)).unwrap();
    let outbox = std::env::temp_dir().join(format!("omoba-public-consent-{}", random_id::<16>()));
    write_record(
        &outbox,
        &PendingRecord {
            kind: RecordKind::Start,
            result: initial.clone(),
            recovery_allocation: None,
            recovered_live: false,
        },
    )
    .unwrap();
    let worker = OwnedWorker::spawn_public(url, outbox);
    worker.wait_for(|reply| match reply {
        Reply::Settled { result, .. } if result.result_id == initial.result_id => {
            assert_eq!(result.outcome, MatchOutcome::Interrupted);
            assert!(!result.gameplay_started);
            true
        }
        Reply::Rejected(id, error) if id == initial.result_id => {
            panic!("persisted allocation lost recovery: {error}")
        }
        _ => false,
    });
    assert_eq!(
        runtime
            .block_on(store.profile(&profile.profile_id))
            .unwrap(),
        profile
    );
    drop(worker);
    close_public_database(&runtime, store, pool, admin, &schema);
}
