//! Authoritative hero clocks and the pure reads over them.
//!
//! The instants live on `ConnectedPlayer`; every remaining-seconds number the
//! request handlers gate on, the sandbox telemetry shows or the replicated
//! `PlayerState` view carries is computed here from those instants and the
//! tick's `now`. `normalize_hero_timers` is the one place that rewrites the
//! instants from derived conditions (death, sandbox `no_cooldowns`).

use std::time::{Duration, Instant};

use shared::SkillSlot;

use crate::entities::ConnectedPlayer;
use crate::game_world::GameWorld;
use crate::hero_stats;

/// The instants a hero's gameplay clocks are measured from. Every
/// remaining-seconds number is derived from these and the tick's `now`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeroTimers {
    /// Last accepted movement, dash or teleport; bounds the next move.
    pub(crate) last_movement_at: Instant,
    /// Per-slot cast timestamps (Q/W/E/R); each ability cools down independently.
    pub(crate) last_cast_at: [Option<Instant>; 4],
    /// Independent of Q/W/E/R and never charged against mana.
    pub(crate) last_basic_attack_at: Option<Instant>,
    pub(crate) dash_ready_at: Option<Instant>,
    pub(crate) haste_ready_at: Option<Instant>,
    pub(crate) haste_expires_at: Option<Instant>,
    pub(crate) respawn_at: Option<Instant>,
}

impl HeroTimers {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            last_movement_at: now,
            last_cast_at: [None; 4],
            last_basic_attack_at: None,
            dash_ready_at: None,
            haste_ready_at: None,
            haste_expires_at: None,
            respawn_at: None,
        }
    }

    /// Every cooldown reads as ready; active haste and the respawn clock stay.
    pub(crate) fn clear_cooldowns(&mut self) {
        self.last_cast_at = [None; 4];
        self.last_basic_attack_at = None;
        self.dash_ready_at = None;
        self.haste_ready_at = None;
    }
}

fn remaining_until(deadline: Option<Instant>, now: Instant) -> f32 {
    deadline.map_or(0.0, |until| {
        until.saturating_duration_since(now).as_secs_f32()
    })
}

fn remaining_of(started: Option<Instant>, duration: Duration, now: Instant) -> f32 {
    started.map_or(0.0, |at| {
        duration
            .saturating_sub(now.saturating_duration_since(at))
            .as_secs_f32()
    })
}

fn no_cooldowns(player: &ConnectedPlayer) -> bool {
    player.modifiers.no_cooldowns
}

/// Dead heroes and sandbox actors without cooldowns report every combat
/// clock as ready.
fn combat_clocks_suspended(player: &ConnectedPlayer) -> bool {
    player.hero.hp <= 0.0 || no_cooldowns(player)
}

/// Full basic-attack cooldown for the hero's current class, level and gear.
pub(crate) fn basic_attack_cooldown(player: &ConnectedPlayer) -> f32 {
    hero_stats::basic_attack_cooldown(player).as_secs_f32()
}

pub(crate) fn basic_attack_remaining(player: &ConnectedPlayer, now: Instant) -> f32 {
    if combat_clocks_suspended(player) {
        return 0.0;
    }
    remaining_of(
        player.timers.last_basic_attack_at,
        hero_stats::basic_attack_cooldown(player),
        now,
    )
}

/// Cooldown left on `slot` regardless of death; the sandbox telemetry keeps
/// showing a dead actor's clocks.
pub(crate) fn skill_cooldown_left(player: &ConnectedPlayer, slot: SkillSlot, now: Instant) -> f32 {
    remaining_of(
        player.timers.last_cast_at[slot.index()],
        hero_stats::ability_cooldown(player, slot),
        now,
    )
}

pub(crate) fn skill_cooldown_remaining(
    player: &ConnectedPlayer,
    slot: SkillSlot,
    now: Instant,
) -> f32 {
    if combat_clocks_suspended(player) {
        return 0.0;
    }
    skill_cooldown_left(player, slot, now)
}

/// Shared inter-skill recovery window, measured from the latest cast.
pub(crate) fn skill_recovery_remaining(player: &ConnectedPlayer, now: Instant) -> f32 {
    if combat_clocks_suspended(player) {
        return 0.0;
    }
    remaining_of(
        player.timers.last_cast_at.iter().flatten().max().copied(),
        hero_stats::skill_recovery(player),
        now,
    )
}

pub(crate) fn dash_remaining(player: &ConnectedPlayer, now: Instant) -> f32 {
    if no_cooldowns(player) {
        return 0.0;
    }
    remaining_until(player.timers.dash_ready_at, now)
}

pub(crate) fn haste_remaining(player: &ConnectedPlayer, now: Instant) -> f32 {
    if no_cooldowns(player) {
        return 0.0;
    }
    remaining_until(player.timers.haste_ready_at, now)
}

pub(crate) fn haste_active(player: &ConnectedPlayer, now: Instant) -> f32 {
    if player.hero.hp <= 0.0 {
        return 0.0;
    }
    remaining_until(player.timers.haste_expires_at, now)
}

/// Clears the clocks that derived conditions make meaningless: a dead hero or
/// a sandbox actor without cooldowns has no pending basic strike, a sandbox
/// actor without cooldowns has no utility cooldowns and a dead hero has no
/// running haste. Runs once per tick after the simulation, before the views
/// are built; the reads above already treat these cases as ready, so this
/// only keeps the stored instants from outliving their meaning.
pub(crate) fn normalize_hero_timers(world: &mut GameWorld) {
    for player in world.players.values_mut() {
        let dead = player.hero.hp <= 0.0;
        let free = no_cooldowns(player);
        if dead || free {
            player.timers.last_basic_attack_at = None;
        }
        if free {
            player.timers.dash_ready_at = None;
            player.timers.haste_ready_at = None;
        }
        if dead {
            player.timers.haste_expires_at = None;
        }
    }
}
