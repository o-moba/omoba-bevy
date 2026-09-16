//! Provider receipts are verified outside the client trust boundary, then recorded atomically.
pub mod solana;
use crate::*;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use shared::supporter::{
    AuraStyle, NativeSupporterAction, SignedNativeSupporterRequest, SupporterGrantSummary,
    SupporterStatus,
};

#[derive(Clone, Default)]
pub struct BillingConfig {
    pub apple: Option<AppleConfig>,
    pub solana: Option<solana::SolanaConfig>,
}
#[derive(Clone)]
pub struct AppleConfig {
    pub verifier_url: String,
    pub secret: String,
    pub product_id: String,
    pub environment: String,
}
impl BillingConfig {
    pub fn from_env() -> std::result::Result<Self, String> {
        let vars = [
            "OMOBA_APPLE_VERIFIER_URL",
            "OMOBA_APPLE_VERIFIER_SECRET",
            "OMOBA_APPLE_PRODUCT_ID",
            "OMOBA_APPLE_ENVIRONMENT",
        ];
        let values: Vec<_> = vars.iter().map(|k| std::env::var(k).ok()).collect();
        let apple = if values.iter().all(Option::is_none) {
            None
        } else {
            if values.iter().any(Option::is_none) {
                return Err("Incomplete Apple provider configuration".into());
            }
            let v: Vec<String> = values.into_iter().map(Option::unwrap).collect();
            let url: reqwest::Url = v[0].parse().map_err(|_| "Invalid Apple verifier URL")?;
            if url.scheme() != "http"
                || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
                || url.path() != "/"
                || url.query().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || crypto::decode::<32>(&v[1]).is_none()
                || !matches!(v[3].as_str(), "Sandbox" | "Production")
                || v[2].is_empty()
                || v[2].len() > 200
            {
                return Err("Invalid Apple verifier configuration".into());
            }
            Some(AppleConfig {
                verifier_url: v[0].trim_end_matches('/').into(),
                secret: v[1].clone(),
                product_id: v[2].clone(),
                environment: v[3].clone(),
            })
        };
        Ok(Self {
            apple,
            solana: solana::SolanaConfig::from_env()?,
        })
    }
}
pub fn unavailable() -> Error {
    Error(
        StatusCode::SERVICE_UNAVAILABLE,
        "payment_provider_unavailable",
    )
}
pub fn payment_invalid() -> Error {
    Error(StatusCode::BAD_REQUEST, "payment_invalid")
}
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()
        .expect("HTTP client")
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedEvent {
    pub event_id: String,
    pub provider: String,
    pub period_id: String,
    pub original_transaction_id: Option<String>,
    pub app_account_token: Option<String>,
    pub product_id: Option<String>,
    pub environment: Option<String>,
    pub valid_from: i64,
    pub valid_until: i64,
    pub revoked_at: Option<i64>,
    pub event_version: i64,
    pub renewal_enabled: Option<bool>,
}
impl VerifiedEvent {
    fn validate(&self) -> Result<()> {
        if !matches!(self.provider.as_str(), "apple" | "solana")
            || self.event_id.is_empty()
            || self.event_id.len() > 256
            || self.period_id.is_empty()
            || self.period_id.len() > 256
            || self.valid_from < 0
            || self.valid_until <= self.valid_from
            || self.valid_until - self.valid_from > 400 * 86400
            || self.event_version < 0
            || self.revoked_at.is_some_and(|t| t < 0)
        {
            return Err(payment_invalid());
        }
        Ok(())
    }
}
/// Call only with provider-verified input. No HTTP route deserializes this directly.
pub async fn apply_event(app: &App, profile: &str, event: &VerifiedEvent) -> Result<bool> {
    let mut tx = app.pool.begin().await?;
    let changed = apply_event_tx(&mut tx, profile, event).await?;
    tx.commit().await?;
    Ok(changed)
}
async fn apply_event_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile: &str,
    event: &VerifiedEvent,
) -> Result<bool> {
    event.validate()?;
    // Serialize by account plus provider period. Ownership can never be reassigned by restore.
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(profile)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(forbidden)?;
    if let Some(original) = &event.original_transaction_id {
        let owner:Option<String>=sqlx::query_scalar("SELECT profile_id FROM portal.supporter_grants WHERE provider=$1 AND original_transaction_id=$2 LIMIT 1").bind(&event.provider).bind(original).fetch_optional(&mut **tx).await?;
        if owner.is_some_and(|id| id != profile) {
            return Err(forbidden());
        }
        // Different accounts cannot race claiming different periods of the same original subscription.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946104))")
            .bind(format!("{}:{original}", event.provider))
            .execute(&mut **tx)
            .await?;
        let owner:Option<String>=sqlx::query_scalar("SELECT profile_id FROM portal.supporter_grants WHERE provider=$1 AND original_transaction_id=$2 LIMIT 1").bind(&event.provider).bind(original).fetch_optional(&mut **tx).await?;
        if owner.is_some_and(|id| id != profile) {
            return Err(forbidden());
        }
    }
    let payload = serde_json::to_value(event).map_err(|_| invalid())?;
    let hash = crypto::hash(&serde_json::to_vec(event).map_err(|_| invalid())?);
    let inserted=sqlx::query("INSERT INTO portal.supporter_events(provider,event_id,profile_id,period_id,event_version,payload_hash,payload,received_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING")
        .bind(&event.provider).bind(&event.event_id).bind(profile).bind(&event.period_id).bind(event.event_version).bind(&hash).bind(payload).bind(now()).execute(&mut **tx).await?.rows_affected();
    if inserted == 0 {
        let (owner,previous):(String,String)=sqlx::query_as("SELECT profile_id,payload_hash FROM portal.supporter_events WHERE provider=$1 AND event_id=$2").bind(&event.provider).bind(&event.event_id).fetch_one(&mut **tx).await?;
        if owner != profile || previous != hash {
            return Err(conflict());
        }
        return Ok(false);
    }
    let existing=sqlx::query("SELECT profile_id,event_version,revoked_at FROM portal.supporter_grants WHERE provider=$1 AND period_id=$2 FOR UPDATE").bind(&event.provider).bind(&event.period_id).fetch_optional(&mut **tx).await?;
    if let Some(row) = existing {
        if row_text(&row, "profile_id")? != profile {
            return Err(forbidden());
        }
        if event.event_version <= row.try_get::<i64, _>("event_version")? {
            // Transaction JWS can arrive before a renewal snapshot with the same
            // verified timestamp. Fill unknown renewal state, never overwrite a
            // known cancellation with an absent or equally old snapshot.
            let enriched = if event.event_version == row.try_get::<i64, _>("event_version")? {
                sqlx::query("UPDATE portal.supporter_grants SET renewal_enabled=$3 WHERE provider=$1 AND period_id=$2 AND renewal_enabled IS NULL AND $3::boolean IS NOT NULL").bind(&event.provider).bind(&event.period_id).bind(event.renewal_enabled).execute(&mut **tx).await?.rows_affected() > 0
            } else {
                false
            };
            // A delayed, verified refund still revokes this period. It cannot roll back
            // dates or renewal state from a newer provider snapshot.
            if let Some(revoked) = event.revoked_at {
                sqlx::query("UPDATE portal.supporter_grants SET revoked_at=COALESCE(revoked_at,$3) WHERE provider=$1 AND period_id=$2").bind(&event.provider).bind(&event.period_id).bind(revoked).execute(&mut **tx).await?;
                return Ok(true);
            }
            return Ok(enriched);
        }
        // Revocation is terminal for this purchased period even if delayed/replayed renewals arrive.
        sqlx::query("UPDATE portal.supporter_grants SET valid_from=$3,valid_until=$4,revoked_at=COALESCE(revoked_at,$5),renewal_enabled=COALESCE($6,renewal_enabled),event_version=$7 WHERE provider=$1 AND period_id=$2")
            .bind(&event.provider).bind(&event.period_id).bind(event.valid_from).bind(event.valid_until).bind(event.revoked_at).bind(event.renewal_enabled).bind(event.event_version).execute(&mut **tx).await?;
    } else {
        sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,original_transaction_id,valid_from,valid_until,revoked_at,renewal_enabled,event_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(&event.provider).bind(&event.period_id).bind(profile).bind(&event.original_transaction_id).bind(event.valid_from).bind(event.valid_until).bind(event.revoked_at).bind(event.renewal_enabled).bind(event.event_version).execute(&mut **tx).await?;
    }
    sqlx::query("INSERT INTO portal.supporter_preferences(profile_id,aura) VALUES($1,'solar') ON CONFLICT DO NOTHING").bind(profile).execute(&mut **tx).await?;
    Ok(true)
}
pub async fn status(app: &App, profile: &str) -> Result<SupporterStatus> {
    let current = now();
    let rows=sqlx::query("SELECT provider,valid_from,valid_until,revoked_at,renewal_enabled FROM portal.supporter_grants WHERE profile_id=$1 ORDER BY (revoked_at IS NULL AND valid_from<=$2 AND valid_until>$2) DESC,valid_until DESC LIMIT 100").bind(profile).bind(current).fetch_all(&app.pool).await?;
    let grants = rows
        .iter()
        .map(|r| {
            Ok(SupporterGrantSummary {
                provider: row_text(r, "provider")?,
                valid_from: r.try_get("valid_from")?,
                valid_until: r.try_get("valid_until")?,
                revoked: r.try_get::<Option<i64>, _>("revoked_at")?.is_some(),
                renewal_enabled: r.try_get("renewal_enabled")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let active_until = grants
        .iter()
        .filter(|g| !g.revoked && g.valid_from <= current && g.valid_until > current)
        .map(|g| g.valid_until)
        .max();
    let aura: Option<String> =
        sqlx::query_scalar("SELECT aura FROM portal.supporter_preferences WHERE profile_id=$1")
            .bind(profile)
            .fetch_optional(&app.pool)
            .await?
            .flatten();
    Ok(SupporterStatus {
        active: active_until.is_some(),
        equipped_aura: if active_until.is_some() {
            aura.as_deref().and_then(AuraStyle::from_id)
        } else {
            None
        },
        active_until,
        grants,
    })
}
pub async fn dashboard(app: &App, profile: &str) -> Result<Value> {
    let apple = match &app.billing.apple {
        Some(c) => json!({"available":true,"product_id":c.product_id,"environment":c.environment}),
        None => json!({"available":false}),
    };
    let solana = match &app.billing.solana {
        Some(c) => {
            json!({"available":true,"amount":solana::decimal_amount(c.amount,c.decimals),"asset":"USDC","duration_days":30,"network":c.network})
        }
        None => json!({"available":false,"duration_days":30}),
    };
    Ok(
        json!({"supporter":status(app,profile).await?,"apple":apple,"solana":solana,"catalog":AuraStyle::ALL.map(|a|json!({"id":a,"name":a.label()}))}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Equip {
    aura: Option<AuraStyle>,
}
pub async fn equip(app: &App, profile: &str, body: &[u8]) -> Result<Value> {
    let p: Equip = parse(body)?;
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(profile)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(forbidden)?;
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.supporter_grants WHERE profile_id=$1 AND valid_from<=$2 AND valid_until>$2 AND revoked_at IS NULL)").bind(profile).bind(now()).fetch_one(&mut *tx).await?;
    if p.aura.is_some() && !active {
        return Err(Error(StatusCode::FORBIDDEN, "supporter_required"));
    }
    sqlx::query("INSERT INTO portal.supporter_preferences(profile_id,aura) VALUES($1,$2) ON CONFLICT(profile_id) DO UPDATE SET aura=EXCLUDED.aura").bind(profile).bind(p.aura.map(AuraStyle::id)).execute(&mut *tx).await?;
    tx.commit().await?;
    dashboard(app, profile).await
}
fn uuid() -> String {
    let mut b = [0; 16];
    getrandom::fill(&mut b).expect("OS randomness");
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let s = crypto::hex(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &s[..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..]
    )
}
pub async fn apple_prepare(app: &App, profile: &str) -> Result<Value> {
    let c = app.billing.apple.as_ref().ok_or_else(unavailable)?;
    let token:String=sqlx::query_scalar("INSERT INTO portal.supporter_accounts(profile_id,apple_account_token) VALUES($1,$2) ON CONFLICT(profile_id) DO UPDATE SET profile_id=EXCLUDED.profile_id RETURNING apple_account_token").bind(profile).bind(uuid()).fetch_one(&app.pool).await?;
    Ok(json!({"app_account_token":token,"product_id":c.product_id,"environment":c.environment}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppleRequest {
    signed_payload: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppleNotification {
    #[serde(rename = "signedPayload")]
    signed_payload: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppleResponse {
    events: Vec<VerifiedEvent>,
}
async fn verified_apple(app: &App, kind: &str, payload: &str) -> Result<Vec<VerifiedEvent>> {
    if payload.is_empty() || payload.len() > 32768 {
        return Err(payment_invalid());
    }
    fetch_apple(
        app,
        &json!({"kind":kind,"signed_payload":payload}),
        kind == "transaction",
    )
    .await
}
async fn fetch_apple(app: &App, request: &Value, require_one: bool) -> Result<Vec<VerifiedEvent>> {
    let c = app.billing.apple.as_ref().ok_or_else(unavailable)?;
    let response = http_client()
        .post(format!("{}/verify", c.verifier_url))
        .bearer_auth(&c.secret)
        .json(request)
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(payment_invalid());
    }
    let bytes = bounded_response(response, 65536).await?;
    let value: AppleResponse = serde_json::from_slice(&bytes).map_err(|_| payment_invalid())?;
    if value.events.len() > 32 || require_one && value.events.len() != 1 {
        return Err(payment_invalid());
    }
    for event in &value.events {
        event.validate()?;
        if event.provider != "apple"
            || event.product_id.as_deref() != Some(&c.product_id)
            || event.environment.as_deref() != Some(&c.environment)
            || event.app_account_token.is_none()
            || event.original_transaction_id.is_none()
            || event.event_version > now() * 1000 + 300000
        {
            return Err(payment_invalid());
        }
    }
    Ok(value.events)
}
async fn accept_apple(app: &App, events: &[VerifiedEvent], expected: Option<&str>) -> Result<()> {
    let mut tx = app.pool.begin().await?;
    for event in events {
        let profile: String = sqlx::query_scalar(
            "SELECT profile_id FROM portal.supporter_accounts WHERE apple_account_token=$1",
        )
        .bind(&event.app_account_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(forbidden)?;
        if expected.is_some_and(|p| p != profile) {
            return Err(forbidden());
        }
        apply_event_tx(&mut tx, &profile, event).await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn apple_verify(app: &App, profile: &str, body: &[u8]) -> Result<Value> {
    let p: AppleRequest = parse(body)?;
    let events = verified_apple(app, "transaction", &p.signed_payload).await?;
    accept_apple(app, &events, Some(profile)).await?;
    Ok(json!({"status":"verified","supporter":status(app,profile).await?}))
}
pub async fn apple_notification(app: &App, body: &[u8]) -> Result<Value> {
    let p: AppleNotification = parse(body)?;
    let events = verified_apple(app, "notification", &p.signed_payload).await?;
    accept_apple(app, &events, None).await?;
    Ok(json!({"status":"accepted"}))
}
pub async fn native(app: &App, body: &[u8]) -> Result<Value> {
    let signed: SignedNativeSupporterRequest = parse(body)?;
    let r = &signed.request;
    r.validate(&app.config.origin, &r.public_key, now() as u64)
        .map_err(|_| invalid())?;
    let key = VerifyingKey::from_bytes(&crypto::decode::<32>(&r.public_key).ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    let sig = Signature::from_bytes(&crypto::decode::<64>(&signed.signature).ok_or_else(invalid)?);
    key.verify_strict(&r.signing_bytes(), &sig)
        .map_err(|_| forbidden())?;
    app.rate(&r.public_key, "native-supporter", 30).await?;
    let mut tx = app.pool.begin().await?;
    let profile: String = sqlx::query_scalar(
        "SELECT profile_id FROM career_keys WHERE public_key=$1 AND revoked_at IS NULL FOR SHARE",
    )
    .bind(&r.public_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(forbidden)?;
    let consumed=sqlx::query("INSERT INTO portal.supporter_nonces(public_key,nonce,expires_at) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(&r.public_key).bind(&r.nonce).bind(r.expires_at as i64).execute(&mut *tx).await?.rows_affected();
    if consumed == 0 {
        return Err(Error(StatusCode::CONFLICT, "request_replayed"));
    }
    tx.commit().await?;
    match &r.action {
        NativeSupporterAction::Status => dashboard(app, &profile).await,
        NativeSupporterAction::Equip { aura } => {
            equip(
                app,
                &profile,
                &serde_json::to_vec(&json!({"aura":aura})).map_err(|_| invalid())?,
            )
            .await
        }
        NativeSupporterAction::ApplePrepare => apple_prepare(app, &profile).await,
        NativeSupporterAction::AppleVerify { signed_payload } => {
            apple_verify(
                app,
                &profile,
                &serde_json::to_vec(&json!({"signed_payload":signed_payload}))
                    .map_err(|_| invalid())?,
            )
            .await
        }
    }
}

/// Periodically reconcile recent subscriptions to recover missed Apple webhooks.
/// Reserve a bounded batch before outbound calls so multiple API workers do not spin.
pub async fn reconcile_apple(app: &App) -> Result<()> {
    if app.billing.apple.is_none() {
        return Ok(());
    }
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(721946109)")
        .execute(&mut *tx)
        .await?;
    let originals: Vec<String> = sqlx::query_scalar("SELECT original_transaction_id FROM portal.supporter_grants WHERE provider='apple' AND original_transaction_id IS NOT NULL AND (valid_until>$1-604800 OR renewal_enabled IS DISTINCT FROM false) GROUP BY original_transaction_id HAVING min(last_reconciled_at)<$1-300 ORDER BY min(last_reconciled_at) LIMIT 16").bind(now()).fetch_all(&mut *tx).await?;
    for original in &originals {
        sqlx::query("UPDATE portal.supporter_grants SET last_reconciled_at=$2 WHERE provider='apple' AND original_transaction_id=$1").bind(original).bind(now()).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    for original in originals {
        let result = async {
            let events = fetch_apple(
                app,
                &json!({"kind":"reconcile","original_transaction_id":original}),
                false,
            )
            .await?;
            if events
                .iter()
                .any(|e| e.original_transaction_id.as_deref() != Some(&original))
            {
                return Err(payment_invalid());
            }
            accept_apple(app, &events, None).await
        }
        .await;
        if let Err(error) = result {
            eprintln!("supporter reconciliation: {}", error.1);
        }
    }
    Ok(())
}
/// Refuse chunked oversized responses before allocating their entire body.
pub async fn bounded_response(mut response: reqwest::Response, maximum: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|n| n > maximum as u64)
    {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len().saturating_add(chunk.len()) > maximum {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
