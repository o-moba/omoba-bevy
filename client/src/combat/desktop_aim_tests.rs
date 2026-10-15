//! Desktop aiming of a held direction skill with the mouse: Wildspark's Last Spark (R,
//! `wild_rocket`), reported as "the ultimate cannot be aimed with the mouse on Linux".
//!
//! These drive the real desktop path, not a shortcut through `PendingCast`: Bevy's
//! keyboard input system fed with `KeyboardInput` messages as winit delivers them, the
//! real controller sampler and resolvers (`GamepadPlugin`), the combat chain from
//! `clear_invalid_selection` to `resolve_pending_cast_system` in its production order,
//! and `draw_aim`. The view is a top-down camera over a 3D ground plane: the cursor
//! pixel `(400 + 40 x, 400 + 40 z)` is the ground point `(x, z)`, the hero stands at the
//! origin facing -Z.
//!
//! Five of these reproduced the report before the fix (a lane taken over by a waking
//! controller, focus loss and a cursor outside the window firing along the hero's facing,
//! the readers disagreeing without focus, a same-frame repeat pair); all are regressions.
use super::cast::{cast_spell_system, resolve_pending_cast_system};
use super::cooldown::tick_local_cast_cooldown;
use super::standard::{SkillAimGizmos, SkillAimVector, draw_aim};
use super::{ActionFeedback, LocalCastCooldown, PendingCast, TargetState};
use crate::camera::MainCamera;
use crate::domain::CombatStats;
use crate::input_context::{GameplayInputContext, InputContextSet};
use crate::net::{
    NetworkCommand, NetworkHeroClass, NetworkPlayerId, PlayerLoadout, PlayerProgression,
    SessionEvent,
};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::targeting::{BasicAttackState, TargetAimPreview};
use crate::team::{Team, TeamSelection};
use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardFocusLost, KeyboardInput};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use shared::HeroClass;

/// Wildspark's R: Last Spark, a `Direction` skill.
const ULT: u8 = 3;
/// The hero's facing at rest (`Transform::default()`): where a cast without an aim goes.
const FACING: Vec2 = Vec2::new(0.0, -1.0);
/// Cursor points on four sides of the hero; none of them along `FACING`.
const EAST: Vec2 = Vec2::new(6.0, 0.0);
const NORTH_EAST: Vec2 = Vec2::new(4.0, 4.0);
const SOUTH: Vec2 = Vec2::new(0.0, 6.0);
const WEST: Vec2 = Vec2::new(-6.0, 0.0);

/// One frame of the desktop path.
#[derive(Debug, Default)]
struct Frame {
    /// `CastSkill` commands sent this frame: slot and aim.
    casts: Vec<(u8, Vec2)>,
    /// The direction of the drawn aim lane, if a lane was drawn.
    preview: Option<Vec2>,
}

struct Desk {
    app: App,
    window: Entity,
}

