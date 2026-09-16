//! Account HTTP boundary. Match settlement is deliberately not an HTTP operation.
pub mod accounts;
pub mod auth;
pub mod crypto;
pub mod devices;
pub mod read;
pub mod supporter;

use axum::{
    Json, Router,
    body::Bytes,
    extract::Request,
    extract::{ConnectInfo, DefaultBodyLimit, Path, RawQuery, State, rejection::BytesRejection},
    http::{HeaderMap, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use server::career_store::CareerStore;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEVICES_MIGRATION: &str = include_str!("../migrations/002_devices.sql");
pub const SUPPORTER_MIGRATION: &str = include_str!("../migrations/003_supporter.sql");
pub const PORTAL_MIGRATION: &str = include_str!("../migrations/001_portal.sql");
#[derive(Clone)]
pub struct Config {
    pub origin: String,
    pub secret: [u8; 32],
    pub trust_loopback_proxy: bool,
    pub release_file: Option<std::path::PathBuf>,
}
#[derive(Clone)]
pub struct App {
    pub billing: Arc<supporter::BillingConfig>,
    pub pool: PgPool,
    pub career: CareerStore,
    pub config: Arc<Config>,
}
#[derive(Debug)]
pub struct Error(pub StatusCode, pub &'static str);
pub type Result<T> = std::result::Result<T, Error>;
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        reply(
            self.0,
            json!({"code":self.1,"request_id":crypto::random::<12>()}),
        )
    }
}
pub fn invalid() -> Error {
    Error(StatusCode::BAD_REQUEST, "invalid_request")
}
pub fn forbidden() -> Error {
    Error(StatusCode::NOT_FOUND, "not_found")
}
pub fn conflict() -> Error {
    Error(StatusCode::CONFLICT, "state_changed")
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs() as i64
}
pub fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| invalid())
}
pub fn reply(status: StatusCode, value: Value) -> Response {
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(crypto::random::<12>);
    let mut r = (status, Json(value)).into_response();
    r.headers_mut().insert("x-request-id", id.parse().unwrap());
    r.headers_mut()
        .insert("cache-control", "private, no-store".parse().unwrap());
    r.headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    if status == StatusCode::TOO_MANY_REQUESTS {
        r.headers_mut().insert("retry-after", "60".parse().unwrap());
    }
    r
}
pub fn game_error(message: String) -> Error {
    if message.contains("unavailable") {
        return Error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable");
    }
    if message.contains("changed") || message.contains("incoming request") {
        return conflict();
    }
    if message.contains("not found") || message.contains("not available") {
        return forbidden();
    }
    invalid()
}

