//! Controller support: desktop (Bevy `Gamepad`, gilrs) and iOS (Apple
//! GameController through a Swift bridge). Android has no gilrs backend in
//! this build and no controller path; see `docs/controller.md`.
//!
//! One [`snapshot::PadSnapshot`] per frame feeds everything downstream:
//! [`ownership`] decides whether the controller owns input, [`gesture`]
//! turns held buttons into skill, attack and lock intents, and [`gameplay`]
//! hands those to the existing movement, cast, attack and upgrade paths.
//! Menus use the kit's generic focus layer (`ui::focus`) and back signal
//! (`ui::back`); this module only feeds them. It never sends a network
//! command of its own kind: casts, attacks and upgrades are the ones mouse,
//! keyboard and touch already send.
// i18n-strict
use bevy::{input::InputSystems, prelude::*, window::PrimaryWindow};

pub(crate) mod gameplay;
pub(crate) mod gesture;
#[cfg(target_os = "ios")]
mod ios;
pub(crate) mod legend;
pub(crate) mod ownership;
pub(crate) mod snapshot;

use crate::input_context::{CombatPointerInputSet, GameplayInputContext, InputContextSet};
use crate::net::TargetId;
use crate::ui::{BackPress, FocusNav, UiFocus};
use gesture::GestureState;
use ownership::Ownership;
use snapshot::{DOWN, EAST, LEFT, PadSnapshot, RIGHT, SOUTH, START, UP};

/// Where controller intents are resolved: after the frame's input context,
/// before pointer combat input and the analog movement step.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GamepadInputSet;

/// The controller's state for this frame. Everything but the private
/// bookkeeping is read by gameplay and HUD systems; only this module and
/// `gameplay` write it.
#[derive(Resource, Default, Debug)]
pub(crate) struct GamepadControls {
    /// A controller is attached (and sampled this frame).
    pub connected: bool,
    /// The controller owns input (not merely connected).
    pub active: bool,
    pub playstation: bool,
    /// Analog movement in screen axes (+Y down), shared with touch movement.
    pub movement: Vec2,
    /// Right-stick direction in screen axes, when pushed.
    pub aim: Option<Vec2>,
    /// The skill being aimed (held).
    pub aiming_slot: Option<usize>,
    /// Skill released this frame.
    pub cast: Option<usize>,
    /// Skill upgraded this frame (North + skill binding).
    pub upgrade: Option<usize>,
    pub attack_held: bool,
    pub lock_pressed: bool,
    /// An explicit R3 lock on `TargetState`'s selection.
    pub locked: bool,
    /// The target the controller would hit now (aim assist result).
    pub candidate: Option<(Entity, TargetId)>,
    /// D-pad right went down in a match: open the shop.
    pub shop_pressed: bool,
    /// D-pad left went down in a match: open the reaction wheel.
    pub reaction_pressed: bool,
    /// Start/Options went down: the `≡` button's path.
    pub menu_pressed: bool,
    pub(crate) raw: Option<PadSnapshot>,
    ownership: Ownership,
    pub(crate) gestures: GestureState,
    cancel_actions: bool,
    back_pressed: bool,
    was_allowed: bool,
}

impl GamepadControls {
    /// Drops the held gesture and every intent derived from it.
    pub(crate) fn cancel_gesture(&mut self) {
        self.gestures = default();
        self.cast = None;
        self.upgrade = None;
        self.aiming_slot = None;
        self.attack_held = false;
        self.lock_pressed = false;
        self.locked = false;
        self.candidate = None;
    }

