use bevy::{
    input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use std::f32::consts::PI;

use crate::minimap::MinimapNavigationState;
use crate::player::{PLAYER_SIZE, Player};

pub const CAMERA_DISTANCE: f32 = 15.0;
pub const CAMERA_HEIGHT: f32 = 12.0;
const CAMERA_MIN_ZOOM: f32 = 0.4;
const CAMERA_MAX_ZOOM: f32 = 2.5;
const CAMERA_ZOOM_SPEED: f32 = 0.1;
const CAMERA_ISO_X: f32 = -1.0;
const CAMERA_ISO_Z: f32 = -1.0;
const CAMERA_FREE_FLY_SPEED: f32 = 18.0;
const CAMERA_FREE_FLY_SPRINT_MULTIPLIER: f32 = 2.2;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraState>().add_systems(
            Update,
            (update_cursor_grab, toggle_camera_lock, update_camera),
        );
    }
}

#[derive(Component)]
pub struct MainCamera;

#[derive(Resource)]
pub struct CameraState {
    pub locked: bool,
    pub pitch: f32,
    pub yaw: f32,
    pub zoom: f32,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            locked: false,
            pitch: 0.0,
            yaw: 0.0,
            zoom: 1.0,
        }
    }
}

pub fn locked_camera_offset(zoom: f32) -> Vec3 {
    let iso_dir = Vec2::new(CAMERA_ISO_X, CAMERA_ISO_Z).normalize_or_zero();
    Vec3::new(
        iso_dir.x * CAMERA_DISTANCE * zoom,
        CAMERA_HEIGHT * zoom,
        iso_dir.y * CAMERA_DISTANCE * zoom,
    )
}

fn toggle_camera_lock(
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut cam_state: ResMut<CameraState>,
    window_query: Query<&Window, With<PrimaryWindow>>,
) {
    if window_query.single().is_ok() {
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

fn update_cursor_grab(
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor_options) = cursor_query.single_mut() else {
        return;
    };

    let should_grab = mouse_button_input.pressed(MouseButton::Right);
    let target_grab_mode = if should_grab {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    let target_visibility = !should_grab;

    if cursor_options.grab_mode != target_grab_mode {
        cursor_options.grab_mode = target_grab_mode;
    }
    if cursor_options.visible != target_visibility {
        cursor_options.visible = target_visibility;
    }
}

fn update_camera(
    time: Res<Time>,
    mut camera_query: Query<&mut Transform, (With<MainCamera>, Without<Player>)>,
    player_query: Query<&Transform, (With<Player>, Without<MainCamera>)>,
    mut cam_state: ResMut<CameraState>,
    mut minimap_nav: Option<ResMut<MinimapNavigationState>>,
    mut mouse_motion_events: MessageReader<MouseMotion>,
    mut mouse_wheel_events: MessageReader<MouseWheel>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
) {
    let Ok(mut camera_transform) = camera_query.single_mut() else {
        return;
    };

    // Always drain mouse motion events to avoid delta bursts after mode switches.
    let mut delta_xy = Vec2::ZERO;
    for event in mouse_motion_events.read() {
        delta_xy += event.delta;
    }

    if cam_state.locked {
        let focus_override = if let Some(nav_state) = minimap_nav.as_deref_mut() {
            if keyboard_input.just_pressed(KeyCode::Space) {
                nav_state.focus_target = None;
            }
            nav_state.focus_target
        } else {
            None
        };

        let mut scroll_delta: f32 = 0.0;
        for event in mouse_wheel_events.read() {
            let scale = match event.unit {
                MouseScrollUnit::Line => 1.0,
                MouseScrollUnit::Pixel => 0.02,
            };
            scroll_delta += event.y * scale;
        }
        if scroll_delta.abs() > 0.001 {
            cam_state.zoom = (cam_state.zoom - scroll_delta * CAMERA_ZOOM_SPEED)
                .clamp(CAMERA_MIN_ZOOM, CAMERA_MAX_ZOOM);
        }
        let follow_target = if let Some(override_target) = focus_override {
            Some(override_target)
        } else {
            player_query
                .single()
                .ok()
                .map(|transform| transform.translation)
        };
        if let Some(target) = follow_target {
            let zoom = cam_state.zoom;
            let target_position = target + locked_camera_offset(zoom);
            let lerp_factor = (time.delta_secs() * 2.0).min(1.0);
            camera_transform.translation = camera_transform
                .translation
                .lerp(target_position, lerp_factor);
            let look_target = Vec3::new(target.x, PLAYER_SIZE / 2.0, target.z);
            let look_direction = look_target - camera_transform.translation;
            if look_direction.length_squared() > 0.0001 {
                let target_transform = Transform::from_translation(camera_transform.translation)
                    .looking_at(look_target, Vec3::Y);
                camera_transform.rotation = camera_transform
                    .rotation
                    .slerp(target_transform.rotation, lerp_factor);
            }
            cam_state.yaw = camera_transform.rotation.to_euler(EulerRot::YXZ).0;
            cam_state.pitch = camera_transform.rotation.to_euler(EulerRot::YXZ).1;
        }
    } else {
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
        let sprinting = keyboard_input.pressed(KeyCode::ShiftLeft)
            || keyboard_input.pressed(KeyCode::ShiftRight);
        let move_speed = if sprinting {
            CAMERA_FREE_FLY_SPEED * CAMERA_FREE_FLY_SPRINT_MULTIPLIER
        } else {
            CAMERA_FREE_FLY_SPEED
        };
        camera_transform.translation +=
            move_direction.normalize_or_zero() * move_speed * time.delta_secs();
    }
}
