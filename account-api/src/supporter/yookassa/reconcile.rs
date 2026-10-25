//! The YooKassa reconciliation loop (next to the Apple one in `main.rs`).
//!
//! Each pass:
//! 1. closes `created` attempts that were never posted for an hour;
//! 2. polls a bounded batch of open attempts, oldest-polled first (queue
//!    rotation): a submitted payment is re-read and settled; a lost create
//!    (`charging` past its lease, or `failed`) is replayed with the **same**
//!    Idempotence-Key while younger than 23 hours, older goes to manual review
//!    and is never posted again;
//! 3. scans the shop's refunds with a durable cursor: a fixed window
//!    `(watermark, window_end]` is paged with YooKassa's opaque cursor and only
//!    then advanced, so a crash never skips an unseen refund. A refund that
//!    cannot be settled now is queued as an issue before the cursor moves.
//!
//! New charges stop when the scan has not succeeded for 30 minutes
//! (`checkout::refund_scan_fresh`); the loop itself always runs.
use super::api::{self, ProviderRefund, RefundStatus};
use super::checkout::{self, Caller, Outcome, PRISTINE_SECONDS, REVIEW_ERRORS};
use super::{Settlement, YooKassaConfig, settle_refund};
use crate::*;
use serde::Serialize;

/// Attempts polled per pass.
pub const ATTEMPT_BATCH: i64 = 8;
/// An attempt is polled at most this often.
pub const POLL_INTERVAL_SECONDS: i64 = 30;
/// Refunds per list page and pages per pass.
pub const REFUND_PAGE: usize = 32;
pub const REFUND_PAGES: usize = 2;
/// Queued refund issues retried per pass.
pub const ISSUE_RETRIES: i64 = 4;
/// How long a pass holds the refund window.
pub const SCAN_LEASE_SECONDS: i64 = 600;
/// The next window starts this far before the last one ended, so a refund
/// YooKassa lists late is still seen (settlement is idempotent).
pub const WATERMARK_OVERLAP_SECONDS: i64 = 301;

/// What one pass did (logged as counts; never provider data).
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Report {
    pub pristine_closed: u64,
    pub attempts: usize,
    pub replayed: usize,
    pub settled: usize,
    pub review: usize,
    pub refunds_checked: usize,
    pub refunds_changed: usize,
    pub refund_issues: usize,
    pub refund_scan: &'static str,
    pub errors: usize,
}

impl Report {
    /// Whether the pass did anything worth a log line.
    pub fn eventful(&self) -> bool {
        self.pristine_closed > 0
            || self.attempts > 0
            || self.refunds_checked > 0
            || self.errors > 0
            || !matches!(self.refund_scan, "" | "complete" | "busy")
    }
}

/// Queue a provider object for an operator (or for a later retry).
pub async fn record_issue<'e>(
    db: impl sqlx::PgExecutor<'e>,
    object: &str,
    object_type: &str,
    code: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO portal.supporter_yookassa_issues(provider_object_id,object_type,code,first_seen_at,last_seen_at) VALUES($1,$2,$3,$4,$4) ON CONFLICT(provider_object_id) DO UPDATE SET code=EXCLUDED.code,occurrences=portal.supporter_yookassa_issues.occurrences+1,last_seen_at=EXCLUDED.last_seen_at,resolved_at=NULL")
        .bind(object)
        .bind(object_type)
        .bind(code)
        .bind(now())
        .execute(db)
        .await?;
    Ok(())
}

/// Clear an object's open issue once it settled.
pub async fn resolve_issue<'e>(db: impl sqlx::PgExecutor<'e>, object: &str) -> Result<bool> {
    Ok(sqlx::query("UPDATE portal.supporter_yookassa_issues SET resolved_at=$2 WHERE provider_object_id=$1 AND resolved_at IS NULL")
        .bind(object)
        .bind(now())
        .execute(db)
        .await?
        .rows_affected()
        > 0)
}

/// Whether a refund of `payment` can be ours: the payment is bound to an
/// attempt, or some posted attempt still waits for its lost create reply
/// (the re-read payment then names it). Otherwise no provider call is made.
pub async fn refund_may_be_ours<'e>(db: impl sqlx::PgExecutor<'e>, payment: &str) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.supporter_yookassa_attempts WHERE provider_payment_id=$1) OR EXISTS(SELECT 1 FROM portal.supporter_yookassa_attempts WHERE provider_payment_id IS NULL AND status IN ('charging','failed') AND first_provider_attempt_at IS NOT NULL)")
        .bind(payment)
        .fetch_one(db)
        .await?)
}

