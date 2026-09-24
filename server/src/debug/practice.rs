//! Practice sandbox commands on the server: restore the standard roster,
//! clear the bots, spawn a stationary dummy or start a configured 1v1 duel.
//! The roster primitives (`spawn_bot`, `remove_bot`, `remove_all_bots`,
//! `place_dummy`) and the bot AI stay in `crate::bots`; the offline
//! simulation implements the same commands for itself
//! (`client/src/net/offline.rs`).
use std::net::SocketAddr;
use std::time::Instant;

use shared::HeroClass;
use shared::debug::{DUMMY_DISTANCE, DUMMY_MAX_HP};
use shared::map::Team;
use shared::practice::{MAX_DUMMIES, PracticeCommand};
use shared::wire::GameState;

use crate::balance::{MAX_LEVEL, PLAYER_GROUND_Y, STARTING_LEVEL};
use crate::bots::{BotKind, auto_rank_skills, auto_shop, place_dummy, remove_bot};
use crate::entities::{ConnectedPlayer, MapLayoutState, Vec3f};
use crate::runtime::ServerRuntime;
use crate::session::clip_live_structures;
use crate::world::spawn_position_for_team;
use crate::{hero_stats, progression, shop};

/// Raise a fresh bot to `level` with full resources, the level's skill ranks
/// and `gold` spent at the base shop.
fn configure_duelist(
    player: &mut ConnectedPlayer,
    level: u32,
    gold: u32,
    map: &MapLayoutState,
    phase: &GameState,
    match_id: u64,
) {
    let level = level.clamp(STARTING_LEVEL, MAX_LEVEL);
    while player.hero.progress.level < level && player.hero.progress.next_level_xp > 0 {
        let needed = player.hero.progress.next_level_xp;
        progression::grant_player_xp(&mut player.hero, needed);
    }
    player.hero.hp = player.hero.max_hp;
    player.hero.mana = player.hero.max_mana;
    auto_rank_skills(player);
    shop::award_gold(player, gold);
    auto_shop(player, map, phase, match_id);
}

fn opposite_team(team: Team) -> Team {
    match team {
        Team::Green => Team::Blue,
        Team::Blue => Team::Green,
    }
}

impl ServerRuntime {
    /// Practice-only sandbox: standard roster, no bots, a stationary dummy or
    /// a configured 1v1 opponent. `handle_debug` has already checked
    /// `debug_access().practice` (a local practice match); commands from
    /// anything but a joined human are ignored here.
    pub(super) fn handle_practice_command(
        &mut self,
        addr: SocketAddr,
        command: PracticeCommand,
        now: Instant,
    ) {
        let Some(human) = self
            .world
            .players
            .get(&addr)
            .filter(|p| p.joined && !p.hero.identity.is_bot)
        else {
            return;
        };
        let requester = human.hero.identity.id;
        let team = human.hero.identity.team;
        let class = human.hero.identity.hero_class;
        let origin = [human.hero.x, human.hero.z];
        match command {
            PracticeCommand::Roster => {
                self.remove_all_bots();
                self.bots.sandbox = false;
                self.fill_practice_bots(now);
                println!("Practice sandbox: standard roster restored by player {requester}");
            }
            PracticeCommand::ClearBots => {
                self.remove_all_bots();
                self.bots.sandbox = true;
                println!("Practice sandbox: bots cleared by player {requester}");
            }
            PracticeCommand::SpawnDummy => {
                if !matches!(self.world.game_state, GameState::Running) {
                    return;
                }
                let mut dummies: Vec<_> = self
                    .world
                    .players
                    .iter()
                    .filter(|(a, _)| matches!(self.bots.kind(**a), Some(BotKind::Dummy { .. })))
                    .map(|(a, p)| (p.hero.identity.id, *a))
                    .collect();
                dummies.sort_unstable();
                if dummies.len() >= MAX_DUMMIES {
                    remove_bot(
                        &mut self.world.players,
                        &mut self.bots,
                        &mut self.combat_log.ledger,
                        dummies[0].1,
                    );
                }
                let anchor = self.dummy_anchor(origin, team);
                let Some(bot) = self.spawn_bot(
                    opposite_team(team),
                    Some(HeroClass::Warrior),
                    BotKind::Dummy { anchor },
                    now,
                ) else {
                    return;
                };
                let dummy = self.world.players.get_mut(&bot).unwrap();
                dummy.modifiers.base_max_hp = Some(DUMMY_MAX_HP);
                dummy.hero.max_hp = hero_stats::max_hp(dummy);
                dummy.hero.hp = dummy.hero.max_hp;
                place_dummy(dummy, anchor, origin, now);
                println!(
                    "Practice sandbox: dummy {} at ({:.1}, {:.1})",
                    dummy.hero.identity.id, anchor[0], anchor[1]
                );
            }
            PracticeCommand::StartDuel { level, gold } => {
                if !matches!(self.world.game_state, GameState::Running) {
                    return;
                }
                self.remove_all_bots();
                self.bots.sandbox = true;
                let Some(bot) =
                    self.spawn_bot(opposite_team(team), Some(class), BotKind::Duelist, now)
                else {
                    return;
                };
                let match_id = self.match_id;
                let duelist = self.world.players.get_mut(&bot).unwrap();
                configure_duelist(
                    duelist,
                    level,
                    gold,
                    &self.world.map_layout,
                    &self.world.game_state,
                    match_id,
                );
                println!(
                    "Practice sandbox: duelist {} level {} ranks {:?} items {:?} gold left {}",
                    duelist.hero.identity.id,
                    duelist.hero.progress.level,
                    duelist.hero.progress.ranks,
                    duelist.economy.inventory,
                    duelist.economy.gold
                );
            }
            // A newer client's command: nothing to do.
            PracticeCommand::Unsupported => {}
        }
    }

    /// A clear spot in front of the requester, toward the enemy base; falls
    /// back to closer spots and finally the requester's own position.
    fn dummy_anchor(&self, origin: [f32; 2], team: Team) -> [f32; 2] {
        let own = spawn_position_for_team(&self.world.map_layout, team);
        let enemy = spawn_position_for_team(&self.world.map_layout, opposite_team(team));
        let dir = Vec3f::new(enemy.x - own.x, 0.0, enemy.z - own.z).normalize_or_zero();
        let nav = shared::navigation::world_navigation();
        for distance in [DUMMY_DISTANCE, DUMMY_DISTANCE * 0.6, 2.0] {
            let candidate = self.world.map_layout.clamp_player_position(Vec3f::new(
                origin[0] + dir.x * distance,
                PLAYER_GROUND_Y,
                origin[1] + dir.z * distance,
            ));
            let clipped = nav.clip_movement(origin, [candidate.x, candidate.z]);
            let clipped = clip_live_structures(origin, clipped, &self.world.structures);
            if nav.point_clear(clipped)
                && (clipped[0] - origin[0]).hypot(clipped[1] - origin[1]) > 1.0
            {
                return clipped;
            }
        }
        origin
    }
}
