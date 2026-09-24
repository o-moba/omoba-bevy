//! Authoritative per-tick simulation of the game world.
use crate::*;

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
        if !player.joined || player.state.hp <= 0.0 {
            continue;
        }
        if player.state.max_mana <= 0.0 {
            player.state.max_mana = MAX_MANA;
        }
        player.state.mana =
            (player.state.mana + MANA_REGEN_PER_SECOND * dt).clamp(0.0, player.state.max_mana);
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
    for player in players
        .values_mut()
        .filter(|p| p.joined && p.state.hp > 0.0)
    {
        let base = match player.state.team {
            Team::Green => map.home,
            Team::Blue => map.away,
        };
        if (player.state.x - base.x).hypot(player.state.z - base.z) <= BASE_HEAL_RADIUS {
            player.state.hp = (player.state.hp
                + player.state.max_hp * BASE_HEAL_FRACTION_PER_SECOND * dt)
                .min(player.state.max_hp);
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
        if !player.joined || player.state.hp <= 0.0 {
            continue;
        }
        let regen = team_buffs.hp_regen_per_second(player.state.team, now);
        if regen > 0.0 {
            player.state.hp = (player.state.hp + regen * dt).min(player.state.max_hp);
        }
    }
}

/// Bulletproof debug invulnerability: after all damage for the tick, force god-mode
/// players back to full HP and cancel any pending respawn, so they never die even if
/// a damage path is missed (TASK04).
pub(crate) fn restore_god_mode_players(world: &mut GameWorld) {
    for player in world.players.values_mut() {
        if player.god_mode {
            player.state.hp = player.state.max_hp;
            if player.sandbox.as_ref().is_none_or(|c| c.infinite_resource) {
                player.state.mana = player.state.max_mana;
            }
            player.respawn_at = None;
        }
    }
}
