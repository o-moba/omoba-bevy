//! Free Ekza avatars a game server may admit with no ownership proof.
//!
//! An avatar published through Ekza Studio, prepared for Omoba and approved by an
//! Omoba project owner is listed by the registry's unified catalogue with
//! `access: "free"`. The server reads that list itself: a client can never make an
//! avatar free by saying so. The slug is the SDK hash of identity and rendition, so
//! an entry pins exact bytes just like a purchased avatar does.

use ekza_bevy_sdk::{
    registry::{DEFAULT_REGISTRY_URL, RegistryClient},
    store::{StoreAvatar, templates_v2},
};

/// The registry a client's connection profile chose, set once at startup
/// before any store or account call ([`set_registry_url`]).
static CHOSEN_REGISTRY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The registry origin compiled into this build (`OMOBA_DEFAULT_REGISTRY_URL`,
/// set by `scripts/release.py`), if any.
pub const COMPILED_REGISTRY_URL: Option<&str> = option_env!("OMOBA_DEFAULT_REGISTRY_URL");

/// The registry this build was compiled for, else the public Ekza registry.
pub fn compiled_registry_url() -> &'static str {
    COMPILED_REGISTRY_URL.unwrap_or(DEFAULT_REGISTRY_URL)
}

/// Pins the registry for this process. The game client calls it once, before
/// the stores start, with its connection profile's registry (which already
/// folds in the desktop `OMOBA_REGISTRY_URL` override). Returns false when a
/// registry was already chosen; the first choice stays.
pub fn set_registry_url(url: String) -> bool {
    CHOSEN_REGISTRY.set(url).is_ok()
}

/// Registry origin: the client's chosen registry, else the operator override
/// (`OMOBA_REGISTRY_URL`, the game server and tools), else the compiled one.
pub fn registry_url() -> String {
    if let Some(chosen) = CHOSEN_REGISTRY.get() {
        return chosen.clone();
    }
    std::env::var(crate::store::REGISTRY_URL_ENV)
        .unwrap_or_else(|_| compiled_registry_url().to_owned())
}

/// Avatars approved for Omoba that are explicitly free. One blocking request.
pub fn fetch_free(registry: &str) -> Result<Vec<StoreAvatar>, String> {
    let selector = crate::selector();
    let client = RegistryClient::new(registry).map_err(|error| error.to_string())?;
    let items = client
        .catalog_v2(
            Some(&selector.project_id),
            Some((&selector.platform, &selector.profile)),
        )
        .map_err(|error| error.to_string())?;
    Ok(free_only(templates_v2(&items, &selector)))
}

/// Owned entries never pass, whatever else the feed contains.
pub fn free_only(items: Vec<StoreAvatar>) -> Vec<StoreAvatar> {
    items.into_iter().filter(|item| item.free).collect()
}
