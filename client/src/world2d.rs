//! Genuine 2D render-world infrastructure.
//!
//! Gameplay and networking deliberately remain in Bevy's XZ plane.  This
//! module is the only place that translates those coordinates into the XY
//! render plane used by `Camera2d`.

use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

use crate::maps::{LANE_WIDTH, MapLayout, RIVER_WIDTH};
use crate::sprite::PlayerVisualMode;

pub const WORLD_TILE_SIZE: f32 = 4.0;
pub const WORLD_TILE_COLUMNS: usize = 55;
pub const WORLD_TILE_ROWS: usize = 55;
pub const STATIC_WORLD_ENTITY_BUDGET: usize = 4_096;
pub const TRANSIENT_VFX_BUDGET: usize = 256;
pub const TRANSIENT_VFX_MAX_LIFETIME: f32 = 2.0;
const WORLD2D_MANIFEST: &str = include_str!("../assets/world2d/manifest.json");

/// Camera-independent render bands.  Values leave room for stable Y sorting
/// within the actor-facing bands.
pub mod layer {
    pub const GROUND: f32 = -100.0;
    pub const WATER: f32 = -90.0;
    pub const PATH: f32 = -80.0;
    pub const LOW_PROP: f32 = -30.0;
    pub const MARKER: f32 = -5.0;
    pub const ACTOR: f32 = 10.0;
    pub const PROJECTILE: f32 = 30.0;
    pub const VFX: f32 = 40.0;
    pub const OVERHEAD: f32 = 60.0;
}

/// Pure simulation XZ -> render XY mapping.  Simulation height is cosmetic
/// data and is intentionally absent from the result.
#[inline]
pub const fn simulation_xz_to_render_xy(simulation: Vec3) -> Vec2 {
    Vec2::new(simulation.x, simulation.z)
}

/// Inverse XY -> XZ mapping used by pointer input.  The supplied simulation Y
/// is retained so clicks never rewrite gameplay height.
#[inline]
pub const fn render_xy_to_simulation_xz(render: Vec2, simulation_y: f32) -> Vec3 {
    Vec3::new(render.x, simulation_y, render.y)
}

/// Stable top-down sort: lower feet draw in front.  Quantisation prevents
/// snapshot interpolation noise from flickering near-equal actors, and the
/// owner id supplies a deterministic tie-break independent of spawn order.
pub fn y_sorted_z(band: f32, render_y: f32, owner: Entity) -> f32 {
    let quantized_y = (render_y * 8.0).round() / 8.0;
    let tie = (owner.to_bits() & 0x3f) as f32 * 0.000_001;
    band - quantized_y * 0.001 + tie
}

#[derive(Component)]
pub struct World2dStatic;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct World2dCounts {
    pub static_entities: usize,
}

#[derive(Debug, Deserialize)]
struct WorldManifest {
    schema_version: u32,
    tile_world_size: f32,
    atlases: AtlasManifest,
    tiles: HashMap<String, TileDefinition>,
    props: HashMap<String, PropDefinition>,
    topology: TopologyManifest,
    generation: GenerationManifest,
}

#[derive(Debug, Deserialize)]
struct AtlasManifest {
    terrain: AtlasDefinition,
    props: AtlasDefinition,
}

#[derive(Debug, Deserialize)]
struct AtlasDefinition {
    path: String,
    grid: [u32; 2],
    frame_size: [u32; 2],
}

#[derive(Debug, Deserialize)]
struct TileDefinition {
    frame: usize,
}

#[derive(Debug, Deserialize)]
struct PropDefinition {
    frame: usize,
    pivot: [f32; 2],
    world_size: [f32; 2],
}

#[derive(Debug, Deserialize)]
struct TopologyManifest {
    bounds: BoundsManifest,
    bases: TeamPoints,
    spawns: TeamPoints,
    lanes: LaneManifest,
    river: RiverManifest,
    lane_towers: Vec<AnchorManifest>,
    base_objectives: Vec<AnchorManifest>,
    camps: Vec<[f32; 2]>,
    boss_pits: Vec<[f32; 2]>,
}

