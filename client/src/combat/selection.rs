use crate::camera::MainCamera;
use crate::domain::CombatStats;
use crate::input_context::GameplayInputContext;
use crate::minimap::MinimapNavigationState;
use crate::net::{
    GameState, GameStateSnapshot, NetworkMinion, NetworkMinionId, NetworkNeutral, NetworkNeutralId,
    NetworkPlayerId, NetworkStructure, NetworkStructureId, RemotePlayer, StructureKind, TargetId,
    TargetKind,
};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::targeting::BasicAttackState;
use crate::team::Team;
use bevy::{
    ecs::system::SystemParam, input::mouse::MouseButton, prelude::*, window::PrimaryWindow,
};

use super::cast::PendingCast;

pub(super) const PLAYER_PICK_RADIUS_PX: f32 = 52.0;
pub(super) const MINION_PICK_RADIUS_PX: f32 = 48.0;
pub(super) const NEUTRAL_PICK_RADIUS_PX: f32 = 52.0;
pub(super) const TOWER_PICK_RADIUS_PX: f32 = 56.0;
pub(super) const BASE_TOWER_PICK_RADIUS_PX: f32 = 68.0;

#[derive(Resource, Default)]
pub struct TargetState {
    pub selected_entity: Option<Entity>,
    pub selected_target: Option<TargetId>,
    pub(super) marker_entity: Option<Entity>,
}

/// Per-frame routing state that prevents one physical press from both
/// attacking a unit and issuing a ground movement command.
#[derive(Resource, Default)]
pub(crate) struct WorldPointerState {
    pub(crate) consumed_primary_press: bool,
    pub(crate) consumed_secondary_press: bool,
}

#[derive(SystemParam)]
pub(crate) struct TargetCandidates<'w, 's> {
    pub(crate) players: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkPlayerId,
            &'static CombatStats,
            &'static Team,
        ),
        (With<RemotePlayer>, Without<Player>),
    >,
    pub(crate) structures: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkStructureId,
            &'static CombatStats,
            &'static Team,
            &'static StructureKind,
        ),
        With<NetworkStructure>,
    >,
    pub(crate) minions: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkMinionId,
            &'static CombatStats,
            &'static Team,
        ),
        With<NetworkMinion>,
    >,
    pub(crate) neutrals: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkNeutralId,
            &'static CombatStats,
        ),
        With<NetworkNeutral>,
    >,
}

pub(super) fn select_target_system(
    input: (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    game_state: Option<Res<GameStateSnapshot>>,
    local_player: Query<(&Transform, &Team), With<Player>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    candidates: TargetCandidates,
    mut target_state: ResMut<TargetState>,
    mut pending_cast: ResMut<PendingCast>,
    mut pointer_state: ResMut<WorldPointerState>,
    visual_mode: Res<PlayerVisualMode>,
    minimap_nav: Option<Res<MinimapNavigationState>>,
    ui_interactions: Query<&Interaction, With<Button>>,
    context: Res<GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut basic: ResMut<BasicAttackState>,
    validity: crate::targeting::TargetValidity,
) {
    let (keyboard, mouse) = input;
    *pointer_state = default();
    if mobile.as_ref().is_some_and(|m| m.enabled)
        || !context.gameplay_allowed()
        || game_state
            .as_ref()
            .is_some_and(|g| !matches!(g.state, GameState::Running))
    {
        return;
    }
    let Ok((local_transform, local_team)) = local_player.single() else {
        return;
    };
    let Ok(window) = window_query.single() else {
        return;
    };
    if !window.focused {
        return;
    }
    let primary = mouse.just_pressed(MouseButton::Left);
    let secondary = mouse.just_pressed(MouseButton::Right)
        && !keyboard.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let mut picked = if keyboard.just_pressed(KeyCode::Tab) {
        find_nearest_enemy_target(
            local_transform.translation,
            *local_team,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures,
        )
    } else {
        None
    };
    if primary || secondary {
        let blocked = ui_interactions.iter().any(|i| *i != Interaction::None)
            || minimap_nav
                .as_ref()
                .is_some_and(|n| n.consumed_primary_click);
        if blocked {
            return;
        }
        let (Some(position), Ok((camera, transform))) =
            (window.cursor_position(), camera_query.single())
        else {
            return;
        };
        picked = find_target_near_screen(
            position,
            camera,
            transform,
            *visual_mode,
            *local_team,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures,
        );
    }
    picked = picked.filter(|(entity, id)| validity.valid(*entity, *id, *local_team));
    if let Some((entity, id)) = picked {
        target_state.selected_entity = Some(entity);
        target_state.selected_target = Some(id);
        pending_cast.cancel();
        if secondary {
            pointer_state.consumed_secondary_press = true;
            basic.start(entity, id, true);
        } else {
            pointer_state.consumed_primary_press = primary;
            basic.cancel();
        }
    } else if primary {
        target_state.selected_entity = None;
        target_state.selected_target = None;
        pending_cast.cancel();
        basic.cancel();
    }
}

pub(super) fn find_nearest_enemy_target(
    local_pos: Vec3,
    local_team: Team,
    validity: &crate::targeting::TargetValidity,
    player_candidates: &Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats, &Team),
        (With<RemotePlayer>, Without<Player>),
    >,
    minion_candidates: &Query<
        (Entity, &Transform, &NetworkMinionId, &CombatStats, &Team),
        With<NetworkMinion>,
    >,
    neutral_candidates: &Query<
        (Entity, &Transform, &NetworkNeutralId, &CombatStats),
        With<NetworkNeutral>,
    >,
    structure_candidates: &Query<
        (
            Entity,
            &Transform,
            &NetworkStructureId,
            &CombatStats,
            &Team,
            &StructureKind,
        ),
        With<NetworkStructure>,
    >,
) -> Option<(Entity, TargetId)> {
    let mut best: Option<(Entity, TargetId, f32)> = None;

    for (entity, transform, id, stats, team) in player_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Player,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    for (entity, transform, id, stats, team) in minion_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Minion,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    for (entity, transform, id, stats) in neutral_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Neutral,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    for (entity, transform, id, stats, team, _kind) in structure_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Structure,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    best.map(|(entity, target, _)| (entity, target))
}