impl App {
    pub async fn connect(url: &str, config: Config) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(3))
            .after_connect(|c, _| {
                Box::pin(async move {
                    for statement in [
                        "SET search_path TO public",
                        "SET timezone TO 'UTC'",
                        "SET statement_timeout='8s'",
                        "SET lock_timeout='3s'",
                    ] {
                        sqlx::query(statement).execute(&mut *c).await?;
                    }
                    Ok(())
                })
            })
            .connect(url)
            .await?;
        let career = CareerStore::runtime_from_pool(pool.clone())
            .await
            .map_err(game_error)?;
        let versions: Vec<i32> =
            sqlx::query_scalar("SELECT version FROM portal.schema_version ORDER BY version")
                .fetch_all(&pool)
                .await?;
        if versions != [1, 2, 3] {
            return Err(Error(
                StatusCode::SERVICE_UNAVAILABLE,
                "unsupported_portal_schema",
            ));
        }
        Ok(Self {
            billing: Arc::new(supporter::BillingConfig::from_env().map_err(|_| {
                Error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "invalid_billing_configuration",
                )
            })?),
            pool,
            career,
            config: Arc::new(config),
        })
    }
    pub async fn rate(&self, identity: &str, scope: &str, limit: i32) -> Result<()> {
        let key = crypto::mac(&self.config.secret, scope, identity);
        let hits:i32=sqlx::query_scalar("INSERT INTO portal.rate_limits(identity_hash,bucket,hits) VALUES($1,$2,1) ON CONFLICT(identity_hash,bucket) DO UPDATE SET hits=least(portal.rate_limits.hits+1,$3+1) RETURNING hits")
            .bind(key).bind(now()/60).bind(limit).fetch_one(&self.pool).await?;
        if hits > limit {
            return Err(Error(StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
        }
        Ok(())
    }
    pub async fn audit(&self, profile: Option<&str>, event: &str) -> Result<()> {
        sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,$2)")
            .bind(profile)
            .bind(event)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn cleanup(&self) -> Result<()> {
        sqlx::query("DELETE FROM portal.device_enrollments WHERE enrollment_id IN (SELECT e.enrollment_id FROM portal.device_enrollments e WHERE e.expires_at<$1 AND NOT EXISTS(SELECT 1 FROM portal.recovery_codes r WHERE r.enrollment_id=e.enrollment_id) LIMIT 256)").bind(now()-86400).execute(&self.pool).await?;
        sqlx::query("DELETE FROM portal.supporter_nonces WHERE (public_key,nonce) IN (SELECT public_key,nonce FROM portal.supporter_nonces WHERE expires_at<$1 LIMIT 1000)").bind(now()-60).execute(&self.pool).await?;
        // Bounded expiry cleanup; active credentials and immutable game results remain untouched.
        sqlx::query("DELETE FROM portal.rate_limits WHERE (identity_hash,bucket) IN (SELECT identity_hash,bucket FROM portal.rate_limits WHERE bucket<$1 LIMIT 1000)").bind(now()/60-2).execute(&self.pool).await?;
        sqlx::query("DELETE FROM portal.web_pairings WHERE pair_id IN (SELECT pair_id FROM portal.web_pairings WHERE expires_at<$1 LIMIT 256)").bind(now()-60).execute(&self.pool).await?;
        sqlx::query("UPDATE portal.web_pairings SET delivery=NULL WHERE delivery_until<$1 AND delivery IS NOT NULL").bind(now()).execute(&self.pool).await?;
        sqlx::query("DELETE FROM portal.operation_receipts WHERE (session_id,operation_key) IN (SELECT session_id,operation_key FROM portal.operation_receipts WHERE expires_at<$1 LIMIT 1000)").bind(now()).execute(&self.pool).await?;
        sqlx::query("DELETE FROM portal.web_sessions WHERE session_id IN (SELECT s.session_id FROM portal.web_sessions s WHERE s.expires_at<$1 AND NOT EXISTS(SELECT 1 FROM portal.operation_receipts r WHERE r.session_id=s.session_id) LIMIT 1000)").bind(now()-86400).execute(&self.pool).await?;
        sqlx::query("DELETE FROM portal.audit_events WHERE id IN (SELECT id FROM portal.audit_events WHERE created_at<clock_timestamp()-interval '30 days' LIMIT 1000)").execute(&self.pool).await?;
        Ok(())
    }
}
pub async fn migrate(url: &str) -> std::result::Result<(), String> {
    CareerStore::connect(url).await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .map_err(|_| "Cannot connect migration database")?;
    let mut tx = pool.begin().await.map_err(|_| "Cannot start migration")?;
    sqlx::query("SELECT pg_advisory_xact_lock(721946101)")
        .execute(&mut *tx)
        .await
        .map_err(|_| "Cannot lock migration")?;
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('portal.schema_version') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "Cannot inspect migration")?;
    if !exists {
        sqlx::raw_sql(PORTAL_MIGRATION)
            .execute(&mut *tx)
            .await
            .map_err(|_| "Cannot apply portal migration")?;
    }
    for (version, migration) in [(2, DEVICES_MIGRATION), (3, SUPPORTER_MIGRATION)] {
        let applied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM portal.schema_version WHERE version=$1)",
        )
        .bind(version)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Cannot inspect portal migration")?;
        if !applied {
            sqlx::raw_sql(migration)
                .execute(&mut *tx)
                .await
                .map_err(|_| "Cannot apply portal migration")?;
        }
    }
    let versions: Vec<i32> =
        sqlx::query_scalar("SELECT version FROM portal.schema_version ORDER BY version")
            .fetch_all(&mut *tx)
            .await
            .map_err(|_| "Cannot read portal version")?;
    if versions != [1, 2, 3] {
        return Err("Unsupported portal schema version".into());
    }
    tx.commit().await.map_err(|_| "Cannot commit migration")?;
    Ok(())
}
pub fn router(app: App) -> Router {
    Router::new()
        .route(
            "/health/live",
            get(|| async { Json(json!({"status":"alive"})) }),
        )
        .route("/health/ready", get(ready))
        .route("/v1/{*path}", any(handle))
        .layer(DefaultBodyLimit::max(65536))
        .layer(middleware::from_fn_with_state(app.clone(), observe))
        .with_state(app)
}
async fn observe(State(app): State<App>, request: Request, next: Next) -> Response {
    let started = std::time::Instant::now();
    let method = request.method().to_string();
    // Never log paths, query strings, headers or bodies: they may carry identity.
    let mut response = next.run(request).await;
    let id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(crypto::random::<12>);
    response
        .headers_mut()
        .insert("x-request-id", id.parse().unwrap());
    eprintln!(
        "{}",
        json!({"event":"http_request","request_id":id,"method":method,
        "status":response.status().as_u16(),"elapsed_us":started.elapsed().as_micros(),
        "pool_connections":app.pool.size(),"pool_idle":app.pool.num_idle()})
    );
    response
}
async fn ready(State(app): State<App>) -> Result<Json<Value>> {
    sqlx::query("SELECT 1").execute(&app.pool).await?;
    Ok(Json(json!({"status":"ready"})))
}
async fn handle(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    method: Method,
    headers: HeaderMap,
    body: std::result::Result<Bytes, BytesRejection>,
) -> Response {
    let Ok(body) = body else {
        return Error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large").into_response();
    };
    if query.as_ref().is_some_and(|q| q.len() > 2048) {
        return invalid().into_response();
    }
    let result = tokio::time::timeout(
        Duration::from_secs(12),
        dispatch(&app, &path, query.as_deref(), method, &headers, &body, addr),
    )
    .await;
    match result {
        Ok(Ok(value)) => reply(StatusCode::OK, value),
        Ok(Err(e)) => e.into_response(),
        Err(_) => Error(StatusCode::SERVICE_UNAVAILABLE, "request_timeout").into_response(),
    }
}
async fn dispatch(
    app: &App,
    path: &str,
    query: Option<&str>,
    method: Method,
    headers: &HeaderMap,
    body: &[u8],
    addr: SocketAddr,
) -> Result<Value> {
    let ip = if app.config.trust_loopback_proxy && addr.ip().is_loopback() {
        headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<std::net::IpAddr>().ok())
            .ok_or_else(invalid)?
            .to_string()
    } else {
        addr.ip().to_string()
    }; // Only an explicitly configured, overwriting loopback ingress is trusted.
    if method == Method::POST && path.starts_with("auth/devices/") {
        app.rate(
            &ip,
            "device-enrollment",
            if path == "auth/devices/status" {
                120
            } else {
                10
            },
        )
        .await?;
        return match path {
            "auth/devices/create" => devices::create(app, body).await,
            "auth/devices/status" => devices::status(app, body).await,
            "auth/devices/complete" => devices::complete(app, body).await,
            "auth/devices/recover" => devices::recover(app, body).await,
            _ => Err(forbidden()),
        };
    }
    if method == Method::POST && path == "supporter/native" {
        app.rate(&ip, "native-supporter-ip", 60).await?;
        return supporter::native(app, body).await;
    }
    if method == Method::POST && path == "supporter/apple/notifications" {
        app.rate(&ip, "apple-notification-ip", 120).await?;
        return supporter::apple_notification(app, body).await;
    }
    if method == Method::POST && path.starts_with("auth/pairings") {
        app.rate(
            &ip,
            if path == "auth/pairings" {
                "pair-create"
            } else {
                "pair-requests"
            },
            if path == "auth/pairings" { 5 } else { 120 },
        )
        .await?;
        return match path {
            "auth/pairings" => auth::create(app, body).await,
            "auth/pairings/lookup" => {
                app.rate(&ip, "pair-lookup", 20).await?;
                auth::lookup(app, body).await
            }
            "auth/pairings/approve" => {
                auth::decide(app, body, shared::web_account::WebPairDecision::Approve).await
            }
            "auth/pairings/deny" => {
                auth::decide(app, body, shared::web_account::WebPairDecision::Deny).await
            }
            "auth/pairings/status" => auth::status(app, body).await,
            "auth/pairings/complete" => auth::complete(app, body).await,
            "auth/pairings/cancel" => auth::cancel(app, body).await,
            _ => Err(forbidden()),
        };
    }
    if method == Method::GET && path == "releases" {
        return read::releases(app);
    }
    if method == Method::GET && path.starts_with("players/") {
        let session = if headers.contains_key("authorization") {
            Some(auth::session(app, headers).await?)
        } else {
            None
        };
        app.rate(&ip, "public-profile", 120).await?;
        return accounts::player(app, session.as_ref(), &path[8..]).await;
    }
    let session = auth::session(app, headers).await?;
    app.rate(
        &session.profile_id,
        if method == Method::GET {
            "read"
        } else {
            "write"
        },
        if method == Method::GET { 120 } else { 20 },
    )
    .await?;
    match (method.as_str(), path) {
        ("GET", "me/devices") => devices::list(app, &session).await,
        ("POST", "me/devices/lookup") => devices::lookup(app, &session, body).await,
        ("POST", "me/devices/approve") => devices::approve(app, &session, body).await,
        ("POST", "me/recovery-codes") => devices::recovery_codes(app, &session, body).await,
        ("DELETE", p) if p.starts_with("me/devices/") => {
            devices::revoke(app, &session, &p[11..]).await
        }
        ("GET", "me/supporter") => supporter::dashboard(app, &session.profile_id).await,
        ("PATCH", "me/supporter/aura") => supporter::equip(app, &session.profile_id, body).await,
        ("GET", "me/supporter/solana/orders") => {
            supporter::solana::orders(app, &session.profile_id).await
        }
        ("POST", "me/supporter/solana/checkout") => {
            supporter::solana::checkout(app, &session.profile_id, body).await
        }
        ("POST", "me/supporter/solana/confirm") => {
            supporter::solana::confirm(app, &session.profile_id, body).await
        }
        ("POST", "me/supporter/apple/prepare") => {
            supporter::apple_prepare(app, &session.profile_id).await
        }
        ("POST", "me/supporter/apple/verify") => {
            supporter::apple_verify(app, &session.profile_id, body).await
        }
        ("GET", "me") => accounts::me(app, &session).await,
        ("GET", "me/matches") => read::history(app, &session, query).await,
        ("GET", "me/statistics") => {
            app.rate(&session.profile_id, "stats", 10).await?;
            read::statistics(app, &session, query).await
        }
        ("GET", p) if p.starts_with("matches/") => read::detail(app, &session, &p[8..]).await,
        ("GET", "me/friends") => accounts::friends(app, &session).await,
        ("GET", "players") => {
            app.rate(&session.profile_id, "search", 30).await?;
            accounts::search(app, &session, query).await
        }
        ("GET", "me/sessions") => auth::sessions(app, &session).await,
        ("POST", "auth/logout") => auth::logout(app, &session).await,
        ("DELETE", p) if p.starts_with("me/sessions/") => {
            auth::revoke(app, &session, &p[12..]).await
        }
        ("POST", "me/friend-actions") | ("PATCH", "me/profile") | ("PATCH", "me/settings") => {
            accounts::mutate(app, &session, path, headers, body).await
        }
        _ => Err(forbidden()),
    }
}

pub fn profile_dto(profile: &shared::career::ProfileSummary) -> Value {
    let mut value = serde_json::to_value(profile).expect("profile DTO");
    value["progression_xp"] = json!(profile.progression_xp.to_string());
    value["level"] = json!(profile.level().to_string());
    value
}
pub async fn timestamp(pool: &PgPool, seconds: i64) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT to_char(to_timestamp($1::double precision) AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')").bind(seconds as f64).fetch_one(pool).await?)
}
pub fn row_text(row: &sqlx::postgres::PgRow, column: &str) -> Result<String> {
    Ok(row.try_get(column)?)
}
