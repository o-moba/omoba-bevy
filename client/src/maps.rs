use bevy::prelude::*;

use crate::player::{PLAYER_SIZE, PLAYER_SPEED};
use crate::sprite::PlayerVisualMode;
use crate::team::Team;

pub const TARGET_BASE_RUN_TIME_SECONDS: f32 = 45.0;
pub const TARGET_BASE_DISTANCE: f32 = PLAYER_SPEED * TARGET_BASE_RUN_TIME_SECONDS;
pub(crate) const BASE_PAD_SIZE: f32 = 46.0;
pub(crate) const BASE_PAD_HEIGHT: f32 = 0.7;
/// Horizontal length of the walk-up ramps around each base pad. Characters
/// ascend/descend over this distance (League-style client-side fake: the
/// server keeps a flat ground plane, only the rendered height changes).
pub(crate) const PAD_RAMP_LENGTH: f32 = 6.0;
const PAD_RAMP_THICKNESS: f32 = 0.12;
/// Blocks whose center is within this distance of a neutral-camp or
/// boss-pit anchor get no decorative box (the creature must be visible).
const JUNGLE_BLOCK_CLEARANCE: f32 = 10.0;
const BASE_EDGE_MARGIN: f32 = 6.0;
const PLAYER_SPAWN_OFFSET: f32 = 7.0;
pub(crate) const LANE_WIDTH: f32 = 12.0;
const LANE_THICKNESS: f32 = 0.2;
const LANE_EDGE_PADDING: f32 = 6.0;
pub(crate) const RIVER_WIDTH: f32 = 18.0;
const RIVER_THICKNESS: f32 = 0.06;
/// Fraction of the map size where the outer jungle blocks/camps sit
/// (mirrors `JUNGLE_MAP_OUTER_FRAC` in server/src/balance.rs).
const JUNGLE_MAP_OUTER_FRAC: f32 = 0.34;
/// Fraction of the map size for the inner jungle blocks/camps
/// (mirrors `JUNGLE_MAP_INNER_FRAC` in server/src/balance.rs).
const JUNGLE_MAP_INNER_FRAC: f32 = 0.22;
/// Fraction of the map size for the two mid jungle blocks.
const JUNGLE_MAP_MID_FRAC: f32 = 0.28;

#[derive(Component)]
pub struct MapStatic;

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
    /// These are the exact control points rendered by `setup_moba_map` and
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

    /// XZ centers of the ten visual jungle blocks spawned by `setup_moba_map`.
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

    /// Jungle blocks that actually get a decorative box: blocks hosting a
    /// neutral camp or a boss pit are skipped so the creatures stand in
    /// open clearings instead of being entombed inside the geometry.
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

pub struct MapsPlugin;

impl Plugin for MapsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLayout>()
            .add_systems(Startup, setup_moba_map)
            .add_systems(Update, apply_map_visual_mode);
    }
}

#[derive(Resource)]
struct MapPresentationMaterials {
    terrain: Handle<StandardMaterial>,
    lane: Handle<StandardMaterial>,
    river: Handle<StandardMaterial>,
    home_base: Handle<StandardMaterial>,
    away_base: Handle<StandardMaterial>,
    prop: Handle<StandardMaterial>,
    arena_texture: Handle<Image>,
}