impl Desk {
    /// A Wildspark hero at the origin on a desktop with a focused 800x800 window and,
    /// when `pad` is given, a connected controller entity with that right-stick reading.
    fn new(pad: Option<Vec2>) -> (Self, Option<Entity>) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            bevy::input::InputPlugin,
        ))
        .init_asset::<bevy::gizmos::GizmoAsset>()
        .init_gizmo_group::<SkillAimGizmos>()
        .add_plugins(crate::gamepad::GamepadPlugin)
        .init_resource::<TeamSelection>()
        .init_resource::<GameplayInputContext>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<PendingCast>()
        .init_resource::<super::cast::DesktopAim>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<ActionFeedback>()
        .init_resource::<SkillAimVector>()
        .insert_resource(PlayerVisualMode::Models3d)
        .add_message::<NetworkCommand>()
        .add_message::<SessionEvent>()
        .configure_sets(
            Update,
            (
                InputContextSet::Social,
                InputContextSet::Modal,
                InputContextSet::Resolve,
                InputContextSet::Actions,
            )
                .chain(),
        )
        // The production combat chain (`CombatPlugin`) between the cooldown tick and the
        // pending-cast resolver; `pad_combat` (from `GamepadPlugin`) slots in between the
        // touch attack and the attack resolver as it does in the game.
        .add_systems(
            Update,
            (
                tick_local_cast_cooldown,
                crate::targeting::tick_basic_attack,
                crate::targeting::clear_invalid_selection,
                super::cast::sample_desktop_aim,
                cast_spell_system,
                crate::targeting::mobile_basic_attack,
                crate::targeting::resolve_basic_attack,
                crate::targeting::face_attack_target,
                resolve_pending_cast_system,
            )
                .chain()
                .after(crate::gamepad::GamepadInputSet)
                .in_set(InputContextSet::Actions),
        )
        .add_systems(Update, draw_aim.after(InputContextSet::Actions));
        #[cfg(feature = "qa")]
        app.init_resource::<super::aim_preview::AimPreviewShown>();

        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: bevy::window::WindowResolution::new(800, 800),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        // Straight down onto the ground, twenty units across: view +X is world +X and
        // view +Y is world -Z, so the screen's +Y (down) is world +Z.
        app.world_mut().spawn((
            MainCamera,
            GlobalTransform::from(
                Transform::from_xyz(0.0, 50.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
            ),
            Camera {
                computed: ComputedCameraValues {
                    clip_from_view: Mat4::from_scale(Vec3::new(0.1, 0.1, 0.01)),
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 800),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        app.world_mut().spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression {
                level: 10,
                ..default()
            },
            NetworkPlayerId(1),
            Team::Green,
            NetworkHeroClass(HeroClass::Wildspark),
            PlayerLoadout(Some(shared::loadout::LoadoutState {
                recipe: shared::loadout::preset_for_class(HeroClass::Wildspark)
                    .map(|preset| preset.recipe()),
                ..default()
            })),
        ));
        let pad = pad.map(|right| {
            let mut gamepad = Gamepad::default();
            gamepad.analog_mut().set(GamepadAxis::RightStickX, right.x);
            gamepad.analog_mut().set(GamepadAxis::RightStickY, right.y);
            app.world_mut().spawn(gamepad).id()
        });
        let mut desk = Self { app, window };
        desk.step();
        (desk, pad)
    }

    fn point_at(&mut self, at: Vec2) {
        let pixel = Vec2::new(400.0 + at.x * 40.0, 400.0 + at.y * 40.0);
        self.app
            .world_mut()
            .get_mut::<Window>(self.window)
            .unwrap()
            .set_cursor_position(Some(pixel));
    }

    fn cursor_leaves(&mut self) {
        self.app
            .world_mut()
            .get_mut::<Window>(self.window)
            .unwrap()
            .set_cursor_position(None);
    }

    fn set_focused(&mut self, focused: bool) {
        self.app
            .world_mut()
            .get_mut::<Window>(self.window)
            .unwrap()
            .focused = focused;
    }

    /// One winit keyboard event for R, as Bevy's winit backend forwards it.
    fn key_r(&mut self, state: ButtonState, repeat: bool) {
        let window = self.window;
        self.app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyR,
            logical_key: Key::Character("r".into()),
            state,
            text: None,
            repeat,
            window,
        });
    }

    fn press_r(&mut self) {
        self.key_r(ButtonState::Pressed, false);
    }

    fn release_r(&mut self) {
        self.key_r(ButtonState::Released, false);
    }

    fn step(&mut self) -> Frame {
        self.app.update();
        let casts = self
            .app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .filter_map(|command| match command {
                NetworkCommand::CastSkill { slot, aim } => Some((slot, aim)),
                _ => None,
            })
            .collect();
        let preview = self
            .app
            .world()
            .resource::<SkillAimVector>()
            .0
            .map(|(from, to, _)| (to - from).normalize_or_zero());
        Frame { casts, preview }
    }

    fn pad_active(&self) -> bool {
        self.app
            .world()
            .resource::<crate::gamepad::GamepadControls>()
            .active
    }
}

fn along(direction: Option<Vec2>, toward: Vec2) -> bool {
    direction.is_some_and(|d| d.dot(toward.normalize()) > 0.999)
}

fn assert_previews(frame: &Frame, cursor: Vec2, case: &str) {
    assert!(
        frame.casts.is_empty(),
        "{case}: nothing is cast while R is held: {:?}",
        frame.casts
    );
    assert!(
        along(frame.preview, cursor),
        "{case}: the lane must point at the cursor {cursor}, drew {:?}",
        frame.preview
    );
}

fn assert_sent_at(frame: &Frame, cursor: Vec2, case: &str) {
    match frame.casts.as_slice() {
        [(ULT, aim)] => assert!(
            aim.distance(cursor) < 1e-3,
            "{case}: R must fly at the cursor {cursor} at release, was sent at {aim} \
             (the hero's facing is {FACING})"
        ),
        other => panic!("{case}: exactly one R cast at release, got {other:?}"),
    }
}

