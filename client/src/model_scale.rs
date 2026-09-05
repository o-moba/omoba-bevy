//! Character model size analysis and normalization.
//!
//! Character GLBs come from several sources (legacy SDK models, VRM-staged
//! roster avatars, raid bosses) with wildly different authored heights
//! (0.6 m .. 2.4 m). This module measures each model's bind-pose bounds
//! straight from the loaded glTF asset data — independent of animation state
//! or spawn timing — and rescales every model root so all characters share
//! the same world height by default.
//!
//! Per-model tweaks live in `assets/config/model_scale_overrides.json`
//! (slug -> multiplier, missing slug = 1.0). The file is polled at runtime so
//! multipliers can be tuned while the game is running.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use bevy::asset::AssetId;
use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::gltf::{Gltf, GltfMesh, GltfNode};
use bevy::math::Affine3A;
use bevy::prelude::*;

use crate::team::CharacterChoice;

/// World-relative hero height: the map is tuned for `PLAYER_SIZE = 1.0`
/// (46-unit base pads, 4-unit jungle blocks, ~3-4-unit trees, camera at
/// ~19 units), so heroes render slightly above one world unit tall.
pub const DEFAULT_MODEL_TARGET_HEIGHT: f32 = 1.15;
pub const MIN_MODEL_TARGET_HEIGHT: f32 = 0.3;
pub const MAX_MODEL_TARGET_HEIGHT: f32 = 3.0;
const NORMALIZATION_MIN_HEIGHT: f32 = 0.001;
const MIN_OVERRIDE_MULTIPLIER: f32 = 0.1;
const MAX_OVERRIDE_MULTIPLIER: f32 = 10.0;
const OVERRIDES_POLL_SECONDS: f32 = 1.0;
const OVERRIDES_RELATIVE_PATH: &str = "config/model_scale_overrides.json";

pub struct ModelScalePlugin;

impl Plugin for ModelScalePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModelScaleSettings>()
            .init_resource::<ModelSizeAnalysis>()
            .init_resource::<ModelScaleOverrides>()
            .add_systems(Startup, load_model_scale_overrides)
            .add_systems(
                Update,
                (
                    poll_model_scale_overrides,
                    apply_model_scale_system,
                    normalize_model_scale_fallback_system,
                ),
            );
    }
}

/// Shared normalized character height, adjustable from the pause menu and
/// persisted across sessions.
#[derive(Resource, Clone, Copy)]
pub struct ModelScaleSettings {
    pub target_height: f32,
}

impl Default for ModelScaleSettings {
    fn default() -> Self {
        Self {
            target_height: DEFAULT_MODEL_TARGET_HEIGHT,
        }
    }
}

/// Marks a model root for height normalization.
#[derive(Component)]
pub struct NormalizeModelScale {
    base_scale: Vec3,
    /// Multiplier on the shared normalized target height (1.0 = player-sized;
    /// raid bosses use [`crate::bosses::BOSS_MODEL_HEIGHT_SCALE`]).
    height_scale: f32,
    /// Effective target height last applied to the root transform. Skips
    /// redundant re-application; a target/override change invalidates it.
    last_applied_target_height: Option<f32>,
    /// Raw (unscaled) model height derived on the first successful fallback
    /// AABB measurement, so later re-applications stay absolute instead of
    /// compounding on the already-scaled transform.
    fallback_raw_height: Option<f32>,
    /// Raw (unscaled) height of the model top above the entity origin,
    /// captured together with `fallback_raw_height`.
    fallback_raw_top: Option<f32>,
    /// Raw (unscaled) height of the model bottom relative to the entity
    /// origin, captured together with `fallback_raw_height`.
    fallback_raw_bottom: Option<f32>,
    /// Height of the model's top above the entity origin after normalization.
    /// Used to anchor the head bar deterministically instead of per-frame AABB
    /// sampling, which is unstable for rigged/center-pivot models.
    pub head_local_y: Option<f32>,
    /// Height of the model's lowest point (feet) relative to the entity
    /// origin after normalization. Grounding places the entity so that
    /// `origin + foot_local_y` rests on the terrain surface.
    foot_local_y: Option<f32>,
}

