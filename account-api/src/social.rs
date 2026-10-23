//! Sign in with VK ID (VK, OK and Mail.ru are one VK ID app, OAuth 2.1 + PKCE).
//!
//! The portal runs the browser redirect and forwards the callback; this service
//! exchanges the code itself, so a forged "VK user X" claim from a caller is
//! never trusted. A VK user is stored only as an HMAC of its user id under
//! `OMOBA_SOCIAL_SECRET`. A linked identity signs the browser in like a game
//! pairing does; an unlinked one gets a short browser ticket and is linked to a
//! game profile only by that game's own signed confirmation ([`link`]).
use crate::{auth::Session, *};
use ed25519_dalek::{Signature, VerifyingKey};
use reqwest::{Client, Url};
use serde::Deserialize;
use shared::device_account::SignedSocialLink;

/// Lifetime of an unlinked identity's browser ticket.
pub const TICKET_SECONDS: i64 = 1800;
/// Unlinked identities nobody signed in with for this long are forgotten.
pub const UNLINKED_RETENTION_SECONDS: i64 = 30 * 86400;
const VK_ORIGIN: &str = "https://id.vk.ru";
const PROVIDER: &str = "vk";

#[derive(Clone)]
pub struct SocialConfig {
    vk: Option<VkConfig>,
}
#[derive(Clone)]
struct VkConfig {
    client_id: String,
    api: String,
    secret: [u8; 32],
    client: Client,
}

fn not_configured() -> Error {
    Error(StatusCode::NOT_FOUND, "not_found")
}
fn provider_unavailable() -> Error {
    Error(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable")
}
fn provider_rejected() -> Error {
    Error(StatusCode::BAD_GATEWAY, "provider_rejected")
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

impl SocialConfig {
    pub fn disabled() -> Self {
        Self { vk: None }
    }
    /// `OMOBA_VKID_CLIENT_ID` (the VK ID app id) and `OMOBA_SOCIAL_SECRET`
    /// (64 lowercase hex, never rotated: it keys every stored identity) enable
    /// VK ID; neither set disables it. Only a `vk_sign_in` build accepts them.
    pub fn from_env() -> std::result::Result<Self, &'static str> {
        let var = |key| std::env::var_os(key).map(|v| v.to_string_lossy().into_owned());
        let fixture = var("OMOBA_VKID_LOCAL_FIXTURE_ORIGIN");
        let (client_id, secret) = match (var("OMOBA_VKID_CLIENT_ID"), var("OMOBA_SOCIAL_SECRET")) {
            (None, None) if fixture.is_none() => return Ok(Self::disabled()),
            (Some(client_id), Some(secret)) => (client_id, secret),
            _ => return Err("Set both OMOBA_VKID_CLIENT_ID and OMOBA_SOCIAL_SECRET"),
        };
        if !shared::market::FEATURES.vk_sign_in {
            return Err("VK ID is configured, but this build has no vk_sign_in feature");
        }
        let secret = crypto::decode::<32>(&secret)
            .ok_or("OMOBA_SOCIAL_SECRET must be 32 random bytes in lowercase hex")?;
        match fixture {
            Some(origin) => Self::vk_fixture(&client_id, &origin, secret),
            None => Self::vk(&client_id, VK_ORIGIN, secret),
        }
    }
    /// Explicit local test seam: a loopback stand-in for id.vk.ru.
    pub fn vk_fixture(
        client_id: &str,
        origin: &str,
        secret: [u8; 32],
    ) -> std::result::Result<Self, &'static str> {
        if !loopback_origin(origin) {
            return Err("OMOBA_VKID_LOCAL_FIXTURE_ORIGIN must be a loopback http origin");
        }
        Self::vk(client_id, origin.trim_end_matches('/'), secret)
    }
    fn vk(client_id: &str, api: &str, secret: [u8; 32]) -> std::result::Result<Self, &'static str> {
        if client_id.is_empty()
            || client_id.len() > 16
            || !client_id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("OMOBA_VKID_CLIENT_ID must be the numeric VK ID app id");
        }
        Ok(Self {
            vk: Some(VkConfig {
                client_id: client_id.into(),
                api: api.into(),
                secret,
                client: Client::builder()
                    .connect_timeout(Duration::from_secs(3))
                    .timeout(Duration::from_secs(8))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|_| "Cannot initialize the VK ID transport")?,
            }),
        })
    }
    pub fn vk_enabled(&self) -> bool {
        self.vk.is_some()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VkSignIn {
    method: String,
    code: String,
    code_verifier: String,
    device_id: String,
    redirect_uri: String,
    state: String,
}
#[derive(Deserialize)]
struct VkToken {
    access_token: Option<String>,
}
#[derive(Deserialize)]
struct VkUserInfo {
    user: Option<VkUser>,
}
#[derive(Deserialize)]
struct VkUser {
    user_id: Option<Value>,
    first_name: Option<String>,
    last_name: Option<String>,
}

fn token_like(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|b| b.is_ascii_graphic())
}
fn method_label(method: &str) -> Option<&'static str> {
    match method {
        "vkid" => Some("VK ID"),
        "ok_ru" => Some("OK"),
        "mail_ru" => Some("Mail.ru"),
        _ => None,
    }
}
/// "Иван П." from the VK profile: a first name and an initial, at most 40 characters.
pub fn display_name(first: Option<&str>, last: Option<&str>) -> String {
    let clean = |s: Option<&str>| {
        s.unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_owned()
    };
    let first: String = clean(first).chars().take(32).collect();
    let initial = clean(last).chars().next();
    let name = match (first.is_empty(), initial) {
        (false, Some(i)) => format!("{first} {i}."),
        (false, None) => first,
        (true, Some(i)) => format!("{i}."),
        (true, None) => "VK".into(),
    };
    name.chars().take(40).collect()
}
fn vk_user_id(value: &Value) -> Option<String> {
    let id = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_u64()?.to_string(),
        _ => return None,
    };
    (!id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()) && id != "0")
        .then_some(id)
}

