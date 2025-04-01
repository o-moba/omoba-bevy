use bevy::{
    app::AppExit,
    core_pipeline::core_3d::Camera3d,
    input::mouse::MouseMotion,
    math::Dir3,
    math::primitives::{InfinitePlane3d, Plane3d},
    // Assuming Mesh3d is in prelude based on compiler hint E0432
    // Explicitly import MeshMaterial3d from pbr
    pbr::MeshMaterial3d,
    pbr::PbrBundle, // Use PbrBundle again
    prelude::*,
    window::PrimaryWindow,
};
use std::f32::consts::PI;

// Constants remain the same
const PLAYER_SPEED: f32 = 5.0;
const PLAYER_SIZE: f32 = 1.0;
const JUMP_HEIGHT: f32 = 1.5;
const JUMP_DURATION: f32 = 0.6;
const CAMERA_DISTANCE: f32 = 15.0;
const CAMERA_HEIGHT: f32 = 12.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .init_resource::<CameraState>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                handle_player_input,
                move_player,
                animate_jump,
                update_camera,
                toggle_camera_lock,
                handle_exit_input,
            ),
        )
        .run();
}

// Components and Resources remain the same
#[derive(Component)]
struct Player;
#[derive(Component)]
struct MovementTarget {
    target: Vec3,
}
#[derive(Component)]
struct Jumping {
    timer: Timer,
    start_y: f32,
}
#[derive(Component)]
struct MainCamera;
#[derive(Resource, Default)]
struct CameraState {
    locked: bool,
    pitch: f32,
    yaw: f32,
}

/// Creates the initial scene - Using PbrBundle + Compiler Hint Wrappers
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cam_state: ResMut<CameraState>,
) {
    //
    let ground_mesh_handle: Handle<Mesh> =
        meshes.add(Mesh::from(Plane3d::default().mesh().size(50.0, 50.0)));
    let ground_material_handle: Handle<StandardMaterial> =
        materials.add(StandardMaterial::from(Color::srgb(0.3, 0.5, 0.3)));

    commands
        .spawn(PbrBundle {
            mesh: Mesh3d(ground_mesh_handle),
            material: MeshMaterial3d(ground_material_handle),
            transform: Transform::from_xyz(0.0, 0.0, 0.0),
            ..default()
        })
        .insert(Name::new("Ground"));

    let player_mesh_handle: Handle<Mesh> = meshes.add(Mesh::from(Cuboid::new(
        PLAYER_SIZE,
        PLAYER_SIZE,
        PLAYER_SIZE,
    )));
    let player_material_handle: Handle<StandardMaterial> =
        materials.add(StandardMaterial::from(Color::srgb(0.8, 0.7, 0.6)));
    let player_transform = Transform::from_xyz(0.0, PLAYER_SIZE / 2.0, 0.0);
    commands.spawn((
        PbrBundle {
            mesh: Mesh3d(player_mesh_handle),
            material: MeshMaterial3d(player_material_handle),
            transform: player_transform,
            ..default()
        },
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
    cam_state.pitch = -initial_cam_transform.rotation.to_euler(EulerRot::YXZ).1;
    cam_state.yaw = initial_cam_transform.rotation.to_euler(EulerRot::YXZ).0;
    // Camera spawn uses components - this should be fine
    commands.spawn((
        Camera3d::default(),
        initial_cam_transform,
        Visibility::default(),
        MainCamera,
        Name::new("Camera"),
    ));
}

// --- Rest of the systems remain unchanged ---
// handle_player_input, move_player, animate_jump, toggle_camera_lock, update_camera, handle_exit_input
// (Code omitted for brevity, identical to previous correct versions)
/// Handles mouse clicks - Use InfinitePlane3d
fn handle_player_input(
    mut commands: Commands,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    player_query: Query<Entity, With<Player>>,
    cam_state: Res<CameraState>,
) {
    if !cam_state.locked {
        return;
    }
    let Ok(window) = window_query.get_single() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else {
        return;
    };

    if mouse_button_input.just_pressed(MouseButton::Left) {
        if let Some(cursor_pos) = window.cursor_position() {
            if let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) {
                if let Ok(plane_normal) = Dir3::new(Vec3::Y) {
                    let plane_origin = Vec3::ZERO;
                    let infinite_plane = InfinitePlane3d::new(plane_normal); // Construct the plane
                    if let Some(distance) = ray.intersect_plane(plane_origin, infinite_plane) {
                        if distance >= 0.0 {
                            let target_pos = ray.get_point(distance);
                            if let Ok(player_entity) = player_query.get_single() {
                                commands
                                    .entity(player_entity)
                                    .insert(MovementTarget { target: target_pos });
                                commands.entity(player_entity).insert(Jumping {
                                    timer: Timer::from_seconds(JUMP_DURATION, TimerMode::Repeating),
                                    start_y: PLAYER_SIZE / 2.0,
                                });
                            }
                        }
                    }
                } else {
                    warn!("Plane normal is zero, cannot raycast");
                }
            }
        }
    }
}

fn move_player(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Transform, &MovementTarget), With<Player>>,
) {
    for (entity, mut transform, movement_target) in query.iter_mut() {
        let current_pos = transform.translation;
        let target_pos_flat = Vec3::new(
            movement_target.target.x,
            current_pos.y,
            movement_target.target.z,
        );
        let direction = (target_pos_flat - current_pos).normalize_or_zero();
        let distance = current_pos.xz().distance(target_pos_flat.xz());
        let move_delta = PLAYER_SPEED * time.delta_secs();

        if distance < move_delta || distance < 0.01 {
            transform.translation.x = movement_target.target.x;
            transform.translation.z = movement_target.target.z;
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
            transform.translation.y = PLAYER_SIZE / 2.0;
        } else {
            transform.translation += direction * move_delta;
            if direction.length_squared() > 0.001 {
                let target_y_angle = direction.x.atan2(direction.z);
                let target_rotation = Quat::from_rotation_y(target_y_angle);
                transform.rotation = transform
                    .rotation
                    .slerp(target_rotation, time.delta_secs() * 10.0);
            }
        }
    }
}

fn animate_jump(
    time: Res<Time>,
    mut query: Query<(&mut Transform, &mut Jumping), (With<Player>, With<MovementTarget>)>,
) {
    for (mut transform, mut jumping) in query.iter_mut() {
        jumping.timer.tick(time.delta());
        let progress = jumping.timer.fraction();
        let jump_offset = (progress * PI).sin() * JUMP_HEIGHT;
        transform.translation.y = jumping.start_y + jump_offset;
    }
}

fn toggle_camera_lock(
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut cam_state: ResMut<CameraState>,
    window_query: Query<&Window, With<PrimaryWindow>>, // Changed to immutable query
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
            // Cursor modification still commented out
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

fn handle_exit_input(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut app_exit_writer: EventWriter<AppExit>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        info!("Escape pressed, exiting application.");
        app_exit_writer.send(AppExit::Success);
    }
}
