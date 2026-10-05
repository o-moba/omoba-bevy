//! The authored Verdant scene is presentation only. Network entities remain
//! the owners of live structures; the static GLBs exclude all eight copies.
use bevy::prelude::*;
use std::collections::HashMap;

use crate::combat::CombatStats;
use crate::decor::DecorRoot;
use crate::map_visuals::MapPropInstance;
use crate::maps::MapLayout;
use crate::net::{NetworkMapStructure, NetworkStructure, StructureKind};
use crate::sprite::{PlayerVisualMode, in_models3d};
use crate::team::Team;

pub struct Verdant3dPlugin;

impl Plugin for Verdant3dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VerdantPaletteMaterials>()
            .add_systems(
                Startup,
                (load_assets, spawn_environment)
                    .chain()
                    .run_if(in_models3d()),
            )
            .add_systems(
                PostUpdate,
                (
                    reconcile_structures,
                    apply_environment_palette,
                    animate_nexus_rings,
                )
                    .before(bevy::transform::TransformSystems::Propagate)
                    .run_if(in_models3d()),
            );
    }
}

/// One persistent world root, independent of round/network teardown.
#[derive(Component)]
pub struct VerdantEnvironment;

/// The separately owned foliage layer shares the existing F4 debug toggle.
#[derive(Component)]
pub struct VerdantFoliage;

/// Exactly one scene child belongs to each authoritative structure root.
#[derive(Component)]
pub struct VerdantStructureVisual {
    pub owner: Entity,
}

#[derive(Component)]
struct AttachedStructure(Entity);

/// The imported ring meshes have their own crystal-centred pivots.
fn animate_nexus_rings(
    time: Res<Time>,
    roots: Query<&VerdantStructureVisual>,
    kinds: Query<&StructureKind, With<NetworkStructure>>,
    parents: Query<&ChildOf>,
    mut rings: Query<(Entity, &Name, &mut Transform), (Without<NetworkStructure>, Without<Mesh3d>)>,
) {
    for (entity, name, mut transform) in &mut rings {
        let Some(index) = name
            .as_str()
            .strip_prefix("NexusOrbit-")
            .and_then(|n| n.parse::<usize>().ok())
        else {
            continue;
        };
        let Some(speed) = [0.24_f32, -0.17, 0.31].get(index) else {
            continue;
        };
        // Names alone are insufficient: only descendants of live base visuals animate.
        let mut ancestor = entity;
        let mut is_nexus = false;
        for _ in 0..32 {
            if let Ok(root) = roots.get(ancestor) {
                is_nexus = kinds
                    .get(root.owner)
                    .is_ok_and(|kind| *kind == StructureKind::BaseTower);
                break;
            }
            let Ok(parent) = parents.get(ancestor) else {
                break;
            };
            ancestor = parent.parent();
        }
        if !is_nexus {
            continue;
        }
        transform.rotation =
            Quat::from_rotation_y((time.elapsed_secs() * speed).rem_euclid(std::f32::consts::TAU));
    }
}

fn structure_presentation_scale(kind: StructureKind, tier: Option<u8>) -> Vec3 {
    if kind == StructureKind::Tower && tier == Some(2) {
        // Inner guards keep a broad foundation and a lower beacon silhouette.
        Vec3::new(0.88, 0.68, 0.88)
    } else {
        Vec3::ONE
    }
}

/// Each authored material has one presentation-only copy shared by all of its
/// map instances. Never edit the source asset: another scene can reuse it.
#[derive(Resource, Default)]
pub(crate) struct VerdantPaletteMaterials(
    HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
);

impl VerdantPaletteMaterials {
    #[cfg(any(test, feature = "qa"))]
    pub(crate) fn count(&self) -> usize {
        self.0.len()
    }

    #[cfg(any(test, feature = "qa"))]
    pub(crate) fn contains_tuned(&self, handle: &Handle<StandardMaterial>) -> bool {
        self.0.values().any(|tuned| tuned == handle)
    }
}

#[derive(Component)]
struct VerdantPaletteApplied;

