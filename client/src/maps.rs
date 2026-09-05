use bevy::prelude::*;

use crate::player::{PLAYER_SIZE, PLAYER_SPEED};
use crate::team::Team;

pub const TARGET_BASE_RUN_TIME_SECONDS: f32 = 45.0;
pub const TARGET_BASE_DISTANCE: f32 = PLAYER_SPEED * TARGET_BASE_RUN_TIME_SECONDS;
pub(crate) const BASE_PAD_SIZE: f32 = 46.0;
pub(crate) const BASE_PAD_HEIGHT: f32 = 0.7;
/// Horizontal length of the walk-up ramps around each base pad. Characters
/// ascend/descend over this distance (League-style client-side fake: the
/// server keeps a flat ground plane, only the rendered height changes).
pub(crate) const PAD_RAMP_LENGTH: f32 = 6.0;
/// Blocks whose center is within this distance of a neutral-camp or
/// boss-pit anchor get no decorative box (the creature must be visible).
const JUNGLE_BLOCK_CLEARANCE: f32 = 10.0;
const BASE_EDGE_MARGIN: f32 = 6.0;
const PLAYER_SPAWN_OFFSET: f32 = 7.0;
pub(crate) const LANE_WIDTH: f32 = 12.0;
const LANE_EDGE_PADDING: f32 = 6.0;
pub(crate) const RIVER_WIDTH: f32 = 18.0;
/// Fraction of the map size where the outer jungle blocks/camps sit
/// (mirrors `JUNGLE_MAP_OUTER_FRAC` in server/src/balance.rs).
const JUNGLE_MAP_OUTER_FRAC: f32 = 0.34;
/// Fraction of the map size for the inner jungle blocks/camps
/// (mirrors `JUNGLE_MAP_INNER_FRAC` in server/src/balance.rs).
const JUNGLE_MAP_INNER_FRAC: f32 = 0.22;
/// Fraction of the map size for the two mid jungle blocks.
const JUNGLE_MAP_MID_FRAC: f32 = 0.28;

/// Height of one pad's walkable surface at an offset (dx, dz) from the pad
/// center. Full height on the 46×46 top, linear ramp over `PAD_RAMP_LENGTH`
/// beyond each edge (ramp slabs span the pad plus both corner extensions),
/// zero outside.
fn pad_surface_height(dx: f32, dz: f32) -> f32 {
    let half = BASE_PAD_SIZE * 0.5;
    let reach = half + PAD_RAMP_LENGTH;
    let ax = dx.abs();
    let az = dz.abs();
    if ax > reach || az > reach {
        return 0.0;
    }
    if ax <= half && az <= half {
        return BASE_PAD_HEIGHT;
    }
    let mut height: f32 = 0.0;
    if ax > half {
        let t = ((ax - half) / PAD_RAMP_LENGTH).clamp(0.0, 1.0);
        height = height.max(BASE_PAD_HEIGHT * (1.0 - t));
    }
    if az > half {
        let t = ((az - half) / PAD_RAMP_LENGTH).clamp(0.0, 1.0);
        height = height.max(BASE_PAD_HEIGHT * (1.0 - t));
    }
    height
}

#[derive(Resource, Clone, Copy)]
pub struct MapLayout {
    pub home_spawn: Vec3,
    pub away_spawn: Vec3,
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for MapLayout {
    fn default() -> Self {
        let inner_side = TARGET_BASE_DISTANCE / 2.0_f32.sqrt();
        let half_inner_side = inner_side * 0.5;
        let base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
        let half_map_size = half_inner_side + base_padding;
        let home_spawn = Vec3::new(-half_inner_side, PLAYER_SIZE * 0.5, -half_inner_side);
        let away_spawn = Vec3::new(half_inner_side, PLAYER_SIZE * 0.5, half_inner_side);

        Self {
            home_spawn,
            away_spawn,
            min: Vec2::new(-half_map_size, -half_map_size),
            max: Vec2::new(half_map_size, half_map_size),
        }
    }
}

impl MapLayout {
    pub fn size(self) -> Vec2 {
        self.max - self.min
    }

    pub fn clamp_position(self, world_pos: Vec3) -> Vec3 {
        Vec3::new(
            world_pos.x.clamp(self.min.x, self.max.x),
            world_pos.y,
            world_pos.z.clamp(self.min.y, self.max.y),
        )
    }

    pub fn team_spawn(self, team: Team) -> Vec3 {
        let base = match team {
            Team::Green => self.home_spawn,
            Team::Blue => self.away_spawn,
        };
        let mut dir = Vec3::new(-base.x, 0.0, -base.z);
        if dir.length_squared() > 0.0001 {
            dir = dir.normalize();
        } else {
            dir = Vec3::ZERO;
        }
        base + Vec3::new(
            dir.x * PLAYER_SPAWN_OFFSET,
            0.0,
            dir.z * PLAYER_SPAWN_OFFSET,
        )
    }