impl NormalizeModelScale {
    pub fn for_player_model() -> Self {
        Self {
            base_scale: Vec3::ONE,
            height_scale: 1.0,
            last_applied_target_height: None,
            fallback_raw_height: None,
            fallback_raw_top: None,
            fallback_raw_bottom: None,
            head_local_y: None,
            foot_local_y: None,
        }
    }

    /// Normalized height of the model's lowest point relative to the entity
    /// origin (negative when geometry hangs below the origin). `None` until
    /// the model has been measured.
    pub fn foot_local_y(&self) -> Option<f32> {
        self.foot_local_y
    }

    /// Like [`Self::for_player_model`], but normalized to `height_scale` times
    /// the player target height (raid-boss presence).
    pub fn scaled_by(height_scale: f32) -> Self {
        Self {
            height_scale: height_scale.max(0.1),
            ..Self::for_player_model()
        }
    }
}

/// Which glTF asset a normalized model root was instantiated from, plus the
/// override key ("agnes", "paco", "wendigo-hollow", ...) used for per-model
/// size multipliers and analysis logging.
#[derive(Component, Clone)]
pub struct ModelScaleSource {
    pub gltf: Handle<Gltf>,
    pub key: String,
}

/// Override key for a player model: roster avatar slug when the player wears
/// a known avatar, legacy character slug otherwise.
pub fn model_scale_key(character: CharacterChoice, avatar: Option<&str>) -> String {
    avatar
        .and_then(shared::avatar_definition)
        .map(|definition| definition.slug.clone())
        .unwrap_or_else(|| character.slug().to_owned())
}

/// Bind-pose bounds of one glTF model, measured in the asset's own units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelMeasurement {
    pub min_y: f32,
    pub max_y: f32,
}

impl ModelMeasurement {
    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }
}

/// One cached measurement per glTF asset. `Some(None)` means the asset loaded
/// but exposes no measurable geometry (logged once, left unscaled).
#[derive(Resource, Default)]
pub struct ModelSizeAnalysis {
    measured: HashMap<AssetId<Gltf>, Option<ModelMeasurement>>,
}

/// Per-model size multipliers loaded from
/// `assets/config/model_scale_overrides.json`.
#[derive(Resource, Default)]
pub struct ModelScaleOverrides {
    multipliers: HashMap<String, f32>,
    last_modified: Option<SystemTime>,
}

impl ModelScaleOverrides {
    pub fn multiplier(&self, key: &str) -> f32 {
        self.multipliers.get(key).copied().unwrap_or(1.0)
    }
}

/// Parses the overrides file: a flat JSON object of slug -> multiplier.
/// Keys starting with `_` are treated as comments; non-numeric values are
/// ignored; multipliers are clamped to a sane range.
fn parse_overrides(raw: &str) -> Result<HashMap<String, f32>, serde_json::Error> {
    let values: HashMap<String, serde_json::Value> = serde_json::from_str(raw)?;
    Ok(values
        .into_iter()
        .filter(|(key, _)| !key.starts_with('_'))
        .filter_map(|(key, value)| {
            value.as_f64().map(|multiplier| {
                (
                    key,
                    (multiplier as f32).clamp(MIN_OVERRIDE_MULTIPLIER, MAX_OVERRIDE_MULTIPLIER),
                )
            })
        })
        .collect())
}

fn overrides_path() -> Option<PathBuf> {
    let path = shared::client_asset_root().join(OVERRIDES_RELATIVE_PATH);
    path.exists().then_some(path)
}

fn read_overrides(overrides: &mut ModelScaleOverrides) {
    let Some(path) = overrides_path() else {
        return;
    };
    let modified = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    if modified.is_some() && modified == overrides.last_modified {
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(raw) => match parse_overrides(&raw) {
            Ok(multipliers) => {
                let tweaked = multipliers
                    .iter()
                    .filter(|(_, multiplier)| (**multiplier - 1.0).abs() > f32::EPSILON)
                    .count();
                info!(
                    "model-scale: loaded {} override entries ({} != 1.0) from {}",
                    multipliers.len(),
                    tweaked,
                    path.display()
                );
                overrides.multipliers = multipliers;
                overrides.last_modified = modified;
            }
            Err(error) => warn!(
                "model-scale: invalid overrides file {}: {error}",
                path.display()
            ),
        },
        Err(error) => warn!("model-scale: cannot read {}: {error}", path.display()),
    }
}

