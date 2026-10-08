// i18n-strict
use crate::equipped_skills;
use crate::net::{GameStateSnapshot, NetworkHeroClass, PlayerProgression};
use crate::player::Player;
use crate::skill_presentation::status;
use crate::team::TeamSelection;
use bevy::prelude::*;
use shared::loadout::{SkillDefinition, SkillEffect, SkillEffectState, SkillSlotState};
use shared::{HeroClass, SkillSlot};

/// Seconds a recast command may be on its way to the server past the snapshot the client
/// stands on.
const RECAST_SEND_LEAD_SECS: f32 = 0.1;

/// Whether a press of the slot is a recast the server accepts where the hero stands. The
/// replicated flag only says that the recast window is open; a recast with a gate (Mountain
/// Echo) is also refused away from the skill's own effect, with the rule the recast marker
/// and the aim preview already follow (`status::recast_in_reach`).
pub(crate) fn recast_usable(
    def: &SkillDefinition,
    slot: &SkillSlotState,
    hero: Vec2,
    your_id: u64,
    effects: &[SkillEffectState],
) -> bool {
    slot.can_recast && status::recast_in_reach(def.id, your_id, hero, effects)
}

/// The same question for the press that is about to be sent. The server answers it later
/// than the snapshot shows, and the body of the skill travels meanwhile: an effect that
/// closes the gap within `RECAST_SEND_LEAD_SECS` still sends, so that a press the server
/// would take is not refused here. Nothing shown to the player uses this wider reach.
pub(super) fn recast_sendable(
    def: &SkillDefinition,
    slot: &SkillSlotState,
    hero: Vec2,
    your_id: u64,
    effects: &[SkillEffectState],
) -> bool {
    let lead = match def.effect {
        SkillEffect::Technique { speed, .. } => speed.max(0.0) * RECAST_SEND_LEAD_SECS,
        _ => 0.0,
    };
    recast_usable(def, slot, hero, your_id, effects)
        || slot.can_recast
            && effects.iter().any(|effect| {
                let closer = hero.move_towards(Vec2::from_array(effect.position), lead);
                status::recast_in_reach(def.id, your_id, closer, std::slice::from_ref(effect))
            })
}

/// Local per-slot cast cooldown mirror for HUD feedback (the server remains
/// authoritative; values come from the shared class kit numbers).
#[derive(Resource, Default)]
pub struct LocalCastCooldown {
    pub remaining_secs: [f32; 4],
    pub(crate) recast: [bool; 4],
    pub(crate) recast_secs: [f32; 4],
    /// Total duration last applied to each active cooldown. Lets an equipment
    /// or rank update change the deadline without rescaling elapsed time.
    pub(super) total_secs: [f32; 4],
    pub(super) recovery_secs: f32,
    pub(super) pending_slot: Option<usize>,
    pub(super) prediction_grace_secs: f32,
}

impl LocalCastCooldown {
    /// A client-only cooldown for a QA layout fixture (the HUD's sweep).
    #[cfg(feature = "qa")]
    pub(crate) fn set_for_qa(&mut self, slot: usize, remaining: f32, total: f32) {
        self.remaining_secs[slot] = remaining;
        self.total_secs[slot] = total;
    }

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
            Option<&crate::net::PlayerLoadout>,
            Option<&Transform>,
            Option<&crate::net::NetworkPlayerId>,
        ),
        With<Player>,
    >,
    mut cooldowns: ResMut<LocalCastCooldown>,
) {
    let Ok((progression, class, equipment, authoritative, loadout, pose, id)) = player.single()
    else {
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
    if let Some(snapshot) = authoritative.as_ref().filter(|s| {
        s.is_changed()
            || (cooldowns.pending_slot.is_some() && cooldowns.prediction_grace_secs <= 0.0)
    }) {
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
    cooldowns.recast = [false; 4];
    cooldowns.recast_secs = [0.0; 4];
    let skills = equipped_skills::resolve(class.0, loadout);
    if let Some(state) = loadout.and_then(|l| l.0.as_ref()) {
        let hero = id
            .map(|id| id.0)
            .or(game.as_ref().map(|game| game.your_id))
            .unwrap_or(0);
        let effects = game.as_ref().map_or(&[][..], |game| &game.skill_effects);
        for (i, slot) in state.slots.iter().enumerate() {
            // An open recast window the server would refuse where the hero stands is not
            // offered: the slot keeps the cooldown that is really running.
            let usable = match (
                skills
                    .as_ref()
                    .and_then(|skills| skills.skill(SkillSlot::ALL[i])),
                pose,
            ) {
                (Some(def), Some(pose)) => {
                    recast_usable(def, slot, pose.translation.xz(), hero, effects)
                }
                _ => slot.can_recast,
            };
            cooldowns.recast[i] = usable;
            cooldowns.recast_secs[i] = slot.recast_remaining_secs;
            if usable {
                cooldowns.remaining_secs[i] = 0.0;
            }
        }
    }
    let Some(skills) = skills else {
        cooldowns.total_secs = [0.0; 4];
        return;
    };
    for slot in SkillSlot::ALL {
        let index = slot.index();
        let duration = equipped_skills::cooldown(
            &skills,
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

#[cfg(test)]
pub(crate) fn effective_cast_duration(
    class: HeroClass,
    level: u32,
    rank: u8,
    slot: SkillSlot,
    bonuses: shared::shop::ItemBonuses,
    sandbox: bool,
) -> f32 {
    let skills = equipped_skills::resolve(class, None).expect("preset or legacy kit");
    equipped_skills::cooldown(&skills, level, rank, slot, bonuses, sandbox)
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
