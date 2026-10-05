//! Deterministic team sight and walkable brush geometry shared by simulation and 3D.
use serde::{Deserialize, Serialize};

pub const HERO_SIGHT_RADIUS: f32 = 32.0;
pub const MINION_SIGHT_RADIUS: f32 = 22.0;
pub const TOWER_SIGHT_RADIUS: f32 = 28.0;
pub const BASE_SIGHT_RADIUS: f32 = 34.0;
/// Temporary team sight carried by a flying impact rocket.
pub const ROCKET_SIGHT_RADIUS: f32 = 8.0;
pub const HOSTILE_REVEAL_SECS: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VisionSource {
    pub position: [f32; 2],
    pub radius: f32,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamVision {
    pub sources: Vec<VisionSource>,
    pub local_brush: Option<u16>,
    pub local_hidden: bool,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushZone {
    pub id: u16,
    pub center: [f32; 2],
    pub radius: f32,
}

pub fn brush_layout() -> &'static [BrushZone] {
    const BRUSH: [BrushZone; 16] = [
        BrushZone {
            id: 1,
            center: [-22.0, -8.0],
            radius: 3.0,
        },
        BrushZone {
            id: 2,
            center: [22.0, 8.0],
            radius: 3.0,
        },
        BrushZone {
            id: 3,
            center: [-42.0, -25.0],
            radius: 3.0,
        },
        BrushZone {
            id: 4,
            center: [42.0, 25.0],
            radius: 3.0,
        },
        BrushZone {
            id: 5,
            center: [-66.0, 45.0],
            radius: 2.0,
        },
        BrushZone {
            id: 6,
            center: [66.0, -45.0],
            radius: 2.0,
        },
        BrushZone {
            id: 7,
            center: [-65.0, -40.0],
            radius: 2.0,
        },
        BrushZone {
            id: 8,
            center: [65.0, 40.0],
            radius: 2.0,
        },
        BrushZone {
            id: 9,
            center: [-39.0, -71.0],
            radius: 2.0,
        },
        BrushZone {
            id: 10,
            center: [39.0, 71.0],
            radius: 2.0,
        },
        // Lane pockets touch the road while leaving its marching centerline
        // clear. Keep the older jungle IDs stable for saved/QA brush fixtures.
        BrushZone {
            id: 11,
            center: [-15.0, -5.0],
            radius: 2.5,
        },
        BrushZone {
            id: 12,
            center: [15.0, 5.0],
            radius: 2.5,
        },
        BrushZone {
            id: 13,
            center: [-93.15, 66.0],
            radius: 2.5,
        },
        BrushZone {
            id: 14,
            center: [93.15, -66.0],
            radius: 2.5,
        },
        BrushZone {
            id: 15,
            center: [-66.0, 93.15],
            radius: 2.5,
        },
        BrushZone {
            id: 16,
            center: [66.0, -93.15],
            radius: 2.5,
        },
    ];
    static SCALED: std::sync::OnceLock<[BrushZone; 16]> = std::sync::OnceLock::new();
    SCALED.get_or_init(|| {
        BRUSH.map(|zone| BrushZone {
            center: zone
                .center
                .map(|coordinate| coordinate * crate::map::WORLD_SCALE),
            radius: zone.radius * crate::map::WORLD_SCALE,
            ..zone
        })
    })
}

pub fn brush_at(point: [f32; 2]) -> Option<u16> {
    brush_layout()
        .iter()
        .find(|zone| distance_squared(point, zone.center) <= zone.radius.powi(2))
        .map(|zone| zone.id)
}
fn distance_squared(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}
pub fn point_visible(sources: &[VisionSource], point: [f32; 2], conceal_in_brush: bool) -> bool {
    let brush = conceal_in_brush.then(|| brush_at(point)).flatten();
    sources.iter().any(|source| {
        source.radius.is_finite()
            && source.radius >= 0.0
            && distance_squared(point, source.position) <= source.radius.powi(2)
            && brush.is_none_or(|id| brush_at(source.position) == Some(id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn radial_boundaries_union_and_invalid_points() {
        let sources = [
            VisionSource {
                position: [0.0, 0.0],
                radius: 32.0,
            },
            VisionSource {
                position: [80.0, 0.0],
                radius: 22.0,
            },
        ];
        assert!(point_visible(&sources, [32.0, 0.0], false));
        assert!(!point_visible(&sources, [32.01, 0.0], false));
        assert!(point_visible(&sources, [60.0, 0.0], false));
        assert!(!point_visible(&sources, [f32::NAN, 0.0], false));
        assert!(!point_visible(&[], [0.0, 0.0], false));
    }
    #[test]
    fn brush_enter_exit_same_and_different_zone() {
        let zone = brush_layout()[0];
        let outside = [VisionSource {
            position: [0.0, 0.0],
            radius: 32.0,
        }];
        assert_eq!(brush_at(zone.center), Some(zone.id));
        assert!(!point_visible(&outside, zone.center, true));
        assert!(point_visible(&outside, zone.center, false));
        assert!(point_visible(
            &outside,
            [zone.center[0], zone.center[1] + 3.01],
            true
        ));
        let same = [VisionSource {
            position: zone.center,
            radius: 64.0,
        }];
        assert!(point_visible(&same, zone.center, true));
        assert!(!point_visible(&same, brush_layout()[1].center, true));
    }
    #[test]
    fn brush_is_symmetric_walkable_and_reachable() {
        let nav = crate::navigation::world_navigation();
        for pair in brush_layout().chunks_exact(2) {
            assert_eq!(pair[0].center, pair[1].center.map(|v| -v));
        }
        for zone in &brush_layout()[..4] {
            // Mid road is x=z (width12), river is x=-z (width18).
            // Keep the full gameplay footprint on the walkable meadow verge.
            assert!(
                (zone.center[0] - zone.center[1]).abs() / 2.0_f32.sqrt()
                    > crate::map::LANE_WIDTH * 0.5 + zone.radius
            );
            assert!((zone.center[0] + zone.center[1]).abs() / 2.0_f32.sqrt() > 9.0 + zone.radius);
        }
        for zone in brush_layout() {
            assert!(
                nav.point_clear_with_radius(zone.center, zone.radius + 0.65),
                "brush {} {:?}",
                zone.id,
                zone.center
            );
            for base in [crate::map::geometry().home, crate::map::geometry().away] {
                let route = nav.plan_route(base, zone.center, &[]).expect("brush route");
                assert!(distance_squared(*route.last().unwrap(), zone.center) < 0.01);
            }
        }
    }

    #[test]
    fn lane_brush_has_open_approaches_and_clears_default_structure_footprints() {
        use crate::map::{Lane, ResolvedMap};
        use crate::navigation::{Disc, HERO_RADIUS, PLANNING_CLEARANCE};

        let nav = crate::navigation::world_navigation();
        let map = ResolvedMap::default();
        let structures: Vec<_> = map
            .structures
            .iter()
            .map(|structure| Disc {
                center: structure.position,
                radius: if structure.lane.is_some() {
                    crate::TOWER_TARGET_RADIUS
                } else {
                    3.2
                },
            })
            .collect();
        let lanes = [Lane::Mid, Lane::Top, Lane::Bot];
        let mut covered = [false; 3];
        for zone in brush_layout().iter().filter(|zone| zone.id >= 11) {
            let (lane_index, approach, lane_distance) = lanes
                .iter()
                .enumerate()
                .flat_map(|(index, &lane)| {
                    crate::map::minion_lane_points(lane)
                        .windows(2)
                        .map(|segment| {
                            let [a, b] = [segment[0], segment[1]];
                            let delta = [b[0] - a[0], b[1] - a[1]];
                            let t = (((zone.center[0] - a[0]) * delta[0]
                                + (zone.center[1] - a[1]) * delta[1])
                                / distance_squared(a, b))
                            .clamp(0.0, 1.0);
                            let point = [a[0] + t * delta[0], a[1] + t * delta[1]];
                            (index, point, distance_squared(point, zone.center).sqrt())
                        })
                        .collect::<Vec<_>>()
                })
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .unwrap();
            covered[lane_index] = true;
            assert!(
                lane_distance < crate::map::LANE_WIDTH * 0.5 + zone.radius,
                "brush {} must touch a lane corridor",
                zone.id
            );
            assert!(
                lane_distance > zone.radius + HERO_RADIUS + PLANNING_CLEARANCE,
                "brush {} must leave the lane centerline unconcealed",
                zone.id
            );
            assert!(nav.segment_clear_with_discs(approach, zone.center, &structures));
            assert_eq!(brush_at(approach), None);
            for structure in &map.structures {
                let footprint = if structure.lane.is_some() {
                    crate::TOWER_TARGET_RADIUS
                } else {
                    3.2
                };
                assert!(
                    distance_squared(zone.center, structure.position)
                        > (zone.radius + footprint + HERO_RADIUS).powi(2),
                    "brush {} overlaps {} collision",
                    zone.id,
                    structure.key
                );
                assert!(
                    !point_visible(
                        &[VisionSource {
                            position: structure.position,
                            radius: TOWER_SIGHT_RADIUS,
                        }],
                        zone.center,
                        true
                    ),
                    "external tower sight must respect brush concealment"
                );
            }
            for base in [crate::map::geometry().home, crate::map::geometry().away] {
                let spawn_scale = 1.0 - 7.0 / distance_squared(base, [0.0; 2]).sqrt();
                let spawn = base.map(|coordinate| coordinate * spawn_scale);
                let route = nav
                    .plan_route(spawn, zone.center, &structures)
                    .expect("lane brush remains reachable with every structure alive");
                assert!(distance_squared(*route.last().unwrap(), zone.center) < 0.01);
            }
        }
        assert!(covered.into_iter().all(|lane| lane));
    }
}
