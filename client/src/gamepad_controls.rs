//! Device adapters produce intents; existing gameplay resolvers retain authority.
use bevy::{ecs::system::NonSendMarker, input::InputSystems, prelude::*, window::PrimaryWindow};

use crate::{
    combat::{CombatPointerInputSet, CombatStats, PendingCast, TargetState},
    input_context::{GameplayInputContext, InputContextSet},
    mobile_controls::MobileCastIntent,
    net::GameStateSnapshot,
    player::{MovementRoute, MovementTarget, Player},
    targeting::{BasicAttackState, TargetAimPreview},
};

pub(crate) const SOUTH: u32 = 1 << 0;
pub(crate) const EAST: u32 = 1 << 1;
pub(crate) const NORTH: u32 = 1 << 3;
const L1: u32 = 1 << 4;
const R1: u32 = 1 << 5;
const L2: u32 = 1 << 6;
const R2: u32 = 1 << 7;
const R3: u32 = 1 << 9;
const UP: u32 = 1 << 10;
const DOWN: u32 = 1 << 11;
const LEFT: u32 = 1 << 12;
const RIGHT: u32 = 1 << 13;
const START: u32 = 1 << 14;
const CHORD: u32 = L2 | R2;
const DEAD_ZONE: f32 = 0.18;
// A short grace interval lets the two ultimate triggers arrive on adjacent frames.
const ATTACK_CHORD_GRACE: f32 = 0.08;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GamepadInputSet {
    Sample,
}

#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct GamepadNavigation {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub confirm: bool,
    pub cancel: bool,
    pub menu: bool,
}

#[derive(Clone, Copy, Default, Debug)]
struct PadSnapshot {
    identity: u64,
    playstation: bool,
    left: Vec2,
    right: Vec2,
    buttons: u32,
}

impl PadSnapshot {
    fn neutral(self) -> bool {
        self.buttons == 0 && self.left == Vec2::ZERO && self.right == Vec2::ZERO
    }
}

#[derive(Default, Debug)]
struct GestureState {
    armed: bool,
    previous: u32,
    slot: Option<usize>,
    upgrading: bool,
    chord_latched: bool,
    attack_seconds: f32,
}

#[derive(Default, Debug)]
struct GestureIntent {
    slot: Option<usize>,
    cast: Option<usize>,
    upgrade: Option<usize>,
    attack: bool,
    lock: bool,
}

impl GestureState {
    fn step(&mut self, pad: PadSnapshot, allowed: bool, dt: f32) -> GestureIntent {
        let mut out = GestureIntent::default();
        if !allowed || pad.buttons & EAST != 0 {
            *self = Self::default();
            return out;
        }
        if !self.armed {
            self.armed = pad.neutral();
            self.previous = pad.buttons;
            return out;
        }
        let pressed = pad.buttons & !self.previous;
        out.lock = pressed & R3 != 0;
        if pad.buttons & CHORD == CHORD && !self.chord_latched {
            self.slot = Some(3);
            self.upgrading = pad.buttons & NORTH != 0;
            self.chord_latched = true;
        } else if self.slot.is_none() && !self.chord_latched {
            if let Some(slot) = [L1, R1, L2].iter().position(|key| pressed & key != 0) {
                self.slot = Some(slot);
                self.upgrading = pad.buttons & NORTH != 0;
            }
        }
        if let Some(slot) = self.slot {
            let held = if slot == 3 {
                pad.buttons & CHORD == CHORD
            } else {
                pad.buttons & [L1, R1, L2][slot] != 0
            };
            if !held {
                if self.upgrading {
                    out.upgrade = Some(slot);
                } else {
                    out.cast = Some(slot);
                }
                self.slot = None;
            }
        }
        // Once either ultimate trigger is released, consume the other until both
        // are up. Otherwise releasing an ultimate could immediately basic-attack.
        if pad.buttons & CHORD == 0 {
            self.chord_latched = false;
        }
        if pad.buttons & R2 != 0 && pad.buttons & L2 == 0 && pressed & R2 == 0 {
            self.attack_seconds += dt.clamp(0.0, 0.1);
        } else {
            self.attack_seconds = 0.0;
        }
        out.slot = self.slot;
        out.attack = self.attack_seconds >= ATTACK_CHORD_GRACE
            && self.slot.is_none()
            && !self.chord_latched
            && out.cast.is_none()
            && out.upgrade.is_none()
            && pad.buttons & NORTH == 0;
        self.previous = pad.buttons;
        out
    }
}

