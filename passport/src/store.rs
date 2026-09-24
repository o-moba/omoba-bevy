//! Omoba's runtime view of the Ekza avatar store, on top of the SDK
//! [`AvatarStore`]. Engine-free so it is testable without a window:
//!
//! - the approved catalogue is registered into `shared`'s roster (from the
//!   offline copy immediately, from the registry in the background);
//! - a store avatar is installed on first use, off the main thread, and only
//!   after the SDK byte checks and Omoba's humanoid profile check pass;
//! - the render layer polls [`model_state`] and [`take_changed`]; it never
//!   blocks on the network and never loads an unverified file.
//!
//! Nothing here grants an entitlement. Wearing a store avatar still requires a
//! passport ticket that the game server consumes.

use ekza_bevy_sdk::{
    cache::AssetCache,
    store::{AvatarStore, StoreAvatar},
};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

/// Operator/developer override of the public registry origin.
pub const REGISTRY_URL_ENV: &str = "OMOBA_REGISTRY_URL";
/// Bevy asset source name the client mounts [`root`] under.
pub const ASSET_SOURCE: &str = "ekza";
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const FAILED_INSTALL_RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelState {
    /// Installed and verified in this process; safe to load.
    Ready,
    /// Download or verification in flight; show the fallback model.
    Pending,
    /// Unknown slug, no store, or the last attempt failed (retried later).
    Unavailable,
}

/// Safe, non-sensitive catalogue state for every avatar picker.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CatalogueStatus {
    Loading { cached: usize },
    Empty,
    Ready { count: usize },
    Unavailable { cached: usize },
}

impl CatalogueStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Loading { .. } => "Loading approved Studio avatars…",
            Self::Empty => "No Studio avatars approved for OMOBA yet",
            Self::Ready { .. } => "Approved free avatars and your saved or purchased library",
            Self::Unavailable { cached: 0 } => "Studio is unavailable · refresh to try again",
            Self::Unavailable { .. } => "Studio is unavailable · showing the saved catalogue",
        }
    }
}

pub fn catalogue_status() -> CatalogueStatus {
    let Some(runtime) = runtime() else {
        return CatalogueStatus::Unavailable { cached: 0 };
    };
    status_for(&runtime.state.lock().unwrap())
}

fn status_for(state: &State) -> CatalogueStatus {
    let count = state.items.len();
    if state.refreshing || state.last_refresh.is_none() {
        CatalogueStatus::Loading { cached: count }
    } else if state.refresh_failed {
        CatalogueStatus::Unavailable { cached: count }
    } else if count == 0 {
        CatalogueStatus::Empty
    } else {
        CatalogueStatus::Ready { count }
    }
}

/// Current accepted catalogue, excluding entries removed by a successful refresh.
/// Shared definitions stay immutable for in-flight renderers; picker membership
/// is derived from this list rather than that append-only definition registry.
pub fn catalogue_slugs() -> Vec<String> {
    let mut slugs = runtime().map_or_else(Vec::new, |runtime| {
        runtime
            .state
            .lock()
            .unwrap()
            .items
            .keys()
            .cloned()
            .collect()
    });
    slugs.sort();
    slugs
}

/// Owned display snapshots; shared model identities remain immutable.
pub fn catalogue_definitions() -> Vec<shared::AvatarDefinition> {
    runtime().map_or_else(Vec::new, |runtime| {
        runtime
            .state
            .lock()
            .unwrap()
            .definitions
            .values()
            .cloned()
            .collect()
    })
}

/// Current access hint from the approved catalogue, independent of an older
/// immutable shared definition. The game server still decides admission.
pub fn free_access(slug: &str) -> Option<bool> {
    runtime().and_then(|runtime| {
        runtime
            .state
            .lock()
            .unwrap()
            .items
            .get(slug)
            .map(|item| item.free)
    })
}

enum Install {
    Pending,
    Ready,
    Failed(Instant),
}