#[derive(Debug, Deserialize)]
struct LaneManifest {
    mid: Vec<[f32; 2]>,
    top: Vec<[f32; 2]>,
    bot: Vec<[f32; 2]>,
}

#[derive(Debug, Deserialize)]
struct RiverManifest {
    polyline: Vec<[f32; 2]>,
    width: f32,
    traversable: bool,
}

#[derive(Debug, Deserialize)]
struct AnchorManifest {
    xz: [f32; 2],
}

#[derive(Debug, Deserialize)]
struct BoundsManifest {
    min: [f32; 2],
    max: [f32; 2],
}

#[derive(Debug, Deserialize)]
struct TeamPoints {
    green: [f32; 2],
    blue: [f32; 2],
}

#[derive(Debug, Deserialize)]
struct GenerationManifest {
    grid: GridManifest,
}

#[derive(Debug, Deserialize)]
struct GridManifest {
    columns: usize,
    rows: usize,
    static_tile_count: usize,
    max_static_entities: usize,
}

#[derive(Resource, Default)]
struct World2dAssets {
    terrain_image: Handle<Image>,
    terrain_layout: Handle<TextureAtlasLayout>,
    prop_image: Handle<Image>,
    prop_layout: Handle<TextureAtlasLayout>,
    tile_frames: HashMap<String, usize>,
    props: HashMap<String, PropDefinition>,
}

pub struct World2dPlugin;

impl Plugin for World2dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<World2dCounts>()
            .init_resource::<World2dAssets>()
            .add_systems(Startup, (load_world2d_assets, setup_world2d).chain());
    }
}

fn load_world2d_assets(
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    asset_server: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut assets: ResMut<World2dAssets>,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    let manifest = match serde_json::from_str::<WorldManifest>(WORLD2D_MANIFEST) {
        Ok(manifest) if manifest.schema_version == 1 => manifest,
        Ok(manifest) => {
            error!("Unsupported world2d schema {}", manifest.schema_version);
            return;
        }
        Err(error) => {
            error!("Invalid world2d manifest: {error}");
            return;
        }
    };
    let topology_matches = |manifest_point: [f32; 2], actual: Vec2| {
        Vec2::from_array(manifest_point).distance(actual) < 0.001
    };
    let grid = &manifest.generation.grid;
    let topology = &manifest.topology;
    if (manifest.tile_world_size - WORLD_TILE_SIZE).abs() > f32::EPSILON
        || grid.columns != WORLD_TILE_COLUMNS
        || grid.rows != WORLD_TILE_ROWS
        || grid.static_tile_count != WORLD_TILE_COLUMNS * WORLD_TILE_ROWS
        || grid.max_static_entities > STATIC_WORLD_ENTITY_BUDGET
        || !topology_matches(topology.bounds.min, layout.min)
        || !topology_matches(topology.bounds.max, layout.max)
        || !topology_matches(topology.bases.green, layout.home_spawn.xz())
        || !topology_matches(topology.bases.blue, layout.away_spawn.xz())
        || !topology_matches(
            topology.spawns.green,
            layout.team_spawn(crate::team::Team::Green).xz(),
        )
        || !topology_matches(
            topology.spawns.blue,
            layout.team_spawn(crate::team::Team::Blue).xz(),
        )
        || [
            &topology.lanes.mid,
            &topology.lanes.top,
            &topology.lanes.bot,
        ]
        .into_iter()
        .zip(layout.lane_polylines())
        .any(|(manifest_lane, actual_lane)| {
            manifest_lane.len() != actual_lane.len()
                || manifest_lane
                    .iter()
                    .zip(actual_lane)
                    .any(|(manifest_point, actual)| !topology_matches(*manifest_point, actual))
        })
        || topology.river.polyline.len() != 2
        || topology
            .river
            .polyline
            .iter()
            .zip(layout.river_polyline())
            .any(|(manifest_point, actual)| !topology_matches(*manifest_point, actual))
        || (topology.river.width - RIVER_WIDTH).abs() > f32::EPSILON
        || !topology.river.traversable
        || topology.lane_towers.len() != 6
        || topology.base_objectives.len() != 2
        || topology
            .lane_towers
            .iter()
            .chain(&topology.base_objectives)
            .any(|anchor| {
                let point = Vec2::from_array(anchor.xz);
                point.cmplt(layout.min).any() || point.cmpgt(layout.max).any()
            })
        || topology.camps.len() != layout.camp_centers().len()
        || topology.boss_pits.len() != layout.boss_pit_centers().len()
    {
        error!("world2d manifest topology/budget does not match the authoritative client map");
        return;
    }
    let terrain = manifest.atlases.terrain;
    let props = manifest.atlases.props;
    assets.terrain_image = asset_server.load(format!("world2d/{}", terrain.path));
    assets.terrain_layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::from_array(terrain.frame_size),
        terrain.grid[0],
        terrain.grid[1],
        None,
        None,
    ));
    assets.prop_image = asset_server.load(format!("world2d/{}", props.path));
    assets.prop_layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::from_array(props.frame_size),
        props.grid[0],
        props.grid[1],
        None,
        None,
    ));
    assets.tile_frames = manifest
        .tiles
        .into_iter()
        .map(|(id, tile)| (id, tile.frame))
        .collect();
    assets.props = manifest.props;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TileKind {
    Grass,
    Forest,
    Water,
    Path,
    GreenBase,
    BlueBase,
    Camp,
    Boss,
}