#[derive(Resource, Default)]
pub(crate) struct GamepadControls {
    pub active: bool,
    pub connected: bool,
    pub playstation: bool,
    /// Screen axes (+Y down), shared with touch movement and target assistance.
    pub movement: Vec2,
    pub aim: Option<Vec2>,
    pub aiming_slot: Option<usize>,
    pub nav: GamepadNavigation,
    pub cast: Option<MobileCastIntent>,
    pub upgrade: Option<usize>,
    pub attack_held: bool,
    pub lock_pressed: bool,
    pub shop_pressed: bool,
    pub reaction_pressed: bool,
    pub locked: bool,
    pub candidate: Option<(Entity, crate::net::TargetId)>,
    raw: Option<PadSnapshot>,
    last_raw: Option<PadSnapshot>,
    activation_reference: Option<PadSnapshot>,
    gestures: GestureState,
    cancel_actions: bool,
    was_allowed: bool,
    round: Option<(u64, u64)>,
}

impl GamepadControls {
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

    pub fn skill_labels(&self) -> [&'static str; 4] {
        if self.playstation {
            ["L1", "R1", "L2", "L2+R2"]
        } else {
            ["LB", "RB", "LT", "LT+RT"]
        }
    }

    fn sample(&mut self, pad: Option<PadSnapshot>, other_input: bool, focused: bool) {
        self.nav = default();
        self.shop_pressed = false;
        self.reaction_pressed = false;
        self.cancel_actions = false;
        self.connected = pad.is_some();
        if self.last_raw.map(|p| p.identity) != pad.map(|p| p.identity) {
            self.cancel_actions = self.active;
            self.gestures = default();
            self.was_allowed = false;
            self.last_raw = pad;
            self.activation_reference = pad;
        }
        let Some(pad) = pad.filter(|_| focused) else {
            self.cancel_actions |= self.active;
            self.active = false;
            self.gestures = default();
            self.raw = None;
            self.last_raw = None;
            return;
        };
        self.playstation = pad.playstation;
        let old = self.last_raw.unwrap_or(pad);
        let pressed = pad.buttons & !old.buttons;
        if other_input {
            self.cancel_actions |= self.active;
            self.active = false;
            self.gestures = default();
            self.activation_reference = Some(pad);
        } else if pressed != 0
            || deliberate_stick(pad.left, self.activation_reference.unwrap_or_default().left)
            || deliberate_stick(
                pad.right,
                self.activation_reference.unwrap_or_default().right,
            )
        {
            if !self.active {
                self.cancel_actions = true;
            }
            self.active = true;
        }
        if self.active {
            self.activation_reference = Some(pad);
            self.shop_pressed = pressed & RIGHT != 0;
            self.reaction_pressed = pressed & LEFT != 0;
            self.nav = GamepadNavigation {
                up: pressed & UP != 0 || (pad.left.y > 0.65 && old.left.y <= 0.65),
                down: pressed & DOWN != 0 || (pad.left.y < -0.65 && old.left.y >= -0.65),
                left: pressed & LEFT != 0 || (pad.left.x < -0.65 && old.left.x >= -0.65),
                right: pressed & RIGHT != 0 || (pad.left.x > 0.65 && old.left.x <= 0.65),
                confirm: pressed & SOUTH != 0,
                cancel: pressed & EAST != 0,
                menu: pressed & START != 0,
            };
        }
        if !self.active {
            let reference = self.activation_reference.get_or_insert(pad);
            if pad.left == Vec2::ZERO {
                reference.left = Vec2::ZERO;
            }
            if pad.right == Vec2::ZERO {
                reference.right = Vec2::ZERO;
            }
        }
        self.last_raw = Some(pad);
        self.raw = Some(pad);
    }
}

fn deliberate_stick(now: Vec2, baseline: Vec2) -> bool {
    now.length() > 0.12
        && (now.length() > baseline.length() + 0.12
            || (baseline.length() > 0.12 && now.normalize().dot(baseline.normalize()) < 0.9))
}

#[cfg(debug_assertions)]
#[path = "gamepad_qa.rs"]
mod qa;

pub(crate) struct GamepadControlsPlugin;
impl Plugin for GamepadControlsPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(debug_assertions)]
        qa::configure(app);
        app.init_resource::<GamepadControls>()
            .add_systems(
                PreUpdate,
                sample_gamepad
                    .after(InputSystems)
                    .in_set(GamepadInputSet::Sample),
            )
            .add_systems(
                Update,
                resolve_gamepad
                    .after(InputContextSet::Resolve)
                    .before(CombatPointerInputSet),
            );
    }
}

