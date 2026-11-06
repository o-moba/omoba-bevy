use super::*;
use shared::ranked::{SEASON_DURATION_MS, calibrated_tier, reset_rating};

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn season_close_archives_badges_resets_once_and_preserves_lifetime() {
    let f = Fixture::with_initial_season(Some(1000)).await;
    let mut before = Vec::new();
    for (n, rating) in [
        0, 999, 1000, 1499, 1500, 1999, 2000, 2499, 2500, 2999, 3000, 5000, 1750, 900,
    ]
    .into_iter()
    .enumerate()
    {
        let p = f.profile(n as u64 + 1).await;
        let games = if n == 12 {
            19
        } else if n == 13 {
            1
        } else {
            20
        };
        let completed = if n == 13 { 0 } else { games };
        sqlx::query("UPDATE career_profiles SET rating=$2,rated_matches=40,matches_played=55,wins=30,losses=25,progression_xp=12000,season_rated_matches=$3,season_wins=0,season_losses=$3,season_completed_matches=$4 WHERE profile_id=$1")
            .bind(&p.profile_id).bind(rating).bind(i64::from(games)).bind(i64::from(completed)).execute(&f.store.pool).await.unwrap();
        before.push(f.store.profile(&p.profile_id).await.unwrap());
    }
    assert!(
        f.store
            .close_ranked_season(2)
            .await
            .unwrap_err()
            .contains("changed")
    );
    let other = f.another_owner().await;
    let (one, two) = tokio::join!(f.store.close_ranked_season(1), other.close_ranked_season(1));
    let receipt = one.unwrap();
    assert_eq!(receipt, two.unwrap());
    assert_eq!(
        (
            receipt.season_id,
            receipt.archived_profiles,
            receipt.awarded_badges
        ),
        (1, 14, 13)
    );
    let current = f.store.ranked_season().await.unwrap();
    assert_eq!(current.season_id, receipt.next_season_id);
    assert_eq!(
        current.ends_at_ms - current.started_at_ms,
        SEASON_DURATION_MS
    );
    for (n, old) in before.iter().enumerate() {
        let after = f.store.profile(&old.profile_id).await.unwrap();
        assert_eq!(after.rating, reset_rating(old.rating));
        assert_eq!(
            (
                after.rated_matches,
                after.matches_played,
                after.wins,
                after.losses,
                after.progression_xp,
                after.completed_public_matches
            ),
            (
                old.rated_matches,
                old.matches_played,
                old.wins,
                old.losses,
                old.progression_xp,
                old.completed_public_matches
            )
        );
        let progress = after.ranked.as_ref().unwrap();
        assert_eq!(
            (
                progress.season_id,
                progress.rated_matches,
                progress.wins,
                progress.losses,
                progress.completed_matches
            ),
            (current.season_id, 0, 0, 0, 0)
        );
        assert!(after.newcomer() && after.ranked_unlocked());
        if n == 13 {
            assert!(progress.latest_badge.is_none());
        } else {
            let badge = progress.latest_badge.as_ref().unwrap();
            assert_eq!(
                (badge.season_id, badge.rating, badge.rated_matches),
                (1, old.rating, old.placement_matches())
            );
            assert_eq!(
                badge.tier,
                calibrated_tier(old.rating, old.placement_matches())
            );
        }
        let archived=sqlx::query("SELECT rating,rated_matches FROM career_ranked_archives WHERE season_id=1 AND profile_id=$1")
            .bind(&old.profile_id).fetch_one(&f.store.pool).await.unwrap();
        assert_eq!(archived.get::<i32, _>("rating"), old.rating);
        assert_eq!(
            archived.get::<i64, _>("rated_matches"),
            i64::from(old.placement_matches())
        );
    }
    assert_eq!(receipt, f.store.close_ranked_season(1).await.unwrap());
    let untouched = f.store.profile(&before[0].profile_id).await.unwrap();
    assert_eq!(untouched.rating, reset_rating(before[0].rating));
    let registered = f.fresh_profile(100).await;
    assert_eq!(
        registered.ranked.as_ref().unwrap().season_id,
        current.season_id
    );
    for query in [
        "UPDATE career_ranked_archives SET rating=0",
        "DELETE FROM career_ranked_closures",
        "DELETE FROM career_ranked_seasons",
    ] {
        assert!(sqlx::query(query).execute(&f.store.pool).await.is_err());
    }
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn active_season_cannot_be_closed_and_initial_migration_does_not_reset() {
    let f = Fixture::new().await;
    let p = f.profile(1).await;
    let current = f.store.ranked_season().await.unwrap();
    assert_eq!(
        current.ends_at_ms - current.started_at_ms,
        SEASON_DURATION_MS
    );
    assert!(
        f.store
            .close_ranked_season(current.season_id)
            .await
            .unwrap_err()
            .contains("not ended")
    );
    assert_eq!(f.store.profile(&p.profile_id).await.unwrap(), p);
    assert!(f.store.close_ranked_season(0).await.is_err());
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn failed_season_archive_rolls_back_without_reset_or_new_window() {
    let f = Fixture::with_initial_season(Some(1000)).await;
    let p = f.profile(1).await;
    sqlx::query("UPDATE career_profiles SET rating=2000,rated_matches=20,matches_played=20,season_rated_matches=20,season_losses=20 WHERE profile_id=$1")
        .bind(&p.profile_id).execute(&f.store.pool).await.unwrap();
    let before = f.store.profile(&p.profile_id).await.unwrap();
    // Explicit conflicting owner fixture forces failure after locks, before
    // any operation can commit. The archive's primary key remains authoritative.
    sqlx::query("INSERT INTO career_ranked_archives(season_id,profile_id,rating,rated_matches,wins,losses,completed_matches) VALUES(1,$1,2000,20,0,20,0)")
        .bind(&p.profile_id).execute(&f.store.pool).await.unwrap();
    assert!(f.store.close_ranked_season(1).await.is_err());
    assert_eq!(f.store.ranked_season().await.unwrap().season_id, 1);
    assert_eq!(f.store.profile(&p.profile_id).await.unwrap(), before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_ranked_closures")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn expired_season_refuses_new_ranked_but_unsettled_original_intent_blocks_close() {
    let f = Fixture::with_initial_season(Some(1000)).await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let completed = result(&a, &b, 1);
    assert_eq!(
        f.store.start(starting(&completed)).await.unwrap_err(),
        shared::career::error_code::RANKED_SEASON_ENDED
    );
    // Model an allocation already durably present before the migrated expired
    // window. Final unrated intent does not erase its original Ranked identity.
    let mut allocated = starting(&completed);
    allocated.ranked_season_id = Some(1);
    sqlx::query("INSERT INTO career_matches(result_id,server_epoch,match_id,owner_id,lease_until,allocation,checkpoint) VALUES($1,$2,$3,$4,clock_timestamp()+interval '90 seconds',$5,$5)")
        .bind(&allocated.result_id).bind(allocated.server_epoch.to_string()).bind(allocated.match_id.to_string()).bind(&f.store.owner).bind(Json(&allocated)).execute(&f.store.pool).await.unwrap();
    sqlx::query("INSERT INTO career_ranked_allocations(result_id,season_id) VALUES($1,1)")
        .bind(&allocated.result_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert!(
        f.store
            .close_ranked_season(1)
            .await
            .unwrap_err()
            .contains("Unsettled Ranked")
    );
    for (i, p) in allocated.participants.iter().enumerate() {
        sqlx::query("INSERT INTO career_participants(result_id,seat,player_id,profile_id) VALUES($1,$2,$3,$4)")
            .bind(&allocated.result_id).bind(i as i16).bind(p.player_id.to_string()).bind(&p.profile_id).execute(&f.store.pool).await.unwrap();
        sqlx::query("INSERT INTO career_active_profiles(profile_id,result_id) VALUES($1,$2)")
            .bind(&p.profile_id)
            .bind(&allocated.result_id)
            .execute(&f.store.pool)
            .await
            .unwrap();
    }
    let mut interrupted = completed;
    interrupted.ranked_season_id = Some(1);
    interrupted.rated = false;
    interrupted.outcome = MatchOutcome::Interrupted;
    interrupted.winner = None;
    interrupted.unrated_reason = Some("server_interrupted".into());
    f.store.stage(interrupted.clone()).await.unwrap();
    assert!(
        f.store
            .close_ranked_season(1)
            .await
            .unwrap_err()
            .contains("Unsettled Ranked")
    );
    f.store.settle(interrupted).await.unwrap();
    f.store.close_ranked_season(1).await.unwrap();
    // An old explicit season or legacy missing metadata cannot be replayed as
    // a new post-reset allocation, even though a new current window is active.
    let mut stale = result(&a, &b, 2);
    stale.ranked_season_id = Some(1);
    assert_eq!(
        f.store.start(starting(&stale)).await.unwrap_err(),
        shared::career::error_code::RANKED_SEASON_ENDED
    );
    stale.ranked_season_id = None;
    assert_eq!(
        f.store.start(starting(&stale)).await.unwrap_err(),
        shared::career::error_code::RANKED_SEASON_ENDED
    );
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn repeatable_read_registration_cannot_stamp_a_closed_snapshot() {
    let f = Fixture::with_initial_season(Some(1000)).await;
    let mut registration = f.store.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *registration)
        .await
        .unwrap();
    let old: i64 = sqlx::query_scalar(
        "SELECT season_id FROM career_ranked_seasons WHERE closed_at_ms IS NULL",
    )
    .fetch_one(&mut *registration)
    .await
    .unwrap();
    assert_eq!(old, 1);
    let receipt = f.store.close_ranked_season(1).await.unwrap();
    let error = sqlx::query(
        "INSERT INTO career_profiles(profile_id,nickname) VALUES($1,'OldSnapshot#0001')",
    )
    .bind("c".repeat(64))
    .execute(&mut *registration)
    .await
    .unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("40001")
    );
    registration.rollback().await.unwrap();
    let fresh = f.fresh_profile(100).await;
    assert_eq!(
        fresh.ranked.as_ref().unwrap().season_id,
        receipt.next_season_id
    );
    f.close().await;
}

async fn wait_for_lock(f: &Fixture, needle: &str) {
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid!=pg_backend_pid() AND wait_event_type='Lock' AND position($1 in query)>0)")
                .bind(needle).fetch_one(&f.admin).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("fixture must observe the actual blocked statement");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn concurrent_registration_waits_for_close_and_joins_the_new_window() {
    let f = Fixture::with_initial_season(Some(1000)).await;
    let mut guard = f.store.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(721946010)")
        .execute(&mut *guard)
        .await
        .unwrap();
    let operator = f.store.clone();
    let close = tokio::spawn(async move { operator.close_ranked_season(1).await });
    wait_for_lock(&f, "pg_advisory_xact_lock(721946010)").await;
    let registrar = f.store.clone();
    let signup = tokio::spawn(async move {
        registrar
            .authenticate(&format!("{:064x}", 303), "Concurrent#0303")
            .await
    });
    wait_for_lock(&f, "INSERT INTO career_profiles").await;
    guard.commit().await.unwrap();
    let receipt = close.await.unwrap().unwrap();
    let profile = signup.await.unwrap().unwrap();
    assert_eq!(
        profile.ranked.as_ref().unwrap().season_id,
        receipt.next_season_id
    );
    assert_eq!(profile.placement_matches(), 0);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn closing_without_a_replacement_window_is_rejected_at_commit() {
    let f = Fixture::with_initial_season(Some(1000)).await;
    let p = f.profile(1).await;
    let mut tx = f.store.pool.begin().await.unwrap();
    sqlx::query("UPDATE career_ranked_seasons SET closed_at_ms=ends_at_ms WHERE season_id=1")
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );
    assert_eq!(f.store.ranked_season().await.unwrap().season_id, 1);
    assert_eq!(f.store.profile(&p.profile_id).await.unwrap(), p);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OMOBA_TEST_DATABASE_URL and real PostgreSQL"]
async fn runtime_roles_can_play_and_read_but_cannot_roll_over_or_archive() {
    let f = Fixture::with_initial_season(Some(1000)).await;
    let game_role = format!(
        "season_game_{}_{}",
        std::process::id(),
        NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
    );
    let portal_role = format!(
        "season_portal_{}_{}",
        std::process::id(),
        NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
    );
    sqlx::raw_sql(&format!(r#"
        CREATE ROLE {game_role} NOLOGIN;
        CREATE ROLE {portal_role} NOLOGIN;
        GRANT USAGE ON SCHEMA {schema} TO {game_role},{portal_role};
        GRANT SELECT ON career_schema_version,career_profiles TO {game_role},{portal_role};
        GRANT SELECT,INSERT,UPDATE,DELETE ON career_profiles,career_matches,career_participants,career_active_profiles,career_friendships,career_presence TO {game_role};
        GRANT SELECT,INSERT(public_key,profile_id) ON career_keys TO {game_role};
        GRANT SELECT,INSERT ON career_abandon_events TO {game_role};
        GRANT SELECT,INSERT,UPDATE ON career_play_penalties TO {game_role};
        GRANT USAGE ON ALL SEQUENCES IN SCHEMA {schema} TO {game_role};
        -- Exact new permission delta from the prepared deployment patch.
        GRANT SELECT ON career_ranked_seasons TO {game_role},{portal_role};
        GRANT SELECT ON career_ranked_archives TO {portal_role};
        GRANT SELECT,INSERT ON career_ranked_allocations TO {game_role};
        REVOKE ALL ON SEQUENCE career_ranked_seasons_season_id_seq FROM {game_role},{portal_role};
    "#,schema=f.schema)).execute(&f.store.pool).await.unwrap();
    let url = std::env::var("OMOBA_TEST_DATABASE_URL").unwrap();
    let mut pools = Vec::new();
    for role in [&game_role, &portal_role] {
        let setup_path = format!("SET search_path TO {}", f.schema);
        let setup_role = format!("SET ROLE {role}");
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .after_connect(move |c, _| {
                let path = setup_path.clone();
                let role = setup_role.clone();
                Box::pin(async move {
                    sqlx::query(&path).execute(&mut *c).await?;
                    sqlx::query(&role).execute(&mut *c).await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        let runtime = CareerStore::runtime_from_pool(pool.clone()).await.unwrap();
        assert_eq!(runtime.ranked_season().await.unwrap().season_id, 1);
        for sql in [
            "UPDATE career_ranked_seasons SET closed_at_ms=ends_at_ms WHERE false",
            "INSERT INTO career_ranked_archives(season_id,profile_id,rating,rated_matches,wins,losses,completed_matches) SELECT 1,repeat('f',64),1000,1,0,1,1 WHERE false",
            "INSERT INTO career_ranked_closures(season_id,next_season_id,closed_at_ms,archived_profiles,awarded_badges) SELECT 1,2,0,0,0 WHERE false",
            "DELETE FROM career_ranked_allocations WHERE false",
            "SELECT nextval('career_ranked_seasons_season_id_seq')",
        ] {
            let error = sqlx::query(sql).execute(&pool).await.unwrap_err();
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("42501")
            );
        }
        assert!(runtime.close_ranked_season(1).await.is_err());
        pools.push((runtime, pool));
    }
    // Owner closes explicitly; ordinary authenticated gameplay then binds to
    // the new window using only the runtime's narrow allocation permission.
    let receipt = f.store.close_ranked_season(1).await.unwrap();
    let a = f.profile(700).await;
    let b = f.profile(701).await;
    let game = &pools[0].0;
    let signed_in = game
        .authenticate(&format!("{:064x}", 702), "RoleSignup#0702")
        .await
        .unwrap();
    assert_eq!(
        signed_in.ranked.as_ref().unwrap().season_id,
        receipt.next_season_id
    );
    // A caller-owned temp table must not redirect the SECURITY DEFINER trigger
    // to the closed old window. Use one connection so the shadow is certain.
    {
        let mut connection = pools[0].1.acquire().await.unwrap();
        sqlx::query(&format!("CREATE TEMP TABLE career_ranked_seasons AS SELECT * FROM {}.career_ranked_seasons WHERE season_id=1",f.schema))
            .execute(&mut *connection).await.unwrap();
        sqlx::query("UPDATE pg_temp.career_ranked_seasons SET closed_at_ms=NULL")
            .execute(&mut *connection)
            .await
            .unwrap();
        let stamped:i64=sqlx::query_scalar("INSERT INTO career_profiles(profile_id,nickname) VALUES($1,'ShadowProof#0703') RETURNING ranked_season_id")
            .bind("d".repeat(64)).fetch_one(&mut *connection).await.unwrap();
        assert_eq!(stamped as u64, receipt.next_season_id);
        sqlx::query("DROP TABLE pg_temp.career_ranked_seasons")
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    let mut completed = result(&a, &b, 700);
    completed.ranked_season_id = Some(receipt.next_season_id);
    game.start(starting(&completed)).await.unwrap();
    let saved = game.settle(completed).await.unwrap();
    assert!(saved.saved && saved.participants.iter().all(|p| p.rating.is_some()));
    let progress = game.profile(&a.profile_id).await.unwrap();
    assert_eq!(progress.placement_matches(), 1);
    assert!(pools[1].0.profile(&a.profile_id).await.is_ok());
    let archived: i64 = sqlx::query_scalar("SELECT count(*) FROM career_ranked_archives")
        .fetch_one(&pools[1].1)
        .await
        .unwrap();
    assert_eq!(archived, 0);
    for (_, pool) in &pools {
        pool.close().await;
    }
    // Both role names are freshly created by this fixture and own no objects.
    // Remove their grants before dropping only these disposable test roles.
    sqlx::raw_sql(&format!(
        "DROP OWNED BY {game_role},{portal_role}; DROP ROLE {game_role}; DROP ROLE {portal_role};"
    ))
    .execute(&f.admin)
    .await
    .unwrap();
    f.close().await;
}
