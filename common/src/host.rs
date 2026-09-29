//! Borrowed combat host: shared bot/practice operations without owning I/O.
use crate::{
    bots::{BotControllers, BotKind},
    combat_feedback::CombatLog,
    game_world::GameWorld,
    match_rules::MatchRules,
};
use shared::{map::Team, wire::GameState};
use std::time::Instant;

pub struct CombatHost<'a> {
    pub world: &'a mut GameWorld,
    pub bots: &'a mut BotControllers,
    pub combat_log: &'a mut CombatLog,
    pub rules: MatchRules,
    pub match_id: u64,
}
impl CombatHost<'_> {
    pub fn fill_practice_bots(&mut self, now: Instant) {
        if !self.rules.fills_with_bots
            || self.bots.sandbox
            || self.bots.defer_fill
            || matches!(self.world.game_state, GameState::Victory { .. })
        {
            return;
        }
        for team in [Team::Green, Team::Blue] {
            while crate::bots::seated_count(&self.world.players, self.bots, team)
                < self.rules.team_size as usize
            {
                if self.spawn_bot(team, None, BotKind::Lane, now).is_none() {
                    break;
                }
            }
        }
    }
}
