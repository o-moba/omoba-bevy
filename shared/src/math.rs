//! Facing conventions shared by simulation and presentation.
//!
//! Two model conventions exist and both are deliberate: hero models are
//! authored looking along their local -Z (Bevy's forward), minion and neutral
//! models look along +Z. Every yaw is rendered as `Quat::from_rotation_y(yaw)`,
//! so the helper picked here decides whether a unit runs forwards or backwards.

/// Yaw that makes a hero model look along the ground direction `(dx, dz)`.
pub fn hero_yaw_towards(dx: f32, dz: f32) -> f32 {
    (-dx).atan2(-dz)
}

/// Yaw that makes a minion or neutral model look along `(dx, dz)`.
pub fn unit_yaw_towards(dx: f32, dz: f32) -> f32 {
    dx.atan2(dz)
}

/// Ground direction a hero model faces at `yaw` (its local -Z after rotation).
pub fn hero_forward(yaw: f32) -> [f32; 2] {
    [-yaw.sin(), -yaw.cos()]
}

/// Ground direction a minion or neutral model faces at `yaw` (its local +Z).
pub fn unit_forward(yaw: f32) -> [f32; 2] {
    [yaw.sin(), yaw.cos()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRECTIONS: [(f32, f32); 5] = [
        (1.0, 0.0),
        (0.0, 1.0),
        (-1.0, 0.0),
        (0.0, -1.0),
        (0.6, -0.8),
    ];

    #[test]
    fn heroes_and_units_face_their_movement_under_their_own_convention() {
        for (dx, dz) in DIRECTIONS {
            let hero = hero_forward(hero_yaw_towards(dx, dz));
            assert!(
                (hero[0] - dx).abs() < 1e-5 && (hero[1] - dz).abs() < 1e-5,
                "hero {dx},{dz}"
            );
            let unit = unit_forward(unit_yaw_towards(dx, dz));
            assert!(
                (unit[0] - dx).abs() < 1e-5 && (unit[1] - dz).abs() < 1e-5,
                "unit {dx},{dz}"
            );
            // Mixing the conventions is exactly a half turn: the backwards-runner bug.
            let mixed = hero_forward(unit_yaw_towards(dx, dz));
            assert!((mixed[0] + dx).abs() < 1e-5 && (mixed[1] + dz).abs() < 1e-5);
        }
    }
}
