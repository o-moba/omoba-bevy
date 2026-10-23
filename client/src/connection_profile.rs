//! Connection profiles: which lobby, Ekza registry, passport, market and Studio
//! this build talks to, and where it keeps the state that belongs to them.
//!
//! - A **production** build has exactly one profile, [`ProfileId::Production`]:
//!   the public lobby, the Ekza registry and passport, `app.omoba.io/market`. It
//!   shows no server or connection selector anywhere. Desktop developers and
//!   self-hosters override it with environment variables (`GAME_SERVER_ADDR`,
//!   `OMOBA_REGISTRY_URL`, `OMOBA_PASSPORT_URL`); phones read none.
//! - A **contour** build (`OMOBA_DEFAULT_CONFIG_DIR_SUFFIX`, the `dev` tester
//!   app) offers [`ProfileId::Dev`] (the default: the contour's own lobby,
//!   registry, market and Studio, the closed passport), [`ProfileId::Local`]
//!   (services on this computer, loopback only) and [`ProfileId::Custom`] (a
//!   typed lobby and an optional registry). It never offers production, and a
//!   custom production lobby or registry is refused.
//!
//! The choice is saved in [`SELECTION_FILE`] in the settings folder and applies
//! at the next start: the Ekza stores, the account link and the `ekza://` asset
//! source are opened once per process, so a running game never switches
//! registries under them. Each contour profile keeps its Ekza link, store
//! caches and match-resume hint in `profiles/<key>/` of the settings folder
//! ([`profile_file`]); production keeps them in the settings folder itself.
//! Graphics, audio, language and the career identity stay shared.
//!
//! A build without Ekza (`shared::market::FEATURES.ekza` off, the free RU/CN
//! markets) runs every profile with the closed loopback registry and passport,
//! no market and no Studio ([`for_market`]), and takes no desktop registry or
//! passport override: no Ekza host is ever contacted.
// i18n-strict
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::session_config::{
    BETA_GAME_SERVER_ADDR, BUILD_CONTOUR, COMPILED_GAME_SERVER_ADDR, FALLBACK_GAME_SERVER_ADDR,
    endpoint_allowed_for,
};

/// The saved choice of a contour build, in the settings folder.
pub(crate) const SELECTION_FILE: &str = "connection_profile.json";
/// Folder (in the settings folder) of the per-profile folders.
pub(crate) const PROFILES_DIR: &str = "profiles";
/// The Ekza store folder (account link, avatar and weapon caches) of a profile.
pub(crate) const STORE_DIR: &str = "ekza-store";
/// A closed loopback port: wallet pairing and purchased-avatar tickets fail
/// closed instead of reaching the production passport.
pub(crate) const CLOSED_PASSPORT: &str = "http://127.0.0.1:9/api/passport";
/// The same closed port as the registry of a build without Ekza: a store or
/// account call that slipped past the gates fails at once on this computer.
pub(crate) const CLOSED_REGISTRY: &str = "http://127.0.0.1:9";
pub(crate) const LOCAL_REGISTRY: &str = "http://127.0.0.1:8018";
const LOCAL_MARKET: &str = "127.0.0.1:3010/market";
const LOCAL_STUDIO: &str = "127.0.0.1:5188/studio";
const PRODUCTION_MARKET: &str = "app.omoba.io/market";
const PRODUCTION_STUDIO: &str = "studio.ekza.io/studio";
/// Longest custom lobby address or registry URL the panel accepts.
pub(crate) const MAX_FIELD_CHARS: usize = 96;
/// Bound on the selection file: it holds two short strings.
const MAX_SELECTION_BYTES: u64 = 4096;
const LOOPBACK_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "::1"];

/// One profile of the compile-time list ([`profiles_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProfileId {
    /// The public lobby and Ekza registry: a production build's only profile.
    Production,
    /// The contour's own servers (dev lobby, dev registry): a contour build's default.
    Dev,
    /// Lobby, registry, market and Studio on this computer.
    Local,
    /// A lobby the tester types, with an optional registry.
    Custom,
}

impl ProfileId {
    /// Storage name: the selection file value and the `profiles/` folder.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            ProfileId::Production => "production",
            ProfileId::Dev => "dev",
            ProfileId::Local => "local",
            ProfileId::Custom => "custom",
        }
    }

    /// Dictionary key of the profile's name.
    pub(crate) const fn label_key(self) -> &'static str {
        match self {
            ProfileId::Production => "connection.profile.production",
            ProfileId::Dev => "connection.profile.dev",
            ProfileId::Local => "connection.profile.local",
            ProfileId::Custom => "connection.profile.custom",
        }
    }

    /// Test id of the profile's button in the connection panel.
    pub(crate) const fn test_id(self) -> &'static str {
        match self {
            ProfileId::Production => "ConnectionProfileProduction",
            ProfileId::Dev => "ConnectionProfileDev",
            ProfileId::Local => "ConnectionProfileLocal",
            ProfileId::Custom => "ConnectionProfileCustom",
        }
    }
}

/// The profiles a build offers, in button order. A production build has one
/// and therefore no selector.
pub(crate) fn profiles_for(contour: Option<&str>) -> &'static [ProfileId] {
    match contour.filter(|contour| !contour.is_empty()) {
        Some(_) => &[ProfileId::Dev, ProfileId::Local, ProfileId::Custom],
        None => &[ProfileId::Production],
    }
}

