//! ECS regressions for the controller adapter at the input/gameplay boundary:
//! safety cancels, ownership handoff, combat intents on the real resolvers,
//! and the menu signals (focus, back).
use std::time::Duration;

use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
use bevy::prelude::*;

use super::snapshot::{CHORD, EAST, L1, PadSnapshot, R2, SOUTH};
use super::{GamepadControls, gameplay};
use crate::camera::MainCamera;
use crate::combat::{ActionFeedback, CombatStats, PendingCast, TargetState};
use crate::domain::RoundId;
use crate::input_context::GameplayInputContext;
use crate::net::{
    NetworkCommand, NetworkHeroClass, NetworkPlayerId, PlayerProgression, RemotePlayer,
    SessionEvent, TargetId, TargetKind,
};
use crate::player::{MovementRoute, MovementTarget, Player};
use crate::sprite::PlayerVisualMode;
use crate::targeting::{BasicAttackState, TargetAimPreview};
use crate::team::{Team, TeamSelection};
use crate::ui::{BackPress, FocusNav, UiFocus};

fn raw(buttons: u32) -> PadSnapshot {
    PadSnapshot {
        identity: 17,
        buttons,
        ..default()
    }
}

// --- Safety cancels (resolve_gamepad) ---

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
        .add_message::<SessionEvent>()
        .add_systems(Update, gameplay::resolve_gamepad);
    let player = app.world_mut().spawn((Player, CombatStats::default())).id();
    (app, player)
}

fn frame(app: &mut App, pad: Option<PadSnapshot>, other: bool, focus: bool) {
    let menu = !app
        .world()
        .resource::<GameplayInputContext>()
        .gameplay_allowed();
    app.world_mut()
        .resource_mut::<GamepadControls>()
        .sample(pad, other, focus, menu);
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
        "cancellation must preserve the attack cooldown"
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
fn ecs_modal_death_focus_disconnect_replacement_and_round_cancel_every_queued_action() {
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
        let controls = app.world().resource::<GamepadControls>();
        assert_eq!(controls.aiming_slot, Some(0));
        assert_ne!(controls.movement, Vec2::ZERO);
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
                app.world_mut().write_message(SessionEvent::RoundChanged {
                    previous: RoundId {
                        server_epoch: 1,
                        match_id: 1,
                    },
                    current: RoundId {
                        server_epoch: 1,
                        match_id: 2,
                    },
                });
            }
            _ => unreachable!(),
        }
        frame(&mut app, sampled, false, focus);
        assert_cancelled(&app, player);
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = false;
        app.world_mut().get_mut::<CombatStats>(player).unwrap().hp = 100.0;

        // Still-held controls cannot resume movement or release-cast after
        // the blocker clears, even when a button changes during recovery.
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
        let controls = app.world().resource::<GamepadControls>();
        assert_eq!(controls.movement, Vec2::ZERO, "after {blocker}");
        assert!(controls.cast.is_none() && !controls.attack_held);
        let neutral = PadSnapshot {
            identity: resumed.identity,
            ..raw(0)
        };
        frame(&mut app, Some(neutral), false, true);
        assert!(app.world().resource::<GamepadControls>().cast.is_none());
        let attack = PadSnapshot {
            identity: resumed.identity,
            ..raw(R2)
        };
        frame(&mut app, Some(attack), false, true);
        frame(&mut app, Some(attack), false, true);
        assert!(
            app.world().resource::<GamepadControls>().attack_held,
            "failed to rearm after {blocker}"
        );
    }
}

#[test]
fn ecs_east_discards_the_ultimate_and_consumes_its_lingering_trigger() {
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
    assert_eq!(app.world().resource::<GamepadControls>().cast, Some(3));
}

