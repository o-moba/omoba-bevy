use crate::net::{GameStateSnapshot, NetworkHeroClass, PlayerProgression};
use crate::player::Player;
use crate::team::TeamSelection;
use bevy::prelude::*;
use shared::{HeroClass, SkillSlot};

/// Local per-slot cast cooldown mirror for HUD feedback (the server remains
/// authoritative; values come from the shared class kit numbers).
#[derive(Resource, Default)]
pub struct LocalCastCooldown {
    pub remaining_secs: [f32; 4],
    /// Total duration last applied to each active cooldown. Lets an equipment
    /// or rank update change the deadline without rescaling elapsed time.
    pub(super) total_secs: [f32; 4],
    pub(super) recovery_secs: f32,
    pub(super) pending_slot: Option<usize>,
    pub(super) prediction_grace_secs: f32,
}

impl LocalCastCooldown {
    pub(crate) fn remaining_fraction(&self, slot: usize) -> f32 {
        if self.total_secs[slot] > 0.0 {
            (self.remaining_secs[slot] / self.total_secs[slot]).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

pub(super) fn tick_local_cast_cooldown(
    time: Res<Time>,
    game: Option<Res<GameStateSnapshot>>,
    mut cd: ResMut<LocalCastCooldown>,
) {
    let elapsed = time.delta_secs() * crate::sandbox::time_scale(game.as_deref());
    cd.recovery_secs = (cd.recovery_secs - elapsed).max(0.0);
    cd.prediction_grace_secs = (cd.prediction_grace_secs - elapsed).max(0.0);
    for remaining in cd.remaining_secs.iter_mut() {
        if *remaining > 0.0 {
            *remaining = (*remaining
                - time.delta_secs() * crate::sandbox::time_scale(game.as_deref()))
            .max(0.0);
        }
    }
}

/// Mirror the server's deadline: last cast time + current kit/item duration.
/// Reducing haste after elapsed time must subtract the duration difference,
/// rather than multiplying the remaining time (which incorrectly rescales history).
pub(super) fn sync_authoritative_cooldown_durations(
    game: Option<Res<GameStateSnapshot>>,
    player: Query<
        (
            &PlayerProgression,
            &NetworkHeroClass,
            &crate::net::PlayerEquipment,
            Option<Ref<crate::net::PlayerSkillCooldowns>>,
        ),
        With<Player>,
    >,
    mut cooldowns: ResMut<LocalCastCooldown>,
) {
    let Ok((progression, class, equipment, authoritative)) = player.single() else {
        return;
    };
    let sandbox_actor = game
        .as_ref()
        .and_then(|g| g.sandbox.as_ref())
        .and_then(|s| {
            s.actors
                .iter()
                .find(|a| a.actor == shared::sandbox::SandboxActor::Player)
        });
    if let Some(snapshot) = authoritative.as_ref().filter(|s| s.is_changed()) {
        // Ignore a pre-cast snapshot briefly while waiting for the accepted slot.
        // A rejection still corrects the optimistic UI after this bounded grace.
        let pending = cooldowns.pending_slot.is_some_and(|slot| {
            cooldowns.prediction_grace_secs > 0.0 && snapshot.remaining_secs[slot] == 0.0
        });
        if !pending {
            cooldowns.remaining_secs = snapshot.remaining_secs;
            cooldowns.recovery_secs = snapshot.recovery_secs;
            cooldowns.pending_slot = None;
        }
    }
    if let Some(actor) = sandbox_actor {
        cooldowns.remaining_secs = actor.cooldowns;
    }
    for slot in SkillSlot::ALL {
        let index = slot.index();
        let duration = effective_cast_duration(
            class.0,
            progression.level,
            progression.ranks[index],
            slot,
            equipment.item_bonuses,
            sandbox_actor.is_some(),
        );
        if authoritative.is_none()
            && sandbox_actor.is_none()
            && cooldowns.remaining_secs[index] > 0.0
            && cooldowns.total_secs[index] > 0.0
            && cooldowns.total_secs[index] != duration
        {
            cooldowns.remaining_secs[index] =
                (cooldowns.remaining_secs[index] + duration - cooldowns.total_secs[index]).max(0.0);
        }
        cooldowns.total_secs[index] = duration;
    }
}

pub(crate) fn effective_cast_duration(
    class: HeroClass,
    level: u32,
    rank: u8,
    slot: SkillSlot,
    mut bonuses: shared::shop::ItemBonuses,
    sandbox: bool,
) -> f32 {
    if !sandbox {
        bonuses.attack_speed_multiplier = bonuses.attack_speed_multiplier.max(1.0);
        bonuses.spell_haste_multiplier = bonuses.spell_haste_multiplier.max(1.0);
    }
    shared::hero_balance::ability_cooldown(class, level, rank, slot, bonuses).as_secs_f32()
}

/// The class whose kit drives the local HUD: server-replicated when available,
/// otherwise the pre-join selection.
pub(super) fn local_hero_class(
    replicated: Option<Option<&NetworkHeroClass>>,
    selection: &TeamSelection,
) -> HeroClass {
    replicated
        .flatten()
        .map(|class| class.0)
        .unwrap_or(selection.hero_class)
}