#[derive(Default)]
struct State {
    items: HashMap<String, StoreAvatar>,
    definitions: HashMap<String, shared::AvatarDefinition>,
    installs: HashMap<String, Install>,
    changed: Vec<String>,
    refreshing: bool,
    last_refresh: Option<Instant>,
    refresh_failed: bool,
}

struct Runtime {
    store: AvatarStore,
    registry: String,
    state: Mutex<State>,
}

static RUNTIME: OnceLock<Option<Runtime>> = OnceLock::new();

fn runtime() -> Option<&'static Runtime> {
    RUNTIME.get().and_then(Option::as_ref)
}

/// Directory the client mounts as the `ekza://` asset source.
pub fn root() -> Option<&'static Path> {
    runtime().map(|runtime| runtime.store.root())
}

/// Asset path of an installed store model.
pub fn model_asset_path(slug: &str) -> String {
    format!("{ASSET_SOURCE}://avatars/{slug}.glb")
}

/// Asset path of a store thumbnail file name from an `AvatarDefinition`.
pub fn thumbnail_asset_path(file: &str) -> String {
    format!("{ASSET_SOURCE}://avatars/{file}")
}

/// Start the store under `root` (a private, writable per-user directory).
/// Registers the offline catalogue at once. `wait_for_catalogue` blocks on one
/// registry refresh so a freshly paired wallet sees its purchases on the first
/// menu; otherwise the refresh runs in the background.
pub fn initialize(root: PathBuf, wait_for_catalogue: bool) {
    let created = RUNTIME.get_or_init(|| {
        let registry = crate::community::registry_url();
        match AvatarStore::new(root, &registry, crate::selector()) {
            Ok(store) => Some(Runtime {
                store,
                registry,
                state: Mutex::default(),
            }),
            Err(error) => {
                eprintln!("Ekza avatar store unavailable: {error}");
                None
            }
        }
    });
    let Some(runtime) = created.as_ref() else {
        return;
    };
    adopt(runtime, runtime.store.cached());
    if wait_for_catalogue {
        refresh_now(runtime);
    } else {
        request_refresh();
    }
}

/// Ask for a catalogue refresh (rate limited, never blocks). Called when a
/// player shows up wearing a store slug this client has not heard of yet.
pub fn request_refresh() {
    request_refresh_after(REFRESH_INTERVAL, false);
}

/// Explicit user retry is still bounded and never starts parallel downloads.
pub fn request_user_refresh() {
    request_refresh_after(Duration::from_secs(2), true);
}

fn request_refresh_after(interval: Duration, retry_models: bool) {
    let Some(runtime) = runtime() else {
        return;
    };
    {
        let mut state = runtime.state.lock().unwrap();
        if state.refreshing || state.last_refresh.is_some_and(|at| at.elapsed() < interval) {
            return;
        }
        state.refreshing = true;
        if retry_models {
            state
                .installs
                .retain(|_, install| !matches!(install, Install::Failed(_)));
        }
    }
    std::thread::spawn(move || refresh_now(runtime));
}

fn refresh_now(runtime: &'static Runtime) {
    runtime.state.lock().unwrap().refreshing = true;
    let result = refresh_catalogue(runtime);
    let failed = result.is_err();
    match result {
        Ok(items) => adopt(runtime, items),
        Err(error) => eprintln!("Ekza avatar catalogue refresh failed: {error}"),
    }
    let mut state = runtime.state.lock().unwrap();
    state.refreshing = false;
    state.refresh_failed = failed;
    state.last_refresh = Some(Instant::now());
}

