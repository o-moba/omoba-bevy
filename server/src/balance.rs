//! Authoritative numeric tuning for the gameplay slice (server simulation).
//!
//! Adjust slice pacing and threat here instead of scattering literals across `main.rs`.
//! The client mirrors **display-only** baselines (`MAX_HP`, `MAX_MANA`) in
//! `client/src/combat.rs`; keep those in sync when you change player baselines.
//!
//! **Skill power:** Per-class ability numbers (mana, cooldown, range, damage,
//! heals, rank scaling) live in the `shared` crate (`shared::HeroClass` kits);
//! the server resolves them authoritatively in `handle_cast_request`. This file
//! keeps only server-side simulation tuning (projectile flight, world pacing).
//!
//! See also: `docs/balance-tuning.md`.

use std::time::Duration;

// --- Player baselines & regeneration ---
pub const MAX_HP: f32 = 100.0;
pub const MAX_MANA: f32 = 100.0;
pub const MANA_REGEN_PER_SECOND: f32 = 8.0;

// --- Projectile simulation (ability numbers live in the shared class kits) ---
pub const PROJECTILE_SPEED: f32 = 19.0;
/// Skill slots tracked by progression (`skill_points`); matches the shared Q/W/E/R kits.
#[allow(dead_code)]
pub const SKILL_SLOT_COUNT: usize = 4;
pub const PROJECTILE_RADIUS: f32 = 0.22;
pub const PROJECTILE_LIFETIME: Duration = Duration::from_secs(3);
pub const PLAYER_HIT_RADIUS: f32 = 0.62;
pub const CAST_SPAWN_HEIGHT: f32 = 0.85;
pub const AIM_HEIGHT: f32 = 0.55;
pub const RESPAWN_DELAY: Duration = Duration::from_secs(5);

// --- Lane towers & base tower threat ---
pub const TOWER_MAX_HP: f32 = 240.0;
pub const BASE_TOWER_MAX_HP: f32 = 650.0;
pub const TOWER_SIZE: f32 = 2.6;
pub const BASE_TOWER_SIZE: f32 = 6.0;
pub const TOWER_RANGE: f32 = 20.0;
pub const TOWER_DAMAGE: f32 = 14.0;
pub const TOWER_COOLDOWN: Duration = Duration::from_millis(900);
pub const TOWER_SHOT_HEIGHT: f32 = 2.4;
pub const BASE_TOWER_RANGE: f32 = 24.0;
pub const BASE_TOWER_DAMAGE: f32 = 18.0;
pub const BASE_TOWER_COOLDOWN: Duration = Duration::from_millis(850);
pub const BASE_TOWER_SHOT_HEIGHT: f32 = 3.2;

// --- Minion waves & lane pressure ---
pub const MINION_MAX_HP: f32 = 65.0;
pub const MINION_SPEED: f32 = 3.1;
pub const MINION_ATTACK_RANGE: f32 = 2.4;
pub const MINION_ATTACK_DAMAGE: f32 = 8.0;
pub const MINION_ATTACK_COOLDOWN: Duration = Duration::from_millis(950);
pub const MINION_VISION_RANGE: f32 = 10.0;
pub const MINION_RADIUS: f32 = 0.55;
pub const MINION_SPAWN_HEIGHT: f32 = 0.5;
pub const MINION_WAVE_INTERVAL: Duration = Duration::from_secs(60);
pub const MINIONS_PER_WAVE: usize = 3;
pub const MINION_KILL_GOLD: u32 = 18;
pub const MINION_KILL_XP: u32 = 32;
pub const PLAYER_SPAWN_OFFSET: f32 = 7.0;
pub const PLAYER_GROUND_Y: f32 = 0.5;
pub const MOVEMENT_POSITION_TOLERANCE: f32 = 0.10;
pub const MOVEMENT_MAX_DELTA_SECONDS: f32 = 0.5;
pub const SESSION_RECLAIM_WINDOW: Duration = Duration::from_secs(30);

// --- Level curve & stat growth ---
/// Debug movement multiplier applied when a client enables the speed-boost toggle.
pub const DEBUG_SPEED_MULTIPLIER: f32 = 2.6;
pub const STARTING_LEVEL: u32 = 1;
pub const MAX_LEVEL: u32 = 10;
pub const LEVEL_UP_HP_BONUS: f32 = 18.0;
pub const LEVEL_UP_MANA_BONUS: f32 = 12.0;
// First level is reachable in ~3 minion kills (3 * MINION_KILL_XP = 96 ≥ 90) so the
// skill-upgrade flow is easy to exercise; later levels keep the original curve.
pub const LEVEL_XP_THRESHOLDS: [u32; 9] = [90, 150, 180, 220, 260, 300, 340, 380, 420];

