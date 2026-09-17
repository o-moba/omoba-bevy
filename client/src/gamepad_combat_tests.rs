//! Headless regressions for the actual controller adapter and attack resolver.
use super::*;
use crate::{
    gamepad_controls::GamepadControls, mobile_controls::MobileCastIntent, net::RemotePlayer,
};
use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
use std::time::Duration;

fn fixture() -> (App, Entity, Entity, Entity) {
    let mut app = App::new();
    let mut pad = GamepadControls::default();
    pad.active = true;
    pad.connected = true;
    pad.aim = Some(Vec2::X);
    app.init_resource::<Time>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<TargetState>()
        .init_resource::<PendingCast>()
        .init_resource::<ActionFeedback>()
        .init_resource::<TeamSelection>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(PlayerVisualMode::Models3d)
        .insert_resource(pad)
        .add_message::<NetworkCommand>()
        .add_systems(
            Update,
            (
                tick_basic_attack,
                clear_invalid_selection,
                mobile_basic_attack,
                gamepad_basic_attack,
                resolve_basic_attack,
            )
                .chain(),
        );
    let player = app
        .world_mut()
        .spawn((
            Player,
            Transform::from_xyz(0.0, 0.0, 0.3),
            Team::Green,
            CombatStats::default(),
            NetworkHeroClass(shared::HeroClass::Mage),
            NetworkPlayerId(1),
        ))
        .id();
    let mut spawn_enemy = |x, id| {
        app.world_mut()
            .spawn((
                RemotePlayer,
                Transform::from_xyz(x, 0.0, 0.3),
                Team::Blue,
                CombatStats::default(),
                NetworkPlayerId(id),
                InheritedVisibility::VISIBLE,
            ))
            .id()
    };
    let first = spawn_enemy(0.4, 2);
    let second = spawn_enemy(0.7, 3);
    // Identity projection provides a deterministic viewport with both enemies
    // on the positive-X aim ray, while retaining the real Camera conversion.
    app.world_mut().spawn((
        MainCamera,
        GlobalTransform::IDENTITY,
        Camera {
            computed: ComputedCameraValues {
                clip_from_view: Mat4::IDENTITY,
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(800, 400),
                    scale_factor: 1.0,
                }),
                ..default()
            },
            ..default()
        },
    ));
    (app, player, first, second)
}

fn emitted(app: &mut App) -> Vec<NetworkCommand> {
    app.world_mut()
        .resource_mut::<Messages<NetworkCommand>>()
        .drain()
        .collect()
}

fn assert_attack_to(commands: Vec<NetworkCommand>, id: u64) {
    assert_eq!(commands.len(), 1, "Expected exactly one attack command");
    assert!(matches!(commands[0], NetworkCommand::BasicAttack { target }
        if target == TargetId { kind: TargetKind::Player, id }));
}

#[test]
fn controller_adapter_selects_aimed_enemy_and_shared_cooldown_prevents_duplicates() {
    let (mut app, player, first, _) = fixture();
    app.world_mut()
        .resource_mut::<GamepadControls>()
        .attack_held = true;
    app.update();
    assert_attack_to(emitted(&mut app), 2);
    assert_eq!(
        app.world()
            .resource::<GamepadControls>()
            .candidate
            .map(|p| p.0),
        Some(first)
    );
    assert_eq!(
        app.world().resource::<TargetAimPreview>().candidate,
        Some(first)
    );
    let deadline = app.world().resource::<BasicAttackState>().remaining_secs;
    assert!(deadline > 0.0);
    assert!(app.world().get::<MovementTarget>(player).is_none());
    assert!(app.world().get::<MovementRoute>(player).is_none());

    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(deadline * 0.25));
    app.update();
    assert!(emitted(&mut app).is_empty());
    assert!(app.world().resource::<BasicAttackState>().remaining_secs > 0.0);

    app.world_mut()
        .resource_mut::<GamepadControls>()
        .attack_held = false;
    app.update();
    assert!(emitted(&mut app).is_empty());
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs(2));
    app.update();
    assert!(
        emitted(&mut app).is_empty(),
        "Released R2 cannot leave a queued repeat"
    );
    app.world_mut()
        .resource_mut::<GamepadControls>()
        .attack_held = true;
    app.update();
    assert_attack_to(emitted(&mut app), 2);
}

