use std::cell::RefCell;
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
                    && grid.corner_clear(index, dx, dz, |cell| grid.walkable[cell])
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

    fn corner_clear(
        &self,
        index: usize,
        dx: isize,
        dz: isize,
        walkable: impl Fn(usize) -> bool,
    ) -> bool {
        dx == 0
            || dz == 0
            || (self.neighbor(index, dx, 0).is_some_and(&walkable)
                && self.neighbor(index, 0, dz).is_some_and(&walkable))
    }

    /// Mark the cells a live disc blocks for this plan. Only the disc's
    /// bounding cells are visited, with the exact per-cell test, so the
    /// result equals testing every cell of the grid against every disc.
    fn overlay(&self, bounds: Bounds, disc: &Disc, blocked: &mut [u32], epoch: u32) {
        let reach = disc.radius + PLANNING_RADIUS;
        let limit = reach.powi(2);
        // One spare cell on each side absorbs any rounding in the index.
        let span = |centre: f32, min: f32, step: f32, cells: usize| {
            let low = ((centre - reach - min) / step).floor() - 1.0;
            let high = ((centre + reach - min) / step).ceil() + 1.0;
            (
                (low.max(0.0) as usize).min(cells - 1),
                (high.max(0.0) as usize).min(cells - 1),
            )
        };
        let (left, right) = span(disc.center[0], bounds.min[0], self.step[0], self.width);
        let (top, bottom) = span(disc.center[1], bounds.min[1], self.step[1], self.height);
        for row in top..=bottom {
            for column in left..=right {
                let index = row * self.width + column;
                if self.walkable[index]
                    && distance_squared(self.point(index, bounds), disc.center) + EPSILON < limit
                {
                    blocked[index] = epoch;
                }
            }
        }
    }
}

/// Per-thread search buffers, reused by every plan on that thread, so a plan
/// allocates nothing in proportion to the grid. An entry is valid only while
/// its stamp equals the current plan's epoch, so no buffer is cleared
/// between plans.
#[derive(Default)]
struct Scratch {
    /// Cells a live disc blocks in this plan (the dynamic overlay).
    blocked: Vec<u32>,
    search: Search,
}

#[derive(Default)]
struct Search {
    epoch: u32,
    /// `costs` and `previous` hold this plan's values.
    seen: Vec<u32>,
    closed: Vec<u32>,
    costs: Vec<f32>,
    previous: Vec<u32>,
    open: BinaryHeap<OpenNode>,
    /// The cell path, then the smoothed route returned as one exact copy.
    points: Vec<Point>,
    route: Vec<Point>,
}

const NO_PREVIOUS: u32 = u32::MAX;

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::default();
}

impl Scratch {
    /// Start a plan over `cells` cells: a fresh epoch, buffers grown once.
    fn begin(&mut self, cells: usize) -> u32 {
        let search = &mut self.search;
        if self.blocked.len() < cells {
            for stamps in [&mut self.blocked, &mut search.seen, &mut search.closed] {
                stamps.resize(cells, 0);
            }
            search.costs.resize(cells, f32::INFINITY);
            search.previous.resize(cells, NO_PREVIOUS);
        }
        search.epoch = search.epoch.wrapping_add(1);
        if search.epoch == 0 {
            // A stamp from 2^32 plans ago would read as current.
            for stamps in [&mut self.blocked, &mut search.seen, &mut search.closed] {
                stamps.fill(0);
            }
            search.epoch = 1;
        }
        search.open.clear();
        search.points.clear();
        search.route.clear();
        search.epoch
    }
}

impl Search {
    fn cost(&self, index: usize) -> f32 {
        if self.seen[index] == self.epoch {
            self.costs[index]
        } else {
            f32::INFINITY
        }
    }

    fn reach(&mut self, index: usize, cost: f32, previous: u32) {
        self.seen[index] = self.epoch;
        self.costs[index] = cost;
        self.previous[index] = previous;
    }

