use bevy::prelude::*;
use bevy::camera::primitives::Aabb;
use bevy::scene::SceneRoot;
use bevy::gltf::Gltf;
use std::f32::consts::PI;

use crate::camera::{CameraState, MainCamera, locked_camera_offset};
use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::player::{PLAYER_SIZE, Player, PlayerBody, VerticalVelocity};
use crate::team::{CharacterChoice, Team, TeamSelection};

const USE_CUSTOM_MODEL: bool = true;
pub const DEFAULT_MODEL_TARGET_HEIGHT: f32 = 0.26;
pub const MIN_MODEL_TARGET_HEIGHT: f32 = 0.08;
pub const MAX_MODEL_TARGET_HEIGHT: f32 = 1.2;
const NORMALIZATION_MIN_HEIGHT: f32 = 0.001;

#[derive(Resource, Clone)]
pub struct PlayerAssets {
    pub scene: Option<Handle<Scene>>,
    pub gltf: Option<Handle<Gltf>>,
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

#[derive(Resource, Clone, Default)]
struct PlayerModelCatalog {
    ipfs_scene: Option<Handle<Scene>>,
    toka_scene: Option<Handle<Scene>>,
    toka_gltf: Option<Handle<Gltf>>,
    wang_scene: Option<Handle<Scene>>,
    wang_gltf: Option<Handle<Gltf>>,
}

pub struct SetupPlugin;

impl Plugin for SetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_scene)
            .init_resource::<ModelScaleSettings>()
            .add_systems(Update, sync_selected_player_assets)
            .add_systems(Update, normalize_model_scale_system)
            .add_systems(Update, spawn_local_player_on_team);
    }
}

#[derive(Component)]
pub struct NormalizeModelScale {
    base_scale: Vec3,
    last_applied_target_height: Option<f32>,
}

impl NormalizeModelScale {
    pub fn for_player_model() -> Self {
        Self {
            base_scale: Vec3::ONE,
            last_applied_target_height: None,
        }
    }
}

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

pub fn load_scene_from_ipfs(url: &str, asset_server: &AssetServer) -> Option<Handle<Scene>> {
    use reqwest::blocking as req_blocking;
    use std::fs;
    use std::path::PathBuf;

    let last_segment = url.split('/').last().unwrap_or("downloaded_scene.glb");

    let filename = if last_segment.ends_with(".glb") {
        last_segment.to_string()
    } else {
        format!("{last_segment}.glb")
    };

    let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("downloaded");
    if let Err(error) = fs::create_dir_all(&assets_dir) {
        warn!("Failed to create ./assets/downloaded folder: {error}");
        return None;
    }

    let final_path = assets_dir.join(&filename);

    let response = match req_blocking::get(url) {
        Ok(response) => response,
        Err(error) => {
            warn!("Failed to download {url}: {error}");
            return None;
        }
    };
    let bytes = response
        .bytes()
        .map_err(|error| warn!("Failed to read bytes from {url}: {error}"))
        .ok()?;
    if !is_valid_glb_bytes(&bytes) {
        warn!("Downloaded asset from {url} is not a valid glb.");
        return None;
    }

    if let Err(error) = fs::write(&final_path, &bytes) {
        warn!("Failed to write asset file {:?}: {error}", final_path);
        return None;
    }

    let relative_path = format!("downloaded/{}#Scene0", filename);
    let scene_handle: Handle<Scene> = asset_server.load(&relative_path);
    Some(scene_handle)
}

fn is_valid_glb_bytes(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[0..4] != b"glTF" {
        return false;
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let length = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    version == 2 && length <= bytes.len()
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cam_state: ResMut<CameraState>,
    asset_server: Res<AssetServer>,
) {
    let player_mesh_handle: Handle<Mesh> = meshes.add(Mesh::from(Cuboid::new(
        PLAYER_SIZE,
        PLAYER_SIZE,
        PLAYER_SIZE,
    )));
    let player_material_handle: Handle<StandardMaterial> =
        materials.add(StandardMaterial::from(Color::srgb(0.8, 0.7, 0.6)));
    let mut catalog = PlayerModelCatalog::default();
    if USE_CUSTOM_MODEL {
        let toka_model_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("downloaded")
            .join("toka.glb");
        if toka_model_path.exists() {
            catalog.toka_scene = Some(asset_server.load("downloaded/toka.glb#Scene0"));
            catalog.toka_gltf = Some(asset_server.load("downloaded/toka.glb"));
        }

        let wang_model_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("downloaded")
            .join("wang.glb");
        if wang_model_path.exists() {
            catalog.wang_scene = Some(asset_server.load("downloaded/wang.glb#Scene0"));
            catalog.wang_gltf = Some(asset_server.load("downloaded/wang.glb"));
        }

        catalog.ipfs_scene = load_scene_from_ipfs(
            "https://ipfs.io/ipfs/QmWMYVUF2pa4GkoMgquyY8nmYjQJDP9yxnSBvjVqH7EJQr",
            &asset_server,
        );
    }

    commands.insert_resource(catalog.clone());
    commands.insert_resource(PlayerAssets {
        scene: catalog.ipfs_scene,
        gltf: None,
        mesh: player_mesh_handle.clone(),
        material: player_material_handle.clone(),
    });

    let light_transform =
        Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, PI / 4.0, -PI / 4.0));
    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            ..default()
        },
        light_transform,
        Name::new("Light"),
    ));

    let map_center = Vec3::new(0.0, PLAYER_SIZE * 0.5, 0.0);
    let zoom = cam_state.zoom;
    let initial_cam_pos = map_center + locked_camera_offset(zoom);
    let initial_cam_transform =
        Transform::from_translation(initial_cam_pos).looking_at(map_center, Vec3::Y);

    let (_yaw, pitch, _roll) = initial_cam_transform.rotation.to_euler(EulerRot::YXZ);
    cam_state.pitch = pitch;
    cam_state.yaw = _yaw;

    commands.spawn((
        Camera3d::default(),
        initial_cam_transform,
        MainCamera,
        Name::new("Camera"),
    ));
}