/// Whether this build shows a connection selector (only contour builds do).
pub(crate) fn selectable() -> bool {
    profiles_for(BUILD_CONTOUR).len() > 1
}

/// What a custom profile names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CustomEndpoints {
    /// The lobby, `host:port`.
    pub lobby: String,
    /// The Ekza registry origin; none uses the contour's registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

/// The saved choice ([`SELECTION_FILE`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Selection {
    pub profile: ProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomEndpoints>,
}

/// Everything one profile points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionProfile {
    pub id: ProfileId,
    /// The lobby, `host:port`.
    pub lobby: String,
    /// The Ekza registry origin.
    pub registry: String,
    /// The Ekza passport API (`…/api/passport`).
    pub passport: String,
    /// The market players get free items from, without the scheme (`{market}`).
    pub market: String,
    /// Ekza Studio, without the scheme.
    pub studio: String,
    /// This profile's own folder, relative to the settings folder; empty for
    /// production (the settings folder itself).
    pub storage: PathBuf,
}

impl ConnectionProfile {
    /// Whether the passport is the closed loopback port.
    pub(crate) fn passport_closed(&self) -> bool {
        self.passport == CLOSED_PASSPORT
    }

    /// The saved choice that selects this profile.
    pub(crate) fn selection(&self) -> Selection {
        Selection {
            profile: self.id,
            custom: (self.id == ProfileId::Custom).then(|| CustomEndpoints {
                lobby: self.lobby.clone(),
                registry: Some(self.registry.clone()),
            }),
        }
    }
}

/// Why a custom profile was refused; each names a `connection.error.*` message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileError {
    /// Not a `host:port` lobby address.
    Lobby,
    /// Not an `https://` (or loopback `http://`) registry origin.
    Registry,
    /// A production lobby or registry, which a contour build never uses.
    Production,
}

impl ProfileError {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            ProfileError::Lobby => "connection.error.lobby",
            ProfileError::Registry => "connection.error.registry",
            ProfileError::Production => "connection.error.production",
        }
    }
}

/// The endpoints compiled into this build (`scripts/release.py` sets them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Compiled {
    pub lobby: Option<&'static str>,
    pub registry: Option<&'static str>,
    pub passport: Option<&'static str>,
}

/// This build's compiled endpoints.
pub(crate) const COMPILED: Compiled = Compiled {
    lobby: COMPILED_GAME_SERVER_ADDR,
    registry: omoba_passport::community::COMPILED_REGISTRY_URL,
    passport: omoba_passport::COMPILED_PASSPORT_URL,
};

fn production_profile(compiled: &Compiled) -> ConnectionProfile {
    ConnectionProfile {
        id: ProfileId::Production,
        lobby: compiled.lobby.unwrap_or(BETA_GAME_SERVER_ADDR).to_owned(),
        registry: compiled
            .registry
            .unwrap_or(ekza_bevy_sdk::registry::DEFAULT_REGISTRY_URL)
            .to_owned(),
        passport: compiled
            .passport
            .unwrap_or(ekza_bevy_sdk::registry::DEFAULT_PASSPORT_URL)
            .to_owned(),
        market: PRODUCTION_MARKET.to_owned(),
        studio: PRODUCTION_STUDIO.to_owned(),
        storage: PathBuf::new(),
    }
}

/// The contour's own servers: what the contour build was compiled with
/// (`--contour dev`: `lobby.dev.omoba.io:4100`, `registry.dev.ekza.io`, the
/// closed passport), else the contour's public names.
fn contour_profile(contour: &str, compiled: &Compiled) -> ConnectionProfile {
    ConnectionProfile {
        id: ProfileId::Dev,
        lobby: compiled
            .lobby
            .map_or_else(|| format!("lobby.{contour}.omoba.io:4100"), str::to_owned),
        registry: compiled.registry.map_or_else(
            || format!("https://registry.{contour}.ekza.io"),
            str::to_owned,
        ),
        passport: compiled.passport.unwrap_or(CLOSED_PASSPORT).to_owned(),
        market: format!("app.{contour}.omoba.io/market"),
        studio: format!("studio.{contour}.ekza.io/studio"),
        storage: Path::new(PROFILES_DIR).join(ProfileId::Dev.key()),
    }
}

/// Lobby `127.0.0.1:4000`, registry `127.0.0.1:8018`, market `:3010`, Studio
/// `:5188` (the local Ekza stack, `docs/ekza-lan.md`), the closed passport.
fn local_profile() -> ConnectionProfile {
    ConnectionProfile {
        id: ProfileId::Local,
        lobby: FALLBACK_GAME_SERVER_ADDR.to_owned(),
        registry: LOCAL_REGISTRY.to_owned(),
        passport: CLOSED_PASSPORT.to_owned(),
        market: LOCAL_MARKET.to_owned(),
        studio: LOCAL_STUDIO.to_owned(),
        storage: Path::new(PROFILES_DIR).join(ProfileId::Local.key()),
    }
}

