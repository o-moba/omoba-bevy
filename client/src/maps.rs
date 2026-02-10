use bevy::prelude::*;

use crate::player::{PLAYER_SIZE, PLAYER_SPEED};

pub const TARGET_BASE_RUN_TIME_SECONDS: f32 = 45.0;
pub const TARGET_BASE_DISTANCE: f32 = PLAYER_SPEED * TARGET_BASE_RUN_TIME_SECONDS;
const BASE_PAD_SIZE: f32 = 46.0;
const BASE_PAD_HEIGHT: f32 = 0.7;
const BASE_EDGE_MARGIN: f32 = 6.0;
const LANE_WIDTH: f32 = 12.0;
const LANE_THICKNESS: f32 = 0.2;
const LANE_EDGE_PADDING: f32 = 6.0;
const RIVER_WIDTH: f32 = 18.0;
const RIVER_THICKNESS: f32 = 0.06;

#[derive(Component)]
pub struct MapStatic;

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

    pub fn center_lane_distance(self) -> f32 {
        Vec2::new(
            self.away_spawn.x - self.home_spawn.x,
            self.away_spawn.z - self.home_spawn.z,
        )
        .length()
    }
}

pub struct MapsPlugin;

impl Plugin for MapsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLayout>()
            .add_systems(Startup, setup_moba_map);
    }
}

fn setup_moba_map(
    mut commands: Commands,
    layout: Res<MapLayout>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let layout = *layout;
    let map_size = layout.size();
    let lane_edge_offset = LANE_EDGE_PADDING + LANE_WIDTH * 0.5;
    let left_x = layout.min.x + lane_edge_offset;
    let right_x = layout.max.x - lane_edge_offset;
    let top_z = layout.max.y - lane_edge_offset;
    let bottom_z = layout.min.y + lane_edge_offset;
    let home = Vec3::new(layout.home_spawn.x, 0.0, layout.home_spawn.z);
    let away = Vec3::new(layout.away_spawn.x, 0.0, layout.away_spawn.z);

    info!(
        "MOBA map spawned: center lane {:.1} units (~{:.1}s at {:.1} u/s)",
        layout.center_lane_distance(),
        layout.center_lane_distance() / PLAYER_SPEED,
        PLAYER_SPEED
    );

    let terrain_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.22, 0.10),
        perceptual_roughness: 0.95,
        ..default()
    });
    let lane_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.38, 0.34, 0.28),
        perceptual_roughness: 0.8,
        ..default()
    });
    let river_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.10, 0.22, 0.40),
        metallic: 0.15,
        perceptual_roughness: 0.2,
        ..default()
    });
    let home_base_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.35, 0.62),
        perceptual_roughness: 0.75,
        ..default()
    });
    let away_base_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.58, 0.18, 0.18),
        perceptual_roughness: 0.75,
        ..default()
    });
    let prop_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.18, 0.26, 0.16),
        perceptual_roughness: 0.9,
        ..default()
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

    let mid_lane_points = [home, away];
    spawn_lane_polyline(
        &mut commands,
        &mut meshes,
        &lane_material,
        &mid_lane_points,
        LANE_WIDTH,
        LANE_THICKNESS,
        "LaneMid",
    );

    let top_lane_points = [
        home,
        Vec3::new(left_x, 0.0, home.z),
        Vec3::new(left_x, 0.0, top_z),
        Vec3::new(right_x, 0.0, top_z),
        Vec3::new(away.x, 0.0, top_z),
        away,
    ];
    let bot_lane_points = [
        home,
        Vec3::new(home.x, 0.0, bottom_z),
        Vec3::new(left_x, 0.0, bottom_z),
        Vec3::new(right_x, 0.0, bottom_z),
        Vec3::new(right_x, 0.0, away.z),
        away,
    ];
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

    let river_points = [
        Vec3::new(left_x, 0.0, top_z),
        Vec3::new(right_x, 0.0, bottom_z),
    ];
    spawn_lane_polyline(
        &mut commands,
        &mut meshes,
        &river_material,
        &river_points,
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

    let jungle_outer = map_size.x * 0.34;
    let jungle_inner = map_size.x * 0.22;
    let jungle_mid = map_size.x * 0.28;
    let jungle_blocks = [
        Vec3::new(-jungle_outer, 2.0, jungle_inner),
        Vec3::new(-jungle_inner, 2.0, jungle_outer),
        Vec3::new(-jungle_outer, 2.0, -jungle_inner),
        Vec3::new(-jungle_inner, 2.0, -jungle_outer),
        Vec3::new(jungle_outer, 2.0, jungle_inner),
        Vec3::new(jungle_inner, 2.0, jungle_outer),
        Vec3::new(jungle_outer, 2.0, -jungle_inner),
        Vec3::new(jungle_inner, 2.0, -jungle_outer),
        Vec3::new(0.0, 2.0, jungle_mid),
        Vec3::new(0.0, 2.0, -jungle_mid),
    ];
    for (idx, position) in jungle_blocks.iter().copied().enumerate() {
        spawn_box(
            &mut commands,
            &mut meshes,
            &prop_material,
            position,
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

fn spawn_lane_polyline(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
    points: &[Vec3],
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
        let yaw = delta.x.atan2(delta.z);
        commands.spawn((
            Mesh3d(meshes.add(Mesh::from(Cuboid::new(width, thickness, segment_length)))),
            MeshMaterial3d(material.clone()),
            Transform {
                translation: Vec3::new(center.x, thickness * 0.5, center.z),
                rotation: Quat::from_rotation_y(yaw),
                ..default()
            },
            MapStatic,
            Name::new(format!("{name_prefix}-{idx}")),
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
}
