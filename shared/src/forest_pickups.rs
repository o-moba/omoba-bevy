//! Bounded healing butterfly locations and their authoritative replicated state.
use crate::navigation::Point;
use serde::{Deserialize, Serialize};

pub const FOREST_PICKUP_COUNT: usize = 6;
pub const PICKUP_RADIUS: f32 = 1.5;
pub const HEAL_FRACTION: f32 = 0.05;
pub const RESPAWN_SECS: f32 = 30.0;

/// Authored forest clearing edges on the shared fixed Verdant geometry.
/// Adjacent entries mirror each other; stable ids are their index plus one.
pub fn pickup_layout() -> [Point; FOREST_PICKUP_COUNT] {
    [
        [-66.0, 45.0],
        [66.0, -45.0],
        [-65.0, -40.0],
        [65.0, 40.0],
        [-39.0, -71.0],
        [39.0, 71.0],
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForestPickupState {
    pub id: u64,
    pub position: Point,
    pub available: bool,
    /// Latest receipt survives packet loss. Consumers seed their high-water
    /// marks on initial snapshot and deduplicate within the snapshot epoch/round.
    pub collection_sequence: u64,
    pub last_collector_id: Option<u64>,
    pub healed_amount: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::world_navigation;

    #[test]
    fn forest_pickups_are_symmetric_finite_reachable_clearings() {
        let nav = world_navigation();
        let anchors = pickup_layout();
        for pair in anchors.chunks_exact(2) {
            assert_eq!(pair[0], pair[1].map(|value| -value));
        }
        let map = crate::map::geometry();
        let camps = crate::jungle::camp_layout(map.bounds.max[0] - map.bounds.min[0]);
        for anchor in anchors {
            assert!(anchor.into_iter().all(f32::is_finite));
            assert!(nav.point_clear_with_radius(anchor, PICKUP_RADIUS + 0.65));
            assert!(
                camps
                    .iter()
                    .all(|(camp, _)| (anchor[0] - camp[0]).hypot(anchor[1] - camp[1]) > 7.0)
            );
            for base in [map.home, map.away] {
                let route = nav.plan_route(base, anchor, &[]).expect("pickup route");
                let end = route.last().copied().unwrap_or(base);
                assert!((end[0] - anchor[0]).hypot(end[1] - anchor[1]) < 0.1);
            }
        }
    }
}