/// A typed lobby (and optional registry) on a contour build. Its market and
/// Studio are the contour's; its passport is closed. Each lobby/registry pair
/// gets its own folder, so another custom server never sees this one's link.
pub(crate) fn custom_profile(
    contour: &str,
    endpoints: &CustomEndpoints,
    compiled: &Compiled,
) -> Result<ConnectionProfile, ProfileError> {
    let lobby = validate_lobby(Some(contour), &endpoints.lobby)?;
    let contour_servers = contour_profile(contour, compiled);
    let registry = match endpoints
        .registry
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
    {
        Some(raw) => validate_registry(Some(contour), raw)?,
        None => contour_servers.registry,
    };
    let storage = Path::new(PROFILES_DIR).join(format!(
        "{}-{:012x}",
        ProfileId::Custom.key(),
        fnv1a(&format!("{lobby}\n{registry}")) >> 16
    ));
    Ok(ConnectionProfile {
        id: ProfileId::Custom,
        lobby,
        registry,
        passport: CLOSED_PASSPORT.to_owned(),
        market: contour_servers.market,
        studio: contour_servers.studio,
        storage,
    })
}

/// `profile` as a build with `ekza` runs it: unchanged with Ekza; without it
/// the closed registry and passport and no market or Studio. The lobby and the
/// profile's folder stay.
pub(crate) fn for_market(profile: ConnectionProfile, ekza: bool) -> ConnectionProfile {
    if ekza {
        return profile;
    }
    ConnectionProfile {
        registry: CLOSED_REGISTRY.to_owned(),
        passport: CLOSED_PASSPORT.to_owned(),
        market: String::new(),
        studio: String::new(),
        ..profile
    }
}

/// A built-in profile of `contour` (production, dev or local); none for custom
/// or for a profile the contour does not offer.
pub(crate) fn builtin_profile(
    contour: Option<&str>,
    id: ProfileId,
    compiled: &Compiled,
) -> Option<ConnectionProfile> {
    if !profiles_for(contour).contains(&id) {
        return None;
    }
    match (contour, id) {
        (None, ProfileId::Production) => Some(production_profile(compiled)),
        (Some(contour), ProfileId::Dev) => Some(contour_profile(contour, compiled)),
        (Some(_), ProfileId::Local) => Some(local_profile()),
        _ => None,
    }
}

/// The profile a build runs with: production always runs production; a contour
/// build runs its saved choice while that is one of its profiles and valid,
/// else its own servers ([`ProfileId::Dev`]).
pub(crate) fn resolve(
    contour: Option<&str>,
    saved: Option<&Selection>,
    compiled: &Compiled,
) -> ConnectionProfile {
    let Some(contour) = contour.filter(|contour| !contour.is_empty()) else {
        return production_profile(compiled);
    };
    match saved {
        Some(Selection {
            profile: ProfileId::Local,
            ..
        }) => local_profile(),
        Some(Selection {
            profile: ProfileId::Custom,
            custom: Some(endpoints),
        }) => custom_profile(contour, endpoints, compiled)
            .unwrap_or_else(|_| contour_profile(contour, compiled)),
        _ => contour_profile(contour, compiled),
    }
}

/// A custom lobby address: `host:port`, and never production on a contour build.
pub(crate) fn validate_lobby(contour: Option<&str>, raw: &str) -> Result<String, ProfileError> {
    let raw = raw.trim();
    if raw.chars().count() > MAX_FIELD_CHARS {
        return Err(ProfileError::Lobby);
    }
    match crate::persistence::validate_game_server_addr_in(contour, raw) {
        Some(address) => Ok(address),
        None if crate::persistence::validate_game_server_addr_in(None, raw).is_some() => {
            Err(ProfileError::Production)
        }
        None => Err(ProfileError::Lobby),
    }
}

/// A custom registry origin, the SDK's rule: `https://host[:port]` anywhere,
/// `http://` only on loopback, no credentials, path, query or fragment; never
/// a production registry on a contour build. Returns it without a trailing `/`.
pub(crate) fn validate_registry(contour: Option<&str>, raw: &str) -> Result<String, ProfileError> {
    let raw = raw.trim();
    if raw.chars().count() > MAX_FIELD_CHARS {
        return Err(ProfileError::Registry);
    }
    let raw = raw.strip_suffix('/').unwrap_or(raw);
    let (scheme, authority) = raw.split_once("://").ok_or(ProfileError::Registry)?;
    let scheme = scheme.to_ascii_lowercase();
    if authority.is_empty()
        || authority
            .chars()
            .any(|ch| matches!(ch, '/' | '?' | '#' | '@' | '\\') || ch.is_whitespace())
    {
        return Err(ProfileError::Registry);
    }
    let host = authority_host(authority).ok_or(ProfileError::Registry)?;
    let loopback = LOOPBACK_HOSTS.contains(&host.to_ascii_lowercase().as_str());
    match scheme.as_str() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(ProfileError::Registry),
    }
    if !endpoint_allowed_for(contour, host) {
        return Err(ProfileError::Production);
    }
    Ok(format!("{scheme}://{authority}"))
}

/// The host of `host[:port]` or `[v6][:port]`, when both parts are valid.
fn authority_host(authority: &str) -> Option<&str> {
    let valid_port = |port: &str| port.parse::<u16>().is_ok_and(|port| port != 0);
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        host.parse::<std::net::Ipv6Addr>().ok()?;
        return match after {
            "" => Some(host),
            after => after
                .strip_prefix(':')
                .filter(|port| valid_port(port))
                .map(|_| host),
        };
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if valid_port(port) => host,
        Some(_) => return None,
        None => authority,
    };
    let name = host.strip_suffix('.').unwrap_or(host);
    let dns = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    dns.then_some(host)
}

