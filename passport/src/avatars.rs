//! The avatar roster and the process-wide Ekza store avatar registry.
//!
//! Moved out of `shared` (step 13): the roster is data loaded at runtime from
//! the packaged asset root (or an operator override), and the registry is
//! process-global state. The binaries read [`RosterSource::from_env`] at
//! startup and call [`init_avatar_roster`]; [`load_roster`] itself is a pure
//! function over candidate paths and a reader.

use ekza_bevy_sdk::passport::{ProtectedAvatar, protected_slug};
use serde::{Deserialize, Serialize};
use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

/// Operator override of the live avatar manifest.
pub const AVATAR_MANIFEST_ENV: &str = "OMOBA_AVATAR_MANIFEST";

/// One shipped avatar. All roster avatars are CC0 VRM models staged as GLB
/// under `client/assets/avatars/<slug>.glb` with retargeted animation clips.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarDefinition {
    pub slug: String,
    pub display_name: String,
    pub collection: String,
    pub license: String,
    pub source_url: String,
    #[serde(default)]
    pub author: Option<String>,
    /// Thumbnail file name relative to `client/assets/avatars/`, if shipped.
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// Explicit paid-cosmetic boundary. Never inferred from URL or format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passport: Option<ProtectedAvatar>,
    /// The Ekza registry marked this store avatar free: the boundary above still
    /// pins the exact rendition, but no ticket is needed to wear it. This is a
    /// presentation hint. The game server decides from its own registry read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub free: bool,
}

#[derive(Debug, Deserialize)]
struct AvatarManifest {
    avatars: Vec<AvatarDefinition>,
}

/// The committed roster manifest, embedded at compile time as the fallback
/// when no manifest file is found on disk.
const AVATAR_MANIFEST_JSON: &str = include_str!("../../client/assets/avatars/manifest.json");

/// Where the live roster manifest is looked up. Tools like `arena-sync`
/// append avatars to the file on disk, so the file (not the embedded copy) is
/// the source of truth when present. Client and server must see the same file
/// or joins with runtime-added avatars will be rejected by the server's slug
/// validation. There is no lookup relative to the working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterSource {
    /// `OMOBA_AVATAR_MANIFEST`, tried first.
    pub manifest_override: Option<PathBuf>,
    /// The packaged asset root; `avatars/manifest.json` under it is tried next.
    pub asset_root: PathBuf,
}

impl RosterSource {
    /// Read the override and the asset root from the process environment.
    pub fn from_env() -> Self {
        Self {
            manifest_override: std::env::var_os(AVATAR_MANIFEST_ENV).map(PathBuf::from),
            asset_root: crate::assets::client_asset_root(),
        }
    }

    /// Manifest files to try, in order.
    pub fn candidates(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::with_capacity(2);
        candidates.extend(self.manifest_override.clone());
        candidates.push(self.asset_root.join("avatars/manifest.json"));
        candidates
    }
}

/// Where the loaded roster came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterOrigin {
    File(PathBuf),
    Embedded,
}

impl fmt::Display for RosterOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Embedded => f.write_str("embedded manifest"),
        }
    }
}

/// A loaded roster with its origin and the problems met on the way.
#[derive(Debug, Clone)]
pub struct LoadedRoster {
    pub avatars: Vec<AvatarDefinition>,
    pub origin: RosterOrigin,
    /// Unreadable-manifest and skipped-entry messages, in order.
    pub warnings: Vec<String>,
}

impl LoadedRoster {
    /// The one startup line the binaries print.
    pub fn summary(&self) -> String {
        format!(
            "Avatar roster: {} avatars from {}",
            self.avatars.len(),
            self.origin
        )
    }
}

/// Why a roster entry breaks the store-avatar rule, if it does. An entry that
/// looks protected (an `ekza-` slug) or carries a passport boundary must be the
/// SDK-derived slug of a valid boundary, exactly as [`register_store_avatar`]
/// requires; otherwise it could pass as a free cosmetic or shadow a store slug.
fn roster_entry_problem(avatar: &AvatarDefinition) -> Option<&'static str> {
    match &avatar.passport {
        None if avatar.slug.starts_with("ekza-") => {
            Some("protected-looking slug without a passport boundary")
        }
        None => None,
        Some(protected) if protected.validate().is_err() => Some("invalid passport boundary"),
        Some(protected) if protected_slug(protected) != avatar.slug => {
            Some("slug is not the derived name of its passport boundary")
        }
        Some(_) => None,
    }
}

