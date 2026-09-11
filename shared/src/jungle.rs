//! Ordinary jungle camp anchors shared by simulation and map presentation.
//! Each pair has the same creature and rewards under a half-turn of the map.
use crate::navigation::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JungleCampKind {
    Skirmisher,
    Bruiser,
    Spitter,
}

/// Stable order is also the six minimap camp slots. Coordinates use full map
/// extent. Bruisers sit slightly toward the base-side clearing to leave more
/// than ten metres of clearance in both halves of the shipped Verdant forest.
pub fn camp_layout(map_size: f32) -> [(Point, JungleCampKind); 6] {
    use JungleCampKind::*;
    [
        ([-0.34 * map_size, 0.22 * map_size], Skirmisher),
        ([0.34 * map_size, -0.22 * map_size], Skirmisher),
        ([-0.30 * map_size, -0.227 * map_size], Bruiser),
        ([0.30 * map_size, 0.227 * map_size], Bruiser),
        ([-0.22 * map_size, -0.34 * map_size], Spitter),
        ([0.22 * map_size, 0.34 * map_size], Spitter),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::world_navigation;

    #[test]
    fn mirrored_camps_have_equal_types_and_reachable_clear_fighting_space() {
        let nav = world_navigation();
        let bounds = nav.bounds();
        let camps = camp_layout(bounds.max[0] - bounds.min[0]);
        for pair in camps.chunks_exact(2) {
            assert_eq!(pair[0].1, pair[1].1);
            assert_eq!(pair[0].0, pair[1].0.map(|v| -v));
        }
        for (point, _) in camps {
            assert!(nav.point_clear(point));
            // A 3m fighting ring is unobstructed, with physical hero clearance.
            for step in 0..16 {
                let angle = step as f32 * std::f32::consts::TAU / 16.0;
                let rim = [point[0] + 3.0 * angle.cos(), point[1] + 3.0 * angle.sin()];
                assert!(nav.segment_clear(point, rim), "blocked camp ring {point:?}");
            }
            for base in [[-74.599_77, -74.599_77], [74.599_77, 74.599_77]] {
                let route = nav.plan_route(base, point, &[]).expect("camp route");
                let end = route.last().copied().unwrap_or(base);
                assert!((end[0] - point[0]).hypot(end[1] - point[1]) < 0.1);
            }
        }
    }
}
