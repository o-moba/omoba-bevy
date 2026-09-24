//! The Lobby -> Forming -> Starting -> Running formation state machine.
//! The mode-dependent decision (`StartPolicy`) comes from `MatchRules`.

use crate::neutrals::schedule_boss_spawns;

use crate::game_world::GameWorld;

use crate::balance::BOTTOM_BOSS_SPAWN_DELAY;

use shared::map::Team;

use crate::match_rules::StartPolicy;

use crate::entities::ConnectedPlayer;

use std::collections::HashMap;

use crate::balance::TOP_BOSS_SPAWN_DELAY;

use std::net::SocketAddr;

use std::time::Instant;

use crate::match_rules::MatchRules;

use shared::wire::GameState;

pub(crate) const MATCH_START_COUNTDOWN_MS: u32 = 3_000;

pub(crate) fn joined_count(players: &HashMap<SocketAddr, ConnectedPlayer>) -> u32 {
    players.values().filter(|player| player.joined).count() as u32
}

pub(crate) fn joined_team_counts(players: &HashMap<SocketAddr, ConnectedPlayer>) -> (u32, u32) {
    let mut green = 0;
    let mut blue = 0;
    for player in players.values().filter(|player| player.joined) {
        match player.hero.identity.team {
            Team::Green => green += 1,
            Team::Blue => blue += 1,
        }
    }
    (green, blue)
}

/// Release-mode team assignment: the joining player goes to the smaller team
/// (tie -> Green). Returns `None` when the match roster is already full.
#[cfg(test)]
pub(crate) fn assign_release_team(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    team_size: u32,
) -> Option<Team> {
    let (green, blue) = joined_team_counts(players);
    if green >= team_size && blue >= team_size {
        return None;
    }
    if green <= blue && green < team_size {
        Some(Team::Green)
    } else {
        Some(Team::Blue)
    }
}

/// Shared `-> Running` transition: arms the raid-boss spawn schedule.
pub(crate) fn start_match_running(world: &mut GameWorld, now: Instant) {
    world.game_state = GameState::Running;
    schedule_boss_spawns(&mut world.neutrals, now);
    println!(
        "Match running. Boss schedule armed: wendigo_boss in {}s, king_mutatio_boss in {}s",
        BOTTOM_BOSS_SPAWN_DELAY.as_secs(),
        TOP_BOSS_SPAWN_DELAY.as_secs()
    );
}

/// Formation step applied right after a successful join.
/// `StartPolicy::FirstJoin` (dev, practice): the first join starts the match
/// immediately (legacy behavior).
/// `StartPolicy::FullRoster` (release): joins only move `Lobby -> Forming`;
/// the per-tick formation logic owns `Forming -> Starting -> Running`.
pub(crate) fn advance_formation_on_join(world: &mut GameWorld, rules: MatchRules, now: Instant) {
    match rules.start {
        StartPolicy::FirstJoin => {
            if matches!(world.game_state, GameState::Lobby) {
                println!(
                    "First player joined - match starting ({} mode)",
                    rules.mode_id()
                );
                start_match_running(world, now);
            }
        }
        StartPolicy::FullRoster => {
            if matches!(world.game_state, GameState::Lobby) {
                let ready = joined_count(&world.players);
                println!(
                    "Matchmaking: forming match {ready}/{} players",
                    rules.roster_size()
                );
                world.game_state = GameState::Forming {
                    ready,
                    needed: rules.roster_size(),
                };
            }
        }
    }
}

/// Per-tick formation logic (release mode): keeps `Forming` counters fresh,
/// promotes a full roster to the `Starting` countdown, rolls back to
/// `Forming` when someone drops out mid-countdown, returns to `Lobby` when
/// everyone leaves, and starts the match when the countdown elapses.
pub(crate) fn tick_match_formation(
    world: &mut GameWorld,
    rules: MatchRules,
    dt: f32,
    now: Instant,
) {
    let needed = rules.roster_size();
    let ready = joined_count(&world.players);
    match world.game_state {
        GameState::Forming { .. } => {
            if ready == 0 {
                println!("Matchmaking: queue empty - back to lobby");
                world.game_state = GameState::Lobby;
            } else if ready >= needed {
                println!(
                    "Matchmaking: match found ({ready}/{needed}) - starting in {}s",
                    MATCH_START_COUNTDOWN_MS / 1000
                );
                world.game_state = GameState::Starting {
                    countdown_ms: MATCH_START_COUNTDOWN_MS,
                };
            } else {
                world.game_state = GameState::Forming { ready, needed };
            }
        }
        GameState::Starting { countdown_ms } => {
            if ready < needed {
                println!(
                    "Matchmaking: player left during countdown ({ready}/{needed}) - back to forming"
                );
                world.game_state = GameState::Forming { ready, needed };
            } else {
                let elapsed_ms = (dt * 1000.0).max(0.0) as u32;
                let remaining = countdown_ms.saturating_sub(elapsed_ms);
                if remaining == 0 {
                    start_match_running(world, now);
                } else {
                    world.game_state = GameState::Starting {
                        countdown_ms: remaining,
                    };
                }
            }
        }
        GameState::Lobby | GameState::Running | GameState::Victory { .. } => {}
    }
}
