//! Who owns input: the controller or the keyboard, mouse and touch.
//!
//! A connected but idle controller never takes over. A new button press or a
//! deliberate stick push does; any keyboard, mouse or touch press gives input
//! back, as do a disconnect, a replaced device and losing window focus. The
//! activation baseline remembers where the sticks rested when the pad last
//! lost ownership, so a drifting or still-tilted stick cannot steal input
//! again; a stick that returns to centre resets its baseline.
// i18n-strict
use bevy::prelude::*;

use super::snapshot::PadSnapshot;

/// Stick travel (beyond the baseline) that counts as a deliberate push.
const DELIBERATE: f32 = 0.12;
/// Left-stick deflection that counts as one navigation step in menus.
pub(crate) const NAV_THRESHOLD: f32 = 0.65;

#[derive(Default, Debug, Clone)]
pub(crate) struct Ownership {
    pub active: bool,
    last: Option<PadSnapshot>,
    baseline: Option<PadSnapshot>,
}

/// What one sampled frame means for the rest of the controller code.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub(crate) struct OwnershipFrame {
    /// The focused snapshot (`None` when disconnected or unfocused).
    pub pad: Option<PadSnapshot>,
    /// Held and queued actions must be dropped now (handoff either way,
    /// disconnect, replacement, focus loss while active).
    pub cancel: bool,
    /// Gesture state must restart and wait for neutral controls.
    pub reset: bool,
    /// A different device (or none) than last frame.
    pub new_device: bool,
    /// Buttons that went down this frame; 0 unless the pad owns input.
    pub pressed: u32,
    /// Left stick crossed [`NAV_THRESHOLD`] this frame: up, down, left, right.
    pub stick_nav: [bool; 4],
}

impl Ownership {
    pub(crate) fn step(
        &mut self,
        pad: Option<PadSnapshot>,
        other_input: bool,
        focused: bool,
    ) -> OwnershipFrame {
        let mut frame = OwnershipFrame::default();
        if self.last.map(|p| p.identity) != pad.map(|p| p.identity) {
            frame.cancel = self.active;
            frame.reset = true;
            frame.new_device = true;
            self.last = pad;
            self.baseline = pad;
        }
        let Some(pad) = pad.filter(|_| focused) else {
            frame.cancel |= self.active;
            frame.reset = true;
            self.active = false;
            self.last = None;
            return frame;
        };
        frame.pad = Some(pad);
        let old = self.last.unwrap_or(pad);
        let pressed = pad.buttons & !old.buttons;
        let baseline = self.baseline.unwrap_or_default();
        if other_input {
            frame.cancel |= self.active;
            frame.reset = true;
            self.active = false;
            self.baseline = Some(pad);
        } else if pressed != 0
            || deliberate_stick(pad.left, baseline.left)
            || deliberate_stick(pad.right, baseline.right)
        {
            // Taking over cancels whatever the mouse or touch had queued.
            frame.cancel |= !self.active;
            self.active = true;
        }
        if self.active {
            self.baseline = Some(pad);
            frame.pressed = pressed;
            let crossed = |now: f32, before: f32| now > NAV_THRESHOLD && before <= NAV_THRESHOLD;
            frame.stick_nav = [
                crossed(pad.left.y, old.left.y),
                crossed(-pad.left.y, -old.left.y),
                crossed(-pad.left.x, -old.left.x),
                crossed(pad.left.x, old.left.x),
            ];
        } else {
            let baseline = self.baseline.get_or_insert(pad);
            if pad.left == Vec2::ZERO {
                baseline.left = Vec2::ZERO;
            }
            if pad.right == Vec2::ZERO {
                baseline.right = Vec2::ZERO;
            }
        }
        self.last = Some(pad);
        frame
    }

    /// The device sampled last (for keeping the same desktop pad).
    #[cfg(not(target_os = "ios"))]
    pub(crate) fn identity(&self) -> Option<u64> {
        self.last.map(|pad| pad.identity)
    }
}