#[test]
fn ecs_hitched_first_trigger_frame_still_allows_adjacent_frame_ultimate_chord() {
    let (mut app, _) = active_app();
    // dt is 100 ms, but it all elapsed before this press: it must not count
    // as held time and send a basic attack before the chord completes.
    frame(&mut app, Some(raw(R2)), false, true);
    assert!(!app.world().resource::<GamepadControls>().attack_held);
    frame(&mut app, Some(raw(CHORD)), false, true);
    let controls = app.world().resource::<GamepadControls>();
    assert_eq!(controls.aiming_slot, Some(3));
    assert!(!controls.attack_held);
    frame(&mut app, Some(raw(R2)), false, true);
    let controls = app.world().resource::<GamepadControls>();
    assert_eq!(controls.cast, Some(3));
    assert!(!controls.attack_held);
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
    // The pointer route issued after the handoff, on the component the
    // movement pipeline reads.
    app.world_mut()
        .entity_mut(player)
        .insert(MovementTarget { target: Vec3::Z });
    frame(&mut app, Some(raw(0)), false, true);
    assert!(!app.world().resource::<GamepadControls>().active);
    assert!(app.world().get::<MovementTarget>(player).is_some());
    frame(&mut app, Some(held), false, true);
    frame(&mut app, Some(held), false, true);
    assert!(
        app.world().get::<MovementTarget>(player).is_none(),
        "a deliberate push takes over and drops the route"
    );
}

#[test]
fn connected_idle_pad_never_overrides_and_menus_get_focus_steps_not_shop_or_reactions() {
    use super::snapshot::{LEFT, RIGHT, START, UP};
    let mut controls = GamepadControls::default();
    assert!(controls.sample(Some(raw(0)), false, true, false).is_empty());
    assert!(!controls.active && controls.connected);
    // In play the D-pad opens the shop and reactions; no focus steps.
    assert!(
        controls
            .sample(Some(raw(RIGHT)), false, true, false)
            .is_empty()
    );
    assert!(controls.active && controls.shop_pressed && !controls.reaction_pressed);
    controls.sample(Some(raw(0)), false, true, false);
    controls.sample(Some(raw(LEFT | START)), false, true, false);
    assert!(controls.reaction_pressed && controls.menu_pressed);
    controls.sample(Some(raw(0)), false, true, true);
    // On a menu the same buttons navigate.
    let steps = controls.sample(Some(raw(RIGHT | UP | SOUTH)), false, true, true);
    assert_eq!(steps, [FocusNav::Up, FocusNav::Right, FocusNav::Confirm]);
    assert!(!controls.shop_pressed && !controls.reaction_pressed);
    assert!(
        controls
            .sample(Some(raw(RIGHT)), false, true, true)
            .is_empty(),
        "held"
    );
    // A mouse press hands input back; menus then get no steps.
    assert!(controls.sample(Some(raw(0)), true, true, true).is_empty());
    assert!(!controls.active);
}

// --- The real sampler on Bevy's Gamepad ---

#[cfg(not(target_os = "ios"))]
fn sampler_app() -> (App, Entity, Entity, Entity) {
    use bevy::input::touch::{TouchInput, touch_screen_input_system};
    use bevy::window::PrimaryWindow;
    let (mut app, player) = test_app();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Touches>()
        .init_resource::<UiFocus>()
        .init_resource::<BackPress>()
        .add_message::<FocusNav>()
        .add_message::<TouchInput>()
        .add_systems(
            PreUpdate,
            (touch_screen_input_system, super::sample_gamepad).chain(),
        );
    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let controller = app.world_mut().spawn(Gamepad::default()).id();
    (app, player, window, controller)
}

#[cfg(not(target_os = "ios"))]
fn set_button(app: &mut App, controller: Entity, button: GamepadButton, down: bool) {
    let mut pad = app.world_mut().get_mut::<Gamepad>(controller).unwrap();
    if down {
        pad.digital_mut().press(button);
    } else {
        pad.digital_mut().release(button);
        pad.digital_mut().clear();
    }
}

