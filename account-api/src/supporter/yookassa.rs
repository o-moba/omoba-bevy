//! YooKassa supporter donations (LR-67): settings and exactly-once settlement.
//!
//! This is the data half. The provider flow (the checkout POST, the webhook
//! route, the reconciliation loop; LR-67b) re-reads every payment and refund
//! from YooKassa with the shop credentials and hands the result to
//! [`settle_payment_tx`] / [`settle_refund_tx`]. Those write the grant through
//! `apply_event_tx` in the caller's transaction, so a grant, its amount, its
//! tier points and the attempt's status change together or not at all:
//!
//! - a succeeded payment grants 30 days from its capture, once, with its amount
//!   in kopecks and one tier point per whole ruble;
//! - a canceled payment never downgrades a success;
//! - a refund (partial or full) recomputes the points from all refunds of that
//!   payment and a full refund revokes the grant; a refund that overtakes its
//!   payment settles the payment first.
//!
//! Never log a provider body (card and customer data); amounts are integer
//! kopecks, never floats.
use super::{VerifiedEvent, apply_event_tx, payment_invalid};
use crate::*;

/// Kopecks per ruble: amounts are integer kopecks, never floats.
pub const KOPECKS: i64 = 100;
/// The smallest donation: 100 ₽.
pub const MIN_AMOUNT_MINOR: i64 = 100 * KOPECKS;
/// The largest donation: 100 000 ₽.
pub const MAX_AMOUNT_MINOR: i64 = 100_000 * KOPECKS;
/// A donation's grant window.
pub const GRANT_SECONDS: i64 = 30 * 86_400;
/// A lost create may be replayed with the same Idempotence-Key only while the
/// first provider attempt is younger than this; older goes to manual review.
pub const REPLAY_WINDOW_SECONDS: i64 = 23 * 3_600;
/// The checkout tiers in rubles (the portal also accepts a custom amount
/// between [`MIN_AMOUNT_MINOR`] and [`MAX_AMOUNT_MINOR`]).
pub const TIER_AMOUNTS_RUB: [i64; 7] = [100, 250, 500, 1000, 2500, 5000, 10000];

/// Tier points of a YooKassa grant: whole rubles kept after refunds.
pub fn tier_points(amount_minor: i64, refunded_minor: i64) -> u32 {
    shared::supporter::tier_points(amount_minor.saturating_sub(refunded_minor).max(0) / 100)
}

/// Whether a ruble amount can be donated (tiers or a custom amount).
pub fn amount_allowed(amount_rub: i64) -> bool {
    (MIN_AMOUNT_MINOR..=MAX_AMOUNT_MINOR).contains(&amount_rub.saturating_mul(100))
}

/// Shop settings. The secret never leaves the Account API and is never logged.
#[derive(Clone)]
pub struct YooKassaConfig {
    pub shop_id: String,
    pub secret_key: String,
    /// 54-FZ `vat_code` of the receipt item (1–12; 1 = без НДС).
    pub vat_code: u8,
    /// Exactly `OMOBA_YOOKASSA_NEW_CHARGES=1` allows new charges;
    /// reconciliation of existing attempts always runs.
    pub new_charges: bool,
}

impl YooKassaConfig {
    pub const VARS: [&'static str; 4] = [
        "OMOBA_YOOKASSA_SHOP_ID",
        "OMOBA_YOOKASSA_SECRET_KEY",
        "OMOBA_YOOKASSA_VAT_CODE",
        "OMOBA_YOOKASSA_NEW_CHARGES",
    ];

    pub fn from_env() -> std::result::Result<Option<Self>, String> {
        Self::parse(|name| std::env::var(name).ok())
    }

