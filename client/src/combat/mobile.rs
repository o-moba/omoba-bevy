// i18n-strict
use crate::camera::MainCamera;
use crate::domain::CombatStats;
use crate::input_context::GameplayInputContext;
use crate::net::{NetworkCommand, NetworkHeroClass, PlayerProgression, TargetId, TargetKind};
use crate::player::Player;
use crate::sprite::PlayerVisualMode;
use crate::team::{Team, TeamSelection};
use bevy::prelude::*;
use shared::{
    MAX_ABILITY_RANK, SkillSlot, TargetingMode, ability_for_class_slot, scaled_cast_range,
};

use super::cast::{PendingCast, queue_cast_request};
use super::feedback::ActionFeedback;
use super::selection::{TargetCandidates, TargetState};

/// Utility commands use the same network request IDs and match identity as
/// other combat actions. Cooldowns come only from authoritative snapshots.
pub(super) fn mobile_utility_system(
    mut mobile: Option<ResMut<crate::mobile_controls::MobileControls>>,
    context: Res<GameplayInputContext>,
    local: Query<(&Transform, &CombatStats, &crate::net::PlayerUtility), With<Player>>,
    camera: Query<&GlobalTransform, With<MainCamera>>,
    mode: Res<PlayerVisualMode>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    let Some(mobile) = mobile.as_deref_mut().filter(|mobile| mobile.enabled) else {
        return;
    };
    let intents = std::mem::take(&mut mobile.utilities);
    if !context.gameplay_allowed() || !mobile.focused || !mobile.landscape {
        return;
    }
    let Ok((transform, stats, utility)) = local.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }
    for (action, aim) in intents {
        use shared::utility::UtilityAction;
        let remaining = match action {
            UtilityAction::Dash => utility.state.dash_remaining_secs,
            UtilityAction::Haste => utility.state.haste_remaining_secs,
            UtilityAction::Recall | UtilityAction::CancelRecall => 0.0,
        };
        if remaining > 0.0 {
            continue;
        }
        let direction = if action == UtilityAction::Dash {
            let screen = aim
                .or_else(|| (mobile.movement.length_squared() > 0.001).then_some(mobile.movement));
            if let (Some(screen), Ok(camera)) = (screen, camera.single()) {
                crate::player::mobile_screen_direction(screen, camera, *mode)
                    .xz()
                    .normalize_or_zero()
            } else {
                transform.forward().xz().normalize_or_zero()
            }
        } else {
            Vec2::ZERO
        };
        commands.write(NetworkCommand::Utility { action, direction });
    }
}

