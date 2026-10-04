//! Nearby hostile turret range, using the authority's radius rather than hero
//! attack reach. The fill is ground-following and has no interaction surface.
use super::CombatStats;
use crate::{
    maps::MapLayout,
    net::{
        GameState, GameStateSnapshot, NetworkStructure, NetworkStructureAttackRange,
        NetworkStructureId,
    },
    player::Player,
    sprite::PlayerVisualMode,
    team::Team,
};
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};
use std::collections::{HashMap, HashSet};

#[derive(Default, Reflect, bevy::gizmos::config::GizmoConfigGroup)]
struct TowerRangeGizmos;
pub(super) struct TowerZonesPlugin;
impl Plugin for TowerZonesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_gizmo_config(
            TowerRangeGizmos,
            bevy::gizmos::config::GizmoConfig {
                line: bevy::gizmos::config::GizmoLineConfig {
                    width: 2.0,
                    ..default()
                },
                depth_bias: 0.0,
                ..default()
            },
        )
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            draw.after(crate::net::ClientNetPipeline::ApplySnapshot),
        );
    }
}
#[derive(Resource)]
struct ZoneAssets {
    material: Handle<StandardMaterial>,
    flat: Handle<ColorMaterial>,
    legacy: shared::map::ResolvedMap,
}
fn setup(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flat: ResMut<Assets<ColorMaterial>>,
) {
    let color = Color::srgba(1.0, 0.055, 0.04, 0.12);
    commands.insert_resource(ZoneAssets {
        material: materials.add(StandardMaterial {
            base_color: color,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            ..default()
        }),
        flat: flat.add(ColorMaterial::from(color)),
        legacy: default(),
    });
}
fn surface(p: Vec2, map: &MapLayout, mode: PlayerVisualMode) -> Vec3 {
    let world = Vec3::new(p.x, map.terrain_height_3d(p.x, p.y) + 0.18, p.y);
    if mode == PlayerVisualMode::Sprite2d {
        crate::world2d::simulation_xz_to_render_xy(world).extend(crate::world2d::layer::VFX - 1.0)
    } else {
        world
    }
}
fn disk(origin: Vec2, radius: f32, map: &MapLayout, mode: PlayerVisualMode) -> Mesh {
    let mut points = Vec::new();
    let mut indices = Vec::new();
    // Short radial strips follow terrain across the complete filled area.
    for ring in 0..=8 {
        for segment in 0..=64 {
            let angle = segment as f32 * std::f32::consts::TAU / 64.0;
            points.push(
                surface(
                    origin + Vec2::new(angle.cos(), angle.sin()) * radius * ring as f32 / 8.0,
                    map,
                    mode,
                )
                .to_array(),
            );
        }
    }
    for ring in 0..8 {
        for segment in 0..64 {
            let a = ring * 65 + segment;
            let b = a + 65;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    let count = points.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
    .with_inserted_indices(Indices::U32(indices))
}
fn danger_visible(local: Vec2, origin: Vec2, radius: f32, hostile: bool, alive: bool) -> bool {
    hostile
        && alive
        && local.is_finite()
        && origin.is_finite()
        && radius.is_finite()
        && (1.0..=60.0).contains(&radius)
        && local.distance(origin) <= radius + 22.0
}
fn draw(
    mut commands: Commands,
    game: Option<Res<GameStateSnapshot>>,
    map: Res<MapLayout>,
    mode: Res<PlayerVisualMode>,
    assets: Res<ZoneAssets>,
    local: Query<(&Transform, &Team, &CombatStats), With<Player>>,
    towers: Query<
        (
            Entity,
            &Transform,
            &Team,
            &CombatStats,
            &NetworkStructureId,
            Option<&NetworkStructureAttackRange>,
        ),
        With<NetworkStructure>,
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut zones: Local<HashMap<Entity, (Entity, f32, Vec2, PlayerVisualMode)>>,
    mut gizmos: Gizmos<TowerRangeGizmos>,
) {
    let observer = local
        .single()
        .ok()
        .filter(|(_, _, stats)| stats.is_alive())
        .filter(|_| {
            game.as_ref()
                .is_some_and(|g| matches!(g.state, GameState::Running))
        });
    let mut shown = HashSet::new();
    if let Some((hero, team, _)) = observer {
        for (entity, pose, other, stats, id, replicated) in &towers {
            let radius = replicated
                .map(|r| r.0)
                .filter(|r| r.is_finite() && *r > 0.0)
                .or_else(|| {
                    assets
                        .legacy
                        .structures
                        .iter()
                        .find(|s| s.id == id.0)
                        .map(|s| s.stats.attack_range)
                })
                .unwrap_or(0.0);
            let origin = pose.translation.xz();
            if !danger_visible(
                hero.translation.xz(),
                origin,
                radius,
                *team != *other,
                stats.is_alive(),
            ) {
                continue;
            }
            shown.insert(entity);
            let rebuild = zones
                .get(&entity)
                .is_none_or(|(_, r, p, m)| *r != radius || *p != origin || *m != *mode);
            if rebuild {
                if let Some((old, ..)) = zones.remove(&entity) {
                    commands.entity(old).despawn();
                }
                let mesh = meshes.add(disk(origin, radius, &map, *mode));
                let mut zone = commands.spawn((
                    Transform::default(),
                    Visibility::Visible,
                    Name::new("HostileTowerRangeFill"),
                ));
                if *mode == PlayerVisualMode::Sprite2d {
                    zone.insert((Mesh2d(mesh), MeshMaterial2d(assets.flat.clone())));
                } else {
                    zone.insert((Mesh3d(mesh), MeshMaterial3d(assets.material.clone())));
                }
                zones.insert(entity, (zone.id(), radius, origin, *mode));
            }
            let inside = hero.translation.xz().distance(origin) <= radius;
            gizmos.linestrip(
                (0..=64).map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 64.0;
                    surface(origin + Vec2::new(a.cos(), a.sin()) * radius, &map, *mode)
                }),
                Color::srgba(1.0, 0.12, 0.08, if inside { 0.9 } else { 0.55 }),
            );
        }
    }
    zones.retain(|source, (entity, ..)| {
        if shown.contains(source) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_cue_is_bounded_hostile_and_uses_authoritative_radius() {
        assert!(danger_visible(Vec2::ZERO, Vec2::X * 25.0, 8.0, true, true));
        assert!(!danger_visible(Vec2::ZERO, Vec2::X * 35.0, 8.0, true, true));
        assert!(!danger_visible(Vec2::ZERO, Vec2::ZERO, 8.0, false, true));
        assert!(!danger_visible(Vec2::ZERO, Vec2::ZERO, 8.0, true, false));
        assert!(!danger_visible(
            Vec2::ZERO,
            Vec2::ZERO,
            f32::NAN,
            true,
            true
        ));
    }
}
