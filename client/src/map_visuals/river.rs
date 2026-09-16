//! Replace the exported stacked river planes with a single joined surface.
//!
//! The runtime export raises the jade shallows to exactly Y=0, the same
//! plane as the lane beds at the corner crossings. Depth bias would only
//! disguise that intersection. Adjacent color bands share an edge instead
//! of overlapping, and every water vertex stays below the walking surfaces.
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};

use crate::{maps::MapLayout, sprite::PlayerVisualMode, verdant3d::VerdantEnvironment};

const WATER_Y: f32 = -0.035;
const MEADOW_Y: f32 = -0.02;
const AUTHORED_BANK_Y: f32 = -0.04;
const HIGHLIGHT_LOWERING: f32 = 0.035;

#[derive(Component)]
struct RiverAdjusted;

#[derive(Component)]
struct PendingBank;

#[derive(Component)]
struct RiverReplacement(Entity);

pub(super) fn register(app: &mut App) {
    app.add_systems(
        PostUpdate,
        (repair_river, remove_orphaned_replacements)
            .chain()
            .before(bevy::transform::TransformSystems::Propagate),
    );
}

fn environment_ancestor(
    mut entity: Entity,
    parents: &Query<&ChildOf>,
    environments: &Query<(), With<VerdantEnvironment>>,
) -> bool {
    for _ in 0..32 {
        if environments.contains(entity) {
            return true;
        }
        let Ok(parent) = parents.get(entity) else {
            return false;
        };
        entity = parent.parent();
    }
    false
}

fn repair_river(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    layout: Option<Res<MapLayout>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    nodes: Query<
        (Entity, &Name, &Transform, &ChildOf),
        (
            Without<RiverAdjusted>,
            Without<Mesh3d>,
            Or<(Added<Name>, With<PendingBank>)>,
        ),
    >,
    mesh_nodes: Query<&Mesh3d>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    environments: Query<(), With<VerdantEnvironment>>,
) {
    if *mode != PlayerVisualMode::Models3d {
        return;
    }
    let Some(layout) = layout else { return };
    for (entity, name, transform, parent) in &nodes {
        let name = name.as_str();
        let channel = name == "River / turquoise channel";
        let shallows = name.starts_with("River / jade shallows");
        let highlight = name.starts_with("River / current highlight");
        let bank = name.starts_with("Landscape / eroded bank ");
        if !(channel || shallows || highlight || bank)
            || !environment_ancestor(entity, &parents, &environments)
        {
            continue;
        }
        if channel {
            // The exported node's +0.585 translation lifted its original
            // -0.6 geometry. This sibling uses world-space XZ and the final
            // water datum directly, inheriting only the scene's transform.
            commands.spawn((
                Mesh3d(meshes.add(water_mesh(&layout))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 0.3,
                    metallic: 0.15,
                    alpha_mode: AlphaMode::Opaque,
                    ..default()
                })),
                Transform::IDENTITY,
                ChildOf(parent.parent()),
                RiverReplacement(entity),
                Name::new("River / continuous joined surface"),
            ));
        }
        if channel || shallows {
            commands.entity(entity).insert(Visibility::Hidden);
        } else if highlight {
            let mut lowered = *transform;
            lowered.translation.y -= HIGHLIGHT_LOWERING;
            commands.entity(entity).insert(lowered);
        } else if bank {
            // Bridge the export's 2 cm meadow/bank crack, preserving all
            // lower bank geometry, UVs, material choices and normals.
            let mut descendants = vec![entity];
            let mut replacements = Vec::new();
            let mut ready = true;
            while let Some(child) = descendants.pop() {
                if let Ok(children) = children.get(child) {
                    descendants.extend(children.iter());
                }
                if let Ok(handle) = mesh_nodes.get(child) {
                    if let Some(mesh) = meshes.get(&handle.0) {
                        let mut mesh = mesh.clone();
                        close_bank_top(&mut mesh);
                        replacements.push((child, mesh));
                    } else {
                        ready = false;
                    }
                }
            }
            if !ready || replacements.is_empty() {
                commands.entity(entity).insert(PendingBank);
                continue;
            }
            for (child, mesh) in replacements {
                commands.entity(child).insert(Mesh3d(meshes.add(mesh)));
            }
        }
        commands
            .entity(entity)
            .insert(RiverAdjusted)
            .remove::<PendingBank>();
    }
}