pub(crate) fn radial_dead_zone(value: Vec2) -> Vec2 {
    if !value.is_finite() {
        return Vec2::ZERO;
    }
    let length = value.length();
    if length <= DEAD_ZONE {
        return Vec2::ZERO;
    }
    value / length * ((length.min(1.0) - DEAD_ZONE) / (1.0 - DEAD_ZONE))
}

#[cfg(not(target_os = "ios"))]
fn desktop_snapshot(
    pads: &Query<(Entity, &Gamepad, Option<&Name>)>,
    previous: Option<u64>,
) -> Option<PadSnapshot> {
    let chosen = pads
        .iter()
        .find(|(entity, _, _)| Some(entity.to_bits()) == previous)
        .or_else(|| pads.iter().min_by_key(|(entity, _, _)| entity.to_bits()))?;
    let (entity, pad, name) = chosen;
    let buttons = [
        GamepadButton::South,
        GamepadButton::East,
        GamepadButton::West,
        GamepadButton::North,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
        GamepadButton::LeftTrigger2,
        GamepadButton::RightTrigger2,
        GamepadButton::LeftThumb,
        GamepadButton::RightThumb,
        GamepadButton::DPadUp,
        GamepadButton::DPadDown,
        GamepadButton::DPadLeft,
        GamepadButton::DPadRight,
        GamepadButton::Start,
        GamepadButton::Select,
    ]
    .iter()
    .enumerate()
    .fold(0, |bits, (i, button)| {
        bits | if pad.pressed(*button) { 1 << i } else { 0 }
    });
    let name = name.map(|n| n.as_str()).unwrap_or_default().to_lowercase();
    Some(PadSnapshot {
        identity: entity.to_bits(),
        buttons,
        playstation: [
            "sony",
            "dualsense",
            "dualshock",
            "playstation",
            "ps4",
            "ps5",
        ]
        .iter()
        .any(|s| name.contains(s)),
        left: radial_dead_zone(pad.left_stick()),
        right: radial_dead_zone(pad.right_stick()),
    })
}

fn sample_gamepad(
    mut controller: ResMut<GamepadControls>,
    #[cfg(not(target_os = "ios"))] pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    window: Query<&Window, With<PrimaryWindow>>,
    _main_thread: NonSendMarker,
) {
    #[cfg(not(target_os = "ios"))]
    let pad = desktop_snapshot(&pads, controller.last_raw.map(|p| p.identity));
    #[cfg(target_os = "ios")]
    let pad = crate::gamepad_ios::poll().map(|p| PadSnapshot {
        identity: p.identity,
        playstation: p.is_playstation,
        buttons: p.buttons,
        left: radial_dead_zone(Vec2::from_array(p.left_stick)),
        right: radial_dead_zone(Vec2::from_array(p.right_stick)),
    });
    let other = keys.get_just_pressed().next().is_some()
        || mouse.get_just_pressed().next().is_some()
        || touches.any_just_pressed();
    controller.sample(pad, other, window.single().is_ok_and(|w| w.focused));
}