fn distance_to_segment(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}

fn near_polyline(point: Vec2, points: &[Vec2], half_width: f32) -> bool {
    points
        .windows(2)
        .any(|pair| distance_to_segment(point, pair[0], pair[1]) <= half_width)
}

fn deterministic_tile_kind(layout: MapLayout, point: Vec2) -> TileKind {
    let home = layout.home_spawn.xz();
    let away = layout.away_spawn.xz();
    if point.distance(home) <= 23.0 {
        return TileKind::GreenBase;
    }
    if point.distance(away) <= 23.0 {
        return TileKind::BlueBase;
    }
    if layout
        .boss_pit_centers()
        .iter()
        .any(|anchor| point.distance(*anchor) <= 9.0)
    {
        return TileKind::Boss;
    }
    if layout
        .camp_centers()
        .iter()
        .any(|anchor| point.distance(*anchor) <= 8.0)
    {
        return TileKind::Camp;
    }
    if layout
        .lane_polylines()
        .iter()
        .any(|lane| near_polyline(point, lane, LANE_WIDTH * 0.5))
    {
        return TileKind::Path;
    }
    let river = layout.river_polyline();
    if distance_to_segment(point, river[0], river[1]) <= RIVER_WIDTH * 0.5 {
        return TileKind::Water;
    }
    if layout
        .decorative_jungle_block_centers()
        .iter()
        .any(|center| point.distance(*center) <= 12.0)
    {
        return TileKind::Forest;
    }
    TileKind::Grass
}

fn tile_color(kind: TileKind, checker: bool) -> Color {
    match kind {
        TileKind::Grass if checker => Color::srgb(0.30, 0.52, 0.28),
        TileKind::Grass => Color::srgb(0.34, 0.57, 0.31),
        TileKind::Forest if checker => Color::srgb(0.17, 0.36, 0.22),
        TileKind::Forest => Color::srgb(0.20, 0.40, 0.24),
        TileKind::Water if checker => Color::srgb(0.24, 0.60, 0.66),
        TileKind::Water => Color::srgb(0.28, 0.66, 0.70),
        TileKind::Path if checker => Color::srgb(0.65, 0.54, 0.36),
        TileKind::Path => Color::srgb(0.70, 0.59, 0.40),
        TileKind::GreenBase => Color::srgb(0.20, 0.62, 0.48),
        TileKind::BlueBase => Color::srgb(0.33, 0.45, 0.78),
        TileKind::Camp => Color::srgb(0.52, 0.43, 0.28),
        TileKind::Boss => Color::srgb(0.38, 0.29, 0.46),
    }
}