/// Mobile abilities share the existing PendingCast/try_cast_slot path. The
/// assistance step changes only target choice, never range, mana or cooldowns.
pub(super) fn mobile_cast_system(
    mut mobile: Option<ResMut<crate::mobile_controls::MobileControls>>,
    context: Res<GameplayInputContext>,
    selection: Res<TeamSelection>,
    local: Query<
        (
            &Transform,
            &Team,
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    candidates: TargetCandidates,
    validity: crate::targeting::TargetValidity,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    visual_mode: Res<PlayerVisualMode>,
    target: Res<TargetState>,
    basic: Res<crate::targeting::BasicAttackState>,
    mut pending: ResMut<PendingCast>,
    mut feedback: ResMut<ActionFeedback>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    let Some(mobile) = mobile.as_deref_mut().filter(|mobile| mobile.enabled) else {
        return;
    };
    if !context.gameplay_allowed() {
        mobile.casts.clear();
        mobile.upgrades.clear();
        return;
    }
    let Ok((transform, team, stats, prog, class)) = local.single() else {
        return;
    };
    if !stats.is_alive() {
        mobile.casts.clear();
        mobile.upgrades.clear();
        return;
    }
    let prog = prog.copied().unwrap_or_default();
    for slot in mobile.upgrades.drain(..) {
        if slot < 4
            && prog.skill_points > 0
            && prog.ranks[slot] < MAX_ABILITY_RANK
            && prog.unlocked()[slot]
        {
            commands.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
        }
    }
    let class = class.map(|class| class.0).unwrap_or(selection.hero_class);
    let intent = mobile.casts.drain(..).next_back();
    let Some(intent) = intent else {
        return;
    };
    let Some(slot) = SkillSlot::from_index(intent.slot as u8) else {
        return;
    };
    let definition = ability_for_class_slot(class, slot);
    if shared::loadout::preset_for_class(class).is_some() {
        let range = scaled_cast_range(definition, prog.ranks[intent.slot].max(1));
        let origin = transform.translation.xz();
        let manual = intent.aim.and_then(|screen| {
            camera.single().ok().map(|(_, pose)| {
                crate::player::mobile_screen_direction(screen, pose, *visual_mode).xz()
            })
        });
        let assisted = intent
            .aim
            .is_none()
            .then(|| {
                quick_cast_target(
                    origin,
                    *team,
                    range,
                    target.selected_entity,
                    basic.order.map(|order| order.entity),
                    &candidates,
                    &validity,
                )
            })
            .flatten();
        queue_cast_request(intent.slot, class, &target, &mut pending, &mut feedback);
        pending.aim = Some(resolve_mobile_aim(
            origin,
            transform.forward().xz(),
            range,
            intent.extent,
            manual,
            assisted,
        ));
        return;
    }
    if definition.targeting == TargetingMode::UnitTarget {
        let Ok((camera, camera_transform)) = camera.single() else {
            return;
        };
        let range = scaled_cast_range(definition, prog.ranks[intent.slot].max(1));
        let pick = if intent.aim.is_none() && target.selected_entity.is_some() {
            // An explicit lock wins even when currently out of range. The skill
            // resolver reports that range error instead of hitting another foe.
            target.selected_entity.zip(target.selected_target)
        } else {
            mobile_assisted_target(
                transform.translation,
                *team,
                range,
                intent.aim,
                &candidates,
                &validity,
                camera,
                camera_transform,
                *visual_mode,
                target.selected_entity,
            )
        };
        let Some((entity, id)) = pick else {
            return;
        };
        let request_target = TargetState {
            selected_entity: Some(entity),
            selected_target: Some(id),
            ..default()
        };
        queue_cast_request(
            intent.slot,
            class,
            &request_target,
            &mut pending,
            &mut feedback,
        );
        return;
    }
    queue_cast_request(intent.slot, class, &target, &mut pending, &mut feedback);
}

/// Short taps prefer a valid lock/chase, then the closest visible hero. Manual
/// drags never pass through this chooser. Snapshot visibility remains authority.
pub(super) fn quick_cast_target(
    origin: Vec2,
    team: Team,
    range: f32,
    selected: Option<Entity>,
    chased: Option<Entity>,
    candidates: &TargetCandidates,
    validity: &crate::targeting::TargetValidity,
) -> Option<Vec2> {
    let range = range.min(shared::vision::HERO_SIGHT_RADIUS);
    let mut choices = Vec::new();
    let mut consider = |entity: Entity, id: TargetId, p: Vec3, stats: &CombatStats| {
        let distance = origin.distance(p.xz());
        if stats.is_alive()
            && p.is_finite()
            && distance > 0.01
            && distance <= range
            && validity.valid(entity, id, team)
        {
            let priority = if selected == Some(entity) {
                0
            } else if chased == Some(entity) {
                1
            } else if id.kind == TargetKind::Player {
                2
            } else {
                3
            };
            choices.push((priority, distance, id.id, p.xz()));
        }
    };
    for (entity, pose, id, stats, _) in &candidates.players {
        consider(
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            pose.translation,
            stats,
        );
    }
    for (entity, pose, id, stats, _) in &candidates.minions {
        consider(
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            pose.translation,
            stats,
        );
    }
    for (entity, pose, id, stats) in &candidates.neutrals {
        consider(
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            pose.translation,
            stats,
        );
    }
    choices
        .into_iter()
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)))
        .map(|choice| choice.3)
}

pub(super) fn resolve_mobile_aim(
    origin: Vec2,
    facing: Vec2,
    range: f32,
    extent: f32,
    manual: Option<Vec2>,
    assisted: Option<Vec2>,
) -> Vec2 {
    if let Some(direction) = manual {
        origin + direction.normalize_or_zero() * range * extent.clamp(0.0, 1.0)
    } else if let Some(target) = assisted {
        target
    } else {
        origin + facing.normalize_or_zero() * range
    }
}

