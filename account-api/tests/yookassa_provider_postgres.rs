//! The YooKassa provider flow (LR-67b) against a disposable PostgreSQL database
//! and a loopback YooKassa stand-in: checkout, the webhook and the
//! reconciliation loop. The stand-in answers like YooKassa's `/v3` API
//! (idempotent creates per Idempotence-Key, payments, refunds and paged refund
//! lists) and counts every call.
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::{
    Json, Router,
    routing::{get, post},
};
use ed25519_dalek::{Signer, SigningKey};
use omoba_account_api::supporter::yookassa::{
    self, KOPECKS, YooKassaConfig, api,
    checkout::{self, DESCRIPTION},
    reconcile,
};
use omoba_account_api::supporter::{self, BillingConfig};
use omoba_account_api::{App, Config, Result, auth, crypto, migrate, now, router};
use serde_json::{Value, json};
use shared::market::PaymentProvider;
use shared::supporter::{AuraStyle, SupporterTier};
use shared::web_account::{SignedWebPair, WebPairChallenge, WebPairDecision};
use sqlx::Row;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const SHOP: &str = "123456";
const SECRET: &str = "test_shop_secret_key";
const ORIGIN: &str = "https://players.example";

fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// One POST /payments the stand-in received.
#[derive(Clone, Debug)]
struct Posted {
    key: String,
    body: Value,
    /// The attempt with this key was committed and claimed before the POST.
    persisted: bool,
}

#[derive(Default)]
struct Shop {
    payments: HashMap<String, Value>,
    keys: HashMap<String, String>,
    refunds: Vec<Value>,
    posts: Vec<Posted>,
    calls: usize,
    /// Create the payment but answer 500 (the reply is lost).
    lose_next_create: bool,
    /// Answer 503 to every GET.
    fail_gets: bool,
    /// Answer 503 to the next refund list.
    fail_next_list: bool,
}

#[derive(Clone)]
struct Stand {
    shop: Arc<Mutex<Shop>>,
    pool: sqlx::PgPool,
    addr: SocketAddr,
}

type Reply = (StatusCode, [(&'static str, &'static str); 1], Json<Value>);

fn reply(status: StatusCode, value: Value) -> Reply {
    (
        status,
        [("x-request-id", "stand-in-request-1")],
        Json(value),
    )
}

fn authorized(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok())
        == Some(&format!(
            "Basic {}",
            base64(format!("{SHOP}:{SECRET}").as_bytes())
        ))
}

