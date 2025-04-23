use bevy::{input::mouse::MouseMotion, prelude::*, window::PrimaryWindow};
use std::f32::consts::PI;

use crate::player::{PLAYER_SIZE, Player};

pub const CAMERA_DISTANCE: f32 = 15.0;
pub const CAMERA_HEIGHT: f32 = 12.0;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraState>()
            .add_systems(Update, (update_camera, toggle_camera_lock));
    }
}

#[derive(Component)]
pub struct MainCamera;

#[derive(Resource, Default)]
pub struct CameraState {
    pub locked: bool,
    pub pitch: f32,
    pub yaw: f32,
}

fn toggle_camera_lock(
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut cam_state: ResMut<CameraState>,
    window_query: Query<&Window, With<PrimaryWindow>>,
) {
    if window_query.get_single().is_ok() {
        let mut toggled = false;
        if mouse_button_input.just_pressed(MouseButton::Right) {
            toggled = true;
        }
        if keyboard_input.just_pressed(KeyCode::AltLeft)
            || keyboard_input.just_pressed(KeyCode::AltRight)
        {
            toggled = true;
        }

        if toggled {
            cam_state.locked = !cam_state.locked;
            info!("Camera Locked: {}", cam_state.locked);
        }
    } else {
        warn!("No primary window found.");
    }
}

fn update_camera(
    time: Res<Time>,
    mut camera_query: Query<&mut Transform, (With<MainCamera>, Without<Player>)>,
    player_query: Query<&Transform, (With<Player>, Without<MainCamera>)>,
    mut cam_state: ResMut<CameraState>,
    mut mouse_motion_events: EventReader<MouseMotion>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
) {
    let Ok(mut camera_transform) = camera_query.get_single_mut() else {
        return;
    };
    if cam_state.locked {
        if let Ok(player_transform) = player_query.get_single() {
            let target_position =
                player_transform.translation + Vec3::new(0.0, CAMERA_HEIGHT, CAMERA_DISTANCE);
            let lerp_factor = (time.delta_secs() * 2.0).min(1.0);
            camera_transform.translation = camera_transform
                .translation
                .lerp(target_position, lerp_factor);
            let look_target = Vec3::new(
                player_transform.translation.x,
                PLAYER_SIZE / 2.0,
                player_transform.translation.z,
            );
            let look_direction = look_target - camera_transform.translation;
            if look_direction.length_squared() > 0.0001 {
                let target_transform = Transform::default().looking_at(look_direction, Vec3::Y);
                camera_transform.rotation = camera_transform
                    .rotation
                    .slerp(target_transform.rotation, lerp_factor);
            }
            cam_state.yaw = camera_transform.rotation.to_euler(EulerRot::YXZ).0;
            cam_state.pitch = -camera_transform.rotation.to_euler(EulerRot::YXZ).1;
        }
    } else {
        let mut delta_xy = Vec2::ZERO;
        for event in mouse_motion_events.read() {
            delta_xy += event.delta;
        }
        let sensitivity = 0.002;
        cam_state.yaw -= delta_xy.x * sensitivity;
        cam_state.pitch -= delta_xy.y * sensitivity;
        cam_state.pitch = cam_state.pitch.clamp(-PI / 2.0 + 0.01, PI / 2.0 - 0.01);
        camera_transform.rotation = Quat::from_axis_angle(Vec3::Y, cam_state.yaw)
            * Quat::from_axis_angle(Vec3::X, cam_state.pitch);
        let mut move_direction = Vec3::ZERO;
        let camera_forward = *camera_transform.forward();
        let camera_right = *camera_transform.right();
        let camera_up = Vec3::Y;
        if keyboard_input.pressed(KeyCode::KeyW) {
            move_direction += camera_forward;
        }
        if keyboard_input.pressed(KeyCode::KeyS) {
            move_direction -= camera_forward;
        }
        if keyboard_input.pressed(KeyCode::KeyA) {
            move_direction -= camera_right;
        }
        if keyboard_input.pressed(KeyCode::KeyD) {
            move_direction += camera_right;
        }
        if keyboard_input.pressed(KeyCode::KeyE) || keyboard_input.pressed(KeyCode::Space) {
            move_direction += camera_up;
        }
        if keyboard_input.pressed(KeyCode::KeyQ) || keyboard_input.pressed(KeyCode::ShiftLeft) {
            move_direction -= camera_up;
        }
        let move_speed = 10.0;
        camera_transform.translation +=
            move_direction.normalize_or_zero() * move_speed * time.delta_secs();
    }
    if !cam_state.locked {
        mouse_motion_events.clear();
    }
}
