//! Native transport and staged importer for the shared Ekza passport contract.
//! Secrets stay in memory. Game packets contain only one-use scoped tickets.

pub mod account;
pub mod community;
pub mod device_account;
pub mod humanoid;
pub mod store;
pub mod supporter_account;
pub mod web_account;

use ekza_bevy_sdk::passport::{
    ConsumedTicket, OMOBA_PROJECT, ProjectSupport, ProtectedAvatar, PurchasedLibrary,
    SupportSelector, client::PassportClient, validate_omoba_support,
};
pub use ekza_bevy_sdk::passport::{
    client::{AvatarTicket, DevicePairing, NativeSession, PairingPoll, valid_session_id},
    pairing::{PairingFlow, PairingState, open_in_browser},
    protected_slug,
};
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path, time::Duration};

/// Operator override of the trusted storefront passport API.
pub const PASSPORT_URL_ENV: &str = "OMOBA_PASSPORT_URL";
/// Omoba only accepts devnet libraries until a mainnet storefront exists.
const LIBRARY_NETWORK: &str = "solana-devnet";

/// The exact approval Omoba's desktop runtime can load.
pub fn selector() -> SupportSelector {
    SupportSelector::omoba_desktop()
}

/// Omoba's view of the Ekza passport: the SDK transport bound to the `omoba`
/// project, plus the humanoid profile check on every downloaded rendition.
#[derive(Clone)]
pub struct PassportApi(PassportClient);

impl PassportApi {
    pub fn new(base: &str) -> Result<Self, String> {
        PassportClient::new(base, OMOBA_PROJECT).map(Self)
    }

    /// `OMOBA_PASSPORT_URL`, then the SDK-wide `EKZA_PASSPORT_URL`, then the
    /// public Ekza storefront. The origin is always operator- or build-defined,
    /// never taken from a game packet.
    pub fn from_env() -> Result<Self, String> {
        let base = std::env::var(PASSPORT_URL_ENV)
            .or_else(|_| std::env::var(ekza_bevy_sdk::passport::client::PASSPORT_URL_ENV))
            .unwrap_or_else(|_| ekza_bevy_sdk::registry::DEFAULT_PASSPORT_URL.to_owned());
        Self::new(&base)
    }

    pub fn client(&self) -> &PassportClient {
        &self.0
    }

    pub fn pair(&self) -> Result<DevicePairing, String> {
        self.0.pair()
    }

    pub fn poll(&self, device_code: &str) -> Result<PairingPoll, String> {
        self.0.poll(device_code)
    }

    pub fn session(&self, token: String, expected_wallet: &str) -> Result<NativeSession, String> {
        let session = self.0.session(token, expected_wallet)?;
        check_network(&session.library)?;
        Ok(session)
    }

    pub fn consume(&self, ticket: &str, session_id: &str) -> Result<ConsumedTicket, String> {
        self.0.consume(ticket, session_id)
    }

    pub fn download(&self, protected: &ProtectedAvatar) -> Result<Vec<u8>, String> {
        download(&self.0, protected)
    }
}

/// Accept a session paired through [`PairingFlow`]: same network rule as
/// [`PassportApi::session`].
pub fn accept_session(session: NativeSession) -> Result<NativeSession, String> {
    check_network(&session.library)?;
    Ok(session)
}

fn check_network(library: &PurchasedLibrary) -> Result<(), String> {
    if library.network != LIBRARY_NETWORK {
        return Err("Passport library belongs to a different wallet or network".into());
    }
    Ok(())
}

/// Bounded, tokenless download checked against the approval and Omoba's profile.
pub fn download(client: &PassportClient, protected: &ProtectedAvatar) -> Result<Vec<u8>, String> {
    protected.validate()?;
    let bytes = client.download(protected, &selector())?;
    validate_humanoid_profile(&bytes)?;
    Ok(bytes)
}

/// One-use ticket for an owned avatar, bound to this game session.
pub fn ticket(
    session: &NativeSession,
    protected: &ProtectedAvatar,
    session_id: &str,
) -> Result<AvatarTicket, String> {
    protected.validate()?;
    session.ticket(protected, &selector(), session_id)
}

pub fn verify_bytes(protected: &ProtectedAvatar, bytes: &[u8]) -> Result<(), String> {
    protected
        .validate_bytes(bytes, &ekza_bevy_sdk::sha256_hex(bytes))
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
            let bytes = download(&session.api, &protected)?;
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