/// Linear RGB, matching glTF's baseColorFactor. The floor stays brighter than
/// the banks, while cool midtones give gold/cyan skill cores room to stand out.
/// Live structures and emissive team beacons keep their authored look.
fn environment_palette(name: &str) -> Option<[f32; 3]> {
    Some(match name {
        "VC / worn ceremonial paving" => [0.22, 0.265, 0.26],
        "VC / ivory limestone" => [0.34, 0.36, 0.32],
        "VC / cut limestone" => [0.46, 0.48, 0.42],
        "VC / moss meadow" => [0.105, 0.205, 0.13],
        "VC / moss shadow" => [0.075, 0.145, 0.105],
        "VC / moss light" => [0.155, 0.255, 0.155],
        "VC / earth and clay" => [0.145, 0.135, 0.095],
        "VC / slate strata" => [0.17, 0.215, 0.205],
        "VC / weathered stone" => [0.155, 0.215, 0.195],
        "VC / deep turquoise water" => [0.025, 0.135, 0.17],
        "VC / jade shallows" => [0.055, 0.235, 0.24],
        "VC / water glints" => [0.20, 0.375, 0.35],
        "VC / deep teal canopy" => [0.035, 0.125, 0.10],
        "VC / jade canopy" => [0.07, 0.23, 0.13],
        "VC / sage canopy" => [0.17, 0.30, 0.17],
        "VC / warm wildflower" => [0.54, 0.36, 0.13],
        // The repaired river has authored colors in its vertex attributes and
        // a white material. This multiplier covers that active surface too.
        "River / continuous joined surface" => [0.8, 0.58, 0.64],
        _ => return None,
    })
}

fn apply_environment_palette(
    mut commands: Commands,
    mut cache: ResMut<VerdantPaletteMaterials>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    roots: Query<(), Or<(With<VerdantEnvironment>, With<VerdantFoliage>)>>,
    parents: Query<&ChildOf>,
    mut meshes: Query<
        (
            Entity,
            Option<&bevy::gltf::GltfMaterialName>,
            Option<&Name>,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<VerdantPaletteApplied>,
    >,
) {
    let Some(mut materials) = materials else {
        return;
    };
    for (entity, gltf_name, entity_name, mut binding) in &mut meshes {
        // glTF primitive entities can be nested several nodes below SceneRoot.
        // The root check keeps avatars, live towers and skill props untouched,
        // even if an imported material happens to have the same name.
        let mut ancestor = entity;
        let mut is_environment = false;
        for _ in 0..32 {
            if roots.contains(ancestor) {
                is_environment = true;
                break;
            }
            let Ok(parent) = parents.get(ancestor) else {
                break;
            };
            ancestor = parent.parent();
        }
        if !is_environment {
            continue;
        }
        let name = gltf_name.map_or_else(
            || entity_name.map_or("", Name::as_str),
            |name| name.0.as_str(),
        );
        if let Some([red, green, blue]) = environment_palette(name) {
            let source = binding.0.id();
            let tuned = if let Some(existing) = cache.0.get(&source) {
                existing.clone()
            } else {
                // Scene dependencies may finish loading on a later frame.
                let Some(original) = materials.get(&binding.0) else {
                    continue;
                };
                let mut material = original.clone();
                material.base_color =
                    Color::linear_rgba(red, green, blue, original.base_color.alpha());
                // Broad glossy floor/river highlights compete with spell cores.
                material.perceptual_roughness = material.perceptual_roughness.max(0.82);
                material.reflectance = material.reflectance.min(0.25);
                let handle = materials.add(material);
                cache.0.insert(source, handle.clone());
                handle
            };
            binding.0 = tuned;
        }
        commands.entity(entity).insert(VerdantPaletteApplied);
    }
}

#[derive(Resource, Clone)]
struct VerdantAssets {
    environment: Handle<Scene>,
    foliage: Handle<Scene>,
    watchtower_green: Handle<Scene>,
    watchtower_blue: Handle<Scene>,
    sanctuary_green: Handle<Scene>,
    sanctuary_blue: Handle<Scene>,
}

impl VerdantAssets {
    fn structure(&self, kind: StructureKind, team: Team) -> Handle<Scene> {
        match (kind, team) {
            (StructureKind::Tower, Team::Green) => self.watchtower_green.clone(),
            (StructureKind::Tower, Team::Blue) => self.watchtower_blue.clone(),
            (StructureKind::BaseTower, Team::Green) => self.sanctuary_green.clone(),
            (StructureKind::BaseTower, Team::Blue) => self.sanctuary_blue.clone(),
        }
    }
}

pub(crate) fn default_structure_profile(kind: StructureKind, team: Team) -> &'static str {
    match (kind, team) {
        (StructureKind::Tower, Team::Green) => "tower_green",
        (StructureKind::Tower, Team::Blue) => "tower_blue",
        (StructureKind::BaseTower, Team::Green) => "base_green",
        (StructureKind::BaseTower, Team::Blue) => "base_blue",
    }
}

