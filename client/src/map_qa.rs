//! Explicit native map proof: authoritative structures and real prop geometry.
//! QA changes only the spectator camera and validated cosmetic registry.
use crate::{
    camera::MainCamera,
    combat::CombatStats,
    help_overlay::HelpOverlayVisible,
    input_context::InputContextSet,
    map_visuals::{MapPropInstance, MapVisualCache, MapVisualRegistry},
    mobile_controls::MobileControls,
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkMapStructure, NetworkStructureId,
        NetworkStructureProtected, StructureKind,
    },
    presentation2d::MapStructure2dVisual,
    sprite::PlayerVisualMode,
    team::Team,
    verdant3d::VerdantStructureVisual,
    world2d::{WORLD_TILE_COLUMNS, WORLD_TILE_ROWS, World2dStatic, simulation_xz_to_render_xy},
};
use bevy::{
    app::AppExit,
    asset::RecursiveDependencyLoadState,
    camera::ScalingMode,
    ecs::system::SystemParam,
    mesh::VertexAttributeValues,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    scene::{SceneInstance, SceneSpawner},
    ui::FocusPolicy,
    window::PrimaryWindow,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

// Fixed Verdant v1 authored prop inventory; bridge remains terrain.
const STATIC_PROPS: &[(&str, usize)] = &[
    ("environment.glb:banner_blue", 2),
    ("environment.glb:banner_green", 2),
    ("environment.glb:lantern", 19),
    ("environment.glb:rock_stratified_outcrop", 5),
    ("environment.glb:ruin_wall", 15),
    ("environment.glb:ruined_arch", 3),
    ("foliage.glb:boulder_moss_flat", 39),
    ("foliage.glb:boulder_moss_tall", 64),
    ("foliage.glb:fallen_log", 5),
    ("foliage.glb:fern_cluster", 249),
    ("foliage.glb:flowering_shrub", 121),
    ("foliage.glb:grass_fan", 241),
    ("foliage.glb:river_reeds", 60),
    ("foliage.glb:rooted_stump", 4),
    ("foliage.glb:tree_cypress_spire", 22),
    ("foliage.glb:tree_jade_canopy", 23),
    ("foliage.glb:tree_river_pine", 22),
    ("foliage.glb:tree_sage_elder", 23),
    ("foliage.glb:tree_windswept_oak", 23),
];

const MODELS: [&str; 2] = ["map-props/lantern.glb", "map-props/flowering_shrub.glb"];
const FILES_3D: [&str; 5] = [
    "01-map-overview.png",
    "02-prop-a.png",
    "03-prop-b.png",
    "04-prop-a-restored.png",
    "05-prop-b-repeated.png",
];
const FILES_2D: [&str; 2] = ["01-map-overview.png", "02-tower-detail.png"];
pub(crate) struct MapQaPlugin;
impl Plugin for MapQaPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("OMOBA_VISUAL_QA_SCENARIO").as_deref() != Ok("map") {
            return;
        }
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let value = |key: &str, fallback: u32| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(fallback)
        };
        app.insert_resource(MapQa {
            directory,
            started: Instant::now(),
            timeout: Duration::from_secs(
                value("OMOBA_VISUAL_QA_TIMEOUT", 120).clamp(30, 600) as u64
            ),
            pixels: UVec2::new(value("OMOBA_QA_WIDTH", 1280), value("OMOBA_QA_HEIGHT", 720)),
            expected_structures: value("OMOBA_MAP_QA_EXPECTED_STRUCTURES", 8) as usize,
            stage: 0,
            settled: 0,
            configured: None,
            selected_key: None,
            focus: Vec3::ZERO,
            frames: Vec::new(),
            readbacks: BTreeSet::new(),
            operations: Vec::new(),
            readiness: serde_json::Value::Null,
        })
        .add_systems(Startup, label)
        .add_systems(PreUpdate, prepare.after(bevy::ui::UiSystems::Focus))
        .add_systems(Update, position_camera.after(InputContextSet::Actions))
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::ui::UiSystems::Layout),
        );
    }
}
#[derive(Resource)]
struct MapQa {
    directory: PathBuf,
    started: Instant,
    timeout: Duration,
    pixels: UVec2,
    expected_structures: usize,
    stage: usize,
    settled: u32,
    configured: Option<usize>,
    selected_key: Option<String>,
    focus: Vec3,
    frames: Vec<serde_json::Value>,
    readbacks: BTreeSet<usize>,
    operations: Vec<serde_json::Value>,
    readiness: serde_json::Value,
}
fn label(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(33.0),
            bottom: Val::Px(3.0),
            ..default()
        },
        Text::new("QA: live map objects · scripted cosmetic swaps"),
        TextFont {
            font_size: 10.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        FocusPolicy::Pass,
        ZIndex(200),
    ));
}
fn prepare(
    qa: Res<MapQa>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
) {
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != qa.pixels.x || window.physical_height() != qa.pixels.y {
            window
                .resolution
                .set_physical_resolution(qa.pixels.x, qa.pixels.y);
        }
    }
    if qa.stage != 0 {
        return;
    }
    for (name, mut interaction) in &mut buttons {
        if (session.is_connected()
            && !session.join_confirmed()
            && name.as_str() == "TeamGreenButton")
            || (session.join_confirmed() && help.0 && name.as_str() == "HelpDismissButton")
        {
            *interaction = Interaction::Pressed;
        }
    }
}
fn position_camera(
    qa: Res<MapQa>,
    mode: Res<PlayerVisualMode>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    let Ok((mut transform, mut projection)) = cameras.single_mut() else {
        return;
    };
    if *mode == PlayerVisualMode::Sprite2d {
        let center = if qa.stage == 0 {
            Vec2::ZERO
        } else {
            simulation_xz_to_render_xy(qa.focus)
        };
        let (position, orthographic) = camera_2d(center, qa.stage == 0);
        *transform = position;
        *projection = Projection::Orthographic(orthographic);
    } else {
        let (position, target) = if qa.stage == 0 {
            (Vec3::new(-180.0, 260.0, 180.0), Vec3::ZERO)
        } else {
            (qa.focus + Vec3::new(-8.5, 12.0, 8.5), qa.focus + Vec3::Y)
        };
        *transform = Transform::from_translation(position).looking_at(target, Vec3::Y);
        *projection = Projection::Perspective(PerspectiveProjection {
            fov: std::f32::consts::FRAC_PI_3,
            far: 1500.0,
            ..default()
        });
    }
}
// Match the production Camera2d depth origin. Moving it to z=999 clips
// negative terrain bands against Bevy's default far=1000 plane.
fn camera_2d(center: Vec2, overview: bool) -> (Transform, OrthographicProjection) {
    (
        Transform::from_xyz(center.x, center.y, 0.0),
        OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: if overview { 260.0 } else { 32.0 },
            },
            ..OrthographicProjection::default_2d()
        },
    )
}
fn inside_depth(depth: f32, projection: &OrthographicProjection) -> bool {
    depth >= projection.near && depth <= projection.far
}
#[derive(SystemParam)]
struct Scene<'w, 's> {
    cameras: Query<
        'w,
        's,
        (
            &'static Camera,
            &'static GlobalTransform,
            &'static Projection,
        ),
        With<MainCamera>,
    >,
    world_sprites: Query<
        'w,
        's,
        (
            &'static Name,
            &'static Sprite,
            &'static GlobalTransform,
            &'static InheritedVisibility,
        ),
        With<World2dStatic>,
    >,
    structures: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkStructureId,
            &'static NetworkMapStructure,
            &'static StructureKind,
            &'static Team,
            &'static CombatStats,
            &'static NetworkStructureProtected,
        ),
    >,
    structure_sprites: Query<'w, 's, (Entity, &'static MapStructure2dVisual)>,
    structure_visuals: Query<'w, 's, (Entity, &'static VerdantStructureVisual)>,
    props: Query<'w, 's, (Entity, &'static MapPropInstance, &'static GlobalTransform)>,
    children: Query<'w, 's, &'static Children>,
    drawables: Query<
        'w,
        's,
        (
            &'static GlobalTransform,
            &'static InheritedVisibility,
            Option<&'static Mesh3d>,
            Option<&'static Sprite>,
        ),
    >,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    nodes: Query<
        'w,
        's,
        (
            &'static Name,
            &'static ComputedNode,
            &'static InheritedVisibility,
        ),
    >,
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
}
fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}
fn geometry(scene: &Scene, meshes: &Assets<Mesh>, root: Entity) -> serde_json::Value {
    let mut signatures = Vec::new();
    let mut vertices = 0_usize;
    let mut indices = 0_usize;
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    for entity in std::iter::once(root).chain(scene.children.iter_descendants(root)) {
        let Ok((transform, visible, mesh, _)) = scene.drawables.get(entity) else {
            continue;
        };
        if !visible.get() {
            continue;
        }
        let Some(mesh) = mesh.and_then(|mesh| meshes.get(&mesh.0)) else {
            continue;
        };
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        if positions.is_empty() {
            continue;
        }
        let mut signature = 0xcbf29ce484222325;
        for position in positions {
            for value in position {
                hash_bytes(&mut signature, &value.to_bits().to_le_bytes());
            }
            let world = transform.transform_point(Vec3::from_array(*position));
            minimum = minimum.min(world);
            maximum = maximum.max(world);
        }
        let count = mesh.indices().map_or(0, |values| values.len());
        if let Some(values) = mesh.indices() {
            for value in values.iter() {
                hash_bytes(&mut signature, &(value as u64).to_le_bytes());
            }
        }
        vertices += positions.len();
        indices += count;
        signatures.push(format!("{signature:016x}:{}:{count}", positions.len()));
    }
    signatures.sort();
    serde_json::json!({"mesh_geometry_signatures":signatures,"mesh_count":signatures.len(),"vertices":vertices,"indices":indices,
        "world_min":if minimum.is_finite(){Some(minimum.to_array())}else{None},"world_max":if maximum.is_finite(){Some(maximum.to_array())}else{None}})
}
fn static_prop_inventory(scene: &Scene, meshes: &Assets<Mesh>) -> (BTreeMap<String, usize>, usize) {
    let mut counts = BTreeMap::new();
    let mut ready = 0;
    for (entity, prop, _) in &scene.props {
        let Some((source, _)) = prop.key.split_once(':') else {
            continue;
        };
        if !matches!(source, "environment.glb" | "foliage.glb") {
            continue;
        }
        *counts
            .entry(format!("{source}:{}", prop.archetype))
            .or_default() += 1;
        let drawable = std::iter::once(entity).chain(scene.children.iter_descendants(entity)).any(|child| {
            let Ok((_, visible, mesh, _)) = scene.drawables.get(child) else { return false };
            visible.get() && mesh.and_then(|mesh| meshes.get(&mesh.0)).is_some_and(|mesh| {
                matches!(mesh.attribute(Mesh::ATTRIBUTE_POSITION), Some(VertexAttributeValues::Float32x3(positions)) if !positions.is_empty())
            })
        });
        if prop.geometry_ready && drawable {
            ready += 1;
        }
    }
    (counts, ready)
}

