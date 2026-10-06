//! Hero stat formulas and the per-hero modifiers they read.
//!
//! `StatModifiers` (`ConnectedPlayer.modifiers`) is the one overlay on top of
//! class, level and gear: the development toggles (god mode, speed boost)
//! and the Combat Sandbox's actor overrides both write it, and nothing in the
//! simulation asks "is this a sandbox actor" any more; it asks the flag it
//! needs. The functions below are the only place the class growth, item
//! bonuses and modifiers are combined into an effective number.

use std::time::{Duration, Instant};

use shared::SkillSlot;
use shared::shop::ItemBonuses;

use crate::balance::{LEVEL_UP_HP_BONUS, LEVEL_UP_MANA_BONUS, MAX_MANA, PLAYER_SPEED};
use crate::entities::ConnectedPlayer;
use crate::utility::utility_movement_multiplier;

/// Per-hero overrides on top of class, level and gear. `Default` is normal
/// play: no multipliers, no mitigation, every rule enforced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatModifiers {
    /// Multiplies the gear damage multiplier (sandbox `damage_multiplier`).
    pub damage_mult: f32,
    /// Multiplies the gear attack-speed multiplier (sandbox `attack_speed`).
    pub attack_speed_mult: f32,
    /// Multiplies the movement envelope: the debug speed boost or the sandbox
    /// `move_speed`.
    pub move_speed_mult: f32,
    /// Flat physical mitigation, `damage * 100 / (100 + armor)`.
    pub armor: f32,
    /// Flat magical mitigation, same curve as `armor`.
    pub resistance: f32,
    /// Replaces the class base HP in `max_hp` (sandbox actors, the practice
    /// dummy); level growth and item HP still apply on top.
    pub base_max_hp: Option<f32>,
    /// Takes no damage; HP is restored and any pending respawn cancelled each
    /// tick.
    pub god_mode: bool,
    /// Damage is resolved and reported but never subtracted (sandbox dummy).
    pub infinite_hp: bool,
    /// Casts are free and the pool is refilled each tick.
    pub infinite_resource: bool,
    /// Every cooldown reads as ready and the instants are cleared each tick.
    pub no_cooldowns: bool,
    /// Every ability slot is castable regardless of level.
    pub unlock_all: bool,
    /// Casts and basic strikes skip the vision check.
    pub bypass_vision: bool,
    /// Minion and neutral kills grant XP (off for sandbox actors, whose level
    /// is configured).
    pub grant_xp: bool,
    /// The respawn clock teleports the hero home (off for sandbox bots, which
    /// the sandbox resets itself).
    pub respawns: bool,
}

impl Default for StatModifiers {
    fn default() -> Self {
        Self {
            damage_mult: 1.0,
            attack_speed_mult: 1.0,
            move_speed_mult: 1.0,
            armor: 0.0,
            resistance: 0.0,
            base_max_hp: None,
            god_mode: false,
            infinite_hp: false,
            infinite_resource: false,
            no_cooldowns: false,
            unlock_all: false,
            bypass_vision: false,
            grant_xp: true,
            respawns: true,
        }
    }
}

/// The hero's gear bonuses as combat reads them: the item multipliers never
/// drop below one, then the modifiers scale them (the sandbox may go below
/// one on purpose, down to zero damage).
pub fn combat_bonuses(player: &ConnectedPlayer) -> ItemBonuses {
    let mut bonuses = player.economy.item_bonuses;
    bonuses.move_speed_multiplier *= player.economy.dragon_speed;
    bonuses.damage_multiplier = bonuses.damage_multiplier.max(1.0) * player.modifiers.damage_mult;
    bonuses.attack_speed_multiplier =
        bonuses.attack_speed_multiplier.max(1.0) * player.modifiers.attack_speed_mult;
    if player
        .hero
        .skills
        .loadout
        .is_some_and(|l| l.passive() == shared::loadout::PassiveId::Souls)
    {
        bonuses.damage_multiplier *= 1.0 + player.hero.skills.advanced.souls as f32 * 0.002;
    }
    bonuses
}

pub fn basic_attack_damage(player: &ConnectedPlayer) -> f32 {
    shared::hero_balance::basic_damage(
        player
            .hero
            .skills
            .loadout
            .as_ref()
            .map_or(player.hero.identity.hero_class, |l| l.core().class()),
        player.hero.progress.level,
        combat_bonuses(player),
    )
}

pub fn basic_attack_cooldown(player: &ConnectedPlayer) -> Duration {
    shared::hero_balance::basic_cooldown(
        player
            .hero
            .skills
            .loadout
            .as_ref()
            .map_or(player.hero.identity.hero_class, |l| l.core().class()),
        player.hero.progress.level,
        combat_bonuses(player),
    )
}

pub fn ability_cooldown(player: &ConnectedPlayer, slot: SkillSlot) -> Duration {
    equipped_skills(player).cooldown(
        player.hero.progress.level,
        player.hero.progress.ranks[slot.index()],
        slot,
        combat_bonuses(player),
    )
}

