// i18n-strict
use crate::domain::CombatStats;
use crate::i18n::{LocaleId, data, tr, tr_in, trf, trf_in};
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
    AbilityDefinition, HeroClass, SkillSlot, TargetingMode, ability_for_class_slot,
    scaled_cast_range, scaled_cooldown, scaled_mana_cost,
};
use std::fmt::Display;

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
    pub(crate) aim: Option<Vec2>,
}

impl PendingCast {
    pub(crate) fn is_pending(&self) -> bool {
        self.request.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.request = None;
        self.aim = None;
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
            aim: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn has_queued_request(&self) -> bool {
        self.request.is_some()
    }
}

/// A cast message about `ability` for the player, in the active language; the
/// log keeps English. `{ability}` is filled from the ability, `args` fill the rest.
fn ability_message(
    key: &'static str,
    ability: &AbilityDefinition,
    args: &[(&str, &dyn Display)],
) -> (String, String) {
    let shown = data::ability_name(ability);
    let mut display: Vec<(&str, &dyn Display)> = vec![("ability", &shown)];
    display.extend_from_slice(args);
    let mut english: Vec<(&str, &dyn Display)> = vec![("ability", &ability.name)];
    english.extend_from_slice(args);
    (trf(key, &display), trf_in(key, LocaleId::ENGLISH, &english))
}

/// Shows `key` about `ability` and logs it in English.
fn report(
    feedback: &mut ActionFeedback,
    key: &'static str,
    ability: &AbilityDefinition,
    args: &[(&str, &dyn Display)],
) {
    let (shown, english) = ability_message(key, ability, args);
    feedback.push_line(shown);
    info!("{english}");
}

/// Shows a fixed message and logs it in English.
fn report_plain(feedback: &mut ActionFeedback, key: &'static str) {
    feedback.push_line(tr(key));
    info!("{}", tr_in(key, LocaleId::ENGLISH));
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
        report(
            feedback,
            "combat.cast.locked",
            def,
            &[("level", &shared::SLOT_UNLOCK_LEVELS[slot.index()])],
        );
        return false;
    }
    if cast_cd.recovery_secs > 0.0 {
        return false;
    }
    if cast_cd.remaining_secs[slot.index()] > 0.0 {
        let seconds = format!("{:.1}", cast_cd.remaining_secs[slot.index()]);
        report(
            feedback,
            "combat.cast.cooling_down",
            def,
            &[("seconds", &seconds)],
        );
        return false;
    }

    let target = match def.targeting {
        TargetingMode::SelfTarget => net_id.map(|id| TargetId {
            kind: TargetKind::Player,
            id: id.0,
        }),
        TargetingMode::UnitTarget => selected_target,
        TargetingMode::Direction | TargetingMode::Point => None,
    };
    let Some(target) = target else {
        report_plain(
            feedback,
            match def.targeting {
                TargetingMode::UnitTarget | TargetingMode::Direction | TargetingMode::Point => {
                    "combat.cast.no_target"
                }
                TargetingMode::SelfTarget => "combat.cast.not_connected",
            },
        );
        return false;
    };

    let rank = prog.ranks[slot.index()].clamp(1, def.max_rank);
    let mana_cost = scaled_mana_cost(def, rank);
    if stats.mana < mana_cost {
        let (mana, cost) = (format!("{:.0}", stats.mana), format!("{mana_cost:.0}"));
        report(
            feedback,
            "combat.cast.no_mana",
            def,
            &[("mana", &mana), ("cost", &cost)],
        );
        return false;
    }
    cast_cd.remaining_secs[slot.index()] = scaled_cooldown(def, rank).as_secs_f32();
    cast_cd.total_secs[slot.index()] = cast_cd.remaining_secs[slot.index()];
    command_writer.write(NetworkCommand::Cast {
        target,
        slot: slot.index() as u8,
    });
    info!("Casting {}", def.name);
    true
}