fn setup_moba_map(
    mut commands: Commands,
    layout: Res<MapLayout>,
    visual_mode: Res<PlayerVisualMode>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let layout = *layout;
    let map_size = layout.size();

    info!(
        "MOBA map spawned: center lane {:.1} units (~{:.1}s at {:.1} u/s)",
        layout.center_lane_distance(),
        layout.center_lane_distance() / PLAYER_SPEED,
        PLAYER_SPEED
    );

    let is_2d = *visual_mode == PlayerVisualMode::Sprite2d;
    // The genuine 2D world is owned by `World2dPlugin`; none of the legacy
    // Mesh3d arena is spawned into a sprite2d session.
    if is_2d {
        return;
    }
    let arena_texture = asset_server.load("presentation2d/arena.png");
    let terrain_material = materials.add(StandardMaterial {
        base_color: if is_2d {
            Color::srgb(0.20, 0.30, 0.25)
        } else {
            Color::srgb(0.08, 0.22, 0.10)
        },
        base_color_texture: is_2d.then(|| arena_texture.clone()),
        unlit: is_2d,
        perceptual_roughness: 0.95,
        ..default()
    });
    let lane_material = materials.add(StandardMaterial {
        base_color: if is_2d {
            Color::srgb(0.48, 0.42, 0.32)
        } else {
            Color::srgb(0.38, 0.34, 0.28)
        },
        unlit: is_2d,
        perceptual_roughness: 0.8,
        ..default()
    });
    let river_material = materials.add(StandardMaterial {
        base_color: if is_2d {
            Color::srgb(0.08, 0.28, 0.36)
        } else {
            Color::srgb(0.10, 0.22, 0.40)
        },
        unlit: is_2d,
        metallic: 0.15,
        perceptual_roughness: 0.2,
        ..default()
    });
    let home_base_material = materials.add(StandardMaterial {
        base_color: if is_2d {
            Color::srgb(0.10, 0.62, 0.68)
        } else {
            Color::srgb(0.12, 0.35, 0.62)
        },
        unlit: is_2d,
        perceptual_roughness: 0.75,
        ..default()
    });
    let away_base_material = materials.add(StandardMaterial {
        base_color: if is_2d {
            Color::srgb(0.78, 0.22, 0.28)
        } else {
            Color::srgb(0.58, 0.18, 0.18)
        },
        unlit: is_2d,
        perceptual_roughness: 0.75,
        ..default()
    });
    let prop_material = materials.add(StandardMaterial {
        base_color: if is_2d {
            Color::srgb(0.16, 0.25, 0.22)
        } else {
            Color::srgb(0.18, 0.26, 0.16)
        },
        unlit: is_2d,
        perceptual_roughness: 0.9,
        ..default()
    });

    commands.insert_resource(MapPresentationMaterials {
        terrain: terrain_material.clone(),
        lane: lane_material.clone(),
        river: river_material.clone(),
        home_base: home_base_material.clone(),
        away_base: away_base_material.clone(),
        prop: prop_material.clone(),
        arena_texture,
    });

    commands.spawn((
        Mesh3d(meshes.add(Mesh::from(
            Plane3d::default().mesh().size(map_size.x, map_size.y),
        ))),
        MeshMaterial3d(terrain_material),
        Transform::from_xyz(0.0, 0.0, 0.0),
        MapStatic,
        Name::new("MapTerrain"),
    ));

    let [mid_lane_points, top_lane_points, bot_lane_points] = layout.lane_polylines();
    spawn_lane_polyline(
        &mut commands,
        &mut meshes,
        &lane_material,
        &mid_lane_points,
        LANE_WIDTH,
        LANE_THICKNESS,
        "LaneMid",
    );
    spawn_lane_polyline(
        &mut commands,
        &mut meshes,
        &lane_material,
        &top_lane_points,
        LANE_WIDTH,
        LANE_THICKNESS,
        "LaneTop",
    );
    spawn_lane_polyline(
        &mut commands,
        &mut meshes,
        &lane_material,
        &bot_lane_points,
        LANE_WIDTH,
        LANE_THICKNESS,
        "LaneBot",
    );

    spawn_lane_polyline(
        &mut commands,
        &mut meshes,
        &river_material,
        &layout.river_polyline(),
        RIVER_WIDTH,
        RIVER_THICKNESS,
        "River",
    );

    spawn_box(
        &mut commands,
        &mut meshes,
        &home_base_material,
        Vec3::new(
            layout.home_spawn.x,
            BASE_PAD_HEIGHT * 0.5,
            layout.home_spawn.z,
        ),
        Vec3::new(BASE_PAD_SIZE, BASE_PAD_HEIGHT, BASE_PAD_SIZE),
        "HomeBasePad",
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        &away_base_material,
        Vec3::new(
            layout.away_spawn.x,
            BASE_PAD_HEIGHT * 0.5,
            layout.away_spawn.z,
        ),
        Vec3::new(BASE_PAD_SIZE, BASE_PAD_HEIGHT, BASE_PAD_SIZE),
        "AwayBasePad",
    );
    spawn_pad_ramps(
        &mut commands,
        &mut meshes,
        &home_base_material,
        Vec2::new(layout.home_spawn.x, layout.home_spawn.z),
        "HomeBasePad",
    );
    spawn_pad_ramps(
        &mut commands,
        &mut meshes,
        &away_base_material,
        Vec2::new(layout.away_spawn.x, layout.away_spawn.z),
        "AwayBasePad",
    );

    // Camp/boss-hosting blocks are skipped: those creatures must be visible
    // in open clearings, not entombed inside decorative boxes.
    for (idx, center) in layout
        .decorative_jungle_block_centers()
        .iter()
        .copied()
        .enumerate()
    {
        spawn_box(
            &mut commands,
            &mut meshes,
            &prop_material,
            Vec3::new(center.x, 2.0, center.y),
            Vec3::new(12.0, 4.0, 12.0),
            &format!("JungleBlock-{idx}"),
        );
    }

    spawn_box(
        &mut commands,
        &mut meshes,
        &prop_material,
        Vec3::new(0.0, 1.5, layout.max.y + 1.0),
        Vec3::new(map_size.x, 3.0, 2.0),
        "NorthWall",
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        &prop_material,
        Vec3::new(0.0, 1.5, layout.min.y - 1.0),
        Vec3::new(map_size.x, 3.0, 2.0),
        "SouthWall",
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        &prop_material,
        Vec3::new(layout.min.x - 1.0, 1.5, 0.0),
        Vec3::new(2.0, 3.0, map_size.y),
        "WestWall",
    );
    spawn_box(
        &mut commands,
        &mut meshes,
        &prop_material,
        Vec3::new(layout.max.x + 1.0, 1.5, 0.0),
        Vec3::new(2.0, 3.0, map_size.y),
        "EastWall",
    );
}