    /// `None` when no YooKassa variable is set; any one of them makes the
    /// whole set required (a partial shop configuration fails closed).
    pub fn parse(
        var: impl Fn(&str) -> Option<String>,
    ) -> std::result::Result<Option<Self>, String> {
        let values = Self::VARS.map(|name| var(name).filter(|value| !value.is_empty()));
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [Some(shop_id), Some(secret_key), Some(vat_code), new_charges] = values else {
            return Err("Incomplete YooKassa configuration".into());
        };
        let vat: u8 = vat_code
            .parse()
            .ok()
            .filter(|code: &u8| (1..=12).contains(code) && code.to_string() == vat_code)
            .ok_or("Invalid YooKassa VAT code")?;
        if shop_id.is_empty()
            || shop_id.len() > 20
            || !shop_id.bytes().all(|b| b.is_ascii_digit())
            || secret_key.len() < 8
            || secret_key.len() > 256
            || !secret_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err("Invalid YooKassa shop configuration".into());
        }
        Ok(Some(Self {
            shop_id,
            secret_key,
            vat_code: vat,
            new_charges: new_charges.as_deref() == Some("1"),
        }))
    }
}

/// A payment's status as YooKassa reports it on a re-read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentStatus {
    Pending,
    WaitingForCapture,
    Succeeded,
    Canceled,
}

/// A payment re-read from YooKassa with the shop credentials (never a webhook body).
#[derive(Clone, Debug)]
pub struct VerifiedPayment {
    pub payment_id: String,
    pub status: PaymentStatus,
    /// Kopecks in RUB (the caller refuses any other currency).
    pub amount_minor: i64,
    /// `captured_at` (or `created_at` for a one-stage payment), epoch seconds.
    pub paid_at: Option<i64>,
    /// `metadata.attempt_id` this API sent with the create: binds the payment
    /// to its attempt when the create response was lost.
    pub attempt_id: Option<String>,
}

/// A succeeded refund re-read from YooKassa with the shop credentials.
#[derive(Clone, Debug)]
pub struct VerifiedRefund {
    pub refund_id: String,
    pub payment_id: String,
    pub amount_minor: i64,
    pub refunded_at: i64,
}

/// What a settlement did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settlement {
    /// No attempt of ours: acknowledge and do nothing.
    Unknown,
    /// Already settled (a duplicate) or nothing to change.
    Unchanged,
    /// The grant or the attempt changed.
    Changed,
}

fn provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn attempt_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl VerifiedPayment {
    fn validate(&self) -> Result<()> {
        if !provider_id(&self.payment_id)
            || !(MIN_AMOUNT_MINOR..=MAX_AMOUNT_MINOR).contains(&self.amount_minor)
            || self.paid_at.is_some_and(|at| at < 0)
            || (self.status == PaymentStatus::Succeeded && self.paid_at.is_none())
            || self.attempt_id.as_deref().is_some_and(|id| !attempt_id(id))
        {
            return Err(payment_invalid());
        }
        Ok(())
    }
}

struct Attempt {
    attempt_id: String,
    profile_id: String,
    amount_minor: i64,
    status: String,
}

