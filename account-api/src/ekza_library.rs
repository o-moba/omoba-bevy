//! Owner-bound Ekza library consent. Usernames are display labels, never identity keys.
//! Credentials stay encrypted on the Account API; the browser sees only consent UI.
use crate::{auth::Session, *};
use reqwest::{Client, Url};
use serde::Deserialize;

#[derive(Clone)]
pub struct EkzaConfig {
    registry: String,
    studio: String,
    prototype: bool,
    client: Client,
}

fn unavailable() -> Error {
    Error(StatusCode::SERVICE_UNAVAILABLE, "ekza_unavailable")
}
fn loopback_origin(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|u| {
        u.scheme() == "http"
            && matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            && u.username().is_empty()
            && u.password().is_none()
            && u.path() == "/"
            && u.query().is_none()
            && u.fragment().is_none()
    })
}
impl EkzaConfig {
    pub fn from_env(portal_origin: &str) -> Result<Self> {
        match std::env::var("OMOBA_EKZA_LOCAL_FIXTURE_ORIGIN") {
            Ok(origin) => Self::local_fixture(&origin, portal_origin),
            Err(std::env::VarError::NotPresent) => {
                Self::new("https://registry.ekza.io", "https://studio.ekza.io", false)
            }
            Err(_) => Err(unavailable()),
        }
    }
    /// Explicit local test seam. Never accepts a remote or credential-bearing origin.
    pub fn local_fixture(origin: &str, portal_origin: &str) -> Result<Self> {
        if !loopback_origin(origin) || !loopback_origin(portal_origin) {
            return Err(unavailable());
        }
        Self::new(
            origin.trim_end_matches('/'),
            origin.trim_end_matches('/'),
            true,
        )
    }
    fn new(registry: &str, studio: &str, prototype: bool) -> Result<Self> {
        Ok(Self {
            registry: registry.into(),
            studio: studio.into(),
            prototype,
            client: Client::builder()
                .timeout(Duration::from_secs(4))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| unavailable())?,
        })
    }
    fn state(&self, state: &str) -> Value {
        let mut v = json!({"state":state});
        if self.prototype {
            v["prototype"] = json!(true);
        }
        v
    }
    async fn request(&self, path: &str, body: Option<Value>, token: Option<&str>) -> Result<Value> {
        let mut req = if let Some(body) = body {
            self.client
                .post(format!("{}{path}", self.registry))
                .json(&body)
        } else {
            self.client.get(format!("{}{path}", self.registry))
        };
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        let mut response = req.send().await.map_err(|_| unavailable())?;
        match response.status().as_u16() {
            401 | 410 => return Err(Error(StatusCode::GONE, "ekza_link_expired")),
            200..=299 => {}
            _ => return Err(unavailable()),
        }
        // A library may contain assets, but an upstream response must remain bounded.
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| unavailable())
    }
}

