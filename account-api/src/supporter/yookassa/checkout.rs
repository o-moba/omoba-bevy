//! The checkout (`POST /v1/me/supporter/yookassa/checkout`), the owner's
//! payment list (`GET /v1/me/supporter/yookassa/payments`) and the one submit
//! path both the checkout and the reconciliation loop use.
//!
//! Rules (ported from the owner's battle-tested integration):
//! - the attempt, its Idempotence-Key and its frozen «Чеки от ЮKassa» receipt
//!   are committed before the first `POST /payments`; the attempt is then
//!   claimed (`charging`, with a short lease) and only then posted;
//! - a lost create is replayed with the **same** key and the same body while
//!   the first provider attempt is younger than 23 hours; older goes to manual
//!   review and is never posted again;
//! - every provider reply is settled through `settle_payment_tx`, so a payment
//!   grants exactly once;
//! - new charges need `OMOBA_YOOKASSA_NEW_CHARGES=1` and a refund scan that
//!   succeeded in the last 30 minutes; recovery of existing attempts always runs.
use super::api::{self, ProviderPayment};
use super::reconcile::{record_issue, resolve_issue};
use super::{
    GRANT_SECONDS, KOPECKS, MAX_AMOUNT_MINOR, MIN_AMOUNT_MINOR, PaymentStatus,
    REPLAY_WINDOW_SECONDS, Settlement, TIER_AMOUNTS_RUB, VerifiedPayment, YooKassaConfig,
    amount_allowed, settle_payment_tx,
};
use crate::supporter::{payment_invalid, unavailable};
use crate::*;
use serde::{Deserialize, Serialize};
use shared::supporter::SupporterTier;

/// The receipt item and the payment description (54-FZ, one service).
pub const DESCRIPTION: &str = "OMOBA — поддержка проекта (аура на 30 дней)";
/// Where YooKassa sends the player back: `<portal origin>` + this.
pub const RETURN_PATH: &str = "/supporter?paid=1";
/// How long one worker holds an attempt while it talks to YooKassa.
pub const LEASE_SECONDS: i64 = 120;
/// New charges stop when the last successful refund scan is older than this.
pub const REFUND_SCAN_MAX_AGE_SECONDS: i64 = 30 * 60;
/// Open attempts a profile may hold at once (created in the last 24 hours).
pub const OPEN_ATTEMPT_LIMIT: i64 = 10;
/// A `created` attempt never posted for this long is closed by reconciliation.
pub const PRISTINE_SECONDS: i64 = 3600;
/// `last_error` of a lost create past the 23-hour replay window.
pub const REPLAY_EXPIRED: &str = "provider_replay_window_expired";
/// `last_error` of a create reply that does not match its attempt.
pub const PAYMENT_MISMATCH: &str = "provider_payment_mismatch";
/// Attempts with these errors wait for an operator and are never posted again.
pub const REVIEW_ERRORS: [&str; 2] = [REPLAY_EXPIRED, PAYMENT_MISMATCH];
const PAYMENT_LIST_LIMIT: i64 = 20;
const PAYMENT_LIST_SECONDS: i64 = 365 * 86_400;

/// What the browser showed the player. The checkout refuses a quote that is
/// not the server's for the same amount (`409 reload`), so a player never
/// pays for a price, period or tier they did not see.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quote {
    pub amount_minor: i64,
    pub currency: String,
    pub duration_days: i64,
    /// The tier this donation reaches by itself (`rose` … `prism_iii`).
    pub tier: Option<String>,
}

/// The server's quote for `amount_rub`.
pub fn quote(amount_rub: i64) -> Quote {
    let amount_minor = amount_rub.saturating_mul(KOPECKS);
    Quote {
        amount_minor,
        currency: "RUB".into(),
        duration_days: GRANT_SECONDS / 86_400,
        tier: SupporterTier::for_points(super::tier_points(amount_minor, 0))
            .map(|tier| tier.id().to_owned()),
    }
}