fn parse_manifest(
    raw: &str,
    origin: &RosterOrigin,
    warnings: &mut Vec<String>,
) -> Result<Vec<AvatarDefinition>, serde_json::Error> {
    let manifest = serde_json::from_str::<AvatarManifest>(raw)?;
    Ok(manifest
        .avatars
        .into_iter()
        .filter(|avatar| match roster_entry_problem(avatar) {
            None => true,
            Some(problem) => {
                warnings.push(format!(
                    "avatar manifest {origin}: skipping {:?} ({problem})",
                    avatar.slug
                ));
                false
            }
        })
        .collect())
}

/// Load the roster: the first readable and parseable candidate wins, the
/// embedded manifest is the fallback. Entries that break the store-avatar rule
/// are skipped with a warning. Pure apart from `read`.
pub fn load_roster(
    candidates: &[PathBuf],
    read: impl Fn(&Path) -> io::Result<String>,
) -> LoadedRoster {
    let mut warnings = Vec::new();
    for candidate in candidates {
        let Ok(raw) = read(candidate) else {
            continue;
        };
        let origin = RosterOrigin::File(candidate.clone());
        match parse_manifest(&raw, &origin, &mut warnings) {
            Ok(avatars) => {
                return LoadedRoster {
                    avatars,
                    origin,
                    warnings,
                };
            }
            Err(error) => warnings.push(format!(
                "avatar manifest {} is invalid ({error}); trying next candidate",
                candidate.display()
            )),
        }
    }
    let origin = RosterOrigin::Embedded;
    let avatars = parse_manifest(AVATAR_MANIFEST_JSON, &origin, &mut warnings)
        .expect("embedded avatar manifest must parse");
    LoadedRoster {
        avatars,
        origin,
        warnings,
    }
}

static ROSTER: OnceLock<LoadedRoster> = OnceLock::new();

fn load_and_warn(source: &RosterSource) -> LoadedRoster {
    let loaded = load_roster(&source.candidates(), |path| std::fs::read_to_string(path));
    for warning in &loaded.warnings {
        eprintln!("{warning}");
    }
    loaded
}

/// Load the process roster from `source` once. Binaries call this at startup
/// with [`RosterSource::from_env`] and print [`LoadedRoster::summary`]. A
/// roster already loaded (by an earlier call or a lookup) is returned as is.
pub fn init_avatar_roster(source: &RosterSource) -> &'static LoadedRoster {
    ROSTER.get_or_init(|| load_and_warn(source))
}

/// All shipped avatars, in manifest order. Falls back to loading from the
/// environment on first use (tests and tools that skip [`init_avatar_roster`]).
pub fn avatar_roster() -> &'static [AvatarDefinition] {
    &ROSTER
        .get_or_init(|| load_and_warn(&RosterSource::from_env()))
        .avatars
}

/// Ekza store avatars learned at runtime (catalogue refresh on the client, a
/// consumed ticket on the server). Entries are leaked once so lookups keep the
/// `'static` lifetime the roster API always had; the set is bounded.
static STORE_AVATARS: RwLock<Vec<&'static AvatarDefinition>> = RwLock::new(Vec::new());
const MAX_STORE_AVATARS: usize = 4096;

/// Register a purchasable Ekza avatar. Only a definition whose slug is the
/// SDK-derived name of its own passport boundary is accepted, so a registered
/// entry can never shadow a shipped slug or masquerade as a free cosmetic.
/// Returns the canonical entry (the first registration wins).
pub fn register_store_avatar(definition: AvatarDefinition) -> Option<&'static AvatarDefinition> {
    let protected = definition.passport.as_ref()?;
    if protected.validate().is_err()
        || protected_slug(protected) != definition.slug
        || avatar_roster()
            .iter()
            .any(|avatar| avatar.slug == definition.slug)
    {
        return None;
    }
    let mut store = STORE_AVATARS.write().unwrap();
    if let Some(existing) = store.iter().find(|avatar| avatar.slug == definition.slug) {
        return Some(existing);
    }
    if store.len() >= MAX_STORE_AVATARS {
        return None;
    }
    let entry: &'static AvatarDefinition = Box::leak(Box::new(definition));
    store.push(entry);
    Some(entry)
}

/// Store avatars registered so far, in registration order.
pub fn store_avatars() -> Vec<&'static AvatarDefinition> {
    STORE_AVATARS.read().unwrap().clone()
}

/// Looks up a shipped or registered store avatar by slug.
pub fn avatar_definition(slug: &str) -> Option<&'static AvatarDefinition> {
    avatar_roster()
        .iter()
        .find(|avatar| avatar.slug == slug)
        .or_else(|| {
            STORE_AVATARS
                .read()
                .unwrap()
                .iter()
                .copied()
                .find(|avatar| avatar.slug == slug)
        })
}

