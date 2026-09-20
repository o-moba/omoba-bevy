//! Explicit, expiring native enrollment and one-use recovery. Keys never change owner.
use crate::*;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use shared::device_account::{DeviceAction, SignedDeviceEnrollment};
use sqlx::{Postgres, Transaction, postgres::PgRow};

fn proof(app: &App, body: &[u8], action: DeviceAction) -> Result<SignedDeviceEnrollment> {
    let p: SignedDeviceEnrollment = parse(body)?;
    let e = &p.enrollment;
    e.validate(&app.config.origin, &e.public_key, now() as u64)
        .map_err(|_| invalid())?;
    if p.action != action
        || (action != DeviceAction::Complete && p.target_profile.is_some())
        || (action != DeviceAction::Recover && p.recovery_code.is_some())
        || (action == DeviceAction::Complete
            && p.target_profile
                .as_deref()
                .is_none_or(|v| crypto::decode::<32>(v).is_none()))
        || (action == DeviceAction::Recover
            && p.recovery_code
                .as_deref()
                .is_none_or(|v| crypto::decode::<32>(v).is_none()))
    {
        return Err(invalid());
    }
    let key = VerifyingKey::from_bytes(&crypto::decode::<32>(&e.public_key).ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    let signature = Signature::from_bytes(&crypto::decode::<64>(&p.signature).ok_or_else(invalid)?);
    key.verify_strict(
        &e.signing_bytes(
            action,
            p.target_profile.as_deref(),
            p.recovery_code.as_deref(),
        ),
        &signature,
    )
    .map_err(|_| forbidden())?;
    Ok(p)
}
fn matches(row: &PgRow, p: &SignedDeviceEnrollment) -> Result<()> {
    let e = &p.enrollment;
    if row_text(row, "public_key")? != e.public_key
        || row_text(row, "origin")? != e.origin
        || row_text(row, "label")? != e.label
        || row.try_get::<i64, _>("expires_at")?.to_string() != e.expires_at
    {
        return Err(forbidden());
    }
    Ok(())
}
async fn lock_profile(tx: &mut Transaction<'_, Postgres>, id: &str) -> Result<()> {
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(forbidden)?;
    Ok(())
}
async fn fresh(tx: &mut Transaction<'_, Postgres>, s: &auth::Session) -> Result<()> {
    lock_profile(tx, &s.profile_id).await?;
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.web_sessions WHERE session_id=$1 AND profile_id=$2 AND authorization_version=2 AND NOT revoked AND expires_at>$3 AND created_at>=$3-600)")
        .bind(&s.session_id).bind(&s.profile_id).bind(now()).fetch_one(&mut **tx).await?;
    if !valid {
        return Err(Error(StatusCode::FORBIDDEN, "fresh_confirmation_required"));
    }
    Ok(())
}
async fn audit(tx: &mut Transaction<'_, Postgres>, profile: &str, event: &str) -> Result<()> {
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,$2)")
        .bind(profile)
        .bind(event)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn render(app: &App, row: &PgRow) -> Result<Value> {
    let profile: Option<String> = row.try_get("profile_id")?;
    let nickname: Option<String> = match &profile {
        Some(id) => {
            sqlx::query_scalar("SELECT nickname FROM career_profiles WHERE profile_id=$1")
                .bind(id)
                .fetch_optional(&app.pool)
                .await?
        }
        None => None,
    };
    Ok(json!({"state":row_text(row,"state")?,"profile_id":profile,"nickname":nickname,"code":null}))
}
pub async fn create(app: &App, body: &[u8]) -> Result<Value> {
    let p = proof(app, body, DeviceAction::Create)?;
    let e = &p.enrollment;
    app.rate(&e.public_key, "device-create-key", 5).await?;
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946002))")
        .bind(&e.public_key)
        .execute(&mut *tx)
        .await?;
    let existing: Option<(String, bool)> = sqlx::query_as(
        "SELECT profile_id,revoked_at IS NOT NULL FROM career_keys WHERE public_key=$1",
    )
    .bind(&e.public_key)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((profile, revoked)) = existing {
        if revoked {
            return Err(forbidden());
        }
        // Recover a completed enrollment after a crash before local activation. This
        // reveals only the account whose key was just proven, and never moves a key.
        let nickname: String =
            sqlx::query_scalar("SELECT nickname FROM career_profiles WHERE profile_id=$1")
                .bind(&profile)
                .fetch_one(&mut *tx)
                .await?;
        return Ok(
            json!({"state":"consumed","profile_id":profile,"nickname":nickname,"code":null}),
        );
    }
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes)
        .map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "random_unavailable"))?;
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let code: String = bytes
        .iter()
        .map(|b| alphabet[usize::from(*b) % alphabet.len()] as char)
        .collect();
    let sealed =
        crypto::seal(&app.config.secret, &e.enrollment_id, &code).map_err(|_| invalid())?;
    sqlx::query("INSERT INTO portal.device_enrollments(enrollment_id,public_key,code_hash,code_delivery,label,origin,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(enrollment_id) DO NOTHING")
        .bind(&e.enrollment_id).bind(&e.public_key).bind(crypto::mac(&app.config.secret,"device-code",&code)).bind(sealed).bind(&e.label).bind(&e.origin).bind(e.expires_at.parse::<i64>().map_err(|_|invalid())?).execute(&mut *tx).await?;
    let row = sqlx::query("SELECT * FROM portal.device_enrollments WHERE enrollment_id=$1")
        .bind(&e.enrollment_id)
        .fetch_one(&mut *tx)
        .await?;
    matches(&row, &p)?;
    let code = crypto::open(
        &app.config.secret,
        &e.enrollment_id,
        &row.try_get::<Vec<u8>, _>("code_delivery")?,
    )
    .map_err(|_| invalid())?;
    tx.commit().await?;
    let mut value = render(app, &row).await?;
    value["code"] = json!(code);
    Ok(value)
}
pub async fn status(app: &App, body: &[u8]) -> Result<Value> {
    let p = proof(app, body, DeviceAction::Status)?;
    let row = sqlx::query("SELECT * FROM portal.device_enrollments WHERE enrollment_id=$1")
        .bind(&p.enrollment.enrollment_id)
        .fetch_optional(&app.pool)
        .await?
        .ok_or_else(forbidden)?;
    matches(&row, &p)?;
    render(app, &row).await
}
pub async fn complete(app: &App, body: &[u8]) -> Result<Value> {
    let p = proof(app, body, DeviceAction::Complete)?;
    let target = p.target_profile.as_deref().ok_or_else(invalid)?;
    let mut tx = app.pool.begin().await?;
    // Same absent-key lock used by game registration prevents a racing new profile.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946002))")
        .bind(&p.enrollment.public_key)
        .execute(&mut *tx)
        .await?;
    lock_profile(&mut tx, target).await?;
    let row =
        sqlx::query("SELECT * FROM portal.device_enrollments WHERE enrollment_id=$1 FOR UPDATE")
            .bind(&p.enrollment.enrollment_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(forbidden)?;
    matches(&row, &p)?;
    if row.try_get::<Option<String>, _>("profile_id")?.as_deref() != Some(target)
        || !matches!(row_text(&row, "state")?.as_str(), "approved" | "consumed")
    {
        return Err(conflict());
    }
    let existing: Option<(String, bool)> = sqlx::query_as(
        "SELECT profile_id,revoked_at IS NOT NULL FROM career_keys WHERE public_key=$1",
    )
    .bind(&p.enrollment.public_key)
    .fetch_optional(&mut *tx)
    .await?;
    match existing {
        Some((id, false)) if id == target && row_text(&row, "state")? == "consumed" => {}
        Some(_) => return Err(Error(StatusCode::CONFLICT, "device_key_already_registered")),
        None => {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM career_keys WHERE profile_id=$1 AND revoked_at IS NULL",
            )
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
            if count >= 16 {
                return Err(Error(StatusCode::CONFLICT, "device_limit"));
            }
            sqlx::query("INSERT INTO career_keys(public_key,profile_id,label) VALUES($1,$2,$3)")
                .bind(&p.enrollment.public_key)
                .bind(target)
                .bind(&p.enrollment.label)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE portal.device_enrollments SET state='consumed' WHERE enrollment_id=$1",
            )
            .bind(&p.enrollment.enrollment_id)
            .execute(&mut *tx)
            .await?;
            audit(&mut tx, target, "device_linked").await?;
        }
    }
    let nickname: String =
        sqlx::query_scalar("SELECT nickname FROM career_profiles WHERE profile_id=$1")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(json!({"state":"consumed","profile_id":target,"nickname":nickname,"code":null}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Code {
    code: String,
}
pub async fn lookup(app: &App, s: &auth::Session, body: &[u8]) -> Result<Value> {
    let p: Code = parse(body)?;
    let code = p.code.trim().to_ascii_uppercase();
    if code.len() != 8 || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(invalid());
    }
    let mut tx = app.pool.begin().await?;
    fresh(&mut tx, s).await?;
    let row=sqlx::query("SELECT enrollment_id,public_key,label,expires_at FROM portal.device_enrollments WHERE code_hash=$1 AND expires_at>$2 AND state='pending'")
        .bind(crypto::mac(&app.config.secret,"device-code",&code)).bind(now()).fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    let nickname: String =
        sqlx::query_scalar("SELECT nickname FROM career_profiles WHERE profile_id=$1")
            .bind(&s.profile_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(
        json!({"enrollment_id":row_text(&row,"enrollment_id")?,"public_key":row_text(&row,"public_key")?,"label":row_text(&row,"label")?,"expires_at":timestamp(&app.pool,row.try_get("expires_at")?).await?,"nickname":nickname,"profile_id":s.profile_id}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    enrollment_id: String,
    public_key: String,
}
pub async fn approve(app: &App, s: &auth::Session, body: &[u8]) -> Result<Value> {
    let p: Approval = parse(body)?;
    if crypto::decode::<32>(&p.enrollment_id).is_none()
        || crypto::decode::<32>(&p.public_key).is_none()
    {
        return Err(invalid());
    }
    let mut tx = app.pool.begin().await?;
    fresh(&mut tx, s).await?;
    let affected=sqlx::query("UPDATE portal.device_enrollments SET state='approved',profile_id=$3,approved_at=$4 WHERE enrollment_id=$1 AND public_key=$2 AND expires_at>$4 AND state='pending'")
        .bind(&p.enrollment_id).bind(&p.public_key).bind(&s.profile_id).bind(now()).execute(&mut *tx).await?.rows_affected();
    if affected != 1 {
        return Err(conflict());
    }
    audit(&mut tx, &s.profile_id, "device_approved").await?;
    tx.commit().await?;
    Ok(json!({"status":"approved"}))
}
pub async fn list(app: &App, s: &auth::Session) -> Result<Value> {
    let rows=sqlx::query("SELECT public_key,label,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created,to_char(revoked_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS revoked FROM career_keys WHERE profile_id=$1 ORDER BY (revoked_at IS NULL) DESC,created_at DESC LIMIT 100")
        .bind(&s.profile_id).fetch_all(&app.pool).await?;
    let devices=rows.iter().map(|row|Ok(json!({"public_key":row_text(row,"public_key")?,"label":row_text(row,"label")?,"created_at":row_text(row,"created")?,"revoked_at":row.try_get::<Option<String>,_>("revoked")?}))).collect::<Result<Vec<_>>>()?;
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal.recovery_codes WHERE profile_id=$1 AND consumed_at IS NULL",
    )
    .bind(&s.profile_id)
    .fetch_one(&app.pool)
    .await?;
    Ok(json!({"devices":devices,"recovery_codes_remaining":remaining}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
pub async fn recovery_codes(app: &App, s: &auth::Session, body: &[u8]) -> Result<Value> {
    let _: Empty = parse(body)?;
    app.rate(&s.profile_id, "recovery-rotate", 2).await?;
    let mut tx = app.pool.begin().await?;
    fresh(&mut tx, s).await?;
    sqlx::query("UPDATE portal.recovery_codes SET consumed_at=$2 WHERE profile_id=$1 AND consumed_at IS NULL").bind(&s.profile_id).bind(now()).execute(&mut *tx).await?;
    let codes: Vec<String> = (0..8).map(|_| crypto::random::<32>()).collect();
    for code in &codes {
        sqlx::query(
            "INSERT INTO portal.recovery_codes(code_hash,profile_id,created_at) VALUES($1,$2,$3)",
        )
        .bind(crypto::mac(&app.config.secret, "recovery-code", code))
        .bind(&s.profile_id)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    }
    audit(&mut tx, &s.profile_id, "recovery_codes_rotated").await?;
    tx.commit().await?;
    Ok(
        json!({"recovery_codes":codes,"warning":"Each code works once. New codes replace previous codes."}),
    )
}
pub async fn recover(app: &App, body: &[u8]) -> Result<Value> {
    let p = proof(app, body, DeviceAction::Recover)?;
    app.rate(&p.enrollment.public_key, "recovery-attempt", 5)
        .await?;
    let hash = crypto::mac(
        &app.config.secret,
        "recovery-code",
        p.recovery_code.as_deref().ok_or_else(invalid)?,
    );
    let mut tx = app.pool.begin().await?;
    let profile: String =
        sqlx::query_scalar("SELECT profile_id FROM portal.recovery_codes WHERE code_hash=$1")
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(forbidden)?;
    lock_profile(&mut tx, &profile).await?;
    let row =
        sqlx::query("SELECT * FROM portal.device_enrollments WHERE enrollment_id=$1 FOR UPDATE")
            .bind(&p.enrollment.enrollment_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(forbidden)?;
    matches(&row, &p)?;
    let code = sqlx::query(
        "SELECT consumed_at,enrollment_id FROM portal.recovery_codes WHERE code_hash=$1 FOR UPDATE",
    )
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await?;
    if code.try_get::<Option<i64>, _>("consumed_at")?.is_some() {
        if code
            .try_get::<Option<String>, _>("enrollment_id")?
            .as_deref()
            != Some(&p.enrollment.enrollment_id)
            || row.try_get::<Option<String>, _>("profile_id")?.as_deref() != Some(&profile)
        {
            return Err(forbidden());
        }
        tx.commit().await?;
        return render(app, &row).await;
    }
    if row_text(&row, "state")? != "pending" {
        return Err(conflict());
    }
    sqlx::query(
        "UPDATE portal.recovery_codes SET consumed_at=$2,enrollment_id=$3 WHERE code_hash=$1",
    )
    .bind(&hash)
    .bind(now())
    .bind(&p.enrollment.enrollment_id)
    .execute(&mut *tx)
    .await?;
    let row=sqlx::query("UPDATE portal.device_enrollments SET state='approved',profile_id=$2,approved_at=$3 WHERE enrollment_id=$1 RETURNING *").bind(&p.enrollment.enrollment_id).bind(&profile).bind(now()).fetch_one(&mut *tx).await?;
    // A recovered account must reauthorize its browsers; lost-device sessions are not trusted.
    sqlx::query("UPDATE portal.web_sessions SET revoked=true WHERE profile_id=$1")
        .bind(&profile)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, &profile, "account_recovered").await?;
    tx.commit().await?;
    render(app, &row).await
}
pub async fn revoke(app: &App, s: &auth::Session, key: &str) -> Result<Value> {
    if crypto::decode::<32>(key).is_none() {
        return Err(invalid());
    }
    let mut tx = app.pool.begin().await?;
    fresh(&mut tx, s).await?;
    let remaining:i64=sqlx::query_scalar("SELECT count(*) FROM career_keys WHERE profile_id=$1 AND public_key!=$2 AND revoked_at IS NULL").bind(&s.profile_id).bind(key).fetch_one(&mut *tx).await?;
    let recovery:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.recovery_codes WHERE profile_id=$1 AND consumed_at IS NULL)").bind(&s.profile_id).fetch_one(&mut *tx).await?;
    if remaining == 0 && !recovery {
        return Err(Error(
            StatusCode::CONFLICT,
            "recovery_required_before_last_device",
        ));
    }
    let changed=sqlx::query("UPDATE career_keys SET revoked_at=clock_timestamp() WHERE profile_id=$1 AND public_key=$2 AND revoked_at IS NULL").bind(&s.profile_id).bind(key).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(forbidden());
    }
    sqlx::query("UPDATE portal.web_sessions SET revoked=true WHERE profile_id=$1")
        .bind(&s.profile_id)
        .execute(&mut *tx)
        .await?;
    // Pending approvals from a revoked browser must not authorize a later device.
    sqlx::query("DELETE FROM portal.device_enrollments WHERE profile_id=$1 AND state='approved' AND NOT EXISTS(SELECT 1 FROM portal.recovery_codes r WHERE r.enrollment_id=portal.device_enrollments.enrollment_id)").bind(&s.profile_id).execute(&mut *tx).await?;
    audit(&mut tx, &s.profile_id, "device_revoked").await?;
    tx.commit().await?;
    Ok(json!({"status":"revoked"}))
}