fn resolve_gamepad(
    mut controller: ResMut<GamepadControls>,
    context: Res<GameplayInputContext>,
    time: Res<Time>,
    snapshot: Option<Res<GameStateSnapshot>>,
    local: Query<(Entity, &CombatStats), With<Player>>,
    mut commands: Commands,
    mut pending: ResMut<PendingCast>,
    mut basic: ResMut<BasicAttackState>,
    mut target: ResMut<TargetState>,
    mut preview: ResMut<TargetAimPreview>,
) {
    let alive = local.single().is_ok_and(|(_, stats)| stats.is_alive());
    let round = snapshot
        .as_ref()
        .map(|s| (s.meta.server_epoch, s.meta.match_id));
    let changed_round = controller.round.is_some() && controller.round != round;
    controller.round = round;
    let allowed =
        controller.active && controller.raw.is_some() && context.gameplay_allowed() && alive;
    let cancel = controller.cancel_actions
        || changed_round
        || (controller.was_allowed && !allowed)
        || (controller.active && controller.nav.cancel);
    if cancel {
        controller.gestures = default();
        controller.locked = false;
        controller.candidate = None;
        pending.cancel();
        basic.cancel();
        *preview = default();
        target.selected_entity = None;
        target.selected_target = None;
        for (entity, _) in &local {
            commands
                .entity(entity)
                .remove::<(MovementTarget, MovementRoute)>();
        }
    }
    let pad = controller.raw.unwrap_or_default();
    let intent = controller.gestures.step(pad, allowed, time.delta_secs());
    controller.was_allowed = allowed;
    controller.movement = if allowed && controller.gestures.armed {
        Vec2::new(pad.left.x, -pad.left.y)
    } else {
        Vec2::ZERO
    };
    controller.aim = (allowed && controller.gestures.armed && pad.right != Vec2::ZERO)
        .then(|| Vec2::new(pad.right.x, -pad.right.y).normalize_or_zero());
    controller.aiming_slot = intent.slot;
    controller.cast = intent.cast.map(|slot| MobileCastIntent {
        slot,
        aim: controller.aim,
    });
    controller.upgrade = intent.upgrade;
    controller.attack_held = intent.attack;
    controller.lock_pressed = intent.lock;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pad(buttons: u32) -> PadSnapshot {
        PadSnapshot {
            buttons,
            ..default()
        }
    }
    fn armed() -> GestureState {
        let mut state = GestureState::default();
        state.step(pad(0), true, 0.016);
        state
    }
    #[test]
    fn radial_filter_removes_drift_preserves_diagonal_and_rejects_nan() {
        assert_eq!(radial_dead_zone(Vec2::splat(0.1)), Vec2::ZERO);
        assert_eq!(radial_dead_zone(Vec2::new(f32::NAN, 0.0)), Vec2::ZERO);
        let v = radial_dead_zone(Vec2::ONE);
        assert!((v.length() - 1.0).abs() < 1e-6);
        let v = radial_dead_zone(Vec2::X * 0.59);
        assert!((v.x - 0.5).abs() < 1e-5);
    }
    #[test]
    fn skills_fire_once_on_release_and_cancel_needs_neutral() {
        let mut s = armed();
        assert_eq!(s.step(pad(L1), true, 0.016).slot, Some(0));
        assert_eq!(s.step(pad(0), true, 0.016).cast, Some(0));
        assert!(s.step(pad(0), true, 0.016).cast.is_none());
        s.step(pad(L1), true, 0.016);
        s.step(pad(L1 | EAST), true, 0.016);
        assert!(s.step(pad(0), true, 0.016).cast.is_none());
    }
    #[test]
    fn ultimate_chord_suppresses_third_skill_and_attack_in_both_orders() {
        for first in [L2, R2, CHORD] {
            let mut s = armed();
            let i = s.step(pad(first), true, 0.016);
            assert!(!i.attack);
            assert_eq!(s.step(pad(CHORD), true, 0.016).slot, Some(3));
            let i = s.step(pad(R2), true, 0.016);
            assert_eq!(i.cast, Some(3));
            assert!(!i.attack);
            assert!(!s.step(pad(R2), true, 0.1).attack);
            assert!(s.step(pad(0), true, 0.016).cast.is_none());
        }
    }
    #[test]
    fn modal_death_disconnect_never_release_cast_or_resume_held_attack() {
        let mut s = armed();
        s.step(pad(L1), true, 0.02);
        s.step(pad(L1), false, 0.02);
        assert!(s.step(pad(0), true, 0.02).cast.is_none());
        assert!(!s.step(pad(R2), true, 0.1).attack);
        assert!(s.step(pad(R2), true, 0.1).attack);
        s.step(pad(R2), false, 0.1);
        assert!(!s.step(pad(R2), true, 0.1).attack);
        s.step(pad(0), true, 0.1);
        assert!(!s.step(pad(R2), true, 0.1).attack);
        assert!(s.step(pad(R2), true, 0.1).attack);
    }
    #[test]
    fn upgrade_modifier_never_casts() {
        let mut s = armed();
        s.step(pad(NORTH | L1), true, 0.016);
        let i = s.step(pad(NORTH), true, 0.016);
        assert_eq!(i.upgrade, Some(0));
        assert!(i.cast.is_none());
    }
    #[test]
    fn connected_idle_pad_does_not_steal_mouse_and_handoff_is_explicit() {
        let mut p = GamepadControls::default();
        p.sample(Some(pad(0)), false, true);
        assert!(!p.active);
        p.sample(Some(pad(R2)), false, true);
        assert!(p.active);
        p.sample(Some(pad(R2)), true, true);
        assert!(!p.active);
        assert!(p.cancel_actions);
        p.sample(Some(pad(R2)), false, true);
        assert!(!p.active);
        p.sample(Some(pad(0)), false, true);
        assert!(!p.active);
        p.sample(Some(pad(L1)), false, true);
        assert!(p.active);
        p.sample(None, false, true);
        assert!(!p.active);
        assert!(p.cancel_actions);
    }
}

#[cfg(test)]
#[path = "gamepad_controls_tests.rs"]
mod integration_tests;