async fn create_payment(
    State(stand): State<Stand>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Reply {
    let key = headers
        .get("idempotence-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let persisted: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.supporter_yookassa_attempts WHERE idempotence_key=$1 AND status='charging' AND lease_until IS NOT NULL AND first_provider_attempt_at IS NOT NULL)")
        .bind(&key)
        .fetch_one(&stand.pool)
        .await
        .unwrap();
    let mut shop = stand.shop.lock().unwrap();
    shop.calls += 1;
    shop.posts.push(Posted {
        key: key.clone(),
        body: body.clone(),
        persisted,
    });
    if !authorized(&headers) || key.is_empty() {
        return reply(StatusCode::UNAUTHORIZED, json!({"type": "error"}));
    }
    // YooKassa answers a repeated key with the payment the first request made.
    if let Some(id) = shop.keys.get(&key).cloned() {
        return reply(StatusCode::OK, shop.payments[&id].clone());
    }
    let id = format!(
        "2f{}-000f-5000-9000-{}",
        crypto::random::<3>(),
        crypto::random::<6>()
    );
    let payment = json!({
        "id": id,
        "status": "pending",
        "paid": false,
        "amount": body["amount"],
        "description": body["description"],
        "metadata": body["metadata"],
        "created_at": api::format_time(now()),
        "confirmation": {
            "type": "redirect",
            "confirmation_url": format!("https://yoomoney.example/checkout/payments/v2/contract?orderId={id}"),
        },
        "recipient": {"account_id": SHOP, "gateway_id": "1"},
        "refundable": false,
        "test": true,
    });
    shop.payments.insert(id.clone(), payment.clone());
    shop.keys.insert(key, id);
    if std::mem::take(&mut shop.lose_next_create) {
        return reply(StatusCode::INTERNAL_SERVER_ERROR, json!({"type": "error"}));
    }
    reply(StatusCode::OK, payment)
}

async fn get_payment(
    State(stand): State<Stand>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Reply {
    let mut shop = stand.shop.lock().unwrap();
    shop.calls += 1;
    if !authorized(&headers) {
        return reply(StatusCode::UNAUTHORIZED, json!({"type": "error"}));
    }
    if shop.fail_gets {
        return reply(StatusCode::SERVICE_UNAVAILABLE, json!({"type": "error"}));
    }
    match shop.payments.get(&id) {
        Some(payment) => reply(StatusCode::OK, payment.clone()),
        None => reply(StatusCode::NOT_FOUND, json!({"type": "error"})),
    }
}

async fn get_refund(
    State(stand): State<Stand>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Reply {
    let mut shop = stand.shop.lock().unwrap();
    shop.calls += 1;
    if !authorized(&headers) {
        return reply(StatusCode::UNAUTHORIZED, json!({"type": "error"}));
    }
    if shop.fail_gets {
        return reply(StatusCode::SERVICE_UNAVAILABLE, json!({"type": "error"}));
    }
    match shop.refunds.iter().find(|refund| refund["id"] == id) {
        Some(refund) => reply(StatusCode::OK, refund.clone()),
        None => reply(StatusCode::NOT_FOUND, json!({"type": "error"})),
    }
}

async fn list_refunds(
    State(stand): State<Stand>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Reply {
    let mut shop = stand.shop.lock().unwrap();
    shop.calls += 1;
    if !authorized(&headers) {
        return reply(StatusCode::UNAUTHORIZED, json!({"type": "error"}));
    }
    if std::mem::take(&mut shop.fail_next_list) {
        return reply(StatusCode::SERVICE_UNAVAILABLE, json!({"type": "error"}));
    }
    let after = api::parse_time(&query["created_at.gt"]).unwrap();
    let until = api::parse_time(&query["created_at.lte"]).unwrap();
    let limit: usize = query["limit"].parse().unwrap();
    let matching: Vec<Value> = shop
        .refunds
        .iter()
        .filter(|refund| {
            let at = api::parse_time(refund["created_at"].as_str().unwrap()).unwrap();
            after < at && at <= until
        })
        .cloned()
        .collect();
    let offset: usize = query
        .get("cursor")
        .map(|cursor| cursor.trim_start_matches("cursor-").parse().unwrap())
        .unwrap_or(0);
    let end = (offset + limit).min(matching.len());
    let mut page = json!({"type": "list", "items": matching[offset.min(end)..end]});
    if end < matching.len() {
        page["next_cursor"] = json!(format!("cursor-{end}"));
    }
    reply(StatusCode::OK, page)
}

impl Stand {
    async fn start(pool: sqlx::PgPool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let stand = Self {
            shop: Arc::default(),
            pool,
            addr,
        };
        let routes = Router::new()
            .route("/v3/payments", post(create_payment))
            .route("/v3/payments/{id}", get(get_payment))
            .route("/v3/refunds", get(list_refunds))
            .route("/v3/refunds/{id}", get(get_refund))
            .with_state(stand.clone());
        tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        stand
    }
    fn config(&self, new_charges: bool) -> YooKassaConfig {
        YooKassaConfig {
            shop_id: SHOP.into(),
            secret_key: SECRET.into(),
            vat_code: 1,
            new_charges,
            api_base: format!("http://{}/v3", self.addr),
        }
    }
    fn calls(&self) -> usize {
        self.shop.lock().unwrap().calls
    }
    fn posts(&self) -> Vec<Posted> {
        self.shop.lock().unwrap().posts.clone()
    }
    fn payments(&self) -> usize {
        self.shop.lock().unwrap().payments.len()
    }
    fn with<T>(&self, change: impl FnOnce(&mut Shop) -> T) -> T {
        change(&mut self.shop.lock().unwrap())
    }
    /// The player pays: the payment succeeds, captured at `at`.
    fn pay(&self, id: &str, at: i64) {
        self.with(|shop| {
            let payment = shop.payments.get_mut(id).unwrap();
            payment["status"] = json!("succeeded");
            payment["paid"] = json!(true);
            payment["captured_at"] = json!(api::format_time(at));
            payment["refundable"] = json!(true);
            payment.as_object_mut().unwrap().remove("confirmation");
        });
    }
    fn set_status(&self, id: &str, status: &str, paid: bool) {
        self.with(|shop| {
            let payment = shop.payments.get_mut(id).unwrap();
            payment["status"] = json!(status);
            payment["paid"] = json!(paid);
        });
    }
    /// A succeeded refund of `rubles`, created at `at`.
    fn refund(&self, payment: &str, rubles: i64, at: i64) -> String {
        let id = format!(
            "3a{}-0015-5000-8000-{}",
            crypto::random::<3>(),
            crypto::random::<6>()
        );
        self.with(|shop| {
            shop.refunds.push(json!({
                "id": id,
                "payment_id": payment,
                "status": "succeeded",
                "amount": {"value": api::amount_value(rubles * KOPECKS), "currency": "RUB"},
                "created_at": api::format_time(at),
            }))
        });
        id
    }
}

fn config() -> Config {
    Config {
        origin: ORIGIN.into(),
        secret: [97; 32],
        trust_loopback_proxy: false,
        release_file: None,
    }
}

async fn owner_app() -> App {
    let url =
        std::env::var("OMOBA_PORTAL_TEST_DATABASE_URL").expect("isolated test database required");
    migrate(&url).await.unwrap();
    App::connect(&url, config()).await.unwrap()
}

fn with_shop(mut app: App, stand: &Stand, new_charges: bool) -> App {
    app.billing = Arc::new(BillingConfig {
        yookassa: Some(stand.config(new_charges)),
        ..Default::default()
    });
    app
}

/// Tests share one disposable database: park other tests' open attempts and
/// issues, and start the refund scan over a fresh window (`fresh`: a recent
/// successful scan, so new charges are open).
async fn quiesce(app: &App, fresh: bool) {
    let current = now();
    for query in [
        "UPDATE portal.supporter_yookassa_attempts SET status='canceled',lease_until=NULL,updated_at=$1 WHERE provider_payment_id IS NULL AND status IN ('created','charging','failed')",
        "UPDATE portal.supporter_yookassa_attempts SET last_polled_at=$1+1000000000 WHERE status='submitted'",
        "UPDATE portal.supporter_yookassa_issues SET resolved_at=$1 WHERE resolved_at IS NULL",
    ] {
        sqlx::query(query)
            .bind(current)
            .execute(&app.pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE portal.supporter_yookassa_refund_scan SET watermark=$1-60,window_end=NULL,cursor=NULL,lease_until=NULL,last_successful_at=$2,updated_at=$1")
        .bind(current)
        .bind(fresh.then_some(current))
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn account(app: &App) -> String {
    app.career
        .authenticate(&crypto::random::<32>(), "Donor")
        .await
        .unwrap()
        .profile_id
}

fn headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("idempotency-key", key.parse().unwrap());
    headers
}

fn checkout_body(rubles: i64, email: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "amount_rub": rubles,
        "receipt_email": email,
        "quote": checkout::quote(rubles),
        "offer_version": "0-draft",
    }))
    .unwrap()
}

async fn donate(app: &App, profile: &str, key: &str, rubles: i64) -> Result<Value> {
    yookassa::checkout(
        app,
        profile,
        &headers(key),
        &checkout_body(rubles, "donor@example.ru"),
    )
    .await
}

async fn notify(app: &App, event: &str, object: Value) -> Result<Value> {
    let body = json!({"type": "notification", "event": event, "object": object});
    yookassa::notification(app, &serde_json::to_vec(&body).unwrap()).await
}

/// (status, provider payment id, last error) of an attempt.
async fn attempt(app: &App, id: &str) -> (String, Option<String>, Option<String>) {
    let row = sqlx::query("SELECT status,provider_payment_id,last_error FROM portal.supporter_yookassa_attempts WHERE attempt_id=$1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    (
        row.get("status"),
        row.get("provider_payment_id"),
        row.get("last_error"),
    )
}

async fn payment_of(app: &App, attempt_id: &str) -> String {
    attempt(app, attempt_id).await.1.expect("bound payment")
}

/// (amount_minor, tier_points, revoked) of a donation's grant.
async fn grant(app: &App, payment: &str) -> (i64, i32, bool) {
    let row = sqlx::query("SELECT amount_minor,tier_points,revoked_at FROM portal.supporter_grants WHERE provider='yookassa' AND period_id=$1")
        .bind(payment)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    (
        row.get("amount_minor"),
        row.get("tier_points"),
        row.get::<Option<i64>, _>("revoked_at").is_some(),
    )
}

async fn grants(app: &App, profile: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM portal.supporter_grants WHERE profile_id=$1")
        .bind(profile)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn open_issue(app: &App, object: &str) -> Option<String> {
    sqlx::query_scalar("SELECT code FROM portal.supporter_yookassa_issues WHERE provider_object_id=$1 AND resolved_at IS NULL")
        .bind(object)
        .fetch_optional(&app.pool)
        .await
        .unwrap()
}

async fn cleanup(app: &App, profile: &str) {
    for query in [
        "DELETE FROM portal.supporter_yookassa_refunds WHERE provider_payment_id IN (SELECT provider_payment_id FROM portal.supporter_yookassa_attempts WHERE profile_id=$1)",
        "DELETE FROM portal.supporter_yookassa_attempts WHERE profile_id=$1",
        "DELETE FROM portal.supporter_events WHERE profile_id=$1",
        "DELETE FROM portal.supporter_grants WHERE profile_id=$1",
        "DELETE FROM portal.supporter_preferences WHERE profile_id=$1",
    ] {
        sqlx::query(query)
            .bind(profile)
            .execute(&app.pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn checkout_persists_the_key_before_the_post_and_sends_the_frozen_receipt() {
    let owner = owner_app().await;
    let stand = Stand::start(owner.pool.clone()).await;
    let app = with_shop(owner, &stand, true);
    quiesce(&app, true).await;
    let profile = account(&app).await;
    let key = crypto::random::<16>();

    // The browser's quote must be the server's; nothing reaches YooKassa otherwise.
    let mut stale = json!({"amount_rub": 1000, "receipt_email": "donor@example.ru",
        "quote": checkout::quote(1000), "offer_version": "0-draft"});
    stale["quote"]["tier"] = json!("azure");
    let error = yookassa::checkout(
        &app,
        &profile,
        &headers(&key),
        &serde_json::to_vec(&stale).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!((error.0, error.1), (StatusCode::CONFLICT, "reload"));
    for (rubles, email, code) in [
        (99, "donor@example.ru", "amount_not_allowed"),
        (100_001, "donor@example.ru", "amount_not_allowed"),
        (1000, "not-an-address", "receipt_email_invalid"),
    ] {
        let error = yookassa::checkout(
            &app,
            &profile,
            &headers(&key),
            &checkout_body(rubles, email),
        )
        .await
        .unwrap_err();
        assert_eq!(error.1, code);
    }
    let error = yookassa::checkout(
        &app,
        &profile,
        &HeaderMap::new(),
        &checkout_body(1000, "donor@example.ru"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.1, "invalid_request", "an Idempotency-Key is required");
    assert_eq!(stand.calls(), 0);

    let created = donate(&app, &profile, &key, 1000).await.unwrap();
    let url = created["confirmation_url"].as_str().unwrap();
    assert!(url.starts_with("https://yoomoney.example/"), "{url}");
    let id = created["attempt_id"].as_str().unwrap().to_owned();
    let posts = stand.posts();
    assert_eq!(posts.len(), 1);
    // The attempt, its key and its receipt were committed and claimed before the POST.
    assert!(posts[0].persisted);
    let stored: String = sqlx::query_scalar(
        "SELECT idempotence_key FROM portal.supporter_yookassa_attempts WHERE attempt_id=$1",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(posts[0].key, stored);
    assert_eq!(
        posts[0].body,
        json!({
            "amount": {"value": "1000.00", "currency": "RUB"},
            "capture": true,
            "confirmation": {"type": "redirect", "return_url": format!("{ORIGIN}/supporter?paid=1")},
            "description": DESCRIPTION,
            "metadata": {"attempt_id": id},
            "receipt": {
                "customer": {"email": "donor@example.ru"},
                "items": [{
                    "description": "OMOBA — поддержка проекта (аура на 30 дней)",
                    "quantity": "1.00",
                    "amount": {"value": "1000.00", "currency": "RUB"},
                    "vat_code": 1,
                    "payment_mode": "full_payment",
                    "payment_subject": "service",
                }],
            },
        })
    );
    let (status, payment, _) = attempt(&app, &id).await;
    assert_eq!(status, "submitted");
    let payment = payment.unwrap();

    // The same Idempotency-Key is the same attempt: the payment is re-read, never re-created.
    let again = donate(&app, &profile, &key, 1000).await.unwrap();
    assert_eq!(again, created);
    assert_eq!(stand.posts().len(), 1);
    let error = donate(&app, &profile, &key, 500).await.unwrap_err();
    assert_eq!(error.1, "idempotency_conflict");
    let error = yookassa::checkout(
        &app,
        &profile,
        &headers(&key),
        &checkout_body(1000, "other@example.ru"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.1, "idempotency_conflict");

    // The owner's list; another account sees nothing of it.
    let listed = yookassa::payments(&app, &profile).await.unwrap();
    assert_eq!(listed["payments"][0]["attempt_id"], id.as_str());
    assert_eq!(listed["payments"][0]["status"], "pending");
    assert_eq!(listed["payments"][0]["amount_minor"], 100_000);
    let stranger = account(&app).await;
    assert_eq!(
        yookassa::payments(&app, &stranger).await.unwrap()["payments"],
        json!([])
    );

    // The player pays; back on the portal the list re-reads the newest payment.
    let paid_at = now() - 5;
    stand.pay(&payment, paid_at);
    sqlx::query(
        "UPDATE portal.supporter_yookassa_attempts SET last_polled_at=0 WHERE attempt_id=$1",
    )
    .bind(&id)
    .execute(&app.pool)
    .await
    .unwrap();
    let listed = yookassa::payments(&app, &profile).await.unwrap();
    assert_eq!(listed["payments"][0]["status"], "succeeded");
    assert_eq!(grant(&app, &payment).await, (100_000, 1000, false));
    let status = supporter::status(&app, &profile).await.unwrap();
    assert_eq!(status.tier(), Some(SupporterTier::Crimson));
    assert_eq!(status.tier_until, Some(paid_at + yookassa::GRANT_SECONDS));
    assert_eq!(status.equipped_aura, Some(AuraStyle::Crimson));
    let error = donate(&app, &profile, &key, 1000).await.unwrap_err();
    assert_eq!(error.1, "payment_succeeded");
    let dashboard = supporter::dashboard(&app, &profile).await.unwrap();
    assert_eq!(dashboard["yookassa"]["available"], true);
    assert_eq!(dashboard["yookassa"]["new_charges"], true);
    assert_eq!(
        dashboard["yookassa"]["tiers_rub"],
        json!(yookassa::TIER_AMOUNTS_RUB)
    );

    // New charges stop with the switch off or a stale refund scan; nothing is posted.
    let posts = stand.posts().len();
    let paused = with_shop(app.clone(), &stand, false);
    let error = donate(&paused, &profile, &crypto::random::<16>(), 250)
        .await
        .unwrap_err();
    assert_eq!(
        (error.0, error.1),
        (StatusCode::SERVICE_UNAVAILABLE, "new_charges_paused")
    );
    sqlx::query("UPDATE portal.supporter_yookassa_refund_scan SET last_successful_at=$1")
        .bind(now() - checkout::REFUND_SCAN_MAX_AGE_SECONDS - 1)
        .execute(&app.pool)
        .await
        .unwrap();
    let error = donate(&app, &profile, &crypto::random::<16>(), 250)
        .await
        .unwrap_err();
    assert_eq!(error.1, "new_charges_paused");
    assert_eq!(
        supporter::dashboard(&app, &profile).await.unwrap()["yookassa"]["new_charges"],
        false
    );
    assert_eq!(stand.posts().len(), posts);
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn lost_creates_replay_with_the_same_key_inside_23_hours_and_go_to_review_after() {
    let owner = owner_app().await;
    let stand = Stand::start(owner.pool.clone()).await;
    let app = with_shop(owner, &stand, true);
    quiesce(&app, true).await;
    let profile = account(&app).await;

    // YooKassa created the payment, but its reply was lost.
    stand.with(|shop| shop.lose_next_create = true);
    let key = crypto::random::<16>();
    let error = donate(&app, &profile, &key, 500).await.unwrap_err();
    assert_eq!(
        (error.0, error.1),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "payment_provider_unavailable"
        )
    );
    let id: String = sqlx::query_scalar(
        "SELECT attempt_id FROM portal.supporter_yookassa_attempts WHERE profile_id=$1",
    )
    .bind(&profile)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        attempt(&app, &id).await,
        (
            "failed".into(),
            None,
            Some("payment_provider_unavailable".into())
        )
    );
    assert_eq!(stand.payments(), 1);

    // Reconciliation replays it with the same key and the same body: one payment.
    let report = reconcile(&app).await.unwrap();
    assert_eq!((report.attempts, report.replayed), (1, 1), "{report:?}");
    let posts = stand.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].key, posts[1].key);
    assert_eq!(posts[0].body, posts[1].body);
    assert!(posts.iter().all(|post| post.persisted));
    assert_eq!(stand.payments(), 1);
    let (status, payment, _) = attempt(&app, &id).await;
    assert_eq!(status, "submitted");
    let payment = payment.unwrap();
    // A submitted attempt is polled, not posted again; once paid it settles.
    stand.pay(&payment, now() - 3);
    sqlx::query(
        "UPDATE portal.supporter_yookassa_attempts SET last_polled_at=0 WHERE attempt_id=$1",
    )
    .bind(&id)
    .execute(&app.pool)
    .await
    .unwrap();
    let report = reconcile(&app).await.unwrap();
    assert_eq!((report.settled, report.replayed), (1, 0), "{report:?}");
    assert_eq!(stand.posts().len(), 2);
    assert_eq!(attempt(&app, &id).await.0, "succeeded");
    assert_eq!(grant(&app, &payment).await, (50_000, 500, false));

    // A crash after the claim (charging, lease expired) is replayed as well.
    stand.with(|shop| shop.lose_next_create = true);
    let crashed_key = crypto::random::<16>();
    donate(&app, &profile, &crashed_key, 100).await.unwrap_err();
    let crashed: String = sqlx::query_scalar("SELECT attempt_id FROM portal.supporter_yookassa_attempts WHERE profile_id=$1 AND amount_minor=10000")
        .bind(&profile)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE portal.supporter_yookassa_attempts SET status='charging',last_error=NULL,lease_until=$2 WHERE attempt_id=$1")
        .bind(&crashed)
        .bind(now() - 1)
        .execute(&app.pool)
        .await
        .unwrap();
    // The browser retries with its key: the same attempt, replayed, with its redirect.
    let recovered = donate(&app, &profile, &crashed_key, 100).await.unwrap();
    assert_eq!(recovered["attempt_id"], crashed.as_str());
    assert!(
        recovered["confirmation_url"]
            .as_str()
            .unwrap()
            .starts_with("https://")
    );
    let posts = stand.posts();
    assert_eq!(posts[posts.len() - 1].key, posts[posts.len() - 2].key);
    assert_eq!(attempt(&app, &crashed).await.0, "submitted");

    // Past 23 hours a lost create goes to manual review and is never posted.
    stand.with(|shop| shop.lose_next_create = true);
    let late_key = crypto::random::<16>();
    donate(&app, &profile, &late_key, 250).await.unwrap_err();
    let late: String = sqlx::query_scalar("SELECT attempt_id FROM portal.supporter_yookassa_attempts WHERE profile_id=$1 AND amount_minor=25000")
        .bind(&profile)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE portal.supporter_yookassa_attempts SET first_provider_attempt_at=$2 WHERE attempt_id=$1")
        .bind(&late)
        .bind(now() - yookassa::REPLAY_WINDOW_SECONDS - 1)
        .execute(&app.pool)
        .await
        .unwrap();
    let posts = stand.posts().len();
    let report = reconcile(&app).await.unwrap();
    assert_eq!(report.review, 1, "{report:?}");
    assert_eq!(stand.posts().len(), posts);
    assert_eq!(
        attempt(&app, &late).await,
        ("failed".into(), None, Some(checkout::REPLAY_EXPIRED.into()))
    );
    assert_eq!(
        open_issue(&app, &late).await.as_deref(),
        Some("replay_window_expired")
    );
    let error = donate(&app, &profile, &late_key, 250).await.unwrap_err();
    assert_eq!(error.1, "payment_manual_review");
    // It stays out of later passes and shows as review to its owner.
    sqlx::query(
        "UPDATE portal.supporter_yookassa_attempts SET last_polled_at=0 WHERE attempt_id=$1",
    )
    .bind(&late)
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        reconcile(&app).await.unwrap().attempts,
        0,
        "a review attempt is never polled again"
    );
    assert_eq!(stand.posts().len(), posts);
    let listed = yookassa::payments(&app, &profile).await.unwrap();
    let late_entry = listed["payments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["attempt_id"] == late.as_str())
        .unwrap()
        .clone();
    assert_eq!(late_entry["status"], "review");
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn webhooks_reread_known_objects_and_acknowledge_unknown_ones_without_a_call() {
    let owner = owner_app().await;
    let stand = Stand::start(owner.pool.clone()).await;
    let app = with_shop(owner, &stand, true);
    quiesce(&app, true).await;
    let profile = account(&app).await;

    // Unknown objects: 200 without any provider call.
    let calls = stand.calls();
    for (event, object) in [
        (
            "payment.succeeded",
            json!({"id": "2f000000-000f-5000-9000-unknown", "status": "succeeded"}),
        ),
        (
            "payment.canceled",
            json!({"id": "2f000000-000f-5000-9000-unknown2", "metadata": {"attempt_id": "0".repeat(32)}}),
        ),
        (
            "refund.succeeded",
            json!({"id": "3a000000-0015-5000-8000-unknown", "payment_id": "2f000000-000f-5000-9000-unknown"}),
        ),
        ("payment.succeeded", json!({"id": "not a provider id"})),
        ("deal.closed", json!({"id": "2f000000"})),
    ] {
        assert_eq!(
            notify(&app, event, object).await.unwrap()["status"],
            "ignored",
            "{event}"
        );
    }
    assert_eq!(stand.calls(), calls, "unknown objects cost no API call");
    assert_eq!(
        yookassa::notification(&app, b"not json")
            .await
            .unwrap_err()
            .1,
        "invalid_request"
    );
    let unconfigured = App {
        billing: Arc::new(BillingConfig::default()),
        ..app.clone()
    };
    assert_eq!(
        notify(&unconfigured, "payment.succeeded", json!({"id": "x"}))
            .await
            .unwrap_err()
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );

    let created = donate(&app, &profile, &crypto::random::<16>(), 1000)
        .await
        .unwrap();
    let id = created["attempt_id"].as_str().unwrap().to_owned();
    let payment = payment_of(&app, &id).await;
    // The body is never trusted: "succeeded" for a pending payment grants nothing.
    let forged = json!({"id": payment, "status": "succeeded", "paid": true,
        "amount": {"value": "100000.00", "currency": "RUB"}});
    assert_eq!(
        notify(&app, "payment.succeeded", forged.clone())
            .await
            .unwrap()["status"],
        "accepted"
    );
    assert_eq!(attempt(&app, &id).await.0, "submitted");
    assert_eq!(grants(&app, &profile).await, 0);

    // A provider error answers 5xx so YooKassa delivers it again.
    stand.pay(&payment, now() - 10);
    stand.with(|shop| shop.fail_gets = true);
    let error = notify(&app, "payment.succeeded", forged.clone())
        .await
        .unwrap_err();
    assert!(error.0.is_server_error(), "{error:?}");
    assert_eq!(grants(&app, &profile).await, 0);
    stand.with(|shop| shop.fail_gets = false);

    // Duplicate deliveries grant once.
    for _ in 0..3 {
        assert_eq!(
            notify(&app, "payment.succeeded", forged.clone())
                .await
                .unwrap()["status"],
            "accepted"
        );
    }
    assert_eq!(attempt(&app, &id).await.0, "succeeded");
    assert_eq!(grants(&app, &profile).await, 1);
    assert_eq!(grant(&app, &payment).await, (100_000, 1000, false));
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal.supporter_events WHERE period_id=$1")
            .bind(&payment)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(events, 1);

    // A late cancel never downgrades a success.
    stand.set_status(&payment, "canceled", false);
    assert_eq!(
        notify(&app, "payment.canceled", json!({"id": payment}))
            .await
            .unwrap()["status"],
        "accepted"
    );
    assert_eq!(attempt(&app, &id).await.0, "succeeded");
    let status = supporter::status(&app, &profile).await.unwrap();
    assert!(status.active);
    assert_eq!(status.tier(), Some(SupporterTier::Crimson));

    // A canceled payment closes its attempt and grants nothing.
    let other = donate(&app, &profile, &crypto::random::<16>(), 250)
        .await
        .unwrap();
    let other_id = other["attempt_id"].as_str().unwrap().to_owned();
    let other_payment = payment_of(&app, &other_id).await;
    stand.set_status(&other_payment, "canceled", false);
    notify(&app, "payment.canceled", json!({"id": other_payment}))
        .await
        .unwrap();
    assert_eq!(attempt(&app, &other_id).await.0, "canceled");
    assert_eq!(grants(&app, &profile).await, 1);
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn refunds_overtake_payments_and_partial_and_full_refunds_recompute_the_tier() {
    let owner = owner_app().await;
    let stand = Stand::start(owner.pool.clone()).await;
    let app = with_shop(owner, &stand, true);
    quiesce(&app, true).await;
    let profile = account(&app).await;
    let created = donate(&app, &profile, &crypto::random::<16>(), 2500)
        .await
        .unwrap();
    let id = created["attempt_id"].as_str().unwrap().to_owned();
    let payment = payment_of(&app, &id).await;
    stand.pay(&payment, now() - 40);

    // The refund webhook overtakes the payment webhook: the payment settles first.
    let first = stand.refund(&payment, 1500, now() - 30);
    let accepted = notify(
        &app,
        "refund.succeeded",
        json!({"id": first, "payment_id": payment}),
    )
    .await
    .unwrap();
    assert_eq!(accepted["status"], "accepted");
    assert_eq!(attempt(&app, &id).await.0, "succeeded");
    assert_eq!(grant(&app, &payment).await, (250_000, 1000, false));
    assert_eq!(
        supporter::status(&app, &profile).await.unwrap().tier(),
        Some(SupporterTier::Crimson)
    );
    // Its duplicate and the late payment webhook change nothing.
    notify(
        &app,
        "refund.succeeded",
        json!({"id": first, "payment_id": payment}),
    )
    .await
    .unwrap();
    notify(&app, "payment.succeeded", json!({"id": payment}))
        .await
        .unwrap();
    assert_eq!(grant(&app, &payment).await.1, 1000);
    // A refund naming a payment this API does not know costs no call.
    let calls = stand.calls();
    let ignored = notify(
        &app,
        "refund.succeeded",
        json!({"id": first, "payment_id": "2f0000-000f-5000-9000-other"}),
    )
    .await
    .unwrap();
    assert_eq!(
        (ignored["status"].as_str(), stand.calls()),
        (Some("ignored"), calls)
    );

    // A partial refund whose webhook is lost: the refund scan finds it (2500-1500-750 = 250, Jade).
    stand.refund(&payment, 750, now() - 20);
    let report = reconcile(&app).await.unwrap();
    assert_eq!(report.refund_scan, "complete", "{report:?}");
    assert_eq!(report.refunds_changed, 1, "{report:?}");
    assert_eq!(grant(&app, &payment).await, (250_000, 250, false));
    let status = supporter::status(&app, &profile).await.unwrap();
    assert_eq!(status.tier(), Some(SupporterTier::Jade));
    assert_eq!(
        status.equipped_aura,
        Some(AuraStyle::Jade),
        "the style steps down with the tier"
    );
    let (watermark, window, last): (i64, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT watermark,window_end,last_successful_at FROM portal.supporter_yookassa_refund_scan WHERE key='refunds'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(window.is_none() && last.is_some_and(|at| at >= now() - 5));
    assert!(watermark >= now() - reconcile::WATERMARK_OVERLAP_SECONDS - 5);

    // The rest: a full refund revokes the grant and ends access.
    stand.refund(&payment, 250, now() - 1);
    sqlx::query("UPDATE portal.supporter_yookassa_refund_scan SET watermark=$1")
        .bind(now() - 60)
        .execute(&app.pool)
        .await
        .unwrap();
    let report = reconcile(&app).await.unwrap();
    assert_eq!(report.refunds_changed, 1, "{report:?}");
    assert_eq!(grant(&app, &payment).await, (250_000, 0, true));
    let status = supporter::status(&app, &profile).await.unwrap();
    assert!(!status.active && status.tier().is_none());
    let refunds: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal.supporter_yookassa_refunds WHERE provider_payment_id=$1",
    )
    .bind(&payment)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(refunds, 3);
    let listed = yookassa::payments(&app, &profile).await.unwrap();
    assert_eq!(listed["payments"][0]["refunded_minor"], 250_000);
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn the_refund_scan_keeps_a_durable_cursor_and_restarts_its_window_after_a_failure() {
    let owner = owner_app().await;
    let stand = Stand::start(owner.pool.clone()).await;
    let app = with_shop(owner, &stand, true);
    quiesce(&app, false).await;
    // More refunds than one pass reads; none of them is a payment of this API.
    let total = reconcile::REFUND_PAGE * reconcile::REFUND_PAGES + 6;
    let mut ids = Vec::new();
    for index in 0..total {
        ids.push(stand.refund(
            &format!("2f0000-000f-5000-9000-{index:06}"),
            100,
            now() - 30,
        ));
    }
    // No scan has succeeded yet: new charges are closed.
    let profile = account(&app).await;
    assert_eq!(
        donate(&app, &profile, &crypto::random::<16>(), 100)
            .await
            .unwrap_err()
            .1,
        "new_charges_paused"
    );
    let report = reconcile(&app).await.unwrap();
    assert_eq!(report.refund_scan, "capped", "{report:?}");
    let (cursor, window): (Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT cursor,window_end FROM portal.supporter_yookassa_refund_scan WHERE key='refunds'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        cursor.as_deref(),
        Some(
            format!(
                "cursor-{}",
                reconcile::REFUND_PAGE * reconcile::REFUND_PAGES
            )
            .as_str()
        )
    );
    // Every unknown refund was queued for an operator before the cursor moved.
    for id in &ids[..reconcile::REFUND_PAGE * reconcile::REFUND_PAGES] {
        assert_eq!(
            open_issue(&app, id).await.as_deref(),
            Some("unknown_payment")
        );
    }
    // A failed list restarts the same fixed window, never a newer one.
    stand.with(|shop| shop.fail_next_list = true);
    let report = reconcile(&app).await.unwrap();
    assert_eq!(report.refund_scan, "failed", "{report:?}");
    let (cursor, same_window): (Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT cursor,window_end FROM portal.supporter_yookassa_refund_scan WHERE key='refunds'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!((cursor, same_window), (None, window));
    let mut passes = 0;
    loop {
        passes += 1;
        let report = reconcile(&app).await.unwrap();
        if report.refund_scan == "complete" {
            break;
        }
        assert_eq!(report.refund_scan, "capped", "{report:?}");
        assert!(passes < 5);
    }
    for id in &ids {
        assert_eq!(
            open_issue(&app, id).await.as_deref(),
            Some("unknown_payment")
        );
    }
    // The scan succeeded: new charges open again.
    assert!(
        donate(&app, &profile, &crypto::random::<16>(), 100)
            .await
            .is_ok()
    );
    quiesce(&app, true).await;
    cleanup(&app, &profile).await;
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL and OMOBA_PORTAL_ROLE_TEST_URL"]
async fn the_restricted_portal_role_runs_checkout_webhooks_and_reconciliation() {
    let owner = owner_app().await;
    let stand = Stand::start(owner.pool.clone()).await;
    quiesce(&owner, true).await;
    let runtime = App::connect(
        &std::env::var("OMOBA_PORTAL_ROLE_TEST_URL").expect("isolated portal runtime role"),
        config(),
    )
    .await
    .unwrap();
    let app = with_shop(runtime, &stand, true);
    let profile = account(&owner).await;
    let created = donate(&app, &profile, &crypto::random::<16>(), 5000)
        .await
        .unwrap();
    assert!(stand.posts().last().unwrap().persisted);
    let id = created["attempt_id"].as_str().unwrap().to_owned();
    let payment = payment_of(&owner, &id).await;
    stand.pay(&payment, now() - 20);
    notify(&app, "payment.succeeded", json!({"id": payment}))
        .await
        .unwrap();
    assert_eq!(
        supporter::status(&app, &profile).await.unwrap().tier(),
        Some(SupporterTier::PrismII)
    );
    // A refund through the scan, an unknown refund queued as an issue, a lost create replayed.
    stand.refund(&payment, 3000, now() - 10);
    stand.refund("2f0000-000f-5000-9000-elsewhere", 100, now() - 10);
    stand.with(|shop| shop.lose_next_create = true);
    donate(&app, &profile, &crypto::random::<16>(), 100)
        .await
        .unwrap_err();
    let report = reconcile(&app).await.unwrap();
    assert_eq!(
        (report.replayed, report.refunds_changed, report.refund_scan),
        (1, 1, "complete"),
        "{report:?}"
    );
    assert_eq!(grant(&owner, &payment).await, (500_000, 2000, false));
    assert_eq!(
        supporter::status(&app, &profile).await.unwrap().tier(),
        Some(SupporterTier::Crimson)
    );
    assert!(
        yookassa::payments(&app, &profile).await.unwrap()["payments"]
            .as_array()
            .unwrap()
            .len()
            == 2
    );
    quiesce(&owner, true).await;
    cleanup(&owner, &profile).await;
}

async fn login(app: &App) -> (String, String) {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).unwrap();
    let key = SigningKey::from_bytes(&seed);
    let public = crypto::hex(key.verifying_key().as_bytes());
    let profile = app
        .career
        .authenticate(&public, "Donor")
        .await
        .unwrap()
        .profile_id;
    let created = auth::create(app, b"{}").await.unwrap();
    let found = auth::lookup(
        app,
        &serde_json::to_vec(&json!({"code": created["code"], "public_key": public})).unwrap(),
    )
    .await
    .unwrap();
    let challenge: WebPairChallenge = serde_json::from_value(found["challenge"].clone()).unwrap();
    let proof = SignedWebPair {
        signature: crypto::hex(
            &key.sign(&challenge.signing_bytes(WebPairDecision::Approve))
                .to_bytes(),
        ),
        challenge,
        decision: WebPairDecision::Approve,
    };
    auth::decide(
        app,
        &serde_json::to_vec(&proof).unwrap(),
        WebPairDecision::Approve,
    )
    .await
    .unwrap();
    let complete = auth::complete(
        app,
        &serde_json::to_vec(
            &json!({"pair_id": created["pair_id"], "poll_secret": created["poll_secret"]}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    (
        profile,
        complete["session_token"].as_str().unwrap().to_owned(),
    )
}

async fn http(
    app: &App,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Vec<u8>>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/v1/{path}"))
        .header("idempotency-key", crypto::random::<16>());
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let mut request = request
        .body(body.map_or_else(Body::empty, Body::from))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:32124".parse::<SocketAddr>().unwrap(),
    ));
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
#[ignore = "requires isolated OMOBA_PORTAL_TEST_DATABASE_URL"]
async fn the_yookassa_routes_exist_only_where_the_market_sells_through_yookassa() {
    let owner = owner_app().await;
    let stand = Stand::start(owner.pool.clone()).await;
    let app = with_shop(owner, &stand, true);
    quiesce(&app, true).await;
    let (profile, token) = login(&app).await;
    let sells = supporter::market_providers().contains(&PaymentProvider::YooKassa);
    let webhook = serde_json::to_vec(
        &json!({"type": "notification", "event": "payment.succeeded",
        "object": {"id": "2f000000-000f-5000-9000-routing"}}),
    )
    .unwrap();
    let (status, body) = http(
        &app,
        "POST",
        "supporter/yookassa/notifications",
        None,
        Some(webhook),
    )
    .await;
    let (checkout_status, checkout) = http(
        &app,
        "POST",
        "me/supporter/yookassa/checkout",
        Some(&token),
        Some(checkout_body(250, "donor@example.ru")),
    )
    .await;
    let (list_status, list) = http(
        &app,
        "GET",
        "me/supporter/yookassa/payments",
        Some(&token),
        None,
    )
    .await;
    if sells {
        // RU: the webhook is public, the checkout and the list need a session.
        assert_eq!(
            (status, body["status"].as_str()),
            (StatusCode::OK, Some("ignored"))
        );
        assert_eq!(checkout_status, StatusCode::OK, "{checkout}");
        assert!(
            checkout["confirmation_url"]
                .as_str()
                .unwrap()
                .starts_with("https://")
        );
        assert_eq!(list_status, StatusCode::OK);
        assert_eq!(list["payments"].as_array().unwrap().len(), 1);
        let (anonymous, _) = http(&app, "GET", "me/supporter/yookassa/payments", None, None).await;
        assert_eq!(anonymous, StatusCode::UNAUTHORIZED);
    } else {
        // Global: no YooKassa route at all.
        for (status, body) in [
            (status, body),
            (checkout_status, checkout),
            (list_status, list),
        ] {
            assert_eq!(
                (status, body["code"].as_str()),
                (StatusCode::NOT_FOUND, Some("not_found"))
            );
        }
    }
    cleanup(&app, &profile).await;
}