/// What became of one refund.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefundOutcome {
    Changed,
    Unchanged,
    /// Not final at YooKassa yet: retried later.
    Pending,
    /// Not a payment of this API: an operator looks at it.
    Unknown,
    /// Could not be settled now: retried later.
    Failed,
}

async fn process_refund(
    app: &App,
    config: &YooKassaConfig,
    refund: &ProviderRefund,
) -> RefundOutcome {
    match refund.status {
        RefundStatus::Canceled => return RefundOutcome::Unchanged,
        RefundStatus::Pending => return RefundOutcome::Pending,
        RefundStatus::Succeeded => {}
    }
    let refund = &refund.refund;
    // Already settled: no provider call.
    let recorded: std::result::Result<Option<(String, i64)>, _> = sqlx::query_as(
        "SELECT provider_payment_id,amount_minor FROM portal.supporter_yookassa_refunds WHERE refund_id=$1",
    )
    .bind(&refund.refund_id)
    .fetch_optional(&app.pool)
    .await;
    match recorded {
        Ok(Some((payment, amount))) => {
            return if payment == refund.payment_id && amount == refund.amount_minor {
                RefundOutcome::Unchanged
            } else {
                RefundOutcome::Failed
            };
        }
        Ok(None) => {}
        Err(_) => return RefundOutcome::Failed,
    }
    match refund_may_be_ours(&app.pool, &refund.payment_id).await {
        Ok(true) => {}
        Ok(false) => return RefundOutcome::Unknown,
        Err(_) => return RefundOutcome::Failed,
    }
    let Ok(payment) = api::get_payment(config, &refund.payment_id).await else {
        return RefundOutcome::Failed;
    };
    match settle_refund(app, &payment.payment, refund).await {
        Ok(Settlement::Changed) => RefundOutcome::Changed,
        Ok(Settlement::Unchanged) => RefundOutcome::Unchanged,
        Ok(Settlement::Unknown) => RefundOutcome::Unknown,
        Err(_) => RefundOutcome::Failed,
    }
}

/// Settle one refund and record what needs another look. An `Err` means the
/// outcome could not be recorded: the caller must not move past this refund.
pub async fn handle_refund(
    app: &App,
    config: &YooKassaConfig,
    refund: &ProviderRefund,
) -> Result<RefundOutcome> {
    let outcome = process_refund(app, config, refund).await;
    let id = &refund.refund.refund_id;
    match outcome {
        RefundOutcome::Changed | RefundOutcome::Unchanged => {
            resolve_issue(&app.pool, id).await?;
        }
        RefundOutcome::Pending => record_issue(&app.pool, id, "refund", "pending_refund").await?,
        RefundOutcome::Unknown => record_issue(&app.pool, id, "refund", "unknown_payment").await?,
        RefundOutcome::Failed => record_issue(&app.pool, id, "refund", "settlement_failed").await?,
    }
    Ok(outcome)
}

/// One reconciliation pass. Without YooKassa settings it does nothing.
pub async fn reconcile(app: &App) -> Result<Report> {
    let Some(config) = app.billing.yookassa.as_ref() else {
        return Ok(Report::default());
    };
    let mut report = Report::default();
    attempts(app, config, &mut report).await?;
    refunds(app, config, &mut report).await?;
    Ok(report)
}

