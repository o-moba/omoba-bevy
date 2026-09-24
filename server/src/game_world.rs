//! The authoritative entity state of one match: every map keyed by id, the
//! id allocators and the round clocks. Simulation and request handlers take
//! `&mut GameWorld` instead of long parameter lists of the individual maps.
use crate::*;

/// Per-tick time context shared by the simulation functions.
#[derive(Clone, Copy)]
pub(crate) struct TickCtx {
    pub now: Instant,
    pub dt: f32,
}

pub(crate) struct GameWorld {
    pub(crate) players: HashMap<SocketAddr, ConnectedPlayer>,
    pub(crate) disconnected_sessions: HashMap<String, DisconnectedSession>,
    pub(crate) projectiles: HashMap<u64, Projectile>,
    pub(crate) structures: HashMap<u64, Structure>,
    pub(crate) minions: HashMap<u64, Minion>,
    pub(crate) neutrals: HashMap<u64, Neutral>,
    pub(crate) team_buffs: TeamBuffs,
    pub(crate) forest_pickups: forest_pickups::ForestPickups,
    pub(crate) game_state: GameState,
    pub(crate) map_layout: MapLayoutState,
    pub(crate) map_config: shared::map::ResolvedMap,
    pub(crate) next_player_id: u64,
    pub(crate) next_projectile_id: u64,
    pub(crate) next_minion_id: u64,
    pub(crate) last_wave_spawn_at: Instant,
}

impl GameWorld {
    pub(crate) fn new(map_config: shared::map::ResolvedMap, now: Instant) -> Self {
        let map_layout = build_map_layout();
        let mut next_neutral_id: u64 = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        // Raid bosses start dormant; the Lobby -> Running transition arms
        // their spawn schedule (see `schedule_boss_spawns`).
        neutrals.extend(build_boss_neutrals(&mut next_neutral_id));
        Self {
            players: HashMap::new(),
            disconnected_sessions: HashMap::new(),
            projectiles: HashMap::new(),
            structures: build_configured_structures(&map_config),
            map_config,
            minions: HashMap::new(),
            game_state: GameState::Lobby,
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
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            players: HashMap::new(),
            disconnected_sessions: HashMap::new(),
            projectiles: HashMap::new(),
            structures: HashMap::new(),
            minions: HashMap::new(),
            neutrals: HashMap::new(),
            team_buffs: TeamBuffs::default(),
            forest_pickups: forest_pickups::ForestPickups::default(),
            game_state: GameState::Running,
            map_layout: build_map_layout(),
            map_config: shared::map::ResolvedMap::default(),
            next_player_id: 1,
            next_projectile_id: 1,
            next_minion_id: 1,
            last_wave_spawn_at: Instant::now(),
        }
    }
}