fn load_model_scale_overrides(mut overrides: ResMut<ModelScaleOverrides>) {
    read_overrides(&mut overrides);
}

/// Re-reads the overrides file when its mtime changes, so per-model
/// multipliers can be tweaked live while the game runs.
fn poll_model_scale_overrides(
    time: Res<Time>,
    mut accumulated: Local<f32>,
    mut overrides: ResMut<ModelScaleOverrides>,
) {
    *accumulated += time.delta_secs();
    if *accumulated < OVERRIDES_POLL_SECONDS {
        return;
    }
    *accumulated = 0.0;
    // Bypass change detection unless the file actually reloads.
    read_overrides(overrides.bypass_change_detection());
}

/// Absolute root scale that brings a model of `raw_height` to `target_height`.
fn scale_for_height(raw_height: f32, target_height: f32) -> Option<f32> {
    (raw_height > NORMALIZATION_MIN_HEIGHT).then(|| target_height / raw_height)
}

/// Effective per-entity target height: shared setting x entity multiplier
/// (bosses) x per-model override.
fn effective_target_height(
    settings: &ModelScaleSettings,
    normalization: &NormalizeModelScale,
    override_multiplier: f32,
) -> f32 {
    settings
        .target_height
        .clamp(MIN_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT)
        * normalization.height_scale
        * override_multiplier
}

/// Walks the glTF node graph (bind pose, no animation) and folds every mesh
/// primitive's AABB, transformed by the composed node transforms, into one
/// model-space bounds measurement. Returns `None` while any referenced
/// sub-asset is still loading; `Some(None)` when the graph holds no geometry.
fn measure_gltf_bind_pose(
    gltf: &Gltf,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
) -> Option<Option<ModelMeasurement>> {
    let mut child_ids: HashSet<AssetId<GltfNode>> = HashSet::new();
    for handle in &gltf.nodes {
        let node = nodes.get(handle)?;
        for child in &node.children {
            child_ids.insert(child.id());
        }
    }

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut has_bounds = false;

    for handle in &gltf.nodes {
        if child_ids.contains(&handle.id()) {
            continue;
        }
        accumulate_node_bounds(
            handle,
            Affine3A::IDENTITY,
            nodes,
            gltf_meshes,
            meshes,
            &mut min,
            &mut max,
            &mut has_bounds,
        )?;
    }

    Some(has_bounds.then_some(ModelMeasurement {
        min_y: min.y,
        max_y: max.y,
    }))
}

#[expect(clippy::too_many_arguments, reason = "internal recursive helper")]
fn accumulate_node_bounds(
    handle: &Handle<GltfNode>,
    parent: Affine3A,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
    min: &mut Vec3,
    max: &mut Vec3,
    has_bounds: &mut bool,
) -> Option<()> {
    let node = nodes.get(handle)?;
    let world = parent * node.transform.compute_affine();
    if let Some(mesh_handle) = &node.mesh {
        let gltf_mesh = gltf_meshes.get(mesh_handle)?;
        for primitive in &gltf_mesh.primitives {
            let mesh = meshes.get(&primitive.mesh)?;
            let Some(aabb) = mesh.compute_aabb() else {
                continue;
            };
            fold_aabb_corners(&aabb, &world, min, max);
            *has_bounds = true;
        }
    }
    for child in &node.children {
        accumulate_node_bounds(
            child,
            world,
            nodes,
            gltf_meshes,
            meshes,
            min,
            max,
            has_bounds,
        )?;
    }
    Some(())
}

fn fold_aabb_corners(aabb: &Aabb, world: &Affine3A, min: &mut Vec3, max: &mut Vec3) {
    let center: Vec3 = aabb.center.into();
    let half: Vec3 = aabb.half_extents.into();
    for sx in [-1.0_f32, 1.0] {
        for sy in [-1.0_f32, 1.0] {
            for sz in [-1.0_f32, 1.0] {
                let corner = world.transform_point3(center + half * Vec3::new(sx, sy, sz));
                *min = min.min(corner);
                *max = max.max(corner);
            }
        }
    }
}

