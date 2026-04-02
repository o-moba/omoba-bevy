//! Central keybinding labels for combat HUD and help copy.
//! Keep cast handling in `combat.rs` in sync with `SKILL_CAST_KEYS`.

use bevy::prelude::KeyCode;

/// Keys that trigger a cast toward the current target (same server action until skill slots exist).
pub const SKILL_CAST_KEYS: [KeyCode; 4] = [
    KeyCode::KeyQ,
    KeyCode::KeyW,
    KeyCode::KeyE,
    KeyCode::KeyR,
];

/// Reserved for future skill-upgrade UI; keep in sync with `upgrade_key_display()`.
#[allow(dead_code)]
pub const SKILL_UPGRADE_KEY: KeyCode = KeyCode::KeyU;

pub const HELP_TOGGLE_KEY: KeyCode = KeyCode::F1;

/// Human-readable primary labels for the four cast slots (matches `SKILL_CAST_KEYS` order).
pub const SKILL_SLOT_KEY_LABELS: [&str; 4] = ["Q", "W", "E", "R"];

/// Compact list for help/HUD copy; stays aligned with [`SKILL_SLOT_KEY_LABELS`].
pub fn skill_keys_display() -> String {
    SKILL_SLOT_KEY_LABELS.join(" / ")
}

/// Four bracketed slot labels for at-a-glance HUD (e.g. `[Q]  [W]  [E]  [R]`).
pub fn skill_slots_bracket_line() -> String {
    SKILL_SLOT_KEY_LABELS
        .iter()
        .map(|k| format!("[{k}]"))
        .collect::<Vec<_>>()
        .join("  ")
}

pub fn upgrade_key_display() -> &'static str {
    "U"
}

pub fn help_key_display() -> &'static str {
    "F1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cast_keys_and_slot_labels_same_length() {
        assert_eq!(SKILL_CAST_KEYS.len(), SKILL_SLOT_KEY_LABELS.len());
    }

    #[test]
    fn skill_keys_display_covers_every_slot_label() {
        let s = skill_keys_display();
        for label in SKILL_SLOT_KEY_LABELS {
            assert!(
                s.contains(label),
                "expected {s:?} to contain slot label {label:?}"
            );
        }
    }

    #[test]
    fn skill_slots_bracket_line_has_four_brackets() {
        let s = skill_slots_bracket_line();
        assert_eq!(s.matches('[').count(), 4);
        assert_eq!(s.matches(']').count(), 4);
        for label in SKILL_SLOT_KEY_LABELS {
            assert!(s.contains(&format!("[{label}]")));
        }
    }
}