/// The locked SDK predates v2 outage semantics. Keep its parser, approved
/// template filter, cache format and verified installer, but do not downgrade
/// a failed/partial Studio catalogue into a successful empty v1 catalogue.
fn refresh_catalogue(runtime: &Runtime) -> Result<Vec<StoreAvatar>, String> {
    use ekza_bevy_sdk::{
        catalog::CatalogV2Response,
        registry::feed_url,
        store::{STORE_SCHEMA, templates_v2},
    };
    let mut url = feed_url(&runtime.registry).map_err(|_| "Invalid Studio origin")?;
    let path = format!("{}/v2/avatars", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.set_query(None);
    let selector = runtime.store.selector();
    url.query_pairs_mut()
        .append_pair("project", &selector.project_id)
        .append_pair("platform", &selector.platform)
        .append_pair("profile", &selector.profile);
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Studio connection unavailable")?;
    let response = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .map_err(|_| "Studio connection unavailable")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // Compatibility is permitted only for a registry without the v2 API.
        return runtime.store.refresh();
    }
    if !response.status().is_success()
        || response
            .headers()
            .get("x-studio-status")
            .is_some_and(|status| status.as_bytes().eq_ignore_ascii_case(b"unavailable"))
    {
        return Err("Studio catalogue temporarily unavailable".into());
    }
    const MAX_CATALOGUE_BYTES: u64 = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    response
        .take(MAX_CATALOGUE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Studio catalogue could not be read")?;
    if bytes.len() as u64 > MAX_CATALOGUE_BYTES {
        return Err("Studio catalogue exceeds the size limit".into());
    }
    let document: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Studio catalogue has an invalid format")?;
    if !document
        .get("items")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err("Studio catalogue is incomplete".into());
    }
    let catalogue: CatalogV2Response =
        serde_json::from_value(document).map_err(|_| "Studio catalogue has an invalid format")?;
    let items = templates_v2(&catalogue.items, selector);
    let persisted = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": STORE_SCHEMA, "avatars": &items,
    }))
    .map_err(|_| "Studio catalogue could not be saved")?;
    ekza_bevy_sdk::cache::atomic_write(&runtime.store.root().join("store.json"), &persisted)
        .map_err(|_| "Studio catalogue could not be saved")?;
    Ok(items)
}

/// Register catalogue items with the shared roster; new slugs are reported
/// through [`take_changed`] so already spawned players can pick them up.
fn adopt(runtime: &Runtime, items: Vec<StoreAvatar>) {
    let mut accepted = HashMap::new();
    let mut definitions = HashMap::new();
    let mut newly_known = Vec::new();
    for item in items {
        let was_known = runtime.state.lock().unwrap().items.contains_key(&item.slug);
        let thumbnail = thumbnail(&runtime.store, &item);
        let definition = shared::AvatarDefinition {
            slug: item.slug.clone(),
            display_name: item.name.clone(),
            collection: "Ekza store".into(),
            license: item
                .license
                .clone()
                .unwrap_or_else(|| "See creator terms".into()),
            source_url: item.protected.support.rendition.url.clone(),
            author: item.author.clone(),
            thumbnail,
            passport: Some(item.protected.clone()),
            free: item.free,
        };
        let registered = shared::register_store_avatar(definition.clone());
        // A slug the server registered first (same derivation) is equally fine.
        if registered.is_some_and(|entry| entry.passport.as_ref() == Some(&item.protected)) {
            if !was_known {
                newly_known.push(item.slug.clone());
            }
            definitions.insert(item.slug.clone(), definition);
            accepted.insert(item.slug.clone(), item);
        }
    }
    // Publish membership and notifications together: a renderer must never
    // drain a newly-known slug before model_state can resolve that slug.
    let mut state = runtime.state.lock().unwrap();
    state.items = accepted;
    state.definitions = definitions;
    state.changed.extend(newly_known);
}

/// Best-effort thumbnail next to the models; the extension follows the real
/// image container. Failure only means a text-only button.
fn thumbnail(store: &AvatarStore, item: &StoreAvatar) -> Option<String> {
    use sha2::{Digest, Sha256};
    let url = item.thumbnail_url.as_deref()?;
    let key = format!("{}-{:x}", item.slug, Sha256::digest(url.as_bytes()));
    let directory = store.root().join("avatars");
    for extension in ["png", "jpg", "webp"] {
        let name = format!("{key}.{extension}");
        if directory.join(&name).is_file() {
            return Some(name);
        }
    }
    let cache = AssetCache::new(store.root().join(".ekza-cache")).ok()?;
    let (file, kind) = cache.fetch_thumbnail(url).ok()?;
    let name = format!("{key}.{}", kind.extension());
    std::fs::create_dir_all(&directory).ok()?;
    std::fs::copy(&file.path, directory.join(&name)).ok()?;
    Some(name)
}

