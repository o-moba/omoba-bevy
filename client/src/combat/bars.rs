use crate::camera::MainCamera;
use crate::domain::CombatStats;
use crate::model_scale::NormalizeModelScale;
use crate::net::{NetworkMinion, NetworkNeutral, StructureKind};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::team::Team;
use bevy::{camera::primitives::Aabb, math::primitives::Rectangle, prelude::*};

use super::marker::{TARGET_MARKER_INNER_RADIUS, TargetMarker};
use super::selection::TargetState;

const BAR_WIDTH: f32 = 1.45;
const BAR_HEIGHT: f32 = 0.09;
const BAR_LAYER_OFFSET: f32 = 0.01;
const MANA_BAR_OFFSET_Y: f32 = -0.15;
const BAR_HEAD_CLEARANCE: f32 = 0.28;
const MIN_PLAYER_BAR_Y: f32 = 1.4;
const TOWER_BAR_Y: f32 = 3.6;
const BASE_TOWER_BAR_Y: f32 = 4.8;

#[derive(Resource)]
pub(super) struct CombatVisualAssets {
    is_2d: bool,
    hp_bg_material: Handle<StandardMaterial>,
    hp_fill_material: Handle<StandardMaterial>,
    hp_local_material: Handle<StandardMaterial>,
    hp_friendly_material: Handle<StandardMaterial>,
    mana_bg_material: Handle<StandardMaterial>,
    mana_fill_material: Handle<StandardMaterial>,
    bar_mesh: Handle<Mesh>,
}

#[derive(Component, Default)]
pub(super) struct CombatBars {
    hp_fill: Option<Entity>,
    mana_fill: Option<Entity>,
}

#[derive(Component)]
pub(super) struct CombatBarRoot;

#[derive(Component)]
pub(crate) struct CombatBarAnchor {
    pub(crate) target: Entity,
    y_offset: f32,
}