/// The receipt e-mail: trimmed, at most 254 characters, `local@domain.tld`,
/// no spaces or control characters.
pub fn receipt_email(raw: &str) -> Option<String> {
    let email = raw.trim();
    let (local, domain) = email.split_once('@')?;
    let dotted = domain
        .char_indices()
        .any(|(index, c)| c == '.' && index > 0 && index + 1 < domain.len());
    (email.len() <= 254
        && !local.is_empty()
        && dotted
        && !domain.contains('@')
        && !email.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then(|| email.to_owned())
}

/// The «Чеки от ЮKassa» receipt: the customer's e-mail and one service item
/// paid in full, with the shop's VAT code.
pub fn receipt(email: &str, amount_minor: i64, vat_code: u8, description: &str) -> Value {
    json!({
        "customer": {"email": email},
        "items": [{
            "description": description,
            "quantity": "1.00",
            "amount": {"value": api::amount_value(amount_minor), "currency": "RUB"},
            "vat_code": vat_code,
            "payment_mode": "full_payment",
            "payment_subject": "service",
        }],
    })
}

/// The attempt's frozen receipt, checked before every POST: exactly the shape
/// [`receipt`] builds for this amount. A corrupt snapshot is never sent.
pub fn frozen_receipt(stored: &Value, amount_minor: i64) -> Option<Value> {
    let email = stored.pointer("/customer/email")?.as_str()?;
    let item = stored.pointer("/items/0")?;
    let description = item.get("description")?.as_str()?;
    let vat = u8::try_from(item.get("vat_code")?.as_u64()?)
        .ok()
        .filter(|vat| (1..=12).contains(vat))?;
    if receipt_email(email).as_deref() != Some(email)
        || description.is_empty()
        || description.chars().count() > 128
    {
        return None;
    }
    let rebuilt = receipt(email, amount_minor, vat, description);
    (rebuilt == *stored).then_some(rebuilt)
}

fn offer_version(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The browser's `Idempotency-Key` (16–80 of `[A-Za-z0-9-]`, as the other
/// idempotent routes).
fn idempotency_key(headers: &HeaderMap) -> Result<&str> {
    let key = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(invalid)?;
    if !(16..=80).contains(&key.len())
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(invalid());
    }
    Ok(key)
}

fn paused() -> Error {
    Error(StatusCode::SERVICE_UNAVAILABLE, "new_charges_paused")
}

/// Whether the last refund scan succeeded recently enough for new charges.
pub async fn refund_scan_fresh<'e>(db: impl sqlx::PgExecutor<'e>, at: i64) -> Result<bool> {
    let last: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT last_successful_at FROM portal.supporter_yookassa_refund_scan WHERE key='refunds'",
    )
    .fetch_optional(db)
    .await?;
    Ok(last
        .flatten()
        .is_some_and(|last| last > at - REFUND_SCAN_MAX_AGE_SECONDS))
}

/// Whether this shop takes new charges now (the switch and a fresh refund scan).
pub async fn new_charges_open<'e>(
    db: impl sqlx::PgExecutor<'e>,
    config: &YooKassaConfig,
    at: i64,
) -> Result<bool> {
    Ok(config.new_charges && refund_scan_fresh(db, at).await?)
}