    pub fn center_lane_distance(self) -> f32 {
        Vec2::new(
            self.away_spawn.x - self.home_spawn.x,
            self.away_spawn.z - self.home_spawn.z,
        )
        .length()
    }

    /// Offset from the arena edge to the outer lane center lines.
    fn lane_edge_offset(self) -> f32 {
        LANE_EDGE_PADDING + LANE_WIDTH * 0.5
    }

    /// Lane-center polylines in the XZ plane, ordered Mid, Top, Bot.
    ///
    /// These control points match the Verdant export and the 2D renderer and
    /// mirror the server's `lane_control_points` (server/src/world.rs).
    pub(crate) fn lane_polylines(self) -> [Vec<Vec2>; 3] {
        let lane_edge_offset = self.lane_edge_offset();
        let left_x = self.min.x + lane_edge_offset;
        let right_x = self.max.x - lane_edge_offset;
        let top_z = self.max.y - lane_edge_offset;
        let bottom_z = self.min.y + lane_edge_offset;
        let home = Vec2::new(self.home_spawn.x, self.home_spawn.z);
        let away = Vec2::new(self.away_spawn.x, self.away_spawn.z);

        [
            vec![home, away],
            vec![
                home,
                Vec2::new(left_x, home.y),
                Vec2::new(left_x, top_z),
                Vec2::new(right_x, top_z),
                Vec2::new(away.x, top_z),
                away,
            ],
            vec![
                home,
                Vec2::new(home.x, bottom_z),
                Vec2::new(left_x, bottom_z),
                Vec2::new(right_x, bottom_z),
                Vec2::new(right_x, away.y),
                away,
            ],
        ]
    }

    /// River center line in the XZ plane (NW corner to SE corner).
    pub(crate) fn river_polyline(self) -> [Vec2; 2] {
        let lane_edge_offset = self.lane_edge_offset();
        [
            Vec2::new(self.min.x + lane_edge_offset, self.max.y - lane_edge_offset),
            Vec2::new(self.max.x - lane_edge_offset, self.min.y + lane_edge_offset),
        ]
    }

    /// XZ centers of the ten legacy jungle regions, retained for 2D tiles.
    pub(crate) fn jungle_block_centers(self) -> [Vec2; 10] {
        let map_size = self.size().x;
        let outer = map_size * JUNGLE_MAP_OUTER_FRAC;
        let inner = map_size * JUNGLE_MAP_INNER_FRAC;
        let mid = map_size * JUNGLE_MAP_MID_FRAC;
        [
            Vec2::new(-outer, inner),
            Vec2::new(-inner, outer),
            Vec2::new(-outer, -inner),
            Vec2::new(-inner, -outer),
            Vec2::new(outer, inner),
            Vec2::new(inner, outer),
            Vec2::new(outer, -inner),
            Vec2::new(inner, -outer),
            Vec2::new(0.0, mid),
            Vec2::new(0.0, -mid),
        ]
    }

    /// Walkable ground height at an XZ position: `BASE_PAD_HEIGHT` on top of
    /// either base pad, a linear descent along the ramp band around it, and
    /// 0.0 on open ground. Matches the rendered pad + ramp-slab geometry:
    /// side ramps extend `PAD_RAMP_LENGTH` past the pad corners and overlap
    /// there, so the corner height is the max of the two slabs.
    pub fn terrain_height(self, x: f32, z: f32) -> f32 {
        let home = pad_surface_height(x - self.home_spawn.x, z - self.home_spawn.z);
        let away = pad_surface_height(x - self.away_spawn.x, z - self.away_spawn.z);
        home.max(away)
    }

    /// Verdant's four mitered ramp trapezoids meet at the height determined
    /// by the larger absolute offset, without the old overlapping slabs.
    /// Runtime export normalizes all other walkable tops (including bridges
    /// and sanctuary paving) to within 0.05 m of this baseline. See the
    /// shipped verdant/manifest.json surface contract and geometry checks.
    /// Server movement remains flat XZ; Sprite2d uses `terrain_height`.
    pub fn terrain_height_3d(self, x: f32, z: f32) -> f32 {
        let height = |center: Vec3| {
            let offset = (x - center.x).abs().max((z - center.z).abs());
            BASE_PAD_HEIGHT
                * ((BASE_PAD_SIZE * 0.5 + PAD_RAMP_LENGTH - offset) / PAD_RAMP_LENGTH)
                    .clamp(0.0, 1.0)
        };
        height(self.home_spawn).max(height(self.away_spawn))
    }

