//! Exercise the shared movement system with desktop controller input.
use super::*;
use crate::{gamepad_controls::GamepadControls, input_context::GameplayInputContext};

#[test]
fn controller_moves_without_touch_resource_and_idle_pad_preserves_click_route() {
    let mut app = App::new();
    let mut pad = GamepadControls::default();
    pad.connected = true;
    app.insert_resource(pad)
        .init_resource::<Time>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<DebugSpeedBoost>()
        .init_resource::<PendingCast>()
        .init_resource::<BasicAttackState>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_systems(Update, move_player_mobile);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(0.05));
    app.world_mut().spawn((
        MainCamera,
        GlobalTransform::from(
            Transform::from_translation(crate::camera::locked_camera_offset(1.0))
                .looking_at(Vec3::ZERO, Vec3::Y),
        ),
    ));
    let destination = Vec3::X * 2.0;
    let actor = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            MovementTarget {
                target: destination,
            },
            MovementRoute {
                requested_target: destination,
                destination,
                structure_revision: 0,
                waypoints: vec![destination],
            },
        ))
        .id();

    app.update();
    assert!(app.world().entity(actor).contains::<MovementRoute>());
    assert!(app.world().entity(actor).contains::<MovementTarget>());
    assert_eq!(
        app.world().get::<Transform>(actor).unwrap().translation,
        Vec3::ZERO
    );

    {
        let mut pad = app.world_mut().resource_mut::<GamepadControls>();
        pad.active = true;
        pad.movement = Vec2::X * 0.5;
    }
    app.update();
    assert!(!app.world().entity(actor).contains::<MovementRoute>());
    assert!(!app.world().entity(actor).contains::<MovementTarget>());
    let half = app.world().get::<Transform>(actor).unwrap().translation;
    assert!(half.z > 0.0 && half.x.abs() < 0.001);
    assert!((half.length() - PLAYER_SPEED * 0.05 * 0.5).abs() < 0.001);

    app.world_mut()
        .get_mut::<Transform>(actor)
        .unwrap()
        .translation = Vec3::ZERO;
    app.world_mut().resource_mut::<GamepadControls>().movement = Vec2::X;
    app.update();
    let full = app.world().get::<Transform>(actor).unwrap().translation;
    assert!((full - half * 2.0).length() < 0.001);

    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = true;
    app.update();
    assert_eq!(
        app.world().get::<Transform>(actor).unwrap().translation,
        full
    );
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = false;
    app.world_mut().get_mut::<CombatStats>(actor).unwrap().hp = 0.0;
    app.update();
    assert_eq!(
        app.world().get::<Transform>(actor).unwrap().translation,
        full
    );
}