impl VkConfig {
    async fn post(&self, path: &str, form: &[(&str, &str)]) -> Result<(StatusCode, Vec<u8>)> {
        let mut response = self
            .client
            .post(format!("{}{path}", self.api))
            .form(form)
            .send()
            .await
            .map_err(|_| provider_unavailable())?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| provider_unavailable())? {
            if bytes.len() + chunk.len() > 65536 {
                return Err(provider_unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((
            StatusCode::from_u16(status.as_u16()).map_err(|_| provider_unavailable())?,
            bytes,
        ))
    }
    /// Code → access token → VK user. Errors carry only safe codes.
    async fn verify(&self, request: &VkSignIn) -> Result<(String, String)> {
        let (status, body) = self
            .post(
                "/oauth2/auth",
                &[
                    ("grant_type", "authorization_code"),
                    ("code", &request.code),
                    ("code_verifier", &request.code_verifier),
                    ("redirect_uri", &request.redirect_uri),
                    ("client_id", &self.client_id),
                    ("device_id", &request.device_id),
                    ("state", &request.state),
                ],
            )
            .await?;
        if status.is_server_error() {
            return Err(provider_unavailable());
        }
        let token = serde_json::from_slice::<VkToken>(&body)
            .ok()
            .and_then(|t| t.access_token)
            .filter(|t| status.is_success() && token_like(t, 4096))
            .ok_or_else(provider_rejected)?;
        let (status, body) = self
            .post(
                "/oauth2/user_info",
                &[("access_token", &token), ("client_id", &self.client_id)],
            )
            .await?;
        if status.is_server_error() {
            return Err(provider_unavailable());
        }
        let user = serde_json::from_slice::<VkUserInfo>(&body)
            .ok()
            .filter(|_| status.is_success())
            .and_then(|u| u.user)
            .ok_or_else(provider_rejected)?;
        let id = user
            .user_id
            .as_ref()
            .and_then(vk_user_id)
            .ok_or_else(provider_rejected)?;
        Ok((
            crypto::mac(&self.secret, "vk-subject", &id),
            display_name(user.first_name.as_deref(), user.last_name.as_deref()),
        ))
    }
}

/// `POST /v1/auth/social/vk` (portal BFF only): verify a VK ID callback and sign in.
pub async fn vk_sign_in(app: &App, body: &[u8]) -> Result<Value> {
    let vk = app.social.vk.as_ref().ok_or_else(not_configured)?;
    let p: VkSignIn = parse(body)?;
    let label = method_label(&p.method).ok_or_else(invalid)?;
    if !token_like(&p.code, 1024)
        || !(43..=128).contains(&p.code_verifier.len())
        || !p
            .code_verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        || !token_like(&p.device_id, 512)
        || !token_like(&p.state, 512)
        // The code is only accepted for this portal's own callback.
        || p.redirect_uri != format!("{}/api/auth/vk/callback", app.config.origin)
    {
        return Err(invalid());
    }
    let (subject, name) = vk.verify(&p).await?;
    let current = now();
    let mut tx = app.pool.begin().await?;
    let profile: Option<String> = sqlx::query_scalar(
        "INSERT INTO portal.social_identities(provider,subject_hash,method,created_at,last_login_at) VALUES($1,$2,$3,$4,$4)
         ON CONFLICT(provider,subject_hash) DO UPDATE SET last_login_at=$4,method=$3 RETURNING profile_id",
    )
    .bind(PROVIDER)
    .bind(&subject)
    .bind(&p.method)
    .bind(current)
    .fetch_one(&mut *tx)
    .await?;
    let Some(profile) = profile else {
        let ticket = crypto::random::<32>();
        let expiry = current + TICKET_SECONDS;
        sqlx::query("INSERT INTO portal.social_tickets(ticket_hash,provider,subject_hash,display_name,expires_at) VALUES($1,$2,$3,$4,$5)")
            .bind(crypto::mac(&app.config.secret, "social-ticket", &ticket))
            .bind(PROVIDER).bind(&subject).bind(&name).bind(expiry)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(
            json!({"state":"unlinked","social_ticket":ticket,"display_name":name,
            "expires_at":timestamp(&app.pool,expiry).await?}),
        );
    };
    // Same per-account session cap and lock as a game-approved pairing.
    let nickname: String =
        sqlx::query_scalar("SELECT nickname FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
            .bind(&profile)
            .fetch_one(&mut *tx)
            .await?;
    let active:i64=sqlx::query_scalar("SELECT count(*) FROM portal.web_sessions WHERE profile_id=$1 AND NOT revoked AND expires_at>$2 AND last_seen>$2-86400")
        .bind(&profile).bind(current).fetch_one(&mut *tx).await?;
    if active >= 20 {
        return Err(Error(StatusCode::CONFLICT, "session_limit"));
    }
    let sid = crypto::random::<32>();
    let token = crypto::random::<32>();
    sqlx::query("INSERT INTO portal.web_sessions(session_id,profile_id,token_hash,created_at,last_seen,expires_at,authorization_version,label) VALUES($1,$2,$3,$4,$4,$5,2,$6)")
        .bind(&sid).bind(&profile).bind(crypto::mac(&app.config.secret,"session-token",&token))
        .bind(current).bind(current+7*86400).bind(label).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,'social_sign_in')")
        .bind(&profile)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(
        json!({"state":"signed_in","session_token":token,"profile":{"profile_id":profile,"nickname":nickname}}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ticket {
    social_ticket: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attach {
    code: String,
    social_ticket: String,
}
fn ticket_hash(app: &App, ticket: &str) -> Result<String> {
    crypto::decode::<32>(ticket).ok_or_else(invalid)?;
    Ok(crypto::mac(&app.config.secret, "social-ticket", ticket))
}

/// `POST /v1/auth/social/ticket` (portal BFF only): what an unlinked ticket names.
pub async fn ticket(app: &App, body: &[u8]) -> Result<Value> {
    app.social.vk.as_ref().ok_or_else(not_configured)?;
    let p: Ticket = parse(body)?;
    let row = sqlx::query("SELECT t.display_name,t.expires_at,i.profile_id IS NOT NULL AS linked FROM portal.social_tickets t JOIN portal.social_identities i USING(provider,subject_hash) WHERE t.ticket_hash=$1 AND t.expires_at>$2")
        .bind(ticket_hash(app, &p.social_ticket)?).bind(now()).fetch_optional(&app.pool).await?.ok_or_else(forbidden)?;
    Ok(json!({"display_name":row_text(&row,"display_name")?,
        "expires_at":timestamp(&app.pool,row.try_get("expires_at")?).await?,
        "linked":row.try_get::<bool,_>("linked")?}))
}

/// `POST /v1/auth/social/attach` (portal BFF only): offer an unlinked sign-in to
/// the game enrollment whose code the game opened the browser with.
pub async fn attach(app: &App, body: &[u8]) -> Result<Value> {
    app.social.vk.as_ref().ok_or_else(not_configured)?;
    let p: Attach = parse(body)?;
    let code = p.code.trim().to_ascii_uppercase();
    if code.len() != 8 || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(invalid());
    }
    let hash = ticket_hash(app, &p.social_ticket)?;
    let current = now();
    let mut tx = app.pool.begin().await?;
    let identity = sqlx::query("SELECT t.provider,t.subject_hash,t.display_name,i.profile_id FROM portal.social_tickets t JOIN portal.social_identities i USING(provider,subject_hash) WHERE t.ticket_hash=$1 AND t.expires_at>$2 FOR UPDATE OF i")
        .bind(&hash).bind(current).fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    if identity
        .try_get::<Option<String>, _>("profile_id")?
        .is_some()
    {
        // Linked meanwhile: the browser signs in again and approves as the account.
        return Err(Error(StatusCode::CONFLICT, "already_linked"));
    }
    let enrollment = sqlx::query("SELECT enrollment_id,label,expires_at FROM portal.device_enrollments WHERE code_hash=$1 AND expires_at>$2 AND state='pending' FOR UPDATE")
        .bind(crypto::mac(&app.config.secret,"device-code",&code)).bind(current)
        .fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    let enrollment_id = row_text(&enrollment, "enrollment_id")?;
    let subject = row_text(&identity, "subject_hash")?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT subject_hash FROM portal.social_links WHERE enrollment_id=$1")
            .bind(&enrollment_id)
            .fetch_optional(&mut *tx)
            .await?;
    match existing {
        Some(other) if other != subject => return Err(conflict()),
        Some(_) => {}
        None => {
            sqlx::query("INSERT INTO portal.social_links(enrollment_id,provider,subject_hash,display_name,created_at) VALUES($1,$2,$3,$4,$5)")
                .bind(&enrollment_id).bind(PROVIDER).bind(&subject)
                .bind(row_text(&identity,"display_name")?).bind(current)
                .execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(
        json!({"status":"social_pending","label":row_text(&enrollment,"label")?,
        "expires_at":timestamp(&app.pool,enrollment.try_get("expires_at")?).await?}),
    )
}

/// `POST /v1/auth/social/link` (native): the game links the sign-in waiting on
/// its enrollment to the profile of its active key, after the player confirmed.
pub async fn link(app: &App, body: &[u8]) -> Result<Value> {
    app.social.vk.as_ref().ok_or_else(not_configured)?;
    let proof: SignedSocialLink = parse(body)?;
    let l = &proof.link;
    l.validate(&app.config.origin, now() as u64)
        .map_err(|_| invalid())?;
    let key = VerifyingKey::from_bytes(&crypto::decode::<32>(&l.public_key).ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    let signature =
        Signature::from_bytes(&crypto::decode::<64>(&proof.signature).ok_or_else(invalid)?);
    key.verify_strict(&l.signing_bytes(), &signature)
        .map_err(|_| forbidden())?;
    app.rate(&l.public_key, "social-link-key", 10).await?;
    let current = now();
    let mut tx = app.pool.begin().await?;
    let enrollment = sqlx::query("SELECT public_key,expires_at,state FROM portal.device_enrollments WHERE enrollment_id=$1 FOR UPDATE")
        .bind(&l.enrollment_id).fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    let social = sqlx::query("SELECT provider,subject_hash,state FROM portal.social_links WHERE enrollment_id=$1 FOR UPDATE")
        .bind(&l.enrollment_id).fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    let profile: String = sqlx::query_scalar(
        "SELECT profile_id FROM career_keys WHERE public_key=$1 AND revoked_at IS NULL",
    )
    .bind(&l.public_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(forbidden)?;
    let nickname: String =
        sqlx::query_scalar("SELECT nickname FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
            .bind(&profile)
            .fetch_one(&mut *tx)
            .await?;
    let subject = row_text(&social, "subject_hash")?;
    let linked_to: Option<String> = sqlx::query_scalar("SELECT profile_id FROM portal.social_identities WHERE provider=$1 AND subject_hash=$2 FOR UPDATE")
        .bind(PROVIDER).bind(&subject).fetch_optional(&mut *tx).await?.ok_or_else(forbidden)?;
    if row_text(&enrollment, "public_key")? != l.enrollment_key {
        return Err(forbidden());
    }
    if row_text(&social, "state")? == "linked" {
        // A lost response: the same game confirming again sees the same outcome.
        if linked_to.as_deref() == Some(profile.as_str()) {
            return Ok(json!({"state":"linked","profile_id":profile,"nickname":nickname}));
        }
        return Err(conflict());
    }
    if row_text(&enrollment, "state")? != "pending"
        || enrollment.try_get::<i64, _>("expires_at")?.to_string() != l.expires_at
        || enrollment.try_get::<i64, _>("expires_at")? <= current
    {
        return Err(conflict());
    }
    if linked_to.is_some() {
        return Err(Error(StatusCode::CONFLICT, "already_linked"));
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portal.social_identities WHERE profile_id=$1 AND provider=$2)",
    )
    .bind(&profile)
    .bind(PROVIDER)
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Err(Error(StatusCode::CONFLICT, "profile_already_linked"));
    }
    sqlx::query("UPDATE portal.social_identities SET profile_id=$3,linked_at=$4 WHERE provider=$1 AND subject_hash=$2")
        .bind(PROVIDER).bind(&subject).bind(&profile).bind(current).execute(&mut *tx).await?;
    sqlx::query("UPDATE portal.social_links SET state='linked' WHERE enrollment_id=$1")
        .bind(&l.enrollment_id)
        .execute(&mut *tx)
        .await?;
    // The enrollment code has done its job: it can no longer approve a device.
    sqlx::query("UPDATE portal.device_enrollments SET expires_at=$2 WHERE enrollment_id=$1")
        .bind(&l.enrollment_id)
        .bind(current)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM portal.social_tickets WHERE provider=$1 AND subject_hash=$2")
        .bind(PROVIDER)
        .bind(&subject)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,'social_linked')")
        .bind(&profile)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({"state":"linked","profile_id":profile,"nickname":nickname}))
}

/// The pending sign-in a native status poll reports (`state: social_pending`).
pub async fn pending(app: &App, enrollment_id: &str) -> Result<Option<Value>> {
    let row = sqlx::query(
        "SELECT provider,display_name FROM portal.social_links WHERE enrollment_id=$1 AND state='pending'",
    )
    .bind(enrollment_id)
    .fetch_optional(&app.pool)
    .await?;
    row.map(|row| {
        Ok(json!({"provider":row_text(&row,"provider")?,"display_name":row_text(&row,"display_name")?}))
    })
    .transpose()
}

/// The sign-in methods linked to the signed-in account (`GET /v1/me`).
pub async fn linked(app: &App, profile: &str) -> Result<Value> {
    let rows = sqlx::query("SELECT provider,method,linked_at FROM portal.social_identities WHERE profile_id=$1 ORDER BY provider")
        .bind(profile).fetch_all(&app.pool).await?;
    let mut items = Vec::new();
    for row in rows {
        items.push(
            json!({"provider":row_text(&row,"provider")?,"method":row_text(&row,"method")?,
            "linked_at":timestamp(&app.pool,row.try_get::<i64,_>("linked_at")?).await?}),
        );
    }
    Ok(Value::Array(items))
}

/// `DELETE /v1/me/social/vk`: forget the account's VK sign-in (fresh session only).
pub async fn unlink(app: &App, s: &Session) -> Result<Value> {
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(&s.profile_id)
        .fetch_one(&mut *tx)
        .await?;
    let fresh:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.web_sessions WHERE session_id=$1 AND profile_id=$2 AND authorization_version=2 AND NOT revoked AND expires_at>$3 AND created_at>=$3-600)")
        .bind(&s.session_id).bind(&s.profile_id).bind(now()).fetch_one(&mut *tx).await?;
    if !fresh {
        return Err(Error(StatusCode::FORBIDDEN, "fresh_confirmation_required"));
    }
    let removed =
        sqlx::query("DELETE FROM portal.social_identities WHERE profile_id=$1 AND provider=$2")
            .bind(&s.profile_id)
            .bind(PROVIDER)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if removed != 1 {
        return Err(forbidden());
    }
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,'social_unlinked')")
        .bind(&s.profile_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({"status":"unlinked"}))
}

/// Bounded expiry cleanup (called from [`App::cleanup`]).
pub async fn cleanup(app: &App) -> Result<()> {
    sqlx::query("DELETE FROM portal.social_tickets WHERE ticket_hash IN (SELECT ticket_hash FROM portal.social_tickets WHERE expires_at<$1 LIMIT 1000)")
        .bind(now()).execute(&app.pool).await?;
    sqlx::query("DELETE FROM portal.social_identities WHERE (provider,subject_hash) IN (SELECT i.provider,i.subject_hash FROM portal.social_identities i WHERE i.profile_id IS NULL AND i.last_login_at<$1 AND NOT EXISTS(SELECT 1 FROM portal.social_links l WHERE l.provider=i.provider AND l.subject_hash=i.subject_hash) LIMIT 256)")
        .bind(now() - UNLINKED_RETENTION_SECONDS).execute(&app.pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_keep_only_a_first_name_and_an_initial() {
        assert_eq!(display_name(Some("Иван"), Some("Петров")), "Иван П.");
        assert_eq!(display_name(Some(" Ann "), None), "Ann");
        assert_eq!(display_name(None, Some("Smith")), "S.");
        assert_eq!(display_name(Some("\u{7}"), Some("")), "VK");
        assert!(
            display_name(Some(&"x".repeat(100)), Some("Y"))
                .chars()
                .count()
                <= 40
        );
    }

    #[test]
    fn vk_user_ids_are_positive_decimal_numbers() {
        assert_eq!(vk_user_id(&json!("12345")), Some("12345".into()));
        assert_eq!(vk_user_id(&json!(12345)), Some("12345".into()));
        for bad in [
            json!(""),
            json!("0"),
            json!("12a"),
            json!(-1),
            json!(null),
            json!("1".repeat(21)),
        ] {
            assert_eq!(vk_user_id(&bad), None, "{bad}");
        }
    }

    #[test]
    fn configuration_refuses_remote_fixtures_and_bad_app_ids() {
        assert!(SocialConfig::vk_fixture("123", "http://127.0.0.1:9999", [1; 32]).is_ok());
        assert!(SocialConfig::vk_fixture("123", "https://id.vk.ru", [1; 32]).is_err());
        assert!(SocialConfig::vk_fixture("123", "http://evil.example", [1; 32]).is_err());
        assert!(SocialConfig::vk("app", VK_ORIGIN, [1; 32]).is_err());
        assert!(SocialConfig::vk("", VK_ORIGIN, [1; 32]).is_err());
        assert!(!SocialConfig::disabled().vk_enabled());
    }
}
