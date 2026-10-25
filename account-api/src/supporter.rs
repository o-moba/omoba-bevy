//! Provider receipts are verified outside the client trust boundary, then recorded atomically.
pub mod recognition;
pub mod solana;
pub mod yookassa;
use crate::*;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use shared::market::{PaymentProvider, sells};
use shared::supporter::{
    AuraStyle, NativeSupporterAction, STATUS_GRANT_LIMIT, SignedNativeSupporterRequest,
    SupporterGrantSummary, SupporterStatus, tier_points,
};

/// The payment providers this Account API sells through. The one place the
/// Supporter code reads the market (LR-68 R0 swaps the body for the region).
pub fn market_providers() -> &'static [PaymentProvider] {
    shared::market::payment_providers()
}

/// Whether a Supporter route belongs to a provider in `providers`. Routes of
/// one provider (`.../apple/...`, `.../solana/...`, `.../yookassa/...`) exist
/// only where the market sells through it; the rest are provider-neutral.
pub fn provider_route_allowed(path: &str, providers: &[PaymentProvider]) -> bool {
    PaymentProvider::ALL.iter().all(|provider| {
        sells(providers, *provider) || !path.split('/').any(|part| part == provider.key())
    })
}

#[derive(Clone, Default)]
pub struct BillingConfig {
    pub apple: Option<AppleConfig>,
    pub solana: Option<solana::SolanaConfig>,
    pub yookassa: Option<yookassa::YooKassaConfig>,
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
            yookassa: yookassa::YooKassaConfig::from_env()?,
        })
    }
    /// The providers this host has settings for.
    pub fn configured(&self) -> Vec<PaymentProvider> {
        [
            (PaymentProvider::Apple, self.apple.is_some()),
            (PaymentProvider::Solana, self.solana.is_some()),
            (PaymentProvider::YooKassa, self.yookassa.is_some()),
        ]
        .into_iter()
        .filter_map(|(provider, set)| set.then_some(provider))
        .collect()
    }
    /// Refuse settings of a provider the market does not sell through: a free
    /// build refuses all of them, a global build YooKassa, an RU build Apple
    /// and Solana (LR-67; LR-69 lets RU accept Apple).
    pub fn check_market(
        &self,
        providers: &[PaymentProvider],
    ) -> std::result::Result<(), &'static str> {
        let configured = self.configured();
        if providers.is_empty() && !configured.is_empty() {
            return Err("billing_configured_without_supporter");
        }
        if configured
            .iter()
            .any(|provider| !sells(providers, *provider))
        {
            return Err("payment_provider_not_in_market");
        }
        Ok(())
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
    /// Kopecks paid (YooKassa only). Absent fields keep the stored payload
    /// hashes of Apple and Solana events unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_minor: Option<i64>,
    /// Kopecks refunded so far, all refunds of this payment (YooKassa only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refunded_minor: Option<i64>,
}
impl VerifiedEvent {
    /// The grant's tier points by the provider rule (ADR-025): YooKassa one
    /// point per whole ruble kept after refunds; Apple tier products add their
    /// catalog points in LR-69; Solana none until O-27.
    pub fn tier_points(&self) -> u32 {
        match (self.provider.as_str(), self.amount_minor) {
            ("yookassa", Some(amount)) => {
                yookassa::tier_points(amount, self.refunded_minor.unwrap_or(0))
            }
            _ => 0,
        }
    }
    fn validate(&self) -> Result<()> {
        let amounts = match self.provider.as_str() {
            "yookassa" => {
                self.amount_minor.is_some_and(|amount| {
                    (yookassa::MIN_AMOUNT_MINOR..=yookassa::MAX_AMOUNT_MINOR).contains(&amount)
                        && self
                            .refunded_minor
                            .is_some_and(|refunded| (0..=amount).contains(&refunded))
                }) && self.original_transaction_id.is_none()
                    && self.app_account_token.is_none()
                    && self.valid_until - self.valid_from == yookassa::GRANT_SECONDS
            }
            "apple" | "solana" => self.amount_minor.is_none() && self.refunded_minor.is_none(),
            _ => false,
        };
        if !amounts
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
    let points = event.tier_points() as i32;
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
        // The amount never changes; tier points follow the newest provider snapshot (refunds).
        let updated = sqlx::query("UPDATE portal.supporter_grants SET valid_from=$3,valid_until=$4,revoked_at=COALESCE(revoked_at,$5),renewal_enabled=COALESCE($6,renewal_enabled),event_version=$7,tier_points=$8 WHERE provider=$1 AND period_id=$2 AND amount_minor IS NOT DISTINCT FROM $9")
            .bind(&event.provider).bind(&event.period_id).bind(event.valid_from).bind(event.valid_until).bind(event.revoked_at).bind(event.renewal_enabled).bind(event.event_version).bind(points).bind(event.amount_minor).execute(&mut **tx).await?
            .rows_affected();
        if updated != 1 {
            return Err(conflict());
        }
        default_aura_tx(tx, profile, false).await?;
    } else {
        let first: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT 1 FROM portal.supporter_grants WHERE profile_id=$1)",
        )
        .bind(profile)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query("INSERT INTO portal.supporter_grants(provider,period_id,profile_id,original_transaction_id,valid_from,valid_until,revoked_at,renewal_enabled,event_version,amount_minor,tier_points) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(&event.provider).bind(&event.period_id).bind(profile).bind(&event.original_transaction_id).bind(event.valid_from).bind(event.valid_until).bind(event.revoked_at).bind(event.renewal_enabled).bind(event.event_version).bind(event.amount_minor).bind(points).execute(&mut **tx).await?;
        default_aura_tx(tx, profile, first).await?;
    }
    Ok(true)
}