/// 64-bit FNV-1a: a stable folder name for a custom lobby/registry pair.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The saved choice at `path`; none when missing, oversized or malformed.
pub(crate) fn load_selection(path: &Path) -> Option<Selection> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_SELECTION_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Saves the choice atomically (a sibling temp file renamed over `path`).
pub(crate) fn save_selection(path: &Path, selection: &Selection) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(selection)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary = path.with_extension("json.tmp");
    let result = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// `connection_profile.json` in this build's settings folder.
pub(crate) fn selection_path() -> Option<PathBuf> {
    crate::platform::settings_directory().map(|base| base.join(SELECTION_FILE))
}

/// `base` joined with `profile`'s own folder (the folder itself for production).
pub(crate) fn profile_directory_in(base: &Path, profile: &ConnectionProfile) -> PathBuf {
    if profile.storage.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(&profile.storage)
    }
}

/// A file or folder that belongs to the running profile (its Ekza store, its
/// match-resume hint).
pub(crate) fn profile_file(name: &str) -> Option<PathBuf> {
    crate::platform::settings_directory()
        .map(|base| profile_directory_in(&base, active()).join(name))
}

/// Contour builds before profiles kept the Ekza store in the settings folder.
/// It holds a link to the contour's own registry, so the Dev profile adopts it
/// once (when it has no store yet). Returns whether it moved.
pub(crate) fn adopt_legacy_store(base: &Path, profile: &ConnectionProfile) -> bool {
    if profile.id != ProfileId::Dev {
        return false;
    }
    let legacy = base.join(STORE_DIR);
    let target = profile_directory_in(base, profile).join(STORE_DIR);
    if !legacy.is_dir() || target.exists() {
        return false;
    }
    target
        .parent()
        .is_some_and(|parent| std::fs::create_dir_all(parent).is_ok())
        && std::fs::rename(&legacy, &target).is_ok()
}

static ACTIVE: OnceLock<ConnectionProfile> = OnceLock::new();

/// The choice saved at `path` that this build runs with. A production build
/// never reads it; unit tests never do either, so a dev-contour test run is the
/// build's default profile, not the developer's own saved choice.
fn saved_choice(path: Option<&Path>) -> Option<Selection> {
    if cfg!(test) {
        return None;
    }
    BUILD_CONTOUR.and(path).and_then(load_selection)
}

/// The profile this process runs with, resolved once.
pub(crate) fn active() -> &'static ConnectionProfile {
    ACTIVE.get_or_init(|| {
        let saved = saved_choice(selection_path().as_deref());
        for_market(
            resolve(BUILD_CONTOUR, saved.as_ref(), &COMPILED),
            shared::market::FEATURES.ekza,
        )
    })
}

/// A desktop developer override from the environment; phones read none.
pub(crate) fn desktop_override(key: &str) -> Option<String> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return None;
    }
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Whether a build of `contour` takes `url` as a desktop endpoint override: a
/// production build takes any (developers and self-hosters; the SDK still
/// validates it), a contour build never one on a production host, the same
/// rule as its custom profile and its lobby override.
pub(crate) fn override_allowed(contour: Option<&str>, url: &str) -> bool {
    if contour.is_none() {
        return true;
    }
    let Some((_, rest)) = url.trim().split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    authority_host(authority).is_some_and(|host| endpoint_allowed_for(contour, host))
}

/// The first set desktop override among `keys` that this build accepts
/// ([`override_allowed`]); a refused one is reported and ignored. A build
/// without Ekza takes none: these are all Ekza endpoints.
fn endpoint_override(keys: &[&str], ekza: bool) -> Option<String> {
    let (key, value) = keys
        .iter()
        .find_map(|key| desktop_override(key).map(|value| (*key, value)))?;
    // Before the app (and its logger) exists.
    if !ekza {
        eprintln!("Ignoring {key}: this build has no Ekza services");
        return None;
    }
    if override_allowed(BUILD_CONTOUR, &value) {
        return Some(value);
    }
    eprintln!("Ignoring {key}: this test build never uses a production endpoint");
    None
}

/// Startup, before any store, account call or socket: resolves the profile and
/// pins the registry and passport it names (or the desktop overrides) for the
/// whole process. A contour build's Dev profile adopts a pre-profile store.
pub(crate) fn install() -> &'static ConnectionProfile {
    let profile = active();
    if BUILD_CONTOUR.is_some()
        && let Some(base) = crate::platform::settings_directory()
        && adopt_legacy_store(&base, profile)
    {
        eprintln!("Moved the Ekza store into {}", profile.storage.display());
    }
    let ekza = shared::market::FEATURES.ekza;
    let registry = endpoint_override(&[omoba_passport::store::REGISTRY_URL_ENV], ekza)
        .unwrap_or_else(|| profile.registry.clone());
    omoba_passport::community::set_registry_url(registry);
    let passport = endpoint_override(
        &[
            omoba_passport::PASSPORT_URL_ENV,
            ekza_bevy_sdk::passport::client::PASSPORT_URL_ENV,
        ],
        ekza,
    )
    .unwrap_or_else(|| profile.passport.clone());
    omoba_passport::set_passport_url(passport);
    // Before the app (and its logger) exists, like the roster summary.
    eprintln!(
        "Connection profile {}: lobby {}, registry {}",
        profile.id.key(),
        profile.lobby,
        profile.registry
    );
    profile
}