pub(super) fn setup_combat_visual_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flat_materials: ResMut<Assets<ColorMaterial>>,
    mode: Res<PlayerVisualMode>,
) {
    let is_2d = *mode == PlayerVisualMode::Sprite2d;
    let bar_mesh = meshes.add(Mesh::from(Rectangle::new(BAR_WIDTH, BAR_HEIGHT)));
    let marker_mesh = meshes.add(Annulus::new(TARGET_MARKER_INNER_RADIUS, 1.0));

    let hp_bg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.02, 0.02),
        perceptual_roughness: 1.0,
        unlit: true,
        ..default()
    });
    let hp_fill_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.15, 0.18),
        perceptual_roughness: 0.9,
        unlit: true,
        ..default()
    });
    let hp_local_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.85, 0.2),
        unlit: true,
        ..default()
    });
    let hp_friendly_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.85, 0.6),
        unlit: true,
        ..default()
    });
    let mana_bg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.04, 0.06, 0.18),
        perceptual_roughness: 1.0,
        unlit: true,
        ..default()
    });
    let mana_fill_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.52, 0.95),
        perceptual_roughness: 0.6,
        unlit: true,
        ..default()
    });
    let target_material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 0.84, 0.24, 0.85),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });

    let mut marker_commands = commands.spawn((
        Transform::from_xyz(0.0, -50.0, 0.0),
        Visibility::Hidden,
        TargetMarker,
        Name::new("TargetMarker"),
    ));
    if is_2d {
        marker_commands.insert((
            Mesh2d(marker_mesh),
            MeshMaterial2d(flat_materials.add(Color::srgba(1.0, 0.84, 0.24, 0.85))),
        ));
    } else {
        marker_commands.insert((
            Mesh3d(marker_mesh),
            MeshMaterial3d(target_material),
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
    }
    let marker_entity = marker_commands.id();

    commands.insert_resource(CombatVisualAssets {
        is_2d,
        hp_bg_material,
        hp_fill_material,
        hp_local_material,
        hp_friendly_material,
        mana_bg_material,
        mana_fill_material,
        bar_mesh,
    });
    commands.insert_resource(TargetState {
        marker_entity: Some(marker_entity),
        ..default()
    });
}

pub(super) fn spawn_combat_bars_system(
    mut commands: Commands,
    assets: Res<CombatVisualAssets>,
    players_without_bars: Query<
        (
            Entity,
            Option<&StructureKind>,
            Option<&NetworkMinion>,
            Option<&NetworkNeutral>,
        ),
        (With<CombatStats>, Without<CombatBars>),
    >,
) {
    for (entity, structure_kind, minion_marker, neutral_marker) in players_without_bars.iter() {
        let bar_y = match structure_kind.copied() {
            Some(StructureKind::Tower) => TOWER_BAR_Y,
            Some(StructureKind::BaseTower) => BASE_TOWER_BAR_Y,
            None => 2.1,
        };
        let show_mana_bar =
            structure_kind.is_none() && minion_marker.is_none() && neutral_marker.is_none();
        let mut bars = CombatBars::default();
        let bar_root = commands
            .spawn((
                Transform::from_xyz(0.0, bar_y, 0.0),
                Visibility::default(),
                CombatBarRoot,
                CombatBarAnchor {
                    target: entity,
                    y_offset: bar_y,
                },
                Name::new("CombatBarRoot"),
            ))
            .id();

        commands.entity(bar_root).with_children(|parent| {
            if assets.is_2d {
                parent.spawn((
                    Sprite::from_color(
                        Color::srgb(0.15, 0.02, 0.02),
                        Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                    ),
                    Transform::default(),
                    Name::new("HpBarBg"),
                ));
                let hp_fill = parent
                    .spawn((
                        Sprite::from_color(
                            Color::srgb(0.85, 0.15, 0.18),
                            Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                        ),
                        Transform::from_xyz(0.0, 0.0, BAR_LAYER_OFFSET),
                        Name::new("HpBarFill"),
                    ))
                    .id();
                bars.hp_fill = Some(hp_fill);
                if show_mana_bar {
                    parent.spawn((
                        Sprite::from_color(
                            Color::srgb(0.04, 0.06, 0.18),
                            Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                        ),
                        Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, 0.0),
                        Name::new("ManaBarBg"),
                    ));
                    bars.mana_fill = Some(
                        parent
                            .spawn((
                                Sprite::from_color(
                                    Color::srgb(0.16, 0.52, 0.95),
                                    Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                                ),
                                Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, BAR_LAYER_OFFSET),
                                Name::new("ManaBarFill"),
                            ))
                            .id(),
                    );
                }
                return;
            }
            parent.spawn((
                Mesh3d(assets.bar_mesh.clone()),
                MeshMaterial3d(assets.hp_bg_material.clone()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                Name::new("HpBarBg"),
            ));

            let hp_fill = parent
                .spawn((
                    Mesh3d(assets.bar_mesh.clone()),
                    MeshMaterial3d(assets.hp_fill_material.clone()),
                    Transform::from_xyz(0.0, 0.0, BAR_LAYER_OFFSET),
                    Name::new("HpBarFill"),
                ))
                .id();

            bars.hp_fill = Some(hp_fill);
            if show_mana_bar {
                parent.spawn((
                    Mesh3d(assets.bar_mesh.clone()),
                    MeshMaterial3d(assets.mana_bg_material.clone()),
                    Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, 0.0),
                    Name::new("ManaBarBg"),
                ));

                let mana_fill = parent
                    .spawn((
                        Mesh3d(assets.bar_mesh.clone()),
                        MeshMaterial3d(assets.mana_fill_material.clone()),
                        Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, BAR_LAYER_OFFSET),
                        Name::new("ManaBarFill"),
                    ))
                    .id();
                bars.mana_fill = Some(mana_fill);
            }
        });
        // `try_insert`: the owner may have been despawned in this same frame
        // (e.g. the duplicate local-player cleanup in `apply_server_snapshot`);
        // a plain `insert` would panic when the command buffer is applied. Any
        // orphaned bar root is cleaned up by `sync_combat_bar_transforms_system`.
        commands.entity(entity).try_insert(bars);
    }
}