fn apply_map_visual_mode(
    mode: Res<PlayerVisualMode>,
    handles: Option<Res<MapPresentationMaterials>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut previous: Local<Option<PlayerVisualMode>>,
) {
    if previous.is_some_and(|previous| previous == *mode) {
        return;
    }
    *previous = Some(*mode);
    let Some(handles) = handles else {
        return;
    };
    let is_2d = *mode == PlayerVisualMode::Sprite2d;

    let updates = [
        (
            &handles.terrain,
            Color::srgb(0.08, 0.22, 0.10),
            Color::srgb(0.20, 0.30, 0.25),
        ),
        (
            &handles.lane,
            Color::srgb(0.38, 0.34, 0.28),
            Color::srgb(0.48, 0.42, 0.32),
        ),
        (
            &handles.river,
            Color::srgb(0.10, 0.22, 0.40),
            Color::srgb(0.08, 0.28, 0.36),
        ),
        (
            &handles.home_base,
            Color::srgb(0.12, 0.35, 0.62),
            Color::srgb(0.10, 0.62, 0.68),
        ),
        (
            &handles.away_base,
            Color::srgb(0.58, 0.18, 0.18),
            Color::srgb(0.78, 0.22, 0.28),
        ),
        (
            &handles.prop,
            Color::srgb(0.18, 0.26, 0.16),
            Color::srgb(0.16, 0.25, 0.22),
        ),
    ];
    for (handle, color_3d, color_2d) in updates {
        if let Some(material) = materials.get_mut(handle) {
            material.base_color = if is_2d { color_2d } else { color_3d };
            material.unlit = is_2d;
        }
    }
    if let Some(terrain) = materials.get_mut(&handles.terrain) {
        terrain.base_color_texture = is_2d.then(|| handles.arena_texture.clone());
    }
}