pub(super) fn find_target_near_screen(
    pointer_position: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    visual_mode: PlayerVisualMode,
    local_team: Team,
    validity: &crate::targeting::TargetValidity,
    player_candidates: &Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats, &Team),
        (With<RemotePlayer>, Without<Player>),
    >,
    minion_candidates: &Query<
        (Entity, &Transform, &NetworkMinionId, &CombatStats, &Team),
        With<NetworkMinion>,
    >,
    neutral_candidates: &Query<
        (Entity, &Transform, &NetworkNeutralId, &CombatStats),
        With<NetworkNeutral>,
    >,
    structure_candidates: &Query<
        (
            Entity,
            &Transform,
            &NetworkStructureId,
            &CombatStats,
            &Team,
            &StructureKind,
        ),
        With<NetworkStructure>,
    >,
) -> Option<(Entity, TargetId)> {
    let mut best: Option<(Entity, TargetId, f32)> = None;

    for (entity, transform, id, stats, team) in player_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            PLAYER_PICK_RADIUS_PX,
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
        );
    }

    for (entity, transform, id, stats, team) in minion_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            MINION_PICK_RADIUS_PX,
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
        );
    }

    for (entity, transform, id, stats) in neutral_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() {
            continue;
        }
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            NEUTRAL_PICK_RADIUS_PX,
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
        );
    }

    for (entity, transform, id, stats, team, kind) in structure_candidates.iter() {
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let radius = match kind {
            StructureKind::Tower => TOWER_PICK_RADIUS_PX,
            StructureKind::BaseTower => BASE_TOWER_PICK_RADIUS_PX,
        };
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            radius,
            entity,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
        );
    }

    best.map(|(entity, target, _)| (entity, target))
}

fn consider_screen_target(
    best: &mut Option<(Entity, TargetId, f32)>,
    pointer_position: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    visual_mode: PlayerVisualMode,
    simulation_position: Vec3,
    pick_radius_px: f32,
    entity: Entity,
    target: TargetId,
) {
    let render_position = if visual_mode == PlayerVisualMode::Sprite2d {
        let xy = crate::world2d::simulation_xz_to_render_xy(simulation_position);
        Vec3::new(xy.x, xy.y, crate::world2d::layer::ACTOR)
    } else {
        simulation_position
    };
    let Ok(screen_position) = camera.world_to_viewport(camera_transform, render_position) else {
        return;
    };
    let Some(distance) = screen_pick_distance(pointer_position, screen_position, pick_radius_px)
    else {
        return;
    };
    if best.is_none_or(|(_, _, best_distance)| distance < best_distance) {
        *best = Some((entity, target, distance));
    }
}

pub(super) fn screen_pick_distance(
    pointer: Vec2,
    actor_center: Vec2,
    radius_px: f32,
) -> Option<f32> {
    let distance = pointer.distance(actor_center);
    (distance <= radius_px).then_some(distance)
}
