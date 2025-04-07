use bevy::prelude::*;
use bevy::scene::SceneRoot;
use std::f32::consts::PI;

use crate::camera::{CAMERA_DISTANCE, CAMERA_HEIGHT, CameraState, MainCamera};
use crate::player::{PLAYER_SIZE, Player};

const USE_CUSTOM_MODEL: bool = false;

pub struct SetupPlugin;

impl Plugin for SetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_scene);
    }
}

pub fn load_scene_from_ipfs(url: &str, asset_server: &AssetServer) -> Handle<Scene> {
    use reqwest::blocking as req_blocking;
    use std::fs;

    let last_segment = url.split('/').last().unwrap_or("downloaded_scene.glb");

    let filename = if last_segment.ends_with(".glb") {
        last_segment.to_string()
    } else {
        format!("{last_segment}.glb")
    };

    let assets_dir = std::env::current_dir()
        .expect("Could not get current directory")
        .join("assets")
        .join("downloaded");
    fs::create_dir_all(&assets_dir).expect("Failed to create ./assets/downloaded folder");

    let final_path = assets_dir.join(&filename);

    let response = req_blocking::get(url)
        .unwrap_or_else(|e| panic!("Failed to download {url}: {e}"));
    let bytes = response
        .bytes()
        .unwrap_or_else(|e| panic!("Failed to read bytes from {url}: {e}"));

    fs::write(&final_path, &bytes)
        .unwrap_or_else(|e| panic!("Failed to write asset file {:?}: {e}", final_path));

    let relative_path = format!("downloaded/{}#Scene0", filename);
    let scene_handle: Handle<Scene> = asset_server.load(&relative_path);

    scene_handle
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cam_state: ResMut<CameraState>,
    asset_server: Res<AssetServer>,
) {
    let ground_mesh_handle: Handle<Mesh> =
        meshes.add(Mesh::from(Plane3d::default().mesh().size(50.0, 50.0)));
    let ground_material_handle: Handle<StandardMaterial> =
        materials.add(StandardMaterial::from(Color::srgb(0.3, 0.5, 0.3)));

    if USE_CUSTOM_MODEL {
        let glb_scene = load_scene_from_ipfs(
            "https://ipfs.io/ipfs/QmWMYVUF2pa4GkoMgquyY8nmYjQJDP9yxnSBvjVqH7EJQr",
            &asset_server,
        );

        commands.spawn((
            SceneRoot(glb_scene),
            Transform {
                translation: Vec3::new(0.0, PLAYER_SIZE / 2.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::splat(1.0),
            },
            GlobalTransform::default(),
            Visibility::default(),
            Player,
            Name::new("Player"),
        ));
    } else {
        let player_mesh_handle: Handle<Mesh> = meshes.add(Mesh::from(Cuboid::new(
            PLAYER_SIZE,
            PLAYER_SIZE,
            PLAYER_SIZE,
        )));
        let player_material_handle: Handle<StandardMaterial> =
            materials.add(StandardMaterial::from(Color::srgb(0.8, 0.7, 0.6)));

        let player_transform = Transform::from_xyz(0.0, PLAYER_SIZE / 2.0, 0.0);
        commands.spawn((
            Mesh3d(player_mesh_handle),
            MeshMaterial3d(player_material_handle),
            player_transform,
            Player,
            Name::new("Player"),
        ));
    }

    commands.spawn((
        Mesh3d(ground_mesh_handle),
        MeshMaterial3d(ground_material_handle),
        Transform::from_xyz(0.0, 0.5, 0.0),
        Name::new("Ground"),
    ));

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

    let initial_cam_pos = Vec3::new(0.0, CAMERA_HEIGHT, CAMERA_DISTANCE);
    let initial_cam_transform =
        Transform::from_translation(initial_cam_pos).looking_at(Vec3::ZERO, Vec3::Y);

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
