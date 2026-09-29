//! Controller intents on the existing gameplay paths.
//!
//! [`resolve_gamepad`] (in `GamepadInputSet`) turns the sampled snapshot into
//! this frame's intents and applies the safety cancels. [`pad_combat`] runs
//! in the combat chain between the touch attack and the attack resolver: it
//! picks the aimed target with the touch aim assist, keeps an R3 lock, and
//! hands skills to `PendingCast`, basic attacks to `BasicAttackState` (never
//! a chase) and upgrades to `NetworkCommand::UpgradeSkill`, exactly as mouse,
//! keyboard and touch do. The hero's analog step is shared with touch
//! (`player::input::move_player_analog`).
// i18n-strict
use bevy::prelude::*;
use shared::{SkillSlot, TargetingMode, ability_for_class_slot, scaled_cast_range};

use super::GamepadControls;
use crate::camera::MainCamera;
use crate::combat::{
    ActionFeedback, CombatStats, PendingCast, TargetCandidates, TargetState,
    mobile_assisted_target, queue_cast_request, upgrade_eligible,
};
use crate::input_context::GameplayInputContext;
use crate::mobile_controls::MobileAttackAim;
use crate::net::{NetworkCommand, NetworkHeroClass, PlayerProgression, SessionEvent};
use crate::player::{MovementRoute, MovementTarget, Player};
use crate::sprite::PlayerVisualMode;
use crate::targeting::{
    BasicAttackState, TargetAimPreview, TargetValidity, aim_cursor, direction_score, pick_mobile,
    projected_position, screen_position,
};
use crate::team::{Team, TeamSelection};

/// Screen pixels a previous aim candidate may trail the best one by and
/// still be kept: absorbs snapshot jitter without trapping a deliberate turn.
pub(crate) const AIM_HYSTERESIS_PX: f32 = 12.0;

pub(crate) fn retain_aim_candidate(previous: Option<f32>, best: Option<f32>) -> bool {
    previous.is_some_and(|previous| best.is_some_and(|best| previous <= best + AIM_HYSTERESIS_PX))
}

/// Intents and safety cancels. A modal, focus loss, a disconnect or new
/// device, death, a round change and East (in play) drop the pad's held and
/// queued actions and its movement; cooldowns are untouched. The gesture
/// then waits for neutral controls before it acts again.
pub(crate) fn resolve_gamepad(
    mut controls: ResMut<GamepadControls>,
    context: Res<GameplayInputContext>,
    time: Res<Time>,
    mut session: MessageReader<SessionEvent>,
    local: Query<(Entity, &CombatStats), With<Player>>,
    mut commands: Commands,
    mut pending: ResMut<PendingCast>,
    mut basic: ResMut<BasicAttackState>,
    mut target: ResMut<TargetState>,
    mut preview: ResMut<TargetAimPreview>,
) {
    let round_changed = session
        .read()
        .any(|event| matches!(event, SessionEvent::RoundChanged { .. }));
    let alive = local.single().is_ok_and(|(_, stats)| stats.is_alive());
    let allowed = controls.active && controls.raw.is_some() && context.gameplay_allowed() && alive;
    let cancel = controls.cancel_actions
        || (controls.active && round_changed)
        || (controls.was_allowed && !allowed)
        || (allowed && controls.back_pressed);
    if cancel {
        controls.cancel_gesture();
        pending.cancel();
        basic.cancel();
        *preview = default();
        target.selected_entity = None;
        target.selected_target = None;
        for (entity, _) in &local {
            commands
                .entity(entity)
                .remove::<(MovementTarget, MovementRoute)>();
        }
    }
    if round_changed {
        controls.gestures = default();
    }
    let pad = controls.raw.unwrap_or_default();
    let intent = controls.gestures.step(pad, allowed, time.delta_secs());
    controls.was_allowed = allowed;
    let live = allowed && controls.gestures.armed;
    controls.movement = if live {
        Vec2::new(pad.left.x, -pad.left.y)
    } else {
        Vec2::ZERO
    };
    controls.aim = (live && pad.right != Vec2::ZERO)
        .then(|| Vec2::new(pad.right.x, -pad.right.y).normalize_or_zero());
    controls.aiming_slot = intent.slot;
    controls.cast = intent.cast;
    controls.upgrade = intent.upgrade;
    controls.attack_held = intent.attack;
    controls.lock_pressed = intent.lock;
}