async fn attempts(app: &App, config: &YooKassaConfig, report: &mut Report) -> Result<()> {
    let current = now();
    report.pristine_closed = sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='canceled',updated_at=$1 WHERE status='created' AND provider_payment_id IS NULL AND first_provider_attempt_at IS NULL AND created_at<$2")
        .bind(current)
        .bind(current - PRISTINE_SECONDS)
        .execute(&app.pool)
        .await?
        .rows_affected();
    // Reserve a bounded batch before any provider call, so several API
    // processes share the queue instead of polling the same attempts.
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(721946110)")
        .execute(&mut *tx)
        .await?;
    let batch: Vec<String> = sqlx::query_scalar("SELECT attempt_id FROM portal.supporter_yookassa_attempts WHERE last_polled_at<=$1-$2 AND (status='submitted' OR (provider_payment_id IS NULL AND (last_error IS NULL OR last_error<>ALL($3)) AND (status='failed' OR (status='charging' AND (lease_until IS NULL OR lease_until<$1))))) ORDER BY last_polled_at,created_at LIMIT $4")
        .bind(current)
        .bind(POLL_INTERVAL_SECONDS)
        .bind(&REVIEW_ERRORS[..])
        .bind(ATTEMPT_BATCH)
        .fetch_all(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE portal.supporter_yookassa_attempts SET last_polled_at=$2 WHERE attempt_id=ANY($1)",
    )
    .bind(&batch)
    .bind(current)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    for attempt in batch {
        report.attempts += 1;
        match checkout::submit(app, config, &attempt, Caller::Reconciliation).await {
            Ok(Outcome::Payment { posted, .. }) => {
                report.settled += 1;
                report.replayed += usize::from(posted);
            }
            Ok(Outcome::Review) => report.review += 1,
            Ok(_) => {}
            Err(error) => {
                report.errors += 1;
                eprintln!("yookassa reconciliation: {}", error.1);
            }
        }
    }
    Ok(())
}

struct Window {
    watermark: i64,
    end: i64,
    cursor: Option<String>,
}

async fn claim_window(app: &App, current: i64) -> Result<Option<Window>> {
    let mut tx = app.pool.begin().await?;
    sqlx::query("INSERT INTO portal.supporter_yookassa_refund_scan(key,watermark,updated_at) VALUES('refunds',$1,$1) ON CONFLICT DO NOTHING")
        .bind(current)
        .execute(&mut *tx)
        .await?;
    let (watermark, window_end, cursor, lease_until): (i64, Option<i64>, Option<String>, Option<i64>) = sqlx::query_as("SELECT watermark,window_end,cursor,lease_until FROM portal.supporter_yookassa_refund_scan WHERE key='refunds' FOR UPDATE")
        .fetch_one(&mut *tx)
        .await?;
    if lease_until.is_some_and(|until| until > current) {
        return Ok(None);
    }
    // An unfinished window is resumed at its cursor; otherwise a new one ends now.
    let (end, cursor) = match window_end {
        Some(end) => (end, cursor),
        None => (current.max(watermark), None),
    };
    sqlx::query("UPDATE portal.supporter_yookassa_refund_scan SET window_end=$1,cursor=$2,lease_until=$3,updated_at=$4 WHERE key='refunds'")
        .bind(end)
        .bind(&cursor)
        .bind(current + SCAN_LEASE_SECONDS)
        .bind(current)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(Window {
        watermark,
        end,
        cursor,
    }))
}

/// Give the window back without moving it (`reset` restarts it from its
/// first page: a provider cursor can expire).
async fn release(app: &App, end: i64, cursor: Option<&str>, reset: bool) -> Result<()> {
    sqlx::query("UPDATE portal.supporter_yookassa_refund_scan SET lease_until=NULL,cursor=CASE WHEN $3 THEN NULL ELSE cursor END,updated_at=$4 WHERE key='refunds' AND window_end=$1 AND cursor IS NOT DISTINCT FROM $2")
        .bind(end)
        .bind(cursor)
        .bind(reset)
        .bind(now())
        .execute(&app.pool)
        .await?;
    Ok(())
}

async fn complete(app: &App, end: i64, cursor: Option<&str>) -> Result<bool> {
    let current = now();
    Ok(sqlx::query("UPDATE portal.supporter_yookassa_refund_scan SET watermark=greatest(watermark,$1-$3),window_end=NULL,cursor=NULL,lease_until=NULL,last_successful_at=$4,updated_at=$4 WHERE key='refunds' AND window_end=$1 AND cursor IS NOT DISTINCT FROM $2")
        .bind(end)
        .bind(cursor)
        .bind(WATERMARK_OVERLAP_SECONDS)
        .bind(current)
        .execute(&app.pool)
        .await?
        .rows_affected()
        == 1)
}

