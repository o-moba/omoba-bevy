use bevy::{
    input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use std::f32::consts::PI;

use crate::input_context::{GameplayInputContext, InputContextSet};
use crate::maps::MapLayout;
use crate::minimap::MinimapNavigationState;
use crate::player::{PLAYER_SIZE, Player};
use crate::sprite::PlayerVisualMode;
use crate::world2d::simulation_xz_to_render_xy;

pub const CAMERA_DISTANCE: f32 = 24.0;
pub const CAMERA_HEIGHT: f32 = 28.0;
pub const CAMERA_MIN_ZOOM: f32 = 0.55;
pub const CAMERA_MAX_ZOOM: f32 = 2.25;
const CAMERA_ZOOM_SPEED: f32 = 0.1;
/// World-units-per-logical-pixel baseline for the genuine 2D camera.
///
/// At `0.16` the 217-unit arena nearly fit across a 1280px viewport and
/// gameplay actors collapsed to 2–8 occupied pixels. `0.08` gives the default
/// view a lane-scale composition while the existing maximum zoom-out still
/// reaches the whole arena.
pub const CAMERA2D_BASE_SCALE: f32 = 0.08;
const CAMERA_ISO_X: f32 = -1.0;
const CAMERA_ISO_Z: f32 = -1.0;
const CAMERA_FREE_FLY_SPEED: f32 = 18.0;
const CAMERA_FREE_FLY_SPRINT_MULTIPLIER: f32 = 2.2;
const CAMERA2D_FREE_PAN_SPEED: f32 = 85.0;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraState>().add_systems(
            Update,
            (toggle_camera_lock, update_cursor_grab, update_camera)
                .chain()
                .in_set(InputContextSet::Actions),
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
    pub orbit_yaw: f32,
    pub orbit_height: f32,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            locked: true,
            pitch: 0.0,
            yaw: 0.0,
            zoom: 1.0,
            orbit_yaw: 0.0,
            orbit_height: 1.0,
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
    keyboard_input: Res<ButtonInput<KeyCode>>,
    context: Res<GameplayInputContext>,
    mut cam_state: ResMut<CameraState>,
    mut minimap_nav: Option<ResMut<MinimapNavigationState>>,
) {
    if !context.camera_allowed() {
        return;
    }
    if keyboard_input.just_pressed(KeyCode::Space) {
        cam_state.locked = true;
        cam_state.orbit_yaw = 0.0;
        cam_state.orbit_height = 1.0;
        if let Some(nav) = minimap_nav.as_deref_mut() {
            nav.focus_target = None;
        }
    } else if context.debug_flight {
        cam_state.locked = false;
    } else if keyboard_input.just_pressed(KeyCode::F8) {
        // Exiting debug flight always restores a useful gameplay view.
        cam_state.locked = true;
    } else if keyboard_input.just_pressed(KeyCode::KeyY) {
        let had_focus_override = minimap_nav
            .as_deref()
            .is_some_and(|nav| nav.focus_target.is_some());
        cam_state.locked = y_follow_state(cam_state.locked, had_focus_override);
        if cam_state.locked {
            if let Some(nav) = minimap_nav.as_deref_mut() {
                nav.focus_target = None;
            }
        }
    }
}

const fn y_follow_state(currently_locked: bool, has_focus_override: bool) -> bool {
    has_focus_override || !currently_locked
}

fn should_capture_cursor(
    mode: PlayerVisualMode,
    right_held: bool,
    alt_held: bool,
    allowed: bool,
) -> bool {
    mode == PlayerVisualMode::Models3d && right_held && alt_held && allowed
}

fn update_cursor_grab(
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    mode: Res<PlayerVisualMode>,
    keyboard: Res<ButtonInput<KeyCode>>,
    context: Res<GameplayInputContext>,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor_options) = cursor_query.single_mut() else {
        return;
    };

    let should_grab = should_capture_cursor(
        *mode,
        mouse_button_input.pressed(MouseButton::Right),
        keyboard.pressed(KeyCode::AltLeft) || keyboard.pressed(KeyCode::AltRight),
        context.camera_allowed(),
    );
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
    mut camera_query: Query<
        (&Camera, &mut Projection, &mut Transform),
        (With<MainCamera>, Without<Player>),
    >,
    player_query: Query<&Transform, (With<Player>, Without<MainCamera>)>,
    mut cam_state: ResMut<CameraState>,
    mode: Res<PlayerVisualMode>,
    map_layout: Res<MapLayout>,
    mut minimap_nav: Option<ResMut<MinimapNavigationState>>,
    mut mouse_motion_events: MessageReader<MouseMotion>,
    mut mouse_wheel_events: MessageReader<MouseWheel>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    context: Res<GameplayInputContext>,
) {
    let Ok((camera, mut projection, mut camera_transform)) = camera_query.single_mut() else {
        return;
    };

    // Always drain mouse motion events to avoid delta bursts after mode switches.
    let mut delta_xy = Vec2::ZERO;
    for event in mouse_motion_events.read() {
        delta_xy += event.delta;
    }

    if !context.camera_allowed() {
        mouse_wheel_events.clear();
        return;
    }
    let rotating = should_capture_cursor(
        *mode,
        mouse_input.pressed(MouseButton::Right),
        keyboard_input.pressed(KeyCode::AltLeft) || keyboard_input.pressed(KeyCode::AltRight),
        true,
    );
    if !rotating {
        delta_xy = Vec2::ZERO;
    }
    if *mode == PlayerVisualMode::Sprite2d {
        update_camera_2d(
            &time,
            camera,
            &mut projection,
            &mut camera_transform,
            &player_query,
            &mut cam_state,
            &map_layout,
            minimap_nav.as_deref_mut(),
            &mut mouse_wheel_events,
            &keyboard_input,
        );
        return;
    }

    if cam_state.locked {
        cam_state.orbit_yaw -= delta_xy.x * 0.004;
        cam_state.orbit_height = (cam_state.orbit_height + delta_xy.y * 0.003).clamp(0.4, 2.2);
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
            let mut offset =
                Quat::from_rotation_y(cam_state.orbit_yaw) * locked_camera_offset(zoom);
            offset.y *= cam_state.orbit_height;
            let target_position = target + offset;
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
    } else if context.debug_flight {
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
        if keyboard_input.pressed(KeyCode::KeyE) {
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

fn clamp_2d_camera_center(
    desired: Vec2,
    map_layout: &MapLayout,
    viewport_size: Vec2,
    projection_scale: f32,
) -> Vec2 {
    let half = viewport_size * projection_scale * 0.5;
    let min = map_layout.min + half;
    let max = map_layout.max - half;
    Vec2::new(
        if min.x <= max.x {
            desired.x.clamp(min.x, max.x)
        } else {
            (map_layout.min.x + map_layout.max.x) * 0.5
        },
        if min.y <= max.y {
            desired.y.clamp(min.y, max.y)
        } else {
            (map_layout.min.y + map_layout.max.y) * 0.5
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn update_camera_2d(
    time: &Time,
    camera: &Camera,
    projection: &mut Projection,
    camera_transform: &mut Transform,
    player_query: &Query<&Transform, (With<Player>, Without<MainCamera>)>,
    cam_state: &mut CameraState,
    map_layout: &MapLayout,
    mut minimap_nav: Option<&mut MinimapNavigationState>,
    mouse_wheel_events: &mut MessageReader<MouseWheel>,
    keyboard_input: &ButtonInput<KeyCode>,
) {
    let Projection::Orthographic(orthographic) = projection else {
        return;
    };

    let mut scroll_delta = 0.0;
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
    orthographic.scale = CAMERA2D_BASE_SCALE * cam_state.zoom;

    if keyboard_input.just_pressed(KeyCode::Space) {
        cam_state.locked = true;
        if let Some(nav) = minimap_nav.as_deref_mut() {
            nav.focus_target = None;
        }
    }

    let mut desired = camera_transform.translation.xy();
    if cam_state.locked {
        let focus = minimap_nav
            .as_deref()
            .and_then(|nav| nav.focus_target)
            .or_else(|| {
                player_query
                    .single()
                    .ok()
                    .map(|transform| transform.translation)
            });
        if let Some(simulation_focus) = focus {
            let target = simulation_xz_to_render_xy(simulation_focus);
            let factor = (time.delta_secs() * 8.0).min(1.0);
            desired = desired.lerp(target, factor);
        }
    } else {
        let mut direction = Vec2::ZERO;
        if keyboard_input.pressed(KeyCode::ArrowUp) {
            direction.y += 1.0;
        }
        if keyboard_input.pressed(KeyCode::ArrowDown) {
            direction.y -= 1.0;
        }
        if keyboard_input.pressed(KeyCode::ArrowLeft) {
            direction.x -= 1.0;
        }
        if keyboard_input.pressed(KeyCode::ArrowRight) {
            direction.x += 1.0;
        }
        desired += direction.normalize_or_zero()
            * CAMERA2D_FREE_PAN_SPEED
            * cam_state.zoom
            * time.delta_secs();
    }

    let viewport = camera
        .logical_viewport_size()
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let clamped = clamp_2d_camera_center(desired, map_layout, viewport, orthographic.scale);
    camera_transform.translation.x = clamped.x;
    camera_transform.translation.y = clamped.y;
    camera_transform.rotation = Quat::IDENTITY;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_limits_are_ordered_and_usable() {
        const {
            assert!(CAMERA_MIN_ZOOM > 0.0);
            assert!(CAMERA_MAX_ZOOM > CAMERA_MIN_ZOOM);
            assert!(CAMERA2D_BASE_SCALE * CAMERA_MAX_ZOOM < 1.0);
            assert!(CAMERA2D_BASE_SCALE * CAMERA_MAX_ZOOM <= 0.18);
        }
    }

    #[test]
    fn clamp_accounts_for_zoom_and_aspect_ratio() {
        let layout = MapLayout::default();
        let viewport = Vec2::new(1600.0, 900.0);
        let scale = CAMERA2D_BASE_SCALE * CAMERA_MIN_ZOOM;
        let clamped = clamp_2d_camera_center(Vec2::splat(10_000.0), &layout, viewport, scale);
        let half = viewport * scale * 0.5;
        assert!(clamped.x <= layout.max.x - half.x + 0.001);
        assert!(clamped.y <= layout.max.y - half.y + 0.001);

        let zoomed_out = clamp_2d_camera_center(
            Vec2::splat(-10_000.0),
            &layout,
            Vec2::new(900.0, 1600.0),
            CAMERA2D_BASE_SCALE * CAMERA_MAX_ZOOM,
        );
        assert!(zoomed_out.x >= layout.min.x - 0.001);
        assert!(zoomed_out.y >= layout.min.y - 0.001);
    }

    #[test]
    fn y_toggles_follow_and_recenters_a_minimap_override() {
        assert!(y_follow_state(false, false));
        assert!(!y_follow_state(true, false));
        assert!(y_follow_state(true, true));
        assert!(y_follow_state(false, true));
    }

    #[test]
    fn camera_gesture_and_recenter_are_explicit() {
        assert!(!should_capture_cursor(
            PlayerVisualMode::Models3d,
            true,
            false,
            true
        ));
        assert!(!should_capture_cursor(
            PlayerVisualMode::Models3d,
            true,
            true,
            false
        ));
        assert!(should_capture_cursor(
            PlayerVisualMode::Models3d,
            true,
            true,
            true
        ));
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<GameplayInputContext>()
            .init_resource::<CameraState>()
            .init_resource::<MinimapNavigationState>()
            .add_systems(Update, toggle_camera_lock);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();
        assert!(app.world().resource::<CameraState>().locked);
        app.world_mut().resource_mut::<CameraState>().locked = false;
        app.world_mut()
            .resource_mut::<MinimapNavigationState>()
            .focus_target = Some(Vec3::X);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.update();
        assert!(app.world().resource::<CameraState>().locked);
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .focus_target
                .is_none()
        );
    }
}
