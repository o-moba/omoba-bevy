use bevy::camera::primitives::Aabb;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use ekza_stellar_sdk::bevy::{EkzaModelCatalog, load_builtin_model_catalog};

use crate::camera::{CameraState, MainCamera, locked_camera_offset};
use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::net::NetworkCharacterChoice;
use crate::player::{PLAYER_SIZE, Player, PlayerBody, VerticalVelocity};
use crate::team::{CharacterChoice, Team, TeamSelection};

const USE_CUSTOM_MODEL: bool = true;
pub const DEFAULT_MODEL_TARGET_HEIGHT: f32 = 0.26;
pub const MIN_MODEL_TARGET_HEIGHT: f32 = 0.08;
pub const MAX_MODEL_TARGET_HEIGHT: f32 = 1.2;
const NORMALIZATION_MIN_HEIGHT: f32 = 0.001;
pub const DEFAULT_LIGHT_ILLUMINANCE: f32 = 25_000.0;
pub const MIN_LIGHT_ILLUMINANCE: f32 = 4_000.0;
pub const MAX_LIGHT_ILLUMINANCE: f32 = 120_000.0;
pub const DEFAULT_AMBIENT_BRIGHTNESS: f32 = 300.0;
pub const MIN_AMBIENT_BRIGHTNESS: f32 = 0.0;
pub const MAX_AMBIENT_BRIGHTNESS: f32 = 3_500.0;
pub const DEFAULT_LIGHT_PITCH_DEG: f32 = 45.0;
pub const MIN_LIGHT_PITCH_DEG: f32 = 10.0;
pub const MAX_LIGHT_PITCH_DEG: f32 = 85.0;
pub const DEFAULT_LIGHT_YAW_DEG: f32 = -45.0;
pub const MIN_LIGHT_YAW_DEG: f32 = -180.0;
pub const MAX_LIGHT_YAW_DEG: f32 = 180.0;

#[derive(Resource, Clone)]
pub struct PlayerAssets {
    pub scene: Option<Handle<Scene>>,
    pub gltf: Option<Handle<Gltf>>,
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

pub type PlayerModelCatalog = EkzaModelCatalog;

pub fn model_assets_for_choice(
    catalog: &PlayerModelCatalog,
    choice: CharacterChoice,
) -> (Option<Handle<Scene>>, Option<Handle<Gltf>>) {
    catalog.handles_for(choice)
}

pub struct SetupPlugin;

impl Plugin for SetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            setup_scene.after(crate::persistence::load_persistent_client_settings),
        )
        .init_resource::<ModelScaleSettings>()
        .init_resource::<LightingSettings>()
        .add_systems(Update, sync_selected_player_assets)
        .add_systems(Update, normalize_model_scale_system)
        .add_systems(Update, apply_lighting_settings_system)
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

#[derive(Resource, Clone, Copy)]
pub struct LightingSettings {
    pub illuminance: f32,
    pub ambient_brightness: f32,
    pub light_pitch_deg: f32,
    pub light_yaw_deg: f32,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            illuminance: DEFAULT_LIGHT_ILLUMINANCE,
            ambient_brightness: DEFAULT_AMBIENT_BRIGHTNESS,
            light_pitch_deg: DEFAULT_LIGHT_PITCH_DEG,
            light_yaw_deg: DEFAULT_LIGHT_YAW_DEG,
        }
    }
}

