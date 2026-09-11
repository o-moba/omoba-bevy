//! Native transport and staged importer for the shared Ekza passport contract.
//! Secrets stay in memory. Game packets contain only one-use scoped tickets.

use ekza_bevy_sdk::passport::{
    ConsumedTicket, OMOBA_PROJECT, ProjectSupport, ProtectedAvatar, PurchasedAvatar,
    PurchasedLibrary, validate_avatar_id, validate_omoba_support,
};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path, time::Duration};

pub const PASSPORT_URL_ENV: &str = "OMOBA_PASSPORT_URL";
const MAX_JSON_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct PassportApi {
    base: Url,
    client: Client,
}

// No Debug implementation: accessToken/deviceCode must never leak through logs.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePairing {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at: String,
    pub interval: u64,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PairingPoll {
    Pending,
    Approved {
        #[serde(rename = "accessToken")]
        access_token: String,
        #[serde(rename = "expiresAt")]
        expires_at: String,
        wallet: String,
    },
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarTicket {
    pub ticket: String,
    pub expires_at: String,
    pub avatar: PurchasedAvatar,
    pub support: ProjectSupport,
}

#[derive(Clone)]
pub struct NativeSession {
    pub api: PassportApi,
    token: String,
    pub library: PurchasedLibrary,
}

fn safe_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "Invalid passport URL".to_string())?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use HTTPS, or explicit localhost development, without URL credentials".into());
    }
    Ok(url)
}

impl PassportApi {
    pub fn new(base: &str) -> Result<Self, String> {
        let base = safe_url(base)?;
        if !base.path().trim_end_matches('/').ends_with("/api/passport") {
            return Err("Passport URL must end in /api/passport".into());
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "Unable to initialize passport HTTPS transport".to_string())?;
        Ok(Self { base, client })
    }

    pub fn from_env() -> Result<Self, String> {
        let base = std::env::var(PASSPORT_URL_ENV).map_err(|_| {
            format!("Set {PASSPORT_URL_ENV} to the trusted storefront passport API")
        })?;
        Self::new(&base)
    }

    fn request<T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Result<T, String> {
        let endpoint = format!("{}/{}", self.base.as_str().trim_end_matches('/'), path);
        let mut request = if let Some(body) = body {
            self.client.post(endpoint).json(&body)
        } else {
            self.client.get(endpoint)
        };
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .map_err(|_| "Passport service could not be reached. Retry.".to_string())?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(MAX_JSON_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Passport response was interrupted".to_string())?;
        if bytes.len() as u64 > MAX_JSON_BYTES {
            return Err("Passport response is too large".into());
        }
        if !status.is_success() {
            // Remote error text may contain sensitive information. Surface status
            // with safe action-oriented copy rather than printing arbitrary JSON.
            return Err(match status.as_u16() {
                401 | 403 => {
                    "Wallet approval expired or this avatar is not owned/supported. Pair again."
                        .into()
                }
                404 | 410 => "Pairing, avatar, or ticket expired or is no longer available.".into(),
                409 => "Pairing or ticket was already used. Start a fresh approval.".into(),
                429 => "Too many passport requests. Wait and retry.".into(),
                _ => format!("Passport request failed (HTTP {}). Retry.", status.as_u16()),
            });
        }
        serde_json::from_slice(&bytes).map_err(|_| "Invalid passport response".into())
    }

    pub fn pair(&self) -> Result<DevicePairing, String> {
        let pairing: DevicePairing =
            self.request("device", None, Some(json!({"projectId": OMOBA_PROJECT})))?;
        let verification = Url::parse(&pairing.verification_url)
            .map_err(|_| "Invalid wallet verification link".to_string())?;
        if verification.origin() != self.base.origin()
            || verification.username() != ""
            || verification.password().is_some()
            || pairing.user_code.is_empty()
            || pairing.device_code.len() < 16
            || pairing.verification_url.contains(&pairing.device_code)
        {
            return Err("Invalid wallet verification link or pairing code".into());
        }
        Ok(pairing)
    }

    pub fn poll(&self, device_code: &str) -> Result<PairingPoll, String> {
        self.request(
            "device/poll",
            None,
            Some(json!({"deviceCode": device_code})),
        )
    }

    pub fn session(&self, token: String, expected_wallet: &str) -> Result<NativeSession, String> {
        let library: PurchasedLibrary = self.request("library", Some(&token), None)?;
        if library.schema != "ekza.passport.library.v1"
            || library.network != "solana-devnet"
            || library.wallet != expected_wallet
        {
            return Err("Passport library belongs to a different wallet or network".into());
        }
        for avatar in &library.items {
            validate_avatar_id(&avatar.avatar_id)?;
            if !ekza_bevy_sdk::passport::valid_base58_key(&avatar.mint) {
                return Err("Invalid owned NFT mint".into());
            }
        }
        Ok(NativeSession {
            api: self.clone(),
            token,
            library,
        })
    }

    pub fn consume(&self, ticket: &str, session_id: &str) -> Result<ConsumedTicket, String> {
        if ticket.len() < 16 || ticket.len() > 4096 || !valid_session_id(session_id) {
            return Err("Missing or malformed avatar admission proof".into());
        }
        self.request(
            "ticket/consume",
            None,
            Some(json!({
                "ticket": ticket, "projectId": OMOBA_PROJECT, "sessionId": session_id,
            })),
        )
    }

    pub fn download(&self, protected: &ProtectedAvatar) -> Result<Vec<u8>, String> {
        protected.validate()?;
        let url = safe_url(&protected.support.rendition.url)?;
        // This request deliberately carries no bearer token, cookies or redirects.
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|_| "Avatar rendition download failed".to_string())?;
        if !response.status().is_success() {
            return Err("Avatar rendition is unavailable".into());
        }
        let limit = protected.support.rendition.size_bytes;
        if response.content_length().is_some_and(|size| size > limit) {
            return Err("Avatar exceeds its approved size".into());
        }
        let mut bytes = Vec::new();
        response
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Avatar download was interrupted".to_string())?;
        verify_bytes(protected, &bytes)?;
        Ok(bytes)
    }
}