fn default_structure_model(kind: StructureKind, team: Team) -> &'static str {
    match (kind, team) {
        (StructureKind::Tower, Team::Green) => "verdant/watchtower_green.glb#Scene0",
        (StructureKind::Tower, Team::Blue) => "verdant/watchtower_blue.glb#Scene0",
        (StructureKind::BaseTower, Team::Green) => "verdant/sanctuary_green.glb#Scene0",
        (StructureKind::BaseTower, Team::Blue) => "verdant/sanctuary_blue.glb#Scene0",
    }
}

fn load_assets(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    server: Option<Res<AssetServer>>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(server) = server else {
        return;
    };
    commands.insert_resource(VerdantAssets {
        environment: server.load("verdant/environment.glb#Scene0"),
        foliage: server.load("verdant/foliage.glb#Scene0"),
        watchtower_green: server.load("verdant/watchtower_green.glb#Scene0"),
        watchtower_blue: server.load("verdant/watchtower_blue.glb#Scene0"),
        sanctuary_green: server.load("verdant/sanctuary_green.glb#Scene0"),
        sanctuary_blue: server.load("verdant/sanctuary_blue.glb#Scene0"),
    });
}

fn spawn_environment(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    assets: Option<Res<VerdantAssets>>,
    existing: Query<Entity, With<VerdantEnvironment>>,
) {
    if *mode != PlayerVisualMode::Models3d || !existing.is_empty() {
        return;
    }
    let Some(assets) = assets else { return };
    // Source export is already Y-up, one meter per unit. Do not reorient it.
    commands.spawn((
        VerdantEnvironment,
        SceneRoot(assets.environment.clone()),
        Transform::from_scale(Vec3::new(
            shared::map::WORLD_SCALE,
            1.0,
            shared::map::WORLD_SCALE,
        )),
        Name::new("Verdant / environment"),
    ));
    commands.spawn((
        VerdantFoliage,
        DecorRoot,
        SceneRoot(assets.foliage.clone()),
        Transform::from_scale(Vec3::new(
            shared::map::WORLD_SCALE,
            1.0,
            shared::map::WORLD_SCALE,
        )),
        Name::new("Verdant / foliage (F4)"),
    ));
    info!(
        "Verdant: shared environment, foliage and four live structure scenes (Y-up meters); authoritative center lane {:.1} m",
        layout.center_lane_distance()
    );
}

fn structure_transform(layout: &MapLayout, root: &Transform, kind: StructureKind) -> Transform {
    let ground = layout.terrain_height_3d(root.translation.x, root.translation.z);
    // The authoritative root's Y is the legacy box center. Keep that root
    // intact and move only its presentation child to the exported ground pivot.
    let foundation = if kind == StructureKind::Tower {
        0.02
    } else {
        0.0
    };
    let rotation = if kind == StructureKind::BaseTower {
        // Open the diagonal spawn approach: authored diagonal ribs otherwise
        // coincide with the player's unchanged seven-meter spawn offset.
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)
    } else {
        Quat::IDENTITY
    };
    Transform::from_xyz(0.0, ground + foundation - root.translation.y, 0.0).with_rotation(rotation)
}

