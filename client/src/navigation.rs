//! Bevy coordinate adapter for the shared forest/structure navigation module.

use bevy::prelude::*;
use shared::navigation::{Bounds, Disc, HERO_RADIUS, NavigationMap, world_navigation};

use crate::maps::MapLayout;
use crate::net::StructureKind;

/// Physical hero-center clearance used by the existing structure collision.
pub(crate) fn structure_collision_radius(kind: StructureKind) -> f32 {
    HERO_RADIUS
        + match kind {
            StructureKind::Tower => 1.3,
            StructureKind::BaseTower => 3.2,
        }
}

/// Plan once per order; waypoints exclude the start and retain its height.
/// Custom test arenas get their own empty static map instead of global trees.
pub(crate) fn plan_route(
    layout: &MapLayout,
    start: Vec3,
    destination: Vec3,
    structures: &[(Vec3, StructureKind)],
) -> Option<Vec<Vec3>> {
    if !start.is_finite()
        || !destination.is_finite()
        || !layout.min.is_finite()
        || !layout.max.is_finite()
        || structures.iter().any(|(position, _)| !position.is_finite())
    {
        return None;
    }
    let bounds = Bounds {
        min: layout.min.to_array(),
        max: layout.max.to_array(),
    };
    let world = world_navigation();
    let world_bounds = world.bounds();
    let custom;
    let map = if (0..2).all(|axis| {
        (bounds.min[axis] - world_bounds.min[axis]).abs() < 0.001
            && (bounds.max[axis] - world_bounds.max[axis]).abs() < 0.001
    }) {
        world
    } else {
        custom = NavigationMap::new(bounds, Vec::new()).ok()?;
        &custom
    };
    let dynamic: Vec<_> = structures
        .iter()
        .map(|&(position, kind)| Disc {
            center: [position.x, position.z],
            radius: structure_collision_radius(kind) - HERO_RADIUS,
        })
        .collect();
    map.plan_route([start.x, start.z], [destination.x, destination.z], &dynamic)
        .map(|route| {
            route
                .into_iter()
                .map(|point| Vec3::new(point[0], start.y, point[1]))
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::navigation::{MAX_DYNAMIC_DISCS as MAX_OBSTACLES, PLANNING_CLEARANCE as CLEARANCE};

    fn in_bounds(layout: &MapLayout, point: Vec2) -> bool {
        point.cmpge(layout.min).all() && point.cmple(layout.max).all()
    }

    fn arena(half_size: f32) -> MapLayout {
        MapLayout {
            min: Vec2::splat(-half_size),
            max: Vec2::splat(half_size),
            ..default()
        }
    }

    // Independently sample the traveled polyline densely against the physical
    // collision discs, including every corner and the final destination.
    fn assert_safe(
        layout: &MapLayout,
        start: Vec3,
        route: &[Vec3],
        structures: &[(Vec3, StructureKind)],
    ) {
        let mut previous = start;
        for &waypoint in route {
            assert_eq!(waypoint.y, start.y);
            let steps = (previous.distance(waypoint) / 0.01).ceil().max(1.0) as usize;
            for step in 0..=steps {
                let point = previous.lerp(waypoint, step as f32 / steps as f32);
                assert!(in_bounds(layout, Vec2::new(point.x, point.z)));
                for &(center, kind) in structures {
                    let distance = Vec2::new(point.x - center.x, point.z - center.z).length();
                    assert!(
                        distance >= structure_collision_radius(kind),
                        "segment hit {kind:?}"
                    );
                }
            }
            previous = waypoint;
        }
    }

    #[test]
    fn direct_route_preserves_height_clamps_destination_and_completes() {
        let layout = arena(10.0);
        let start = Vec3::new(-5.0, 0.7, 0.0);
        let route = plan_route(&layout, start, Vec3::new(30.0, 99.0, 4.0), &[]).unwrap();
        assert_eq!(route, [Vec3::new(10.0, 0.7, 4.0)]);
        assert_safe(&layout, start, &route, &[]);
        assert!(
            plan_route(&layout, route[0], route[0], &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn tower_and_base_routes_clear_every_segment() {
        for kind in [StructureKind::Tower, StructureKind::BaseTower] {
            let layout = arena(20.0);
            let structures = [(Vec3::ZERO, kind)];
            let start = Vec3::new(-10.0, 0.5, 0.0);
            let destination = Vec3::new(10.0, 0.5, 0.0);
            let route = plan_route(&layout, start, destination, &structures).unwrap();
            assert!(route.len() >= 2);
            assert_eq!(route.last(), Some(&destination));
            assert_safe(&layout, start, &route, &structures);
        }
    }

    #[test]
    fn real_map_base_detours_stay_short_on_both_sides_and_after_translation() {
        // Native minimap QA found a 150 m detour for this 14 m order. Preserve
        // the real map's two bases and six lane towers, including their large
        // absolute coordinates, instead of testing only a circle at the origin.
        let layout = arena(108.549_515);
        let structures = [
            (
                Vec3::new(-79.549_515, 0.0, -79.549_515),
                StructureKind::BaseTower,
            ),
            (
                Vec3::new(79.549_515, 0.0, 79.549_515),
                StructureKind::BaseTower,
            ),
            (
                Vec3::new(-96.549_515, 0.0, 29.509_907),
                StructureKind::Tower,
            ),
            (Vec3::new(4.490_105, 0.0, 96.549_515), StructureKind::Tower),
            (
                Vec3::new(-31.819_803, 0.0, -31.819_803),
                StructureKind::Tower,
            ),
            (Vec3::new(31.819_803, 0.0, 31.819_803), StructureKind::Tower),
            (
                Vec3::new(-4.490_089, 0.0, -96.549_515),
                StructureKind::Tower,
            ),
            (
                Vec3::new(96.549_515, 0.0, -29.509_897),
                StructureKind::Tower,
            ),
        ];
        for side in [-1.0, 1.0] {
            let start = Vec3::new(side * 74.599_77, 0.5, side * 74.599_77);
            let destination = Vec3::new(side * 84.499_26, 0.5, side * 84.499_31);
            let mut baseline_length: Option<f32> = None;
            for translation in [
                Vec3::ZERO,
                Vec3::new(-side * 79.549_515, 0.0, -side * 79.549_515),
                Vec3::new(150.25, 0.0, -150.25),
            ] {
                let translated_layout = MapLayout {
                    min: layout.min + translation.xz(),
                    max: layout.max + translation.xz(),
                    ..default()
                };
                let translated_structures: Vec<_> = structures
                    .iter()
                    .map(|&(position, kind)| (position + translation, kind))
                    .collect();
                let start = start + translation;
                let destination = destination + translation;
                let route = plan_route(
                    &translated_layout,
                    start,
                    destination,
                    &translated_structures,
                )
                .expect("base crossing must have a route");
                assert_safe(&translated_layout, start, &route, &translated_structures);
                assert_eq!(route.last(), Some(&destination));
                let mut previous = start;
                let length: f32 = route
                    .iter()
                    .map(|&point| {
                        let length = previous.distance(point);
                        previous = point;
                        length
                    })
                    .sum();
                // Two 7 m approaches around a 3.85 m padded disc have an
                // approximately 16.2 m shortest path; 20 m allows polygonal
                // approximation while rejecting a cross-map excursion.
                assert!(length < 20.0, "base route is {length} m: {route:?}");
                if let Some(baseline_length) = baseline_length {
                    assert!((length - baseline_length).abs() < 0.01);
                } else {
                    baseline_length = Some(length);
                }
            }
        }
    }

    #[test]
    fn multiple_obstacles_are_deterministic_and_traversable_without_oscillation() {
        let layout = arena(20.0);
        let mut structures = vec![
            (Vec3::new(-5.0, 0.0, 0.0), StructureKind::Tower),
            (Vec3::new(0.0, 0.0, -2.0), StructureKind::BaseTower),
            (Vec3::new(5.0, 0.0, 1.0), StructureKind::Tower),
        ];
        let start = Vec3::new(-15.0, 0.5, 0.0);
        let destination = Vec3::new(15.0, 0.5, 0.0);
        let route = plan_route(&layout, start, destination, &structures).unwrap();
        assert_safe(&layout, start, &route, &structures);
        structures.reverse();
        assert_eq!(
            plan_route(&layout, start, destination, &structures).unwrap(),
            route
        );
        let mut position = start;
        let mut steps = 0;
        for target in route {
            while position.distance(target) > 0.0001 {
                let before = position.distance(target);
                position = position.move_towards(target, 5.0 / 60.0);
                assert!(position.distance(target) < before);
                steps += 1;
                assert!(
                    steps < 1000,
                    "route failed to complete at normal movement speed"
                );
            }
        }
        assert!(position.distance(destination) < 0.0001);
    }

    #[test]
    fn obstacle_near_boundary_takes_the_open_side() {
        let layout = arena(10.0);
        let structures = [(Vec3::new(0.0, 0.0, 9.0), StructureKind::BaseTower)];
        let start = Vec3::new(-8.0, 0.5, 9.0);
        let destination = Vec3::new(8.0, 0.5, 9.0);
        let route = plan_route(&layout, start, destination, &structures).unwrap();
        assert!(route.iter().any(|point| point.z < 6.0));
        assert_safe(&layout, start, &route, &structures);
    }

    #[test]
    fn blocked_destination_projects_to_near_reachable_boundary() {
        let layout = arena(10.0);
        let structures = [(Vec3::ZERO, StructureKind::Tower)];
        let start = Vec3::new(-8.0, 0.5, 0.0);
        let route = plan_route(&layout, start, Vec3::ZERO, &structures).unwrap();
        let end = *route.last().unwrap();
        assert!(end.x < 0.0);
        assert!(end.z.abs() < 0.01);
        assert!((end.x.abs() - (1.8 + CLEARANCE)).abs() < 0.01);
        assert_safe(&layout, start, &route, &structures);
    }

    #[test]
    fn overlapping_discs_and_boundary_cannot_receive_an_unsafe_destination() {
        let layout = arena(10.0);
        let structures = [
            (Vec3::new(9.0, 0.0, 0.0), StructureKind::BaseTower),
            (Vec3::new(7.0, 0.0, 0.0), StructureKind::Tower),
        ];
        let start = Vec3::new(-8.0, 0.5, 0.0);
        let route = plan_route(&layout, start, Vec3::new(10.0, 0.0, 0.0), &structures).unwrap();
        assert_safe(&layout, start, &route, &structures);
        assert!(!route.is_empty());
    }

    #[test]
    fn overlapped_start_escapes_outward_before_detouring() {
        let layout = arena(10.0);
        let structures = [(Vec3::ZERO, StructureKind::Tower)];
        let start = Vec3::new(1.0, 0.5, 0.0);
        let destination = Vec3::new(-8.0, 0.5, 0.0);
        let route = plan_route(&layout, start, destination, &structures).unwrap();
        assert!(route[0].x >= start.x);
        assert!(route[0].xz().length() >= 1.8 + CLEARANCE);
        assert_safe(&layout, route[0], &route[1..], &structures);
        assert_eq!(route.last(), Some(&destination));
    }

    #[test]
    fn wall_of_discs_and_fully_covered_map_return_no_route() {
        let layout = arena(5.0);
        let wall: Vec<_> = [-4.0, -2.0, 0.0, 2.0, 4.0]
            .into_iter()
            .map(|z| (Vec3::new(0.0, 0.0, z), StructureKind::Tower))
            .collect();
        assert!(plan_route(&layout, Vec3::new(-4.0, 0.5, 0.0), Vec3::X * 4.0, &wall).is_none());
        assert!(
            plan_route(
                &arena(1.0),
                Vec3::ZERO,
                Vec3::X,
                &[(Vec3::ZERO, StructureKind::BaseTower)],
            )
            .is_none()
        );
    }

    #[test]
    fn invalid_inputs_and_excessive_obstacle_count_fail_closed() {
        let layout = arena(10.0);
        assert!(plan_route(&layout, Vec3::NAN, Vec3::X, &[]).is_none());
        assert!(plan_route(&layout, Vec3::ZERO, Vec3::INFINITY, &[]).is_none());
        assert!(plan_route(&arena(0.0), Vec3::ZERO, Vec3::X, &[]).is_none());
        assert!(plan_route(&layout, Vec3::X * 20.0, Vec3::ZERO, &[]).is_none());
        assert!(
            plan_route(
                &layout,
                Vec3::ZERO,
                Vec3::X,
                &[(Vec3::NAN, StructureKind::Tower)],
            )
            .is_none()
        );
        let excessive = vec![(Vec3::ZERO, StructureKind::Tower); MAX_OBSTACLES + 1];
        assert!(plan_route(&layout, Vec3::ZERO, Vec3::X, &excessive).is_none());
    }
}