fn remove_orphaned_replacements(
    mut commands: Commands,
    replacements: Query<(Entity, &RiverReplacement)>,
    sources: Query<(), With<RiverAdjusted>>,
) {
    // Scene hot reload removes the old authored nodes but can retain their
    // parent. Do not leave a second coplanar river behind on the next load.
    for (entity, source) in &replacements {
        if !sources.contains(source.0) {
            commands.entity(entity).despawn();
        }
    }
}

fn close_bank_top(mesh: &mut Mesh) {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        return;
    };
    for point in positions {
        if (point[1] - AUTHORED_BANK_Y).abs() < 0.0001 {
            point[1] = MEADOW_Y;
        }
    }
}

/// Clip a convex polygon to dot(normal, point) <= distance. The two adjacent
/// bands use the same intersection equation, including the square map tips.
fn clip_half_plane(polygon: Vec<Vec2>, normal: Vec2, distance: f32) -> Vec<Vec2> {
    let Some(mut previous) = polygon.last().copied() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for current in polygon {
        let a = normal.dot(previous) - distance;
        let b = normal.dot(current) - distance;
        if (a <= 0.0) != (b <= 0.0) {
            result.push(previous.lerp(current, a / (a - b)));
        }
        if b <= 0.0 {
            result.push(current);
        }
        previous = current;
    }
    result.dedup_by(|a, b| a.distance_squared(*b) < 0.000_000_1);
    if result.len() > 1 && result[0].distance_squared(*result.last().unwrap()) < 0.000_000_1 {
        result.pop();
    }
    result
}

