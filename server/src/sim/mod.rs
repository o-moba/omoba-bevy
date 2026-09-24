//! Authoritative per-tick simulation of the game world.

use crate::balance::MANA_REGEN_PER_SECOND;
use crate::balance::MAX_MANA;
use crate::game_world::TickCtx;

use crate::entities::ConnectedPlayer;

use std::collections::HashMap;

use std::net::SocketAddr;

use shared::wire::GameState;

use crate::balance::BASE_HEAL_RADIUS;

use crate::balance::BASE_HEAL_FRACTION_PER_SECOND;

use crate::game_world::GameWorld;

use shared::map::Team;

pub(crate) mod cast;
pub(crate) mod minions;
pub(crate) mod neutrals;
pub(crate) mod projectiles;
pub(crate) mod towers;

/// Mana regeneration for joined, living heroes, clamped to the hero's pool.
/// A pool that was never sized (max 0) is given the default one first.
pub(crate) fn regenerate_mana(players: &mut HashMap<SocketAddr, ConnectedPlayer>, dt: f32) {
    if dt <= 0.0 {
        return;
    }
    for player in players.values_mut() {
        if !player.joined || player.hero.hp <= 0.0 {
            continue;
        }
        if player.hero.max_mana <= 0.0 {
            player.hero.max_mana = MAX_MANA;
        }
        player.hero.mana =
            (player.hero.mana + MANA_REGEN_PER_SECOND * dt).clamp(0.0, player.hero.max_mana);
    }
}

/// Recovery is authoritative and only active inside the hero's own fountain.
pub(crate) fn regenerate_base_hp(world: &mut GameWorld, dt: f32) {
    let GameWorld {
        players,
        map_layout: map,
        game_state: phase,
        ..
    } = world;
    if !matches!(phase, GameState::Running) || !dt.is_finite() || dt <= 0.0 {
        return;
    }
    for player in players.values_mut().filter(|p| p.joined && p.hero.hp > 0.0) {
        let base = match player.hero.identity.team {
            Team::Green => map.home,
            Team::Blue => map.away,
        };
        if (player.hero.x - base.x).hypot(player.hero.z - base.z) <= BASE_HEAL_RADIUS {
            player.hero.hp = (player.hero.hp
                + player.hero.max_hp * BASE_HEAL_FRACTION_PER_SECOND * dt)
                .min(player.hero.max_hp);
        }
    }
}

/// Applies boss-buff HP regeneration to every alive player of a buffed team,
/// clamped to max HP. Runs each simulation tick (piggybacks the regen cadence).
pub(crate) fn regenerate_team_buff_hp(world: &mut GameWorld, tick: TickCtx) {
    let TickCtx { now, dt } = tick;
    let GameWorld {
        players,
        team_buffs,
        game_state,
        ..
    } = world;
    if !matches!(game_state, GameState::Running) || dt <= 0.0 {
        return;
    }
    for player in players.values_mut() {
        if !player.joined || player.hero.hp <= 0.0 {
            continue;
        }
        let regen = team_buffs.hp_regen_per_second(player.hero.identity.team, now);
        if regen > 0.0 {
            player.hero.hp = (player.hero.hp + regen * dt).min(player.hero.max_hp);
        }
    }
}

/// Bulletproof debug invulnerability: after all damage for the tick, force god-mode
/// players back to full HP and cancel any pending respawn, so they never die even if
/// a damage path is missed (TASK04). `infinite_resource` refills the pool the
/// same way; the development toggle sets both, the sandbox sets each on its own.
pub(crate) fn restore_god_mode_players(world: &mut GameWorld) {
    for player in world.players.values_mut() {
        if player.modifiers.god_mode {
            player.hero.hp = player.hero.max_hp;
            player.timers.respawn_at = None;
        }
        if player.modifiers.infinite_resource {
            player.hero.mana = player.hero.max_mana;
        }
    }
}
