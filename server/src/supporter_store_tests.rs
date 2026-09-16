//! Run against an isolated database after Account API migrations.
use super::*;
use shared::supporter::AuraStyle;

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL database with Account API migrations"]
async fn supporter_runtime_checks_grants_expiry_revocation_and_never_changes_progress() {
    let url = std::env::var("OMOBA_TEST_DATABASE_URL").expect("isolated database URL required");
    let store = CareerStore::connect(&url).await.unwrap();
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).unwrap();
    let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let profile = store.authenticate(&key, "AuraTester").await.unwrap();
    assert!(
        !store
            .supporter_status(&profile.profile_id)
            .await
            .unwrap()
            .active
    );
    assert!(
        store
            .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Solar))
            .await
            .is_err()
    );
    let now: i64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM now())::bigint")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let grant = |period: String, from: i64, until: i64| {
        sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version) VALUES('solana',$1,$2,$3,$4,1)").bind(period).bind(profile.profile_id.clone()).bind(from).bind(until)
    };
    let period = format!("test-{key}");
    grant(period.clone(), now - 100, now - 1)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Lunar))
            .await
            .is_err()
    );
    sqlx::query("UPDATE portal.supporter_grants SET valid_until=$1 WHERE period_id=$2")
        .bind(now + 3600)
        .bind(&period)
        .execute(&store.pool)
        .await
        .unwrap();
    // Future/revoked receipts must never crowd an active grant out of a
    // bounded status payload. The live grant expires before all these rows.
    sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,valid_from,valid_until,event_version) SELECT 'solana',$1 || '-' || n::text,$2,$3,$4+n,1 FROM generate_series(1,101) AS n")
        .bind(format!("future-{key}")).bind(&profile.profile_id).bind(now+7200).bind(now+10800).execute(&store.pool).await.unwrap();
    let active = store
        .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Verdant))
        .await
        .unwrap();
    assert!(active.active);
    assert_eq!(active.equipped_aura, Some(AuraStyle::Verdant));
    assert_eq!(active.active_until, Some(now + 3600));
    let hidden = store
        .equip_supporter_aura(&profile.profile_id, None)
        .await
        .unwrap();
    assert!(hidden.active);
    assert_eq!(hidden.equipped_aura, None);
    sqlx::query("UPDATE portal.supporter_grants SET revoked_at=$1 WHERE period_id=$2")
        .bind(now)
        .bind(&period)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(
        store
            .equip_supporter_aura(&profile.profile_id, Some(AuraStyle::Solar))
            .await
            .is_err()
    );
    assert!(
        !store
            .supporter_status(&profile.profile_id)
            .await
            .unwrap()
            .active
    );
    assert_eq!(store.profile(&profile.profile_id).await.unwrap(), profile);
    // Leave no fixture rows; this database belongs only to the current task.
    sqlx::query("DELETE FROM portal.supporter_preferences WHERE profile_id=$1")
        .bind(&profile.profile_id)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM portal.supporter_grants WHERE profile_id=$1")
        .bind(&profile.profile_id)
        .execute(&store.pool)
        .await
        .unwrap();
}