pub(crate) fn queue_cast_request(
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
        TargetingMode::SelfTarget | TargetingMode::Direction | TargetingMode::Point => (None, None),
        TargetingMode::UnitTarget => {
            let (Some(entity), Some(target)) =
                (target_state.selected_entity, target_state.selected_target)
            else {
                report_plain(feedback, "combat.cast.no_target");
                return;
            };
            (Some(entity), Some(target))
        }
    };
    pending_cast.aim = None;
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
    mut aimed: Local<[bool; 4]>,
) {
    if !context.gameplay_allowed() {
        *aimed = [false; 4];
        return;
    }

    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Ok((_stats, _prog, _net_id, class)) = local_player.single() else {
        return;
    };
    let class = local_hero_class(Some(class), &team_selection);
    let slot_index = if class.is_standard() {
        let mut released = None;
        for (slot, key) in SKILL_CAST_KEYS.iter().enumerate() {
            if keyboard_input.just_pressed(*key) {
                aimed[slot] = true;
            }
            if keyboard_input.just_released(*key) {
                if aimed[slot] {
                    released = Some(slot);
                }
                aimed[slot] = false;
            }
        }
        released
    } else {
        *aimed = [false; 4];
        SKILL_CAST_KEYS
            .iter()
            .position(|key| keyboard_input.just_pressed(*key))
    };
    let Some(slot_index) = slot_index else {
        return;
    };
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
    sticks: (
        Option<Res<crate::mobile_controls::MobileControls>>,
        Option<Res<crate::gamepad::GamepadControls>>,
    ),
    validity: crate::targeting::TargetValidity,
    game: Option<Res<GameStateSnapshot>>,
    loadouts: Query<&crate::net::PlayerLoadout, With<Player>>,
    aim_view: (
        Query<&Window, With<bevy::window::PrimaryWindow>>,
        Query<(&Camera, &GlobalTransform), With<crate::camera::MainCamera>>,
        Option<Res<crate::sprite::PlayerVisualMode>>,
    ),
) {
    // Touch and controller casts never walk to an out-of-range target.
    let touch_mode = sticks.0.as_ref().is_some_and(|mobile| mobile.enabled)
        || sticks.1.as_ref().is_some_and(|pad| pad.active);
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
    if let Some(preset) = shared::loadout::preset_for_class(class) {
        let state = loadouts.single().ok().and_then(|s| s.0.as_ref());
        let cursor = aim_view
            .0
            .single()
            .ok()
            .filter(|w| w.focused)
            .and_then(|w| w.cursor_position())
            .and_then(|p| {
                let (camera, pose) = aim_view.1.single().ok()?;
                crate::player::viewport_to_simulation_world(
                    camera,
                    pose,
                    p,
                    aim_view
                        .2
                        .as_deref()
                        .copied()
                        .unwrap_or(crate::sprite::PlayerVisualMode::Models3d),
                    0.0,
                )
            })
            .map(|p| p.xz());
        let aim = pending_cast.aim.or(cursor).unwrap_or_else(|| {
            player_transform.translation.xz() + player_transform.forward().xz() * 10.0
        });
        let recast = state.is_some_and(|s| s.slots[slot.index()].can_recast);
        if !stats.is_alive() || !prog.unlocked()[slot.index()] || !aim.is_finite() {
            pending_cast.cancel();
            return;
        }
        if !recast && cast_cd.recovery_secs > 0.0 {
            return;
        }
        if !recast
            && (cast_cd.remaining_secs[slot.index()] > 0.0
                || stats.mana < scaled_mana_cost(definition, rank))
        {
            report_plain(
                &mut feedback,
                if stats.mana < scaled_mana_cost(definition, rank) {
                    "combat.hotbar.need_mana"
                } else {
                    "combat.standard.not_ready"
                },
            );
            pending_cast.cancel();
            return;
        }
        let aim = super::standard::bounded_aim(
            player_transform.translation.xz(),
            aim,
            definition.targeting,
            scaled_cast_range(definition, rank),
        );
        command_writer.write(NetworkCommand::CastSkill {
            slot: slot.index() as u8,
            aim,
        });
        commands
            .entity(player_entity)
            .remove::<(MovementTarget, crate::player::MovementRoute)>();
        if !recast {
            let no_cooldowns = game
                .as_ref()
                .and_then(|g| g.sandbox.as_ref())
                .is_some_and(|s| s.config.player.no_cooldowns);
            let skill = preset.skill(slot);
            cast_cd.recovery_secs = if no_cooldowns {
                0.0
            } else {
                skill.windup_secs.max(
                    if matches!(
                        skill.effect,
                        shared::loadout::SkillEffect::WeaponToggle { .. }
                    ) {
                        0.0
                    } else {
                        0.15
                    },
                )
            };
            let bonuses = equipment
                .single()
                .map(|e| e.item_bonuses)
                .unwrap_or_default();
            let duration = if no_cooldowns {
                0.0
            } else {
                effective_cast_duration(
                    class,
                    prog.level,
                    rank,
                    slot,
                    bonuses,
                    game.as_ref().is_some_and(|g| g.sandbox.is_some()),
                )
            };
            cast_cd.remaining_secs[slot.index()] = duration;
            cast_cd.total_secs[slot.index()] = duration;
            cast_cd.pending_slot = Some(slot.index());
            cast_cd.prediction_grace_secs = 0.3;
        }
        pending_cast.cancel();
        return;
    }
    let rejection = if !stats.is_alive() {
        Some(tr("combat.cast.wait_respawn").to_string())
    } else if !prog.unlocked()[slot.index()] {
        Some(
            ability_message(
                "combat.cast.unlocks_at",
                definition,
                &[("level", &shared::SLOT_UNLOCK_LEVELS[slot.index()])],
            )
            .0,
        )
    } else if cast_cd.remaining_secs[slot.index()] > 0.0 {
        let seconds = format!("{:.1}", cast_cd.remaining_secs[slot.index()]);
        Some(ability_message("combat.cast.ready_in", definition, &[("seconds", &seconds)]).0)
    } else if stats.mana < scaled_mana_cost(definition, rank) {
        let mana = format!("{:.0}", stats.mana);
        let cost = format!("{:.0}", scaled_mana_cost(definition, rank));
        Some(
            ability_message(
                "combat.cast.no_mana",
                definition,
                &[("mana", &mana), ("cost", &cost)],
            )
            .0,
        )
    } else if request
        .target_entity
        .and_then(|entity| protection.get(entity).ok())
        .is_some_and(|protected| protected.0)
    {
        Some(tr("combat.cast.protected").to_string())
    } else if definition.targeting == TargetingMode::UnitTarget
        && request
            .target_entity
            .zip(request.target)
            .is_none_or(|(entity, id)| !validity.valid(entity, id, *team))
    {
        Some(tr("combat.cast.target_gone").to_string())
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
                feedback.push_line(tr("combat.cast.out_of_range"));
                pending_cast.cancel();
                return;
            }
            commands.entity(player_entity).insert(MovementTarget {
                target: target_transform.translation,
            });
            if !request.approach_announced {
                report(&mut feedback, "combat.cast.approaching", definition, &[]);
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