fn text<'a>(value: &'a Value, key: &str, min: usize, max: usize) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| (min..=max).contains(&v.len()) && !v.chars().any(char::is_control))
        .ok_or_else(unavailable)
}
fn aad(profile: &str, origin: &str, phase: &str) -> String {
    format!("ekza-library:{profile}:{origin}:{phase}")
}
fn sealed(app: &App, profile: &str, phase: &str, raw: &str) -> Result<Vec<u8>> {
    crypto::seal(
        &app.config.secret,
        &aad(profile, &app.ekza.registry, phase),
        raw,
    )
    .map_err(|_| unavailable())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// Serializes link state across browser sessions. Network failures keep existing consent.
pub async fn handle(
    app: &App,
    session: &Session,
    action: &str,
    query: Option<&str>,
    body: &[u8],
) -> Result<Value> {
    if !read::parameters(query)?.is_empty() {
        return Err(invalid());
    }
    if matches!(action, "connect" | "poll") {
        let _: Empty = parse(body)?;
    } else if !body.is_empty() {
        return Err(invalid());
    }
    let ready: bool =
        sqlx::query_scalar("SELECT to_regclass('portal.ekza_library_links') IS NOT NULL")
            .fetch_one(&app.pool)
            .await?;
    if !ready {
        return Ok(app.ekza.state("unavailable"));
    }
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946105))")
        .bind(&session.profile_id)
        .execute(&mut *tx)
        .await?;
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.web_sessions WHERE session_id=$1 AND profile_id=$2 AND authorization_version=2 AND NOT revoked AND expires_at>$3 AND last_seen>$3-86400)")
        .bind(&session.session_id).bind(&session.profile_id).bind(now()).fetch_one(&mut *tx).await?;
    if !active {
        return Err(Error(StatusCode::UNAUTHORIZED, "session_expired"));
    }
    if action == "disconnect" {
        sqlx::query("DELETE FROM portal.ekza_library_links WHERE profile_id=$1")
            .bind(&session.profile_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(app.ekza.state("not_connected"));
    }
    let mut row = sqlx::query("SELECT * FROM portal.ekza_library_links WHERE profile_id=$1")
        .bind(&session.profile_id)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(stored) = &row {
        let phase = row_text(stored, "phase")?;
        if row_text(stored, "registry_origin")? != app.ekza.registry {
            // An operator switching between fixture and live service must explicitly unlink.
            return Ok(app.ekza.state("unavailable"));
        }
        if phase == "pending" && stored.try_get::<i64, _>("pending_until")? <= now() {
            sqlx::query("DELETE FROM portal.ekza_library_links WHERE profile_id=$1")
                .bind(&session.profile_id)
                .execute(&mut *tx)
                .await?;
            row = None;
        }
    }
    let response = if let Some(row) = row {
        let phase = row_text(&row, "phase")?;
        let secret: Vec<u8> = row.try_get("sealed_secret")?;
        let raw = crypto::open(
            &app.config.secret,
            &aad(&session.profile_id, &app.ekza.registry, &phase),
            &secret,
        )
        .map_err(|_| unavailable())?;
        if phase == "connected" {
            match app
                .ekza
                .request("/v1/account/library", None, Some(&raw))
                .await
            {
                Ok(library) => {
                    if library["schema"] != "ekza.account.library.v1"
                        || library["projectId"] != "omoba"
                    {
                        return Err(unavailable());
                    }
                    let username = text(&library["account"], "username", 1, 128)?;
                    let expires = text(&library, "expiresAt", 1, 64)?;
                    sqlx::query("UPDATE portal.ekza_library_links SET username=$2,expires_at=$3,updated_at=clock_timestamp() WHERE profile_id=$1")
                        .bind(&session.profile_id).bind(username).bind(expires).execute(&mut *tx).await?;
                    let mut v = app.ekza.state("connected");
                    v["username"] = json!(username);
                    v["expiresAt"] = json!(expires);
                    v
                }
                Err(Error(StatusCode::GONE, _)) => {
                    sqlx::query("DELETE FROM portal.ekza_library_links WHERE profile_id=$1")
                        .bind(&session.profile_id)
                        .execute(&mut *tx)
                        .await?;
                    app.ekza.state("not_connected")
                }
                Err(_) => app.ekza.state("unavailable"),
            }
        } else if action == "poll" && row.try_get::<i64, _>("next_poll")? <= now() {
            // Respect Registry's polling interval even across multiple tabs.
            sqlx::query("UPDATE portal.ekza_library_links SET next_poll=$2 WHERE profile_id=$1")
                .bind(&session.profile_id)
                .bind(now() + 3)
                .execute(&mut *tx)
                .await?;
            match app
                .ekza
                .request(
                    "/v1/account/device/poll",
                    Some(json!({"deviceCode":raw})),
                    None,
                )
                .await
            {
                Ok(approved) if approved["status"] == "approved" => {
                    if approved["projectId"] != "omoba" || approved["scope"] != "library" {
                        return Err(unavailable());
                    }
                    let token = text(&approved, "accessToken", 32, 256)?;
                    let username = text(&approved["account"], "username", 1, 128)?;
                    let expires = text(&approved, "expiresAt", 1, 64)?;
                    let encrypted = sealed(app, &session.profile_id, "connected", token)?;
                    sqlx::query("UPDATE portal.ekza_library_links SET phase='connected',sealed_secret=$2,username=$3,expires_at=$4,user_code=NULL,verification_url=NULL,updated_at=clock_timestamp() WHERE profile_id=$1")
                        .bind(&session.profile_id).bind(encrypted).bind(username).bind(expires).execute(&mut *tx).await?;
                    let mut v = app.ekza.state("connected");
                    v["username"] = json!(username);
                    v["expiresAt"] = json!(expires);
                    v
                }
                Ok(pending) if pending["status"] == "pending" => pending_dto(&app.ekza, &row)?,
                Err(Error(StatusCode::GONE, _)) => {
                    sqlx::query("DELETE FROM portal.ekza_library_links WHERE profile_id=$1")
                        .bind(&session.profile_id)
                        .execute(&mut *tx)
                        .await?;
                    app.ekza.state("not_connected")
                }
                _ => app.ekza.state("unavailable"),
            }
        } else {
            pending_dto(&app.ekza, &row)?
        }
    } else if action == "connect" {
        let started = app
            .ekza
            .request(
                "/v1/account/device",
                Some(json!({"projectId":"omoba","scope":"library"})),
                None,
            )
            .await?;
        let device = text(&started, "deviceCode", 32, 128)?;
        let code = text(&started, "userCode", 8, 8)?;
        if !code
            .bytes()
            .all(|b| b"ABCDEFGHJKMNPQRSTUVWXYZ23456789".contains(&b))
        {
            return Err(unavailable());
        }
        let url = text(&started, "verificationUrl", 1, 512)?;
        if url != format!("{}/studio?view=connect&code={code}", app.ekza.studio) {
            return Err(unavailable());
        }
        let expires = text(&started, "expiresAt", 1, 64)?;
        let interval = started["interval"]
            .as_i64()
            .filter(|v| *v == 3)
            .ok_or_else(unavailable)?;
        sqlx::query("INSERT INTO portal.ekza_library_links(profile_id,registry_origin,phase,sealed_secret,user_code,verification_url,expires_at,pending_until,next_poll) VALUES($1,$2,'pending',$3,$4,$5,$6,$7,$8)")
            .bind(&session.profile_id).bind(&app.ekza.registry).bind(sealed(app,&session.profile_id,"pending",device)?)
            .bind(code).bind(url).bind(expires).bind(now()+600).bind(now()+interval).execute(&mut *tx).await?;
        let mut v = app.ekza.state("pending");
        v["userCode"] = json!(code);
        v["verificationUrl"] = json!(url);
        v["expiresAt"] = json!(expires);
        v
    } else {
        app.ekza.state("not_connected")
    };
    tx.commit().await?;
    Ok(response)
}
fn pending_dto(config: &EkzaConfig, row: &sqlx::postgres::PgRow) -> Result<Value> {
    let mut v = config.state("pending");
    for (out, column) in [
        ("userCode", "user_code"),
        ("verificationUrl", "verification_url"),
        ("expiresAt", "expires_at"),
    ] {
        v[out] = json!(row_text(row, column)?);
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_is_explicit_loopback_only() {
        assert!(
            EkzaConfig::local_fixture("http://127.0.0.1:40561", "http://localhost:3010").is_ok()
        );
        for bad in [
            "https://attacker.test",
            "http://127.0.0.1:40561/x",
            "http://user@localhost:40561",
            "http://localhost:40561?x=1",
        ] {
            assert!(EkzaConfig::local_fixture(bad, "http://localhost:3010").is_err());
        }
        assert!(EkzaConfig::local_fixture("http://localhost:40561", "https://omoba.io").is_err());
    }
}