fn tile_id(kind: TileKind, variant: usize) -> &'static str {
    match kind {
        TileKind::Grass => ["grass_a", "grass_b", "grass_c", "grass_d"][variant % 4],
        TileKind::Forest => ["forest_floor_a", "forest_floor_b"][variant % 2],
        TileKind::Water => ["water_a", "water_b", "water_c"][variant % 3],
        TileKind::Path => ["path_earth_a", "path_earth_b"][variant % 2],
        TileKind::GreenBase => "green_base",
        TileKind::BlueBase => "blue_base",
        TileKind::Camp => "camp_clearing",
        TileKind::Boss => "boss_clearing",
    }
}

fn tile_manifest_id(
    layout: MapLayout,
    point: Vec2,
    kind: TileKind,
    variant: usize,
    tile_size: Vec2,
) -> &'static str {
    let cardinal = [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y];
    let edge = cardinal
        .into_iter()
        .any(|direction| deterministic_tile_kind(layout, point + direction * tile_size) != kind);
    match kind {
        TileKind::Path => {
            let river = layout.river_polyline();
            if distance_to_segment(point, river[0], river[1]) <= RIVER_WIDTH * 0.5 {
                "stone_crossing"
            } else if edge {
                "path_edge"
            } else {
                tile_id(kind, variant)
            }
        }
        TileKind::Water if edge => "water_edge",
        TileKind::Forest if edge => "forest_edge",
        TileKind::Grass if variant.is_multiple_of(29) => "grass_flowers",
        TileKind::Boss if variant.is_multiple_of(2) => "objective_gold",
        _ => tile_id(kind, variant),
    }
}

fn atlas_sprite(
    image: &Handle<Image>,
    layout: &Handle<TextureAtlasLayout>,
    frame: usize,
    size: Vec2,
) -> Sprite {
    let mut sprite = Sprite::from_atlas_image(
        image.clone(),
        TextureAtlas {
            layout: layout.clone(),
            index: frame,
        },
    );
    sprite.custom_size = Some(size);
    sprite
}

fn tile_layer(kind: TileKind) -> f32 {
    match kind {
        TileKind::Water => layer::WATER,
        TileKind::Path | TileKind::GreenBase | TileKind::BlueBase => layer::PATH,
        _ => layer::GROUND,
    }
}