/// T1. The baseline: hold R, sweep the cursor around the hero, release.
#[test]
fn held_ultimate_follows_the_cursor_and_flies_where_it_points_at_release() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    for cursor in [NORTH_EAST, SOUTH, WEST] {
        desk.point_at(cursor);
        assert_previews(&desk.step(), cursor, "swept");
        assert_previews(&desk.step(), cursor, "held still");
    }
    desk.release_r();
    let released = desk.step();
    assert_sent_at(&released, WEST, "released");
    assert!(released.preview.is_none(), "no lane once R is up");
    for _ in 0..3 {
        assert!(desk.step().casts.is_empty(), "one release, one cast");
    }
}

/// T2 (H1), the Linux key repeat that winit actually delivers: on Wayland winit repeats
/// in the client, on X11 it turns detectable auto-repeat on
/// (`XkbSetDetectableAutoRepeat`, else window creation fails). Either way a held key
/// repeats as `Pressed` with `repeat: true` and no release.
#[test]
fn key_repeat_as_winit_delivers_it_on_linux_neither_casts_nor_freezes_the_aim() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    for cursor in [NORTH_EAST, SOUTH, WEST, SOUTH] {
        desk.point_at(cursor);
        desk.key_r(ButtonState::Pressed, true);
        assert_previews(&desk.step(), cursor, "repeating");
    }
    desk.point_at(WEST);
    desk.key_r(ButtonState::Pressed, true);
    assert_previews(&desk.step(), WEST, "repeating");
    desk.release_r();
    assert_sent_at(&desk.step(), WEST, "released");
}

/// T2 (H1), the classic X11 auto-repeat without detectable repeat: every repeat is a
/// `Released` + `Pressed` pair with the same timestamp, drained in one winit batch, so
/// both land in one frame. The key is still down (`pressed`), yet `cast_spell_system`
/// sees `just_released` and casts before the player has finished aiming; the real
/// release then casts nothing. winit 0.30.12 does not deliver such pairs (the test above),
/// so this is not the Linux report; it pins the cheap guard against any source that does.
#[test]
fn auto_repeat_release_press_pair_in_one_frame_does_not_cast_before_the_real_release() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    desk.point_at(NORTH_EAST);
    assert_previews(&desk.step(), NORTH_EAST, "aiming");
    // The first auto-repeat, about half a second after the press.
    desk.point_at(SOUTH);
    desk.release_r();
    desk.press_r();
    let repeat = desk.step();
    assert!(
        repeat.casts.is_empty(),
        "an auto-repeat pair while R stays down is not a release: sent {:?}",
        repeat.casts
    );
    desk.point_at(WEST);
    assert_previews(&desk.step(), WEST, "still aiming after the repeat");
    desk.release_r();
    assert_sent_at(&desk.step(), WEST, "the real release");
}

/// T2 (H1), a release and a press on consecutive frames. This is indistinguishable from
/// a real release and a new press, so it pins the current contract: the release casts at
/// the cursor of that frame, the new press only aims again (R is on cooldown).
#[test]
fn release_then_press_on_the_next_frame_casts_once_at_the_cursor_of_the_release() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    desk.point_at(SOUTH);
    desk.release_r();
    assert_sent_at(&desk.step(), SOUTH, "release");
    desk.press_r();
    desk.point_at(WEST);
    let again = desk.step();
    assert!(again.casts.is_empty(), "{:?}", again.casts);
    desk.release_r();
    assert!(
        desk.step().casts.is_empty(),
        "R is on its local cooldown: the second release sends nothing"
    );
}

/// T3 (H2), a connected controller that is idle (any pad, or a Linux joystick device
/// such as a Steam virtual pad): it never owns input, the mouse aims.
#[test]
fn an_idle_connected_controller_leaves_the_aim_to_the_mouse() {
    let (mut desk, pad) = Desk::new(Some(Vec2::ZERO));
    assert!(pad.is_some());
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    for cursor in [SOUTH, WEST] {
        desk.point_at(cursor);
        assert_previews(&desk.step(), cursor, "swept");
    }
    assert!(!desk.pad_active());
    desk.release_r();
    assert_sent_at(&desk.step(), WEST, "released");
}

/// The ways a connected controller wakes up while R is held with the mouse: a stick that
/// drifts past the dead zone and wobbles (a worn stick, a Linux joystick device with noisy
/// axes), or a button blip.
const PAD_WAKES: [&str; 2] = ["stick drift turns", "button blip"];