/// The accepted recipe owns binding metadata; legacy actors use their class kit.
pub fn equipped_skills(player: &ConnectedPlayer) -> shared::loadout::EquippedSkills {
    shared::loadout::EquippedSkills::from_resolved(
        player.hero.identity.hero_class,
        player.hero.skills.loadout,
    )
}

/// Shared inter-skill recovery window after any cast.
pub fn skill_recovery(player: &ConnectedPlayer) -> Duration {
    Duration::from_secs_f32(shared::hero_balance::skill_recovery_secs(
        player.hero.progress.level,
    ))
}

/// Attack rate relative to the class baseline (sandbox telemetry).
pub fn attack_speed(player: &ConnectedPlayer) -> f32 {
    combat_bonuses(player).attack_speed_multiplier
        * shared::hero_balance::attack_rate_multiplier(
            player.hero.identity.hero_class,
            player.hero.progress.level,
        )
}

/// Steady movement speed in world units per second, without haste: what the
/// bots and the sandbox step by and what the sandbox telemetry reports.
pub fn move_speed(player: &ConnectedPlayer) -> f32 {
    PLAYER_SPEED
        * player.modifiers.move_speed_mult
        * combat_bonuses(player).move_speed_multiplier
        * shared::hero_balance::movement_multiplier(
            player.hero.identity.hero_class,
            player.hero.progress.level,
        )
}

/// Distance a movement request may cover after `elapsed` seconds, including
/// haste and the unspent position tolerance (`timers.movement_slack`, which
/// starts at `MOVEMENT_POSITION_TOLERANCE`). The factor order is the one the
/// envelope has always used, so a single request from a fresh budget is
/// accepted bit-identically to the old per-packet allowance.
pub fn movement_envelope(player: &ConnectedPlayer, now: Instant, elapsed: f32) -> f32 {
    let multiplier = player.modifiers.move_speed_mult.max(0.1)
        * utility_movement_multiplier(player, now)
        * shared::hero_balance::movement_multiplier(
            player.hero.identity.hero_class,
            player.hero.progress.level,
        );
    PLAYER_SPEED
        * multiplier
        * combat_bonuses(player).move_speed_multiplier
        * player.hero.skills.movement(now)
        * elapsed
        + player.timers.movement_slack
}

/// Full HP pool: the class base (or the configured override), level growth
/// and item HP. Level-ups and purchases add their delta to `hero.max_hp`
/// incrementally; this is the closed form they agree with.
pub fn max_hp(player: &ConnectedPlayer) -> f32 {
    player
        .modifiers
        .base_max_hp
        .unwrap_or_else(|| shared::hero_balance::base_hp(player.hero.identity.hero_class))
        + player.hero.progress.level.saturating_sub(1) as f32 * LEVEL_UP_HP_BONUS
        + player.economy.item_bonuses.max_hp
}

/// Full mana pool: the shared base, level growth and item mana.
pub fn max_mana(player: &ConnectedPlayer) -> f32 {
    if player
        .hero
        .skills
        .loadout
        .is_some_and(|l| l.core() == shared::loadout::CoreId::Stormfist)
    {
        return 200.0;
    }
    MAX_MANA
        + player.hero.progress.level.saturating_sub(1) as f32 * LEVEL_UP_MANA_BONUS
        + player.economy.item_bonuses.max_mana
}

/// Sets `hero.max_hp` / `hero.max_mana` from the closed forms and clamps the
/// current pools to them.
pub fn resize_pools(player: &mut ConnectedPlayer) {
    let max_hp = max_hp(player);
    let max_mana = max_mana(player);
    player.hero.max_hp = max_hp;
    player.hero.max_mana = max_mana;
    player.hero.hp = player.hero.hp.min(max_hp);
    player.hero.mana = player.hero.mana.min(max_mana);
}

/// Incoming damage after the hero's flat mitigation.
pub fn mitigate(player: &ConnectedPlayer, damage: f32, magical: bool) -> f32 {
    let mitigation = if magical {
        player.modifiers.resistance
    } else {
        player.modifiers.armor
    };
    let s = &player.hero.skills.advanced;
    let passive = player.hero.skills.loadout.map(|l| l.passive());
    let extra = if passive == Some(shared::loadout::PassiveId::Tempered) {
        20.0
    } else {
        0.0
    } + if passive == Some(shared::loadout::PassiveId::Souls) && !magical {
        s.souls as f32 * 0.5
    } else {
        0.0
    } + if s.forged { 15.0 } else { 0.0 };
    damage * 100.0 / (100.0 + mitigation + extra + player.economy.dragon_armor)
}

/// Mode and passive timing apply after the shared class/gear baseline.
pub fn basic_attack_cooldown_at(player: &ConnectedPlayer, now: Instant) -> Duration {
    basic_attack_cooldown(player)
        .mul_f32(crate::skills::attack_modifiers(player).2)
        .div_f32(player.hero.skills.attack_rate(now))
}
