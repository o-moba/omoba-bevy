//! Runtime handheld discovery and verified installation through the typed SDK.
//! Only server-authorized IDs travel over gameplay transport. Files live in the
//! writable Ekza source on every platform, never in the application bundle.
use crate::{
    store::{CatalogueStatus, ModelState},
    weapons::{Grip, Weapon},
};
use ekza_bevy_sdk::{
    assets::{AssetKind, AssetStore, StoreAsset, templates},
    passport::SupportSelector,
    registry::RegistryClient,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub fn selector() -> SupportSelector {
    SupportSelector {
        project_id: "omoba".into(),
        platform: "desktop".into(),
        profile: crate::weapons::PROFILE.into(),
        formats: vec!["glb".into()],
    }
}
/// Metadata-only lookup for authoritative servers: no model downloads.
pub fn fetch_approved(registry: &str) -> Result<Vec<StoreAsset>, String> {
    let entries = RegistryClient::new(registry)
        .map_err(|e| e.to_string())?
        .assets_v2(
            AssetKind::Weapon,
            Some("omoba"),
            Some(("desktop", crate::weapons::PROFILE)),
        )
        .map_err(|e| e.to_string())?;
    Ok(templates(&entries, AssetKind::Weapon, &selector()))
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
}
enum Install {
    Pending,
    Ready(Grip),
    Failed(Instant),
}
#[derive(Default)]
struct State {
    items: HashMap<String, StoreAsset>,
    installs: HashMap<String, Install>,
    refreshing: bool,
    attempted: Option<Instant>,
    failed: bool,
    revision: u64,
}
struct Runtime {
    store: AssetStore,
    state: Mutex<State>,
}
static RUNTIME: OnceLock<Option<Runtime>> = OnceLock::new();
fn runtime() -> Option<&'static Runtime> {
    RUNTIME.get().and_then(Option::as_ref)
}
pub fn initialize(root: PathBuf) {
    let Some(runtime) = RUNTIME
        .get_or_init(|| {
            AssetStore::new(
                root,
                &crate::community::registry_url(),
                AssetKind::Weapon,
                selector(),
            )
            .ok()
            .map(|store| Runtime {
                store,
                state: Mutex::default(),
            })
        })
        .as_ref()
    else {
        return;
    };
    adopt(runtime, runtime.store.cached());
    refresh(false);
}
fn adopt(runtime: &Runtime, entries: Vec<StoreAsset>) {
    let mut state = runtime.state.lock().unwrap();
    state.items = entries
        .into_iter()
        .map(|item| (item.slug.clone(), item))
        .collect();
    state.revision = state.revision.wrapping_add(1);
}
pub fn refresh(user: bool) {
    let Some(runtime) = runtime() else { return };
    {
        let mut state = runtime.state.lock().unwrap();
        if state.refreshing
            || state
                .attempted
                .is_some_and(|at| at.elapsed() < Duration::from_secs(if user { 2 } else { 30 }))
        {
            return;
        }
        state.refreshing = true;
        state.revision = state.revision.wrapping_add(1);
        if user {
            state
                .installs
                .retain(|_, state| !matches!(state, Install::Failed(_)));
        }
    }
    std::thread::spawn(move || {
        let result = runtime.store.refresh();
        let failed = result.is_err();
        if let Ok(items) = result {
            adopt(runtime, items);
        }
        let mut state = runtime.state.lock().unwrap();
        state.refreshing = false;
        state.failed = failed;
        state.attempted = Some(Instant::now());
        state.revision = state.revision.wrapping_add(1);
    });
}
pub fn snapshot() -> (u64, CatalogueStatus, Vec<Entry>) {
    let Some(runtime) = runtime() else {
        return (0, CatalogueStatus::Unavailable { cached: 0 }, vec![]);
    };
    let state = runtime.state.lock().unwrap();
    let count = state.items.len();
    let status = if state.refreshing || state.attempted.is_none() {
        CatalogueStatus::Loading { cached: count }
    } else if state.failed {
        CatalogueStatus::Unavailable { cached: count }
    } else if count == 0 {
        CatalogueStatus::Empty
    } else {
        CatalogueStatus::Ready { count }
    };
    let mut entries: Vec<_> = state
        .items
        .values()
        .map(|e| Entry {
            id: e.slug.clone(),
            name: e.name.clone(),
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    (state.revision, status, entries)
}
pub fn eligible(id: &str) -> bool {
    crate::weapons::catalog().items.iter().any(|w| w.id == id)
        || runtime().is_some_and(|r| r.state.lock().unwrap().items.contains_key(id))
}
pub fn model_state(id: &str) -> ModelState {
    if crate::weapons::catalog().items.iter().any(|w| w.id == id) {
        return ModelState::Ready;
    }
    let Some(runtime) = runtime() else {
        return ModelState::Unavailable;
    };
    let mut state = runtime.state.lock().unwrap();
    let Some(item) = state.items.get(id).cloned() else {
        drop(state);
        refresh(false);
        return ModelState::Unavailable;
    };
    match state.installs.get(id) {
        Some(Install::Ready(_)) => return ModelState::Ready,
        Some(Install::Pending) => return ModelState::Pending,
        Some(Install::Failed(at)) if at.elapsed() < Duration::from_secs(60) => {
            return ModelState::Unavailable;
        }
        _ => {}
    }
    state.installs.insert(id.into(), Install::Pending);
    drop(state);
    std::thread::spawn(move || {
        let result = runtime
            .store
            .install(&item, |bytes| {
                crate::weapons::validate_model(bytes).map(drop)
            })
            .and_then(|path| std::fs::read(path).map_err(|e| e.to_string()))
            .and_then(|bytes| crate::weapons::validate_model(&bytes));
        let mut state = runtime.state.lock().unwrap();
        state.installs.insert(
            item.slug,
            match result {
                Ok(grip) => Install::Ready(grip),
                Err(_) => Install::Failed(Instant::now()),
            },
        );
        state.revision = state.revision.wrapping_add(1);
    });
    ModelState::Pending
}
pub fn definition(id: &str) -> Option<Weapon> {
    if let Some(def) = crate::weapons::catalog().items.iter().find(|w| w.id == id) {
        return Some(def.clone());
    }
    if model_state(id) != ModelState::Ready {
        return None;
    }
    let state = runtime()?.state.lock().unwrap();
    let item = state.items.get(id)?;
    let Install::Ready(grip) = state.installs.get(id)? else {
        return None;
    };
    Some(Weapon {
        id: id.into(),
        name: item.name.clone(),
        model: format!("ekza://weapons/{id}.glb"),
        grip: grip.clone(),
        source_id: Some(item.asset_id.clone()),
        sha256: Some(item.support.rendition.sha256.clone()),
    })
}