/// Normalizes every model root with a known glTF source to the effective
/// target height, using the cached bind-pose measurement. Absolute and
/// idempotent: re-running with the same target is a no-op, and target changes
/// rescale from the raw measurement instead of compounding.
fn apply_model_scale_system(
    settings: Res<ModelScaleSettings>,
    overrides: Res<ModelScaleOverrides>,
    gltf_assets: Res<Assets<Gltf>>,
    node_assets: Res<Assets<GltfNode>>,
    gltf_mesh_assets: Res<Assets<GltfMesh>>,
    mesh_assets: Res<Assets<Mesh>>,
    mut analysis: ResMut<ModelSizeAnalysis>,
    mut roots: Query<(&mut Transform, &mut NormalizeModelScale, &ModelScaleSource)>,
) {
    for (mut transform, mut normalization, source) in &mut roots {
        let target_height =
            effective_target_height(&settings, &normalization, overrides.multiplier(&source.key));
        if normalization
            .last_applied_target_height
            .is_some_and(|applied| (applied - target_height).abs() < f32::EPSILON)
        {
            continue;
        }

        let gltf_id = source.gltf.id();
        if let std::collections::hash_map::Entry::Vacant(vacant) = analysis.measured.entry(gltf_id)
        {
            let Some(gltf) = gltf_assets.get(&source.gltf) else {
                continue; // Still loading; retry next frame.
            };
            let Some(measurement) =
                measure_gltf_bind_pose(gltf, &node_assets, &gltf_mesh_assets, &mesh_assets)
            else {
                continue; // Sub-assets still loading; retry next frame.
            };
            match measurement {
                Some(measured) => info!(
                    "model-scale: '{}' bind-pose height {:.3} (y {:.3}..{:.3})",
                    source.key,
                    measured.height(),
                    measured.min_y,
                    measured.max_y,
                ),
                None => warn!(
                    "model-scale: '{}' has no measurable geometry; leaving unscaled",
                    source.key
                ),
            }
            vacant.insert(measurement);
        }

        let Some(Some(measured)) = analysis.measured.get(&gltf_id) else {
            // Unmeasurable model: remember the attempt so we don't warn-loop.
            normalization.last_applied_target_height = Some(target_height);
            continue;
        };

        let Some(scale_factor) = scale_for_height(measured.height(), target_height) else {
            normalization.last_applied_target_height = Some(target_height);
            continue;
        };
        transform.scale = normalization.base_scale * Vec3::splat(scale_factor);
        normalization.head_local_y = Some(measured.max_y * scale_factor);
        normalization.foot_local_y = Some(measured.min_y * scale_factor);
        normalization.last_applied_target_height = Some(target_height);
        info!(
            "model-scale: '{}' normalized to {:.3} (scale x{:.4})",
            source.key, target_height, scale_factor
        );
    }
}