/// Holds R with the mouse while the controller `wakes` (one of [`PAD_WAKES`]), sweeping
/// the cursor; returns each frame of the hold with its cursor and whether the pad owned
/// input after it, then the frame of the release at [`WEST`].
fn hold_while_a_pad_wakes(wakes: &str) -> (Vec<(Vec2, bool, Frame)>, Frame) {
    // A drift rests at the baseline the sampler records on connection.
    let rest = if wakes == PAD_WAKES[0] {
        Vec2::new(0.4, 0.0)
    } else {
        Vec2::ZERO
    };
    let (mut desk, pad) = Desk::new(Some(rest));
    let pad = pad.unwrap();
    let mut held = Vec::new();
    desk.point_at(EAST);
    desk.press_r();
    let pressed = desk.step();
    assert!(!desk.pad_active(), "{wakes}: the key press keeps the mouse");
    held.push((EAST, false, pressed));
    {
        let mut gamepad = desk.app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        if wakes == PAD_WAKES[0] {
            gamepad.analog_mut().set(GamepadAxis::RightStickX, 0.0);
            gamepad.analog_mut().set(GamepadAxis::RightStickY, 0.4);
        } else {
            gamepad.digital_mut().press(GamepadButton::South);
        }
    }
    desk.point_at(SOUTH);
    let woke = desk.step();
    held.push((SOUTH, desk.pad_active(), woke));
    if wakes == PAD_WAKES[1] {
        let mut gamepad = desk.app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad.digital_mut().release(GamepadButton::South);
        gamepad.digital_mut().clear();
    }
    for cursor in [WEST, NORTH_EAST, WEST] {
        desk.point_at(cursor);
        let frame = desk.step();
        held.push((cursor, desk.pad_active(), frame));
    }
    desk.release_r();
    (held, desk.step())
}

/// T3 (H2): whoever owns input, the R held and released on the keyboard is sent at the
/// cursor. `cast_spell_system` queues it without an aim and `pad_combat` sets one only
/// for a controller release, so the resolver reads the cursor.
#[test]
fn a_controller_waking_up_during_a_mouse_hold_still_sends_r_at_the_cursor() {
    for wakes in PAD_WAKES {
        let (held, released) = hold_while_a_pad_wakes(wakes);
        for (_, _, frame) in &held {
            assert!(frame.casts.is_empty(), "{wakes}: {:?}", frame.casts);
        }
        assert_sent_at(&released, WEST, wakes);
    }
}

/// T3 (H2): ownership passes to the pad on its "deliberate" input, mouse motion never
/// takes it back, and `draw_aim` switches to the controller branch: for the rest of the
/// hold the lane points along the hero's facing (or the stick), not at the cursor that
/// the release will be sent at.
#[test]
fn a_controller_waking_up_during_a_mouse_hold_does_not_take_the_lane_off_the_cursor() {
    for wakes in PAD_WAKES {
        let (held, _) = hold_while_a_pad_wakes(wakes);
        for (cursor, pad_owns, frame) in &held {
            assert_previews(frame, *cursor, &format!("{wakes} (pad owns: {pad_owns})"));
        }
    }
}

/// T4 (H3), `window.focused` false while the cursor is over the window. Real winit
/// delivers no key to an unfocused window (X11 and Wayland alike), so this is reachable
/// only with a stale focus flag. The lane and the cast read one aim source, so whatever
/// is sent is what was drawn, and never the hero's facing.
#[test]
fn without_window_focus_the_sent_aim_matches_the_drawn_lane() {
    let (mut desk, _) = Desk::new(None);
    desk.set_focused(false);
    desk.point_at(EAST);
    desk.press_r();
    let mut drawn = desk.step().preview;
    desk.point_at(WEST);
    let frame = desk.step();
    drawn = frame.preview.or(drawn);
    desk.release_r();
    let released = desk.step();
    for (_, aim) in &released.casts {
        assert!(
            along(drawn, *aim),
            "sent at {aim} ({:?} from the hero) but the lane showed {drawn:?}",
            aim.normalize()
        );
        assert!(
            aim.normalize().dot(FACING) < 0.999,
            "sent along the hero's facing {FACING}, not at the cursor"
        );
    }
}

