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

/// Registry origin: the same operator override the client store uses.
pub fn registry_url() -> String {
    std::env::var(crate::store::REGISTRY_URL_ENV)
        .unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_owned())
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
