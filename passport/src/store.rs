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
    registry::DEFAULT_REGISTRY_URL,
    store::{AvatarStore, StoreAvatar},
};
use std::{
    collections::HashMap,
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

enum Install {
    Pending,
    Ready,
    Failed(Instant),
}

#[derive(Default)]
struct State {
    items: HashMap<String, StoreAvatar>,
    installs: HashMap<String, Install>,
    changed: Vec<String>,
    refreshing: bool,
    last_refresh: Option<Instant>,
}

struct Runtime {
    store: AvatarStore,
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
        let registry =
            std::env::var(REGISTRY_URL_ENV).unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_owned());
        match AvatarStore::new(root, &registry, crate::selector()) {
            Ok(store) => Some(Runtime {
                store,
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
    let Some(runtime) = runtime() else {
        return;
    };
    {
        let mut state = runtime.state.lock().unwrap();
        if state.refreshing
            || state
                .last_refresh
                .is_some_and(|at| at.elapsed() < REFRESH_INTERVAL)
        {
            return;
        }
        state.refreshing = true;
    }
    std::thread::spawn(move || refresh_now(runtime));
}

fn refresh_now(runtime: &'static Runtime) {
    runtime.state.lock().unwrap().refreshing = true;
    let result = runtime.store.refresh();
    match result {
        Ok(items) => adopt(runtime, items),
        Err(error) => eprintln!("Ekza avatar catalogue refresh failed: {error}"),
    }
    let mut state = runtime.state.lock().unwrap();
    state.refreshing = false;
    state.last_refresh = Some(Instant::now());
}

/// Register catalogue items with the shared roster; new slugs are reported
/// through [`take_changed`] so already spawned players can pick them up.
fn adopt(runtime: &Runtime, items: Vec<StoreAvatar>) {
    for item in items {
        if runtime.state.lock().unwrap().items.contains_key(&item.slug) {
            continue;
        }
        let thumbnail = thumbnail(&runtime.store, &item);
        let registered = shared::register_store_avatar(shared::AvatarDefinition {
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
        });
        // A slug the server registered first (same derivation) is equally fine.
        if registered.is_some_and(|entry| entry.passport.as_ref() == Some(&item.protected)) {
            let mut state = runtime.state.lock().unwrap();
            state.changed.push(item.slug.clone());
            state.items.insert(item.slug.clone(), item);
        }
    }
}

/// Best-effort thumbnail next to the models; the extension follows the real
/// image container. Failure only means a text-only button.
fn thumbnail(store: &AvatarStore, item: &StoreAvatar) -> Option<String> {
    let directory = store.root().join("avatars");
    for extension in ["png", "jpg", "webp"] {
        let name = format!("{}.{extension}", item.slug);
        if directory.join(&name).is_file() {
            return Some(name);
        }
    }
    let url = item.thumbnail_url.as_deref()?;
    let cache = AssetCache::new(store.root().join(".ekza-cache")).ok()?;
    let (file, kind) = cache.fetch_thumbnail(url).ok()?;
    let name = format!("{}.{}", item.slug, kind.extension());
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