fn sync_selected_player_assets(
    team_selection: Res<TeamSelection>,
    catalog: Res<PlayerModelCatalog>,
    mut player_assets: ResMut<PlayerAssets>,
) {
    let (scene, gltf, label) = match team_selection.character {
        CharacterChoice::Ipfs => (catalog.ipfs_scene.clone(), None, "IPFS"),
        CharacterChoice::Toka => (
            catalog.toka_scene.clone(),
            catalog.toka_gltf.clone(),
            "downloaded/toka.glb",
        ),
        CharacterChoice::Wang => (
            catalog.wang_scene.clone(),
            catalog.wang_gltf.clone(),
            "downloaded/wang.glb",
        ),
        CharacterChoice::Cube => (None, None, "Cube"),
    };

    let changed = player_assets.scene != scene || player_assets.gltf != gltf;
    if changed {
        player_assets.scene = scene;
        player_assets.gltf = gltf;
        info!("Selected player model: {label}");
    }
}

fn spawn_local_player_on_team(
    mut commands: Commands,
    team_selection: Res<TeamSelection>,
    player_assets: Res<PlayerAssets>,
    map_layout: Res<MapLayout>,
    existing_players: Query<Entity, With<Player>>,
    mut cam_state: ResMut<CameraState>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
) {
    if team_selection.team.is_none() {
        return;
    }
    if existing_players.iter().next().is_some() {
        return;
    }
    let team = team_selection.team.unwrap();
    let spawn = map_layout.team_spawn(team);

    spawn_player_entity(&mut commands, &player_assets, spawn, team);
    if let Ok(mut camera_transform) = camera_query.single_mut() {
        cam_state.locked = true;
        let zoom = cam_state.zoom;
        camera_transform.translation = spawn + locked_camera_offset(zoom);
        let look_target = Vec3::new(spawn.x, PLAYER_SIZE * 0.5, spawn.z);
        *camera_transform = camera_transform.looking_at(look_target, Vec3::Y);
    }
}

fn spawn_player_entity(commands: &mut Commands, assets: &PlayerAssets, spawn: Vec3, team: Team) {
    if let Some(glb_scene) = assets.scene.clone() {
        commands.spawn((
            SceneRoot(glb_scene),
            Transform {
                translation: spawn,
                rotation: Quat::IDENTITY,
                scale: Vec3::splat(1.0),
            },
            GlobalTransform::default(),
            Visibility::default(),
            Player,
            PlayerBody,
            CombatStats::default(),
            VerticalVelocity::default(),
            team,
            NormalizeModelScale::for_player_model(),
            Name::new(format!("Player-{}", team.as_str())),
        ));
    } else {
        let player_transform = Transform::from_translation(spawn);
        commands.spawn((
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.material.clone()),
            player_transform,
            Player,
            PlayerBody,
            CombatStats::default(),
            VerticalVelocity::default(),
            team,
            Name::new(format!("Player-{}", team.as_str())),
        ));
    }
}

fn normalize_model_scale_system(
    settings: Res<ModelScaleSettings>,
    mut roots: Query<(Entity, &mut Transform, &mut NormalizeModelScale)>,
    children_query: Query<&Children>,
    aabb_query: Query<&Aabb>,
    globals_query: Query<&GlobalTransform>,
) {
    for (entity, mut transform, mut normalization) in &mut roots {
        let target_height = settings
            .target_height
            .clamp(MIN_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT);
        if normalization
            .last_applied_target_height
            .is_some_and(|applied| (applied - target_height).abs() < f32::EPSILON)
        {
            continue;
        }

        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut has_bounds = false;

        for descendant in children_query.iter_descendants(entity) {
            let (Ok(aabb), Ok(global)) = (aabb_query.get(descendant), globals_query.get(descendant))
            else {
                continue;
            };
            let center: Vec3 = aabb.center.into();
            let half: Vec3 = aabb.half_extents.into();
            for sx in [-1.0_f32, 1.0] {
                for sy in [-1.0_f32, 1.0] {
                    for sz in [-1.0_f32, 1.0] {
                        let local_corner = center + Vec3::new(half.x * sx, half.y * sy, half.z * sz);
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

        let scale_factor = target_height / current_height;
        transform.scale = normalization.base_scale * Vec3::splat(scale_factor);
        normalization.last_applied_target_height = Some(target_height);
    }
}
