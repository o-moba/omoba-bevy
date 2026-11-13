//! `POST /v1/supporter/yookassa/notifications`: YooKassa's webhook.
//!
//! The pushed body is only a hint. It names an object; nothing in it is
//! trusted. A payment or refund this API cannot have created is acknowledged
//! with 200 and no provider call (so the public route is no lever on the
//! shop's API): a payment is known when it is bound to an attempt, or when its
//! metadata names a posted attempt still waiting for its create reply; a
//! refund when its payment is bound. A known one is re-read with the shop
//! credentials and settled exactly once; one YooKassa does not have (404) is
//! acknowledged the same way. Any other processing error answers 5xx, so
//! YooKassa delivers the notification again; the reconciliation loop recovers
//! it either way.
use super::api;
use super::checkout::settle;
use super::reconcile::{RefundOutcome, handle_refund};
use super::{PaymentStatus, Settlement};
use crate::supporter::{payment_invalid, unavailable};
use crate::*;

/// The events the shop sends (ops/vps/ru/README.md, "YooKassa").
pub const EVENTS: [&str; 4] = [
    "payment.succeeded",
    "payment.waiting_for_capture",
    "payment.canceled",
    "refund.succeeded",
];

fn ignored() -> Result<Value> {
    Ok(json!({"status": "ignored"}))
}

fn accepted() -> Result<Value> {
    Ok(json!({"status": "accepted"}))
}

/// A failure YooKassa should retry: server errors stay as they are, anything
/// else (an object that did not settle) becomes a 500.
fn retry(error: Error) -> Error {
    if error.0.is_server_error() {
        error
    } else {
        Error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "notification_not_processed",
        )
    }
}

/// Whether this API created the payment: bound to an attempt, or (a create
/// reply that was lost) naming an unbound attempt that was posted. A pristine
/// or closed attempt has no payment; one in manual review stays here, so an
/// operator's hand-made notification binds its payment.
async fn known_payment(app: &App, payment: &str, attempt: Option<&str>) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.supporter_yookassa_attempts WHERE provider_payment_id=$1) OR ($2::text IS NOT NULL AND EXISTS(SELECT 1 FROM portal.supporter_yookassa_attempts WHERE attempt_id=$2 AND provider_payment_id IS NULL AND status IN ('charging','failed') AND first_provider_attempt_at IS NOT NULL))")
        .bind(payment)
        .bind(attempt)
        .fetch_one(&app.pool)
        .await?)
}

pub async fn notification(app: &App, body: &[u8]) -> Result<Value> {
    let config = app.billing.yookassa.as_ref().ok_or_else(unavailable)?;
    let value: Value = parse(body)?;
    let text = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|id| api::provider_id(id))
    };
    let (Some(event), Some(id)) = (
        value.get("event").and_then(Value::as_str),
        text("/object/id"),
    ) else {
        return ignored();
    };
    if !EVENTS.contains(&event) {
        return ignored();
    }
    if event == "refund.succeeded" {
        let Some(payment) = text("/object/payment_id") else {
            return ignored();
        };
        if !super::reconcile::refund_may_be_ours(&app.pool, payment).await? {
            return ignored();
        }
        let Some(refund) = api::find_refund(config, id).await.map_err(retry)? else {
            return ignored();
        };
        if refund.refund.payment_id != payment {
            return Err(retry(payment_invalid()));
        }
        return match handle_refund(app, config, &refund).await? {
            RefundOutcome::Changed | RefundOutcome::Unchanged => accepted(),
            RefundOutcome::Unknown => ignored(),
            // Queued for the refund scan; YooKassa delivers it again too.
            RefundOutcome::Pending | RefundOutcome::Failed => Err(retry(payment_invalid())),
        };
    }
    let attempt = value
        .pointer("/object/metadata/attempt_id")
        .and_then(Value::as_str);
    if !known_payment(app, id, attempt).await? {
        return ignored();
    }
    let Some(payment) = api::find_payment(config, id).await.map_err(retry)? else {
        return ignored();
    };
    match settle(app, &payment.payment).await.map_err(retry)? {
        Settlement::Unknown => ignored(),
        Settlement::Changed | Settlement::Unchanged => {
            if payment.payment.status == PaymentStatus::WaitingForCapture {
                // Payments are created with capture: true; a held payment means
                // the shop settings changed. It is not captured here.
                eprintln!("yookassa notification: a payment waits for capture");
            }
            accepted()
        }
    }
}
