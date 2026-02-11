use bevy::prelude::*;
use bevy::scene::SceneRoot;
use std::f32::consts::PI;

use crate::camera::{CameraState, MainCamera, locked_camera_offset};
use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::player::{PLAYER_SIZE, Player, PlayerBody, VerticalVelocity};
use crate::team::{Team, TeamSelection};

const USE_CUSTOM_MODEL: bool = true;

#[derive(Resource, Clone)]
pub struct PlayerAssets {
    pub scene: Option<Handle<Scene>>,
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

pub struct SetupPlugin;

impl Plugin for SetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_scene)
            .add_systems(Update, spawn_local_player_on_team);
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
    let scene_handle = if USE_CUSTOM_MODEL {
        load_scene_from_ipfs(
            "https://ipfs.io/ipfs/QmWMYVUF2pa4GkoMgquyY8nmYjQJDP9yxnSBvjVqH7EJQr",
            &asset_server,
        )
    } else {
        None
    };

    commands.insert_resource(PlayerAssets {
        scene: scene_handle.clone(),
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