/// The attempt this payment belongs to, locked: by provider payment id, else
/// (a lost create response) by the attempt id it carried, binding the payment.
///
/// The fallback locks the attempt by id alone. A concurrent settlement of the
/// same lost-create payment may bind it while this one waits for the row lock;
/// a `provider_payment_id IS NULL` predicate would then be re-checked after
/// that commit and skip the row, and a payment of ours would settle as
/// Unknown (a refund lost). The bound id is checked after the lock instead:
/// unbound binds, this payment goes on, another payment's attempt is not this
/// payment's (Unknown: an operator reviews it).
async fn locked_attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    payment: &VerifiedPayment,
) -> Result<Option<Attempt>> {
    let read = |row: &sqlx::postgres::PgRow| -> Result<Attempt> {
        Ok(Attempt {
            attempt_id: row_text(row, "attempt_id")?,
            profile_id: row_text(row, "profile_id")?,
            amount_minor: row.try_get("amount_minor")?,
            status: row_text(row, "status")?,
        })
    };
    if let Some(row) = sqlx::query("SELECT attempt_id,profile_id,amount_minor,status FROM portal.supporter_yookassa_attempts WHERE provider_payment_id=$1 FOR UPDATE")
        .bind(&payment.payment_id)
        .fetch_optional(&mut **tx)
        .await?
    {
        return read(&row).map(Some);
    }
    let Some(id) = &payment.attempt_id else {
        return Ok(None);
    };
    let Some(row) = sqlx::query("SELECT attempt_id,profile_id,amount_minor,status,provider_payment_id FROM portal.supporter_yookassa_attempts WHERE attempt_id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
    else {
        return Ok(None);
    };
    let bound: Option<String> = row.try_get("provider_payment_id")?;
    match bound.as_deref() {
        None => {
            sqlx::query("UPDATE portal.supporter_yookassa_attempts SET provider_payment_id=$2,updated_at=$3 WHERE attempt_id=$1")
                .bind(id)
                .bind(&payment.payment_id)
                .bind(now())
                .execute(&mut **tx)
                .await?;
        }
        // Bound by a settlement that committed while this one waited.
        Some(bound) if bound == payment.payment_id => {}
        Some(_) => return Ok(None),
    }
    read(&row).map(Some)
}

fn grant_event(
    payment: &VerifiedPayment,
    paid_at: i64,
    event_id: String,
    refunded_minor: i64,
    revoked_at: Option<i64>,
    event_version: i64,
) -> VerifiedEvent {
    VerifiedEvent {
        event_id,
        provider: "yookassa".into(),
        period_id: payment.payment_id.clone(),
        original_transaction_id: None,
        app_account_token: None,
        product_id: None,
        environment: None,
        valid_from: paid_at,
        valid_until: paid_at + GRANT_SECONDS,
        revoked_at,
        event_version,
        // A one-time donation: never renews by itself.
        renewal_enabled: Some(false),
        amount_minor: Some(payment.amount_minor),
        refunded_minor: Some(refunded_minor),
    }
}

/// Settle a re-read payment inside `tx`: a success grants once, a cancel never
/// downgrades a success, a pending payment marks its attempt submitted.
pub async fn settle_payment_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    payment: &VerifiedPayment,
) -> Result<Settlement> {
    payment.validate()?;
    let Some(attempt) = locked_attempt(tx, payment).await? else {
        return Ok(Settlement::Unknown);
    };
    if attempt.amount_minor != payment.amount_minor {
        // Never grant a different amount than the player accepted; an operator reviews it.
        return Err(payment_invalid());
    }
    let current = now();
    match payment.status {
        PaymentStatus::Succeeded => {
            let paid_at = payment.paid_at.ok_or_else(payment_invalid)?;
            let event = grant_event(
                payment,
                paid_at,
                format!("payment:{}", payment.payment_id),
                0,
                None,
                0,
            );
            let granted = apply_event_tx(tx, &attempt.profile_id, &event).await?;
            let settled = sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='succeeded',lease_until=NULL,last_error=NULL,first_provider_attempt_at=COALESCE(first_provider_attempt_at,$2),settled_at=COALESCE(settled_at,$2),updated_at=$2 WHERE attempt_id=$1 AND status<>'succeeded'")
                .bind(&attempt.attempt_id)
                .bind(current)
                .execute(&mut **tx)
                .await?
                .rows_affected();
            Ok(if granted || settled > 0 {
                Settlement::Changed
            } else {
                Settlement::Unchanged
            })
        }
        PaymentStatus::Canceled => {
            let canceled = sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='canceled',lease_until=NULL,updated_at=$2 WHERE attempt_id=$1 AND status NOT IN ('succeeded','canceled')")
                .bind(&attempt.attempt_id)
                .bind(current)
                .execute(&mut **tx)
                .await?
                .rows_affected();
            Ok(if canceled > 0 {
                Settlement::Changed
            } else {
                Settlement::Unchanged
            })
        }
        PaymentStatus::Pending | PaymentStatus::WaitingForCapture => {
            if !matches!(attempt.status.as_str(), "created" | "charging" | "failed") {
                return Ok(Settlement::Unchanged);
            }
            sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='submitted',lease_until=NULL,first_provider_attempt_at=COALESCE(first_provider_attempt_at,$2),updated_at=$2 WHERE attempt_id=$1")
                .bind(&attempt.attempt_id)
                .bind(current)
                .execute(&mut **tx)
                .await?;
            Ok(Settlement::Changed)
        }
    }
}

