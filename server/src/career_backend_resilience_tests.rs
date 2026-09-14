//! Actual worker/PostgreSQL failure-path tests. Never substitutes a mock DB.
use super::*;
use shared::{HeroClass, map::Team};
use std::thread::JoinHandle;

struct OwnedWorker {
    jobs: Option<SyncSender<Job>>,
    replies: Receiver<Reply>,
    handle: Option<JoinHandle<()>>,
    outbox: PathBuf,
}
impl OwnedWorker {
    fn spawn(url: String, full_replies: bool) -> Self {
        let outbox = std::env::temp_dir().join(format!("omoba-resilience-{}", random_id::<16>()));
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
        let handle = std::thread::spawn(move || worker(url, path, receiver, sender));
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
fn allocation(profile: &ProfileSummary) -> MatchResult {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    MatchResult {
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
    let profile = runtime
        .block_on(store.authenticate(&random_id::<32>(), "Tombstone tester"))
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