// --- Jungle neutrals (camp stats & reward pacing) ---
pub const NEUTRAL_RADIUS: f32 = 0.62;
pub const NEUTRAL_SPAWN_HEIGHT: f32 = 0.5;
pub const NEUTRAL_AGGRO_RADIUS: f32 = 7.5;
pub const NEUTRAL_LEASH_DISTANCE: f32 = 13.0;
pub const NEUTRAL_ATTACK_COOLDOWN: Duration = Duration::from_millis(850);
pub const NEUTRAL_CHASE_SPEED: f32 = 2.9;
pub const NEUTRAL_RESPAWN_COOLDOWN: Duration = Duration::from_secs(40);

pub const SKIRMISHER_MAX_HP: f32 = 72.0;
pub const SKIRMISHER_ATTACK_DAMAGE: f32 = 7.0;
pub const SKIRMISHER_ATTACK_RANGE: f32 = 2.45;
pub const SKIRMISHER_KILL_GOLD: u32 = 28;
pub const SKIRMISHER_KILL_XP: u32 = 50;

pub const BRUISER_MAX_HP: f32 = 130.0;
pub const BRUISER_ATTACK_DAMAGE: f32 = 11.0;
pub const BRUISER_ATTACK_RANGE: f32 = 2.65;
pub const BRUISER_KILL_GOLD: u32 = 52;
pub const BRUISER_KILL_XP: u32 = 85;

pub const SPITTER_MAX_HP: f32 = 58.0;
pub const SPITTER_ATTACK_DAMAGE: f32 = 9.0;
pub const SPITTER_ATTACK_RANGE: f32 = 7.6;
pub const SPITTER_KILL_GOLD: u32 = 35;
pub const SPITTER_KILL_XP: u32 = 55;

/// Fraction of half-map extent used for jungle camp anchor distance (outer ring).
pub const JUNGLE_MAP_OUTER_FRAC: f32 = 0.34;
/// Fraction of half-map extent used for jungle camp anchor distance (inner ring).
pub const JUNGLE_MAP_INNER_FRAC: f32 = 0.22;

// --- Raid bosses (TASK-19): epic neutral objectives with team buffs ---
//
// Two bosses sit in mirror-symmetric pits derived with the same map formula as
// the jungle camps. They spawn on a match-time schedule, hit much harder than
// camps, respawn on their own (longer) cooldown, and grant the killer's whole
// team a timed buff.

/// Boss pit distance fractions (mirror the jungle ring formula; see
/// `neutrals::boss_blueprints`).
pub const BOSS_PIT_OUTER_FRAC: f32 = JUNGLE_MAP_OUTER_FRAC;
pub const BOSS_PIT_INNER_FRAC: f32 = JUNGLE_MAP_INNER_FRAC;

/// Bottom boss (Wendigo) spawns this long after match start (Lobby -> Running).
pub const BOTTOM_BOSS_SPAWN_DELAY: Duration = Duration::from_secs(60);
/// Top boss (King Mutatio) spawns this long after match start.
pub const TOP_BOSS_SPAWN_DELAY: Duration = Duration::from_secs(180);
/// Bosses respawn this long after death (camps keep `NEUTRAL_RESPAWN_COOLDOWN`).
pub const BOSS_RESPAWN_COOLDOWN: Duration = Duration::from_secs(180);

/// Bosses chase further from their pit than camps before resetting.
pub const BOSS_LEASH_DISTANCE: f32 = 18.0;
/// Boss proximity-aggro radius (kept at camp scale; bosses mostly punish attackers).
pub const BOSS_AGGRO_RADIUS: f32 = NEUTRAL_AGGRO_RADIUS;

// Bottom boss: Wendigo (dragon-slot objective).
pub const WENDIGO_MAX_HP: f32 = 900.0;
pub const WENDIGO_ATTACK_DAMAGE: f32 = 20.0;
pub const WENDIGO_ATTACK_RANGE: f32 = 3.2;
pub const WENDIGO_KILL_GOLD: u32 = 150;
pub const WENDIGO_KILL_XP: u32 = 200;

// Top boss: King Mutatio (Baron-slot objective).
pub const MUTATIO_MAX_HP: f32 = 1500.0;
pub const MUTATIO_ATTACK_DAMAGE: f32 = 28.0;
pub const MUTATIO_ATTACK_RANGE: f32 = 3.4;
pub const MUTATIO_KILL_GOLD: u32 = 250;
pub const MUTATIO_KILL_XP: u32 = 300;

// Team buffs granted on boss kill (refresh on re-kill; multiplicative if both).
/// Bottom boss buff: +15% ability damage for the killing team.
pub const BOTTOM_BOSS_BUFF_DAMAGE_MULT: f32 = 1.15;
pub const BOTTOM_BOSS_BUFF_DURATION: Duration = Duration::from_secs(90);
/// Top boss buff: +25% ability damage plus flat HP regen for the killing team.
pub const TOP_BOSS_BUFF_DAMAGE_MULT: f32 = 1.25;
pub const TOP_BOSS_BUFF_HP_REGEN_PER_SECOND: f32 = 2.0;
pub const TOP_BOSS_BUFF_DURATION: Duration = Duration::from_secs(90);

// --- Match flow ---
pub const VICTORY_REMATCH_DELAY: Duration = Duration::from_secs(10);