/// Settle a re-read succeeded refund of a re-read payment inside `tx`. The
/// payment is settled first (a refund can overtake it). Each refund counts
/// once; the grant's points are recomputed from all refunds of the payment and
/// a full refund revokes the grant.
pub async fn settle_refund_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    payment: &VerifiedPayment,
    refund: &VerifiedRefund,
) -> Result<Settlement> {
    if !provider_id(&refund.refund_id)
        || refund.payment_id != payment.payment_id
        || payment.status != PaymentStatus::Succeeded
        || refund.amount_minor <= 0
        || refund.amount_minor > payment.amount_minor
        || refund.refunded_at < 0
    {
        return Err(payment_invalid());
    }
    match settle_payment_tx(tx, payment).await? {
        Settlement::Unknown => return Ok(Settlement::Unknown),
        Settlement::Changed | Settlement::Unchanged => {}
    }
    let inserted = sqlx::query("INSERT INTO portal.supporter_yookassa_refunds(refund_id,provider_payment_id,amount_minor,refunded_at,recorded_at) VALUES($1,$2,$3,$4,$5) ON CONFLICT(refund_id) DO NOTHING")
        .bind(&refund.refund_id)
        .bind(&refund.payment_id)
        .bind(refund.amount_minor)
        .bind(refund.refunded_at)
        .bind(now())
        .execute(&mut **tx)
        .await?
        .rows_affected();
    if inserted == 0 {
        let (payment_id, amount): (String, i64) = sqlx::query_as(
            "SELECT provider_payment_id,amount_minor FROM portal.supporter_yookassa_refunds WHERE refund_id=$1",
        )
        .bind(&refund.refund_id)
        .fetch_one(&mut **tx)
        .await?;
        if payment_id != refund.payment_id || amount != refund.amount_minor {
            return Err(conflict());
        }
        return Ok(Settlement::Unchanged);
    }
    let (count, refunded): (i64, i64) = sqlx::query_as(
        "SELECT count(*),coalesce(sum(amount_minor),0)::bigint FROM portal.supporter_yookassa_refunds WHERE provider_payment_id=$1",
    )
    .bind(&refund.payment_id)
    .fetch_one(&mut **tx)
    .await?;
    if refunded > payment.amount_minor {
        return Err(payment_invalid());
    }
    let paid_at = payment.paid_at.ok_or_else(payment_invalid)?;
    let profile: String = sqlx::query_scalar(
        "SELECT profile_id FROM portal.supporter_yookassa_attempts WHERE provider_payment_id=$1",
    )
    .bind(&refund.payment_id)
    .fetch_one(&mut **tx)
    .await?;
    let event = grant_event(
        payment,
        paid_at,
        format!("refund:{}", refund.refund_id),
        refunded,
        (refunded >= payment.amount_minor).then_some(refund.refunded_at),
        count,
    );
    apply_event_tx(tx, &profile, &event).await?;
    Ok(Settlement::Changed)
}

/// [`settle_payment_tx`] in its own transaction.
pub async fn settle_payment(app: &App, payment: &VerifiedPayment) -> Result<Settlement> {
    let mut tx = app.pool.begin().await?;
    let settled = settle_payment_tx(&mut tx, payment).await?;
    tx.commit().await?;
    Ok(settled)
}

