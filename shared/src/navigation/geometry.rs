//! Exact convex geometry in the horizontal simulation plane.

use super::{EPSILON, Point};

pub(super) fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}

pub(super) fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

pub(super) fn scale(a: Point, factor: f32) -> Point {
    [a[0] * factor, a[1] * factor]
}

pub(super) fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

pub(super) fn cross(a: Point, b: Point) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

pub(super) fn distance_squared(a: Point, b: Point) -> f32 {
    let offset = sub(a, b);
    dot(offset, offset)
}

fn closest_on_segment(point: Point, a: Point, b: Point) -> Point {
    let edge = sub(b, a);
    let denominator = dot(edge, edge);
    let fraction = if denominator > EPSILON * EPSILON {
        (dot(sub(point, a), edge) / denominator).clamp(0.0, 1.0)
    } else {
        0.0
    };
    add(a, scale(edge, fraction))
}

fn segment_distance_squared(a: Point, b: Point, c: Point, d: Point) -> f32 {
    let first = sub(b, a);
    let second = sub(d, c);
    let denominator = cross(first, second);
    if denominator.abs() > EPSILON * EPSILON {
        let t = cross(sub(c, a), second) / denominator;
        let u = cross(sub(c, a), first) / denominator;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            return 0.0;
        }
    }
    distance_squared(a, closest_on_segment(a, c, d))
        .min(distance_squared(b, closest_on_segment(b, c, d)))
        .min(distance_squared(c, closest_on_segment(c, a, b)))
        .min(distance_squared(d, closest_on_segment(d, a, b)))
}

/// Signed Euclidean clearance and an outward supporting normal. Polygons are
/// validated and normalized counterclockwise by NavigationMap::new.
pub(super) fn polygon_clearance(point: Point, vertices: &[Point]) -> (f32, Point) {
    let mut inside = true;
    let mut nearest_distance = f32::INFINITY;
    let mut nearest = point;
    let mut edge_normal = [1.0, 0.0];
    for (a, b) in vertices.iter().zip(vertices.iter().cycle().skip(1)) {
        let edge = sub(*b, *a);
        inside &= cross(edge, sub(point, *a)) >= 0.0;
        let candidate = closest_on_segment(point, *a, *b);
        let distance = distance_squared(point, candidate);
        if distance < nearest_distance {
            nearest_distance = distance;
            nearest = candidate;
            let length = dot(edge, edge).sqrt();
            edge_normal = [edge[1] / length, -edge[0] / length];
        }
    }
    let distance = nearest_distance.sqrt();
    let normal = if distance > EPSILON {
        scale(
            if inside {
                sub(nearest, point)
            } else {
                sub(point, nearest)
            },
            1.0 / distance,
        )
    } else {
        edge_normal
    };
    (if inside { -distance } else { distance }, normal)
}

pub(super) fn polygon_segment_clear(
    from: Point,
    to: Point,
    vertices: &[Point],
    radius: f32,
    allow_escape: bool,
) -> bool {
    let (clearance, normal) = polygon_clearance(from, vertices);
    if clearance + EPSILON < radius {
        // Signed distance to a convex body is convex along this ray. A
        // nonnegative initial outward derivative therefore cannot dig deeper
        // or re-enter later. Partial recovery steps are allowed by collision.
        return allow_escape && dot(normal, sub(to, from)) >= 0.0;
    }
    if polygon_clearance(to, vertices).0 + EPSILON < radius {
        return false;
    }
    vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .all(|(a, b)| segment_distance_squared(from, to, *a, *b) + EPSILON >= radius * radius)
}

pub(super) fn disc_segment_clear(
    from: Point,
    to: Point,
    center: Point,
    radius: f32,
    allow_escape: bool,
) -> bool {
    let offset = sub(from, center);
    if dot(offset, offset) + EPSILON < radius * radius {
        return allow_escape && dot(offset, sub(to, from)) >= 0.0;
    }
    distance_squared(center, closest_on_segment(center, from, to)) + EPSILON >= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [Point; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

    #[test]
    fn swept_segments_cannot_tunnel_through_a_polygon_or_disc() {
        assert!(!polygon_segment_clear(
            [-100.0, 0.0],
            [100.0, 0.0],
            &SQUARE,
            0.5,
            false
        ));
        assert!(!disc_segment_clear(
            [-100.0, 0.0],
            [100.0, 0.0],
            [0.0, 0.0],
            0.5,
            false
        ));
        assert!(polygon_segment_clear(
            [-100.0, 1.5],
            [100.0, 1.5],
            &SQUARE,
            0.5,
            false
        ));
        assert!(!polygon_segment_clear(
            [-100.0, 1.49],
            [100.0, 1.49],
            &SQUARE,
            0.5,
            false
        ));
    }

    #[test]
    fn overlap_can_recover_only_outward_and_can_take_small_steps() {
        assert!(polygon_segment_clear(
            [1.2, 0.0],
            [1.3, 0.0],
            &SQUARE,
            0.5,
            true
        ));
        assert!(!polygon_segment_clear(
            [1.2, 0.0],
            [-3.0, 0.0],
            &SQUARE,
            0.5,
            true
        ));
        assert!(polygon_segment_clear(
            [0.8, 0.0],
            [3.0, 0.0],
            &SQUARE,
            0.5,
            true
        ));
        assert!(!polygon_segment_clear(
            [0.8, 0.0],
            [-3.0, 0.0],
            &SQUARE,
            0.5,
            true
        ));
        assert!(disc_segment_clear(
            [0.2, 0.0],
            [0.3, 0.0],
            [0.0, 0.0],
            0.5,
            true
        ));
        assert!(!disc_segment_clear(
            [0.2, 0.0],
            [-3.0, 0.0],
            [0.0, 0.0],
            0.5,
            true
        ));
    }
}