pub(super) fn update_combat_bars_system(
    owners: Query<(&CombatStats, &CombatBars, Option<&Team>, Has<Player>)>,
    local_team: Query<&Team, With<Player>>,
    assets: Res<CombatVisualAssets>,
    mut fills: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut transforms: Query<&mut Transform>,
) {
    for (stats, bars, team, is_local) in owners.iter() {
        let hp_ratio = if stats.max_hp > 0.0 {
            (stats.hp / stats.max_hp).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mana_ratio = if stats.max_mana > 0.0 {
            (stats.mana / stats.max_mana).clamp(0.0, 1.0)
        } else {
            0.0
        };

        if let Some(hp_fill) = bars.hp_fill {
            if let Ok(mut material) = fills.get_mut(hp_fill) {
                material.0 = if is_local {
                    assets.hp_local_material.clone()
                } else if team
                    .zip(local_team.single().ok())
                    .is_some_and(|(team, local)| team == local)
                {
                    assets.hp_friendly_material.clone()
                } else {
                    assets.hp_fill_material.clone()
                };
            }
            if let Ok(mut transform) = transforms.get_mut(hp_fill) {
                transform.scale.x = hp_ratio.max(0.001);
                transform.translation.x = (hp_ratio - 1.0) * BAR_WIDTH * 0.5;
            }
        }

        if let Some(mana_fill) = bars.mana_fill {
            if let Ok(mut transform) = transforms.get_mut(mana_fill) {
                transform.scale.x = mana_ratio.max(0.001);
                transform.translation.x = (mana_ratio - 1.0) * BAR_WIDTH * 0.5;
            }
        }
    }
}

pub(super) fn sync_combat_bar_transforms_system(
    mut commands: Commands,
    camera_query: Query<&GlobalTransform, With<MainCamera>>,
    global_query: Query<&GlobalTransform>,
    aabb_query: Query<&Aabb>,
    children_query: Query<&Children>,
    normalized_query: Query<&NormalizeModelScale>,
    mut bar_query: Query<(Entity, &CombatBarAnchor, &mut Transform), With<CombatBarRoot>>,
    mode: Res<PlayerVisualMode>,
) {
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };
    let camera_rotation = camera_transform.compute_transform().rotation;

    for (bar_entity, anchor, mut bar_transform) in bar_query.iter_mut() {
        let Ok(target_transform) = global_query.get(anchor.target) else {
            commands
                .entity(bar_entity)
                .despawn_related::<Children>()
                .despawn();
            continue;
        };
        if *mode == PlayerVisualMode::Sprite2d {
            let xy = crate::world2d::simulation_xz_to_render_xy(target_transform.translation());
            bar_transform.translation = Vec3::new(
                xy.x,
                xy.y + anchor.y_offset,
                crate::world2d::layer::OVERHEAD,
            );
            bar_transform.rotation = Quat::IDENTITY;
            continue;
        }
        // Normalized player models report a deterministic head height; prefer it
        // over per-frame AABB sampling (unstable for rigged/center-pivot meshes).
        let bar_world_y = match normalized_query
            .get(anchor.target)
            .ok()
            .and_then(|n| n.head_local_y)
        {
            Some(head_local_y) => {
                target_transform.translation().y + head_local_y + BAR_HEAD_CLEARANCE
            }
            None => compute_bar_world_y_for_entity(
                anchor.target,
                target_transform.translation().y + anchor.y_offset,
                &children_query,
                &aabb_query,
                &global_query,
            ),
        };
        bar_transform.translation = Vec3::new(
            target_transform.translation().x,
            bar_world_y,
            target_transform.translation().z,
        );
        bar_transform.rotation = camera_rotation;
    }
}

fn compute_bar_world_y_for_entity(
    entity: Entity,
    fallback_world_y: f32,
    children_query: &Query<&Children>,
    aabb_query: &Query<&Aabb>,
    global_query: &Query<&GlobalTransform>,
) -> f32 {
    let mut max_y = f32::NEG_INFINITY;
    let mut has_bounds = false;

    let mut sample_entity = |sample: Entity| {
        let (Ok(aabb), Ok(global)) = (aabb_query.get(sample), global_query.get(sample)) else {
            return;
        };
        let center: Vec3 = aabb.center.into();
        let half: Vec3 = aabb.half_extents.into();
        for sx in [-1.0_f32, 1.0] {
            for sy in [-1.0_f32, 1.0] {
                for sz in [-1.0_f32, 1.0] {
                    let local_corner = center + Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                    let world_corner = global.transform_point(local_corner);
                    max_y = max_y.max(world_corner.y);
                    has_bounds = true;
                }
            }
        }
    };

    sample_entity(entity);
    for child in children_query.iter_descendants(entity) {
        sample_entity(child);
    }

    if !has_bounds {
        return fallback_world_y;
    }

    (max_y + BAR_HEAD_CLEARANCE).max(MIN_PLAYER_BAR_Y)
}