fn setup_world2d(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    layout: Res<MapLayout>,
    mut counts: ResMut<World2dCounts>,
    assets: Res<World2dAssets>,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }

    let size = layout.size();
    let tile_size = Vec2::new(
        size.x / WORLD_TILE_COLUMNS as f32,
        size.y / WORLD_TILE_ROWS as f32,
    );
    debug_assert!((tile_size.x - WORLD_TILE_SIZE).abs() < 0.25);
    let mut spawned = 0usize;
    for row in 0..WORLD_TILE_ROWS {
        for column in 0..WORLD_TILE_COLUMNS {
            let point = Vec2::new(
                layout.min.x + (column as f32 + 0.5) * tile_size.x,
                layout.min.y + (row as f32 + 0.5) * tile_size.y,
            );
            let kind = deterministic_tile_kind(*layout, point);
            let variant = row.wrapping_mul(31) ^ column.wrapping_mul(17);
            let id = tile_manifest_id(*layout, point, kind, variant, tile_size);
            let sprite = assets.tile_frames.get(id).map_or_else(
                || {
                    Sprite::from_color(
                        tile_color(kind, (row + column) % 2 == 0),
                        tile_size + Vec2::splat(0.02),
                    )
                },
                |frame| {
                    atlas_sprite(
                        &assets.terrain_image,
                        &assets.terrain_layout,
                        *frame,
                        tile_size + Vec2::splat(0.02),
                    )
                },
            );
            commands.spawn((
                sprite,
                Transform::from_xyz(point.x, point.y, tile_layer(kind)),
                World2dStatic,
                Name::new(format!("World2dTile-{column}-{row}")),
            ));
            spawned += 1;
            if kind == TileKind::Forest && (row + column) % 2 == 0 {
                let prop_id = if (row * 7 + column) % 3 == 0 {
                    "tree_oak_b"
                } else {
                    "tree_oak_a"
                };
                let prop_sprite = assets.props.get(prop_id).map_or_else(
                    || {
                        Sprite::from_color(
                            Color::srgb(0.10, 0.27, 0.15),
                            Vec2::new(tile_size.x * 0.72, tile_size.y * 1.10),
                        )
                    },
                    |prop| {
                        atlas_sprite(
                            &assets.prop_image,
                            &assets.prop_layout,
                            prop.frame,
                            Vec2::from_array(prop.world_size),
                        )
                    },
                );
                commands.spawn((
                    prop_sprite,
                    Transform::from_xyz(
                        point.x,
                        point.y + tile_size.y * 0.18,
                        y_sorted_z(
                            layer::LOW_PROP,
                            point.y,
                            Entity::from_bits((row * WORLD_TILE_COLUMNS + column) as u64 + 1),
                        ),
                    ),
                    World2dStatic,
                    Name::new(format!("World2dTree-{column}-{row}")),
                ));
                spawned += 1;
            }
        }
    }

    for (index, (position, prop_id)) in layout
        .camp_centers()
        .into_iter()
        .map(|p| (p, "camp_totem"))
        .chain(
            layout
                .boss_pit_centers()
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    (
                        p,
                        if i == 0 {
                            "wendigo_shrine"
                        } else {
                            "boss_runes"
                        },
                    )
                }),
        )
        .chain([
            (layout.home_spawn.xz(), "green_base_lantern"),
            (layout.away_spawn.xz(), "blue_base_obelisk"),
        ])
        .enumerate()
    {
        if let Some(prop) = assets.props.get(prop_id) {
            let pivot_offset = Vec2::new(
                (0.5 - prop.pivot[0]) * prop.world_size[0],
                (0.5 - prop.pivot[1]) * prop.world_size[1],
            );
            commands.spawn((
                atlas_sprite(
                    &assets.prop_image,
                    &assets.prop_layout,
                    prop.frame,
                    Vec2::from_array(prop.world_size),
                ),
                Transform::from_xyz(
                    position.x + pivot_offset.x,
                    position.y + pivot_offset.y,
                    y_sorted_z(
                        layer::LOW_PROP,
                        position.y,
                        Entity::from_bits(60_000 + index as u64),
                    ),
                ),
                World2dStatic,
                Name::new(format!("World2dAnchorProp-{prop_id}-{index}")),
            ));
            spawned += 1;
        }
    }

    assert!(spawned <= STATIC_WORLD_ENTITY_BUDGET);
    counts.static_entities = spawned;
    info!("Spawned deterministic 2D world: {spawned} static tile entities");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_round_trips_map_corners_and_negative_points() {
        let layout = MapLayout::default();
        for point in [
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(-17.25, 3.0, 42.75),
            Vec3::new(layout.min.x, -2.0, layout.min.y),
            Vec3::new(layout.max.x, 9.0, layout.max.y),
        ] {
            let render = simulation_xz_to_render_xy(point);
            let round_trip = render_xy_to_simulation_xz(render, point.y);
            assert_eq!(round_trip, point);
        }
        assert_eq!(
            simulation_xz_to_render_xy(Vec3::new(2.0, 99.0, 7.0)),
            Vec2::new(2.0, 7.0)
        );
    }

    #[test]
    fn deterministic_world_covers_required_topology() {
        let layout = MapLayout::default();
        assert_eq!(
            deterministic_tile_kind(layout, layout.home_spawn.xz()),
            TileKind::GreenBase
        );
        assert_eq!(
            deterministic_tile_kind(layout, layout.away_spawn.xz()),
            TileKind::BlueBase
        );
        for camp in layout.camp_centers() {
            assert_eq!(deterministic_tile_kind(layout, camp), TileKind::Camp);
        }
        for boss in layout.boss_pit_centers() {
            assert_eq!(deterministic_tile_kind(layout, boss), TileKind::Boss);
        }
        for lane in layout.lane_polylines() {
            assert!(lane.windows(2).any(|pair| {
                deterministic_tile_kind(layout, (pair[0] + pair[1]) * 0.5) == TileKind::Path
            }));
        }
        assert_eq!(
            tile_manifest_id(
                layout,
                Vec2::ZERO,
                TileKind::Path,
                0,
                Vec2::splat(WORLD_TILE_SIZE),
            ),
            "stone_crossing"
        );
        const { assert!(WORLD_TILE_COLUMNS * WORLD_TILE_ROWS <= STATIC_WORLD_ENTITY_BUDGET) };
    }

    #[test]
    fn manifest_matches_authoritative_layout_and_budget() {
        let manifest: WorldManifest = serde_json::from_str(WORLD2D_MANIFEST).unwrap();
        let layout = MapLayout::default();
        assert_eq!(manifest.schema_version, 1);
        assert!((manifest.tile_world_size - WORLD_TILE_SIZE).abs() < f32::EPSILON);
        assert_eq!(manifest.generation.grid.columns, WORLD_TILE_COLUMNS);
        assert_eq!(manifest.generation.grid.rows, WORLD_TILE_ROWS);
        assert_eq!(
            manifest.generation.grid.static_tile_count,
            WORLD_TILE_COLUMNS * WORLD_TILE_ROWS
        );
        assert!(manifest.generation.grid.max_static_entities <= STATIC_WORLD_ENTITY_BUDGET);
        for (actual, expected) in [
            (manifest.topology.bounds.min, layout.min.to_array()),
            (manifest.topology.bounds.max, layout.max.to_array()),
            (
                manifest.topology.bases.green,
                layout.home_spawn.xz().to_array(),
            ),
            (
                manifest.topology.bases.blue,
                layout.away_spawn.xz().to_array(),
            ),
            (
                manifest.topology.spawns.green,
                layout.team_spawn(crate::team::Team::Green).xz().to_array(),
            ),
            (
                manifest.topology.spawns.blue,
                layout.team_spawn(crate::team::Team::Blue).xz().to_array(),
            ),
        ] {
            assert!(Vec2::from_array(actual).distance(Vec2::from_array(expected)) < 0.001);
        }
        assert_eq!(manifest.topology.camps.len(), 3);
        assert_eq!(manifest.topology.boss_pits.len(), 2);
        for id in [
            "grass_a",
            "grass_flowers",
            "forest_floor_a",
            "forest_edge",
            "path_earth_a",
            "path_edge",
            "water_a",
            "water_edge",
            "stone_crossing",
            "green_base",
            "blue_base",
            "camp_clearing",
            "boss_clearing",
            "objective_gold",
        ] {
            assert!(manifest.tiles.contains_key(id), "runtime tile {id} missing");
        }
    }

    #[test]
    fn sorting_is_stable_and_layered() {
        let a = Entity::from_bits(10);
        let b = Entity::from_bits(11);
        let near_a = y_sorted_z(layer::ACTOR, 3.000_01, a);
        let near_a_again = y_sorted_z(layer::ACTOR, 3.000_02, a);
        assert_eq!(near_a, near_a_again);
        assert_ne!(near_a, y_sorted_z(layer::ACTOR, 3.0, b));
        const {
            assert!(layer::PROJECTILE > layer::ACTOR);
            assert!(layer::OVERHEAD > layer::VFX);
            assert!(layer::LOW_PROP > layer::PATH);
        }
    }
}
