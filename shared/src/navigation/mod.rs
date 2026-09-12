//! Shared horizontal navigation for heroes, authoritative movement and bots.
//!
//! Static convex footprints are indexed and rasterized once. Orders use bounded
//! A* over cached safe grid edges, then exact swept geometry to smooth the path.
//! No renderer, ECS, physics engine or additional dependency is required.

mod geometry;
mod search;
#[cfg(test)]
mod tests;

use geometry::{add, cross, distance_squared, polygon_clearance, scale, sub};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;

pub type Point = [f32; 2];
pub const HERO_RADIUS: f32 = 0.5;
pub const PLANNING_CLEARANCE: f32 = 0.15;
pub const MAX_ROUTE_WAYPOINTS: usize = 256;
pub const MAX_DYNAMIC_DISCS: usize = 32;
const PLANNING_RADIUS: f32 = HERO_RADIUS + PLANNING_CLEARANCE;
const EPSILON: f32 = 0.00001;
const BIN_SIZE: f32 = 8.0;
const MAX_GRID_CELLS: usize = 262_144;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
}

impl Bounds {
    fn contains(self, point: Point) -> bool {
        finite(point)
            && (0..2).all(|axis| point[axis] >= self.min[axis] && point[axis] <= self.max[axis])
    }

    fn clamp(self, point: Point) -> Point {
        [
            point[0].clamp(self.min[0], self.max[0]),
            point[1].clamp(self.min[1], self.max[1]),
        ]
    }
}

/// A live structure's physical footprint. Planning adds the hero and padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disc {
    pub center: Point,
    pub radius: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Obstacle {
    pub id: String,
    pub kind: String,
    pub vertices: Vec<Point>,
}

#[derive(Deserialize)]
struct CollisionData {
    format_version: u32,
    bounds: Bounds,
    obstacles: Vec<Obstacle>,
}

pub struct NavigationMap {
    bounds: Bounds,
    obstacles: Vec<Obstacle>,
    bin_width: usize,
    bin_height: usize,
    bins: Vec<Vec<usize>>,
    grid: search::Grid,
}

/// The embedded, validated world map and its occupancy/edge cache are shared
/// by every order and movement check for the lifetime of the process.
pub fn world_navigation() -> &'static NavigationMap {
    static WORLD: OnceLock<NavigationMap> = OnceLock::new();
    WORLD.get_or_init(|| {
        NavigationMap::from_json(include_str!("../../assets/verdant-collision.json"))
            .expect("versioned Verdant collision data must be valid")
    })
}

fn finite(point: Point) -> bool {
    point.into_iter().all(f32::is_finite)
}

impl NavigationMap {
    pub fn from_json(source: &str) -> Result<Self, String> {
        let data: CollisionData =
            serde_json::from_str(source).map_err(|error| error.to_string())?;
        if data.format_version != 1 {
            return Err("unsupported collision format_version".into());
        }
        Self::new(data.bounds, data.obstacles)
    }