// --- Map generation (affects lane length and jungle placement) ---
pub const TARGET_BASE_RUN_TIME_SECONDS: f32 = 45.0;
pub const PLAYER_SPEED: f32 = 5.0;
pub const TARGET_BASE_DISTANCE: f32 = PLAYER_SPEED * TARGET_BASE_RUN_TIME_SECONDS;
pub const BASE_PAD_SIZE: f32 = 46.0;
pub const BASE_EDGE_MARGIN: f32 = 6.0;
pub const LANE_WIDTH: f32 = 12.0;
pub const LANE_EDGE_PADDING: f32 = 6.0;

#[cfg(test)]
mod tests {
    #![allow(clippy::assertions_on_constants)]

    use super::*;

    #[test]
    fn player_baselines_and_spell_economy_are_coherent() {
        assert!(MAX_HP > 0.0 && MAX_MANA > 0.0);
        assert!(MANA_REGEN_PER_SECOND >= 0.0);
        assert!(PROJECTILE_SPEED > 0.0);
        // Every shared-kit ability must be affordable at the player mana baseline.
        for class in shared::HeroClass::ALL {
            for def in class.abilities() {
                assert!(def.base_mana_cost <= MAX_MANA, "{}", def.id);
            }
        }
    }

    #[test]
    fn level_xp_table_matches_level_span() {
        assert_eq!(
            LEVEL_XP_THRESHOLDS.len(),
            (MAX_LEVEL - STARTING_LEVEL) as usize
        );
        for &t in LEVEL_XP_THRESHOLDS.iter() {
            assert!(t > 0);
        }
    }

    #[test]
    fn lane_tower_and_minion_tuning_is_positive() {
        assert!(TOWER_RANGE > 0.0 && TOWER_DAMAGE > 0.0);
        assert!(MINION_MAX_HP > 0.0 && MINIONS_PER_WAVE > 0);
        assert!(!MINION_WAVE_INTERVAL.is_zero());
        assert!(BASE_TOWER_MAX_HP >= TOWER_MAX_HP);
    }

    #[test]
    fn jungle_pacing_constants_are_positive() {
        assert!(NEUTRAL_RESPAWN_COOLDOWN > Duration::ZERO);
        assert!(SKIRMISHER_MAX_HP > 0.0 && BRUISER_MAX_HP > 0.0 && SPITTER_MAX_HP > 0.0);
        assert!(JUNGLE_MAP_INNER_FRAC > 0.0 && JUNGLE_MAP_OUTER_FRAC > JUNGLE_MAP_INNER_FRAC);
    }

    #[test]
    fn boss_tuning_outclasses_camps_and_timings_are_positive() {
        // Bosses must be clearly stronger and more rewarding than every camp.
        for camp_hp in [SKIRMISHER_MAX_HP, BRUISER_MAX_HP, SPITTER_MAX_HP] {
            assert!(WENDIGO_MAX_HP > camp_hp * 2.0);
        }
        assert!(MUTATIO_MAX_HP > WENDIGO_MAX_HP);
        assert!(WENDIGO_ATTACK_DAMAGE > BRUISER_ATTACK_DAMAGE);
        assert!(MUTATIO_ATTACK_DAMAGE > WENDIGO_ATTACK_DAMAGE);
        assert!(WENDIGO_KILL_GOLD > BRUISER_KILL_GOLD && WENDIGO_KILL_XP > BRUISER_KILL_XP);
        assert!(MUTATIO_KILL_GOLD > WENDIGO_KILL_GOLD && MUTATIO_KILL_XP > WENDIGO_KILL_XP);
        // Schedule: bottom before top; boss respawn slower than camps.
        assert!(BOTTOM_BOSS_SPAWN_DELAY < TOP_BOSS_SPAWN_DELAY);
        assert!(BOSS_RESPAWN_COOLDOWN > NEUTRAL_RESPAWN_COOLDOWN);
        assert!(BOSS_LEASH_DISTANCE > NEUTRAL_LEASH_DISTANCE);
        // Buffs: top stronger than bottom; both multiplicative bonuses > 1.
        assert!(BOTTOM_BOSS_BUFF_DAMAGE_MULT > 1.0);
        assert!(TOP_BOSS_BUFF_DAMAGE_MULT > BOTTOM_BOSS_BUFF_DAMAGE_MULT);
        assert!(TOP_BOSS_BUFF_HP_REGEN_PER_SECOND > 0.0);
        assert!(!BOTTOM_BOSS_BUFF_DURATION.is_zero() && !TOP_BOSS_BUFF_DURATION.is_zero());
        assert!(BOSS_PIT_OUTER_FRAC > BOSS_PIT_INNER_FRAC && BOSS_PIT_INNER_FRAC > 0.0);
    }

    #[test]
    fn skill_slot_count_matches_shared_kits() {
        assert_eq!(SKILL_SLOT_COUNT, 4);
        for class in shared::HeroClass::ALL {
            assert_eq!(class.abilities().len(), SKILL_SLOT_COUNT);
        }
    }
}