fn water_mesh(layout: &MapLayout) -> Mesh {
    let [start, end] = layout.river_polyline();
    let direction = (end - start).normalize();
    let across = Vec2::new(-direction.y, direction.x);
    let datum = start.dot(across);
    let half = crate::maps::RIVER_WIDTH * 0.5;
    let offsets = [-half, -half + 2.0, 0.0, half - 2.0, half];
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for band in offsets.windows(2) {
        let polygon = vec![
            layout.min,
            Vec2::new(layout.max.x, layout.min.y),
            layout.max,
            Vec2::new(layout.min.x, layout.max.y),
        ];
        let polygon = clip_half_plane(polygon, across, datum + band[1]);
        let polygon = clip_half_plane(polygon, -across, -datum - band[0]);
        if polygon.len() < 3 {
            continue;
        }
        let base = positions.len() as u32;
        for point in &polygon {
            positions.push([point.x, WATER_Y, point.y]);
            let edge = ((point.dot(across) - datum).abs() - (half - 2.0)).clamp(0.0, 2.0) / 2.0;
            // Exported Blender base-color factors are already linear RGB.
            colors.push([
                0.025 + (0.08 - 0.025) * edge,
                0.24 + (0.39 - 0.24) * edge,
                0.27 + (0.36 - 0.27) * edge,
                1.0,
            ]);
        }
        for i in 1..polygon.len() - 1 {
            if (polygon[i] - polygon[0])
                .perp_dot(polygon[i + 1] - polygon[0])
                .abs()
                > 0.0001
            {
                // The XZ polygon is counterclockwise; invert for a +Y normal.
                indices.extend([base, base + i as u32 + 1, base + i as u32]);
            }
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangles(mesh: &Mesh) -> Vec<[Vec3; 3]> {
        let VertexAttributeValues::Float32x3(vertices) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("missing vertices")
        };
        let Indices::U32(indices) = mesh.indices().unwrap() else {
            panic!("missing indices")
        };
        indices
            .chunks_exact(3)
            .map(|ids| [ids[0], ids[1], ids[2]].map(|id| Vec3::from_array(vertices[id as usize])))
            .collect()
    }

    fn strictly_contains(triangle: &[Vec3; 3], point: Vec2) -> bool {
        let [a, b, c] = triangle.map(|v| Vec2::new(v.x, v.z));
        let determinant = (b - a).perp_dot(c - a);
        let u = (point - a).perp_dot(c - a) / determinant;
        let v = (b - a).perp_dot(point - a) / determinant;
        u > 0.000_001 && v > 0.000_001 && u + v < 1.0 - 0.000_001
    }

    #[test]
    fn river_bands_cover_the_corridor_once_including_both_map_corners() {
        let layout = MapLayout::default();
        let mesh = water_mesh(&layout);
        let triangles = triangles(&mesh);
        assert!(!triangles.is_empty());
        let mut area = 0.0;
        for triangle in &triangles {
            let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            assert!(normal.y > 0.0001, "degenerate or inverted water triangle");
            area += normal.y * 0.5;
            for point in triangle {
                assert!(point.x >= layout.min.x - 0.0001 && point.x <= layout.max.x + 0.0001);
                assert!(point.z >= layout.min.y - 0.0001 && point.z <= layout.max.y + 0.0001);
                assert_eq!(point.y, WATER_Y);
            }
        }
        let width = layout.size().x;
        let half_river = crate::maps::RIVER_WIDTH * 0.5;
        let expected_area = width * width - (width - half_river * 2.0_f32.sqrt()).powi(2);
        assert!(
            (area - expected_area).abs() < 0.02,
            "corridor gap or overlapping area"
        );

        // Use non-symmetric offsets to avoid triangle boundaries. Sampling
        // the whole diagonal includes the narrow clipped tips at both ends.
        let [start, end] = layout.river_polyline();
        let across = Vec2::new(-(end - start).y, (end - start).x).normalize();
        for row in 0..577 {
            for column in 0..51 {
                let point = Vec2::new(
                    layout.min.x + (row as f32 + 0.137) / 577.0 * width,
                    layout.max.y - (row as f32 + 0.391) / 577.0 * width,
                ) + across * ((column as f32 + 0.293) / 51.0 * 20.0 - 10.0);
                let expected = point.x >= layout.min.x
                    && point.x <= layout.max.x
                    && point.y >= layout.min.y
                    && point.y <= layout.max.y
                    && (point - start).dot(across).abs() < half_river;
                let count = triangles
                    .iter()
                    .filter(|triangle| strictly_contains(triangle, point))
                    .count();
                assert_eq!(count, usize::from(expected), "coverage at {point:?}");
            }
        }
    }

    #[test]
    fn river_stays_below_lane_crossings_and_joins_the_meadow_bank() {
        // Baked road tops are Y=0, meadow=-0.02, bank=-0.04. The old
        // shallows were exactly coplanar with roads. All new water uses the
        // same lower datum; it remains within the 5 cm walk-surface budget.
        assert!(WATER_Y < MEADOW_Y && WATER_Y >= -0.05);
        assert!(0.0 - WATER_Y >= 0.03);
        assert!(0.016 - HIGHLIGHT_LOWERING < 0.0);
        let mut bank = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [1.0, AUTHORED_BANK_Y, 2.0],
                [1.0, -1.6, 2.0],
                [2.0, -3.0, 3.0],
            ],
        );
        close_bank_top(&mut bank);
        let VertexAttributeValues::Float32x3(positions) =
            bank.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            positions,
            &vec![[1.0, MEADOW_Y, 2.0], [1.0, -1.6, 2.0], [2.0, -3.0, 3.0]]
        );
    }

    fn fixture() -> App {
        let mut app = App::new();
        app.insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(PlayerVisualMode::Models3d)
            .insert_resource(MapLayout::default());
        register(&mut app);
        app
    }

    fn authored_node(app: &mut App, parent: Entity, name: &'static str, y: f32) -> Entity {
        app.world_mut()
            .spawn((
                Name::new(name),
                Transform::from_xyz(0.0, y, 0.0),
                ChildOf(parent),
            ))
            .id()
    }

    fn replacement_count(app: &mut App) -> usize {
        app.world_mut()
            .query_filtered::<Entity, With<RiverReplacement>>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn river_repair_is_scoped_idempotent_and_survives_scene_reload() {
        let mut app = fixture();
        let root = app.world_mut().spawn(VerdantEnvironment).id();
        let unrelated = app.world_mut().spawn_empty().id();
        let channel = authored_node(&mut app, root, "River / turquoise channel", 0.585);
        let shallow = authored_node(&mut app, root, "River / jade shallows.001", 0.585);
        let highlight = authored_node(&mut app, root, "River / current highlight.001", 0.585);
        let other = authored_node(&mut app, unrelated, "River / turquoise channel", 0.585);
        app.update();
        app.update();
        assert_eq!(replacement_count(&mut app), 1);
        assert_eq!(
            app.world().get::<Visibility>(channel),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<Visibility>(shallow),
            Some(&Visibility::Hidden)
        );
        assert!(
            (app.world()
                .get::<Transform>(highlight)
                .unwrap()
                .translation
                .y
                - 0.55)
                .abs()
                < 0.0001
        );
        assert!(app.world().get::<RiverAdjusted>(other).is_none());
        app.world_mut().despawn(channel);
        authored_node(&mut app, root, "River / turquoise channel", 0.585);
        app.update();
        app.update();
        assert_eq!(
            replacement_count(&mut app),
            1,
            "scene reload must not stack water surfaces"
        );
    }
}