/// Whether `slug` belongs to the store runtime (as opposed to a roster entry
/// staged into the asset root by `passport-import`).
pub fn knows(slug: &str) -> bool {
    runtime().is_some_and(|runtime| runtime.state.lock().unwrap().items.contains_key(slug))
}

/// Poll the install state of a store model, starting the install on first use.
pub fn model_state(slug: &str) -> ModelState {
    let Some(runtime) = runtime() else {
        return ModelState::Unavailable;
    };
    let mut state = runtime.state.lock().unwrap();
    let Some(item) = state.items.get(slug).cloned() else {
        return ModelState::Unavailable;
    };
    match state.installs.get(slug) {
        Some(Install::Ready) => return ModelState::Ready,
        Some(Install::Pending) => return ModelState::Pending,
        Some(Install::Failed(at)) if at.elapsed() < FAILED_INSTALL_RETRY => {
            return ModelState::Unavailable;
        }
        _ => {}
    }
    state.installs.insert(slug.to_owned(), Install::Pending);
    drop(state);
    std::thread::spawn(move || {
        let outcome = install_blocking_inner(runtime, &item);
        if let Err(error) = &outcome {
            eprintln!(
                "Ekza avatar '{}' could not be installed: {error}",
                item.name
            );
        }
    });
    ModelState::Pending
}

/// Install synchronously. For worker threads that need the file before they
/// continue, such as the join ticket request.
pub fn install_blocking(slug: &str) -> Result<(), String> {
    let runtime = runtime().ok_or("Ekza avatar store is unavailable")?;
    let item = runtime
        .state
        .lock()
        .unwrap()
        .items
        .get(slug)
        .cloned()
        .ok_or("Avatar is not in the Ekza store catalogue")?;
    install_blocking_inner(runtime, &item)
}

fn install_blocking_inner(runtime: &Runtime, item: &StoreAvatar) -> Result<(), String> {
    let outcome = runtime
        .store
        .install(item, crate::validate_humanoid_profile)
        .map(drop);
    let mut state = runtime.state.lock().unwrap();
    let was_ready = matches!(state.installs.get(&item.slug), Some(Install::Ready));
    state.installs.insert(
        item.slug.clone(),
        match &outcome {
            Ok(()) => Install::Ready,
            Err(_) => Install::Failed(Instant::now()),
        },
    );
    if outcome.is_ok() && !was_ready {
        state.changed.push(item.slug.clone());
    }
    outcome
}

/// Slugs that became known or installed since the last call. The render layer
/// re-resolves the model of any player wearing one of them.
pub fn take_changed() -> Vec<String> {
    runtime().map_or_else(Vec::new, |runtime| {
        std::mem::take(&mut runtime.state.lock().unwrap().changed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_states_distinguish_loading_empty_and_failed_cache() {
        let mut state = State::default();
        assert_eq!(status_for(&state), CatalogueStatus::Loading { cached: 0 });
        state.last_refresh = Some(Instant::now());
        assert_eq!(status_for(&state), CatalogueStatus::Empty);
        state.refresh_failed = true;
        assert_eq!(
            status_for(&state),
            CatalogueStatus::Unavailable { cached: 0 }
        );
        state.refreshing = true;
        assert_eq!(status_for(&state), CatalogueStatus::Loading { cached: 0 });
        assert!(
            !CatalogueStatus::Unavailable { cached: 2 }
                .label()
                .contains("http")
        );
        assert!(
            CatalogueStatus::Unavailable { cached: 2 }
                .label()
                .contains("saved")
        );
    }
}

#[cfg(test)]
mod fixture_tests;