#[cfg(not(target_os = "ios"))]
#[test]
fn desktop_adapter_consumes_real_touch_started_event_and_cancels_held_controller_actions() {
    use bevy::input::touch::{TouchInput, TouchPhase};
    let (mut app, player, window, controller) = sampler_app();
    app.update();
    set_button(&mut app, controller, GamepadButton::South, true);
    app.update();
    set_button(&mut app, controller, GamepadButton::South, false);
    app.update();
    set_button(&mut app, controller, GamepadButton::RightTrigger2, true);
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

#[cfg(not(target_os = "ios"))]
#[test]
fn desktop_adapter_feeds_focus_and_back_only_while_it_owns_a_menu() {
    let (mut app, _, _, controller) = sampler_app();
    app.world_mut()
        .resource_mut::<GameplayInputContext>()
        .modal_open = true;
    app.update();
    // Connected and idle: no focus driver, so no ring for a mouse player.
    assert!(app.world().resource::<UiFocus>().focused().is_none());
    set_button(&mut app, controller, GamepadButton::DPadDown, true);
    app.update();
    let steps: Vec<FocusNav> = app
        .world_mut()
        .resource_mut::<Messages<FocusNav>>()
        .drain()
        .collect();
    assert_eq!(steps, [FocusNav::Down]);
    set_button(&mut app, controller, GamepadButton::DPadDown, false);
    set_button(&mut app, controller, GamepadButton::East, true);
    app.update();
    assert!(
        app.world().resource::<BackPress>().pending(),
        "East is a back press"
    );
    // A keyboard press takes input back: no focus driver any more.
    set_button(&mut app, controller, GamepadButton::East, false);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyA);
    app.update();
    assert!(!app.world().resource::<GamepadControls>().active);
}

// --- Combat intents on the real attack and cast resolvers ---

fn combat_fixture() -> (App, Entity, Entity, Entity) {
    let mut app = App::new();
    let pad = GamepadControls {
        active: true,
        connected: true,
        aim: Some(Vec2::X),
        ..default()
    };
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
                crate::targeting::tick_basic_attack,
                crate::targeting::clear_invalid_selection,
                crate::targeting::mobile_basic_attack,
                gameplay::pad_combat,
                crate::targeting::resolve_basic_attack,
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
    // An identity projection gives a deterministic viewport with both
    // enemies on the +X aim ray, through the real Camera conversion.
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
    assert_eq!(
        commands.len(),
        1,
        "expected exactly one attack: {commands:?}"
    );
    assert!(matches!(commands[0], NetworkCommand::BasicAttack { target }
        if target == TargetId { kind: TargetKind::Player, id }));
}

fn pad_mut(app: &mut App) -> Mut<'_, GamepadControls> {
    app.world_mut().resource_mut::<GamepadControls>()
}

#[test]
fn held_r2_attacks_the_aimed_enemy_on_the_shared_cooldown_and_never_chases() {
    let (mut app, player, first, _) = combat_fixture();
    pad_mut(&mut app).attack_held = true;
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

    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(deadline * 0.25));
    app.update();
    assert!(emitted(&mut app).is_empty(), "the cooldown is the cadence");

    pad_mut(&mut app).attack_held = false;
    app.update();
    assert!(emitted(&mut app).is_empty());
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs(2));
    app.update();
    assert!(
        emitted(&mut app).is_empty(),
        "a released R2 leaves no queued repeat"
    );
    pad_mut(&mut app).attack_held = true;
    app.update();
    assert_attack_to(emitted(&mut app), 2);

    // Out of reach: no strike, no walk.
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs(2));
    for enemy in [2, 3] {
        let entity = app
            .world_mut()
            .query::<(Entity, &NetworkPlayerId)>()
            .iter(app.world())
            .find(|(_, id)| id.0 == enemy)
            .unwrap()
            .0;
        app.world_mut()
            .get_mut::<Transform>(entity)
            .unwrap()
            .translation
            .x = 50.0;
    }
    app.update();
    assert!(emitted(&mut app).is_empty());
    assert!(app.world().get::<MovementTarget>(player).is_none());
    assert!(app.world().get::<MovementRoute>(player).is_none());
}

