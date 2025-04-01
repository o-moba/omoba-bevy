use bevy::prelude::*;
use std::f32::consts::PI;

use crate::camera::{CAMERA_DISTANCE, CAMERA_HEIGHT, CameraState, MainCamera};
use crate::player::{PLAYER_SIZE, Player};

pub struct SetupPlugin;

impl Plugin for SetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_scene);
    }
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cam_state: ResMut<CameraState>,
) {
    let ground_mesh_handle: Handle<Mesh> =
        meshes.add(Mesh::from(Plane3d::default().mesh().size(50.0, 50.0)));
    let ground_material_handle: Handle<StandardMaterial> =
        materials.add(StandardMaterial::from(Color::srgb(0.3, 0.5, 0.3)));

    let player_mesh_handle: Handle<Mesh> = meshes.add(Mesh::from(Cuboid::new(
        PLAYER_SIZE,
        PLAYER_SIZE,
        PLAYER_SIZE,
    )));
    let player_material_handle: Handle<StandardMaterial> =
        materials.add(StandardMaterial::from(Color::srgb(0.8, 0.7, 0.6)));

    commands.spawn((
        Mesh3d(ground_mesh_handle),
        MeshMaterial3d(ground_material_handle),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Name::new("Ground"),
    ));

    let player_transform = Transform::from_xyz(0.0, PLAYER_SIZE / 2.0, 0.0);
    commands.spawn((
        Mesh3d(player_mesh_handle),
        MeshMaterial3d(player_material_handle),
        player_transform,
        Player,
        Name::new("Player"),
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
