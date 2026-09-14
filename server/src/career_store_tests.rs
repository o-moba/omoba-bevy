//! Real PostgreSQL integration fixtures. No fallback/in-memory database is used.
//! Run with OMOBA_TEST_DATABASE_URL=... cargo test -p server career_store -- --ignored.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SCHEMA: AtomicU64 = AtomicU64::new(1);
struct Fixture {
    store: CareerStore,
    admin: PgPool,
    schema: String,
}
impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("OMOBA_TEST_DATABASE_URL")
            .expect("Set an isolated PostgreSQL test database URL");
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect actual PostgreSQL");
        let schema = format!(
            "career_test_{}_{}",
            std::process::id(),
            NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
        );
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let search_path = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |connection, _| {
                let schema = search_path.clone();
                Box::pin(async move {
                    sqlx::query(&format!("SET search_path TO {schema}"))
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SET lock_timeout='5s'")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        let store = CareerStore::from_pool(pool).await.unwrap();
        Self {
            store,
            admin,
            schema,
        }
    }
    async fn another_owner(&self) -> CareerStore {
        CareerStore::from_pool(self.store.pool.clone())
            .await
            .unwrap()
    }
    async fn profile(&self, n: u64) -> ProfileSummary {
        self.store
            .authenticate(&format!("{n:064x}"), &format!("Player-{n}"))
            .await
            .unwrap()
    }
    async fn expire(&self, id: &str) {
        sqlx::query("UPDATE career_matches SET lease_until=clock_timestamp()-interval '1 second' WHERE result_id=$1")
            .bind(id).execute(&self.store.pool).await.unwrap();
    }
    async fn close(self) {
        self.store.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
fn participant(profile: &ProfileSummary, player_id: u64, team: Team) -> ParticipantResult {
    ParticipantResult {
        is_bot: false,
        player_id,
        profile_id: Some(profile.profile_id.clone()),
        nickname: profile.nickname.clone(),
        team,
        hero_class: shared::HeroClass::Ranger,
        character: "archer".into(),
        avatar: None,
        sprite_character: None,
        stats: shared::career::MatchStats {
            final_level: 1,
            ..Default::default()
        },
        disconnected: false,
        rating: None,
        progression_xp_gained: 0,
    }
}
fn result(a: &ProfileSummary, b: &ProfileSummary, n: u64) -> MatchResult {
    MatchResult {
        result_id: format!("fixture-result-{n}"),
        server_epoch: u64::MAX,
        match_id: n,
        started_at_ms: 1000,
        ended_at_ms: 2000,
        duration_ms: 1000,
        map_profile: "verdant".into(),
        ruleset: RULESET.into(),
        outcome: MatchOutcome::Completed,
        winner: Some(Team::Green),
        rated: true,
        unrated_reason: None,
        participants: vec![
            participant(a, 1, Team::Green),
            participant(b, 2, Team::Blue),
        ],
        saved: false,
    }
}
fn starting(result: &MatchResult) -> MatchResult {
    let mut start = result.clone();
    start.outcome = MatchOutcome::Interrupted;
    start.winner = None;
    start.ended_at_ms = 0;
    start.duration_ms = 0;
    start
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_identity_reopen_and_explicit_rename() {
    let f = Fixture::new().await;
    let key = format!("{:064x}", 1);
    let (a, b) = tokio::join!(
        f.store.authenticate(&key, "小明"),
        f.store.authenticate(&key, "Other")
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a, b);
    assert_ne!(a.profile_id, key);
    assert!(valid_profile_id(&a.profile_id));
    let other = f.another_owner().await;
    assert_eq!(other.authenticate(&key, "Ignored").await.unwrap(), a);
    let renamed = other.rename(&a.profile_id, "  Дмитрий-7  ").await.unwrap();
    assert_eq!(renamed.nickname, "Дмитрий-7");
    assert_eq!(
        f.store.authenticate(&key, "Ignored").await.unwrap(),
        renamed
    );
    assert!(f.store.rename(&a.profile_id, "bad\nname").await.is_err());
    assert!(f.store.authenticate("bad-key", "Name").await.is_err());
    let p2 = f.profile(2).await;
    assert!(
        f.store
            .public_profile(&p2.profile_id, &a.profile_id)
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .public_profile(&a.profile_id, &a.profile_id)
            .await
            .unwrap(),
        renamed
    );
    let version: i32 = sqlx::query_scalar("SELECT version FROM career_schema_version")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(version, 1);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_distinct_owners_can_share_legacy_epoch_and_round_metadata() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let c = f.profile(3).await;
    let d = f.profile(4).await;
    let other = f.another_owner().await;
    let first = result(&a, &b, 1);
    let mut second = result(&c, &d, 1);
    second.result_id = other.new_result_id().await.unwrap();
    let (left, right) = tokio::join!(
        f.store.start(starting(&first)),
        other.start(starting(&second))
    );
    left.unwrap();
    right.unwrap();
    let (left, right) = tokio::join!(f.store.settle(first), other.settle(second));
    let left = left.unwrap();
    let right = right.unwrap();
    assert_ne!(left.result_id, right.result_id);
    assert_eq!(
        (left.server_epoch, left.match_id),
        (right.server_epoch, right.match_id)
    );
    assert!(
        f.store
            .detail(&a.profile_id, &right.result_id)
            .await
            .is_err()
    );
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_allocation_rechecks_current_experience_and_rating_after_remote_match() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let opponent = f.profile(3).await;
    sqlx::query("UPDATE career_profiles SET rated_matches=19,matches_played=19,wins=9,losses=10")
        .execute(&f.store.pool)
        .await
        .unwrap();
    let server_b = f.another_owner().await;
    let cached_a = server_b.profile(&a.profile_id).await.unwrap();
    let cached_b = server_b.profile(&b.profile_id).await.unwrap();
    assert!(cached_a.newcomer() && cached_b.newcomer());
    let on_a = result(&cached_a, &opponent, 1);
    f.store.start(starting(&on_a)).await.unwrap();
    f.store.settle(on_a).await.unwrap();
    let current_a = f.store.profile(&a.profile_id).await.unwrap();
    assert_eq!(current_a.rated_matches, 20);
    assert!(!current_a.newcomer());

    let stale = result(&cached_a, &cached_b, 2);
    let error = "Queued ratings changed. Clear the queue and try again.";
    assert_eq!(server_b.start(starting(&stale)).await.unwrap_err(), error);
    assert_eq!(
        server_b
            .ensure_allocation(starting(&stale))
            .await
            .unwrap_err(),
        error
    );
    let allocations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM career_matches WHERE result_id=$1")
            .bind(&stale.result_id)
            .fetch_one(&f.store.pool)
            .await
            .unwrap();
    assert_eq!(
        allocations, 0,
        "failed validation rolls back the allocation"
    );
    let seats: i64 = sqlx::query_scalar("SELECT count(*) FROM career_active_profiles")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(seats, 0, "failed validation cannot reserve either account");

    sqlx::query("UPDATE career_profiles SET rated_matches=20,matches_played=20,losses=11,rating=$2 WHERE profile_id=$1")
        .bind(&b.profile_id).bind(current_a.rating + 301).execute(&f.store.pool).await.unwrap();
    assert_eq!(server_b.start(starting(&stale)).await.unwrap_err(), error);
    sqlx::query("UPDATE career_profiles SET rating=$2 WHERE profile_id=$1")
        .bind(&b.profile_id)
        .bind(current_a.rating + 300)
        .execute(&f.store.pool)
        .await
        .unwrap();
    server_b.start(starting(&stale)).await.unwrap();
    server_b.settle(stale).await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_ensure_allocation_replays_missing_start_and_late_unrated_checkpoint() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let completed = result(&a, &b, 1);
    // The terminal spool survived while the original Start never reached PG.
    assert!(f.store.settle(completed.clone()).await.is_err());
    let old_start = starting(&completed);
    let reopened = f.another_owner().await;
    reopened.ensure_allocation(old_start.clone()).await.unwrap();
    let saved = reopened.settle(completed.clone()).await.unwrap();
    // Completed allocations remain replayable by a new worker without stealing
    // ownership or rewriting the stored receipt.
    f.store.ensure_allocation(old_start).await.unwrap();
    assert_eq!(f.store.settle(completed).await.unwrap(), saved);

    for use_original_start in [false, true] {
        let mut completed = result(&a, &b, if use_original_start { 3 } else { 2 });
        completed.rated = false;
        completed.unrated_reason = Some("development".into());
        let mut original = starting(&completed);
        original.participants.truncate(1);
        f.store.start(original.clone()).await.unwrap();
        let mut checkpoint = starting(&completed);
        checkpoint.ended_at_ms = 1500;
        checkpoint.duration_ms = 500;
        checkpoint.participants[0].stats.damage_to_heroes = 20.0;
        f.store.checkpoint(checkpoint.clone()).await.unwrap();
        let mut forged = checkpoint.clone();
        forged.participants[0].team = Team::Blue;
        f.expire(&completed.result_id).await;
        assert!(reopened.ensure_allocation(forged).await.is_err());
        let owner: String =
            sqlx::query_scalar("SELECT owner_id FROM career_matches WHERE result_id=$1")
                .bind(&completed.result_id)
                .fetch_one(&f.store.pool)
                .await
                .unwrap();
        assert_eq!(
            owner, f.store.owner,
            "invalid metadata cannot adopt an allocation"
        );
        let recovery = if use_original_start {
            original
        } else {
            checkpoint
        };
        reopened.ensure_allocation(recovery).await.unwrap();
        assert!(
            f.store
                .ensure_allocation(starting(&completed))
                .await
                .is_err(),
            "a live owner is not stolen"
        );
        completed.participants[0].stats.damage_to_heroes = 40.0;
        let saved = reopened.settle(completed).await.unwrap();
        assert_eq!(saved.participants.len(), 2);
        assert_eq!(saved.participants[0].stats.damage_to_heroes, 40.0);
    }
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_bulk_presence_is_atomic_validates_foreign_owner_and_deduplicates() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let other = f.another_owner().await;
    let game = result(&a, &b, 1);
    f.store.start(starting(&game)).await.unwrap();
    let batch = vec![
        (a.profile_id.clone(), None),
        (b.profile_id.clone(), Some(game.result_id.clone())),
    ];
    assert!(other.touch_presences(&batch).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_presence")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "foreign batch must not partially refresh valid Online entries"
    );
    let invalid = vec![(a.profile_id.clone(), None), ("0".repeat(64), None)];
    assert!(f.store.touch_presences(&invalid).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_presence")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let mut repeated = batch.clone();
    repeated.push((b.profile_id.clone(), None));
    repeated.push(batch[0].clone());
    f.store.touch_presences(&repeated).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_presence")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let playing: Option<String> =
        sqlx::query_scalar("SELECT result_id FROM career_presence WHERE profile_id=$1")
            .bind(&b.profile_id)
            .fetch_one(&f.store.pool)
            .await
            .unwrap();
    assert_eq!(playing.as_deref(), Some(game.result_id.as_str()));
    f.store.touch_presences(&[]).await.unwrap();
    let mut conflicting = repeated;
    conflicting.push((b.profile_id.clone(), Some("different-result".into())));
    assert!(f.store.touch_presences(&conflicting).await.is_err());
    assert!(
        f.store
            .touch_presences(&[("bad-profile".into(), None)])
            .await
            .is_err()
    );
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_atomic_duplicate_settlement_and_conflicting_intent() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let mut final_result = result(&a, &b, 1);
    assert!(
        f.store.settle(final_result.clone()).await.is_err(),
        "must durably start first"
    );
    f.store.start(starting(&final_result)).await.unwrap();
    f.store.start(starting(&final_result)).await.unwrap();
    let mut checkpoint = starting(&final_result);
    checkpoint.duration_ms = 500;
    checkpoint.ended_at_ms = 1500;
    checkpoint.participants[0].stats.damage_to_heroes = 321.5;
    f.store.checkpoint(checkpoint).await.unwrap();
    final_result.participants[0].stats.damage_to_heroes = 555.5;
    let (one, two) = tokio::join!(
        f.store.settle(final_result.clone()),
        f.store.settle(final_result.clone())
    );
    let saved = one.unwrap();
    assert_eq!(saved, two.unwrap());
    assert!(saved.saved);
    assert_eq!(saved.participants[0].rating.as_ref().unwrap().delta, 16);
    assert_eq!(saved.participants[0].progression_xp_gained, 150);
    assert_eq!(saved.participants[1].progression_xp_gained, 100);
    let profiles = (
        f.store.profile(&a.profile_id).await.unwrap(),
        f.store.profile(&b.profile_id).await.unwrap(),
    );
    assert_eq!(
        (
            profiles.0.rating,
            profiles.0.matches_played,
            profiles.0.wins,
            profiles.0.progression_xp
        ),
        (1016, 1, 1, 150)
    );
    assert_eq!(
        (
            profiles.1.rating,
            profiles.1.matches_played,
            profiles.1.losses
        ),
        (984, 1, 1)
    );
    let reopened = f.another_owner().await;
    assert_eq!(reopened.settle(final_result.clone()).await.unwrap(), saved);
    assert_eq!(
        reopened
            .detail(&a.profile_id, &saved.result_id)
            .await
            .unwrap(),
        saved
    );
    final_result.winner = Some(Team::Blue);
    assert!(reopened.settle(final_result).await.is_err());
    assert_eq!(f.store.profile(&a.profile_id).await.unwrap(), profiles.0);
    assert_eq!(f.store.profile(&b.profile_id).await.unwrap(), profiles.1);
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM career_active_profiles")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(active, 0);
    assert!(
        sqlx::query("UPDATE career_matches SET result='{}'::jsonb")
            .execute(&f.store.pool)
            .await
            .is_err()
    );
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_failure_after_first_profile_update_rolls_back_and_recovers_intent() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let final_result = result(&a, &b, 1);
    f.store.start(starting(&final_result)).await.unwrap();
    // Player 1 is updated first by settlement; fail on player 2 after that write.
    let last = &b.profile_id;
    sqlx::raw_sql(&format!("CREATE FUNCTION fixture_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.profile_id='{last}' AND NEW.matches_played>OLD.matches_played THEN RAISE EXCEPTION 'fixture fault' USING ERRCODE='23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER fixture_fail BEFORE UPDATE ON career_profiles FOR EACH ROW EXECUTE FUNCTION fixture_fail();"))
        .execute(&f.store.pool).await.unwrap();
    assert!(f.store.settle(final_result.clone()).await.is_err());
    assert_eq!(f.store.profile(&a.profile_id).await.unwrap(), a);
    assert_eq!(f.store.profile(&b.profile_id).await.unwrap(), b);
    let state: String = sqlx::query_scalar("SELECT status FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(state, "pending");
    sqlx::raw_sql("DROP TRIGGER fixture_fail ON career_profiles; DROP FUNCTION fixture_fail();")
        .execute(&f.store.pool)
        .await
        .unwrap();
    f.expire(&final_result.result_id).await;
    let reopened = f.another_owner().await;
    let recovered = reopened.recover_expired().await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].saved);
    assert_eq!(recovered[0].outcome, MatchOutcome::Completed);
    assert_eq!(
        f.store.profile(&a.profile_id).await.unwrap().matches_played,
        1
    );
    assert!(reopened.recover_expired().await.unwrap().is_empty());
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_live_ownership_fencing_active_seats_and_interrupted_recovery() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let final_result = result(&a, &b, 1);
    f.store.start(starting(&final_result)).await.unwrap();
    let other = f.another_owner().await;
    assert!(other.adopt_expired(&final_result.result_id).await.is_err());
    assert!(other.settle(final_result.clone()).await.is_err());
    assert!(other.recover_expired().await.unwrap().is_empty());
    assert!(
        other.start(starting(&result(&a, &b, 2))).await.is_err(),
        "one profile cannot occupy two allocations"
    );
    f.expire(&final_result.result_id).await;
    other.adopt_expired(&final_result.result_id).await.unwrap();
    f.store.heartbeat().await.unwrap();
    assert!(
        f.store.settle(final_result.clone()).await.is_err(),
        "old worker is fenced after takeover"
    );
    let generation: i64 = sqlx::query_scalar("SELECT generation FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(generation, 2);
    f.expire(&final_result.result_id).await;
    let recovered = other.recover_expired().await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].outcome, MatchOutcome::Interrupted);
    assert!(recovered[0].winner.is_none());
    assert!(!recovered[0].rated);
    assert!(
        recovered[0]
            .participants
            .iter()
            .all(|p| p.rating.is_none() && p.progression_xp_gained == 0)
    );
    let p = f.store.profile(&a.profile_id).await.unwrap();
    assert_eq!(
        (p.rating, p.rated_matches, p.wins, p.progression_xp),
        (1000, 0, 0, 0)
    );
    assert!(
        other.settle(final_result).await.is_err(),
        "cannot rewrite interrupted history as victory"
    );
    f.close().await;
}

#[test]
fn career_store_rated_policy_rejects_profile_bearing_bots_at_start_and_finish() {
    let a = ProfileSummary::new("a".repeat(64), "Player-A".into());
    let b = ProfileSummary::new("b".repeat(64), "Player-B".into());
    let mut rated = result(&a, &b, 1);
    assert!(validate_rated(&starting(&rated)).is_ok());
    rated.participants[1].is_bot = true;
    assert!(rated.participants[1].profile_id.is_some());
    assert!(validate_rated(&starting(&rated)).is_err());
    assert!(normalize_terminal(rated.clone()).is_err());
    rated.rated = false;
    rated.unrated_reason = Some("practice".into());
    assert!(normalize_terminal(rated).is_ok());
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_roster_policy_unrated_append_and_invalid_result_rejection() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let rated = result(&a, &b, 1);
    let mut bot = rated.clone();
    bot.participants[1].is_bot = true;
    assert!(f.store.start(starting(&bot)).await.is_err());
    assert!(f.store.ensure_allocation(starting(&bot)).await.is_err());
    let allocations: i64 = sqlx::query_scalar("SELECT count(*) FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(
        allocations, 0,
        "bot rejection cannot leave a durable rated allocation"
    );
    f.store.start(starting(&rated)).await.unwrap();
    assert!(f.store.settle(bot).await.is_err());
    let mut changed = rated.clone();
    changed.participants[0].team = Team::Blue;
    assert!(f.store.settle(changed).await.is_err());
    let mut changed = rated.clone();
    changed.participants[0].stats.damage_taken = f64::NAN;
    assert!(f.store.settle(changed).await.is_err());
    let mut changed = rated.clone();
    changed.participants[0].nickname = "Different".into();
    assert!(f.store.settle(changed).await.is_err());
    f.store.settle(rated).await.unwrap();
    let mut unrated = result(&a, &b, 2);
    unrated.rated = false;
    unrated.unrated_reason = Some("development".into());
    let mut initial = starting(&unrated);
    initial.participants.truncate(1);
    f.store.start(initial).await.unwrap();
    let mut changed_role = starting(&unrated);
    changed_role.participants[0].is_bot = true;
    assert!(
        f.store.checkpoint(changed_role).await.is_err(),
        "an unrated checkpoint cannot change a frozen human into a bot"
    );
    f.store.checkpoint(starting(&unrated)).await.unwrap();
    let before = f.store.profile(&a.profile_id).await.unwrap();
    let saved = f.store.settle(unrated).await.unwrap();
    assert!(
        saved
            .participants
            .iter()
            .all(|p| p.rating.is_none() && p.progression_xp_gained == 0)
    );
    let after = f.store.profile(&a.profile_id).await.unwrap();
    assert_eq!(before.rating, after.rating);
    assert_eq!(before.progression_xp, after.progression_xp);
    let mut invalid = result(&a, &b, 3);
    invalid.ruleset = "custom".into();
    assert!(f.store.start(starting(&invalid)).await.is_err());
    invalid.ruleset = RULESET.into();
    invalid.participants[1].profile_id = Some(a.profile_id.clone());
    assert!(f.store.start(starting(&invalid)).await.is_err());
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_history_keyset_authorization_and_frozen_names() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let outsider = f.profile(3).await;
    for n in 1..=12 {
        let result = result(&a, &b, n);
        f.store.start(starting(&result)).await.unwrap();
        f.store.settle(result).await.unwrap();
    }
    f.store.rename(&a.profile_id, "Renamed").await.unwrap();
    let (first, next) = f.store.history(&a.profile_id, None).await.unwrap();
    assert_eq!(first.len(), 10);
    assert!(next.is_some());
    let added = result(&a, &b, 13);
    f.store.start(starting(&added)).await.unwrap();
    f.store.settle(added).await.unwrap();
    let (second, next) = f.store.history(&a.profile_id, next).await.unwrap();
    assert_eq!(second.len(), 2);
    assert!(next.is_none());
    assert!(
        second
            .iter()
            .all(|s| !first.iter().any(|p| p.result_id == s.result_id))
    );
    assert!(
        f.store
            .history(&outsider.profile_id, None)
            .await
            .unwrap()
            .0
            .is_empty()
    );
    assert!(
        f.store
            .detail(&outsider.profile_id, &first[0].result_id)
            .await
            .is_err()
    );
    let detail = f
        .store
        .detail(&a.profile_id, &first[0].result_id)
        .await
        .unwrap();
    assert_eq!(detail.participants[0].nickname, a.nickname);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn career_store_social_reverse_requests_idempotent_actions_and_cross_owner_presence() {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let unrelated = f.profile(3).await;
    let other = f.another_owner().await;
    assert!(
        f.store
            .request_friend(&a.profile_id, &a.profile_id)
            .await
            .is_err()
    );
    let (ab, ba) = tokio::join!(
        f.store.request_friend(&a.profile_id, &b.profile_id),
        other.request_friend(&b.profile_id, &a.profile_id)
    );
    assert_ne!(ab.is_ok(), ba.is_ok(), "reverse requests never auto-accept");
    let (sender, recipient) = if ab.is_ok() { (&a, &b) } else { (&b, &a) };
    assert_eq!(
        f.store
            .public_profile(&sender.profile_id, &recipient.profile_id)
            .await
            .unwrap(),
        *recipient
    );
    assert_eq!(
        other
            .public_profile(&recipient.profile_id, &sender.profile_id)
            .await
            .unwrap(),
        *sender
    );
    assert!(
        f.store
            .public_profile(&unrelated.profile_id, &sender.profile_id)
            .await
            .is_err()
    );
    assert!(
        f.store
            .public_profile(&recipient.profile_id, &unrelated.profile_id)
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .request_friend(&sender.profile_id, &recipient.profile_id)
            .await
            .unwrap()
            .outgoing
            .len(),
        1
    );
    assert!(
        f.store
            .accept_friend(&sender.profile_id, &recipient.profile_id)
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .accept_friend(&recipient.profile_id, &sender.profile_id)
            .await
            .unwrap()
            .friends
            .len(),
        1
    );
    assert_eq!(
        f.store
            .accept_friend(&recipient.profile_id, &sender.profile_id)
            .await
            .unwrap()
            .friends
            .len(),
        1
    );
    assert_eq!(
        other
            .public_profile(&a.profile_id, &b.profile_id)
            .await
            .unwrap()
            .profile_id,
        b.profile_id
    );
    assert_eq!(
        other.friends(&a.profile_id).await.unwrap().friends[0].presence,
        FriendPresence::Offline
    );
    f.store.touch_presence(&b.profile_id, None).await.unwrap();
    assert_eq!(
        other.friends(&a.profile_id).await.unwrap().friends[0].presence,
        FriendPresence::Online
    );
    let game = result(&a, &b, 1);
    f.store.start(starting(&game)).await.unwrap();
    f.store
        .touch_presence(&b.profile_id, Some(&game.result_id))
        .await
        .unwrap();
    assert_eq!(
        other.friends(&a.profile_id).await.unwrap().friends[0].presence,
        FriendPresence::Playing
    );
    assert!(
        other
            .touch_presence(&b.profile_id, Some(&game.result_id))
            .await
            .is_err()
    );
    sqlx::query("UPDATE career_presence SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(
        other.friends(&a.profile_id).await.unwrap().friends[0].presence,
        FriendPresence::Offline
    );
    f.store.settle(game).await.unwrap();
    for _ in 0..2 {
        assert!(
            f.store
                .friend_action(&a.profile_id, &b.profile_id, FriendAction::Remove)
                .await
                .unwrap()
                .friends
                .is_empty()
        );
    }
    assert!(
        f.store
            .public_profile(&a.profile_id, &b.profile_id)
            .await
            .is_err()
    );
    assert!(
        other
            .public_profile(&b.profile_id, &a.profile_id)
            .await
            .is_err()
    );
    f.store
        .request_friend(&a.profile_id, &b.profile_id)
        .await
        .unwrap();
    assert!(
        f.store
            .friend_action(&a.profile_id, &b.profile_id, FriendAction::Reject)
            .await
            .is_err()
    );
    for _ in 0..2 {
        assert!(
            f.store
                .friend_action(&b.profile_id, &a.profile_id, FriendAction::Reject)
                .await
                .unwrap()
                .incoming
                .is_empty()
        );
    }
    f.store
        .request_friend(&a.profile_id, &b.profile_id)
        .await
        .unwrap();
    for _ in 0..2 {
        assert!(
            f.store
                .friend_action(&a.profile_id, &b.profile_id, FriendAction::Cancel)
                .await
                .unwrap()
                .outgoing
                .is_empty()
        );
    }
    f.close().await;
}
