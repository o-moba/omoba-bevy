//! Local XZ navigation for the same structure discs used by player collision.
//! This is a bounded visibility graph, not terrain or dynamic-player navigation.

use bevy::prelude::*;

use crate::maps::MapLayout;
use crate::net::StructureKind;
use crate::player::PLAYER_SIZE;

const CLEARANCE: f32 = 0.15;
const NODE_CLEARANCE: f32 = 0.001;
const RING_SAMPLES: usize = 24;
// The current map has eight structures. Fail closed on unexpectedly large input
// instead of allowing an unbounded visibility graph on the input thread.
const MAX_OBSTACLES: usize = 32;
const EPSILON: f32 = 0.00001;

#[derive(Clone, Copy)]
struct Obstacle {
    center: Vec2,
    radius: f32,
}

/// Physical hero-center clearance, shared with the movement collision solver.
pub(crate) fn structure_collision_radius(kind: StructureKind) -> f32 {
    PLAYER_SIZE * 0.5
        + match kind {
            StructureKind::Tower => 1.3,
            StructureKind::BaseTower => 3.2,
        }
}

/// Plan once per order. Waypoints exclude `start` and preserve its Y coordinate.
/// An empty route means arrival; `None` means invalid input or no safe route.
///
/// Destinations are clamped to the map. A destination inside a padded structure
/// is projected to the nearest reachable graph candidate (including the radial
/// projection), so overlapping discs or map edges cannot produce an unsafe end.
/// A start already overlapping a disc may only move monotonically outward until
/// clear; the normal collision solver remains responsible for overlap recovery.
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
        || !layout.size().is_finite()
        || (layout.max - layout.min).min_element() <= EPSILON
        || structures.len() > MAX_OBSTACLES
    {
        return None;
    }
    let start_xz = Vec2::new(start.x, start.z);
    if !in_bounds(layout, start_xz) {
        return None;
    }
    let destination = Vec2::new(destination.x, destination.z).clamp(layout.min, layout.max);
    let mut obstacles = Vec::with_capacity(structures.len());
    for &(position, kind) in structures {
        if !position.is_finite() {
            return None;
        }
        obstacles.push(Obstacle {
            center: Vec2::new(position.x, position.z),
            radius: structure_collision_radius(kind) + CLEARANCE,
        });
    }
    // ECS query order must not change which equally short side of a tower wins.
    obstacles.sort_by(|a, b| {
        a.center
            .x
            .total_cmp(&b.center.x)
            .then_with(|| a.center.y.total_cmp(&b.center.y))
            .then_with(|| a.radius.total_cmp(&b.radius))
    });
    let destination_clear = point_clear(destination, &obstacles);
    if destination_clear && segment_clear(start_xz, destination, &obstacles, true) {
        return Some(
            if start_xz.distance_squared(destination) <= EPSILON * EPSILON {
                Vec::new()
            } else {
                vec![Vec3::new(destination.x, start.y, destination.y)]
            },
        );
    }

    let mut nodes = vec![start_xz];
    if destination_clear {
        nodes.push(destination);
    }
    for obstacle in &obstacles {
        // Leave positive clearance at each chord, because adding small offsets
        // to large world coordinates rounds f32 vertices. Exactly tangent
        // chords can otherwise round inside the disc and disconnect its ring.
        // Every actual graph edge still passes the unchanged clearance check.
        let ring_radius =
            (obstacle.radius + NODE_CLEARANCE) / (std::f32::consts::PI / RING_SAMPLES as f32).cos();
        for sample in 0..RING_SAMPLES {
            let angle = std::f32::consts::TAU * sample as f32 / RING_SAMPLES as f32;
            let point = obstacle.center + Vec2::new(angle.cos(), angle.sin()) * ring_radius;
            add_node(&mut nodes, point, layout, &obstacles);
        }
        for reference in [destination, start_xz] {
            let mut direction = reference - obstacle.center;
            if direction.length_squared() <= EPSILON * EPSILON {
                direction = start_xz - obstacle.center;
            }
            let direction = direction.try_normalize().unwrap_or(Vec2::X);
            let point = obstacle.center + direction * (obstacle.radius + NODE_CLEARANCE);
            add_node(&mut nodes, point, layout, &obstacles);
        }
        // Ring samples alone can miss the narrow space between a disc and the
        // arena edge. Include exact ring/edge intersections, then check edges.
        for edge_x in [layout.min.x, layout.max.x] {
            let remainder = ring_radius * ring_radius - (edge_x - obstacle.center.x).powi(2);
            if remainder >= 0.0 {
                for side in [-1.0, 1.0] {
                    add_node(
                        &mut nodes,
                        Vec2::new(edge_x, obstacle.center.y + side * remainder.sqrt()),
                        layout,
                        &obstacles,
                    );
                }
            }
        }
        for edge_z in [layout.min.y, layout.max.y] {
            let remainder = ring_radius * ring_radius - (edge_z - obstacle.center.y).powi(2);
            if remainder >= 0.0 {
                for side in [-1.0, 1.0] {
                    add_node(
                        &mut nodes,
                        Vec2::new(obstacle.center.x + side * remainder.sqrt(), edge_z),
                        layout,
                        &obstacles,
                    );
                }
            }
        }
    }
    for corner in [
        layout.min,
        Vec2::new(layout.min.x, layout.max.y),
        layout.max,
        Vec2::new(layout.max.x, layout.min.y),
    ] {
        add_node(&mut nodes, corner, layout, &obstacles);
    }

    let mut distance = vec![f32::INFINITY; nodes.len()];
    let mut previous = vec![None; nodes.len()];
    let mut visited = vec![false; nodes.len()];
    distance[0] = 0.0;
    for _ in 0..nodes.len() {
        let Some(current) = (0..nodes.len())
            .filter(|&index| !visited[index] && distance[index].is_finite())
            .min_by(|&a, &b| distance[a].total_cmp(&distance[b]))
        else {
            break;
        };
        visited[current] = true;
        if destination_clear && current == 1 {
            break;
        }
        for next in 1..nodes.len() {
            if visited[next] {
                continue;
            }
            let candidate = distance[current] + nodes[current].distance(nodes[next]);
            if candidate < distance[next]
                && segment_clear(nodes[current], nodes[next], &obstacles, current == 0)
            {
                distance[next] = candidate;
                previous[next] = Some(current);
            }
        }
    }
    let end = if destination_clear {
        distance[1].is_finite().then_some(1)?
    } else {
        (1..nodes.len())
            .filter(|&index| distance[index].is_finite())
            .min_by(|&a, &b| {
                nodes[a]
                    .distance_squared(destination)
                    .total_cmp(&nodes[b].distance_squared(destination))
                    .then_with(|| distance[a].total_cmp(&distance[b]))
            })?
    };
    let mut route = Vec::new();
    let mut current = end;
    while current != 0 {
        let point = nodes[current];
        route.push(Vec3::new(point.x, start.y, point.y));
        current = previous[current]?;
    }
    route.reverse();
    Some(route)
}