    fn previous(&self, index: usize) -> Option<usize> {
        (self.seen[index] == self.epoch && self.previous[index] != NO_PREVIOUS)
            .then(|| self.previous[index] as usize)
    }

    fn closed(&self, index: usize) -> bool {
        self.closed[index] == self.epoch
    }
}

/// One order's inputs.
struct Query<'a> {
    map: &'a NavigationMap,
    start: Point,
    destination: Point,
    destination_clear: bool,
    dynamic: &'a [Disc],
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

    let query = Query {
        map,
        start,
        destination,
        destination_clear,
        dynamic,
    };
    SCRATCH.with(|scratch| match scratch.try_borrow_mut() {
        Ok(mut scratch) => search(&query, &mut scratch),
        // Only a plan nested inside another on this thread gets here.
        Err(_) => search(&query, &mut Scratch::default()),
    })
}

fn search(query: &Query, scratch: &mut Scratch) -> Option<Vec<Point>> {
    let map = query.map;
    let grid = &map.grid;
    let epoch = scratch.begin(grid.walkable.len());
    // Only live structures are overlaid per order. Static occupancy and exact
    // static adjacency have already been calculated once by the map cache.
    for disc in query.dynamic {
        grid.overlay(map.bounds, disc, &mut scratch.blocked, epoch);
    }
    let blocked = &scratch.blocked;
    astar(query, &mut scratch.search, |index| {
        grid.walkable[index] && blocked[index] != epoch
    })
}

