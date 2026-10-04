//! The authoritative entity state of one match: every map keyed by id, the
//! id allocators and the round clocks. Simulation and request handlers take
//! `&mut GameWorld` instead of long parameter lists of the individual maps.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;

use shared::wire::GameState;

use crate::entities::{
    ConnectedPlayer, DisconnectedSession, MapLayoutState, Minion, Neutral, Projectile, Structure,
    TeamBuffs,
};
use crate::forest_pickups;
use crate::neutrals::{build_boss_neutrals, build_neutral_camps};
use crate::world::{build_configured_structures, build_map_layout};

/// Per-tick time context shared by the simulation functions.
#[derive(Clone, Copy)]
pub struct TickCtx {
    pub now: Instant,
    pub dt: f32,
}

pub struct GameWorld {
    pub skill_runtime: crate::skills::SkillWorld,
    pub players: HashMap<SocketAddr, ConnectedPlayer>,
    pub disconnected_sessions: HashMap<String, DisconnectedSession>,
    pub projectiles: HashMap<u64, Projectile>,
    pub structures: HashMap<u64, Structure>,
    pub minions: HashMap<u64, Minion>,
    pub neutrals: HashMap<u64, Neutral>,
    pub team_buffs: TeamBuffs,
    pub forest_pickups: forest_pickups::ForestPickups,
    pub game_state: GameState,
    /// Authoritative running simulation time; lobby, pause and terminal frames do not advance it.
    pub match_elapsed_secs: f32,
    pub map_layout: MapLayoutState,
    pub map_config: shared::map::ResolvedMap,
    pub next_player_id: u64,
    pub next_projectile_id: u64,
    pub next_minion_id: u64,
    pub last_wave_spawn_at: Instant,
}

impl GameWorld {
    pub fn new(map_config: shared::map::ResolvedMap, now: Instant) -> Self {
        let map_layout = build_map_layout();
        let mut next_neutral_id: u64 = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        // Raid bosses start dormant; the Lobby -> Running transition arms
        // their spawn schedule (see `schedule_boss_spawns`).
        neutrals.extend(build_boss_neutrals(&mut next_neutral_id));
        Self {
            skill_runtime: crate::skills::SkillWorld::default(),
            players: HashMap::new(),
            disconnected_sessions: HashMap::new(),
            projectiles: HashMap::new(),
            structures: build_configured_structures(&map_config),
            map_config,
            minions: HashMap::new(),
            game_state: GameState::Lobby,
            match_elapsed_secs: 0.0,
            map_layout,
            next_player_id: 1,
            next_projectile_id: 1,
            next_minion_id: 1,
            neutrals,
            team_buffs: TeamBuffs::default(),
            forest_pickups: forest_pickups::ForestPickups::default(),
            last_wave_spawn_at: now,
        }
    }

    /// A running world with the default map layout and no entities at all:
    /// the fixture for unit tests that used to build loose `HashMap`s.
    #[cfg(any(test, feature = "test-support"))]
    pub fn empty() -> Self {
        Self {
            skill_runtime: crate::skills::SkillWorld::default(),
            players: HashMap::new(),
            disconnected_sessions: HashMap::new(),
            projectiles: HashMap::new(),
            structures: HashMap::new(),
            minions: HashMap::new(),
            neutrals: HashMap::new(),
            team_buffs: TeamBuffs::default(),
            forest_pickups: forest_pickups::ForestPickups::default(),
            game_state: GameState::Running,
            match_elapsed_secs: 0.0,
            map_layout: build_map_layout(),
            map_config: shared::map::ResolvedMap::default(),
            next_player_id: 1,
            next_projectile_id: 1,
            next_minion_id: 1,
            last_wave_spawn_at: Instant::now(),
        }
    }
}