/// What the connection UI shows: the running profile, what this build offers,
/// the saved choice and where it is saved.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct ActiveConnection {
    pub contour: Option<String>,
    /// The profile this process runs with.
    pub profile: ConnectionProfile,
    /// A saved choice that differs from the running profile: it applies at the
    /// next start.
    pub pending: Option<ConnectionProfile>,
    /// The custom lobby/registry last saved, to prefill the panel.
    pub custom: Option<CustomEndpoints>,
    /// Where the panel saves the choice; none when there is no settings folder.
    pub selection_file: Option<PathBuf>,
    pub compiled: Compiled,
}

impl Default for ActiveConnection {
    fn default() -> Self {
        Self::current()
    }
}

impl ActiveConnection {
    /// This process: its build contour, running profile and saved choice.
    pub(crate) fn current() -> Self {
        let selection_file = selection_path();
        let saved = saved_choice(selection_file.as_deref());
        Self {
            contour: BUILD_CONTOUR.map(str::to_owned),
            profile: active().clone(),
            pending: None,
            custom: saved.and_then(|saved| saved.custom),
            selection_file,
            compiled: COMPILED,
        }
    }

    /// A build of `contour` running its default profile, saving to `file`.
    #[cfg(test)]
    pub(crate) fn for_contour(contour: Option<&str>, file: Option<PathBuf>) -> Self {
        let compiled = Compiled::default();
        Self {
            contour: contour.map(str::to_owned),
            profile: resolve(contour, None, &compiled),
            pending: None,
            custom: None,
            selection_file: file,
            compiled,
        }
    }

    /// The profiles the selector offers.
    pub(crate) fn choices(&self) -> &'static [ProfileId] {
        profiles_for(self.contour.as_deref())
    }

    /// Whether this build shows a connection selector.
    pub(crate) fn selectable(&self) -> bool {
        self.choices().len() > 1
    }

    /// The profile that will run after a restart: the pending one, else the running one.
    pub(crate) fn next(&self) -> &ConnectionProfile {
        self.pending.as_ref().unwrap_or(&self.profile)
    }

    /// The profile `id` names on this build; `custom` for [`ProfileId::Custom`].
    pub(crate) fn candidate(
        &self,
        id: ProfileId,
        custom: &CustomEndpoints,
    ) -> Result<ConnectionProfile, ProfileError> {
        match (self.contour.as_deref(), id) {
            (Some(contour), ProfileId::Custom) => custom_profile(contour, custom, &self.compiled),
            (contour, id) => {
                builtin_profile(contour, id, &self.compiled).ok_or(ProfileError::Production)
            }
        }
        .map(|profile| for_market(profile, shared::market::FEATURES.ekza))
    }

    /// Saves `profile` as the choice for the next start. Choosing the running
    /// profile cancels a pending switch.
    pub(crate) fn choose(&mut self, profile: ConnectionProfile) -> io::Result<()> {
        let path = self
            .selection_file
            .as_deref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let selection = profile.selection();
        save_selection(path, &selection)?;
        if let Some(custom) = selection.custom {
            self.custom = Some(custom);
        }
        self.pending = (profile != self.profile).then_some(profile);
        Ok(())
    }
}

pub(crate) struct ConnectionProfilePlugin;

