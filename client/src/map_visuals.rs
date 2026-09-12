//! Local map cosmetics bound to authored instance roots, never simulation state.
use std::collections::{HashMap, HashSet};

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    gltf::GltfExtras,
    mesh::VertexAttributeValues,
    prelude::*,
    scene::SceneInstance,
};
use serde::Deserialize;

use crate::{
    combat_visuals::safe_asset_path,
    sprite::PlayerVisualMode,
    verdant3d::{VerdantEnvironment, VerdantFoliage},
};

const MAX_INSTANCES: usize = 2048;
const MAX_MODELS: usize = 64;
const MAX_MATERIALS: usize = 512;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MapModel {
    pub path: String,
    #[serde(default)]
    pub scene: usize,
    /// Artist assertion that the model represents the unchanged collision.
    /// Runtime additionally checks horizontal extent and pivot alignment.
    #[serde(default)]
    pub preserve_collision: bool,
}

impl MapModel {
    pub fn asset_path(&self) -> String {
        format!("{}#Scene{}", self.path, self.scene)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MapVisualProfile {
    pub model: Option<MapModel>,
    pub palette: Option<String>,
    pub tint: Option<[f32; 4]>,
    pub offset: Option<[f32; 3]>,
    pub rotation_degrees: Option<[f32; 3]>,
    pub scale: Option<[f32; 3]>,
    /// Existing world2d prop or presentation2d actor key; unknown keys fall back.
    pub sprite_key: Option<String>,
}

impl MapVisualProfile {
    pub fn color(&self) -> Color {
        let [r, g, b, a] = self.tint.unwrap_or([1.0; 4]);
        Color::srgba(r, g, b, a)
    }

    fn overlay(&mut self, other: &Self) {
        macro_rules! field {
            ($name:ident) => {
                if other.$name.is_some() {
                    self.$name.clone_from(&other.$name);
                }
            };
        }
        field!(model);
        field!(tint);
        field!(offset);
        field!(rotation_degrees);
        field!(scale);
        field!(sprite_key);
    }

    fn transform(&self) -> Transform {
        let [x, y, z] = self
            .rotation_degrees
            .unwrap_or([0.0; 3])
            .map(f32::to_radians);
        Transform::from_translation(Vec3::from_array(self.offset.unwrap_or([0.0; 3])))
            .with_rotation(Quat::from_euler(EulerRot::XYZ, x, y, z))
            .with_scale(Vec3::from_array(self.scale.unwrap_or([1.0; 3])))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    #[serde(default)]
    palettes: HashMap<String, [f32; 4]>,
    #[serde(default)]
    archetypes: HashMap<String, MapVisualProfile>,
    #[serde(default)]
    instances: HashMap<String, MapVisualProfile>,
}

#[derive(Resource, Clone, Debug)]
pub struct MapVisualRegistry {
    config: Config,
    revision: u64,
}

impl Default for MapVisualRegistry {
    fn default() -> Self {
        let mut archetypes = HashMap::new();
        for (key, model, sprite) in [
            ("tower_green", "watchtower_green", "green_tower"),
            ("tower_blue", "watchtower_blue", "blue_tower"),
            ("base_green", "sanctuary_green", "green_base_tower"),
            ("base_blue", "sanctuary_blue", "blue_base_tower"),
        ] {
            archetypes.insert(
                key.into(),
                MapVisualProfile {
                    model: Some(MapModel {
                        path: format!("verdant/{model}.glb"),
                        scene: 0,
                        preserve_collision: true,
                    }),
                    sprite_key: Some(sprite.into()),
                    ..default()
                },
            );
        }
        Self {
            config: Config {
                schema_version: 1,
                palettes: HashMap::new(),
                archetypes,
                instances: HashMap::new(),
            },
            revision: 0,
        }
    }
}

impl MapVisualRegistry {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Complete override document; omitted entries restore immutable defaults.
    pub fn replace_json(&mut self, json: &str) -> Result<(), String> {
        let mut replacement = Self::from_json(json)?;
        replacement.revision = self.revision.wrapping_add(1);
        *self = replacement;
        Ok(())
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > 256 * 1024 {
            return Err("map visuals exceed 256 KiB".into());
        }
        let config: Config = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if config.schema_version != 1 {
            return Err("unsupported map visual schema_version".into());
        }
        if config.archetypes.len() > 128
            || config.instances.len() > 1024
            || config.palettes.len() > 64
        {
            return Err("map visual entry budget exceeded".into());
        }
        let valid_key =
            |s: &str| !s.is_empty() && s.len() <= 160 && !s.chars().any(char::is_control);
        let valid_color = |c: [f32; 4]| {
            c.into_iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
                && c[3] == 1.0
        };
        for (key, color) in &config.palettes {
            if !valid_key(key) || !valid_color(*color) {
                return Err("invalid opaque map palette".into());
            }
        }
        for (key, profile) in config.archetypes.iter().chain(&config.instances) {
            if !valid_key(key) {
                return Err("invalid map visual key".into());
            }
            if profile.tint.is_some_and(|v| !valid_color(v)) {
                return Err(format!("{key}: tint must be finite opaque RGBA"));
            }
            if profile
                .palette
                .as_ref()
                .is_some_and(|v| !config.palettes.contains_key(v))
            {
                return Err(format!("{key}: unknown palette"));
            }
            for (values, min, max) in [
                (profile.offset, -3.0, 3.0),
                (profile.rotation_degrees, -360.0, 360.0),
                (profile.scale, 0.25, 3.0),
            ] {
                if values.is_some_and(|values| {
                    values
                        .into_iter()
                        .any(|v| !v.is_finite() || !(min..=max).contains(&v))
                }) {
                    return Err(format!("{key}: cosmetic transform exceeds bounds"));
                }
            }
            if let Some(model) = &profile.model
                && (!safe_asset_path(&model.path, ".glb") || model.scene > 31)
            {
                return Err(format!("{key}: unsafe packaged GLB path/scene"));
            }
            if profile.sprite_key.as_ref().is_some_and(|v| !valid_key(v)) {
                return Err(format!("{key}: invalid sprite key"));
            }
        }
        let mut result = Self::default();
        result.config.palettes = config.palettes;
        for (key, profile) in config.archetypes {
            result
                .config
                .archetypes
                .entry(key)
                .or_default()
                .overlay(&Self::with_palette(profile, &result.config.palettes));
        }
        result.config.instances = config
            .instances
            .into_iter()
            .map(|(key, value)| (key, Self::with_palette(value, &result.config.palettes)))
            .collect();
        Ok(result)
    }

    fn with_palette(
        mut profile: MapVisualProfile,
        palettes: &HashMap<String, [f32; 4]>,
    ) -> MapVisualProfile {
        if profile.tint.is_none() {
            profile.tint = profile
                .palette
                .as_ref()
                .and_then(|key| palettes.get(key))
                .copied();
        }
        profile
    }

    pub fn resolve(&self, archetype: &str, instance_key: &str) -> MapVisualProfile {
        let mut result = self
            .config
            .archetypes
            .get(archetype)
            .cloned()
            .unwrap_or_default();
        if let Some(instance) = self.config.instances.get(instance_key) {
            result.overlay(instance);
        }
        result
    }
}

/// Stable binding on the real prop/structure presentation root. Readiness means
/// loaded mesh vertices, not merely a requested SceneRoot or a marker entity.
#[derive(Component, Debug)]
pub struct MapPropInstance {
    pub key: String,
    pub archetype: String,
    pub role: String,
    pub solid: bool,
    pub desired_model: Option<String>,
    pub active_model: Option<String>,
    pub geometry_ready: bool,
    pub authored_model: Option<String>,
}

impl MapPropInstance {
    pub(crate) fn structure(key: String, archetype: String, authored_model: String) -> Self {
        Self {
            key,
            archetype,
            role: "live_structure".into(),
            solid: true,
            desired_model: None,
            active_model: Some(authored_model.clone()),
            geometry_ready: false,
            authored_model: Some(authored_model),
        }
    }
}

#[derive(Component)]
struct PropState {
    authored_transform: Transform,
    original_children: Vec<Entity>,
    original_bounds: MeshBounds,
    replacement: Option<(String, Entity, Handle<Scene>, ReplacementStatus)>,
    applied_revision: Option<u64>,
    applied_key: String,
    applied_archetype: String,
    tint_dirty: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplacementStatus {
    Pending,
    Ready,
    Rejected,
}

#[derive(Component)]
struct ReplacementRoot;

#[derive(Component)]
struct OriginalMaterial(Handle<StandardMaterial>);

#[derive(Resource, Default)]
pub struct MapVisualCache {
    models: HashMap<String, Handle<Scene>>,
    materials: HashMap<(AssetId<StandardMaterial>, [u32; 4]), Handle<StandardMaterial>>,
}

impl MapVisualCache {
    pub fn counts(&self) -> (usize, usize) {
        (self.models.len(), self.materials.len())
    }
    fn scene(&mut self, path: &str, server: &AssetServer) -> Option<Handle<Scene>> {
        if let Some(handle) = self.models.get(path) {
            return Some(handle.clone());
        }
        if self.models.len() >= MAX_MODELS {
            return None;
        }
        let handle = server.load(path.to_owned());
        self.models.insert(path.into(), handle.clone());
        Some(handle)
    }
}

#[derive(Clone, Copy, Debug)]
struct MeshBounds {
    min: Vec3,
    max: Vec3,
    meshes: usize,
}

impl MeshBounds {
    fn collision_compatible(self, other: Self) -> bool {
        let own = self.max - self.min;
        let new = other.max - other.min;
        let center_delta = ((self.max + self.min) - (other.max + other.min)) * 0.5;
        [0, 2].into_iter().all(|axis| {
            own[axis] > 0.01
                && (0.85..=1.15).contains(&(new[axis] / own[axis]))
                && center_delta[axis].abs() <= 0.25
        }) && (self.min.y - other.min.y).abs() <= 0.25
    }
}

/// Bounds in the target root's local frame, including intermediate node TRS.
fn drawable_bounds(
    root: Entity,
    children: &Query<&Children>,
    nodes: &Query<(&Transform, Option<&ChildOf>)>,
    drawables: &Query<&Mesh3d>,
    meshes: &Assets<Mesh>,
) -> Option<MeshBounds> {
    let mut result = MeshBounds {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
        meshes: 0,
    };
    for entity in children.iter_descendants(root) {
        let Ok(handle) = drawables.get(entity) else {
            continue;
        };
        let mesh = meshes.get(&handle.0)?;
        let VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?
        else {
            return None;
        };
        if points.is_empty() {
            return None;
        }
        let mut matrix = Mat4::IDENTITY;
        let mut current = entity;
        let mut reached = false;
        for _ in 0..32 {
            if current == root {
                reached = true;
                break;
            }
            let (transform, parent) = nodes.get(current).ok()?;
            matrix = transform.to_matrix() * matrix;
            current = parent?.parent();
        }
        if !reached {
            return None;
        }
        for point in points {
            let point = matrix.transform_point3(Vec3::from_array(*point));
            if !point.is_finite() {
                return None;
            }
            result.min = result.min.min(point);
            result.max = result.max.max(point);
        }
        result.meshes += 1;
    }
    (result.meshes > 0).then_some(result)
}

#[derive(Asset, TypePath)]
struct LoadedMapVisuals(MapVisualRegistry);
#[derive(Default, TypePath)]
struct MapVisualLoader;
impl AssetLoader for MapVisualLoader {
    type Asset = LoadedMapVisuals;
    type Settings = ();
    type Error = std::io::Error;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let json = std::str::from_utf8(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        MapVisualRegistry::from_json(json)
            .map(LoadedMapVisuals)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}
#[derive(Resource, Default)]
struct PendingConfig(Option<Handle<LoadedMapVisuals>>);

pub struct MapVisualsPlugin;
impl Plugin for MapVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapVisualRegistry>()
            .init_resource::<MapVisualCache>()
            .init_resource::<PendingConfig>()
            .init_asset::<LoadedMapVisuals>()
            .init_asset_loader::<MapVisualLoader>()
            .add_systems(Startup, request_config)
            .add_systems(Update, apply_config)
            .add_systems(
                PostUpdate,
                (
                    import_authored_props,
                    initialize_props,
                    reconcile_props,
                    tint_props,
                )
                    .chain()
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}
fn request_config(server: Res<AssetServer>, mut pending: ResMut<PendingConfig>) {
    pending.0 = Some(server.load("config/map_visuals.json"));
}
fn apply_config(
    server: Res<AssetServer>,
    assets: Res<Assets<LoadedMapVisuals>>,
    mut pending: ResMut<PendingConfig>,
    mut registry: ResMut<MapVisualRegistry>,
) {
    let Some(handle) = pending.0.as_ref() else {
        return;
    };
    if let Some(loaded) = assets.get(handle) {
        let revision = registry.revision.wrapping_add(1);
        *registry = loaded.0.clone();
        registry.revision = revision;
        pending.0 = None;
    } else if matches!(
        server.load_state(handle.id()),
        bevy::asset::LoadState::Failed(_)
    ) {
        warn!("Map visual manifest unavailable/invalid; using authored defaults");
        registry.revision = registry.revision.wrapping_add(1);
        pending.0 = None;
    }
}

fn import_authored_props(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    candidates: Query<
        (Entity, &Name, &GltfExtras, &ChildOf),
        (Without<MapPropInstance>, Added<GltfExtras>),
    >,
    environment: Query<(), With<VerdantEnvironment>>,
    foliage: Query<(), With<VerdantFoliage>>,
    existing: Query<&MapPropInstance>,
    parents: Query<&ChildOf>,
    replacements: Query<(), With<ReplacementRoot>>,
) {
    if *mode != PlayerVisualMode::Models3d || candidates.is_empty() {
        return;
    }
    let solid_ids: HashSet<_> = shared::navigation::world_navigation()
        .obstacles()
        .iter()
        .map(|o| o.id.as_str())
        .collect();
    let mut count = existing.iter().count();
    for (entity, name, extras, parent) in &candidates {
        if count >= MAX_INSTANCES {
            break;
        }
        // Bevy inserts a GLTF world-root wrapper. Never import a nested
        // replacement/library model as another authored map instance.
        let mut ancestor = parent.parent();
        let mut source = None;
        for _ in 0..32 {
            if replacements.contains(ancestor) || existing.contains(ancestor) {
                break;
            }
            if environment.contains(ancestor) {
                source = Some("environment.glb");
                break;
            }
            if foliage.contains(ancestor) {
                source = Some("foliage.glb");
                break;
            }
            let Ok(parent) = parents.get(ancestor) else {
                break;
            };
            ancestor = parent.parent();
        }
        let Some(source) = source else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
            continue;
        };
        let Some(archetype) = value.get("asset_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let role = value
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("decoration");
        // The bridge is an adapted walking surface, not a replaceable decoration.
        if archetype == "bridge" {
            continue;
        }
        let key = format!("{source}:{}", name.as_str());
        commands.entity(entity).insert(MapPropInstance {
            solid: solid_ids.contains(key.as_str()),
            key,
            archetype: archetype.into(),
            role: role.into(),
            desired_model: None,
            active_model: None,
            geometry_ready: false,
            authored_model: None,
        });
        count += 1;
    }
}

fn initialize_props(
    mut commands: Commands,
    props: Query<(Entity, &Transform, &Children), (With<MapPropInstance>, Without<PropState>)>,
    children: Query<&Children>,
    nodes: Query<(&Transform, Option<&ChildOf>)>,
    drawables: Query<&Mesh3d>,
    meshes: Option<Res<Assets<Mesh>>>,
) {
    let Some(meshes) = meshes else {
        return;
    };
    for (entity, transform, original_children) in &props {
        let Some(bounds) = drawable_bounds(entity, &children, &nodes, &drawables, &meshes) else {
            continue;
        };
        commands.entity(entity).insert(PropState {
            authored_transform: *transform,
            original_children: original_children.iter().collect(),
            original_bounds: bounds,
            replacement: None,
            applied_revision: None,
            applied_key: String::new(),
            applied_archetype: String::new(),
            tint_dirty: true,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn reconcile_props(
    mut commands: Commands,
    registry: Res<MapVisualRegistry>,
    server: Option<Res<AssetServer>>,
    mut cache: ResMut<MapVisualCache>,
    mut props: Query<(Entity, &mut MapPropInstance, &mut PropState)>,
    children: Query<&Children>,
    nodes: Query<(&Transform, Option<&ChildOf>)>,
    drawables: Query<&Mesh3d>,
    meshes: Option<Res<Assets<Mesh>>>,
    instances: Query<&SceneInstance>,
    spawner: Option<Res<SceneSpawner>>,
) {
    for (entity, mut prop, mut state) in &mut props {
        let dirty = state.applied_revision != Some(registry.revision())
            || state.applied_key != prop.key
            || state.applied_archetype != prop.archetype;
        let pending = state
            .replacement
            .as_ref()
            .is_some_and(|(_, _, _, status)| *status == ReplacementStatus::Pending);
        if !dirty && !pending {
            continue;
        }
        if dirty {
            let profile = registry.resolve(&prop.archetype, &prop.key);
            let model = profile
                .model
                .as_ref()
                .filter(|model| !prop.solid || model.preserve_collision);
            let desired = model
                .map(MapModel::asset_path)
                .filter(|path| Some(path) != prop.authored_model.as_ref());
            prop.desired_model = profile.model.as_ref().map(MapModel::asset_path);
            if !prop.solid {
                commands
                    .entity(entity)
                    .insert(state.authored_transform.mul_transform(profile.transform()));
            }
            let current = state.replacement.as_ref().map(|(path, _, _, _)| path);
            if current != desired.as_ref() {
                if let Some((_, old, _, _)) = state.replacement.take() {
                    commands.entity(old).despawn();
                }
                if let Some(path) = desired.as_ref()
                    && let Some(server) = server.as_ref()
                    && let Some(scene) = cache.scene(path, server)
                {
                    let replacement = commands
                        .spawn((
                            SceneRoot(scene.clone()),
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            ReplacementRoot,
                            ChildOf(entity),
                            Name::new("Map prop / replacement"),
                        ))
                        .id();
                    state.replacement =
                        Some((path.clone(), replacement, scene, ReplacementStatus::Pending));
                }
            }
            state.applied_revision = Some(registry.revision());
            state.applied_key.clone_from(&prop.key);
            state.applied_archetype.clone_from(&prop.archetype);
            state.tint_dirty = true;
        }
        let mut status_changed = false;
        let original_bounds = state.original_bounds;
        if let Some((_, replacement, handle, status)) = state.replacement.as_mut()
            && *status == ReplacementStatus::Pending
        {
            let next = if server.as_ref().is_some_and(|server| {
                matches!(
                    server.load_state(handle.id()),
                    bevy::asset::LoadState::Failed(_)
                )
            }) {
                ReplacementStatus::Rejected
            } else if server
                .as_ref()
                .is_some_and(|s| s.is_loaded_with_dependencies(handle.id()))
                && instances.get(*replacement).ok().is_some_and(|instance| {
                    spawner
                        .as_ref()
                        .is_some_and(|spawner| spawner.instance_is_ready(**instance))
                })
            {
                let compatible = meshes
                    .as_ref()
                    .and_then(|meshes| {
                        drawable_bounds(*replacement, &children, &nodes, &drawables, meshes)
                    })
                    .is_some_and(|bounds| {
                        if prop.solid {
                            original_bounds.collision_compatible(bounds)
                        } else {
                            (bounds.max - bounds.min).max_element() <= 16.0
                        }
                    });
                if compatible {
                    ReplacementStatus::Ready
                } else {
                    ReplacementStatus::Rejected
                }
            } else {
                ReplacementStatus::Pending
            };
            status_changed = next != *status;
            *status = next;
        }
        if !dirty && !status_changed {
            continue;
        }
        let ready = state
            .replacement
            .as_ref()
            .is_some_and(|(_, _, _, status)| *status == ReplacementStatus::Ready);
        for original in &state.original_children {
            commands.entity(*original).insert(if ready {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });
        }
        if let Some((_, replacement, _, _)) = state.replacement.as_ref() {
            commands.entity(*replacement).insert(if ready {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
        prop.active_model = if ready {
            state
                .replacement
                .as_ref()
                .map(|(path, _, _, _)| path.clone())
        } else {
            prop.authored_model.clone()
        };
        prop.geometry_ready = true;
        state.tint_dirty |= status_changed;
    }
}

fn tint_props(
    mut commands: Commands,
    registry: Res<MapVisualRegistry>,
    mut props: Query<(Entity, &MapPropInstance, &mut PropState)>,
    children: Query<&Children>,
    drawables: Query<(
        Entity,
        &MeshMaterial3d<StandardMaterial>,
        Option<&OriginalMaterial>,
    )>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut cache: ResMut<MapVisualCache>,
) {
    let Some(materials) = materials.as_mut() else {
        return;
    };
    for (root, prop, mut state) in &mut props {
        if !state.tint_dirty {
            continue;
        }
        state.tint_dirty = false;
        let tint = registry
            .resolve(&prop.archetype, &prop.key)
            .tint
            .unwrap_or([1.0; 4]);
        for entity in children.iter_descendants(root) {
            let Ok((entity, current, original)) = drawables.get(entity) else {
                continue;
            };
            let original_handle = original.map_or_else(|| current.0.clone(), |v| v.0.clone());
            if original.is_none() {
                commands
                    .entity(entity)
                    .insert(OriginalMaterial(original_handle.clone()));
            }
            let desired = if tint == [1.0; 4] {
                original_handle.clone()
            } else {
                let key = (original_handle.id(), tint.map(f32::to_bits));
                if let Some(handle) = cache.materials.get(&key) {
                    handle.clone()
                } else if cache.materials.len() < MAX_MATERIALS {
                    let Some(mut material) = materials.get(&original_handle).cloned() else {
                        state.tint_dirty = true;
                        continue;
                    };
                    let base = material.base_color.to_srgba();
                    material.base_color = Color::srgba(
                        base.red * tint[0],
                        base.green * tint[1],
                        base.blue * tint[2],
                        base.alpha,
                    );
                    let handle = materials.add(material);
                    cache.materials.insert(key, handle.clone());
                    handle
                } else {
                    original_handle.clone()
                }
            };
            if current.0 != desired {
                commands.entity(entity).insert(MeshMaterial3d(desired));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_palette_and_instance_precedence_restore_defaults() {
        let mut registry = MapVisualRegistry::from_json(r#"{
          "schema_version":1,"palettes":{"warm":[1,0.5,0.2,1]},
          "archetypes":{"lantern":{"palette":"warm","scale":[2,2,2]},"tower_green":{"tint":[0.4,1,0.4,1]}},
          "instances":{"one":{"tint":[0.5,0.5,1,1]}}
        }"#).unwrap();
        let profile = registry.resolve("lantern", "one");
        assert_eq!(profile.tint, Some([0.5, 0.5, 1.0, 1.0]));
        assert_eq!(profile.scale, Some([2.0; 3]));
        assert_eq!(
            registry.resolve("lantern", "two").tint,
            Some([1.0, 0.5, 0.2, 1.0])
        );
        assert!(registry.resolve("tower_green", "unknown").model.is_some());
        registry.replace_json(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(registry.revision(), 1);
        assert_eq!(
            registry.resolve("lantern", "one"),
            MapVisualProfile::default()
        );
        assert_eq!(
            registry.resolve("tower_green", "one").sprite_key.as_deref(),
            Some("green_tower")
        );
    }

    #[test]
    fn invalid_cosmetics_fail_atomically_and_cannot_hide_or_tune_gameplay() {
        let mut registry = MapVisualRegistry::default();
        for profile in [
            r#"{"model":{"path":"../outside.glb"}}"#,
            r#"{"model":{"path":"https://host/model.glb"}}"#,
            r#"{"model":{"path":"map-props/lantern.glb","scene":99}}"#,
            r#"{"tint":[1,1,1,0]}"#,
            r#"{"scale":[-1,1,1]}"#,
            r#"{"offset":[100,0,0]}"#,
            r#"{"palette":"missing"}"#,
            r#"{"hp":5000}"#,
            r#"{"hidden":true}"#,
        ] {
            let json = format!(r#"{{"schema_version":1,"archetypes":{{"lantern":{profile}}}}}"#);
            assert!(registry.replace_json(&json).is_err(), "{json}");
        }
        assert_eq!(registry.revision(), 0);
        assert!(registry.resolve("tower_green", "").model.is_some());
    }

    #[test]
    fn solid_replacement_checks_extent_pivot_and_ground() {
        let original = MeshBounds {
            min: Vec3::new(-2.0, 0.0, -3.0),
            max: Vec3::new(2.0, 8.0, 3.0),
            meshes: 1,
        };
        assert!(original.collision_compatible(original));
        assert!(!original.collision_compatible(MeshBounds {
            max: Vec3::new(4.0, 8.0, 3.0),
            ..original
        }));
        assert!(!original.collision_compatible(MeshBounds {
            min: original.min + Vec3::X,
            max: original.max + Vec3::X,
            ..original
        }));
        assert!(!original.collision_compatible(MeshBounds {
            min: original.min + Vec3::Y,
            max: original.max + Vec3::Y,
            ..original
        }));
    }

    fn headless() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin {
                file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
                ..default()
            })
            .add_plugins((
                bevy::mesh::MeshPlugin,
                bevy::scene::ScenePlugin,
                bevy::transform::TransformPlugin,
            ))
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<bevy::animation::AnimationClip>()
            .add_plugins(bevy::gltf::GltfPlugin::default())
            .register_type::<Name>()
            .register_type::<Transform>()
            .register_type::<GlobalTransform>()
            .register_type::<bevy::transform::components::TransformTreeChanged>()
            .register_type::<Children>()
            .register_type::<ChildOf>()
            .register_type::<Visibility>()
            .register_type::<InheritedVisibility>()
            .register_type::<ViewVisibility>()
            .register_type::<Mesh3d>()
            .register_type::<MeshMaterial3d<StandardMaterial>>()
            .register_type::<bevy::camera::primitives::Aabb>()
            .register_type::<GltfExtras>()
            .register_type::<bevy::gltf::GltfSceneExtras>()
            .register_type::<bevy::gltf::GltfMeshExtras>()
            .register_type::<bevy::gltf::GltfMaterialExtras>()
            .register_type::<bevy::gltf::GltfMaterialName>()
            .register_type::<bevy::gltf::GltfMeshName>()
            .insert_resource(PlayerVisualMode::Models3d)
            .add_plugins(MapVisualsPlugin);
        app.finish();
        app.cleanup();
        app
    }

    fn pump(app: &mut App, mut done: impl FnMut(&World) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !done(app.world()) {
            assert!(
                std::time::Instant::now() < deadline,
                "map scene did not become ready"
            );
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
    }

    fn descendants(world: &World, root: Entity) -> Vec<Entity> {
        let mut result = Vec::new();
        let mut pending = vec![root];
        while let Some(parent) = pending.pop() {
            if let Some(children) = world.get::<Children>(parent) {
                for child in children.iter() {
                    result.push(child);
                    pending.push(child);
                }
            }
        }
        result
    }

    #[test]
    fn shipped_glb_wrappers_import_all_real_props_and_keep_ground_unbound() {
        let mut app = headless();
        let environment_scene: Handle<Scene> = app
            .world()
            .resource::<AssetServer>()
            .load("verdant/environment.glb#Scene0");
        let foliage_scene: Handle<Scene> = app
            .world()
            .resource::<AssetServer>()
            .load("verdant/foliage.glb#Scene0");
        let environment = app
            .world_mut()
            .spawn((
                VerdantEnvironment,
                SceneRoot(environment_scene),
                Transform::IDENTITY,
            ))
            .id();
        let foliage = app
            .world_mut()
            .spawn((
                VerdantFoliage,
                crate::decor::DecorRoot,
                SceneRoot(foliage_scene),
                Transform::IDENTITY,
            ))
            .id();
        pump(&mut app, |world| {
            [environment, foliage]
                .into_iter()
                .flat_map(|root| descendants(world, root))
                .filter_map(|entity| world.get::<MapPropInstance>(entity))
                .filter(|p| p.geometry_ready)
                .count()
                == 942
        });
        let bindings: Vec<_> = [environment, foliage]
            .into_iter()
            .flat_map(|root| descendants(app.world(), root))
            .filter_map(|entity| app.world().get::<MapPropInstance>(entity))
            .collect();
        assert_eq!(
            bindings
                .iter()
                .filter(|p| p.key.starts_with("environment.glb:"))
                .count(),
            46
        );
        assert_eq!(
            bindings
                .iter()
                .filter(|p| p.key.starts_with("foliage.glb:"))
                .count(),
            896
        );
        assert_eq!(bindings.iter().filter(|p| p.solid).count(), 236);
        assert_eq!(
            bindings.iter().filter(|p| p.archetype == "lantern").count(),
            19
        );
        assert!(bindings.iter().all(|p| p.archetype != "bridge"));
        assert!(bindings.iter().all(|p| p.active_model.is_none()));
        let unique: HashSet<_> = bindings.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(unique.len(), 942);
    }

    #[test]
    fn actual_loaded_prop_a_b_a_has_no_stale_descendants_and_bounded_cache() {
        let mut app = headless();
        let scene: Handle<Scene> = app
            .world()
            .resource::<AssetServer>()
            .load("map-props/lantern.glb#Scene0");
        let environment = app
            .world_mut()
            .spawn((
                VerdantEnvironment,
                Transform::IDENTITY,
                Visibility::default(),
            ))
            .id();
        let prop = app
            .world_mut()
            .spawn((
                Name::new("lantern / fixture"),
                GltfExtras {
                    value: r#"{"asset_id":"lantern","role":"base_accent"}"#.into(),
                },
                Transform::from_xyz(4.0, 0.0, 2.0),
                Visibility::default(),
                ChildOf(environment),
            ))
            .id();
        app.world_mut()
            .spawn((SceneRoot(scene), Transform::IDENTITY, ChildOf(prop)));
        pump(&mut app, |world| {
            world.resource::<MapVisualRegistry>().revision() > 0
                && world
                    .get::<MapPropInstance>(prop)
                    .is_some_and(|p| p.geometry_ready)
        });
        let authored = *app.world().get::<Transform>(prop).unwrap();
        let mut old_descendants = Vec::new();
        let mut a_bounds = None;
        for name in [
            "lantern",
            "flowering_shrub",
            "lantern",
            "flowering_shrub",
            "lantern",
        ] {
            let path = format!("map-props/{name}.glb#Scene0");
            let json = format!(
                r#"{{"schema_version":1,"instances":{{"environment.glb:lantern / fixture":{{"model":{{"path":"map-props/{name}.glb"}}}}}}}}"#
            );
            app.world_mut()
                .resource_mut::<MapVisualRegistry>()
                .replace_json(&json)
                .unwrap();
            pump(&mut app, |world| {
                world
                    .get::<MapPropInstance>(prop)
                    .unwrap()
                    .active_model
                    .as_deref()
                    == Some(path.as_str())
            });
            for old in old_descendants.drain(..) {
                assert!(app.world().get_entity(old).is_err());
            }
            let replacement = app
                .world()
                .get::<PropState>(prop)
                .unwrap()
                .replacement
                .as_ref()
                .unwrap()
                .1;
            old_descendants = descendants(app.world(), replacement);
            old_descendants.push(replacement);
            assert!(old_descendants.len() > 2);
            assert_eq!(*app.world().get::<Transform>(prop).unwrap(), authored);
            let (models, materials) = app.world().resource::<MapVisualCache>().counts();
            assert!(models <= 2 && materials == 0);
            let current_meshes = app.world_mut().query::<&Mesh3d>().iter(app.world()).count();
            if name == "lantern" {
                if let Some(expected) = a_bounds {
                    assert_eq!(current_meshes, expected);
                }
                a_bounds = Some(current_meshes);
            }
        }
        app.world_mut()
            .resource_mut::<MapVisualRegistry>()
            .replace_json(r#"{"schema_version":1}"#)
            .unwrap();
        app.update();
        assert!(
            app.world()
                .get::<PropState>(prop)
                .unwrap()
                .replacement
                .is_none()
        );
        assert!(
            app.world()
                .get::<MapPropInstance>(prop)
                .unwrap()
                .active_model
                .is_none()
        );
        for old in old_descendants {
            assert!(app.world().get_entity(old).is_err());
        }
        app.world_mut().despawn(environment);
        assert!(app.world().get_entity(prop).is_err());
    }
}