/// T5 (H4), focus lost while R is held: alt-tab, or GNOME's Super key opening the
/// overview. winit marks the window unfocused; Bevy's winit backend then releases the
/// held keys (a `KeyboardInput` release and `KeyboardFocusLost`, written in `Last`, so
/// read one frame later). `clear_invalid_selection` runs before `cast_spell_system`, which
/// sees R `just_released` and queues the cast; the resolver, with the cursor filtered out
/// by focus, sends it along the hero's facing.
#[test]
fn losing_focus_while_r_is_held_cancels_the_hold_instead_of_casting_along_the_facing() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    desk.point_at(SOUTH);
    assert_previews(&desk.step(), SOUTH, "aiming");
    // Frame N: winit's `Focused(false)`.
    desk.set_focused(false);
    let unfocused = desk.step();
    // Frame N + 1: the releases `check_keyboard_focus_lost` wrote at the end of frame N.
    desk.release_r();
    desk.app.world_mut().write_message(KeyboardFocusLost);
    let released = desk.step();
    let sent: Vec<_> = unfocused.casts.iter().chain(&released.casts).collect();
    assert!(
        sent.is_empty(),
        "a hold interrupted by focus loss casts nothing (as for a modal); sent {sent:?}"
    );
    // Back in the window, a fresh press aims from the cursor again.
    desk.set_focused(true);
    desk.point_at(WEST);
    desk.press_r();
    assert_previews(&desk.step(), WEST, "refocused");
    desk.release_r();
    assert_sent_at(&desk.step(), WEST, "refocused release");
}

/// T5 (H4), the cursor leaves a windowed (or multi-monitor) game while R is held:
/// `cursor_position()` is `None`, the lane disappears, and a release out there sends R
/// along the hero's facing instead of where it was last aimed.
#[test]
fn releasing_with_the_cursor_outside_the_window_never_fires_along_the_facing() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    desk.point_at(SOUTH);
    assert_previews(&desk.step(), SOUTH, "aiming");
    desk.cursor_leaves();
    let outside = desk.step();
    assert!(outside.casts.is_empty());
    desk.release_r();
    let released = desk.step();
    for (_, aim) in &released.casts {
        assert!(
            along(Some(aim.normalize()), SOUTH),
            "either the last aim {SOUTH} or nothing; sent at {aim}"
        );
    }
}

/// T5 (H4), the hero turns while R is held (it walks, or a basic attack faces it at a
/// target): the aim is the cursor's ground point, never the hero's new facing.
#[test]
fn turning_the_hero_during_the_hold_does_not_move_the_aim() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "pressed");
    let mut heroes = desk
        .app
        .world_mut()
        .query_filtered::<&mut Transform, With<Player>>();
    for yaw in [0.5_f32, 1.5, 3.0] {
        heroes.single_mut(desk.app.world_mut()).unwrap().rotation = Quat::from_rotation_y(yaw);
        desk.point_at(SOUTH);
        assert_previews(&desk.step(), SOUTH, "turning");
    }
    desk.release_r();
    assert_sent_at(&desk.step(), SOUTH, "released");
}

/// A tap: R goes down and up within one frame. The repeat-pair guard must not swallow it.
#[test]
fn press_and_release_in_one_frame_casts_once_at_the_cursor() {
    let (mut desk, _) = Desk::new(None);
    desk.point_at(SOUTH);
    desk.press_r();
    desk.release_r();
    assert_sent_at(&desk.step(), SOUTH, "tap");
    assert!(desk.step().casts.is_empty());
}

/// While a skill key is held the controller cannot take input: its wake-up counts as
/// keyboard use, so the lane and the hotbar stay on the mouse and keyboard.
#[test]
fn a_pad_cannot_take_ownership_while_a_skill_key_is_held() {
    for wakes in PAD_WAKES {
        let (held, released) = hold_while_a_pad_wakes(wakes);
        for (cursor, pad_owns, _) in &held {
            assert!(!pad_owns, "{wakes}: the pad took input at {cursor}");
        }
        assert_sent_at(&released, WEST, wakes);
    }
}

/// A tap with the cursor outside the window for the whole hold has no aim: nothing is
/// cast, and in particular not along the hero's facing.
#[test]
fn cursor_outside_for_the_whole_tap_casts_nothing() {
    let (mut desk, _) = Desk::new(None);
    desk.cursor_leaves();
    desk.press_r();
    let pressed = desk.step();
    assert!(pressed.casts.is_empty() && pressed.preview.is_none());
    desk.release_r();
    assert!(desk.step().casts.is_empty());
    desk.point_at(EAST);
    desk.press_r();
    assert_previews(&desk.step(), EAST, "back inside");
    desk.release_r();
    assert_sent_at(&desk.step(), EAST, "back inside");
}