#[test]
fn invalid_explicit_lock_cancels_held_attack_or_skill_release_instead_of_retargeting() {
    for invalidation in ["dead", "hidden", "friendly", "identity", "despawned"] {
        for skill_release in [false, true] {
            let (mut app, _, first, second) = combat_fixture();
            // Acquire the lock through the real R3 path.
            pad_mut(&mut app).lock_pressed = true;
            app.update();
            assert!(emitted(&mut app).is_empty());
            assert!(app.world().resource::<GamepadControls>().locked);
            assert_eq!(
                app.world().resource::<TargetState>().selected_entity,
                Some(first)
            );
            {
                let mut pad = pad_mut(&mut app);
                pad.lock_pressed = false;
                pad.attack_held = !skill_release;
                pad.cast = skill_release.then_some(0);
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
                "retargeted after {invalidation}"
            );
            let pad = app.world().resource::<GamepadControls>();
            assert!(!pad.locked && !pad.attack_held);
            assert!(pad.cast.is_none() && pad.aiming_slot.is_none());
            assert!(pad.candidate.is_none());
            assert!(!app.world().resource::<PendingCast>().is_pending());
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

            // The replacement was selectable: a fresh attack intent hits it.
            if invalidation == "identity" {
                app.world_mut().get_mut::<CombatStats>(first).unwrap().hp = 0.0;
            }
            pad_mut(&mut app).attack_held = true;
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
fn valid_explicit_lock_does_not_switch_to_a_nearer_enemy_or_chase_out_of_range() {
    let (mut app, player, first, second) = combat_fixture();
    pad_mut(&mut app).lock_pressed = true;
    app.update();
    pad_mut(&mut app).lock_pressed = false;
    app.world_mut()
        .get_mut::<Transform>(first)
        .unwrap()
        .translation
        .x = 50.0;
    pad_mut(&mut app).attack_held = true;
    app.update();
    assert!(emitted(&mut app).is_empty());
    let pad = app.world().resource::<GamepadControls>();
    assert!(pad.locked);
    assert_eq!(pad.candidate.map(|p| p.0), Some(first));
    assert_ne!(pad.candidate.map(|p| p.0), Some(second));
    assert!(app.world().get::<MovementTarget>(player).is_none());
    assert!(app.world().get::<MovementRoute>(player).is_none());
    assert!(app.world().resource::<BasicAttackState>().order.is_none());
    // R3 again unlocks.
    pad_mut(&mut app).lock_pressed = true;
    app.update();
    assert!(!app.world().resource::<GamepadControls>().locked);
}

#[test]
fn skill_release_queues_the_aimed_target_and_north_upgrades_only_when_eligible() {
    let (mut app, player, first, _) = combat_fixture();
    // Aiming skill 1 picks with the skill's own range and aim assist.
    pad_mut(&mut app).aiming_slot = Some(0);
    app.update();
    assert_eq!(
        app.world()
            .resource::<GamepadControls>()
            .candidate
            .map(|p| p.0),
        Some(first)
    );
    {
        let mut pad = pad_mut(&mut app);
        pad.aiming_slot = None;
        pad.cast = Some(0);
    }
    app.update();
    assert!(
        app.world().resource::<PendingCast>().has_queued_request(),
        "the release goes through PendingCast"
    );
    assert!(app.world().resource::<GamepadControls>().cast.is_none());
    assert!(
        app.world()
            .resource::<TargetState>()
            .selected_entity
            .is_none(),
        "an aim-assisted cast is not a lock"
    );
    assert!(emitted(&mut app).is_empty(), "the cast resolver sends it");

    // Upgrades use the hotbar's eligibility rule.
    pad_mut(&mut app).upgrade = Some(0);
    app.update();
    assert!(emitted(&mut app).is_empty(), "no skill point");
    app.world_mut()
        .entity_mut(player)
        .insert(PlayerProgression {
            skill_points: 1,
            ..default()
        });
    pad_mut(&mut app).upgrade = Some(0);
    app.update();
    let sent = emitted(&mut app);
    assert!(
        matches!(sent[..], [NetworkCommand::UpgradeSkill { slot: 0 }]),
        "{sent:?}"
    );
    pad_mut(&mut app).upgrade = Some(3);
    app.update();
    assert!(emitted(&mut app).is_empty(), "the ultimate is still locked");
}

/// The aim a release of `slot` by a hero of `class` leaves in the cast queue, with the
/// right stick at `stick`. Two enemies stand next to the hero, an allied hero at full
/// health five units along +X and a hurt one eight units along +Z.
fn released_aim(class: shared::HeroClass, slot: usize, stick: Option<Vec2>) -> Vec2 {
    let (mut app, player, ..) = combat_fixture();
    app.world_mut().entity_mut(player).insert((
        NetworkHeroClass(class),
        crate::net::PlayerLoadout(Some(shared::loadout::LoadoutState {
            recipe: shared::loadout::preset_for_class(class).map(|kit| kit.recipe()),
            ..default()
        })),
    ));
    for (id, at, health) in [(7, Vec2::new(5.0, 0.3), 1.0), (8, Vec2::new(0.0, 8.3), 0.3)] {
        let mut stats = CombatStats::default();
        stats.hp *= health;
        app.world_mut().spawn((
            RemotePlayer,
            Transform::from_xyz(at.x, 0.0, at.y),
            Team::Green,
            stats,
            NetworkPlayerId(id),
            InheritedVisibility::VISIBLE,
        ));
    }
    {
        let mut pad = pad_mut(&mut app);
        pad.aim = stick;
        pad.cast = Some(slot);
    }
    app.update();
    let pending = app.world().resource::<PendingCast>();
    assert!(pending.has_queued_request(), "{class:?} {slot}");
    pending.aim.expect("a modular cast is queued with its aim")
}

#[test]
fn a_press_without_the_stick_aims_a_skill_cast_on_an_ally_at_an_ally() {
    use shared::HeroClass;
    use shared::loadout::{SkillId, preset_for_class, skill};

    let slot_of = |class, id| {
        let kit = preset_for_class(class).unwrap();
        kit.skills().iter().position(|skill| *skill == id).unwrap()
    };
    let origin = Vec2::new(0.0, 0.3);
    let leap = (HeroClass::Frostguard, SkillId::ShelteringLeap);
    let guard = (HeroClass::Orbitwright, SkillId::OrbitalGuard);
    // The leap goes to the allied hero lowest on health, the orb to the nearest one: not
    // straight ahead, where nobody stands, and not at the enemies beside the hero.
    let aim = released_aim(leap.0, slot_of(leap.0, leap.1), None);
    assert!(aim.distance(Vec2::new(0.0, 8.3)) < 1e-4, "{aim}");
    let aim = released_aim(guard.0, slot_of(guard.0, guard.1), None);
    assert!(aim.distance(Vec2::new(5.0, 0.3)) < 1e-4, "{aim}");
    // The stick is the player's own aim and is not replaced.
    for (class, id) in [leap, guard] {
        let aim = released_aim(class, slot_of(class, id), Some(Vec2::X));
        let range = skill(id).ability.cast_range;
        assert!(
            aim.distance(origin + Vec2::X * range) < 1e-4,
            "{id:?} {aim}"
        );
    }
    // Every other skill is still sent straight ahead without the stick: a lane, and the
    // leap that may go without an ally (Anchor Step).
    for (class, id) in [
        (HeroClass::Frostguard, SkillId::WinterShard),
        (HeroClass::Stormfist, SkillId::AnchorStep),
    ] {
        let aim = released_aim(class, slot_of(class, id), None);
        let range = skill(id).ability.cast_range;
        assert!(
            aim.distance(origin + Vec2::NEG_Y * range) < 1e-4,
            "{id:?} {aim}"
        );
    }
}