#[test]
fn invalid_explicit_lock_cancels_held_attack_or_skill_release_before_replacement_is_used() {
    for invalidation in ["dead", "hidden", "friendly", "identity", "despawned"] {
        for skill_release in [false, true] {
            let (mut app, _, first, second) = fixture();
            // Acquire the lock via the actual R3 adapter callback, not by
            // installing a synthetic selected target that may be unreachable.
            app.world_mut()
                .resource_mut::<GamepadControls>()
                .lock_pressed = true;
            app.update();
            assert!(emitted(&mut app).is_empty());
            assert!(app.world().resource::<GamepadControls>().locked);
            assert_eq!(
                app.world().resource::<TargetState>().selected_entity,
                Some(first)
            );
            {
                let mut pad = app.world_mut().resource_mut::<GamepadControls>();
                pad.lock_pressed = false;
                pad.attack_held = !skill_release;
                pad.cast = skill_release.then_some(MobileCastIntent {
                    slot: 0,
                    aim: Some(Vec2::X),
                });
            }
            match invalidation {
                "dead" => app.world_mut().get_mut::<CombatStats>(first).unwrap().hp = 0.0,
                "hidden" => {
                    app.world_mut()
                        .entity_mut(first)
                        .insert(InheritedVisibility::HIDDEN);
                }
                "friendly" => {
                    app.world_mut().entity_mut(first).insert(Team::Green);
                }
                "identity" => {
                    app.world_mut()
                        .entity_mut(first)
                        .insert(NetworkPlayerId(99));
                }
                "despawned" => {
                    app.world_mut().despawn(first);
                }
                _ => unreachable!(),
            }
            app.update();
            assert!(
                emitted(&mut app).is_empty(),
                "Retargeted after {invalidation}"
            );
            let pad = app.world().resource::<GamepadControls>();
            assert!(!pad.locked && !pad.attack_held);
            assert!(pad.cast.is_none() && pad.aiming_slot.is_none());
            assert!(pad.candidate.is_none());
            assert!(
                app.world()
                    .resource::<TargetState>()
                    .selected_entity
                    .is_none()
            );
            assert!(app.world().resource::<BasicAttackState>().order.is_none());
            assert!(!app.world().resource::<TargetAimPreview>().active);
            app.update();
            assert!(emitted(&mut app).is_empty());

            // Prove the replacement was in fact selectable and attackable: a
            // fresh allowed attack intent can hit B after the old one is gone.
            if invalidation == "identity" {
                app.world_mut().get_mut::<CombatStats>(first).unwrap().hp = 0.0;
            }
            app.world_mut()
                .resource_mut::<GamepadControls>()
                .attack_held = true;
            app.update();
            assert_attack_to(emitted(&mut app), 3);
            assert_eq!(
                app.world()
                    .resource::<GamepadControls>()
                    .candidate
                    .map(|p| p.0),
                Some(second)
            );
        }
    }
}

#[test]
fn valid_explicit_lock_does_not_switch_to_nearer_enemy_or_create_an_out_of_range_chase() {
    let (mut app, player, first, second) = fixture();
    app.world_mut()
        .resource_mut::<GamepadControls>()
        .lock_pressed = true;
    app.update();
    app.world_mut()
        .resource_mut::<GamepadControls>()
        .lock_pressed = false;
    app.world_mut()
        .get_mut::<Transform>(first)
        .unwrap()
        .translation
        .x = 50.0;
    app.world_mut()
        .resource_mut::<GamepadControls>()
        .attack_held = true;
    app.update();
    assert!(emitted(&mut app).is_empty());
    let pad = app.world().resource::<GamepadControls>();
    assert!(pad.locked);
    assert_eq!(pad.candidate.map(|p| p.0), Some(first));
    assert_ne!(pad.candidate.map(|p| p.0), Some(second));
    assert!(app.world().get::<MovementTarget>(player).is_none());
    assert!(app.world().get::<MovementRoute>(player).is_none());
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
}