/// Fallback for model roots without a glTF source (e.g. primitive-mesh
/// stand-ins): measures the spawned world-space AABB once, derives the raw
/// model height from it, and from then on applies absolute rescales exactly
/// like the glTF path.
fn normalize_model_scale_fallback_system(
    settings: Res<ModelScaleSettings>,
    mut roots: Query<(Entity, &mut Transform, &mut NormalizeModelScale), Without<ModelScaleSource>>,
    children_query: Query<&Children>,
    aabb_query: Query<&Aabb>,
    globals_query: Query<&GlobalTransform>,
) {
    for (entity, mut transform, mut normalization) in &mut roots {
        let target_height = effective_target_height(&settings, &normalization, 1.0);
        if normalization
            .last_applied_target_height
            .is_some_and(|applied| (applied - target_height).abs() < f32::EPSILON)
        {
            continue;
        }

        if normalization.fallback_raw_height.is_none() {
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            let mut has_bounds = false;

            for descendant in children_query.iter_descendants(entity) {
                let (Ok(aabb), Ok(global)) =
                    (aabb_query.get(descendant), globals_query.get(descendant))
                else {
                    continue;
                };
                let center: Vec3 = aabb.center.into();
                let half: Vec3 = aabb.half_extents.into();
                for sx in [-1.0_f32, 1.0] {
                    for sy in [-1.0_f32, 1.0] {
                        for sz in [-1.0_f32, 1.0] {
                            let local_corner =
                                center + Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                            let world_corner = global.transform_point(local_corner);
                            min_y = min_y.min(world_corner.y);
                            max_y = max_y.max(world_corner.y);
                            has_bounds = true;
                        }
                    }
                }
            }

            if !has_bounds {
                continue;
            }
            let current_height = max_y - min_y;
            if current_height <= NORMALIZATION_MIN_HEIGHT {
                continue;
            }
            // De-scale the sampled world bounds back to raw model units so
            // later re-applications stay absolute.
            let current_scale_y = transform.scale.y.max(NORMALIZATION_MIN_HEIGHT);
            normalization.fallback_raw_height = Some(current_height / current_scale_y);
            normalization.fallback_raw_top =
                Some((max_y - transform.translation.y) / current_scale_y);
            normalization.fallback_raw_bottom =
                Some((min_y - transform.translation.y) / current_scale_y);
        }

        let Some(raw_height) = normalization.fallback_raw_height else {
            continue;
        };
        let Some(scale_factor) = scale_for_height(raw_height, target_height) else {
            normalization.last_applied_target_height = Some(target_height);
            continue;
        };
        transform.scale = normalization.base_scale * Vec3::splat(scale_factor);
        normalization.head_local_y = normalization
            .fallback_raw_top
            .map(|raw_top| raw_top * scale_factor);
        normalization.foot_local_y = normalization
            .fallback_raw_bottom
            .map(|raw_bottom| raw_bottom * scale_factor);
        normalization.last_applied_target_height = Some(target_height);
    }
}

