use crate::domain::CombatStats;
use crate::input_bindings::SKILL_CAST_KEYS;
use crate::input_context::GameplayInputContext;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkPlayerId,
    PlayerProgression, TargetId, TargetKind,
};
use crate::player::{MovementTarget, Player};
use crate::team::{Team, TeamSelection};
use bevy::prelude::*;
use shared::{
    HeroClass, SkillSlot, TargetingMode, ability_for_class_slot, scaled_cast_range,
    scaled_cooldown, scaled_mana_cost,
};

use super::cooldown::{LocalCastCooldown, effective_cast_duration, local_hero_class};
use super::feedback::ActionFeedback;
use super::selection::TargetState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingCastRequest {
    pub(super) slot: usize,
    pub(super) target_entity: Option<Entity>,
    pub(super) target: Option<TargetId>,
    pub(super) approach_announced: bool,
}

#[derive(Resource, Default)]
pub(crate) struct PendingCast {
    pub(super) request: Option<PendingCastRequest>,
}

impl PendingCast {
    pub(crate) fn is_pending(&self) -> bool {
        self.request.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.request = None;
    }

    #[cfg(test)]
    pub(crate) fn queued_for_movement_test() -> Self {
        Self {
            request: Some(PendingCastRequest {
                slot: 0,
                target_entity: None,
                target: None,
                approach_announced: true,
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn has_queued_request(&self) -> bool {
        self.request.is_some()
    }
}

/// Resolves the target and sends a slot cast for the local player's class kit.
/// Client-side checks (unlock level, local cooldown, target presence) exist for
/// responsive UX only; the server re-validates everything authoritatively.
fn try_cast_slot(
    slot_index: usize,
    class: HeroClass,
    local: (&CombatStats, PlayerProgression, Option<&NetworkPlayerId>),
    selected_target: Option<TargetId>,
    command_writer: &mut MessageWriter<NetworkCommand>,
    feedback: &mut ActionFeedback,
    cast_cd: &mut LocalCastCooldown,
) -> bool {
    let Some(slot) = SkillSlot::from_index(slot_index as u8) else {
        return false;
    };
    let (stats, prog, net_id) = local;
    if !stats.is_alive() {
        return false;
    }
    let def = ability_for_class_slot(class, slot);
    if !prog.unlocked()[slot.index()] {
        let message = format!(
            "{} is locked until level {}.",
            def.name,
            shared::SLOT_UNLOCK_LEVELS[slot.index()]
        );
        feedback.push_line(message.clone());
        info!("{message}");
        return false;
    }
    if cast_cd.recovery_secs > 0.0 {
        return false;
    }
    if cast_cd.remaining_secs[slot.index()] > 0.0 {
        let message = format!(
            "{} is cooling down for {:.1}s.",
            def.name,
            cast_cd.remaining_secs[slot.index()]
        );
        feedback.push_line(message.clone());
        info!("{message}");
        return false;
    }

    let target = match def.targeting {
        TargetingMode::SelfTarget => net_id.map(|id| TargetId {
            kind: TargetKind::Player,
            id: id.0,
        }),
        TargetingMode::UnitTarget => selected_target,
    };
    let Some(target) = target else {
        let message = match def.targeting {
            TargetingMode::UnitTarget => {
                "No target available. Click or tap an enemy, or use Tab to select."
            }
            TargetingMode::SelfTarget => "Not connected yet; self-cast unavailable.",
        };
        feedback.push_line(message);
        info!("{message}");
        return false;
    };

    let rank = prog.ranks[slot.index()].clamp(1, def.max_rank);
    let mana_cost = scaled_mana_cost(def, rank);
    if stats.mana < mana_cost {
        let message = format!(
            "Not enough mana for {} ({:.0}/{:.0}).",
            def.name, stats.mana, mana_cost
        );
        feedback.push_line(message.clone());
        info!("{message}");
        return false;
    }
    cast_cd.remaining_secs[slot.index()] = scaled_cooldown(def, rank).as_secs_f32();
    cast_cd.total_secs[slot.index()] = cast_cd.remaining_secs[slot.index()];
    command_writer.write(NetworkCommand::Cast {
        target,
        slot: slot.index() as u8,
    });
    let message = format!("Casting {}.", def.name);
    feedback.push_line(message.clone());
    info!("{message}");
    true
}

pub(super) fn queue_cast_request(
    slot_index: usize,
    class: HeroClass,
    target_state: &TargetState,
    pending_cast: &mut PendingCast,
    feedback: &mut ActionFeedback,
) {
    let Some(slot) = SkillSlot::from_index(slot_index as u8) else {
        return;
    };
    let def = ability_for_class_slot(class, slot);
    let (target_entity, target) = match def.targeting {
        TargetingMode::SelfTarget => (None, None),
        TargetingMode::UnitTarget => {
            let (Some(entity), Some(target)) =
                (target_state.selected_entity, target_state.selected_target)
            else {
                let message = "No target available. Click or tap an enemy, or use Tab to select.";
                feedback.push_line(message);
                info!("{message}");
                return;
            };
            (Some(entity), Some(target))
        }
    };
    pending_cast.request = Some(PendingCastRequest {
        slot: slot_index,
        target_entity,
        target,
        approach_announced: false,
    });
}

pub(super) fn cast_spell_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    game_state: Option<Res<GameStateSnapshot>>,
    team_selection: Res<TeamSelection>,
    local_player: Query<
        (
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkPlayerId>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    target_state: Res<TargetState>,
    mut pending_cast: ResMut<PendingCast>,
    mut feedback: ResMut<ActionFeedback>,
    context: Res<GameplayInputContext>,
) {
    if !context.gameplay_allowed() {
        return;
    }

    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Some(slot_index) = SKILL_CAST_KEYS
        .iter()
        .position(|key| keyboard_input.just_pressed(*key))
    else {
        return;
    };

    let Ok((_stats, _prog, _net_id, class)) = local_player.single() else {
        return;
    };
    let class = local_hero_class(Some(class), &team_selection);
    queue_cast_request(
        slot_index,
        class,
        &target_state,
        &mut pending_cast,
        &mut feedback,
    );
}

pub(super) fn resolve_pending_cast_system(
    mut commands: Commands,
    team_selection: Res<TeamSelection>,
    local_player: Query<
        (
            Entity,
            &Transform,
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkPlayerId>,
            Option<&NetworkHeroClass>,
            &Team,
        ),
        With<Player>,
    >,
    target_query: Query<(&Transform, &CombatStats), Without<Player>>,
    mut pending_cast: ResMut<PendingCast>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut feedback: ResMut<ActionFeedback>,
    mut cast_cd: ResMut<LocalCastCooldown>,
    context: Res<GameplayInputContext>,
    protection: Query<&crate::net::NetworkStructureProtected>,
    equipment: Query<&crate::net::PlayerEquipment, With<Player>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    validity: crate::targeting::TargetValidity,
    game: Option<Res<GameStateSnapshot>>,
) {
    let touch_mode = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    if !context.gameplay_allowed() {
        pending_cast.cancel();
        return;
    }

    let Some(request) = pending_cast.request else {
        return;
    };
    let Ok((player_entity, player_transform, stats, progression, net_id, class, team)) =
        local_player.single()
    else {
        pending_cast.cancel();
        return;
    };
    let class = local_hero_class(Some(class), &team_selection);
    let Some(slot) = SkillSlot::from_index(request.slot as u8) else {
        pending_cast.cancel();
        return;
    };
    let definition = ability_for_class_slot(class, slot);
    let prog = progression.copied().unwrap_or_default();
    let rank = prog.ranks[slot.index()].clamp(1, definition.max_rank);
    let rejection = if !stats.is_alive() {
        Some("Wait for respawn.".to_string())
    } else if !prog.unlocked()[slot.index()] {
        Some(format!(
            "{} unlocks at level {}.",
            definition.name,
            shared::SLOT_UNLOCK_LEVELS[slot.index()]
        ))
    } else if cast_cd.remaining_secs[slot.index()] > 0.0 {
        Some(format!(
            "{} ready in {:.1}s.",
            definition.name,
            cast_cd.remaining_secs[slot.index()]
        ))
    } else if stats.mana < scaled_mana_cost(definition, rank) {
        Some(format!(
            "Not enough mana for {} ({:.0}/{:.0}).",
            definition.name,
            stats.mana,
            scaled_mana_cost(definition, rank)
        ))
    } else if request
        .target_entity
        .and_then(|entity| protection.get(entity).ok())
        .is_some_and(|protected| protected.0)
    {
        Some("Structure protected — destroy the preceding lane towers first.".to_string())
    } else if definition.targeting == TargetingMode::UnitTarget
        && request
            .target_entity
            .zip(request.target)
            .is_none_or(|(entity, id)| !validity.valid(entity, id, *team))
    {
        Some("Target is no longer a visible hostile unit.".to_string())
    } else {
        None
    };
    if let Some(message) = rejection {
        feedback.push_line(message);
        pending_cast.cancel();
        commands
            .entity(player_entity)
            .remove::<(MovementTarget, crate::player::MovementRoute)>();
        return;
    }

    // Buffer the latest skill through the short shared recovery window.
    if cast_cd.recovery_secs > 0.0 {
        return;
    }
    if definition.targeting == TargetingMode::UnitTarget {
        let (Some(target_entity), Some(_target)) = (request.target_entity, request.target) else {
            pending_cast.cancel();
            return;
        };
        let Ok((target_transform, target_stats)) = target_query.get(target_entity) else {
            pending_cast.cancel();
            commands
                .entity(player_entity)
                .remove::<(MovementTarget, crate::player::MovementRoute)>();
            return;
        };
        if !target_stats.is_alive() {
            pending_cast.cancel();
            return;
        }
        let progression = progression.copied().unwrap_or_default();
        let rank = progression.ranks[slot.index()].clamp(1, definition.max_rank);
        let cast_range = scaled_cast_range(definition, rank);
        if !within_cast_range(
            player_transform.translation,
            target_transform.translation,
            cast_range,
        ) {
            if touch_mode {
                feedback.push_line("Target out of range — move closer.");
                pending_cast.cancel();
                return;
            }
            commands.entity(player_entity).insert(MovementTarget {
                target: target_transform.translation,
            });
            if !request.approach_announced {
                let message = format!("Approaching target for {}.", definition.name);
                feedback.push_line(message.clone());
                info!("{message}");
                if let Some(request) = pending_cast.request.as_mut() {
                    request.approach_announced = true;
                }
            }
            return;
        }
    }

    commands.entity(player_entity).remove::<MovementTarget>();
    let sent = try_cast_slot(
        request.slot,
        class,
        (stats, progression.copied().unwrap_or_default(), net_id),
        request.target,
        &mut command_writer,
        &mut feedback,
        &mut cast_cd,
    );
    if sent {
        let bonuses = equipment
            .single()
            .map(|equipment| equipment.item_bonuses)
            .unwrap_or_default();
        cast_cd.remaining_secs[slot.index()] = effective_cast_duration(
            class,
            prog.level,
            rank,
            slot,
            bonuses,
            game.as_ref().is_some_and(|g| g.sandbox.is_some()),
        );
        cast_cd.total_secs[slot.index()] = cast_cd.remaining_secs[slot.index()];
        let no_cooldowns = game
            .as_ref()
            .and_then(|g| g.sandbox.as_ref())
            .is_some_and(|s| s.config.player.no_cooldowns);
        if no_cooldowns {
            cast_cd.remaining_secs[slot.index()] = 0.0;
        }
        cast_cd.recovery_secs = if no_cooldowns {
            0.0
        } else {
            shared::hero_balance::skill_recovery_secs(prog.level)
        };
        cast_cd.pending_slot = Some(slot.index());
        cast_cd.prediction_grace_secs = 0.3;
    }
    pending_cast.cancel();
}

pub(super) fn within_cast_range(
    local_position: Vec3,
    target_position: Vec3,
    cast_range: f32,
) -> bool {
    local_position.xz().distance(target_position.xz()) <= cast_range
}