/// After a grant write: a profile without a preference row gets the highest
/// style its points unlock now (`AuraStyle::highest_unlocked`, the career
/// store's rule for a missing row). On the first grant a row that only holds
/// the placeholder base style (written by the recognition toggle before any
/// payment) is upgraded the same way; a hidden aura stays hidden.
async fn default_aura_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile: &str,
    first_grant: bool,
) -> Result<()> {
    let points: i64 = sqlx::query_scalar("SELECT portal.supporter_points($1,$2)")
        .bind(profile)
        .bind(now())
        .fetch_one(&mut **tx)
        .await?;
    let default = AuraStyle::highest_unlocked(tier_points(points)).id();
    if first_grant {
        sqlx::query("INSERT INTO portal.supporter_preferences(profile_id,aura) VALUES($1,$2) ON CONFLICT(profile_id) DO UPDATE SET aura=EXCLUDED.aura WHERE portal.supporter_preferences.aura=$3")
            .bind(profile)
            .bind(default)
            .bind(PLACEHOLDER_AURA.id())
            .execute(&mut **tx)
            .await?;
    } else {
        sqlx::query("INSERT INTO portal.supporter_preferences(profile_id,aura) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(profile)
            .bind(default)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// The base style a preference row holds before any grant exists (the
/// recognition toggle creates the row): the default for zero points.
pub const PLACEHOLDER_AURA: AuraStyle = AuraStyle::Solar;
pub async fn status(app: &App, profile: &str) -> Result<SupporterStatus> {
    let current = now();
    // One snapshot: the bounded grant list (active first), the unbounded tier
    // sum and the stored preference.
    let mut tx = app.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let rows=sqlx::query("SELECT provider,valid_from,valid_until,revoked_at,renewal_enabled,tier_points FROM portal.supporter_grants WHERE profile_id=$1 ORDER BY (revoked_at IS NULL AND valid_from<=$2 AND valid_until>$2) DESC,valid_until DESC LIMIT $3").bind(profile).bind(current).bind(STATUS_GRANT_LIMIT).fetch_all(&mut *tx).await?;
    let grants = rows
        .iter()
        .map(|r| {
            Ok(SupporterGrantSummary {
                provider: row_text(r, "provider")?,
                valid_from: r.try_get("valid_from")?,
                valid_until: r.try_get("valid_until")?,
                revoked: r.try_get::<Option<i64>, _>("revoked_at")?.is_some(),
                renewal_enabled: r.try_get("renewal_enabled")?,
                tier_points: tier_points(i64::from(r.try_get::<i32, _>("tier_points")?)),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let points: i64 = sqlx::query_scalar("SELECT portal.supporter_points($1,$2)")
        .bind(profile)
        .bind(current)
        .fetch_one(&mut *tx)
        .await?;
    let aura: Option<Option<String>> =
        sqlx::query_scalar("SELECT aura FROM portal.supporter_preferences WHERE profile_id=$1")
            .bind(profile)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    // An unknown stored id (none can pass the CHECK) reads as hidden.
    let preference = aura.map(|aura| aura.as_deref().and_then(AuraStyle::from_id));
    Ok(SupporterStatus::from_grants(
        grants,
        current,
        tier_points(points),
        preference,
    ))
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
    let supporter = status(app, profile).await?;
    Ok(json!({
        "providers": market_providers().iter().map(|p| p.key()).collect::<Vec<_>>(),
        "catalog": catalog(&supporter),
        "supporter": supporter,
        "recognition": recognition::get(app, profile).await?,
        "apple": apple,
        "solana": solana,
    }))
}
/// Every style with what it needs (`tier_points` 0 = any active grant) and
/// whether this supporter may wear it now, by the shared entitlement rule.
pub fn catalog(status: &SupporterStatus) -> Vec<Value> {
    AuraStyle::ALL
        .iter()
        .map(|style| {
            json!({
                "id": style,
                "name": style.label(),
                "tier_points": style.required_points(),
                "unlocked": style.unlocked(status.active, status.tier_points),
            })
        })
        .collect()
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
    let (active,points):(bool,i64)=sqlx::query_as("SELECT EXISTS(SELECT 1 FROM portal.supporter_grants WHERE profile_id=$1 AND valid_from<=$2 AND valid_until>$2 AND revoked_at IS NULL),portal.supporter_points($1,$2)").bind(profile).bind(now()).fetch_one(&mut *tx).await?;
    if let Some(style) = p.aura {
        if !active {
            return Err(Error(StatusCode::FORBIDDEN, "supporter_required"));
        }
        if !style.unlocked(active, tier_points(points)) {
            return Err(Error(StatusCode::FORBIDDEN, "tier_required"));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::market::Market;

    fn yookassa() -> yookassa::YooKassaConfig {
        yookassa::YooKassaConfig {
            shop_id: "123456".into(),
            secret_key: "test_secret_key".into(),
            vat_code: 1,
            new_charges: false,
        }
    }
    fn apple() -> AppleConfig {
        AppleConfig {
            verifier_url: "http://127.0.0.1:1".into(),
            secret: "0".repeat(64),
            product_id: "test.supporter".into(),
            environment: "Sandbox".into(),
        }
    }

    #[test]
    fn a_market_refuses_another_markets_payment_settings() {
        let global = Market::Global.payment_providers();
        let ru = Market::Ru.payment_providers();
        let only_yookassa = BillingConfig {
            yookassa: Some(yookassa()),
            ..Default::default()
        };
        let only_apple = BillingConfig {
            apple: Some(apple()),
            ..Default::default()
        };
        assert_eq!(BillingConfig::default().check_market(&[]), Ok(()));
        assert_eq!(BillingConfig::default().check_market(ru), Ok(()));
        assert_eq!(only_yookassa.check_market(ru), Ok(()));
        assert_eq!(only_apple.check_market(global), Ok(()));
        assert_eq!(
            only_yookassa.check_market(global),
            Err("payment_provider_not_in_market")
        );
        // Until LR-69 the RU Account API refuses Apple (and Solana) settings.
        assert_eq!(
            only_apple.check_market(ru),
            Err("payment_provider_not_in_market")
        );
        let both = BillingConfig {
            apple: Some(apple()),
            yookassa: Some(yookassa()),
            ..Default::default()
        };
        assert!(both.check_market(ru).is_err() && both.check_market(global).is_err());
        // A build without `supporter` refuses every provider's settings.
        for config in [only_yookassa, only_apple] {
            assert_eq!(
                config.check_market(&[]),
                Err("billing_configured_without_supporter")
            );
        }
    }

    #[test]
    fn provider_routes_exist_only_where_the_market_sells_through_them() {
        let global = Market::Global.payment_providers();
        let ru = Market::Ru.payment_providers();
        for path in [
            "me/supporter",
            "me/supporter/aura",
            "me/supporter/recognition",
            "supporter/native",
        ] {
            assert!(provider_route_allowed(path, ru), "{path}");
            assert!(provider_route_allowed(path, global), "{path}");
        }
        for path in [
            "me/supporter/apple/prepare",
            "me/supporter/solana/checkout",
            "supporter/apple/notifications",
        ] {
            assert!(!provider_route_allowed(path, ru), "{path}");
            assert!(provider_route_allowed(path, global), "{path}");
        }
        for path in [
            "me/supporter/yookassa/checkout",
            "supporter/yookassa/notifications",
        ] {
            assert!(provider_route_allowed(path, ru), "{path}");
            assert!(!provider_route_allowed(path, global), "{path}");
        }
        // A segment, not a substring: an id containing a provider name is not one.
        assert!(provider_route_allowed("me/supporter/applesauce", ru));
    }

    fn event(provider: &str) -> VerifiedEvent {
        VerifiedEvent {
            event_id: "e".into(),
            provider: provider.into(),
            period_id: "p".into(),
            original_transaction_id: None,
            app_account_token: None,
            product_id: None,
            environment: None,
            valid_from: 1000,
            valid_until: 1000 + yookassa::GRANT_SECONDS,
            revoked_at: None,
            event_version: 0,
            renewal_enabled: Some(false),
            amount_minor: None,
            refunded_minor: None,
        }
    }

    #[test]
    fn grant_points_follow_the_provider_rule_and_only_yookassa_carries_amounts() {
        let mut donation = event("yookassa");
        donation.amount_minor = Some(1000 * yookassa::KOPECKS);
        donation.refunded_minor = Some(0);
        assert!(donation.validate().is_ok());
        assert_eq!(donation.tier_points(), 1000);
        donation.refunded_minor = Some(600 * yookassa::KOPECKS + 1);
        assert_eq!(donation.tier_points(), 399);
        for bad in [
            VerifiedEvent {
                amount_minor: None,
                ..donation.clone()
            },
            VerifiedEvent {
                refunded_minor: None,
                ..donation.clone()
            },
            VerifiedEvent {
                refunded_minor: Some(1001 * yookassa::KOPECKS),
                ..donation.clone()
            },
            VerifiedEvent {
                amount_minor: Some(99 * yookassa::KOPECKS),
                refunded_minor: Some(0),
                ..donation.clone()
            },
            VerifiedEvent {
                valid_until: 1000 + 31 * 86_400,
                ..donation.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        // Apple and Solana grants carry no amount and no points in M0.
        for provider in ["apple", "solana"] {
            let plain = event(provider);
            assert!(plain.validate().is_ok());
            assert_eq!(plain.tier_points(), 0);
            let forged = VerifiedEvent {
                amount_minor: Some(1000 * yookassa::KOPECKS),
                ..plain
            };
            assert!(forged.validate().is_err(), "{provider}");
        }
        assert!(event("paypal").validate().is_err());
        // Absent amounts keep the stored payload of Apple and Solana events.
        let json = serde_json::to_string(&event("solana")).unwrap();
        assert!(!json.contains("amount_minor") && !json.contains("refunded_minor"));
    }

    #[test]
    fn the_dashboard_catalog_uses_the_shared_entitlement_rule() {
        let status = SupporterStatus {
            active: true,
            tier_points: 1000,
            ..Default::default()
        };
        let catalog = catalog(&status);
        assert_eq!(catalog.len(), AuraStyle::ALL.len());
        for (entry, style) in catalog.iter().zip(AuraStyle::ALL) {
            assert_eq!(entry["id"], style.id());
            assert_eq!(entry["tier_points"], style.required_points());
            assert_eq!(entry["unlocked"], style.unlocked(true, 1000));
        }
        let none = catalog_for_inactive();
        assert!(none.iter().all(|entry| entry["unlocked"] == false));
    }

    fn catalog_for_inactive() -> Vec<Value> {
        catalog(&SupporterStatus::default())
    }
}