/// Target choice, lock, preview, skills, upgrades and the held basic attack.
/// A locked target that became invalid cancels the gesture instead of
/// silently switching to another unit.
pub(crate) fn pad_combat(
    mut pad: ResMut<GamepadControls>,
    context: Res<GameplayInputContext>,
    local: Query<
        (
            &Transform,
            &Team,
            &CombatStats,
            Option<&NetworkHeroClass>,
            Option<&PlayerProgression>,
            Option<&crate::net::PlayerLoadout>,
        ),
        With<Player>,
    >,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    view: (Res<PlayerVisualMode>, Res<TeamSelection>),
    candidates: TargetCandidates,
    validity: TargetValidity,
    mut target: ResMut<TargetState>,
    mut preview: ResMut<TargetAimPreview>,
    mut basic: ResMut<BasicAttackState>,
    mut pending: ResMut<PendingCast>,
    mut feedback: ResMut<ActionFeedback>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    let (mode, selection) = view;
    if !pad.active {
        return;
    }
    let (Ok((position, team, stats, class, progression, loadout)), Ok((camera, camera_transform))) =
        (local.single(), camera.single())
    else {
        basic.cancel();
        pad.candidate = None;
        return;
    };
    if !context.gameplay_allowed() || !stats.is_alive() {
        basic.cancel();
        return;
    }
    let class = class.map_or(selection.hero_class, |c| c.0);
    let prog = progression.copied().unwrap_or_default();
    let attack_range = crate::combat::standard::attack_range(class, loadout);
    let slot = pad.aiming_slot.or(pad.cast);
    let skill_range = slot.and_then(|slot| {
        SkillSlot::from_index(slot as u8)
            .map(|s| scaled_cast_range(ability_for_class_slot(class, s), prog.ranks[slot].max(1)))
    });
    if pad.locked
        && target
            .selected_entity
            .zip(target.selected_target)
            .is_none_or(|(entity, id)| !validity.valid(entity, id, *team))
    {
        pad.cancel_gesture();
        basic.cancel();
        *preview = default();
        target.selected_entity = None;
        target.selected_target = None;
        return;
    }
    let aim = pad.aim.map(|direction| MobileAttackAim {
        direction,
        extent: 1.0,
    });
    let mut pick = if pad.locked {
        target.selected_entity.zip(target.selected_target)
    } else if let Some(range) = skill_range {
        mobile_assisted_target(
            position.translation,
            *team,
            range,
            pad.aim,
            &candidates,
            &validity,
            camera,
            camera_transform,
            *mode,
            pad.candidate.map(|p| p.0),
        )
    } else {
        pick_mobile(
            position.translation,
            *team,
            attack_range,
            aim,
            None,
            &candidates,
            &validity,
            camera,
            camera_transform,
            *mode,
        )
        .map(|(entity, id, _)| (entity, id))
    };
    if !pad.locked
        && skill_range.is_none()
        && let (Some(direction), Some((old, old_id)), Some(origin)) = (
            pad.aim,
            pad.candidate,
            projected_position(camera, camera_transform, *mode, position.translation),
        )
        && validity.valid(old, old_id, *team)
    {
        let score = |entity| {
            validity
                .position(entity)
                .and_then(|p| projected_position(camera, camera_transform, *mode, p))
                .and_then(|p| direction_score(origin, direction, p))
        };
        if retain_aim_candidate(score(old), pick.and_then(|p| score(p.0))) {
            pick = Some((old, old_id));
        }
    }
    pad.candidate = pick;
    if pad.lock_pressed {
        pad.locked = !pad.locked && pick.is_some();
        target.selected_entity = pick.map(|p| p.0).filter(|_| pad.locked);
        target.selected_target = pick.map(|p| p.1).filter(|_| pad.locked);
    }
    if let (Some(aim), Some(origin), Some(viewport)) = (
        aim,
        screen_position(camera, camera_transform, *mode, position.translation),
        camera.logical_viewport_size(),
    ) {
        *preview = TargetAimPreview {
            active: true,
            origin,
            cursor: aim_cursor(origin, viewport, aim),
            candidate: pick.map(|p| p.0),
            target: pick.map(|p| p.1),
            candidate_screen: pick
                .and_then(|p| validity.position(p.0))
                .and_then(|p| projected_position(camera, camera_transform, *mode, p)),
            in_attack_range: pick.is_some_and(|(entity, id)| {
                validity.position(entity).is_some_and(|p| {
                    position.translation.xz().distance(p.xz())
                        <= skill_range.unwrap_or(attack_range + validity.radius(entity, id) - 0.08)
                })
            }),
            ..default()
        };
    }
    if let Some(slot) = pad.upgrade.take()
        && slot < 4
        && upgrade_eligible(&prog, slot)
    {
        commands.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
    }
    if let Some(slot) = pad.cast.take() {
        basic.cancel();
        let Some(skill) = SkillSlot::from_index(slot as u8) else {
            return;
        };
        if shared::loadout::preset_for_class(class).is_some() {
            let definition = ability_for_class_slot(class, skill);
            let range = scaled_cast_range(definition, prog.ranks[slot].max(1));
            let direction = pad
                .aim
                .map(|screen| {
                    crate::player::mobile_screen_direction(screen, camera_transform, *mode).xz()
                })
                .unwrap_or_else(|| position.forward().xz())
                .normalize_or_zero();
            let extent = if pad.aim.is_some() {
                pad.raw
                    .map_or(1.0, |raw| raw.right.length().clamp(0.15, 1.0))
            } else {
                1.0
            };
            queue_cast_request(slot, class, &target, &mut pending, &mut feedback);
            pending.aim = Some(position.translation.xz() + direction * range * extent);
            return;
        }
        if ability_for_class_slot(class, skill).targeting == TargetingMode::UnitTarget {
            // The adapter just resolved the displayed candidate (or the lock).
            let Some((entity, id)) =
                pick.filter(|(entity, id)| validity.valid(*entity, *id, *team))
            else {
                feedback.push_line(crate::i18n::tr("combat.attack.no_enemy_aimed"));
                return;
            };
            let request_target = TargetState::for_request(entity, id);
            queue_cast_request(slot, class, &request_target, &mut pending, &mut feedback);
        } else {
            queue_cast_request(slot, class, &target, &mut pending, &mut feedback);
        }
        return;
    }
    if !pad.attack_held || slot.is_some() || pending.is_pending() {
        basic.cancel();
        return;
    }
    // The resolver's cooldown is the only attack cadence: never queue during
    // it, and never chase from analog controls.
    if basic.remaining_secs > 0.0 {
        return;
    }
    if let Some((entity, id)) = pick {
        if !pad.locked {
            target.selected_entity = Some(entity);
            target.selected_target = Some(id);
        }
        basic.start(entity, id, false);
    } else {
        basic.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aim_hysteresis_keeps_small_jitter_but_releases_new_direction() {
        assert!(retain_aim_candidate(Some(20.0), Some(14.0)));
        assert!(retain_aim_candidate(Some(26.0), Some(14.0)), "12 px band");
        assert!(!retain_aim_candidate(Some(26.5), Some(14.0)));
        assert!(!retain_aim_candidate(Some(80.0), Some(14.0)));
        assert!(!retain_aim_candidate(None, Some(14.0)));
        assert!(!retain_aim_candidate(Some(14.0), None));
    }
}
