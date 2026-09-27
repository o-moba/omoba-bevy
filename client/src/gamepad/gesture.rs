//! Button gestures on a normalized snapshot; no ECS, no side effects.
//!
//! - Hold L1 / R1 / L2 to aim skill 1 / 2 / 3, release to cast.
//! - Hold L2+R2 (either order) to aim the ultimate, release either to cast.
//!   The chord latches until both triggers are up, so the leftover trigger
//!   neither casts skill 3 nor starts a basic attack.
//! - Hold North while pressing a skill binding to upgrade it instead; an
//!   upgrade never casts.
//! - Hold R2 alone for the basic attack, after an 80 ms grace that lets the
//!   second trigger of the chord arrive on an adjacent frame.
//! - R3 toggles the target lock (an edge).
//! - East, or any frame where gameplay is not allowed, drops everything; the
//!   gesture then rearms only after the pad has been neutral once.
// i18n-strict
use super::snapshot::{CHORD, EAST, L1, L2, NORTH, PadSnapshot, R1, R2, R3};

/// Grace before a lone R2 counts as the basic attack.
pub(crate) const ATTACK_CHORD_GRACE: f32 = 0.08;
const SKILL_BUTTONS: [u32; 3] = [L1, R1, L2];
const ULTIMATE: usize = 3;

#[derive(Default, Debug, Clone)]
pub(crate) struct GestureState {
    pub(crate) armed: bool,
    previous: u32,
    slot: Option<usize>,
    upgrading: bool,
    chord_latched: bool,
    attack_seconds: f32,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GestureIntent {
    /// The skill being aimed (held).
    pub slot: Option<usize>,
    /// Released this frame: cast it.
    pub cast: Option<usize>,
    /// Released this frame with the upgrade modifier: spend a point.
    pub upgrade: Option<usize>,
    /// The basic attack is held.
    pub attack: bool,
    /// R3 went down this frame.
    pub lock: bool,
}

impl GestureState {
    pub(crate) fn step(&mut self, pad: PadSnapshot, allowed: bool, dt: f32) -> GestureIntent {
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
            self.slot = Some(ULTIMATE);
            self.upgrading = pad.buttons & NORTH != 0;
            self.chord_latched = true;
        } else if self.slot.is_none() && !self.chord_latched {
            if let Some(slot) = SKILL_BUTTONS.iter().position(|key| pressed & key != 0) {
                self.slot = Some(slot);
                self.upgrading = pad.buttons & NORTH != 0;
            }
        }
        if let Some(slot) = self.slot {
            let held = if slot == ULTIMATE {
                pad.buttons & CHORD == CHORD
            } else {
                pad.buttons & SKILL_BUTTONS[slot] != 0
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
        // Once either ultimate trigger is released, consume the other until
        // both are up; otherwise releasing an ultimate could basic-attack.
        if pad.buttons & CHORD == 0 {
            self.chord_latched = false;
        }
        // Time counts only from the frame after the press: a hitched first
        // frame's delta happened before the trigger went down.
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

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
    fn skills_fire_once_on_release_and_cancel_needs_neutral() {
        let mut s = armed();
        for (slot, key) in SKILL_BUTTONS.iter().enumerate() {
            assert_eq!(s.step(pad(*key), true, 0.016).slot, Some(slot));
            assert_eq!(s.step(pad(0), true, 0.016).cast, Some(slot));
            assert!(s.step(pad(0), true, 0.016).cast.is_none());
        }
        s.step(pad(L1), true, 0.016);
        s.step(pad(L1 | EAST), true, 0.016);
        assert!(s.step(pad(0), true, 0.016).cast.is_none());
    }

    #[test]
    fn a_held_pad_does_not_arm_until_neutral() {
        let mut s = GestureState::default();
        assert_eq!(s.step(pad(L1), true, 0.016), GestureIntent::default());
        assert!(s.step(pad(0), true, 0.016).cast.is_none(), "arms, no cast");
        assert!(s.armed);
        let tilted = PadSnapshot {
            left: Vec2::X,
            ..pad(0)
        };
        let mut s = GestureState::default();
        s.step(tilted, true, 0.016);
        assert!(!s.armed, "a tilted stick is not neutral");
    }

    #[test]
    fn ultimate_chord_suppresses_third_skill_and_attack_in_both_orders() {
        for first in [L2, R2, CHORD] {
            let mut s = armed();
            let i = s.step(pad(first), true, 0.016);
            assert!(!i.attack);
            assert_eq!(s.step(pad(CHORD), true, 0.016).slot, Some(3));
            let i = s.step(pad(R2), true, 0.016);
            assert_eq!(i.cast, Some(3), "released {first:#b} first");
            assert!(!i.attack);
            assert!(!s.step(pad(R2), true, 0.1).attack, "leftover trigger");
            assert!(s.step(pad(0), true, 0.016).cast.is_none());
        }
    }

    #[test]
    fn basic_attack_waits_for_the_chord_grace() {
        let mut s = armed();
        assert!(!s.step(pad(R2), true, 0.1).attack, "the press frame");
        assert!(!s.step(pad(R2), true, 0.05).attack, "inside 80 ms");
        assert!(s.step(pad(R2), true, 0.05).attack);
        assert!(
            !s.step(pad(R2 | NORTH), true, 0.05).attack,
            "upgrade modifier"
        );
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
    fn upgrade_modifier_never_casts_and_r3_is_an_edge() {
        let mut s = armed();
        s.step(pad(NORTH | L1), true, 0.016);
        let i = s.step(pad(NORTH), true, 0.016);
        assert_eq!(i.upgrade, Some(0));
        assert!(i.cast.is_none());
        s.step(pad(0), true, 0.016);
        s.step(pad(NORTH | CHORD), true, 0.016);
        let i = s.step(pad(NORTH | R2), true, 0.016);
        assert_eq!(i.upgrade, Some(3), "ultimate upgrade");
        assert!(i.cast.is_none() && !i.attack);
        let mut s = armed();
        assert!(s.step(pad(R3), true, 0.016).lock);
        assert!(!s.step(pad(R3), true, 0.016).lock);
    }
}