fn spawn_lane_polyline(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    points: &[Vec2],
    width: f32,
    thickness: f32,
    name_prefix: &str,
) {
    for (idx, pair) in points.windows(2).enumerate() {
        let start = pair[0];
        let end = pair[1];
        let delta = end - start;
        let segment_length = delta.length();
        if segment_length < 0.001 {
            continue;
        }

        let center = (start + end) * 0.5;
        let yaw = delta.x.atan2(delta.y);
        commands.spawn((
            Mesh3d(meshes.add(Mesh::from(Cuboid::new(width, thickness, segment_length)))),
            MeshMaterial3d(material.clone()),
            Transform {
                translation: Vec3::new(center.x, thickness * 0.5, center.y),
                rotation: Quat::from_rotation_y(yaw),
                ..default()
            },
            MapStatic,
            Name::new(format!("{name_prefix}-{idx}")),
        ));
    }
}

/// Four thin sloped slabs connecting a base pad's top edges to the ground.
/// Their walkable tops match `MapLayout::terrain_height` exactly: each slab
/// spans the pad side plus both corner extensions, and overlapping slabs at
/// the corners form the max-of-ramps surface the height function returns.
fn spawn_pad_ramps(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    pad_center: Vec2,
    name_prefix: &str,
) {
    let half = BASE_PAD_SIZE * 0.5;
    let slope_len = (PAD_RAMP_LENGTH * PAD_RAMP_LENGTH + BASE_PAD_HEIGHT * BASE_PAD_HEIGHT).sqrt();
    let span = BASE_PAD_SIZE + 2.0 * PAD_RAMP_LENGTH;
    let angle = BASE_PAD_HEIGHT.atan2(PAD_RAMP_LENGTH);
    let mid = half + PAD_RAMP_LENGTH * 0.5;
    // Slab center sits half a thickness below the walk line so the top face
    // is what characters stand on.
    let center_y = BASE_PAD_HEIGHT * 0.5 - PAD_RAMP_THICKNESS * 0.5;

    let x_mesh = meshes.add(Mesh::from(Cuboid::new(slope_len, PAD_RAMP_THICKNESS, span)));
    let z_mesh = meshes.add(Mesh::from(Cuboid::new(span, PAD_RAMP_THICKNESS, slope_len)));

    let ramps = [
        (
            Vec3::new(mid, 0.0, 0.0),
            Quat::from_rotation_z(-angle),
            &x_mesh,
            "East",
        ),
        (
            Vec3::new(-mid, 0.0, 0.0),
            Quat::from_rotation_z(angle),
            &x_mesh,
            "West",
        ),
        (
            Vec3::new(0.0, 0.0, mid),
            Quat::from_rotation_x(angle),
            &z_mesh,
            "South",
        ),
        (
            Vec3::new(0.0, 0.0, -mid),
            Quat::from_rotation_x(-angle),
            &z_mesh,
            "North",
        ),
    ];
    for (offset, rotation, mesh, side) in ramps {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(Vec3::new(
                pad_center.x + offset.x,
                center_y,
                pad_center.y + offset.z,
            ))
            .with_rotation(rotation),
            MapStatic,
            Name::new(format!("{name_prefix}-Ramp-{side}")),
        ));
    }
}

fn spawn_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    center: Vec3,
    size: Vec3,
    name: &str,
) {
    commands.spawn((
        Mesh3d(meshes.add(Mesh::from(Cuboid::new(size.x, size.y, size.z)))),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(center),
        MapStatic,
        Name::new(name.to_owned()),
    ));
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