fn in_bounds(layout: &MapLayout, point: Vec2) -> bool {
    point.cmpge(layout.min).all() && point.cmple(layout.max).all()
}

fn point_clear(point: Vec2, obstacles: &[Obstacle]) -> bool {
    obstacles.iter().all(|obstacle| {
        point.distance_squared(obstacle.center) + EPSILON >= obstacle.radius * obstacle.radius
    })
}

fn add_node(nodes: &mut Vec<Vec2>, point: Vec2, layout: &MapLayout, obstacles: &[Obstacle]) {
    if point.is_finite()
        && in_bounds(layout, point)
        && point_clear(point, obstacles)
        && !nodes
            .iter()
            .any(|node| node.distance_squared(point) <= EPSILON * EPSILON)
    {
        nodes.push(point);
    }
}

fn segment_clear(from: Vec2, to: Vec2, obstacles: &[Obstacle], allow_start_escape: bool) -> bool {
    let step = to - from;
    let length_squared = step.length_squared();
    obstacles.iter().all(|obstacle| {
        let offset = from - obstacle.center;
        let radius_squared = obstacle.radius * obstacle.radius;
        if allow_start_escape && offset.length_squared() + EPSILON < radius_squared {
            return offset.dot(step) >= -EPSILON
                && to.distance_squared(obstacle.center) + EPSILON >= radius_squared;
        }
        let projection = if length_squared > EPSILON * EPSILON {
            (-offset.dot(step) / length_squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (offset + step * projection).length_squared() + EPSILON >= radius_squared
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(route.len() >= 3);
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