    /// One sampled frame. `menu` says gameplay was not allowed (last
    /// frame's context): then the D-pad, the left stick and South navigate
    /// the focus layer, and they do not open the shop or reactions. Returns
    /// the focus steps to send.
    pub(crate) fn sample(
        &mut self,
        pad: Option<PadSnapshot>,
        other_input: bool,
        focused: bool,
        menu: bool,
    ) -> Vec<FocusNav> {
        let frame = self.ownership.step(pad, other_input, focused);
        self.connected = pad.is_some();
        self.active = self.ownership.active;
        self.cancel_actions = frame.cancel;
        if frame.reset {
            self.gestures = default();
        }
        if frame.new_device {
            self.was_allowed = false;
        }
        self.raw = frame.pad;
        if let Some(pad) = frame.pad {
            self.playstation = pad.playstation;
        }
        let pressed = frame.pressed;
        self.back_pressed = pressed & EAST != 0;
        self.menu_pressed = pressed & START != 0;
        self.shop_pressed = !menu && pressed & RIGHT != 0;
        self.reaction_pressed = !menu && pressed & LEFT != 0;
        if !menu || !self.active {
            return Vec::new();
        }
        let [up, down, left, right] = frame.stick_nav;
        [
            (pressed & UP != 0 || up, FocusNav::Up),
            (pressed & DOWN != 0 || down, FocusNav::Down),
            (pressed & LEFT != 0 || left, FocusNav::Left),
            (pressed & RIGHT != 0 || right, FocusNav::Right),
            (pressed & SOUTH != 0, FocusNav::Confirm),
        ]
        .into_iter()
        .filter_map(|(on, step)| on.then_some(step))
        .collect()
    }
}

pub(crate) struct GamepadPlugin;

impl Plugin for GamepadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GamepadControls>()
            .init_resource::<BackPress>()
            .init_resource::<UiFocus>()
            .add_message::<FocusNav>()
            .configure_sets(
                Update,
                GamepadInputSet
                    .after(InputContextSet::Resolve)
                    .before(CombatPointerInputSet)
                    .before(crate::mobile_controls::MobileControlsSet::Input),
            )
            .add_systems(PreUpdate, sample_gamepad.after(InputSystems))
            .add_systems(Update, gameplay::resolve_gamepad.in_set(GamepadInputSet))
            .add_systems(
                Update,
                gameplay::pad_combat
                    .after(crate::targeting::mobile_basic_attack)
                    .before(crate::targeting::resolve_basic_attack),
            )
            .add_systems(Startup, legend::setup_legend)
            .add_systems(Update, legend::draw_legend.after(InputContextSet::Resolve));
    }
}

/// Samples the platform's controller once per frame, before any input is
/// read, and feeds the kit's back and focus signals.
fn sample_gamepad(
    mut controls: ResMut<GamepadControls>,
    #[cfg(not(target_os = "ios"))] pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    touches: Option<Res<Touches>>,
    window: Query<&Window, With<PrimaryWindow>>,
    context: Option<Res<GameplayInputContext>>,
    mut focus: ResMut<UiFocus>,
    mut back: ResMut<BackPress>,
    mut nav: MessageWriter<FocusNav>,
    #[cfg(target_os = "ios")] _main_thread: bevy::ecs::system::NonSendMarker,
) {
    #[cfg(not(target_os = "ios"))]
    let pad = snapshot::desktop_snapshot(&pads, controls.ownership.identity());
    #[cfg(target_os = "ios")]
    let pad = snapshot::ios_snapshot();
    let other = keys
        .as_ref()
        .is_some_and(|keys| keys.get_just_pressed().next().is_some())
        || mouse
            .as_ref()
            .is_some_and(|mouse| mouse.get_just_pressed().next().is_some())
        || touches
            .as_ref()
            .is_some_and(|touches| touches.any_just_pressed());
    let focused = window.single().is_ok_and(|window| window.focused);
    let menu = context
        .as_ref()
        .is_none_or(|context| !context.gameplay_allowed());
    let steps = controls.sample(pad, other, focused, menu);
    nav.write_batch(steps);
    focus.set_enabled(controls.active && focused && menu);
    if controls.back_pressed {
        back.press();
    }
}

#[cfg(test)]
mod tests;