    /// XZ centers of the three neutral jungle camps
    /// (mirrors `jungle_camp_blueprints` in server/src/neutrals.rs).
    pub(crate) fn camp_centers(self) -> [Vec2; 3] {
        let map_size = self.size().x;
        let outer = map_size * JUNGLE_MAP_OUTER_FRAC;
        let inner = map_size * JUNGLE_MAP_INNER_FRAC;
        [
            Vec2::new(-outer, inner),
            Vec2::new(outer, -inner),
            Vec2::new(-inner, -outer),
        ]
    }

    /// XZ anchors of the two raid-boss pits
    /// (mirrors `boss_blueprints` in server/src/neutrals.rs; the boss pit
    /// fracs equal the jungle fracs, see server/src/balance.rs).
    pub(crate) fn boss_pit_centers(self) -> [Vec2; 2] {
        let map_size = self.size().x;
        let outer = map_size * JUNGLE_MAP_OUTER_FRAC;
        let inner = map_size * JUNGLE_MAP_INNER_FRAC;
        [Vec2::new(inner, -outer), Vec2::new(-inner, outer)]
    }

    /// Jungle regions that receive 2D forest tiles: regions hosting a neutral
    /// camp or a boss pit remain open. The Verdant scene has authored clearings.
    pub(crate) fn decorative_jungle_block_centers(self) -> Vec<Vec2> {
        let camps = self.camp_centers();
        let pits = self.boss_pit_centers();
        self.jungle_block_centers()
            .into_iter()
            .filter(|center| {
                camps
                    .iter()
                    .chain(pits.iter())
                    .all(|anchor| center.distance(*anchor) > JUNGLE_BLOCK_CLEARANCE)
            })
            .collect()
    }
}

/// Layout authority shared by both presentation modes. The 3D scene is owned
/// by `Verdant3dPlugin`; the 2D tile world remains owned by `World2dPlugin`.
pub struct MapsPlugin;

impl Plugin for MapsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLayout>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.0001;

    #[test]
    fn layout_is_square_and_symmetric() {
        let layout = MapLayout::default();
        let size = layout.size();
        assert!((size.x - size.y).abs() < EPSILON);
        assert!((layout.home_spawn.x + layout.away_spawn.x).abs() < EPSILON);
        assert!((layout.home_spawn.z + layout.away_spawn.z).abs() < EPSILON);
    }

    #[test]
    fn clamp_position_stays_in_bounds() {
        let layout = MapLayout::default();
        let clamped =
            layout.clamp_position(Vec3::new(layout.max.x + 10.0, 5.0, layout.min.y - 10.0));
        assert!(clamped.x <= layout.max.x + EPSILON);
        assert!(clamped.z >= layout.min.y - EPSILON);
        assert_eq!(clamped.y, 5.0);
    }

    #[test]
    fn terrain_height_covers_pad_top_and_spawn_points() {
        let layout = MapLayout::default();
        let home = layout.home_spawn;
        assert!((layout.terrain_height(home.x, home.z) - BASE_PAD_HEIGHT).abs() < EPSILON);
        for team in [Team::Green, Team::Blue] {
            let spawn = layout.team_spawn(team);
            assert!(
                (layout.terrain_height(spawn.x, spawn.z) - BASE_PAD_HEIGHT).abs() < EPSILON,
                "spawn point must be on the pad top"
            );
        }
    }

    #[test]
    fn terrain_height_ramp_descends_linearly_and_continuously() {
        let layout = MapLayout::default();
        let home = layout.home_spawn;
        let half = BASE_PAD_SIZE * 0.5;
        // Pad edge: still full height (C0-continuous with the top).
        assert!((layout.terrain_height(home.x + half, home.z) - BASE_PAD_HEIGHT).abs() < EPSILON);
        // Mid-ramp: half height.
        let mid = layout.terrain_height(home.x + half + PAD_RAMP_LENGTH * 0.5, home.z);
        assert!((mid - BASE_PAD_HEIGHT * 0.5).abs() < EPSILON);
        // Ramp end: ground level (C0-continuous with open ground).
        assert!(
            layout
                .terrain_height(home.x + half + PAD_RAMP_LENGTH, home.z)
                .abs()
                < EPSILON
        );
        // No step anywhere along the walk-off line.
        let mut previous = layout.terrain_height(home.x, home.z);
        let steps = 200;
        let total = half + PAD_RAMP_LENGTH + 2.0;
        for step in 1..=steps {
            let x = home.x + total * step as f32 / steps as f32;
            let current = layout.terrain_height(x, home.z);
            assert!(
                (current - previous).abs()
                    < BASE_PAD_HEIGHT * total / PAD_RAMP_LENGTH / steps as f32 + EPSILON,
                "terrain must not step discontinuously"
            );
            previous = current;
        }
    }

