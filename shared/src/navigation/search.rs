use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::geometry::{disc_segment_clear, distance_squared};
use super::{
    Bounds, Disc, EPSILON, MAX_GRID_CELLS, MAX_ROUTE_WAYPOINTS, NavigationMap, PLANNING_RADIUS,
    Point,
};

const CELL_SIZE: f32 = 1.0;
const NEIGHBORS: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

pub(super) struct Grid {
    width: usize,
    height: usize,
    step: Point,
    walkable: Vec<bool>,
    edges: Vec<u8>,
}

impl Grid {
    pub(super) fn new(bounds: Bounds) -> Result<Self, String> {
        let size = super::geometry::sub(bounds.max, bounds.min);
        let width = (size[0] / CELL_SIZE).ceil() as usize + 1;
        let height = (size[1] / CELL_SIZE).ceil() as usize + 1;
        let count = width
            .checked_mul(height)
            .ok_or("navigation grid is too large")?;
        if count > MAX_GRID_CELLS {
            return Err("navigation grid exceeds bounded cell count".into());
        }
        Ok(Self {
            width,
            height,
            step: [size[0] / (width - 1) as f32, size[1] / (height - 1) as f32],
            walkable: vec![false; count],
            edges: vec![0; count],
        })
    }

    pub(super) fn build(map: &NavigationMap) -> Result<Self, String> {
        let mut grid = Self::new(map.bounds)?;
        for index in 0..grid.walkable.len() {
            grid.walkable[index] =
                map.static_point_clear(grid.point(index, map.bounds), PLANNING_RADIUS);
        }
        for index in 0..grid.walkable.len() {
            if !grid.walkable[index] {
                continue;
            }
            let from = grid.point(index, map.bounds);
            for (direction, &(dx, dz)) in NEIGHBORS.iter().enumerate() {
                let Some(next) = grid.neighbor(index, dx, dz) else {
                    continue;
                };
                if grid.walkable[next]
                    && grid.corner_clear(index, dx, dz, &grid.walkable)
                    && map.static_segment_clear(
                        from,
                        grid.point(next, map.bounds),
                        PLANNING_RADIUS,
                        false,
                    )
                {
                    grid.edges[index] |= 1 << direction;
                }
            }
        }
        Ok(grid)
    }

    fn point(&self, index: usize, bounds: Bounds) -> Point {
        let x = index % self.width;
        let z = index / self.width;
        [
            if x + 1 == self.width {
                bounds.max[0]
            } else {
                bounds.min[0] + x as f32 * self.step[0]
            },
            if z + 1 == self.height {
                bounds.max[1]
            } else {
                bounds.min[1] + z as f32 * self.step[1]
            },
        ]
    }

    fn neighbor(&self, index: usize, dx: isize, dz: isize) -> Option<usize> {
        let x = (index % self.width).checked_add_signed(dx)?;
        let z = (index / self.width).checked_add_signed(dz)?;
        (x < self.width && z < self.height).then_some(z * self.width + x)
    }

    fn corner_clear(&self, index: usize, dx: isize, dz: isize, walkable: &[bool]) -> bool {
        dx == 0
            || dz == 0
            || (self
                .neighbor(index, dx, 0)
                .is_some_and(|side| walkable[side])
                && self
                    .neighbor(index, 0, dz)
                    .is_some_and(|side| walkable[side]))
    }
}

#[derive(Clone, Copy, PartialEq)]
struct OpenNode {
    priority: f32,
    cost: f32,
    index: usize,
}

impl Eq for OpenNode {}

impl Ord for OpenNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .total_cmp(&self.priority)
            .then_with(|| other.index.cmp(&self.index))
            .then_with(|| other.cost.total_cmp(&self.cost))
    }
}