/// Normalizes a client-supplied avatar slug: only slugs present in the shipped
/// roster or the registered store survive; anything else (unknown, malformed,
/// path-like) becomes `None` so the receiving side falls back to the default
/// model.
pub fn normalize_avatar_slug(raw: Option<&str>) -> Option<&'static str> {
    let slug = raw?.trim();
    avatar_definition(slug).map(|avatar| avatar.slug.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekza_bevy_sdk::passport::{ProjectSupport, Rendition};
    use std::collections::{HashMap, HashSet};

    fn protected(avatar_digit: char, sha_digit: char) -> ProtectedAvatar {
        ProtectedAvatar {
            avatar_id: format!(
                "solana:devnet:avatar-data:{}",
                avatar_digit.to_string().repeat(32)
            ),
            support: ProjectSupport {
                project_id: "omoba".into(),
                platform: "desktop".into(),
                profile: "humanoid-glb-v1".into(),
                status: "approved".into(),
                rendition: Rendition {
                    id: "r1".into(),
                    url: "https://example.test/store.glb".into(),
                    sha256: sha_digit.to_string().repeat(64),
                    size_bytes: 4096,
                    format: "glb".into(),
                },
            },
        }
    }

    fn definition(slug: String, passport: Option<ProtectedAvatar>) -> AvatarDefinition {
        AvatarDefinition {
            slug,
            display_name: "Store".into(),
            collection: "Ekza store".into(),
            license: "See creator terms".into(),
            source_url: "https://example.test/store.glb".into(),
            author: None,
            thumbnail: None,
            passport,
            free: false,
        }
    }

    fn manifest(avatars: &[AvatarDefinition]) -> String {
        serde_json::json!({ "avatars": avatars }).to_string()
    }

    #[test]
    fn avatar_roster_is_committed_and_licensed() {
        // The roster is the committed manifest plus any avatars synced from
        // the Ekza Arena chain (arena-sync), so only a lower bound holds.
        let roster = avatar_roster();
        assert!(
            roster.len() >= 10,
            "roster size {} below the committed baseline",
            roster.len()
        );
        let mut slugs = HashSet::new();
        for avatar in roster {
            assert!(slugs.insert(avatar.slug.as_str()), "duplicate slug");
            assert!(!avatar.license.is_empty(), "{}", avatar.slug);
            assert!(!avatar.source_url.is_empty(), "{}", avatar.slug);
            assert!(!avatar.display_name.is_empty(), "{}", avatar.slug);
        }
    }

    #[test]
    fn approved_release_roster_retains_fifteen_heroes_and_removed_selections_fall_back() {
        let shipped: AvatarManifest = serde_json::from_str(AVATAR_MANIFEST_JSON).unwrap();
        assert_eq!(shipped.avatars.len(), 15);
        for removed in ["el-bueno", "slime-green", "slime-blue", "wendigo-hollow"] {
            assert!(shipped.avatars.iter().all(|avatar| avatar.slug != removed));
            assert_eq!(normalize_avatar_slug(Some(removed)), None);
            assert_eq!(normalize_avatar_slug(Some(&format!(" {removed} "))), None);
            assert!(avatar_definition(removed).is_none());
        }
    }

    #[test]
    fn avatar_slug_normalization_rejects_unknown_values() {
        let first = &avatar_roster()[0];
        assert_eq!(
            normalize_avatar_slug(Some(first.slug.as_str())),
            Some(first.slug.as_str())
        );
        assert_eq!(normalize_avatar_slug(Some("../etc/passwd")), None);
        assert_eq!(normalize_avatar_slug(Some("not-a-real-avatar")), None);
        assert_eq!(normalize_avatar_slug(Some("")), None);
        assert_eq!(normalize_avatar_slug(None), None);
    }

    #[test]
    fn store_avatars_register_only_under_their_derived_passport_slug() {
        // Unique avatar id and rendition: the registry is process-global.
        let protected = protected('7', '9');
        let slug = protected_slug(&protected);
        assert_eq!(normalize_avatar_slug(Some(&slug)), None);

        // Free entries, shipped slugs and mismatched names are all refused.
        assert!(register_store_avatar(definition(slug.clone(), None)).is_none());
        let shipped = avatar_roster()[0].slug.clone();
        assert!(
            register_store_avatar(definition(shipped.clone(), Some(protected.clone()))).is_none()
        );
        assert!(avatar_definition(&shipped).unwrap().passport.is_none());
        assert!(
            register_store_avatar(definition("ekza-forged".into(), Some(protected.clone())))
                .is_none()
        );
        let mut other_project = protected.clone();
        other_project.support.project_id = "ekza-space".into();
        assert!(
            register_store_avatar(definition(
                protected_slug(&other_project),
                Some(other_project)
            ))
            .is_none()
        );

        let registered =
            register_store_avatar(definition(slug.clone(), Some(protected.clone()))).unwrap();
        assert_eq!(registered.passport.as_ref(), Some(&protected));
        assert_eq!(
            normalize_avatar_slug(Some(&format!(" {slug} "))),
            Some(slug.as_str())
        );
        // Idempotent: the first registration stays canonical.
        let again = register_store_avatar(definition(slug.clone(), Some(protected))).unwrap();
        assert!(std::ptr::eq(registered, again));
        assert_eq!(store_avatars().iter().filter(|a| a.slug == slug).count(), 1);
    }

    /// Raid-boss models (TASK-19) live in `client/assets/bosses/`, outside the
    /// player roster manifest: their slugs must never be player-selectable.
    #[test]
    fn boss_slugs_are_not_player_selectable() {
        for boss_slug in ["wendigo-hollow", "king-mutatio"] {
            assert!(
                avatar_roster()
                    .iter()
                    .all(|avatar| avatar.slug != boss_slug),
                "boss slug {boss_slug:?} leaked into the player roster manifest"
            );
            assert_eq!(normalize_avatar_slug(Some(boss_slug)), None);
        }
    }

    #[test]
    fn roster_loads_override_then_asset_root_then_embedded_without_cwd_lookup() {
        let source = RosterSource {
            manifest_override: Some(PathBuf::from("/operator/roster.json")),
            asset_root: PathBuf::from("/package/assets"),
        };
        assert_eq!(
            source.candidates(),
            [
                PathBuf::from("/operator/roster.json"),
                PathBuf::from("/package/assets/avatars/manifest.json"),
            ]
        );
        let packaged = manifest(&[definition("packaged".into(), None)]);
        let files = HashMap::from([
            (
                PathBuf::from("/operator/roster.json"),
                "{not json".to_owned(),
            ),
            (
                PathBuf::from("/package/assets/avatars/manifest.json"),
                packaged,
            ),
        ]);
        let read = |path: &Path| {
            files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        };
        // An unparseable override falls through to the asset root, with a warning.
        let loaded = load_roster(&source.candidates(), read);
        assert_eq!(
            loaded.origin,
            RosterOrigin::File("/package/assets/avatars/manifest.json".into())
        );
        assert_eq!(loaded.avatars.len(), 1);
        assert_eq!(loaded.avatars[0].slug, "packaged");
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("/operator/roster.json"));
        assert_eq!(
            loaded.summary(),
            "Avatar roster: 1 avatars from /package/assets/avatars/manifest.json"
        );
        // No file at all: the embedded committed manifest, no warnings.
        let embedded = load_roster(
            &RosterSource {
                manifest_override: None,
                asset_root: PathBuf::from("/missing"),
            }
            .candidates(),
            read,
        );
        assert_eq!(embedded.origin, RosterOrigin::Embedded);
        assert_eq!(embedded.avatars.len(), 15);
        assert!(embedded.warnings.is_empty());
        assert_eq!(
            embedded.summary(),
            "Avatar roster: 15 avatars from embedded manifest"
        );
    }

    #[test]
    fn roster_entries_must_follow_the_store_avatar_rule() {
        let valid = protected('5', '6');
        let valid_slug = protected_slug(&valid);
        let mut invalid_boundary = protected('5', '8');
        invalid_boundary.support.project_id = "ekza-space".into();
        let entries = [
            definition("free-hero".into(), None),
            definition(valid_slug.clone(), Some(valid.clone())),
            // An `ekza-` slug with no boundary would pass as a free cosmetic.
            definition(format!("ekza-{}", "a".repeat(64)), None),
            // A boundary under a name that is not derived from it.
            definition("renamed-store-avatar".into(), Some(valid.clone())),
            definition(format!("ekza-{}", "0".repeat(64)), Some(valid)),
            // A boundary that fails validation, under its own derived slug.
            definition(
                protected_slug(&invalid_boundary),
                Some(invalid_boundary.clone()),
            ),
        ];
        let path = PathBuf::from("/roster.json");
        let raw = manifest(&entries);
        let loaded = load_roster(std::slice::from_ref(&path), |_| Ok(raw.clone()));
        assert_eq!(loaded.origin, RosterOrigin::File(path));
        assert_eq!(
            loaded
                .avatars
                .iter()
                .map(|avatar| avatar.slug.as_str())
                .collect::<Vec<_>>(),
            ["free-hero", valid_slug.as_str()]
        );
        assert_eq!(loaded.warnings.len(), 4, "{:?}", loaded.warnings);
        assert!(
            loaded
                .warnings
                .iter()
                .all(|warning| warning.contains("skipping"))
        );
    }
}