/// The dashboard's YooKassa block: what the portal needs to render the offer.
pub async fn offer(app: &App) -> Result<Value> {
    let Some(config) = &app.billing.yookassa else {
        return Ok(json!({"available": false}));
    };
    Ok(json!({
        "available": true,
        "new_charges": new_charges_open(&app.pool, config, now()).await?,
        "currency": "RUB",
        "duration_days": GRANT_SECONDS / 86_400,
        "min_rub": MIN_AMOUNT_MINOR / KOPECKS,
        "max_rub": MAX_AMOUNT_MINOR / KOPECKS,
        "tiers_rub": TIER_AMOUNTS_RUB,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckoutRequest {
    amount_rub: i64,
    receipt_email: String,
    quote: Quote,
    offer_version: String,
}

/// `POST /v1/me/supporter/yookassa/checkout`.
pub async fn checkout(app: &App, profile: &str, headers: &HeaderMap, body: &[u8]) -> Result<Value> {
    let config = app.billing.yookassa.as_ref().ok_or_else(unavailable)?;
    let key = idempotency_key(headers)?;
    let request: CheckoutRequest = parse(body)?;
    if !amount_allowed(request.amount_rub) {
        return Err(Error(StatusCode::BAD_REQUEST, "amount_not_allowed"));
    }
    let email = receipt_email(&request.receipt_email)
        .ok_or(Error(StatusCode::BAD_REQUEST, "receipt_email_invalid"))?;
    if !offer_version(&request.offer_version) {
        return Err(invalid());
    }
    if request.quote != quote(request.amount_rub) {
        return Err(Error(StatusCode::CONFLICT, "reload"));
    }
    app.rate(profile, "yookassa-checkout", 10).await?;
    let request_key = crypto::mac(
        &app.config.secret,
        "yookassa-checkout",
        &format!("{profile}\0{key}"),
    );
    let attempt = attempt_for(
        app,
        config,
        profile,
        &request_key,
        request.amount_rub * KOPECKS,
        &email,
        &request.offer_version,
    )
    .await?;
    match submit(app, config, &attempt, Caller::Player).await? {
        Outcome::Payment { payment, .. } => match payment.payment.status {
            PaymentStatus::Pending => match payment.confirmation_url {
                Some(url) => Ok(json!({"confirmation_url": url, "attempt_id": attempt})),
                None => {
                    eprintln!("yookassa checkout: pending payment without a usable redirect");
                    Err(unavailable())
                }
            },
            PaymentStatus::WaitingForCapture => Err(in_progress()),
            PaymentStatus::Succeeded => Err(Error(StatusCode::CONFLICT, "payment_succeeded")),
            PaymentStatus::Canceled => Err(Error(StatusCode::CONFLICT, "payment_canceled")),
        },
        Outcome::Succeeded => Err(Error(StatusCode::CONFLICT, "payment_succeeded")),
        Outcome::Canceled => Err(Error(StatusCode::CONFLICT, "payment_canceled")),
        Outcome::Busy | Outcome::Pristine => Err(in_progress()),
        Outcome::Review => Err(Error(StatusCode::CONFLICT, "payment_manual_review")),
        Outcome::Paused => Err(paused()),
    }
}

fn in_progress() -> Error {
    Error(StatusCode::CONFLICT, "payment_in_progress")
}

/// The attempt of this (profile, Idempotency-Key): the existing one with the
/// same amount, e-mail and offer, or a new one with its Idempotence-Key and
/// frozen receipt, committed before any provider call.
async fn attempt_for(
    app: &App,
    config: &YooKassaConfig,
    profile: &str,
    request_key: &str,
    amount_minor: i64,
    email: &str,
    offer: &str,
) -> Result<String> {
    let current = now();
    let mut tx = app.pool.begin().await?;
    // Serializes one profile's checkouts (the open-attempt cap). Settlement
    // never takes this lock, so it cannot deadlock with it.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946111))")
        .bind(profile)
        .execute(&mut *tx)
        .await?;
    if let Some(row) = sqlx::query("SELECT attempt_id,amount_minor,receipt->'customer'->>'email' AS email,offer_version FROM portal.supporter_yookassa_attempts WHERE request_key=$1")
        .bind(request_key)
        .fetch_optional(&mut *tx)
        .await?
    {
        if row.try_get::<i64, _>("amount_minor")? != amount_minor
            || row.try_get::<Option<String>, _>("email")?.as_deref() != Some(email)
            || row_text(&row, "offer_version")? != offer
        {
            return Err(Error(StatusCode::CONFLICT, "idempotency_conflict"));
        }
        tx.commit().await?;
        return row_text(&row, "attempt_id");
    }
    if !new_charges_open(&mut *tx, config, current).await? {
        return Err(paused());
    }
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM portal.supporter_yookassa_attempts WHERE profile_id=$1 AND status IN ('created','charging','submitted','failed') AND created_at>$2")
        .bind(profile)
        .bind(current - 86_400)
        .fetch_one(&mut *tx)
        .await?;
    if open >= OPEN_ATTEMPT_LIMIT {
        return Err(Error(StatusCode::CONFLICT, "too_many_open_payments"));
    }
    let attempt = crypto::random::<16>();
    sqlx::query("INSERT INTO portal.supporter_yookassa_attempts(attempt_id,profile_id,request_key,idempotence_key,amount_minor,receipt,offer_version,status,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,'created',$8,$8)")
        .bind(&attempt)
        .bind(profile)
        .bind(request_key)
        .bind(crypto::random::<24>())
        .bind(amount_minor)
        .bind(receipt(email, amount_minor, config.vat_code, DESCRIPTION))
        .bind(offer)
        .bind(current)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(attempt)
}

/// Who submits: a player's checkout may post a fresh attempt; the
/// reconciliation loop only recovers attempts that were already posted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    Player,
    Reconciliation,
}

/// What [`submit`] found or did.
#[derive(Debug)]
pub enum Outcome {
    /// YooKassa's payment, settled; `posted` when this call sent a create.
    Payment {
        payment: ProviderPayment,
        posted: bool,
    },
    Succeeded,
    Canceled,
    /// Another worker holds the attempt.
    Busy,
    /// Past the replay window or mismatched: an operator reviews it.
    Review,
    /// A fresh attempt while new charges are paused.
    Paused,
    /// A fresh attempt the reconciliation loop leaves alone.
    Pristine,
}

struct AttemptRow {
    attempt_id: String,
    idempotence_key: String,
    provider_payment_id: Option<String>,
    amount_minor: i64,
    receipt: Value,
    status: String,
    lease_until: Option<i64>,
    first_provider_attempt_at: Option<i64>,
    last_error: Option<String>,
}

enum Claim {
    Post(AttemptRow),
    Bound(String),
    Done(Outcome),
}

/// Lock the attempt and decide: post (claiming it), re-read a bound payment,
/// or stop. The claim commits before the provider call.
async fn claim(app: &App, config: &YooKassaConfig, attempt: &str, caller: Caller) -> Result<Claim> {
    let current = now();
    let mut tx = app.pool.begin().await?;
    let row = sqlx::query("SELECT attempt_id,idempotence_key,provider_payment_id,amount_minor,receipt,status,lease_until,first_provider_attempt_at,last_error FROM portal.supporter_yookassa_attempts WHERE attempt_id=$1 FOR UPDATE")
        .bind(attempt)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(forbidden)?;
    let row = AttemptRow {
        attempt_id: row_text(&row, "attempt_id")?,
        idempotence_key: row_text(&row, "idempotence_key")?,
        provider_payment_id: row.try_get("provider_payment_id")?,
        amount_minor: row.try_get("amount_minor")?,
        receipt: row.try_get::<sqlx::types::Json<Value>, _>("receipt")?.0,
        status: row_text(&row, "status")?,
        lease_until: row.try_get("lease_until")?,
        first_provider_attempt_at: row.try_get("first_provider_attempt_at")?,
        last_error: row.try_get("last_error")?,
    };
    let review = row
        .last_error
        .as_deref()
        .is_some_and(|code| REVIEW_ERRORS.contains(&code));
    // A never-posted attempt is a new charge: only a player may post it, and
    // only while new charges are open.
    let pristine = row.status == "created" && row.first_provider_attempt_at.is_none();
    let open =
        caller == Caller::Player && pristine && new_charges_open(&mut *tx, config, current).await?;
    let decided = match row.status.as_str() {
        "succeeded" => Some(Outcome::Succeeded),
        "canceled" => Some(Outcome::Canceled),
        _ if row.provider_payment_id.is_some() => {
            return Ok(Claim::Bound(row.provider_payment_id.unwrap_or_default()));
        }
        "charging" if row.lease_until.is_some_and(|until| until > current) => Some(Outcome::Busy),
        "failed" if review => Some(Outcome::Review),
        _ if pristine => match caller {
            Caller::Reconciliation => Some(Outcome::Pristine),
            Caller::Player if !open => Some(Outcome::Paused),
            Caller::Player => None,
        },
        "created" | "charging" | "failed" => match row.first_provider_attempt_at {
            // A lost create may be replayed with its key only inside the
            // window; without the durable boundary the state is ambiguous.
            Some(first) if current - first < REPLAY_WINDOW_SECONDS => None,
            _ => {
                sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='failed',lease_until=NULL,last_error=$2,updated_at=$3 WHERE attempt_id=$1")
                    .bind(&row.attempt_id)
                    .bind(REPLAY_EXPIRED)
                    .bind(current)
                    .execute(&mut *tx)
                    .await?;
                record_issue(
                    &mut *tx,
                    &row.attempt_id,
                    "payment",
                    "replay_window_expired",
                )
                .await?;
                tx.commit().await?;
                eprintln!("yookassa: an attempt passed the replay window; manual review");
                return Ok(Claim::Done(Outcome::Review));
            }
        },
        _ => Some(Outcome::Busy),
    };
    if let Some(outcome) = decided {
        tx.commit().await?;
        return Ok(Claim::Done(outcome));
    }
    sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='charging',lease_until=$2,first_provider_attempt_at=COALESCE(first_provider_attempt_at,$3),updated_at=$3 WHERE attempt_id=$1")
        .bind(&row.attempt_id)
        .bind(current + LEASE_SECONDS)
        .bind(current)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Claim::Post(row))
}

/// The create request of an attempt: the same body on every replay (the frozen
/// receipt, its description, the amount, the attempt id), so YooKassa answers
/// a replay with the payment the first request created.
fn create_request(origin: &str, row: &AttemptRow) -> Option<Value> {
    let receipt = frozen_receipt(&row.receipt, row.amount_minor)?;
    let description = receipt.pointer("/items/0/description")?.clone();
    Some(json!({
        "amount": {"value": api::amount_value(row.amount_minor), "currency": "RUB"},
        "capture": true,
        "confirmation": {"type": "redirect", "return_url": format!("{origin}{RETURN_PATH}")},
        "description": description,
        "receipt": receipt,
        "metadata": {"attempt_id": row.attempt_id},
    }))
}

async fn mark_failed(app: &App, attempt: &str, code: &str) -> Result<()> {
    sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='failed',lease_until=NULL,last_error=$2,updated_at=$3 WHERE attempt_id=$1 AND status IN ('created','charging','failed') AND provider_payment_id IS NULL")
        .bind(attempt)
        .bind(code)
        .bind(now())
        .execute(&app.pool)
        .await?;
    Ok(())
}

/// Settle a payment YooKassa reported, in its own transaction. A payment that
/// cannot be settled is queued for an operator; a settled one clears its issue.
pub async fn settle(app: &App, payment: &VerifiedPayment) -> Result<Settlement> {
    let result: Result<Settlement> = async {
        let mut tx = app.pool.begin().await?;
        let settled = settle_payment_tx(&mut tx, payment).await?;
        tx.commit().await?;
        Ok(settled)
    }
    .await;
    match result {
        Ok(Settlement::Unknown) => Ok(Settlement::Unknown),
        Ok(settled) => {
            resolve_issue(&app.pool, &payment.payment_id).await?;
            Ok(settled)
        }
        Err(error) => {
            if !error.0.is_server_error() {
                record_issue(
                    &app.pool,
                    &payment.payment_id,
                    "payment",
                    "settlement_failed",
                )
                .await?;
            }
            Err(error)
        }
    }
}

/// Post, replay or re-read one attempt and settle what YooKassa answers.
pub async fn submit(
    app: &App,
    config: &YooKassaConfig,
    attempt: &str,
    caller: Caller,
) -> Result<Outcome> {
    match claim(app, config, attempt, caller).await? {
        Claim::Done(outcome) => Ok(outcome),
        Claim::Bound(payment_id) => {
            let payment = api::get_payment(config, &payment_id).await?;
            settle(app, &payment.payment).await?;
            Ok(Outcome::Payment {
                payment,
                posted: false,
            })
        }
        Claim::Post(row) => {
            let Some(request) = create_request(&app.config.origin, &row) else {
                mark_failed(app, &row.attempt_id, PAYMENT_MISMATCH).await?;
                record_issue(&app.pool, &row.attempt_id, "payment", "settlement_failed").await?;
                return Ok(Outcome::Review);
            };
            let payment = match api::create_payment(config, &row.idempotence_key, &request).await {
                Ok(payment) => payment,
                Err(error) => {
                    mark_failed(app, &row.attempt_id, error.1).await?;
                    return Err(error);
                }
            };
            // The reply must be this attempt's payment for this amount.
            if payment.payment.attempt_id.as_deref() != Some(row.attempt_id.as_str())
                || payment.payment.amount_minor != row.amount_minor
            {
                mark_failed(app, &row.attempt_id, PAYMENT_MISMATCH).await?;
                record_issue(&app.pool, &row.attempt_id, "payment", "amount_mismatch").await?;
                return Err(payment_invalid());
            }
            if settle(app, &payment.payment).await? == Settlement::Unknown {
                // Bound to another payment meanwhile: an operator reviews it.
                record_issue(&app.pool, &row.attempt_id, "payment", "settlement_failed").await?;
                return Err(payment_invalid());
            }
            Ok(Outcome::Payment {
                payment,
                posted: true,
            })
        }
    }
}

/// Re-read the owner's newest submitted attempt of the last hour (at most once
/// per 5 s), so a player back from YooKassa sees the result even before the
/// webhook arrives. Errors leave the list as it is.
async fn refresh_newest(app: &App, config: &YooKassaConfig, profile: &str) {
    let current = now();
    let claimed: std::result::Result<Option<Option<String>>, sqlx::Error> = sqlx::query_scalar("UPDATE portal.supporter_yookassa_attempts SET last_polled_at=$3 WHERE attempt_id=(SELECT attempt_id FROM portal.supporter_yookassa_attempts WHERE profile_id=$1 AND status='submitted' AND created_at>$2-3600 AND last_polled_at<$2-5 ORDER BY created_at DESC LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING provider_payment_id")
        .bind(profile)
        .bind(current)
        .bind(current)
        .fetch_optional(&app.pool)
        .await;
    let Ok(Some(Some(payment_id))) = claimed else {
        return;
    };
    let refreshed = async {
        let payment = api::get_payment(config, &payment_id).await?;
        settle(app, &payment.payment).await
    }
    .await;
    if let Err(error) = refreshed {
        eprintln!("yookassa payment refresh: {}", error.1);
    }
}

fn public_status(status: &str, last_error: Option<&str>) -> &'static str {
    match status {
        "succeeded" => "succeeded",
        "canceled" => "canceled",
        "failed" if last_error.is_some_and(|code| REVIEW_ERRORS.contains(&code)) => "review",
        _ => "pending",
    }
}

/// `GET /v1/me/supporter/yookassa/payments`: the owner's recent donations.
pub async fn payments(app: &App, profile: &str) -> Result<Value> {
    if let Some(config) = &app.billing.yookassa {
        refresh_newest(app, config, profile).await;
    }
    let rows = sqlx::query("SELECT a.attempt_id,a.provider_payment_id,a.amount_minor,a.currency,a.status,a.last_error,a.created_at,a.settled_at,coalesce((SELECT sum(r.amount_minor) FROM portal.supporter_yookassa_refunds r WHERE r.provider_payment_id=a.provider_payment_id),0)::bigint AS refunded_minor FROM portal.supporter_yookassa_attempts a WHERE a.profile_id=$1 AND a.created_at>$2 ORDER BY a.created_at DESC LIMIT $3")
        .bind(profile)
        .bind(now() - PAYMENT_LIST_SECONDS)
        .bind(PAYMENT_LIST_LIMIT)
        .fetch_all(&app.pool)
        .await?;
    let payments = rows
        .iter()
        .map(|row| {
            let status = row_text(row, "status")?;
            let last_error: Option<String> = row.try_get("last_error")?;
            Ok(json!({
                "attempt_id": row_text(row, "attempt_id")?,
                "provider_payment_id": row.try_get::<Option<String>, _>("provider_payment_id")?,
                "amount_minor": row.try_get::<i64, _>("amount_minor")?,
                "currency": row_text(row, "currency")?,
                "status": public_status(&status, last_error.as_deref()),
                "refunded_minor": row.try_get::<i64, _>("refunded_minor")?,
                "created_at": row.try_get::<i64, _>("created_at")?,
                "settled_at": row.try_get::<Option<i64>, _>("settled_at")?,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"payments": payments}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quote_names_amount_period_and_tier() {
        let crimson = quote(1000);
        assert_eq!(
            serde_json::to_value(&crimson).unwrap(),
            json!({"amount_minor": 100_000, "currency": "RUB", "duration_days": 30, "tier": "crimson"})
        );
        assert_eq!(quote(100).tier.as_deref(), Some("rose"));
        assert_eq!(quote(2499).tier.as_deref(), Some("crimson"));
        assert_eq!(quote(100_000).tier.as_deref(), Some("prism_iii"));
        assert_ne!(quote(1000), quote(999));
        // The browser's quote is parsed strictly.
        assert!(
            serde_json::from_value::<Quote>(json!({"amount_minor": 1, "currency": "RUB", "duration_days": 30, "tier": null, "x": 1}))
                .is_err()
        );
    }

    #[test]
    fn receipt_emails_are_plain_addresses() {
        assert_eq!(
            receipt_email("  donor@example.ru ").as_deref(),
            Some("donor@example.ru")
        );
        assert!(receipt_email("a.b+c@mail.example.org").is_some());
        for bad in [
            "",
            "donor",
            "donor@",
            "@example.ru",
            "donor@example",
            "donor@.ru",
            "donor@example.",
            "do nor@example.ru",
            "donor@exa@mple.ru",
            "donor@exam\nple.ru",
            "donor\u{7}@example.ru",
        ] {
            assert_eq!(receipt_email(bad), None, "{bad:?}");
        }
        assert!(receipt_email(&format!("{}@example.ru", "a".repeat(250))).is_none());
    }

    #[test]
    fn the_receipt_is_one_full_payment_service_item_and_stays_frozen() {
        let frozen = receipt("donor@example.ru", 250_000, 1, DESCRIPTION);
        assert_eq!(
            frozen,
            json!({
                "customer": {"email": "donor@example.ru"},
                "items": [{
                    "description": "OMOBA — поддержка проекта (аура на 30 дней)",
                    "quantity": "1.00",
                    "amount": {"value": "2500.00", "currency": "RUB"},
                    "vat_code": 1,
                    "payment_mode": "full_payment",
                    "payment_subject": "service",
                }],
            })
        );
        assert_eq!(frozen_receipt(&frozen, 250_000), Some(frozen.clone()));
        // Another amount, a changed item, a second item or no e-mail is refused.
        assert_eq!(frozen_receipt(&frozen, 100_000), None);
        for pointer in [
            "/items/0/payment_mode",
            "/items/0/payment_subject",
            "/items/0/quantity",
            "/customer/email",
        ] {
            let mut corrupt = frozen.clone();
            *corrupt.pointer_mut(pointer).unwrap() = json!("x");
            assert_eq!(frozen_receipt(&corrupt, 250_000), None, "{pointer}");
        }
        let mut two = frozen.clone();
        let item = two["items"][0].clone();
        two["items"].as_array_mut().unwrap().push(item);
        assert_eq!(frozen_receipt(&two, 250_000), None);
        let mut vat = frozen.clone();
        vat["items"][0]["vat_code"] = json!(13);
        assert_eq!(frozen_receipt(&vat, 250_000), None);
    }

    #[test]
    fn the_player_sees_four_states() {
        assert_eq!(public_status("created", None), "pending");
        assert_eq!(public_status("charging", None), "pending");
        assert_eq!(public_status("submitted", None), "pending");
        assert_eq!(
            public_status("failed", Some("payment_provider_unavailable")),
            "pending"
        );
        assert_eq!(public_status("failed", Some(REPLAY_EXPIRED)), "review");
        assert_eq!(public_status("failed", Some(PAYMENT_MISMATCH)), "review");
        assert_eq!(public_status("succeeded", None), "succeeded");
        assert_eq!(public_status("canceled", None), "canceled");
    }

    #[test]
    fn offer_versions_and_idempotency_keys_are_bounded() {
        assert!(offer_version("0-draft") && offer_version("2026.10.25_1"));
        assert!(!offer_version("") && !offer_version("a b") && !offer_version(&"1".repeat(41)));
        let mut headers = HeaderMap::new();
        assert!(idempotency_key(&headers).is_err());
        headers.insert("idempotency-key", "short".parse().unwrap());
        assert!(idempotency_key(&headers).is_err());
        headers.insert(
            "idempotency-key",
            "0b8f1c2e-4d5a-4b6c-8d7e-9f0a1b2c3d4e".parse().unwrap(),
        );
        assert!(idempotency_key(&headers).is_ok());
    }
}