fn astar(
    query: &Query,
    scratch: &mut Search,
    walkable: impl Fn(usize) -> bool,
) -> Option<Vec<Point>> {
    let &Query {
        map,
        start,
        destination,
        destination_clear,
        dynamic,
    } = query;
    let grid = &map.grid;

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
                if walkable(index)
                    && distance_squared(start, point) <= radius * radius
                    && map.planning_segment_clear(start, point, dynamic, true)
                {
                    let cost = distance_squared(start, point).sqrt();
                    scratch.reach(index, cost, NO_PREVIOUS);
                    scratch.open.push(OpenNode {
                        priority: cost + distance_squared(point, destination).sqrt(),
                        cost,
                        index,
                    });
                }
            }
        }
        if !scratch.open.is_empty() {
            break;
        }
    }

    let mut best: Option<(usize, Point, f32)> = None;
    while let Some(node) = scratch.open.pop() {
        if scratch.closed(node.index) || node.cost > scratch.cost(node.index) {
            continue;
        }
        scratch.closed[node.index] = scratch.epoch;
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
                    || ((distance - prior).abs() <= EPSILON
                        && scratch.cost(node.index) < scratch.cost(index))
            }) {
                best = Some((node.index, endpoint, distance));
            }
        }
        for (direction, &(dx, dz)) in NEIGHBORS.iter().enumerate() {
            if grid.edges[node.index] & (1 << direction) == 0 {
                continue;
            }
            let next = grid.neighbor(node.index, dx, dz)?;
            if scratch.closed(next)
                || !walkable(next)
                || !grid.corner_clear(node.index, dx, dz, &walkable)
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
            if cost < scratch.cost(next) {
                scratch.reach(next, cost, node.index as u32);
                scratch.open.push(OpenNode {
                    priority: cost + distance_squared(to, destination).sqrt(),
                    cost,
                    index: next,
                });
            }
        }
    }

    let (mut index, endpoint, _) = best?;
    scratch.points.push(endpoint);
    loop {
        let point = grid.point(index, map.bounds);
        if distance_squared(*scratch.points.last()?, point) > EPSILON * EPSILON {
            scratch.points.push(point);
        }
        let Some(parent) = scratch.previous(index) else {
            break;
        };
        index = parent;
    }
    let Search { points, route, .. } = scratch;
    points.push(start);
    points.reverse();

    // Linear bounded smoothing: extend visibility until the next turn is
    // blocked, commit the previous safe waypoint, and continue from there.
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
    Some(route.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::{Obstacle, world_navigation};

    fn noise() -> impl FnMut() -> f32 {
        let mut state = 0x2545_f491_u32;
        move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as f32 / u32::MAX as f32
        }
    }

    /// The bounding-box overlay marks exactly the cells a scan of the whole
    /// grid against the disc marks, including discs over the map edge, a
    /// zero radius and one larger than the map.
    #[test]
    fn disc_overlay_matches_a_full_grid_scan() {
        let map = world_navigation();
        let grid = &map.grid;
        let bounds = map.bounds;
        let mut blocked = vec![0; grid.walkable.len()];
        let mut next = noise();
        let size = [bounds.max[0] - bounds.min[0], bounds.max[1] - bounds.min[1]];
        for epoch in 1..=96_u32 {
            let radius = match epoch {
                1 => 0.0,
                2 => 400.0,
                _ => next() * 14.0,
            };
            let disc = Disc {
                center: [
                    bounds.min[0] - 6.0 + next() * (size[0] + 12.0),
                    bounds.min[1] - 6.0 + next() * (size[1] + 12.0),
                ],
                radius,
            };
            grid.overlay(bounds, &disc, &mut blocked, epoch);
            let limit = (disc.radius + PLANNING_RADIUS).powi(2);
            for (index, &stamp) in blocked.iter().enumerate() {
                let expected = grid.walkable[index]
                    && distance_squared(grid.point(index, bounds), disc.center) + EPSILON < limit;
                assert_eq!(stamp == epoch, expected, "cell {index}, {disc:?}");
            }
        }
    }

    fn rectangle(id: &str, min: Point, max: Point) -> Obstacle {
        Obstacle {
            id: id.into(),
            kind: "test_solid".into(),
            vertices: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
        }
    }

    /// One thread's buffers serve maps of different sizes and survive the
    /// epoch wrapping around: every plan equals the same plan on a thread
    /// that has never planned.
    #[test]
    fn reused_buffers_plan_like_fresh_ones_across_maps_and_an_epoch_wrap() {
        let small = NavigationMap::new(
            Bounds {
                min: [-12.0; 2],
                max: [12.0; 2],
            },
            vec![
                rectangle("left", [-5.0, -5.0], [-3.0, 5.0]),
                rectangle("right", [3.0, -5.0], [5.0, 5.0]),
                rectangle("back", [-5.0, -5.0], [5.0, -3.0]),
            ],
        )
        .unwrap();
        let world = world_navigation();
        let scale = crate::map::WORLD_SCALE;
        let tower = Disc {
            center: [20.0, 0.0],
            radius: 3.2,
        };
        let orders: Vec<(&NavigationMap, Point, Point, Vec<Disc>)> = vec![
            (&small, [0.0, -1.0], [0.0, -9.0], vec![]),
            (
                world,
                [66.641_04 * scale, 11.667_086 * scale],
                [76.641_04 * scale, 11.667_086 * scale],
                vec![],
            ),
            // An occupied destination searches the reachable component.
            (world, [0.0, 0.0], tower.center, vec![tower]),
            (
                &small,
                [-8.0, 8.0],
                [8.0, -8.0],
                vec![Disc {
                    center: [0.0, 8.0],
                    radius: 2.0,
                }],
            ),
            (world, [-60.0, -60.0], [60.0, 60.0], vec![tower]),
        ];
        let plan_all = || -> Vec<Option<Vec<Point>>> {
            orders
                .iter()
                .map(|(map, start, destination, discs)| map.plan_route(*start, *destination, discs))
                .collect()
        };
        let fresh = std::thread::scope(|scope| scope.spawn(plan_all).join().unwrap());
        assert!(fresh.iter().all(Option::is_some), "{fresh:?}");
        assert_eq!(plan_all(), fresh);
        // Stamps from the plans above would read as current after a wrap
        // that did not clear them.
        SCRATCH.with(|scratch| scratch.borrow_mut().search.epoch = u32::MAX - 2);
        for _ in 0..3 {
            assert_eq!(plan_all(), fresh);
        }
        assert!(SCRATCH.with(|scratch| scratch.borrow().search.epoch) < 64);
    }
}