async fn advance(app: &App, end: i64, cursor: Option<&str>, next: &str) -> Result<bool> {
    let current = now();
    Ok(sqlx::query("UPDATE portal.supporter_yookassa_refund_scan SET cursor=$3,lease_until=$4,last_successful_at=$5,updated_at=$5 WHERE key='refunds' AND window_end=$1 AND cursor IS NOT DISTINCT FROM $2")
        .bind(end)
        .bind(cursor)
        .bind(next)
        .bind(current + SCAN_LEASE_SECONDS)
        .bind(current)
        .execute(&app.pool)
        .await?
        .rows_affected()
        == 1)
}

fn count(report: &mut Report, outcome: RefundOutcome) {
    match outcome {
        RefundOutcome::Changed => report.refunds_changed += 1,
        RefundOutcome::Unchanged => {}
        RefundOutcome::Pending | RefundOutcome::Unknown | RefundOutcome::Failed => {
            report.refund_issues += 1
        }
    }
}

/// Retry queued refunds that may settle now (pending ones, failed settlements).
async fn retry_issues(app: &App, config: &YooKassaConfig, report: &mut Report) -> Result<()> {
    let queued: Vec<String> = sqlx::query_scalar("SELECT provider_object_id FROM portal.supporter_yookassa_issues WHERE resolved_at IS NULL AND object_type='refund' AND code IN ('pending_refund','settlement_failed') ORDER BY last_seen_at LIMIT $1")
        .bind(ISSUE_RETRIES)
        .fetch_all(&app.pool)
        .await?;
    for id in queued {
        report.refunds_checked += 1;
        match api::get_refund(config, &id).await {
            Ok(refund) => count(report, handle_refund(app, config, &refund).await?),
            Err(error) => {
                report.errors += 1;
                report.refund_issues += 1;
                eprintln!("yookassa refund retry: {}", error.1);
                record_issue(&app.pool, &id, "refund", "settlement_failed").await?;
            }
        }
    }
    Ok(())
}

async fn refunds(app: &App, config: &YooKassaConfig, report: &mut Report) -> Result<()> {
    let Some(window) = claim_window(app, now()).await? else {
        report.refund_scan = "busy";
        return Ok(());
    };
    retry_issues(app, config, report).await?;
    let mut cursor = window.cursor.clone();
    if window.watermark >= window.end {
        report.refund_scan = if complete(app, window.end, cursor.as_deref()).await? {
            "complete"
        } else {
            "lost"
        };
        return Ok(());
    }
    let mut seen = std::collections::HashSet::new();
    for _ in 0..REFUND_PAGES {
        let page = match api::list_refunds(
            config,
            window.watermark,
            window.end,
            cursor.as_deref(),
            REFUND_PAGE,
        )
        .await
        {
            Ok(page) => page,
            Err(error) => {
                report.errors += 1;
                eprintln!("yookassa refund scan: {}", error.1);
                // Restart the same fixed window, never a newer one.
                release(app, window.end, cursor.as_deref(), cursor.is_some()).await?;
                report.refund_scan = "failed";
                return Ok(());
            }
        };
        for item in &page.items {
            report.refunds_checked += 1;
            match handle_refund(app, config, item).await {
                Ok(outcome) => count(report, outcome),
                Err(error) => {
                    // Not quarantined: this page must be read again.
                    report.errors += 1;
                    eprintln!("yookassa refund scan: {}", error.1);
                    release(app, window.end, cursor.as_deref(), false).await?;
                    report.refund_scan = "failed";
                    return Ok(());
                }
            }
        }
        match page.next_cursor {
            None => {
                report.refund_scan = if complete(app, window.end, cursor.as_deref()).await? {
                    "complete"
                } else {
                    "lost"
                };
                return Ok(());
            }
            Some(next)
                if cursor.as_deref() == Some(next.as_str()) || !seen.insert(next.clone()) =>
            {
                report.errors += 1;
                release(app, window.end, cursor.as_deref(), true).await?;
                report.refund_scan = "restarted";
                return Ok(());
            }
            Some(next) => {
                if !advance(app, window.end, cursor.as_deref(), &next).await? {
                    report.refund_scan = "lost";
                    return Ok(());
                }
                cursor = Some(next);
            }
        }
    }
    // More pages than one pass reads: the next pass continues at the cursor.
    release(app, window.end, cursor.as_deref(), false).await?;
    report.refund_scan = "capped";
    Ok(())
}
