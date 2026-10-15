//! Compiled cosmetic slots. These never enter navigation or combat state.
pub const SLOT_ID: &str = "opposite-dragon";
pub const MODEL_ENVELOPE_RADIUS: f32 = 2.0;
/// Beside the Wendigo pit, more than its 18 m leash radius from the objective.
/// Keep the slot code-owned: a Registry response cannot move scenery.
pub fn slot_position() -> [f32; 3] {
    let bounds = crate::map::geometry().bounds;
    let width = bounds.max[0] - bounds.min[0];
    [width * 0.22 - 22., 0.05, -width * 0.34 + 6.]
}
#[cfg(test)]
mod tests {
    #[test]
    fn decorative_slot_is_inside_map_and_outside_objective_leash() {
        let p = super::slot_position();
        let g = crate::map::geometry();
        let width = g.bounds.max[0] - g.bounds.min[0];
        assert!(p[0] > g.bounds.min[0] && p[0] < g.bounds.max[0]);
        assert!(p[2] > g.bounds.min[1] && p[2] < g.bounds.max[1]);
        let dist = ((p[0] - width * 0.22).powi(2) + (p[2] + width * 0.34).powi(2)).sqrt();
        assert!(dist - super::MODEL_ENVELOPE_RADIUS > 18.);
    }
}