pub fn valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= 64
        && session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl NativeSession {
    pub fn owns(&self, protected: &ProtectedAvatar) -> Option<&PurchasedAvatar> {
        self.library.items.iter().find(|avatar| {
            avatar.avatar_id == protected.avatar_id
                && avatar
                    .support
                    .iter()
                    .any(|support| support == &protected.support)
        })
    }

    pub fn ticket(
        &self,
        protected: &ProtectedAvatar,
        session_id: &str,
    ) -> Result<AvatarTicket, String> {
        protected.validate()?;
        let avatar = self
            .owns(protected)
            .ok_or("This wallet does not own this supported avatar")?;
        if !valid_session_id(session_id) {
            return Err("Invalid game session".into());
        }
        let response: AvatarTicket = self.api.request(
            "ticket",
            Some(&self.token),
            Some(json!({
                "projectId": OMOBA_PROJECT, "avatarId": avatar.avatar_id,
                "mint": avatar.mint, "sessionId": session_id,
            })),
        )?;
        if response.avatar.avatar_id != protected.avatar_id
            || response.avatar.mint != avatar.mint
            || response.support != protected.support
            || response.ticket.len() < 16
            || response.ticket.len() > 4096
        {
            return Err("Passport ticket does not match the selected avatar".into());
        }
        Ok(response)
    }
}

pub fn verify_bytes(protected: &ProtectedAvatar, bytes: &[u8]) -> Result<(), String> {
    protected
        .validate_bytes(bytes, &format!("{:x}", Sha256::digest(bytes)))
        .map_err(str::to_owned)?;
    validate_humanoid_profile(bytes)
}

/// Omoba v1 expects a self-contained skinned model with the complete cosmetic
/// action set. Approval is still supplied by the operator, never this check.
pub fn validate_humanoid_profile(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 20 {
        return Err("Missing glTF JSON chunk".into());
    }
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    if &bytes[16..20] != b"JSON" || length > bytes.len() - 20 {
        return Err("Invalid glTF JSON chunk".into());
    }
    let doc: Value =
        serde_json::from_slice(&bytes[20..20 + length]).map_err(|_| "Invalid glTF JSON")?;
    let nodes = doc["nodes"]
        .as_array()
        .ok_or("Avatar has no skeleton nodes")?;
    let skins = doc["skins"]
        .as_array()
        .filter(|skins| !skins.is_empty())
        .ok_or("Avatar has no skin")?;
    if !skins.iter().all(|skin| {
        skin["joints"].as_array().is_some_and(|joints| {
            !joints.is_empty()
                && joints.iter().all(|joint| {
                    joint
                        .as_u64()
                        .is_some_and(|index| index < nodes.len() as u64)
                })
        })
    }) {
        return Err("Avatar skeleton has invalid joint references".into());
    }
    for field in ["buffers", "images"] {
        if doc[field]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry.get("uri").is_some()))
        {
            return Err("Avatar must embed every model and texture resource".into());
        }
    }
    let animations = doc["animations"]
        .as_array()
        .ok_or("Avatar needs embedded idle/walk/attack/cast/death clips")?;
    for action in ["idle", "walk", "attack", "cast", "death"] {
        let animation = animations
            .iter()
            .find(|clip| clip["name"].as_str() == Some(action))
            .ok_or_else(|| format!("Avatar is missing its {action} clip"))?;
        if !animation["channels"].as_array().is_some_and(|channels| {
            !channels.is_empty()
                && channels.iter().all(|channel| {
                    channel["target"]["node"]
                        .as_u64()
                        .is_some_and(|index| index < nodes.len() as u64)
                })
        }) {
            return Err(format!("Avatar {action} clip has invalid skeleton targets"));
        }
    }
    Ok(())
}

