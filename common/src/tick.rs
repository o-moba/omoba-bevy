//! One simulation tick: prepare, simulate, then broadcast.

use std::collections::HashSet;
use std::time::Instant;

use shared::wire::TargetKind;

use crate::game_world::{GameWorld, TickCtx};
use crate::hero_timers;

use crate::session::handle_respawns;
use crate::shop::accrue_passive_gold;
use crate::sim::minions::simulate_minions;
use crate::sim::neutrals::simulate_neutrals;
use crate::sim::projectiles::simulate_projectiles_filtered;
use crate::sim::towers::simulate_tower_attacks;
use crate::sim::{
    regenerate_base_hp, regenerate_mana, regenerate_team_buff_hp, restore_god_mode_players,
};
use crate::world::spawn_minion_waves_if_due;

use crate::combat_feedback::CombatLog;
#[derive(Clone, Copy)]
pub struct TickOptions {
    pub targeting_qa: bool,
    pub minions_running: bool,
    pub simulating: bool,
}
impl Default for TickOptions {
    fn default() -> Self {
        Self {
            targeting_qa: false,
            minions_running: true,
            simulating: true,
        }
    }
}
pub fn prepare(world: &mut GameWorld, combat_log: &mut CombatLog, now: Instant, dt: f32) {
    regenerate_mana(&mut world.players, dt);
    // Minion-targeted projectiles resolve ahead of formation, bots and the
    // rest of the simulation, where the ECS combat systems used to run;
    // like them, a zero-length step leaves those projectiles alone.
    if dt > 0.0 {
        let minion_hits = simulate_projectiles_filtered(world, TickCtx { now, dt }, |kind| {
            kind == TargetKind::Minion
        });
        crate::skills::observe(world, &minion_hits, now);
        combat_log.extend(now, minion_hits);
    }
}
pub fn skills(world: &mut GameWorld, combat_log: &mut CombatLog, now: Instant, dt: f32) {
    crate::skills::normalize(world, now);
    let events = crate::skills::tick(world, TickCtx { now, dt });
    crate::skills::observe(world, &events, now);
    combat_log.extend(now, events);
}
pub fn finish(
    world: &mut GameWorld,
    combat_log: &mut CombatLog,
    now: Instant,
    dt: f32,
    gold_dt: f32,
    options: TickOptions,
) {
    let tick = TickCtx { now, dt };

    if !options.targeting_qa && options.minions_running && options.simulating {
        spawn_minion_waves_if_due(world, now);
        let events = simulate_minions(world, tick);
        crate::skills::observe(world, &events, now);
        combat_log.extend(now, events);
    }
    if !options.targeting_qa && options.simulating {
        let tower_events = simulate_tower_attacks(world, now);
        crate::skills::observe(world, &tower_events, now);
        combat_log.extend(now, tower_events);
    }
    let projectile_events = if options.simulating {
        simulate_projectiles_filtered(world, tick, |kind| kind != TargetKind::Minion)
    } else {
        Vec::new()
    };
    crate::skills::observe(world, &projectile_events, now);
    combat_log.extend(now, projectile_events);
    if !options.targeting_qa && options.simulating {
        let events = simulate_neutrals(world, tick);
        crate::skills::observe(world, &events, now);
        combat_log.extend(now, events);
    }
    if options.simulating {
        world
            .forest_pickups
            .tick(&mut world.players, &world.game_state, now);
    }
    regenerate_team_buff_hp(world, tick);
    regenerate_base_hp(world, dt);
    accrue_passive_gold(&mut world.players, &world.game_state, gold_dt);
    restore_god_mode_players(world);
    crate::skills::normalize(world, now);
    handle_respawns(world, now);
    hero_timers::normalize_hero_timers(world);

    let live_player_ids = world
        .players
        .values()
        .map(|player| player.hero.identity.id)
        .collect::<HashSet<_>>();
    let GameWorld {
        projectiles,
        minions,
        structures,
        neutrals,
        ..
    } = world;
    projectiles.retain(|_, projectile| match projectile.target.kind {
        TargetKind::Player => live_player_ids.contains(&projectile.target.id),
        TargetKind::Minion => minions
            .get(&projectile.target.id)
            .is_some_and(|minion| minion.state.hp > 0.0),
        TargetKind::Structure => structures
            .get(&projectile.target.id)
            .is_some_and(|structure| structure.state.hp > 0.0),
        TargetKind::Neutral => neutrals
            .get(&projectile.target.id)
            .is_some_and(|neutral| neutral.dead_until.is_none() && neutral.state.hp > 0.0),
    });

    minions.retain(|_, minion| minion.state.hp > 0.0);
}
