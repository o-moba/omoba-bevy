//! Match mode configuration and the Lobby -> Forming -> Starting -> Running
//! formation state machine.
use crate::*;

/// How matches are allowed to start.
///
/// * `Release` — production-like: the match forms to a full
///   `2 x team_size` roster with server-assigned balanced teams before it
///   starts. Safe default.
/// * `Dev` — local development: the first join starts the match immediately
///   and the client-chosen team is honored (the pre-TASK-22 behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchMode {
    Release,
    Dev,
    Practice,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MatchConfig {
    pub(crate) mode: MatchMode,
    pub(crate) team_size: u32,
}

pub(crate) const DEFAULT_TEAM_SIZE: u32 = 5;

pub(crate) const MIN_TEAM_SIZE: u32 = 1;

pub(crate) const MAX_TEAM_SIZE: u32 = 16;

pub(crate) const MATCH_START_COUNTDOWN_MS: u32 = 3_000;

pub(crate) fn parse_match_mode(raw: Option<&str>) -> MatchMode {
    match raw
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") | Some("release") | Some("normal") => MatchMode::Release,
        Some("dev") | Some("debug") => MatchMode::Dev,
        Some("practice") => MatchMode::Practice,
        Some(other) => {
            eprintln!(
                "Unknown OMOBA_MATCH_MODE '{other}' - falling back to release (expected 'release', 'dev' or 'practice')"
            );
            MatchMode::Release
        }
    }
}

pub(crate) fn parse_team_size(raw: Option<&str>) -> u32 {
    let Some(raw) = raw else {
        return DEFAULT_TEAM_SIZE;
    };
    match raw.trim().parse::<u32>() {
        Ok(value) => value.clamp(MIN_TEAM_SIZE, MAX_TEAM_SIZE),
        Err(_) => {
            eprintln!("Invalid OMOBA_TEAM_SIZE '{raw}' - using default {DEFAULT_TEAM_SIZE}");
            DEFAULT_TEAM_SIZE
        }
    }
}

impl MatchConfig {
    pub(crate) fn mode_id(&self) -> &'static str {
        match self.mode {
            MatchMode::Release => "release",
            MatchMode::Dev => "dev",
            MatchMode::Practice => "practice",
        }
    }
    pub(crate) fn from_env() -> Self {
        Self {
            mode: parse_match_mode(std::env::var("OMOBA_MATCH_MODE").ok().as_deref()),
            team_size: parse_team_size(std::env::var("OMOBA_TEAM_SIZE").ok().as_deref()),
        }
    }

    /// Instant-start config used by unit tests and as the documented dev
    /// baseline.
    #[cfg(test)]
    pub(crate) fn dev() -> Self {
        Self {
            mode: MatchMode::Dev,
            team_size: DEFAULT_TEAM_SIZE,
        }
    }

    #[cfg(test)]
    pub(crate) fn release(team_size: u32) -> Self {
        Self {
            mode: MatchMode::Release,
            team_size,
        }
    }

    pub(crate) fn roster_size(&self) -> u32 {
        self.team_size * 2
    }
}

pub(crate) fn joined_count(players: &HashMap<SocketAddr, ConnectedPlayer>) -> u32 {
    players.values().filter(|player| player.joined).count() as u32
}

pub(crate) fn joined_team_counts(players: &HashMap<SocketAddr, ConnectedPlayer>) -> (u32, u32) {
    let mut green = 0;
    let mut blue = 0;
    for player in players.values().filter(|player| player.joined) {
        match player.state.team {
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
/// Dev: first join starts the match immediately (legacy behavior).
/// Release: joins only move `Lobby -> Forming`; the per-tick formation
/// logic owns `Forming -> Starting -> Running`.
pub(crate) fn advance_formation_on_join(world: &mut GameWorld, config: MatchConfig, now: Instant) {
    match config.mode {
        MatchMode::Dev | MatchMode::Practice => {
            if matches!(world.game_state, GameState::Lobby) {
                println!(
                    "First player joined - match starting ({} mode)",
                    config.mode_id()
                );
                start_match_running(world, now);
            }
        }
        MatchMode::Release => {
            if matches!(world.game_state, GameState::Lobby) {
                let ready = joined_count(&world.players);
                println!(
                    "Matchmaking: forming match {ready}/{} players",
                    config.roster_size()
                );
                world.game_state = GameState::Forming {
                    ready,
                    needed: config.roster_size(),
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
    config: MatchConfig,
    dt: f32,
    now: Instant,
) {
    let needed = config.roster_size();
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
