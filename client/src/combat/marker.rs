use crate::net::{NetworkMinion, NetworkNeutral, NetworkStructure, RemotePlayer, StructureKind};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::team::Team;
use bevy::prelude::*;

use super::selection::TargetState;

const TARGET_MARKER_SIZE: f32 = 2.0;
const TARGET_MARKER_Y: f32 = 0.08;
pub(super) const TARGET_MARKER_INNER_RADIUS: f32 = 0.90;
const PLAYER_MARKER_RADIUS: f32 = 1.25;
const MINION_MARKER_RADIUS: f32 = 1.05;
const NEUTRAL_MARKER_RADIUS: f32 = 1.1;
const TOWER_MARKER_RADIUS: f32 = 2.0;
const BASE_TOWER_MARKER_RADIUS: f32 = 3.75;

#[derive(Component)]
pub(super) struct TargetMarker;

pub(super) fn update_target_marker_system(
    target_state: Res<TargetState>,
    mut poses: ParamSet<(
        bevy::transform::helper::TransformHelper,
        crate::targeting::TargetValidity,
        Query<(&mut Transform, &mut Visibility), With<TargetMarker>>,
    )>,
    local: Query<&Team, With<Player>>,
    structure_kinds: Query<&StructureKind, With<NetworkStructure>>,
    minions: Query<(), With<NetworkMinion>>,
    neutrals: Query<(), With<NetworkNeutral>>,
    players: Query<(), Or<(With<Player>, With<RemotePlayer>)>>,
    map: Res<crate::maps::MapLayout>,
    mode: Res<PlayerVisualMode>,
) {
    let Some(marker_entity) = target_state.marker_entity else {
        return;
    };
    let selected = target_state
        .selected_entity
        .zip(target_state.selected_target)
        .filter(|(entity, id)| {
            local
                .single()
                .is_ok_and(|team| poses.p1().valid(*entity, *id, *team))
        });
    let anchor = selected.and_then(|(entity, _)| {
        poses
            .p0()
            .compute_global_transform(entity)
            .ok()
            .map(|pose| (entity, pose.translation()))
    });
    let mut markers = poses.p2();
    let Ok((mut transform, mut visibility)) = markers.get_mut(marker_entity) else {
        return;
    };
    let Some((entity, position)) = anchor.filter(|(_, p)| p.is_finite()) else {
        *visibility = Visibility::Hidden;
        return;
    };
    let radius = if let Ok(kind) = structure_kinds.get(entity) {
        match kind {
            StructureKind::Tower => TOWER_MARKER_RADIUS,
            StructureKind::BaseTower => BASE_TOWER_MARKER_RADIUS,
        }
    } else if minions.contains(entity) {
        MINION_MARKER_RADIUS
    } else if neutrals.contains(entity) {
        NEUTRAL_MARKER_RADIUS
    } else if players.contains(entity) {
        PLAYER_MARKER_RADIUS
    } else {
        TARGET_MARKER_SIZE * 0.5
    };
    // Anchor to terrain, never to animated/skinned bounds or a model's pivot.
    // A fixed ring cannot wobble when the hero turns, attacks, or changes skin.
    *transform = if *mode == PlayerVisualMode::Sprite2d {
        let xy = crate::world2d::simulation_xz_to_render_xy(position);
        Transform::from_translation(xy.extend(crate::world2d::layer::MARKER))
            .with_scale(Vec3::splat(radius))
    } else {
        Transform::from_xyz(
            position.x,
            map.terrain_height_3d(position.x, position.z) + TARGET_MARKER_Y,
            position.z,
        )
        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
        .with_scale(Vec3::splat(radius))
    };
    *visibility = Visibility::Visible;
}