    #[test]
    fn terrain_height_corner_matches_overlapping_ramps() {
        let layout = MapLayout::default();
        let home = layout.home_spawn;
        let half = BASE_PAD_SIZE * 0.5;
        // 1.0 beyond the X edge, 3.0 beyond the Z edge: the X-side slab is
        // higher and its top is the walkable surface.
        let corner = layout.terrain_height(home.x + half + 1.0, home.z + half + 3.0);
        let expected = BASE_PAD_HEIGHT * (1.0 - 1.0 / PAD_RAMP_LENGTH);
        assert!((corner - expected).abs() < EPSILON);
        // Beyond the ramp reach diagonally: open ground.
        let beyond = layout.terrain_height(
            home.x + half + PAD_RAMP_LENGTH + 0.1,
            home.z + half + PAD_RAMP_LENGTH + 0.1,
        );
        assert!(beyond.abs() < EPSILON);
    }

    #[test]
    fn no_decorative_block_entombs_a_camp_or_boss() {
        let layout = MapLayout::default();
        let spawned = layout.decorative_jungle_block_centers();
        let anchors: Vec<Vec2> = layout
            .camp_centers()
            .into_iter()
            .chain(layout.boss_pit_centers())
            .collect();
        for anchor in &anchors {
            for block in &spawned {
                assert!(
                    block.distance(*anchor) > JUNGLE_BLOCK_CLEARANCE,
                    "block at {block:?} would entomb the creature at {anchor:?}"
                );
            }
        }
        // Exactly the 5 non-hosting blocks remain (3 camps + 2 boss pits
        // each claim one of the 10 block slots).
        assert_eq!(
            spawned.len(),
            layout.jungle_block_centers().len() - anchors.len()
        );
    }

    #[test]
    fn terrain_height_open_ground_is_flat() {
        let layout = MapLayout::default();
        assert_eq!(layout.terrain_height(0.0, 0.0), 0.0);
        let mid_lane = (layout.home_spawn + layout.away_spawn) * 0.5;
        assert_eq!(layout.terrain_height(mid_lane.x, mid_lane.z), 0.0);
    }
}

#[cfg(test)]
mod verdant_surface_tests {
    use super::*;

    #[test]
    fn mitered_pad_samples_cover_edges_sides_and_unequal_corners() {
        let layout = MapLayout::default();
        for center in [layout.home_spawn, layout.away_spawn] {
            for (x, z, expected) in [
                (0.0, 0.0, 0.7),
                (23.0, 0.0, 0.7),
                (26.0, 0.0, 0.35),
                (29.0, 0.0, 0.0),
                (24.0, 26.0, 0.35),
                (26.0, 24.0, 0.35),
                (29.0, 29.0, 0.0),
                (29.1, 29.1, 0.0),
            ] {
                for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                    let actual = layout.terrain_height_3d(center.x + sx * x, center.z + sz * z);
                    assert!((actual - expected).abs() < 0.00001, "{x}, {z}: {actual}");
                }
            }
        }
        // Keep the separately tested legacy Sprite2d corner contract intact.
        let c = layout.home_spawn;
        assert!(
            (layout.terrain_height(c.x + 24.0, c.z + 26.0)
                - layout.terrain_height_3d(c.x + 24.0, c.z + 26.0))
            .abs()
                > 0.2
        );
    }

    #[test]
    fn normalized_crossings_approaches_decks_and_exits_match_within_five_centimeters() {
        let layout = MapLayout::default();
        let edge = layout.max.x - 12.0;
        for delta in [-17.0, -14.0, -9.0, 0.0, 9.0, 14.0, 17.0] {
            let p = Vec2::splat(delta / 2.0_f32.sqrt());
            assert!((layout.terrain_height_3d(p.x, p.y) - 0.02).abs() <= 0.05);
        }
        // Outer lanes turn through square watergates; sample their actual
        // inbound and outbound center lines, not a fictitious diagonal route.
        for center in [Vec2::new(-edge, edge), Vec2::new(edge, -edge)] {
            let sign = center.x.signum();
            for distance in [0.0, 9.0, 12.5, 13.0, 17.0] {
                for offset in [
                    Vec2::new(-sign * distance, 0.0),
                    Vec2::new(0.0, sign * distance),
                ] {
                    let p = center + offset;
                    assert!((layout.terrain_height_3d(p.x, p.y) - 0.02).abs() <= 0.05);
                }
            }
        }
        for base in [layout.home_spawn, layout.away_spawn] {
            let mut previous = 0.7;
            for step in 0..=320 {
                let d = step as f32 * 0.1;
                let current = layout.terrain_height_3d(base.x + d, base.z + d);
                assert!((current - previous).abs() < 0.012);
                previous = current;
            }
        }
    }
}
