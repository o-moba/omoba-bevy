//! ECS regressions for cancellation at the input/gameplay boundary.
use super::*;
use crate::net::{TargetId, TargetKind};
use std::time::Duration;

fn raw(buttons: u32) -> PadSnapshot {
    PadSnapshot {
        identity: 17,
        buttons,
        ..default()
    }
}

fn test_app() -> (App, Entity) {
    let mut app = App::new();
    let mut time = Time::<()>::default();
    time.advance_by(Duration::from_millis(100));
    app.insert_resource(time)
        .init_resource::<GamepadControls>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<PendingCast>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<GameStateSnapshot>()
        .add_systems(Update, resolve_gamepad);
    let player = app.world_mut().spawn((Player, CombatStats::default())).id();
    (app, player)
}

fn frame(app: &mut App, pad: Option<PadSnapshot>, other: bool, focus: bool) {
    app.world_mut()
        .resource_mut::<GamepadControls>()
        .sample(pad, other, focus);
    app.update();
}

fn active_app() -> (App, Entity) {
    let (mut app, player) = test_app();
    frame(&mut app, Some(raw(0)), false, true);
    frame(&mut app, Some(raw(SOUTH)), false, true);
    frame(&mut app, Some(raw(0)), false, true);
    assert!(app.world().resource::<GamepadControls>().gestures.armed);
    (app, player)
}

fn seed_queued_actions(app: &mut App, player: Entity) {
    let enemy = app.world_mut().spawn_empty().id();
    let id = TargetId {
        kind: TargetKind::Player,
        id: 999,
    };
    app.insert_resource(PendingCast::queued_for_movement_test());
    let mut basic = app.world_mut().resource_mut::<BasicAttackState>();
    basic.start(enemy, id, true);
    basic.remaining_secs = 0.75;
    let mut target = app.world_mut().resource_mut::<TargetState>();
    target.selected_entity = Some(enemy);
    target.selected_target = Some(id);
    let mut preview = app.world_mut().resource_mut::<TargetAimPreview>();
    preview.active = true;
    preview.candidate = Some(enemy);
    preview.target = Some(id);
    let mut controls = app.world_mut().resource_mut::<GamepadControls>();
    controls.locked = true;
    controls.candidate = Some((enemy, id));
    app.world_mut().entity_mut(player).insert((
        MovementTarget { target: Vec3::X },
        MovementRoute {
            requested_target: Vec3::X,
            destination: Vec3::X,
            structure_revision: 1,
            waypoints: vec![Vec3::X],
        },
    ));
}

fn assert_cancelled(app: &App, player: Entity) {
    let controls = app.world().resource::<GamepadControls>();
    assert_eq!(controls.movement, Vec2::ZERO);
    assert!(controls.aim.is_none());
    assert!(controls.aiming_slot.is_none());
    assert!(controls.cast.is_none() && controls.upgrade.is_none());
    assert!(!controls.attack_held && !controls.lock_pressed && !controls.locked);
    assert!(controls.candidate.is_none());
    assert!(!app.world().resource::<PendingCast>().is_pending());
    let basic = app.world().resource::<BasicAttackState>();
    assert!(basic.order.is_none());
    assert_eq!(
        basic.remaining_secs, 0.75,
        "Cancellation must preserve attack cooldown"
    );
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none()
    );
    assert!(!app.world().resource::<TargetAimPreview>().active);
    assert!(app.world().get::<MovementTarget>(player).is_none());
    assert!(app.world().get::<MovementRoute>(player).is_none());
}

#[test]
fn ecs_modal_death_focus_disconnect_and_replacement_cancel_every_queued_action() {
    for blocker in [
        "modal",
        "death",
        "focus",
        "disconnect",
        "replacement",
        "round",
    ] {
        let (mut app, player) = active_app();
        let held = PadSnapshot {
            left: Vec2::X,
            right: Vec2::Y,
            ..raw(L1)
        };
        frame(&mut app, Some(held), false, true);
        assert_eq!(
            app.world().resource::<GamepadControls>().aiming_slot,
            Some(0)
        );
        assert_ne!(
            app.world().resource::<GamepadControls>().movement,
            Vec2::ZERO
        );
        seed_queued_actions(&mut app, player);
        let mut sampled = Some(held);
        let mut focus = true;
        match blocker {
            "modal" => {
                app.world_mut()
                    .resource_mut::<GameplayInputContext>()
                    .modal_open = true
            }
            "death" => app.world_mut().get_mut::<CombatStats>(player).unwrap().hp = 0.0,
            "focus" => focus = false,
            "disconnect" => sampled = None,
            "replacement" => sampled.as_mut().unwrap().identity += 1,
            "round" => {
                app.world_mut()
                    .resource_mut::<GameStateSnapshot>()
                    .meta
                    .match_id += 1
            }
            _ => unreachable!(),
        }
        frame(&mut app, sampled, false, focus);
        assert_cancelled(&app, player);
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = false;
        app.world_mut().get_mut::<CombatStats>(player).unwrap().hp = 100.0;

        // Still-held controls cannot resume movement or generate a release cast
        // after the blocker clears, even when a button changes during recovery.
        let resumed = sampled.unwrap_or(held);
        frame(&mut app, Some(resumed), false, true);
        frame(
            &mut app,
            Some(PadSnapshot {
                buttons: L1 | R2,
                ..resumed
            }),
            false,
            true,
        );
        assert_eq!(
            app.world().resource::<GamepadControls>().movement,
            Vec2::ZERO
        );
        assert!(app.world().resource::<GamepadControls>().cast.is_none());
        assert!(!app.world().resource::<GamepadControls>().attack_held);
        frame(
            &mut app,
            Some(PadSnapshot {
                identity: resumed.identity,
                ..raw(0)
            }),
            false,
            true,
        );
        assert!(app.world().resource::<GamepadControls>().cast.is_none());
        frame(
            &mut app,
            Some(PadSnapshot {
                identity: resumed.identity,
                ..raw(R2)
            }),
            false,
            true,
        );
        frame(
            &mut app,
            Some(PadSnapshot {
                identity: resumed.identity,
                ..raw(R2)
            }),
            false,
            true,
        );
        assert!(
            app.world().resource::<GamepadControls>().attack_held,
            "Failed to rearm after {blocker}"
        );
    }
}

