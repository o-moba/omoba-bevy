//! Possession of an existing game key authorizes a browser-bound, short-lived pairing.
use crate::*;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use shared::web_account::{SignedWebPair, WEB_SCOPES, WebPairChallenge, WebPairDecision};

pub struct Session {
    pub session_id: String,
    pub profile_id: String,
    pub created_at: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lookup {
    code: String,
    public_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Poll {
    pair_id: String,
    poll_secret: String,
}
fn credential_error() -> Error {
    Error(StatusCode::UNAUTHORIZED, "session_expired")
}
fn poll(app: &App, body: &[u8]) -> Result<(String, String)> {
    let p: Poll = parse(body)?;
    if crypto::decode::<32>(&p.pair_id).is_none() || crypto::decode::<32>(&p.poll_secret).is_none()
    {
        return Err(invalid());
    }
    Ok((
        p.pair_id,
        crypto::mac(&app.config.secret, "pair-poll", &p.poll_secret),
    ))
}
pub async fn create(app: &App, body: &[u8]) -> Result<Value> {
    let _: Empty = parse(body)?;
    let id = crypto::random::<32>();
    let secret = crypto::random::<32>();
    let nonce = crypto::random::<32>();
    // 40 bits of uniformly random human-readable entropy, independent of the poll secret.
    let mut random = [0; 8];
    getrandom::fill(&mut random)
        .map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "random_unavailable"))?;
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let code: String = random
        .iter()
        .map(|b| alphabet[usize::from(*b) % 32] as char)
        .collect();
    let expiry = now() + 300;
    sqlx::query("INSERT INTO portal.web_pairings(pair_id,code_hash,poll_hash,nonce,expires_at) VALUES($1,$2,$3,$4,$5)")
        .bind(&id).bind(crypto::mac(&app.config.secret,"pair-code",&code)).bind(crypto::mac(&app.config.secret,"pair-poll",&secret)).bind(nonce).bind(expiry).execute(&app.pool).await?;
    Ok(
        json!({"pair_id":id,"poll_secret":secret,"code":code,"expires_at":timestamp(&app.pool,expiry).await?}),
    )
}
pub async fn lookup(app: &App, body: &[u8]) -> Result<Value> {
    let p: Lookup = parse(body)?;
    let code = p.code.trim().to_ascii_uppercase();
    if code.len() != 8
        || !code.bytes().all(|b| b.is_ascii_alphanumeric())
        || crypto::decode::<32>(&p.public_key).is_none()
    {
        return Err(invalid());
    }
    app.rate(&p.public_key, "lookup-key", 20).await?;
    let row=sqlx::query("SELECT pair_id,nonce,poll_hash,expires_at FROM portal.web_pairings WHERE code_hash=$1 AND expires_at>$2 AND state='pending'")
        .bind(crypto::mac(&app.config.secret,"pair-code",&code)).bind(now()).fetch_optional(&app.pool).await?.ok_or_else(forbidden)?;
    let challenge = WebPairChallenge {
        pair_id: row_text(&row, "pair_id")?,
        nonce: row_text(&row, "nonce")?,
        origin: app.config.origin.clone(),
        public_key: p.public_key,
        browser_binding: row_text(&row, "poll_hash")?,
        scopes: WEB_SCOPES.map(str::to_owned).to_vec(),
        expires_at: row.try_get::<i64, _>("expires_at")?.to_string(),
    };
    Ok(json!({"challenge":challenge}))
}
pub async fn decide(app: &App, body: &[u8], decision: WebPairDecision) -> Result<Value> {
    let proof: SignedWebPair = parse(body)?;
    let c = &proof.challenge;
    if proof.decision != decision {
        return Err(invalid());
    }
    c.validate(&app.config.origin, &c.public_key, now() as u64)
        .map_err(|_| invalid())?;
    let key = VerifyingKey::from_bytes(&crypto::decode::<32>(&c.public_key).ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    let signature =
        Signature::from_bytes(&crypto::decode::<64>(&proof.signature).ok_or_else(invalid)?);
    key.verify_strict(&c.signing_bytes(decision), &signature)
        .map_err(|_| forbidden())?;
    app.rate(&c.public_key, "pair-decision", 10).await?;
    let mut tx = app.pool.begin().await?;
    let row = sqlx::query("SELECT * FROM portal.web_pairings WHERE pair_id=$1 FOR UPDATE")
        .bind(&c.pair_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(forbidden)?;
    if row_text(&row, "nonce")? != c.nonce
        || row_text(&row, "poll_hash")? != c.browser_binding
        || row.try_get::<i64, _>("expires_at")?.to_string() != c.expires_at
    {
        return Err(forbidden());
    }
    let profile: String = sqlx::query_scalar(
        "SELECT profile_id FROM career_keys WHERE public_key=$1 AND revoked_at IS NULL",
    )
    .bind(&c.public_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(forbidden)?;
    let state = if decision == WebPairDecision::Approve {
        "approved"
    } else {
        "denied"
    };
    let previous = row_text(&row, "state")?;
    if previous != "pending" {
        if previous == state
            && row.try_get::<Option<String>, _>("approved_key")?.as_deref() == Some(&c.public_key)
        {
            return Ok(json!({"status":state}));
        }
        return Err(conflict());
    }
    sqlx::query(
        "UPDATE portal.web_pairings SET state=$2,profile_id=$3,approved_key=$4,authorization_version=2 WHERE pair_id=$1",
    )
    .bind(&c.pair_id)
    .bind(state)
    .bind(&profile)
    .bind(&c.public_key)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,$2)")
        .bind(profile)
        .bind(if decision == WebPairDecision::Approve {
            "pair_approved"
        } else {
            "pair_denied"
        })
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({"status":state}))
}
pub async fn status(app: &App, body: &[u8]) -> Result<Value> {
    let (id, hash) = poll(app, body)?;
    let row=sqlx::query("SELECT state,expires_at,delivery_until FROM portal.web_pairings WHERE pair_id=$1 AND poll_hash=$2").bind(id).bind(hash).fetch_optional(&app.pool).await?.ok_or_else(forbidden)?;
    let current = row_text(&row, "state")?;
    let state = if current == "consumed"
        && row
            .try_get::<Option<i64>, _>("delivery_until")?
            .is_some_and(|t| t > now())
    {
        current
    } else if row.try_get::<i64, _>("expires_at")? <= now() {
        "expired".into()
    } else {
        current
    };
    Ok(json!({"state":state}))
}
pub async fn cancel(app: &App, body: &[u8]) -> Result<Value> {
    let (id, hash) = poll(app, body)?;
    sqlx::query("UPDATE portal.web_pairings SET state='cancelled',delivery=NULL WHERE pair_id=$1 AND poll_hash=$2 AND state IN('pending','approved')").bind(id).bind(hash).execute(&app.pool).await?;
    Ok(json!({"status":"cancelled"}))
}
pub async fn complete(app: &App, body: &[u8]) -> Result<Value> {
    let (id, hash) = poll(app, body)?;
    let mut tx = app.pool.begin().await?;
    let row = sqlx::query(
        "SELECT * FROM portal.web_pairings WHERE pair_id=$1 AND poll_hash=$2 FOR UPDATE",
    )
    .bind(&id)
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(forbidden)?;
    let context = format!("{id}:{hash}");
    let state = row_text(&row, "state")?;
    let current = now();
    if state == "consumed" {
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.web_sessions WHERE session_id=$1 AND authorization_version=2 AND NOT revoked AND expires_at>$2 AND last_seen>$2-86400)").bind(row.try_get::<Option<String>,_>("session_id")?).bind(current).fetch_one(&mut *tx).await?;
        if row
            .try_get::<Option<i64>, _>("delivery_until")?
            .is_some_and(|t| t > current)
            && active
        {
            let delivery: Vec<u8> = row
                .try_get::<Option<Vec<u8>>, _>("delivery")?
                .ok_or_else(conflict)?;
            let token =
                crypto::open(&app.config.secret, &context, &delivery).map_err(|_| conflict())?;
            return Ok(json!({"session_token":token}));
        }
        return Err(conflict());
    }
    if state != "approved"
        || row.try_get::<i16, _>("authorization_version")? != 2
        || row.try_get::<i64, _>("expires_at")? <= current
    {
        return Err(conflict());
    }
    let profile: String = row
        .try_get::<Option<String>, _>("profile_id")?
        .ok_or_else(conflict)?;
    // Serialize the per-account cap across independent approved browser pairings.
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(&profile)
        .fetch_one(&mut *tx)
        .await?;
    let key_active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_keys WHERE public_key=$1 AND profile_id=$2 AND revoked_at IS NULL)")
        .bind(row.try_get::<Option<String>,_>("approved_key")?).bind(&profile).fetch_one(&mut *tx).await?;
    if !key_active {
        return Err(forbidden());
    }
    let active:i64=sqlx::query_scalar("SELECT count(*) FROM portal.web_sessions WHERE profile_id=$1 AND NOT revoked AND expires_at>$2 AND last_seen>$2-86400")
        .bind(&profile).bind(current).fetch_one(&mut *tx).await?;
    if active >= 20 {
        return Err(Error(StatusCode::CONFLICT, "session_limit"));
    }
    let sid = crypto::random::<32>();
    let token = crypto::random::<32>();
    let delivery = crypto::seal(&app.config.secret, &context, &token)
        .map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "crypto_unavailable"))?;
    sqlx::query("INSERT INTO portal.web_sessions(session_id,profile_id,token_hash,created_at,last_seen,expires_at,authorization_version) VALUES($1,$2,$3,$4,$4,$5,2)").bind(&sid).bind(&profile).bind(crypto::mac(&app.config.secret,"session-token",&token)).bind(current).bind(current+7*86400).execute(&mut *tx).await?;
    sqlx::query("UPDATE portal.web_pairings SET state='consumed',session_id=$2,delivery=$3,delivery_until=$4 WHERE pair_id=$1").bind(id).bind(sid).bind(delivery).bind(current+60).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,'session_created')")
        .bind(profile)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({"session_token":token}))
}
pub async fn session(app: &App, headers: &HeaderMap) -> Result<Session> {
    let token = headers
        .get("authorization")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(credential_error)?;
    if crypto::decode::<32>(token).is_none() {
        return Err(credential_error());
    }
    let row=sqlx::query("UPDATE portal.web_sessions SET last_seen=$2 WHERE token_hash=$1 AND authorization_version=2 AND NOT revoked AND expires_at>$2 AND last_seen>$2-86400 RETURNING session_id,profile_id,created_at")
        .bind(crypto::mac(&app.config.secret,"session-token",token)).bind(now()).fetch_optional(&app.pool).await?.ok_or_else(credential_error)?;
    Ok(Session {
        session_id: row_text(&row, "session_id")?,
        profile_id: row_text(&row, "profile_id")?,
        created_at: row.try_get("created_at")?,
    })
}
pub async fn sessions(app: &App, s: &Session) -> Result<Value> {
    let rows=sqlx::query("SELECT session_id,label,created_at,last_seen,expires_at FROM portal.web_sessions WHERE profile_id=$1 AND NOT revoked AND expires_at>$2 AND last_seen>$2-86400 ORDER BY created_at DESC LIMIT 100").bind(&s.profile_id).bind(now()).fetch_all(&app.pool).await?;
    let mut items = Vec::new();
    for row in rows {
        let id = row_text(&row, "session_id")?;
        items.push(json!({"current":id==s.session_id,"session_id":id,"label":row_text(&row,"label")?,"created_at":timestamp(&app.pool,row.try_get("created_at")?).await?,"last_seen":timestamp(&app.pool,row.try_get("last_seen")?).await?,"expires_at":timestamp(&app.pool,row.try_get("expires_at")?).await?}));
    }
    Ok(json!({"sessions":items}))
}
pub async fn logout(app: &App, s: &Session) -> Result<Value> {
    revoke(app, s, &s.session_id).await
}
pub async fn revoke(app: &App, s: &Session, id: &str) -> Result<Value> {
    if crypto::decode::<32>(id).is_none() {
        return Err(invalid());
    }
    if id != s.session_id && s.created_at < now() - 600 {
        return Err(Error(StatusCode::FORBIDDEN, "fresh_confirmation_required"));
    }
    sqlx::query(
        "UPDATE portal.web_sessions SET revoked=true WHERE session_id=$1 AND profile_id=$2",
    )
    .bind(id)
    .bind(&s.profile_id)
    .execute(&app.pool)
    .await?;
    app.audit(Some(&s.profile_id), "session_revoked").await?;
    Ok(json!({"status":"revoked"}))
}