pub fn verify_local(protected: &ProtectedAvatar, path: &Path) -> Result<(), String> {
    protected.validate()?;
    let file = fs::File::open(path)
        .map_err(|_| "Approved avatar is not installed. Run passport-import first.".to_string())?;
    if file
        .metadata()
        .map_err(|_| "Avatar metadata unavailable")?
        .len()
        != protected.support.rendition.size_bytes
    {
        return Err("Installed avatar size differs from the approved rendition".into());
    }
    let mut bytes = Vec::new();
    file.take(protected.support.rendition.size_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Installed avatar could not be read".to_string())?;
    verify_bytes(protected, &bytes)
}

pub fn protected_slug(protected: &ProtectedAvatar) -> String {
    // Include canonical identity and full revision hash; two templates using the
    // same geometry must not collapse into one ownership entry.
    let key = format!(
        "{}\n{}",
        protected.avatar_id, protected.support.rendition.sha256
    );
    format!("ekza-{:x}", Sha256::digest(key.as_bytes()))
}

fn is_desktop_omoba_selector(support: &ProjectSupport) -> bool {
    support.project_id == OMOBA_PROJECT
        && support.platform == "desktop"
        && support.profile == "humanoid-glb-v1"
        && support.status == "approved"
}

/// Stage immutable models and a sidecar manifest; never rewrite the shipped
/// manifest. Publish the same directory/manifest to all game consumers, then
/// restart both client and server with OMOBA_AVATAR_MANIFEST pointing at it.
pub fn import_owned(
    session: &NativeSession,
    asset_root: &Path,
    manifest: &Path,
) -> Result<usize, String> {
    let source = fs::read(asset_root.join("avatars/manifest.json"))
        .map_err(|_| "Shipped avatar manifest is missing")?;
    let mut document: Value =
        serde_json::from_slice(&source).map_err(|_| "Shipped avatar manifest is invalid")?;
    let roster = document
        .get_mut("avatars")
        .and_then(Value::as_array_mut)
        .ok_or("Invalid avatar roster")?;
    // Preserve only the free base; every protected entry is generated from
    // today's approved library, never inherited from unknown local metadata.
    roster.retain(|entry| entry.get("passport").is_none_or(Value::is_null));
    let mut seen = std::collections::HashSet::new();
    let mut added = 0;
    for avatar in &session.library.items {
        for support in avatar
            .support
            .iter()
            .filter(|support| is_desktop_omoba_selector(support))
        {
            validate_omoba_support(support)?;
            let protected = ProtectedAvatar {
                avatar_id: avatar.avatar_id.clone(),
                support: support.clone(),
            };
            let slug = protected_slug(&protected);
            if !seen.insert(slug.clone()) {
                continue;
            }
            let bytes = session.api.download(&protected)?;
            let destination = asset_root.join("avatars").join(format!("{slug}.glb"));
            fs::create_dir_all(destination.parent().unwrap())
                .map_err(|_| "Cannot create avatar asset directory")?;
            atomic_write(&destination, &bytes)?;
            roster.push(json!({
                "slug": slug, "display_name": avatar.name, "collection": "Ekza purchased avatars",
                "license": "See creator terms", "source_url": support.rendition.url,
                "author": null, "thumbnail": null, "passport": protected,
            }));
            added += 1;
        }
    }
    atomic_write(
        manifest,
        &serde_json::to_vec_pretty(&document).map_err(|_| "Cannot encode imported roster")?,
    )?;
    Ok(added)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "Cannot create import directory")?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes).map_err(|_| "Cannot write staged avatar")?;
    fs::rename(&temporary, path).map_err(|_| "Cannot publish staged avatar".to_string())
}

/// Explicit terminal-native pairing for the first integration. The terminal
/// shows only the public userCode/verificationUrl, never deviceCode or tokens.
pub fn pair_interactively(api: PassportApi) -> Result<NativeSession, String> {
    let pairing = api.pair()?;
    eprintln!("Connect Omoba to your wallet: {}", pairing.verification_url);
    eprintln!(
        "Approval code: {}. Waiting for browser approval (expires {}).",
        pairing.user_code, pairing.expires_at
    );
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(600) {
            return Err("Pairing expired. Start Omoba again to retry.".into());
        }
        std::thread::sleep(Duration::from_secs(pairing.interval.clamp(3, 10)));
        match api.poll(&pairing.device_code)? {
            PairingPoll::Pending => {}
            PairingPoll::Approved {
                access_token,
                wallet,
                ..
            } => {
                let session = api.session(access_token, &wallet)?;
                let count = session
                    .library
                    .items
                    .iter()
                    .filter(|avatar| avatar.support.iter().any(is_desktop_omoba_selector))
                    .count();
                eprintln!("Wallet connected. {count} purchased avatar(s) approved for Omoba.");
                if count == 0 {
                    eprintln!(
                        "Your supported library is empty. The free roster remains available."
                    );
                }
                return Ok(session);
            }
        }
    }
}

#[cfg(test)]
mod tests;