impl PartialOrd for OpenNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub(super) fn plan(
    map: &NavigationMap,
    start: Point,
    destination: Point,
    dynamic: &[Disc],
) -> Option<Vec<Point>> {
    let destination_clear = map.planning_point_clear(destination, dynamic);
    if destination_clear && map.planning_segment_clear(start, destination, dynamic, true) {
        return Some(
            if distance_squared(start, destination) <= EPSILON * EPSILON {
                Vec::new()
            } else {
                vec![destination]
            },
        );
    }

    let grid = &map.grid;
    // Only live structures are overlaid per order. Static occupancy and exact
    // static adjacency have already been calculated once by the map cache.
    let walkable: Vec<bool> = grid
        .walkable
        .iter()
        .enumerate()
        .map(|(index, &clear)| {
            clear
                && dynamic.iter().all(|disc| {
                    distance_squared(grid.point(index, map.bounds), disc.center) + EPSILON
                        >= (disc.radius + PLANNING_RADIUS).powi(2)
                })
        })
        .collect();
    let mut costs = vec![f32::INFINITY; walkable.len()];
    let mut previous = vec![None; walkable.len()];
    let mut closed = vec![false; walkable.len()];
    let mut open = BinaryHeap::new();

    // Exact visible attachments allow sub-cell starts and overlap recovery.
    // Expand a bounded local window only when no nearby safe attachment exists.
    for radius in [2.5_f32, 8.0, 16.0] {
        let left = ((start[0] - radius - map.bounds.min[0]) / grid.step[0])
            .floor()
            .max(0.0) as usize;
        let right = (((start[0] + radius - map.bounds.min[0]) / grid.step[0]).ceil() as usize)
            .min(grid.width - 1);
        let top = ((start[1] - radius - map.bounds.min[1]) / grid.step[1])
            .floor()
            .max(0.0) as usize;
        let bottom = (((start[1] + radius - map.bounds.min[1]) / grid.step[1]).ceil() as usize)
            .min(grid.height - 1);
        for row in top..=bottom {
            for column in left..=right {
                let index = row * grid.width + column;
                let point = grid.point(index, map.bounds);
                if walkable[index]
                    && distance_squared(start, point) <= radius * radius
                    && map.planning_segment_clear(start, point, dynamic, true)
                {
                    let cost = distance_squared(start, point).sqrt();
                    costs[index] = cost;
                    open.push(OpenNode {
                        priority: cost + distance_squared(point, destination).sqrt(),
                        cost,
                        index,
                    });
                }
            }
        }
        if !open.is_empty() {
            break;
        }
    }

    let mut best: Option<(usize, Point, f32)> = None;
    while let Some(node) = open.pop() {
        if closed[node.index] || node.cost > costs[node.index] {
            continue;
        }
        closed[node.index] = true;
        let from = grid.point(node.index, map.bounds);
        if destination_clear {
            if distance_squared(from, destination) <= 2.5 * 2.5
                && map.planning_segment_clear(from, destination, dynamic, false)
            {
                best = Some((node.index, destination, 0.0));
                break;
            }
        } else if best.is_none_or(|(_, _, distance)| {
            distance_squared(from, destination).sqrt() <= distance.sqrt() + 2.0
        }) {
            // The target is occupied: inspect the reachable component, then
            // retain the nearest safe continuous boundary approach. The exact
            // sweep also handles several overlapping discs and map edges.
            let endpoint = map.clip_segment(from, destination, |a, b| {
                map.planning_segment_clear(a, b, dynamic, false)
            });
            let distance = distance_squared(endpoint, destination);
            if best.is_none_or(|(index, _, prior)| {
                distance < prior - EPSILON
                    || ((distance - prior).abs() <= EPSILON && costs[node.index] < costs[index])
            }) {
                best = Some((node.index, endpoint, distance));
            }
        }
        for (direction, &(dx, dz)) in NEIGHBORS.iter().enumerate() {
            if grid.edges[node.index] & (1 << direction) == 0 {
                continue;
            }
            let next = grid.neighbor(node.index, dx, dz)?;
            if closed[next] || !walkable[next] || !grid.corner_clear(node.index, dx, dz, &walkable)
            {
                continue;
            }
            let to = grid.point(next, map.bounds);
            if !dynamic.iter().all(|disc| {
                disc_segment_clear(from, to, disc.center, disc.radius + PLANNING_RADIUS, false)
            }) {
                continue;
            }
            let cost = node.cost + distance_squared(from, to).sqrt();
            if cost < costs[next] {
                costs[next] = cost;
                previous[next] = Some(node.index);
                open.push(OpenNode {
                    priority: cost + distance_squared(to, destination).sqrt(),
                    cost,
                    index: next,
                });
            }
        }
    }

    let (mut index, endpoint, _) = best?;
    let mut points = vec![endpoint];
    loop {
        let point = grid.point(index, map.bounds);
        if distance_squared(*points.last()?, point) > EPSILON * EPSILON {
            points.push(point);
        }
        let Some(parent) = previous[index] else { break };
        index = parent;
    }
    points.push(start);
    points.reverse();

    // Linear bounded smoothing: extend visibility until the next turn is
    // blocked, commit the previous safe waypoint, and continue from there.
    let mut route = Vec::new();
    let mut anchor = 0;
    while anchor + 1 < points.len() {
        let mut next = anchor + 1;
        while next + 1 < points.len()
            && map.planning_segment_clear(points[anchor], points[next + 1], dynamic, anchor == 0)
        {
            next += 1;
        }
        if distance_squared(points[anchor], points[next]) > EPSILON * EPSILON {
            route.push(points[next]);
            if route.len() > MAX_ROUTE_WAYPOINTS {
                return None;
            }
        }
        anchor = next;
    }
    Some(route)
}