fn world_2d_readback(
    scene: &Scene,
    images: &Assets<Image>,
    atlases: &Assets<TextureAtlasLayout>,
) -> serde_json::Value {
    let Ok((camera, camera_transform, Projection::Orthographic(projection))) =
        scene.cameras.single()
    else {
        return serde_json::Value::Null;
    };
    let Some(viewport) = camera.logical_viewport_size() else {
        return serde_json::Value::Null;
    };
    let mut tiles = 0;
    let mut loaded_tiles = 0;
    let mut in_depth = 0;
    let mut on_screen = 0;
    let mut ground_min = f32::INFINITY;
    let mut ground_max = f32::NEG_INFINITY;
    for (name, sprite, transform, visible) in &scene.world_sprites {
        if !name.as_str().starts_with("World2dTile-") {
            continue;
        }
        tiles += 1;
        let position = transform.translation();
        ground_min = ground_min.min(position.z);
        ground_max = ground_max.max(position.z);
        let loaded = visible.get()
            && sprite.color.alpha() > 0.0
            && sprite
                .custom_size
                .is_some_and(|size| size.is_finite() && size.min_element() > 0.0)
            && images
                .get(&sprite.image)
                .is_some_and(|image| image.width() > 0 && image.height() > 0)
            && sprite.texture_atlas.as_ref().is_some_and(|atlas| {
                atlases
                    .get(&atlas.layout)
                    .is_some_and(|layout| atlas.index < layout.textures.len())
            });
        if !loaded {
            continue;
        }
        loaded_tiles += 1;
        let depth = -camera_transform
            .affine()
            .inverse()
            .transform_point3(position)
            .z;
        if inside_depth(depth, projection) {
            in_depth += 1;
            if camera
                .world_to_viewport(camera_transform, position)
                .is_ok_and(|point| point.cmpge(Vec2::ZERO).all() && point.cmple(viewport).all())
            {
                on_screen += 1;
            }
        }
    }
    serde_json::json!({"tile_count":tiles,"loaded_atlas_tiles":loaded_tiles,"tiles_inside_depth":in_depth,"ground_tiles_on_screen":on_screen,
        "static_sprites":scene.world_sprites.iter().count(),"camera_position":camera_transform.translation().to_array(),"camera_near":projection.near,"camera_far":projection.far,
        "ground_z":if ground_min.is_finite(){Some([ground_min,ground_max])}else{None}})
}
fn sprites(scene: &Scene, root: Entity) -> usize {
    std::iter::once(root)
        .chain(scene.children.iter_descendants(root))
        .filter(|entity| {
            scene
                .drawables
                .get(*entity)
                .is_ok_and(|(_, visible, _, sprite)| {
                    visible.get() && sprite.is_some_and(|sprite| sprite.color.alpha() > 0.0)
                })
        })
        .count()
}
#[allow(clippy::too_many_arguments)]
fn observe(
    mut commands: Commands,
    mut qa: ResMut<MapQa>,
    scene: Scene,
    snapshot: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mobile: Res<MobileControls>,
    mode: Res<PlayerVisualMode>,
    assets: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    meshes: Res<Assets<Mesh>>,
    cache: Res<MapVisualCache>,
    images: Res<Assets<Image>>,
    atlases: Res<Assets<TextureAtlasLayout>>,
    mut registry: ResMut<MapVisualRegistry>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage == usize::MAX {
        return;
    }
    if qa.started.elapsed() > qa.timeout {
        fail(&mut qa, &mut exit, "bounded map scenario timed out");
        return;
    }
    if !session.join_confirmed() || !matches!(snapshot.state, GameState::Running) || help.0 {
        return;
    }
    let is_3d = *mode == PlayerVisualMode::Models3d;
    let files: &[&str] = if is_3d { &FILES_3D } else { &FILES_2D };
    if qa.stage == files.len() {
        if qa.readbacks.len() != files.len()
            || files
                .iter()
                .any(|f| !qa.directory.join(f).metadata().is_ok_and(|m| m.len() > 32))
        {
            return;
        }
        let summary = serde_json::json!({"scenario":"map","version":env!("CARGO_PKG_VERSION"),"pass":true,
            "geometry_id":snapshot.geometry_id,"map_profile":snapshot.map_profile,"visual_mode":format!("{:?}",*mode),"pixels":qa.pixels.to_array(),
            "scripted_camera":true,"scripted_cosmetic_registry":is_3d,"synthetic_structures":false,"physical_device_verified":false,"manual_interaction_verified":false,
            "geometry_fingerprint":"FNV-1a over actual mesh position/index buffers; not a cryptographic asset hash",
            "two_d_contract":"configured authoritative structure sprites; 3D prop model swaps are not applied",
            "captures":qa.frames,"operations":qa.operations,"map_visual_cache_counts":cache.counts()});
        if std::fs::write(
            qa.directory.join("qa-summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .is_err()
        {
            fail(&mut qa, &mut exit, "cannot write summary");
            return;
        }
        qa.stage = usize::MAX;
        if let Ok(window) = scene.windows.single() {
            commands.entity(window).despawn();
        }
        exit.write(AppExit::Success);
        return;
    }
    if qa.stage > 0 && !qa.readbacks.contains(&(qa.stage - 1)) {
        return;
    }
    if registry.revision() == 0 || scene.structures.iter().count() != qa.expected_structures {
        return;
    }
    let world_2d = if is_3d {
        serde_json::Value::Null
    } else {
        world_2d_readback(&scene, &images, &atlases)
    };
    if !is_3d {
        qa.readiness = world_2d.clone();
        let expected = (WORLD_TILE_COLUMNS * WORLD_TILE_ROWS) as u64;
        if ["tile_count", "loaded_atlas_tiles", "tiles_inside_depth"]
            .iter()
            .any(|key| world_2d[*key].as_u64() != Some(expected))
            || world_2d["ground_tiles_on_screen"].as_u64().unwrap_or(0) == 0
        {
            qa.settled = 0;
            return;
        }
    }
    let (static_counts, ready_static_props) = if is_3d {
        static_prop_inventory(&scene, &meshes)
    } else {
        (BTreeMap::new(), 0)
    };
    let expected_static: BTreeMap<String, usize> = STATIC_PROPS
        .iter()
        .map(|(key, count)| (key.to_string(), *count))
        .collect();
    qa.readiness = serde_json::json!({"static_props_by_archetype":static_counts,"ready_static_props":ready_static_props});
    if is_3d && (static_counts != expected_static || ready_static_props != 942) {
        qa.settled = 0;
        return;
    }
    let scenes_ready = scene.scenes.iter().all(|(root, instance)| {
        matches!(
            assets.get_recursive_dependency_load_state(root.0.id()),
            Some(RecursiveDependencyLoadState::Loaded)
        ) && instance.is_some_and(|instance| spawner.instance_is_ready(**instance))
    });
    if !scenes_ready {
        qa.settled = 0;
        return;
    }
    if qa.selected_key.is_none() && is_3d {
        let requested = std::env::var("OMOBA_MAP_QA_PROP_KEY").ok();
        let selected = scene
            .props
            .iter()
            .filter(|(_, prop, _)| {
                !prop.solid
                    && prop.archetype == "lantern"
                    && requested.as_ref().is_none_or(|key| *key == prop.key)
            })
            .min_by(|(_, left, a), (_, right, b)| {
                a.translation()
                    .length_squared()
                    .total_cmp(&b.translation().length_squared())
                    .then(left.key.cmp(&right.key))
            });
        let Some((_, prop, transform)) = selected else {
            return;
        };
        qa.selected_key = Some(prop.key.clone());
        qa.focus = transform.translation();
    } else if !is_3d && qa.stage == 0 {
        if let Some((_, transform, _, _, _, _, _, _)) = scene
            .structures
            .iter()
            .filter(|(_, _, _, _, kind, _, _, _)| **kind == StructureKind::Tower)
            .min_by(|(_, a, _, _, _, _, _, _), (_, b, _, _, _, _, _, _)| {
                a.translation
                    .length_squared()
                    .total_cmp(&b.translation.length_squared())
            })
        {
            qa.focus = transform.translation;
        }
    }
    if is_3d && qa.stage > 0 && qa.configured != Some(qa.stage) {
        let key = qa.selected_key.as_ref().unwrap();
        let model = MODELS[(qa.stage - 1) % 2];
        // Validated production registry; only this cosmetic instance changes.
        let config = serde_json::json!({"schema_version":1,"instances":{key:{"model":{"path":model,"scene":0}}}});
        if let Err(error) = registry.replace_json(&config.to_string()) {
            fail(
                &mut qa,
                &mut exit,
                &format!("cosmetic registry rejected QA request: {error}"),
            );
            return;
        }
        let operation = serde_json::json!({"stage":qa.stage,"key":key,"model":model,"registry_revision":registry.revision()});
        qa.operations.push(operation);
        qa.configured = Some(qa.stage);
        qa.settled = 0;
        return;
    }
    let selected = qa
        .selected_key
        .as_ref()
        .and_then(|key| scene.props.iter().find(|(_, prop, _)| &prop.key == key));
    let mut prop_record = serde_json::Value::Null;
    if let Some((entity, prop, _)) = selected {
        let measured = geometry(&scene, &meshes, entity);
        if !prop.geometry_ready || measured["vertices"].as_u64().unwrap_or(0) == 0 {
            qa.settled = 0;
            return;
        }
        let expected = format!("{}#Scene0", MODELS[qa.stage.saturating_sub(1) % 2]);
        if qa.stage > 0 && prop.active_model.as_deref() != Some(expected.as_str()) {
            qa.settled = 0;
            return;
        }
        prop_record = serde_json::json!({"entity":format!("{entity:?}"),"key":prop.key,"archetype":prop.archetype,"role":prop.role,"solid":prop.solid,
            "desired_model":prop.desired_model,"active_model":prop.active_model,"geometry_ready":prop.geometry_ready,"geometry":measured});
    }
    qa.settled += 1;
    if qa.settled < 15 {
        return;
    }
    let structures:Vec<_>=scene.structures.iter().map(|(owner,t,id,map,kind,team,stats,protected)|{
        let children:Vec<_>=scene.structure_visuals.iter().filter(|(_,visual)|visual.owner==owner).map(|(entity,_)|geometry(&scene,&meshes,entity)).collect();
        let sprites: Vec<_> = scene.structure_sprites.iter().filter(|(_, visual)|visual.owner == owner).map(|(entity,visual)|serde_json::json!({"entity":format!("{entity:?}"),"owner":format!("{owner:?}"),"sprite_key":visual.sprite_key,"profile_key":visual.profile_key,"visible_drawables":sprites(&scene,entity)})).collect();
        serde_json::json!({"id":id.0,"lane":map.lane,"tier":map.tier,"sprite_geometry":sprites,"map_key":map.key,"visual_profile":map.visual_profile,"kind":kind,"team":team,"position":t.translation.to_array(),"hp":stats.hp,"max_hp":stats.max_hp,"protected":protected.0,"model_geometry":children})
    }).collect();
    if is_3d
        && structures.iter().any(|s| {
            s["model_geometry"].as_array().is_none_or(|children| {
                children.len() != 1 || children[0]["vertices"].as_u64().unwrap_or(0) == 0
            })
        })
    {
        qa.settled = 0;
        return;
    }
    if !is_3d
        && structures.iter().any(|s| {
            s["sprite_geometry"].as_array().is_none_or(|children| {
                children.len() != 1
                    || children[0]["visible_drawables"].as_u64().unwrap_or(0) == 0
                    || children[0]["profile_key"] != s["visual_profile"]
            })
        })
    {
        qa.settled = 0;
        return;
    }
    let nodes:Vec<_>=scene.nodes.iter().filter(|(name,_,_)|matches!(name.as_str(),"MobileJoystick"|"MobileAttack"|"MobileAbility-0"|"MobileAbility-1"|"MobileAbility-2"|"MobileAbility-3"))
        .map(|(name,node,visible)|serde_json::json!({"name":name.as_str(),"size":node.size().to_array(),"visible":visible.get()})).collect();
    let frame = serde_json::json!({"stage":qa.stage,"file":files[qa.stage],"pixels":qa.pixels.to_array(),"mobile_controls":mobile.enabled,"visual_mode":format!("{:?}",*mode),
        "snapshot_tick":snapshot.meta.snapshot_tick,"server_epoch":snapshot.meta.server_epoch,"match_id":snapshot.meta.match_id,"geometry_id":snapshot.geometry_id,"map_profile":snapshot.map_profile,
        "structures":structures,"selected_prop":prop_record,"world_2d":world_2d,"static_props_by_archetype":static_counts,"ready_static_props":ready_static_props,"prop_instances":scene.props.iter().count(),"scene_roots":scene.scenes.iter().count(),"mesh_assets":meshes.len(),"map_visual_cache_counts":cache.counts(),"nodes":nodes});
    let index = qa.stage;
    qa.frames.push(frame);
    commands
        .spawn((Screenshot::primary_window(), Shot(index)))
        .observe(save_to_disk(qa.directory.join(files[index])))
        .observe(readback);
    qa.stage += 1;
    qa.settled = 0;
}
#[derive(Component)]
struct Shot(usize);
fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<MapQa>) {
    if let Ok(shot) = shots.get(captured.entity)
        && captured.image.width() == qa.pixels.x
        && captured.image.height() == qa.pixels.y
    {
        qa.readbacks.insert(shot.0);
    }
}
fn fail(qa: &mut MapQa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    let _=std::fs::write(qa.directory.join("qa-failure.json"),serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"reason":reason,"readiness":qa.readiness,"captures":qa.frames,"operations":qa.operations})).unwrap());
    error!("MAP_QA failed stage={}: {reason}", qa.stage);
    qa.stage = usize::MAX;
    exit.write(AppExit::error());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qa_two_d_camera_includes_ground_and_foreground_bands() {
        use crate::world2d::layer;
        for overview in [false, true] {
            let (transform, projection) = camera_2d(Vec2::new(-22.0, -22.0), overview);
            assert_eq!(transform.translation.z, 0.0);
            for z in [
                layer::GROUND,
                layer::WATER,
                layer::PATH,
                layer::LOW_PROP,
                layer::ACTOR,
                layer::OVERHEAD,
            ] {
                assert!(
                    inside_depth(transform.translation.z - z, &projection),
                    "clipped render band {z}"
                );
            }
            assert!(
                !inside_depth(999.0 - layer::GROUND, &projection),
                "regression fixture must detect the old clipped ground"
            );
        }
    }
}
