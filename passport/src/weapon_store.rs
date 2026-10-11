//! Runtime handheld discovery and verified installation through the typed SDK.
//! Only server-authorized IDs travel over gameplay transport. Files live in the
//! writable Ekza source on every platform, never in the application bundle.
//!
//! Two sources list SDK weapons: the public catalogue (`/v2/assets`) and the
//! connected player's Ekza library (`/v1/account/library`, see
//! [`set_library`]). The library only decides what is listed first as
//! "My Ekza library"; both install through the same verified store, and the
//! server still admits a free weapon from its own registry read.
use crate::{
    account::AccountSession,
    store::{CatalogueStatus, ModelState},
    weapons::{Grip, Weapon, WildsparkSetHint},
};
use ekza_bevy_sdk::{
    assets::{AssetKind, AssetStore, StoreAsset, templates},
    passport::SupportSelector,
    registry::RegistryClient,
};
use std::{
    collections::{HashMap, HashSet},
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub license: Option<String>,
    /// Listed in the connected player's Ekza library.
    pub owned: bool,
}
impl Entry {
    fn of(item: &StoreAsset, owned: bool) -> Self {
        Self {
            id: item.slug.clone(),
            name: item.name.clone(),
            author: item.author.clone(),
            license: item.license.clone(),
            owned,
        }
    }
}
/// Weapons the account library lists, under the catalogue's own selector.
pub fn owned(session: &AccountSession) -> Vec<StoreAsset> {
    session.assets(AssetKind::Weapon, &selector())
}
enum Install {
    Pending,
    Ready(Grip, Option<WildsparkSetHint>),
    Failed(Instant),
}
#[derive(Default)]
struct State {
    items: HashMap<String, StoreAsset>,
    /// The account library's weapons, in library order; empty when signed out.
    library: Vec<StoreAsset>,
    installs: HashMap<String, Install>,
    refreshing: bool,
    attempted: Option<Instant>,
    failed: bool,
    revision: u64,
}
impl State {
    /// A catalogue item, else a library item the catalogue has not listed yet.
    fn item(&self, id: &str) -> Option<&StoreAsset> {
        self.items
            .get(id)
            .or_else(|| self.library.iter().find(|item| item.slug == id))
    }
    /// "My Ekza library" first, then the public catalogue without the assets
    /// already listed there. One entry per asset id, so a library revision
    /// that differs from the cached catalogue's never shows twice.
    fn entries(&self) -> Vec<Entry> {
        let sort = |entries: &mut Vec<Entry>| {
            entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        };
        let mut assets = HashSet::new();
        let mut owned: Vec<_> = self
            .library
            .iter()
            .filter(|item| assets.insert(item.asset_id.as_str()))
            .map(|item| Entry::of(item, true))
            .collect();
        sort(&mut owned);
        let mut public: Vec<_> = self
            .items
            .values()
            .filter(|item| !assets.contains(item.asset_id.as_str()))
            .map(|item| Entry::of(item, false))
            .collect();
        sort(&mut public);
        owned.append(&mut public);
        owned
    }
    /// Replaces the library. `None` when nothing changed, else whether it
    /// lists a weapon the cached catalogue does not know yet.
    fn set_library(&mut self, items: Vec<StoreAsset>) -> Option<bool> {
        let items: Vec<_> = items
            .into_iter()
            .filter(|item| item.asset_kind == AssetKind::Weapon)
            .collect();
        if self.library == items {
            return None;
        }
        let unknown = items
            .iter()
            .any(|item| !self.items.contains_key(&item.slug));
        self.library = items;
        self.revision = self.revision.wrapping_add(1);
        Some(unknown)
    }
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
/// The connected account's library weapons, or none after sign-out. Called on
/// every library read (about every 15 s); only a change bumps the revision.
/// A newly listed weapon also asks the public catalogue to catch up.
pub fn set_library(items: Vec<StoreAsset>) {
    let Some(runtime) = runtime() else { return };
    let changed = runtime.state.lock().unwrap().set_library(items);
    if changed == Some(true) {
        refresh(false);
    }
}
/// Status describes the public catalogue; entries list the library first.
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
    (state.revision, status, state.entries())
}
pub fn eligible(id: &str) -> bool {
    crate::weapons::catalog().items.iter().any(|w| w.id == id)
        || runtime().is_some_and(|r| r.state.lock().unwrap().item(id).is_some())
}
pub fn model_state(id: &str) -> ModelState {
    if crate::weapons::catalog().items.iter().any(|w| w.id == id) {
        return ModelState::Ready;
    }
    let Some(runtime) = runtime() else {
        return ModelState::Unavailable;
    };
    let mut state = runtime.state.lock().unwrap();
    let Some(item) = state.item(id).cloned() else {
        drop(state);
        refresh(false);
        return ModelState::Unavailable;
    };
    match state.installs.get(id) {
        Some(Install::Ready(..)) => return ModelState::Ready,
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
            .and_then(|bytes| {
                crate::weapons::validate_model(&bytes)
                    .map(|grip| (grip, crate::weapons::wildspark_set_hint(&bytes)))
            });
        let mut state = runtime.state.lock().unwrap();
        state.installs.insert(
            item.slug,
            match result {
                Ok((grip, hint)) => Install::Ready(grip, hint),
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
    let item = state.item(id)?;
    let Install::Ready(grip, _) = state.installs.get(id)? else {
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
/// The optional set hint embedded in an installed SDK model. Packaged props have
/// none; the client then uses the baseline muzzle of the shown variant.
pub fn wildspark_set_hint(id: &str) -> Option<WildsparkSetHint> {
    match runtime()?.state.lock().unwrap().installs.get(id)? {
        Install::Ready(_, hint) => *hint,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekza_bevy_sdk::{
        assets::asset_slug,
        passport::{ProjectSupport, Rendition},
    };

    fn weapon(uuid: &str, name: &str, sha: char) -> StoreAsset {
        let id = format!("ekza:weapon:{uuid}");
        let sha256 = sha.to_string().repeat(64);
        StoreAsset {
            slug: asset_slug(AssetKind::Weapon, &id, &sha256),
            asset_id: id,
            asset_kind: AssetKind::Weapon,
            name: name.into(),
            author: Some(format!("{name} maker")),
            license: Some("CC-BY-4.0".into()),
            thumbnail_url: None,
            support: ProjectSupport {
                project_id: "omoba".into(),
                platform: "desktop".into(),
                profile: crate::weapons::PROFILE.into(),
                status: "approved".into(),
                rendition: Rendition {
                    id: format!("sha256:{sha256}"),
                    url: format!("https://registry.ekza.io/v1/studio/assets/{sha256}/rendition"),
                    sha256,
                    size_bytes: 128,
                    format: "glb".into(),
                },
            },
            free: true,
        }
    }
    const HAMMER: &str = "1c366765-3f44-4d36-b14c-acaba3bfce53";
    const SET: &str = "11111111-2222-4333-8444-555555555555";
    const AXE: &str = "22222222-3333-4444-8555-666666666666";

    fn catalogue(items: &[StoreAsset]) -> State {
        State {
            items: items.iter().map(|i| (i.slug.clone(), i.clone())).collect(),
            ..State::default()
        }
    }
    fn names(entries: &[Entry]) -> Vec<(&str, bool)> {
        entries.iter().map(|e| (e.name.as_str(), e.owned)).collect()
    }

    #[test]
    fn the_library_lists_first_and_an_owned_asset_never_repeats_in_the_catalogue() {
        let (hammer, set, axe) = (
            weapon(HAMMER, "Forge Hammer", 'b'),
            weapon(SET, "Wildspark set", 'c'),
            weapon(AXE, "Axe", 'd'),
        );
        let mut state = catalogue(&[hammer.clone(), set.clone(), axe.clone()]);
        // Signed out: today's catalogue, alphabetical, nothing owned.
        assert_eq!(
            names(&state.entries()),
            [
                ("Axe", false),
                ("Forge Hammer", false),
                ("Wildspark set", false)
            ]
        );
        assert_eq!(state.set_library(vec![set.clone()]), Some(false));
        let entries = state.entries();
        assert_eq!(
            names(&entries),
            [
                ("Wildspark set", true),
                ("Axe", false),
                ("Forge Hammer", false)
            ]
        );
        assert_eq!(entries[0].id, set.slug);
        assert_eq!(entries[0].author.as_deref(), Some("Wildspark set maker"));
        assert_eq!(entries[0].license.as_deref(), Some("CC-BY-4.0"));
        // A newer library revision of a catalogue asset replaces it, not joins it.
        let newer = weapon(HAMMER, "Forge Hammer II", 'e');
        assert_ne!(newer.slug, hammer.slug);
        assert_eq!(
            state.set_library(vec![set.clone(), newer.clone()]),
            Some(true)
        );
        assert_eq!(
            names(&state.entries()),
            [
                ("Forge Hammer II", true),
                ("Wildspark set", true),
                ("Axe", false)
            ]
        );
        // Sign-out restores the public group exactly.
        assert_eq!(state.set_library(Vec::new()), Some(false));
        assert_eq!(state.entries().len(), 3);
        assert!(state.entries().iter().all(|e| !e.owned));
    }

    #[test]
    fn library_only_weapons_list_and_resolve_before_the_catalogue_catches_up() {
        let hammer = weapon(HAMMER, "Forge Hammer", 'b');
        let claimed = weapon(SET, "Wildspark set", 'c');
        let mut state = catalogue(std::slice::from_ref(&hammer));
        assert!(state.item(&claimed.slug).is_none());
        // A web claim of a weapon approved after the last catalogue read.
        assert_eq!(state.set_library(vec![claimed.clone()]), Some(true));
        assert_eq!(state.item(&claimed.slug), Some(&claimed));
        assert_eq!(state.item(&hammer.slug), Some(&hammer));
        assert_eq!(
            names(&state.entries()),
            [("Wildspark set", true), ("Forge Hammer", false)]
        );
        // Repeated revisions in the library collapse to the first listed.
        let other = weapon(SET, "Wildspark set (android)", 'f');
        state.set_library(vec![claimed.clone(), other]);
        assert_eq!(state.entries().iter().filter(|e| e.owned).count(), 1);
    }

    #[test]
    fn an_unchanged_library_read_keeps_the_revision() {
        let set = weapon(SET, "Wildspark set", 'c');
        let mut state = catalogue(std::slice::from_ref(&set));
        assert_eq!(state.set_library(Vec::new()), None);
        assert_eq!(state.revision, 0);
        assert_eq!(state.set_library(vec![set.clone()]), Some(false));
        let revision = state.revision;
        // The 15 s session refresh re-sends the same list.
        assert_eq!(state.set_library(vec![set.clone()]), None);
        assert_eq!(state.revision, revision);
        // Only weapons are kept, whatever the caller passes.
        let mut avatar = set.clone();
        avatar.asset_kind = AssetKind::Avatar;
        assert_eq!(state.set_library(vec![avatar]), Some(false));
        assert!(state.library.is_empty());
    }
}