#[derive(Component)]
struct SceneDirectionalLight;

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cam_state: ResMut<CameraState>,
    asset_server: Res<AssetServer>,
    lighting_settings: Res<LightingSettings>,
) {
    let player_mesh_handle: Handle<Mesh> = meshes.add(Mesh::from(Cuboid::new(
        PLAYER_SIZE,
        PLAYER_SIZE,
        PLAYER_SIZE,
    )));
    let player_material_handle: Handle<StandardMaterial> =
        materials.add(StandardMaterial::from(Color::srgb(0.8, 0.7, 0.6)));
    let assets_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    let catalog = if USE_CUSTOM_MODEL {
        load_builtin_model_catalog(&asset_server, &assets_dir)
    } else {
        PlayerModelCatalog::default()
    };
    let (initial_scene, initial_gltf) = model_assets_for_choice(&catalog, CharacterChoice::Ipfs);

    commands.insert_resource(catalog.clone());
    commands.insert_resource(PlayerAssets {
        scene: initial_scene,
        gltf: initial_gltf,
        mesh: player_mesh_handle.clone(),
        material: player_material_handle.clone(),
    });

    let pitch_rad = lighting_settings
        .light_pitch_deg
        .clamp(MIN_LIGHT_PITCH_DEG, MAX_LIGHT_PITCH_DEG)
        .to_radians();
    let yaw_rad = lighting_settings
        .light_yaw_deg
        .clamp(MIN_LIGHT_YAW_DEG, MAX_LIGHT_YAW_DEG)
        .to_radians();
    let light_transform =
        Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, pitch_rad, yaw_rad));
    commands.spawn((
        DirectionalLight {
            illuminance: lighting_settings
                .illuminance
                .clamp(MIN_LIGHT_ILLUMINANCE, MAX_LIGHT_ILLUMINANCE),
            shadows_enabled: true,
            ..default()
        },
        light_transform,
        SceneDirectionalLight,
        Name::new("Light"),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: lighting_settings
            .ambient_brightness
            .clamp(MIN_AMBIENT_BRIGHTNESS, MAX_AMBIENT_BRIGHTNESS),
        ..default()
    });

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

fn apply_lighting_settings_system(
    settings: Res<LightingSettings>,
    mut ambient_light: ResMut<GlobalAmbientLight>,
    mut light_query: Query<(&mut DirectionalLight, &mut Transform), With<SceneDirectionalLight>>,
) {
    if !settings.is_changed() {
        return;
    }

    ambient_light.brightness = settings
        .ambient_brightness
        .clamp(MIN_AMBIENT_BRIGHTNESS, MAX_AMBIENT_BRIGHTNESS);

    if let Ok((mut light, mut transform)) = light_query.single_mut() {
        light.illuminance = settings
            .illuminance
            .clamp(MIN_LIGHT_ILLUMINANCE, MAX_LIGHT_ILLUMINANCE);
        let pitch_rad = settings
            .light_pitch_deg
            .clamp(MIN_LIGHT_PITCH_DEG, MAX_LIGHT_PITCH_DEG)
            .to_radians();
        let yaw_rad = settings
            .light_yaw_deg
            .clamp(MIN_LIGHT_YAW_DEG, MAX_LIGHT_YAW_DEG)
            .to_radians();
        transform.rotation = Quat::from_euler(EulerRot::ZYX, 0.0, pitch_rad, yaw_rad);
    }
}

fn sync_selected_player_assets(
    team_selection: Res<TeamSelection>,
    catalog: Res<PlayerModelCatalog>,
    mut player_assets: ResMut<PlayerAssets>,
) {
    let (scene, gltf) = model_assets_for_choice(&catalog, team_selection.character);
    let label = catalog.label_for(team_selection.character);

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
    let character = team_selection.character;
    let spawn = map_layout.team_spawn(team);

    spawn_player_entity(&mut commands, &player_assets, spawn, team, character);
    if let Ok(mut camera_transform) = camera_query.single_mut() {
        cam_state.locked = true;
        let zoom = cam_state.zoom;
        camera_transform.translation = spawn + locked_camera_offset(zoom);
        let look_target = Vec3::new(spawn.x, PLAYER_SIZE * 0.5, spawn.z);
        *camera_transform = camera_transform.looking_at(look_target, Vec3::Y);
    }
}

fn spawn_player_entity(
    commands: &mut Commands,
    assets: &PlayerAssets,
    spawn: Vec3,
    team: Team,
    character: CharacterChoice,
) {
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
            NetworkCharacterChoice(character),
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
            NetworkCharacterChoice(character),
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

        let scale_factor = target_height / current_height;
        transform.scale = normalization.base_scale * Vec3::splat(scale_factor);
        normalization.last_applied_target_height = Some(target_height);
    }
}