/// [`settle_refund_tx`] in its own transaction.
pub async fn settle_refund(
    app: &App,
    payment: &VerifiedPayment,
    refund: &VerifiedRefund,
) -> Result<Settlement> {
    let mut tx = app.pool.begin().await?;
    let settled = settle_refund_tx(&mut tx, payment, refund).await?;
    tx.commit().await?;
    Ok(settled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn one_point_per_whole_ruble_kept_after_refunds() {
        assert_eq!(tier_points(100 * KOPECKS, 0), 100);
        assert_eq!(tier_points(1000 * KOPECKS, 0), 1000);
        assert_eq!(tier_points(1000 * KOPECKS, 600 * KOPECKS), 400);
        assert_eq!(tier_points(1000 * KOPECKS, 600 * KOPECKS + 50), 399);
        assert_eq!(tier_points(1000 * KOPECKS, 1000 * KOPECKS), 0);
        assert_eq!(tier_points(1000 * KOPECKS, 2000 * KOPECKS), 0);
        assert_eq!(tier_points(123 * KOPECKS + 45, 0), 123);
    }

    #[test]
    fn custom_amounts_stay_between_100_and_100000_rubles() {
        for rub in TIER_AMOUNTS_RUB {
            assert!(amount_allowed(rub), "{rub}");
        }
        assert!(amount_allowed(100) && amount_allowed(100_000) && amount_allowed(777));
        for rub in [0, 1, 99, 100_001, -100, i64::MAX] {
            assert!(!amount_allowed(rub), "{rub}");
        }
        assert_eq!(
            TIER_AMOUNTS_RUB
                .map(|rub| shared::supporter::SupporterTier::for_points(tier_points(rub * 100, 0))),
            shared::supporter::SupporterTier::ALL.map(Some)
        );
    }

    #[test]
    fn shop_settings_are_all_or_nothing_and_new_charges_need_exactly_one() {
        assert!(YooKassaConfig::parse(env(&[])).unwrap().is_none());
        let full = [
            ("OMOBA_YOOKASSA_SHOP_ID", "123456"),
            ("OMOBA_YOOKASSA_SECRET_KEY", "test_abcdefgh123"),
            ("OMOBA_YOOKASSA_VAT_CODE", "1"),
        ];
        let config = YooKassaConfig::parse(env(&full)).unwrap().unwrap();
        assert_eq!((config.shop_id.as_str(), config.vat_code), ("123456", 1));
        assert!(!config.new_charges);
        for flag in ["0", "true", "yes", " 1", "01"] {
            let mut pairs = full.to_vec();
            pairs.push(("OMOBA_YOOKASSA_NEW_CHARGES", flag));
            assert!(
                !YooKassaConfig::parse(env(&pairs))
                    .unwrap()
                    .unwrap()
                    .new_charges
            );
        }
        let mut on = full.to_vec();
        on.push(("OMOBA_YOOKASSA_NEW_CHARGES", "1"));
        assert!(
            YooKassaConfig::parse(env(&on))
                .unwrap()
                .unwrap()
                .new_charges
        );
        // Any one variable makes the set required.
        assert!(YooKassaConfig::parse(env(&[("OMOBA_YOOKASSA_NEW_CHARGES", "1")])).is_err());
        assert!(YooKassaConfig::parse(env(&full[..2])).is_err());
        for (name, bad) in [
            ("OMOBA_YOOKASSA_VAT_CODE", "0"),
            ("OMOBA_YOOKASSA_VAT_CODE", "13"),
            ("OMOBA_YOOKASSA_VAT_CODE", "01"),
            ("OMOBA_YOOKASSA_VAT_CODE", "x"),
            ("OMOBA_YOOKASSA_SHOP_ID", "12a"),
            ("OMOBA_YOOKASSA_SECRET_KEY", "short"),
            ("OMOBA_YOOKASSA_SECRET_KEY", "has space inside"),
        ] {
            let pairs: Vec<_> = full
                .iter()
                .map(|(k, v)| if *k == name { (*k, bad) } else { (*k, *v) })
                .collect();
            assert!(YooKassaConfig::parse(env(&pairs)).is_err(), "{name}={bad}");
        }
    }

    #[test]
    fn payments_need_a_provider_id_an_allowed_amount_and_a_paid_time() {
        let ok = VerifiedPayment {
            payment_id: "2f1b6a3c-000f-5000-9000-1b4c0a4d6b0e".into(),
            status: PaymentStatus::Succeeded,
            amount_minor: 1000 * KOPECKS,
            paid_at: Some(1_790_000_000),
            attempt_id: Some("a".repeat(32)),
        };
        assert!(ok.validate().is_ok());
        for bad in [
            VerifiedPayment {
                payment_id: "bad id".into(),
                ..ok.clone()
            },
            VerifiedPayment {
                amount_minor: 99 * KOPECKS + 99,
                ..ok.clone()
            },
            VerifiedPayment {
                amount_minor: 100_000 * KOPECKS + 1,
                ..ok.clone()
            },
            VerifiedPayment {
                paid_at: None,
                ..ok.clone()
            },
            VerifiedPayment {
                attempt_id: Some("A".repeat(32)),
                ..ok.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert!(
            VerifiedPayment {
                status: PaymentStatus::Pending,
                paid_at: None,
                ..ok
            }
            .validate()
            .is_ok()
        );
    }
}