/// A push well past where the stick rested, or a clear change of direction.
pub(crate) fn deliberate_stick(now: Vec2, baseline: Vec2) -> bool {
    now.length() > DELIBERATE
        && (now.length() > baseline.length() + DELIBERATE
            || (baseline.length() > DELIBERATE && now.normalize().dot(baseline.normalize()) < 0.9))
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::{L1, R2};
    use super::*;

    fn pad(buttons: u32) -> Option<PadSnapshot> {
        Some(PadSnapshot {
            identity: 7,
            buttons,
            ..default()
        })
    }

    #[test]
    fn connected_idle_pad_does_not_steal_mouse_and_handoff_is_explicit() {
        let mut o = Ownership::default();
        assert!(!o.step(pad(0), false, true).cancel);
        assert!(!o.active, "connected and idle");
        let frame = o.step(pad(R2), false, true);
        assert!(
            o.active && frame.cancel,
            "takeover drops the mouse's orders"
        );
        assert_eq!(frame.pressed, R2);
        let frame = o.step(pad(R2), true, true);
        assert!(
            !o.active && frame.cancel && frame.reset,
            "mouse takes it back"
        );
        assert!(!o.step(pad(R2), false, true).cancel);
        assert!(!o.active, "a still-held trigger is not a new press");
        o.step(pad(0), false, true);
        assert!(!o.active);
        o.step(pad(L1), false, true);
        assert!(o.active);
        let frame = o.step(None, false, true);
        assert!(!o.active && frame.cancel && frame.new_device, "disconnect");
        o.step(pad(0), false, true);
        o.step(pad(L1), false, true);
        assert!(o.active);
        let frame = o.step(pad(L1), false, false);
        assert!(
            !o.active && frame.cancel && frame.pad.is_none(),
            "focus loss"
        );
        let replaced = Some(PadSnapshot {
            identity: 8,
            buttons: L1,
            ..default()
        });
        let frame = o.step(replaced, false, true);
        assert!(
            frame.new_device && frame.reset && !o.active,
            "held on arrival"
        );
    }

    #[test]
    fn smooth_deliberate_stick_motion_activates_but_resting_drift_does_not() {
        let mut o = Ownership::default();
        o.step(pad(0), false, true);
        for step in 1..=20 {
            o.step(
                Some(PadSnapshot {
                    identity: 7,
                    left: Vec2::X * (step as f32 * 0.05),
                    ..default()
                }),
                false,
                true,
            );
        }
        assert!(o.active, "a full tilt activates at any frame rate");
        // Hand back with the stick still tilted: the tilt is the new baseline.
        let tilted = Some(PadSnapshot {
            identity: 7,
            left: Vec2::X,
            ..default()
        });
        o.step(tilted, true, true);
        for _ in 0..10 {
            o.step(tilted, false, true);
        }
        assert!(!o.active, "a resting tilt never steals input back");
        o.step(pad(0), false, true);
        o.step(tilted, false, true);
        assert!(o.active, "centre, then push again");
    }

    #[test]
    fn left_stick_navigation_is_an_edge_past_the_threshold() {
        let mut o = Ownership::default();
        o.step(pad(0), false, true);
        o.step(pad(L1), false, true);
        let at = |y: f32| {
            Some(PadSnapshot {
                identity: 7,
                buttons: L1,
                left: Vec2::Y * y,
                ..default()
            })
        };
        assert_eq!(o.step(at(0.5), false, true).stick_nav, [false; 4]);
        assert_eq!(
            o.step(at(0.7), false, true).stick_nav,
            [true, false, false, false]
        );
        assert_eq!(o.step(at(0.9), false, true).stick_nav, [false; 4], "held");
        assert_eq!(
            o.step(at(-0.8), false, true).stick_nav,
            [false, true, false, false]
        );
    }
}