fn reconcile_structures(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    assets: Option<Res<VerdantAssets>>,
    roots: Query<
        (
            Entity,
            &Transform,
            &StructureKind,
            &Team,
            &CombatStats,
            Option<&AttachedStructure>,
            Option<&NetworkMapStructure>,
        ),
        With<NetworkStructure>,
    >,
    mut visuals: Query<
        (
            &SceneRoot,
            &mut Transform,
            &mut Visibility,
            &VerdantStructureVisual,
            Option<&mut MapPropInstance>,
        ),
        Without<NetworkStructure>,
    >,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(assets) = assets else { return };
    for (owner, root, kind, team, stats, attached, config) in &roots {
        let key = config.filter(|v| !v.key.is_empty()).map_or_else(
            || format!("structure:{}", owner.to_bits()),
            |v| v.key.clone(),
        );
        let profile = config.filter(|v| !v.visual_profile.is_empty()).map_or_else(
            || default_structure_profile(*kind, *team).to_owned(),
            |v| v.visual_profile.clone(),
        );
        let transform = structure_transform(&layout, root, *kind)
            .with_scale(structure_presentation_scale(*kind, config.map(|c| c.tier)));
        let visibility = if stats.hp > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let scene = assets.structure(*kind, *team);
        if let Some(attached) = attached
            && let Ok((
                current_scene,
                mut current_transform,
                mut current_visibility,
                visual,
                marker,
            )) = visuals.get_mut(attached.0)
            && visual.owner == owner
        {
            if current_scene.0 == scene {
                *current_transform = transform;
                *current_visibility = visibility;
                if let Some(mut marker) = marker {
                    marker.key = key;
                    marker.archetype = profile;
                }
                continue;
            }
            // A restarted server can reuse an ID for another kind/team. The
            // old presentation owns both authored mesh IDs and cached bounds;
            // replace that child as a unit while preserving the network owner.
            commands.entity(attached.0).despawn();
        }
        let child = commands
            .spawn((
                SceneRoot(scene),
                transform,
                visibility,
                VerdantStructureVisual { owner },
                MapPropInstance::structure(
                    key,
                    profile,
                    default_structure_model(*kind, *team).into(),
                ),
                Name::new(format!("Verdant / {team:?} {kind:?}")),
            ))
            .id();
        commands
            .entity(owner)
            .add_child(child)
            .insert(AttachedStructure(child));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_towers_have_a_shorter_guard_silhouette_only() {
        assert_eq!(
            structure_presentation_scale(StructureKind::Tower, Some(0)),
            Vec3::ONE
        );
        assert_eq!(
            structure_presentation_scale(StructureKind::Tower, Some(1)),
            Vec3::ONE
        );
        let inner = structure_presentation_scale(StructureKind::Tower, Some(2));
        assert!(inner.y < inner.x && inner.x < 1.0);
        assert_eq!(
            structure_presentation_scale(StructureKind::BaseTower, Some(2)),
            Vec3::ONE
        );
    }

    #[test]
    fn nexus_rings_rotate_around_their_pivots_without_moving_the_base() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .add_systems(Update, animate_nexus_rings);
        let base = app
            .world_mut()
            .spawn((
                NetworkStructure,
                StructureKind::BaseTower,
                Transform::IDENTITY,
            ))
            .id();
        let visual = app
            .world_mut()
            .spawn(VerdantStructureVisual { owner: base })
            .id();
        let pivot = Vec3::new(0.0, 7.33, 0.0);
        let ring = app
            .world_mut()
            .spawn((
                Name::new("NexusOrbit-0"),
                Transform::from_translation(pivot),
                ChildOf(visual),
            ))
            .id();
        let foreign = app
            .world_mut()
            .spawn((Name::new("NexusOrbit-0"), Transform::IDENTITY))
            .id();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(2));
        app.update();
        let actual = app.world().get::<Transform>(ring).unwrap();
        assert_eq!(actual.translation, pivot);
        assert_ne!(actual.rotation, Quat::IDENTITY);
        assert_eq!(
            *app.world().get::<Transform>(base).unwrap(),
            Transform::IDENTITY
        );
        assert_eq!(
            *app.world().get::<Transform>(foreign).unwrap(),
            Transform::IDENTITY
        );
    }

    #[test]
    fn shipped_scene_uses_cool_paving_with_map_cosmetics_and_repaired_river() {
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
            .register_type::<bevy::gltf::GltfExtras>()
            .register_type::<bevy::gltf::GltfSceneExtras>()
            .register_type::<bevy::gltf::GltfMeshExtras>()
            .register_type::<bevy::gltf::GltfMaterialExtras>()
            .register_type::<bevy::gltf::GltfMaterialName>()
            .register_type::<bevy::gltf::GltfMeshName>()
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<MapLayout>()
            .add_plugins((Verdant3dPlugin, crate::map_visuals::MapVisualsPlugin));
        app.finish();
        app.cleanup();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while app.world().resource::<VerdantPaletteMaterials>().count() < 17 {
            assert!(
                std::time::Instant::now() < deadline,
                "packaged palette did not load: {} copies",
                app.world().resource::<VerdantPaletteMaterials>().count()
            );
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        // Let map instance discovery and original-material capture settle.
        for _ in 0..20 {
            app.update();
        }
        let mut query = app.world_mut().query::<(
            Option<&bevy::gltf::GltfMaterialName>,
            Option<&Name>,
            &MeshMaterial3d<StandardMaterial>,
        )>();
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let palette = app.world().resource::<VerdantPaletteMaterials>();
        let mut paving = 0;
        let mut rivers = 0;
        for (name, node_name, handle) in query.iter(app.world()) {
            if name.is_some_and(|name| name.0 == "VC / worn ceremonial paving") {
                let material = materials.get(&handle.0).unwrap();
                assert_eq!(material.base_color, Color::linear_rgb(0.22, 0.265, 0.26));
                assert!(palette.contains_tuned(&handle.0));
                paving += 1;
            }
            if node_name.is_some_and(|name| name.as_str() == "River / continuous joined surface") {
                let material = materials.get(&handle.0).unwrap();
                assert_eq!(material.base_color, Color::linear_rgb(0.8, 0.58, 0.64));
                assert!(palette.contains_tuned(&handle.0));
                rivers += 1;
            }
        }
        assert!(
            paving >= 20,
            "all authored lane/base paving is actually loaded"
        );
        assert_eq!(rivers, 1, "the active replacement river is also graded");
    }

    #[test]
    fn palette_copies_only_environment_materials_and_reuses_each_copy() {
        let mut app = App::new();
        app.init_resource::<Assets<StandardMaterial>>()
            .init_resource::<VerdantPaletteMaterials>()
            .add_systems(Update, apply_environment_palette);
        let source = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::linear_rgba(0.49, 0.455, 0.34, 0.8),
                perceptual_roughness: 0.3,
                reflectance: 0.5,
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                ..default()
            });
        let environment = app.world_mut().spawn(VerdantEnvironment).id();
        let intermediate = app.world_mut().spawn(ChildOf(environment)).id();
        let foliage = app.world_mut().spawn(VerdantFoliage).id();
        let tower = app
            .world_mut()
            .spawn(VerdantStructureVisual { owner: environment })
            .id();
        let mut primitives = Vec::new();
        for parent in [Some(intermediate), Some(foliage), Some(tower), None] {
            let mut entity = app.world_mut().spawn((
                bevy::gltf::GltfMaterialName("VC / worn ceremonial paving".into()),
                MeshMaterial3d(source.clone()),
            ));
            if let Some(parent) = parent {
                entity.insert(ChildOf(parent));
            }
            primitives.push(entity.id());
        }
        let beacon = app
            .world_mut()
            .spawn((
                ChildOf(environment),
                bevy::gltf::GltfMaterialName("VC / emerald beacon".into()),
                MeshMaterial3d(source.clone()),
            ))
            .id();
        for _ in 0..3 {
            app.update();
        }
        let material = |entity| {
            &app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .unwrap()
                .0
        };
        assert_ne!(material(primitives[0]), &source);
        assert_eq!(material(primitives[0]), material(primitives[1]));
        assert_eq!(
            material(primitives[2]),
            &source,
            "live structures keep authored colors"
        );
        assert_eq!(
            material(primitives[3]),
            &source,
            "non-map meshes must remain untouched"
        );
        assert_eq!(
            material(beacon),
            &source,
            "team beacons keep their authored material"
        );
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        assert_eq!(
            materials.len(),
            2,
            "one cached copy, without per-frame growth"
        );
        assert_eq!(
            materials.get(&source).unwrap().base_color,
            Color::linear_rgba(0.49, 0.455, 0.34, 0.8)
        );
        let tuned = materials.get(material(primitives[0])).unwrap();
        assert_eq!(tuned.base_color.alpha(), 0.8);
        assert_eq!(tuned.alpha_mode, AlphaMode::Blend);
        assert!(tuned.double_sided);
        assert!(tuned.perceptual_roughness >= 0.8);
        assert!(tuned.reflectance < 0.5);
    }

    #[test]
    fn cool_midtones_preserve_floor_bank_and_landmark_hierarchy() {
        let luminance = |name| {
            let [r, g, b] = environment_palette(name).unwrap();
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let paving = luminance("VC / worn ceremonial paving");
        assert!(paving > luminance("VC / moss meadow"));
        assert!(paving > luminance("VC / deep turquoise water"));
        assert!(paving < luminance("VC / cut limestone"));
        assert!(
            paving > 0.20 && paving < 0.30,
            "readable midtone rather than a black floor"
        );
        assert!(environment_palette("VC / azure beacon").is_none());
        assert!(environment_palette("avatar skin").is_none());
    }

    fn fixture(mode: PlayerVisualMode) -> App {
        let mut app = App::new();
        app.insert_resource(mode).init_resource::<MapLayout>();
        let mut scenes = Assets::<Scene>::default();
        let mut scene = || scenes.add(Scene::new(World::new()));
        app.insert_resource(VerdantAssets {
            environment: scene(),
            foliage: scene(),
            watchtower_green: scene(),
            watchtower_blue: scene(),
            sanctuary_green: scene(),
            sanctuary_blue: scene(),
        });
        app.insert_resource(scenes)
            .add_systems(Update, (spawn_environment, reconcile_structures).chain());
        app
    }

    fn structure(app: &mut App, kind: StructureKind, team: Team) -> Entity {
        let layout = *app.world().resource::<MapLayout>();
        let location = match kind {
            StructureKind::Tower => Vec3::new(20.0, 2.0, 20.0),
            StructureKind::BaseTower => match team {
                Team::Green => layout.home_spawn,
                Team::Blue => layout.away_spawn,
            },
        };
        app.world_mut()
            .spawn((
                NetworkStructure,
                kind,
                team,
                Transform::from_translation(location),
                CombatStats {
                    hp: 100.0,
                    max_hp: 100.0,
                    ..default()
                },
            ))
            .id()
    }

    #[test]
    fn sprite2d_never_loads_or_spawns_the_verdant_scene() {
        let mut app = App::new();
        app.insert_resource(PlayerVisualMode::Sprite2d)
            .init_resource::<MapLayout>()
            .add_plugins(Verdant3dPlugin);
        app.update();
        assert!(!app.world().contains_resource::<VerdantAssets>());
        assert_eq!(
            app.world_mut()
                .query::<&SceneRoot>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn repeated_snapshots_keep_one_environment_and_eight_shared_structure_children() {
        let mut app = fixture(PlayerVisualMode::Models3d);
        let mut owners = Vec::new();
        for team in [Team::Green, Team::Blue] {
            owners.push(structure(&mut app, StructureKind::BaseTower, team));
            for _ in 0..3 {
                owners.push(structure(&mut app, StructureKind::Tower, team));
            }
        }
        for _ in 0..20 {
            app.update();
        }
        assert_eq!(
            app.world_mut()
                .query::<&VerdantEnvironment>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantFoliage>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantStructureVisual>()
                .iter(app.world())
                .count(),
            8
        );
        assert_eq!(app.world().resource::<Assets<Scene>>().len(), 6);
        let assets = app.world().resource::<VerdantAssets>();
        for owner in &owners {
            let entity = app.world().entity(*owner);
            let child = entity.get::<AttachedStructure>().unwrap().0;
            assert_eq!(entity.get::<Children>().unwrap().len(), 1);
            let visual = app.world().entity(child);
            assert_eq!(
                visual.get::<VerdantStructureVisual>().unwrap().owner,
                *owner
            );
            assert_eq!(visual.get::<ChildOf>().unwrap().parent(), *owner);
            assert_eq!(
                visual.get::<SceneRoot>().unwrap().0,
                assets.structure(
                    *entity.get::<StructureKind>().unwrap(),
                    *entity.get::<Team>().unwrap()
                )
            );
            assert!(entity.get::<Mesh3d>().is_none());
        }
    }

    #[test]
    fn damage_death_and_rematch_preserve_hp_ownership_without_scene_accumulation() {
        let mut app = fixture(PlayerVisualMode::Models3d);
        let owner = structure(&mut app, StructureKind::Tower, Team::Green);
        app.update();
        let child = app.world().get::<AttachedStructure>(owner).unwrap().0;
        let scene = app.world().get::<SceneRoot>(child).unwrap().0.clone();
        app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 37.0;
        app.update();
        assert_eq!(app.world().get::<CombatStats>(owner).unwrap().hp, 37.0);
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 0.0;
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Hidden
        );
        // A reset using the same server ID/entity restores the existing scene.
        app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 100.0;
        app.update();
        assert_eq!(
            app.world().get::<AttachedStructure>(owner).unwrap().0,
            child
        );
        assert_eq!(
            *app.world().get::<Visibility>(child).unwrap(),
            Visibility::Inherited
        );
        // Disconnect/full rematch teardown recursively removes the owned child.
        app.world_mut().despawn(owner);
        assert!(app.world().get_entity(child).is_err());
        let restored = structure(&mut app, StructureKind::Tower, Team::Green);
        app.update();
        let restored_child = app.world().get::<AttachedStructure>(restored).unwrap().0;
        assert_eq!(
            app.world().get::<SceneRoot>(restored_child).unwrap().0,
            scene
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantStructureVisual>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&VerdantEnvironment>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn reused_owner_kind_or_team_replaces_authored_visual_and_all_cached_descendants() {
        let mut app = fixture(PlayerVisualMode::Models3d);
        let owner = structure(&mut app, StructureKind::Tower, Team::Green);
        app.update();
        let original = app.world().get::<AttachedStructure>(owner).unwrap().0;
        let old_model = app
            .world_mut()
            .spawn((Transform::IDENTITY, ChildOf(original)))
            .id();
        app.world_mut()
            .entity_mut(owner)
            .insert(NetworkMapStructure {
                key: "same-id".into(),
                visual_profile: "custom-tower".into(),
                ..default()
            });
        app.update();
        assert_eq!(
            app.world().get::<AttachedStructure>(owner).unwrap().0,
            original,
            "profile changes use the normal safe replacement lifecycle"
        );
        for (kind, team, model) in [
            (
                StructureKind::BaseTower,
                Team::Blue,
                "verdant/sanctuary_blue.glb#Scene0",
            ),
            (
                StructureKind::Tower,
                Team::Green,
                "verdant/watchtower_green.glb#Scene0",
            ),
        ] {
            let previous = app.world().get::<AttachedStructure>(owner).unwrap().0;
            app.world_mut().entity_mut(owner).insert((kind, team));
            app.update();
            let current = app.world().get::<AttachedStructure>(owner).unwrap().0;
            assert_ne!(current, previous);
            assert!(app.world().get_entity(previous).is_err());
            assert!(app.world().get_entity(old_model).is_err());
            assert_eq!(app.world().get::<Children>(owner).unwrap().len(), 1);
            assert_eq!(app.world().get::<CombatStats>(owner).unwrap().hp, 100.0);
            let binding = app.world().get::<MapPropInstance>(current).unwrap();
            assert_eq!(binding.authored_model.as_deref(), Some(model));
            assert!(!binding.geometry_ready);
            assert_eq!(binding.key, "same-id");
        }
    }

    #[test]
    fn structure_offsets_cancel_authoritative_box_centers_and_open_spawn_approach() {
        let layout = MapLayout::default();
        let tower = Transform::from_xyz(20.0, 2.0, 20.0);
        let visual = structure_transform(&layout, &tower, StructureKind::Tower);
        assert!((tower.translation.y + visual.translation.y - 0.02).abs() < 0.00001);
        let base = Transform::from_translation(layout.home_spawn);
        let visual = structure_transform(&layout, &base, StructureKind::BaseTower);
        assert!((base.translation.y + visual.translation.y - 0.7).abs() < 0.00001);
        let spawn = layout.team_spawn(Team::Green) - layout.home_spawn;
        for i in 0..4 {
            let angle = std::f32::consts::FRAC_PI_4 + i as f32 * std::f32::consts::FRAC_PI_2;
            let foot = visual.rotation * Vec3::new(angle.cos() * 7.15, 0.0, angle.sin() * 7.15);
            assert!(
                foot.distance(spawn) > 4.0,
                "sanctuary rib blocks unchanged player spawn"
            );
        }
    }
}