    /// Custom arenas can supply an empty obstacle list. Invalid/unbounded data
    /// is rejected before allocating a search grid or constructing geometry.
    pub fn new(bounds: Bounds, mut obstacles: Vec<Obstacle>) -> Result<Self, String> {
        let size = sub(bounds.max, bounds.min);
        if !finite(bounds.min)
            || !finite(bounds.max)
            || !finite(size)
            || size
                .into_iter()
                .any(|dimension| !(EPSILON..=4096.0).contains(&dimension))
            || obstacles.len() > 4096
        {
            return Err("invalid or excessive navigation bounds/obstacles".into());
        }
        let grid = search::Grid::new(bounds)?;
        let mut identifiers = HashSet::new();
        for obstacle in &mut obstacles {
            if obstacle.id.is_empty()
                || !identifiers.insert(obstacle.id.clone())
                || !(3..=64).contains(&obstacle.vertices.len())
                || obstacle.vertices.iter().any(|&point| {
                    !finite(point)
                        || point
                            .into_iter()
                            .any(|coordinate| coordinate.abs() > 1_000_000.0)
                })
            {
                return Err("obstacles require unique ids and finite convex polygons".into());
            }
            let area: f64 = obstacle
                .vertices
                .iter()
                .zip(obstacle.vertices.iter().cycle().skip(1))
                .map(|(a, b)| a[0] as f64 * b[1] as f64 - a[1] as f64 * b[0] as f64)
                .sum();
            if !area.is_finite() || area.abs() <= EPSILON as f64 {
                return Err(format!("degenerate obstacle {}", obstacle.id));
            }
            if area < 0.0 {
                obstacle.vertices.reverse();
            }
            for (a, b) in obstacle
                .vertices
                .iter()
                .zip(obstacle.vertices.iter().cycle().skip(1))
            {
                let edge = sub(*b, *a);
                if distance_squared(*a, *b) <= EPSILON * EPSILON
                    || obstacle
                        .vertices
                        .iter()
                        .any(|&point| cross(edge, sub(point, *a)) < -EPSILON)
                {
                    return Err(format!("non-convex or repeated edge in {}", obstacle.id));
                }
            }
        }
        // Identifiers stabilize both indexing and equally short route choices.
        obstacles.sort_by(|a, b| a.id.cmp(&b.id));
        let bin_width = (size[0] / BIN_SIZE).ceil() as usize + 1;
        let bin_height = (size[1] / BIN_SIZE).ceil() as usize + 1;
        let mut map = Self {
            bounds,
            obstacles,
            bin_width,
            bin_height,
            bins: vec![Vec::new(); bin_width * bin_height],
            grid,
        };
        for index in 0..map.obstacles.len() {
            let mut min = [f32::INFINITY; 2];
            let mut max = [f32::NEG_INFINITY; 2];
            for point in &map.obstacles[index].vertices {
                for axis in 0..2 {
                    min[axis] = min[axis].min(point[axis] - PLANNING_RADIUS);
                    max[axis] = max[axis].max(point[axis] + PLANNING_RADIUS);
                }
            }
            if (0..2).any(|axis| max[axis] < bounds.min[axis] || min[axis] > bounds.max[axis]) {
                continue;
            }
            let [left, top] = map.bin_coordinates(min);
            let [right, bottom] = map.bin_coordinates(max);
            for row in top..=bottom {
                for column in left..=right {
                    map.bins[row * bin_width + column].push(index);
                }
            }
        }
        map.grid = search::Grid::build(&map)?;
        Ok(map)
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    pub fn obstacles(&self) -> &[Obstacle] {
        &self.obstacles
    }

    fn bin_coordinates(&self, point: Point) -> [usize; 2] {
        [
            (((point[0] - self.bounds.min[0]) / BIN_SIZE).max(0.0) as usize)
                .min(self.bin_width - 1),
            (((point[1] - self.bounds.min[1]) / BIN_SIZE).max(0.0) as usize)
                .min(self.bin_height - 1),
        ]
    }

    fn candidates(&self, from: Point, to: Point) -> Vec<usize> {
        let [left, top] = self.bin_coordinates([from[0].min(to[0]), from[1].min(to[1])]);
        let [right, bottom] = self.bin_coordinates([from[0].max(to[0]), from[1].max(to[1])]);
        let mut candidates = Vec::new();
        for row in top..=bottom {
            for column in left..=right {
                candidates.extend_from_slice(&self.bins[row * self.bin_width + column]);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    fn static_point_clear(&self, point: Point, radius: f32) -> bool {
        self.bounds.contains(point)
            && self.candidates(point, point).into_iter().all(|index| {
                polygon_clearance(point, &self.obstacles[index].vertices).0 + EPSILON >= radius
            })
    }

    fn static_segment_clear(&self, from: Point, to: Point, radius: f32, escape: bool) -> bool {
        self.bounds.contains(from)
            && self.bounds.contains(to)
            && self.candidates(from, to).into_iter().all(|index| {
                geometry::polygon_segment_clear(
                    from,
                    to,
                    &self.obstacles[index].vertices,
                    radius,
                    escape,
                )
            })
    }

    /// Validate a complete authored object's circular footprint, including
    /// arena edges. This does not change the hero movement/edge contract.
    pub fn point_clear_with_radius(&self, point: Point, radius: f32) -> bool {
        finite(point)
            && radius.is_finite()
            && radius >= 0.0
            && (0..2).all(|axis| {
                point[axis] - radius >= self.bounds.min[axis]
                    && point[axis] + radius <= self.bounds.max[axis]
            })
            && self
                .candidates(
                    [point[0] - radius, point[1] - radius],
                    [point[0] + radius, point[1] + radius],
                )
                .into_iter()
                .all(|index| {
                    polygon_clearance(point, &self.obstacles[index].vertices).0 + EPSILON >= radius
                })
    }

    /// Static physical hero clearance, without optional route padding.
    pub fn point_clear(&self, point: Point) -> bool {
        self.static_point_clear(point, HERO_RADIUS)
    }

    pub fn segment_clear(&self, from: Point, to: Point) -> bool {
        self.static_segment_clear(from, to, HERO_RADIUS, false)
    }

    /// Validate a cached route after an external displacement, using physical
    /// hero clearance and the current live structure footprints.
    pub fn segment_clear_with_discs(&self, from: Point, to: Point, dynamic: &[Disc]) -> bool {
        self.segment_clear(from, to)
            && dynamic.iter().all(|disc| {
                geometry::disc_segment_clear(
                    from,
                    to,
                    disc.center,
                    disc.radius + HERO_RADIUS,
                    false,
                )
            })
    }

    /// Sweep the complete step, including arbitrarily large network updates.
    /// Return the last safe point before contact. An overlapped start can only
    /// recover outward; invalid inputs freeze at `from`.
    pub fn clip_movement(&self, from: Point, to: Point) -> Point {
        if !self.bounds.contains(from) || !finite(to) {
            return from;
        }
        let to = self.bounds.clamp(to);
        self.clip_segment(from, to, |a, b| {
            self.static_segment_clear(a, b, HERO_RADIUS, true)
        })
    }

    fn clip_segment(&self, from: Point, to: Point, clear: impl Fn(Point, Point) -> bool) -> Point {
        if clear(from, to) {
            return to;
        }
        let mut low = 0.0;
        let mut high = 1.0;
        for _ in 0..24 {
            let middle = (low + high) * 0.5;
            if clear(from, add(from, scale(sub(to, from), middle))) {
                low = middle;
            } else {
                high = middle;
            }
        }
        let distance = distance_squared(from, to).sqrt();
        let fraction = (low - 0.001 / distance.max(EPSILON)).max(0.0);
        add(from, scale(sub(to, from), fraction))
    }

    fn planning_point_clear(&self, point: Point, dynamic: &[Disc]) -> bool {
        self.static_point_clear(point, PLANNING_RADIUS)
            && dynamic.iter().all(|disc| {
                distance_squared(point, disc.center) + EPSILON
                    >= (disc.radius + PLANNING_RADIUS).powi(2)
            })
    }

    fn planning_segment_clear(
        &self,
        from: Point,
        to: Point,
        dynamic: &[Disc],
        escape: bool,
    ) -> bool {
        self.static_segment_clear(from, to, PLANNING_RADIUS, escape)
            && dynamic.iter().all(|disc| {
                geometry::disc_segment_clear(
                    from,
                    to,
                    disc.center,
                    disc.radius + PLANNING_RADIUS,
                    escape,
                )
            })
    }

    /// Plan once per movement order. Waypoints exclude `start`; an empty route
    /// means arrival. Clear unreachable destinations return None. Blocked
    /// destinations use the nearest reachable grid approach with an exact
    /// boundary projection. Every segment is checked with hero clearance.
    pub fn plan_route(
        &self,
        start: Point,
        destination: Point,
        dynamic: &[Disc],
    ) -> Option<Vec<Point>> {
        if !self.bounds.contains(start)
            || !finite(destination)
            || dynamic.len() > MAX_DYNAMIC_DISCS
            || dynamic.iter().any(|disc| {
                !finite(disc.center)
                    || !disc.radius.is_finite()
                    || !(0.0..=4096.0).contains(&disc.radius)
            })
        {
            return None;
        }
        search::plan(self, start, self.bounds.clamp(destination), dynamic)
    }
}