#[test]
fn ecs_cancel_button_discards_ultimate_and_consumes_its_lingering_trigger() {
    let (mut app, player) = active_app();
    frame(&mut app, Some(raw(CHORD)), false, true);
    assert_eq!(
        app.world().resource::<GamepadControls>().aiming_slot,
        Some(3)
    );
    seed_queued_actions(&mut app, player);
    frame(&mut app, Some(raw(CHORD | EAST)), false, true);
    assert_cancelled(&app, player);
    for buttons in [R2, R2, 0] {
        frame(&mut app, Some(raw(buttons)), false, true);
        let controls = app.world().resource::<GamepadControls>();
        assert!(controls.cast.is_none() && !controls.attack_held);
    }
    frame(&mut app, Some(raw(CHORD)), false, true);
    frame(&mut app, Some(raw(0)), false, true);
    assert_eq!(
        app.world()
            .resource::<GamepadControls>()
            .cast
            .map(|cast| cast.slot),
        Some(3)
    );
}

#[test]
fn ecs_hitched_first_trigger_frame_still_allows_adjacent_frame_ultimate_chord() {
    let (mut app, _) = active_app();
    // dt is 100ms, but all of it elapsed before this newly sampled press. It
    // must not count as already-held time and prematurely send a basic attack.
    frame(&mut app, Some(raw(R2)), false, true);
    assert!(!app.world().resource::<GamepadControls>().attack_held);
    frame(&mut app, Some(raw(CHORD)), false, true);
    let controls = app.world().resource::<GamepadControls>();
    assert_eq!(controls.aiming_slot, Some(3));
    assert!(!controls.attack_held);
    frame(&mut app, Some(raw(R2)), false, true);
    let controls = app.world().resource::<GamepadControls>();
    assert_eq!(controls.cast.map(|cast| cast.slot), Some(3));
    assert!(!controls.attack_held);
}

#[test]
fn smooth_deliberate_stick_motion_activates_without_a_single_large_frame_delta() {
    let mut controls = GamepadControls::default();
    controls.sample(Some(raw(0)), false, true);
    for step in 1..=20 {
        controls.sample(
            Some(PadSnapshot {
                left: Vec2::X * (step as f32 * 0.05),
                ..raw(0)
            }),
            false,
            true,
        );
    }
    assert!(
        controls.active,
        "A deliberate full stick tilt must activate at any frame rate"
    );
}

#[test]
fn centering_old_stick_after_mouse_handoff_cannot_cancel_the_mouse_route() {
    let (mut app, player) = active_app();
    let held = PadSnapshot {
        left: Vec2::X,
        ..raw(0)
    };
    frame(&mut app, Some(held), false, true);
    frame(&mut app, Some(held), true, true);
    assert!(!app.world().resource::<GamepadControls>().active);
    // Represents the new pointer route issued after controller cancellation in
    // the handoff frame, using the same component the movement pipeline reads.
    app.world_mut()
        .entity_mut(player)
        .insert(MovementTarget { target: Vec3::Z });
    frame(&mut app, Some(raw(0)), false, true);
    assert!(!app.world().resource::<GamepadControls>().active);
    assert!(app.world().get::<MovementTarget>(player).is_some());
}

#[cfg(not(target_os = "ios"))]
#[test]
fn desktop_adapter_consumes_real_touch_started_event_and_cancels_held_controller_actions() {
    use bevy::input::touch::{TouchInput, TouchPhase, touch_screen_input_system};
    let (mut app, player) = test_app();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Touches>()
        .add_message::<TouchInput>()
        .add_systems(
            PreUpdate,
            (touch_screen_input_system, sample_gamepad).chain(),
        );
    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let controller = app.world_mut().spawn(Gamepad::default()).id();
    app.update();
    app.world_mut()
        .get_mut::<Gamepad>(controller)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::South);
    app.update();
    app.world_mut()
        .get_mut::<Gamepad>(controller)
        .unwrap()
        .digital_mut()
        .release(GamepadButton::South);
    app.update();
    app.world_mut()
        .get_mut::<Gamepad>(controller)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::RightTrigger2);
    app.update();
    app.update();
    assert!(app.world().resource::<GamepadControls>().attack_held);
    seed_queued_actions(&mut app, player);
    app.world_mut().write_message(TouchInput {
        phase: TouchPhase::Started,
        position: Vec2::new(80.0, 120.0),
        window,
        force: None,
        id: 3,
    });
    app.update();
    assert_cancelled(&app, player);
    assert!(!app.world().resource::<GamepadControls>().active);
    app.update();
    assert!(!app.world().resource::<GamepadControls>().active);
    assert!(!app.world().resource::<GamepadControls>().attack_held);
}