impl Plugin for ConnectionProfilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveConnection>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV_BUILD: Compiled = Compiled {
        lobby: Some("lobby.dev.omoba.io:4100"),
        registry: Some("https://registry.dev.ekza.io"),
        passport: Some(CLOSED_PASSPORT),
    };

    fn custom(lobby: &str, registry: Option<&str>) -> CustomEndpoints {
        CustomEndpoints {
            lobby: lobby.into(),
            registry: registry.map(str::to_owned),
        }
    }

    #[test]
    fn production_offers_one_profile_and_dev_offers_dev_local_custom() {
        assert_eq!(profiles_for(None), [ProfileId::Production]);
        assert_eq!(profiles_for(Some("")), [ProfileId::Production]);
        assert_eq!(
            profiles_for(Some("dev")),
            [ProfileId::Dev, ProfileId::Local, ProfileId::Custom]
        );
        assert!(!profiles_for(Some("dev")).contains(&ProfileId::Production));
        assert_eq!(selectable(), BUILD_CONTOUR.is_some());
    }

    #[test]
    fn production_runs_the_public_lobby_and_registry_whatever_was_saved() {
        let profile = resolve(None, None, &Compiled::default());
        assert_eq!(profile.id, ProfileId::Production);
        assert_eq!(profile.lobby, BETA_GAME_SERVER_ADDR);
        assert_eq!(profile.registry, "https://registry.ekza.io");
        assert_eq!(profile.passport, "https://avatar.ekza.io/api/passport");
        assert_eq!(profile.market, "app.omoba.io/market");
        assert_eq!(
            profile.storage,
            PathBuf::new(),
            "the settings folder itself"
        );
        for saved in [
            Selection {
                profile: ProfileId::Local,
                custom: None,
            },
            Selection {
                profile: ProfileId::Custom,
                custom: Some(custom("192.168.1.20:4000", None)),
            },
        ] {
            assert_eq!(resolve(None, Some(&saved), &Compiled::default()), profile);
        }
        // A self-hosted production build keeps its compiled endpoints.
        let compiled = Compiled {
            lobby: Some("play.example.org:4000"),
            registry: Some("https://registry.example.org"),
            passport: None,
        };
        let own = resolve(None, None, &compiled);
        assert_eq!(own.lobby, "play.example.org:4000");
        assert_eq!(own.registry, "https://registry.example.org");
    }

    #[test]
    fn a_build_without_ekza_closes_every_ekza_endpoint_of_every_profile() {
        use shared::market::Market;
        let production = resolve(None, None, &Compiled::default());
        let ekza = Market::Global.default_features().ekza;
        assert_eq!(for_market(production.clone(), ekza), production);
        let profiles = [
            production,
            resolve(Some("dev"), None, &DEV_BUILD),
            local_profile(),
            custom_profile(
                "dev",
                &custom("192.168.1.20:4000", Some("https://registry.lan.example")),
                &DEV_BUILD,
            )
            .unwrap(),
        ];
        for market in [Market::Ru, Market::Cn] {
            for profile in &profiles {
                let free = for_market(profile.clone(), market.default_features().ekza);
                assert_eq!(free.registry, CLOSED_REGISTRY, "{:?}", profile.id);
                assert!(free.passport_closed());
                assert!(free.market.is_empty() && free.studio.is_empty());
                // The lobby and the profile's own folder are not Ekza's.
                assert_eq!((free.id, &free.lobby), (profile.id, &profile.lobby));
                assert_eq!(free.storage, profile.storage);
            }
        }
        assert!(CLOSED_PASSPORT.starts_with(CLOSED_REGISTRY));
    }

    #[test]
    fn a_dev_build_defaults_to_its_own_servers() {
        for compiled in [DEV_BUILD, Compiled::default()] {
            let dev = resolve(Some("dev"), None, &compiled);
            assert_eq!(dev.id, ProfileId::Dev);
            assert_eq!(dev.lobby, "lobby.dev.omoba.io:4100");
            assert_eq!(dev.registry, "https://registry.dev.ekza.io");
            assert_eq!(dev.market, "app.dev.omoba.io/market");
            assert_eq!(dev.studio, "studio.dev.ekza.io/studio");
            assert!(dev.passport_closed());
            assert_eq!(dev.storage, Path::new("profiles/dev"));
        }
        // `--server <dev VPS IP>:4100` wins over the DNS name.
        let by_ip = Compiled {
            lobby: Some("203.0.113.10:4100"),
            ..DEV_BUILD
        };
        assert_eq!(
            resolve(Some("dev"), None, &by_ip).lobby,
            "203.0.113.10:4100"
        );
    }

    #[test]
    fn local_is_loopback_only() {
        let local = resolve(
            Some("dev"),
            Some(&Selection {
                profile: ProfileId::Local,
                custom: None,
            }),
            &DEV_BUILD,
        );
        assert_eq!(local.lobby, "127.0.0.1:4000");
        assert_eq!(local.registry, "http://127.0.0.1:8018");
        assert_eq!(local.market, "127.0.0.1:3010/market");
        assert_eq!(local.studio, "127.0.0.1:5188/studio");
        assert!(local.passport_closed());
        for address in [&local.lobby, &local.registry, &local.market, &local.studio] {
            assert!(address.contains("127.0.0.1"), "{address}");
        }
    }

    #[test]
    fn a_saved_custom_profile_runs_and_an_invalid_one_falls_back_to_dev() {
        let saved = Selection {
            profile: ProfileId::Custom,
            custom: Some(custom(
                "192.168.1.20:4000",
                Some("https://registry.lan.example/"),
            )),
        };
        let profile = resolve(Some("dev"), Some(&saved), &DEV_BUILD);
        assert_eq!(profile.id, ProfileId::Custom);
        assert_eq!(profile.lobby, "192.168.1.20:4000");
        assert_eq!(profile.registry, "https://registry.lan.example");
        assert_eq!(profile.market, "app.dev.omoba.io/market");
        assert!(profile.passport_closed());
        // No registry: the contour's.
        let lobby_only = resolve(
            Some("dev"),
            Some(&Selection {
                profile: ProfileId::Custom,
                custom: Some(custom("host.example:4100", None)),
            }),
            &DEV_BUILD,
        );
        assert_eq!(lobby_only.registry, "https://registry.dev.ekza.io");
        for broken in [
            Selection {
                profile: ProfileId::Custom,
                custom: None,
            },
            Selection {
                profile: ProfileId::Custom,
                custom: Some(custom("77.246.105.57:4000", None)),
            },
            Selection {
                profile: ProfileId::Custom,
                custom: Some(custom(
                    "host.example:4100",
                    Some("https://registry.ekza.io"),
                )),
            },
            Selection {
                profile: ProfileId::Production,
                custom: None,
            },
        ] {
            assert_eq!(
                resolve(Some("dev"), Some(&broken), &DEV_BUILD).id,
                ProfileId::Dev,
                "{broken:?}"
            );
        }
    }

    #[test]
    fn custom_refuses_production_and_malformed_endpoints() {
        let dev = Some("dev");
        for (lobby, error) in [
            ("77.246.105.57:4000", ProfileError::Production),
            ("lobby.omoba.io:4000", ProfileError::Production),
            ("[::ffff:77.246.105.57]:4000", ProfileError::Production),
            ("192.168.1.20", ProfileError::Lobby),
            ("", ProfileError::Lobby),
            ("bad host:4000", ProfileError::Lobby),
        ] {
            assert_eq!(validate_lobby(dev, lobby), Err(error), "{lobby}");
        }
        assert_eq!(
            validate_lobby(dev, " lobby.dev.omoba.io:4100 "),
            Ok("lobby.dev.omoba.io:4100".into())
        );
        for (registry, error) in [
            ("https://registry.ekza.io", ProfileError::Production),
            ("https://REGISTRY.EKZA.IO/", ProfileError::Production),
            ("https://ekza.io", ProfileError::Production),
            ("http://registry.dev.ekza.io", ProfileError::Registry),
            ("ftp://registry.dev.ekza.io", ProfileError::Registry),
            ("registry.dev.ekza.io", ProfileError::Registry),
            (
                "https://user:pass@registry.dev.ekza.io",
                ProfileError::Registry,
            ),
            ("https://registry.dev.ekza.io/feed", ProfileError::Registry),
            ("https://registry.dev.ekza.io?x=1", ProfileError::Registry),
            ("https://registry.dev.ekza.io:0", ProfileError::Registry),
            ("https://", ProfileError::Registry),
            ("https://bad host", ProfileError::Registry),
        ] {
            assert_eq!(validate_registry(dev, registry), Err(error), "{registry}");
        }
        for (registry, origin) in [
            (
                "https://registry.dev.ekza.io/",
                "https://registry.dev.ekza.io",
            ),
            (
                "HTTPS://registry.lan.example:8443",
                "https://registry.lan.example:8443",
            ),
            ("http://127.0.0.1:8018", "http://127.0.0.1:8018"),
            ("http://localhost:8018", "http://localhost:8018"),
            ("http://[::1]:8018", "http://[::1]:8018"),
        ] {
            assert_eq!(validate_registry(dev, registry), Ok(origin.into()));
        }
        // Production builds have no custom profile; the URL rule alone accepts production.
        assert!(validate_registry(None, "https://registry.ekza.io").is_ok());
    }

    #[test]
    fn every_dev_build_profile_stays_off_production() {
        let dev = Some("dev");
        for id in profiles_for(dev) {
            let profile = match id {
                ProfileId::Custom => {
                    custom_profile("dev", &custom("192.168.1.20:4000", None), &DEV_BUILD).unwrap()
                }
                id => builtin_profile(dev, *id, &DEV_BUILD).unwrap(),
            };
            let lobby_host = profile.lobby.rsplit_once(':').unwrap().0;
            assert!(endpoint_allowed_for(dev, lobby_host), "{id:?}");
            let registry_host = authority_host(profile.registry.split_once("://").unwrap().1);
            assert!(endpoint_allowed_for(dev, registry_host.unwrap()), "{id:?}");
            let market_host = profile.market.split('/').next().unwrap();
            assert!(endpoint_allowed_for(
                dev,
                market_host.split(':').next().unwrap()
            ));
            assert!(profile.passport_closed());
        }
        assert_eq!(
            builtin_profile(dev, ProfileId::Production, &DEV_BUILD),
            None
        );
        assert_eq!(builtin_profile(None, ProfileId::Dev, &DEV_BUILD), None);
    }

    #[test]
    fn each_profile_keeps_its_own_folder() {
        let base = Path::new("/settings/omoba-bevy-dev");
        let dev = resolve(Some("dev"), None, &DEV_BUILD);
        let local = local_profile();
        let a = custom_profile("dev", &custom("192.168.1.20:4000", None), &DEV_BUILD).unwrap();
        let b = custom_profile("dev", &custom("192.168.1.21:4000", None), &DEV_BUILD).unwrap();
        let a_again =
            custom_profile("dev", &custom(" 192.168.1.20:4000 ", Some("")), &DEV_BUILD).unwrap();
        assert_eq!(profile_directory_in(base, &dev), base.join("profiles/dev"));
        assert_eq!(
            profile_directory_in(base, &local),
            base.join("profiles/local")
        );
        assert_eq!(
            a.storage, a_again.storage,
            "the same server keeps its folder"
        );
        assert_ne!(a.storage, b.storage, "another custom server gets another");
        let name = a.storage.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(name.starts_with("custom-") && name.len() == "custom-".len() + 12);
        assert!(name.bytes().skip(7).all(|b| b.is_ascii_hexdigit()));
        let folders: std::collections::BTreeSet<_> = [&dev, &local, &a, &b]
            .iter()
            .map(|profile| profile_directory_in(base, profile))
            .collect();
        assert_eq!(folders.len(), 4);
        // Production keeps the settings folder itself.
        let production = resolve(None, None, &Compiled::default());
        assert_eq!(
            profile_directory_in(Path::new("/settings/omoba-bevy"), &production),
            Path::new("/settings/omoba-bevy")
        );
    }

    fn temporary(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let directory = std::env::temp_dir().join(format!(
            "omoba-profiles-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn the_selection_round_trips_and_a_broken_file_is_ignored() {
        let directory = temporary("selection");
        let path = directory.join(SELECTION_FILE);
        assert_eq!(load_selection(&path), None);
        let saved = Selection {
            profile: ProfileId::Custom,
            custom: Some(custom(
                "192.168.1.20:4000",
                Some("https://registry.lan.example"),
            )),
        };
        save_selection(&path, &saved).unwrap();
        assert_eq!(load_selection(&path), Some(saved));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"custom\""), "{text}");
        save_selection(
            &path,
            &Selection {
                profile: ProfileId::Local,
                custom: None,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            "{\n  \"profile\": \"local\"\n}"
        );
        for broken in ["", "{", "{\"profile\":\"staging\"}", "[]"] {
            std::fs::write(&path, broken).unwrap();
            assert_eq!(load_selection(&path), None, "{broken}");
        }
        std::fs::write(&path, " ".repeat(5000)).unwrap();
        assert_eq!(load_selection(&path), None);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn choosing_saves_for_the_next_start_and_the_running_profile_cancels() {
        let directory = temporary("choose");
        let path = directory.join(SELECTION_FILE);
        let mut connection = ActiveConnection::for_contour(Some("dev"), Some(path.clone()));
        assert!(connection.selectable());
        assert_eq!(connection.profile.id, ProfileId::Dev);
        let local = connection
            .candidate(ProfileId::Local, &CustomEndpoints::default())
            .unwrap();
        connection.choose(local.clone()).unwrap();
        assert_eq!(connection.pending.as_ref(), Some(&local));
        assert_eq!(connection.next().id, ProfileId::Local);
        assert_eq!(
            connection.profile.id,
            ProfileId::Dev,
            "nothing switches before a restart"
        );
        assert_eq!(load_selection(&path).unwrap().profile, ProfileId::Local);
        let dev = connection
            .candidate(ProfileId::Dev, &CustomEndpoints::default())
            .unwrap();
        connection.choose(dev).unwrap();
        assert_eq!(connection.pending, None);
        assert_eq!(load_selection(&path).unwrap().profile, ProfileId::Dev);
        let typed = custom("192.168.1.20:4000", None);
        let chosen = connection.candidate(ProfileId::Custom, &typed).unwrap();
        connection.choose(chosen).unwrap();
        assert_eq!(
            connection
                .custom
                .as_ref()
                .map(|custom| custom.lobby.as_str()),
            Some("192.168.1.20:4000")
        );
        // The next start runs exactly what was saved.
        let saved = load_selection(&path).unwrap();
        assert_eq!(
            resolve(Some("dev"), Some(&saved), &connection.compiled),
            *connection.pending.as_ref().unwrap()
        );
        assert_eq!(
            connection.candidate(ProfileId::Custom, &custom("77.246.105.57:4000", None)),
            Err(ProfileError::Production)
        );
        assert_eq!(
            connection.candidate(ProfileId::Production, &CustomEndpoints::default()),
            Err(ProfileError::Production)
        );
        let mut nowhere = ActiveConnection::for_contour(Some("dev"), None);
        assert!(nowhere.choose(local_profile()).is_err());
        let production = ActiveConnection::for_contour(None, None);
        assert!(!production.selectable());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn the_dev_profile_adopts_a_store_saved_before_profiles() {
        let base = temporary("adopt");
        let dev = resolve(Some("dev"), None, &DEV_BUILD);
        std::fs::create_dir_all(base.join("ekza-store/account")).unwrap();
        std::fs::write(base.join("ekza-store/account/session.json"), b"{}").unwrap();
        assert!(
            !adopt_legacy_store(&base, &local_profile()),
            "only Dev adopts it"
        );
        assert!(adopt_legacy_store(&base, &dev));
        assert!(!base.join("ekza-store").exists());
        assert!(
            base.join("profiles/dev/ekza-store/account/session.json")
                .is_file()
        );
        // Never over an existing store.
        std::fs::create_dir_all(base.join("ekza-store")).unwrap();
        assert!(!adopt_legacy_store(&base, &dev));
        assert!(base.join("ekza-store").is_dir());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn this_build_runs_its_contours_default_profile() {
        // Tests never read a selection file, so this process runs the default
        // whatever the developer chose in their own dev app.
        let profile = active();
        let default = if BUILD_CONTOUR.is_some() {
            ProfileId::Dev
        } else {
            ProfileId::Production
        };
        assert_eq!(profile.id, default);
        assert_eq!(profile, &resolve(BUILD_CONTOUR, None, &COMPILED));
        assert!(saved_choice(selection_path().as_deref()).is_none());
        assert_eq!(ActiveConnection::current().profile, *profile);
        assert!(crate::persistence::validate_game_server_addr(&profile.lobby).is_some());
        assert_eq!(
            crate::session_config::default_server_addr(),
            profile.lobby.as_str()
        );
    }

    #[test]
    fn a_dev_build_ignores_a_production_registry_or_passport_override() {
        let dev = Some("dev");
        for url in [
            "https://registry.dev.ekza.io",
            "http://127.0.0.1:8018",
            "http://[::1]:8018",
            "https://registry.lan.example:8443/",
            "http://127.0.0.1:9/api/passport",
        ] {
            assert!(override_allowed(dev, url), "{url}");
        }
        for url in [
            "https://registry.ekza.io",
            "https://avatar.ekza.io/api/passport",
            "https://user@registry.ekza.io",
            "http://77.246.105.57:8018",
            "registry.dev.ekza.io",
            "",
        ] {
            assert!(!override_allowed(dev, url), "{url}");
        }
        // Developers and self-hosters of a production build point it anywhere.
        for url in ["https://registry.ekza.io", "https://registry.example.org"] {
            assert!(override_allowed(None, url), "{url}");
        }
    }

    #[test]
    fn phones_take_no_environment_override() {
        let expected = !cfg!(any(target_os = "android", target_os = "ios"));
        // PATH is set in every test environment.
        assert_eq!(desktop_override("PATH").is_some(), expected);
        assert_eq!(desktop_override("OMOBA_TEST_SURELY_UNSET_VARIABLE"), None);
    }
}