pub(crate) fn mobile_assisted_target(
    position: Vec3,
    team: Team,
    range: f32,
    aim: Option<Vec2>,
    candidates: &TargetCandidates,
    validity: &crate::targeting::TargetValidity,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    mode: PlayerVisualMode,
    selected: Option<Entity>,
) -> Option<(Entity, TargetId)> {
    let screen = |p: Vec3| {
        let render = if mode == PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(p).extend(0.0)
        } else {
            p
        };
        camera.world_to_viewport(camera_transform, render).ok()
    };
    let origin = screen(position)?;
    let viewport = camera.logical_viewport_size()?;
    let mut best: Option<(Entity, TargetId, f32)> = None;
    let mut consider = |entity: Entity, id: TargetId, p: Vec3, stats: &CombatStats, enemy: bool| {
        if !enemy || !stats.is_alive() || !validity.valid(entity, id, team) {
            return;
        }
        let distance = position.xz().distance(p.xz());
        if distance > range {
            return;
        }
        let Some(projected) = screen(p) else {
            return;
        };
        if projected.x < 0.0
            || projected.y < 0.0
            || projected.x > viewport.x
            || projected.y > viewport.y
        {
            return;
        }
        let Some(score) = mobile_target_score(
            distance,
            range,
            projected - origin,
            aim,
            selected == Some(entity),
        ) else {
            return;
        };
        if best.is_none_or(|(_, _, previous)| score < previous) {
            best = Some((entity, id, score));
        }
    };
    for (e, t, id, s, target_team) in &candidates.players {
        consider(
            e,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    for (e, t, id, s, target_team) in &candidates.minions {
        consider(
            e,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    for (e, t, id, s) in &candidates.neutrals {
        consider(
            e,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            t.translation,
            s,
            true,
        );
    }
    for (e, t, id, s, target_team, _) in &candidates.structures {
        consider(
            e,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    best.map(|(entity, id, _)| (entity, id))
}

pub(super) fn mobile_target_score(
    distance: f32,
    range: f32,
    screen_delta: Vec2,
    aim: Option<Vec2>,
    selected: bool,
) -> Option<f32> {
    if !distance.is_finite() || distance > range || range <= 0.0 {
        return None;
    }
    if let Some(aim) = aim {
        let alignment = screen_delta
            .normalize_or_zero()
            .dot(aim.normalize_or_zero());
        // Directional assist uses a 45-degree half cone and has no fallback
        // behind the player when the requested direction contains no enemy.
        (alignment >= std::f32::consts::FRAC_1_SQRT_2)
            .then_some((1.0 - alignment) * 4.0 + distance / range)
    } else {
        Some(distance / range - if selected { 2.0 } else { 0.0 })
    }
}

#[cfg(test)]
mod quick_cast_tests {
    use super::*;
    #[test]
    fn quick_cast_uses_lock_then_chase_then_visible_hero_without_hidden_or_dead_targets() {
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        let mut hero = |id, x, team, visibility| {
            world
                .spawn((
                    crate::net::RemotePlayer,
                    crate::net::NetworkPlayerId(id),
                    Transform::from_xyz(x, 0.0, 0.0),
                    CombatStats::default(),
                    team,
                    visibility,
                ))
                .id()
        };
        let near = hero(2, 8.0, Team::Blue, InheritedVisibility::VISIBLE);
        let chase = hero(3, 16.0, Team::Blue, InheritedVisibility::VISIBLE);
        let hidden = hero(4, 2.0, Team::Blue, InheritedVisibility::HIDDEN);
        hero(5, 1.0, Team::Green, InheritedVisibility::VISIBLE);
        let far = hero(6, 60.0, Team::Blue, InheritedVisibility::VISIBLE);
        let pick = |world: &mut World, selected, chased| {
            let mut state =
                SystemState::<(TargetCandidates, crate::targeting::TargetValidity)>::new(world);
            let (candidates, validity) = state.get(world);
            quick_cast_target(
                Vec2::ZERO,
                Team::Green,
                100.0,
                selected,
                chased,
                &candidates,
                &validity,
            )
        };
        assert_eq!(pick(&mut world, None, None), Some(Vec2::X * 8.0));
        assert_eq!(pick(&mut world, None, Some(chase)), Some(Vec2::X * 16.0));
        assert_eq!(
            pick(&mut world, Some(near), Some(chase)),
            Some(Vec2::X * 8.0)
        );
        assert_eq!(
            pick(&mut world, Some(hidden), Some(chase)),
            Some(Vec2::X * 16.0)
        );
        assert_eq!(pick(&mut world, Some(far), None), Some(Vec2::X * 8.0));
        world.get_mut::<CombatStats>(near).unwrap().hp = 0.0;
        assert_eq!(pick(&mut world, Some(near), None), Some(Vec2::X * 16.0));
    }

    #[test]
    fn tap_faces_target_but_drag_and_no_enemy_fallback_preserve_intent() {
        let origin = Vec2::new(3.0, 4.0);
        let enemy = Vec2::new(-5.0, 8.0);
        assert_eq!(
            resolve_mobile_aim(origin, Vec2::X, 40.0, 1.0, None, Some(enemy)),
            enemy
        );
        assert_eq!(
            resolve_mobile_aim(origin, Vec2::X, 40.0, 0.5, Some(Vec2::Y), Some(enemy)),
            origin + Vec2::Y * 20.0
        );
        assert_eq!(
            resolve_mobile_aim(origin, Vec2::X, 40.0, 1.0, None, None),
            origin + Vec2::X * 40.0
        );
    }
}