/// Headless CLI analyzer: loads every staged character/boss GLB through the
/// real glTF pipeline (no window, no GPU), measures bind-pose bounds with the
/// exact same code the game uses, and prints a size table. Run with
/// `OMOBA_MEASURE_MODELS=1 cargo run -p client`.
pub fn run_model_measurement_analyzer() {
    use bevy::asset::{AssetPlugin, RecursiveDependencyLoadState};

    let assets_root = shared::client_asset_root();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin {
            file_path: assets_root.to_string_lossy().into_owned(),
            ..default()
        })
        .add_plugins(bevy::mesh::MeshPlugin)
        .add_plugins(bevy::scene::ScenePlugin)
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<bevy::animation::AnimationClip>()
        .add_plugins(bevy::gltf::GltfPlugin::default());
    app.finish();
    app.cleanup();

    let mut relative_paths: Vec<String> = Vec::new();
    for dir in ["downloaded", "avatars", "bosses", "minions"] {
        let Ok(entries) = std::fs::read_dir(assets_root.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".glb") {
                relative_paths.push(format!("{dir}/{name}"));
            }
        }
    }
    relative_paths.sort();

    let asset_server = app.world().resource::<AssetServer>().clone();
    let mut pending: Vec<(String, Handle<Gltf>)> = relative_paths
        .into_iter()
        .map(|path| (path.clone(), asset_server.load(path)))
        .collect();

    println!(
        "model-scale analyzer: measuring {} GLBs (bind pose)",
        pending.len()
    );
    println!(
        "{:<45} {:>10} {:>10} {:>10} {:>12}",
        "model",
        "height",
        "min_y",
        "max_y",
        format!("scale@{DEFAULT_MODEL_TARGET_HEIGHT}"),
    );

    let mut rounds = 0;
    while !pending.is_empty() && rounds < 6000 {
        app.update();
        rounds += 1;
        pending.retain(|(path, handle)| {
            if let RecursiveDependencyLoadState::Failed(error) =
                asset_server.recursive_dependency_load_state(handle)
            {
                println!("{path:<45} LOAD FAILED: {error}");
                return false;
            }
            let world = app.world();
            let Some(gltf) = world.resource::<Assets<Gltf>>().get(handle) else {
                return true;
            };
            let Some(measurement) = measure_gltf_bind_pose(
                gltf,
                world.resource::<Assets<GltfNode>>(),
                world.resource::<Assets<GltfMesh>>(),
                world.resource::<Assets<Mesh>>(),
            ) else {
                return true;
            };
            match measurement {
                Some(measured) => println!(
                    "{path:<45} {:>10.4} {:>10.4} {:>10.4} {:>12.4}",
                    measured.height(),
                    measured.min_y,
                    measured.max_y,
                    DEFAULT_MODEL_TARGET_HEIGHT / measured.height(),
                ),
                None => println!("{path:<45} no measurable geometry"),
            }
            false
        });
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    for (path, _) in &pending {
        println!("{path:<45} TIMED OUT while loading");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_overrides_reads_flat_map_and_skips_comments() {
        let parsed =
            parse_overrides(r#"{ "_comment": "docs", "agnes": 1.2, "paco": 0.9, "bad": "nope" }"#)
                .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["agnes"], 1.2);
        assert_eq!(parsed["paco"], 0.9);
    }

    #[test]
    fn parse_overrides_clamps_extremes() {
        let parsed = parse_overrides(r#"{ "tiny": 0.0001, "huge": 500.0 }"#).unwrap();
        assert_eq!(parsed["tiny"], MIN_OVERRIDE_MULTIPLIER);
        assert_eq!(parsed["huge"], MAX_OVERRIDE_MULTIPLIER);
    }

    #[test]
    fn parse_overrides_rejects_invalid_json() {
        assert!(parse_overrides("not json").is_err());
    }

    #[test]
    fn missing_override_defaults_to_one() {
        let overrides = ModelScaleOverrides::default();
        assert_eq!(overrides.multiplier("unknown"), 1.0);
    }

    #[test]
    fn scale_for_height_is_absolute_and_idempotent() {
        // A 1.6 m avatar normalized to 0.26, then retargeted to 0.52: both
        // scales derive from the raw height, so no compounding.
        let first = scale_for_height(1.6, 0.26).unwrap();
        let second = scale_for_height(1.6, 0.52).unwrap();
        assert!((first - 0.1625).abs() < 1e-6);
        assert!((second - 2.0 * first).abs() < 1e-6);
        // Degenerate heights are rejected.
        assert!(scale_for_height(0.0, 0.26).is_none());
    }

    #[test]
    fn effective_target_combines_setting_boss_scale_and_override() {
        let settings = ModelScaleSettings { target_height: 1.0 };
        let player = NormalizeModelScale::for_player_model();
        let boss = NormalizeModelScale::scaled_by(3.0);
        assert!((effective_target_height(&settings, &player, 1.0) - 1.0).abs() < 1e-6);
        assert!((effective_target_height(&settings, &boss, 1.0) - 3.0).abs() < 1e-6);
        assert!((effective_target_height(&settings, &player, 1.5) - 1.5).abs() < 1e-6);
        // Out-of-range settings are clamped before multipliers apply.
        let wild = ModelScaleSettings { target_height: 9.0 };
        assert!(
            (effective_target_height(&wild, &player, 1.0) - MAX_MODEL_TARGET_HEIGHT).abs() < 1e-6
        );
    }

    #[test]
    fn foot_offset_scales_with_measurement() {
        // Toka-like model: geometry hangs 0.33 below the origin, 0.84 tall.
        let measured = ModelMeasurement {
            min_y: -0.3328,
            max_y: 0.5036,
        };
        let scale = scale_for_height(measured.height(), 1.15).unwrap();
        let foot = measured.min_y * scale;
        assert!(foot < 0.0);
        // Feet land exactly on the surface when the origin is lifted by -foot.
        assert!((measured.min_y * scale - foot).abs() < 1e-6);
        assert!(((measured.max_y - measured.min_y) * scale - 1.15).abs() < 1e-4);
    }

    #[test]
    fn model_scale_key_prefers_known_avatar_slug() {
        assert_eq!(
            model_scale_key(CharacterChoice::Paco, Some("agnes")),
            "agnes"
        );
        assert_eq!(
            model_scale_key(CharacterChoice::Paco, Some("definitely-not-a-slug")),
            "paco"
        );
        assert_eq!(model_scale_key(CharacterChoice::Toka, None), "toka");
    }
}
