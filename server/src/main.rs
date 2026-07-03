#![allow(clippy::items_after_test_module)]

mod balance;
mod gameplay;
mod neutrals;
mod progression;
mod session;
mod world;

use balance::*;
use bevy::{app::ScheduleRunnerPlugin, prelude::*};
use ekza_bevy_sdk::EkzaCharacter as CharacterChoice;
use gameplay::GameplayPlugin;
use neutrals::*;
use progression::*;
use serde::{Deserialize, Serialize};
use session::*;
use shared::{
    HeroClass, SkillSlot, TargetingMode, ability_for_class_slot, rank_effect_scale,
    scaled_cast_range, scaled_cooldown, scaled_mana_cost, unlocked_slots_for_level,
};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};
use world::*;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:4000";
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(50);
const PLAYER_TIMEOUT: Duration = Duration::from_secs(5);
const SIMULATION_STEP_SLEEP: Duration = Duration::from_millis(10);
const MAX_PACKET_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientPacket {
    Transform {
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
    },
    Cast {
        target: TargetId,
        /// Hotbar slot index (0=Q .. 3=R); the server resolves the ability
        /// from the caster's class kit. Defaults to Q for legacy packets.
        #[serde(default)]
        slot: u8,
    },
    Join {
        team: Team,
        #[serde(default = "default_character_choice")]
        character: CharacterChoice,
        /// Selected class; unknown wire values decode as the default class.
        #[serde(default)]
        hero_class: HeroClass,
        /// Cosmetic roster avatar slug; validated against the shipped roster.
        #[serde(default)]
        avatar: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    Ping,
    RequestRematch,
    SetGodMode {
        enabled: bool,
    },
    SetSpeedBoost {
        enabled: bool,
    },
    UpgradeSkill {
        slot: u8,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Team {
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum Lane {
    Top,
    Mid,
    Bot,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TargetKind {
    Player,
    Minion,
    Structure,
    Neutral,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct TargetId {
    kind: TargetKind,
    id: u64,
}

fn default_character_choice() -> CharacterChoice {
    CharacterChoice::Ipfs
}

/// All ability ranks start at 1 (1-based; rank 1 = base power).
fn default_skill_ranks() -> [u8; 4] {
    [1; 4]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlayerState {
    id: u64,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    team: Team,
    hp: f32,
    max_hp: f32,
    mana: f32,
    max_mana: f32,
    gold: u32,
    xp: u32,
    level: u32,
    next_level_xp: u32,
    skill_points: u32,
    #[serde(default = "default_skill_ranks")]
    ranks: [u8; 4],
    #[serde(default = "default_character_choice")]
    character: CharacterChoice,
    /// Authoritative class assigned at join time (kit resolution key).
    #[serde(default)]
    hero_class: HeroClass,
    /// Cosmetic roster avatar slug replicated to every client; `None` means
    /// the legacy `character` model is used.
    #[serde(default)]
    avatar: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StructureKind {
    Tower,
    BaseTower,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StructureState {
    id: u64,
    kind: StructureKind,
    team: Team,
    x: f32,
    y: f32,
    z: f32,
    hp: f32,
    max_hp: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MinionState {
    id: u64,
    team: Team,
    lane: Lane,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    hp: f32,
    max_hp: f32,
    state: MinionBrainState,
    target_kind: Option<MinionTargetKind>,
    target_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MinionBrainState {
    Marching,
    Chasing,
    Attacking,
    Dead,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MinionTargetKind {
    Player,
    Minion,
    Structure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NeutralCampType {
    Skirmisher,
    Bruiser,
    Spitter,
    /// Bottom raid boss ("Wendigo", dragon-slot objective).
    WendigoBoss,
    /// Top raid boss ("King Mutatio", Baron-slot objective).
    KingMutatioBoss,
}

impl NeutralCampType {
    fn is_boss(self) -> bool {
        matches!(
            self,
            NeutralCampType::WendigoBoss | NeutralCampType::KingMutatioBoss
        )
    }

    /// Team buff granted to the killer's team when this neutral dies.
    fn team_buff_kind(self) -> Option<TeamBuffKind> {
        match self {
            NeutralCampType::WendigoBoss => Some(TeamBuffKind::WendigoFavor),
            NeutralCampType::KingMutatioBoss => Some(TeamBuffKind::MutatioMight),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NeutralAiState {
    Idle,
    Aggro,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NeutralState {
    id: u64,
    camp_type: NeutralCampType,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    hp: f32,
    max_hp: f32,
    ai_state: NeutralAiState,
}

/// Team-wide buff kinds granted by raid-boss kills (TASK-19).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TeamBuffKind {
    /// Bottom boss (Wendigo): +ability damage.
    WendigoFavor,
    /// Top boss (King Mutatio): +ability damage and HP regen.
    MutatioMight,
}

impl TeamBuffKind {
    const ALL: [TeamBuffKind; 2] = [TeamBuffKind::WendigoFavor, TeamBuffKind::MutatioMight];

    fn index(self) -> usize {
        match self {
            TeamBuffKind::WendigoFavor => 0,
            TeamBuffKind::MutatioMight => 1,
        }
    }

    fn duration(self) -> Duration {
        match self {
            TeamBuffKind::WendigoFavor => BOTTOM_BOSS_BUFF_DURATION,
            TeamBuffKind::MutatioMight => TOP_BOSS_BUFF_DURATION,
        }
    }

    fn damage_multiplier(self) -> f32 {
        match self {
            TeamBuffKind::WendigoFavor => BOTTOM_BOSS_BUFF_DAMAGE_MULT,
            TeamBuffKind::MutatioMight => TOP_BOSS_BUFF_DAMAGE_MULT,
        }
    }

    fn hp_regen_per_second(self) -> f32 {
        match self {
            TeamBuffKind::WendigoFavor => 0.0,
            TeamBuffKind::MutatioMight => TOP_BOSS_BUFF_HP_REGEN_PER_SECOND,
        }
    }
}

/// Replicated team-buff entry (additive snapshot field, `serde(default)`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TeamBuffState {
    team: Team,
    kind: TeamBuffKind,
    remaining_secs: f32,
}

fn team_index(team: Team) -> usize {
    match team {
        Team::Green => 0,
        Team::Blue => 1,
    }
}

/// Authoritative active team buffs keyed by (team, kind) with absolute expiry
/// instants. A re-kill refreshes the expiry (no stacking of the same kind);
/// different kinds combine multiplicatively for damage.
#[derive(Default)]
struct TeamBuffs {
    /// `expires[team_index][kind_index]`
    expires: [[Option<Instant>; TeamBuffKind::ALL.len()]; 2],
}

impl TeamBuffs {
    fn grant(&mut self, team: Team, kind: TeamBuffKind, now: Instant) {
        self.expires[team_index(team)][kind.index()] = Some(now + kind.duration());
    }

    fn is_active(&self, team: Team, kind: TeamBuffKind, now: Instant) -> bool {
        self.expires[team_index(team)][kind.index()].is_some_and(|expiry| now < expiry)
    }

    /// Combined outgoing ability-damage multiplier for a team (1.0 = no buff).
    fn damage_multiplier(&self, team: Team, now: Instant) -> f32 {
        TeamBuffKind::ALL
            .iter()
            .filter(|kind| self.is_active(team, **kind, now))
            .map(|kind| kind.damage_multiplier())
            .product()
    }

    /// Combined flat HP regen per second for a team (0.0 = no buff).
    fn hp_regen_per_second(&self, team: Team, now: Instant) -> f32 {
        TeamBuffKind::ALL
            .iter()
            .filter(|kind| self.is_active(team, **kind, now))
            .map(|kind| kind.hp_regen_per_second())
            .sum()
    }

    fn clear(&mut self) {
        self.expires = Default::default();
    }

    /// Snapshot representation of every active buff (deterministic order).
    fn snapshot(&self, now: Instant) -> Vec<TeamBuffState> {
        let mut out = Vec::new();
        for team in [Team::Green, Team::Blue] {
            for kind in TeamBuffKind::ALL {
                if let Some(expiry) = self.expires[team_index(team)][kind.index()] {
                    if now < expiry {
                        out.push(TeamBuffState {
                            team,
                            kind,
                            remaining_secs: expiry.duration_since(now).as_secs_f32(),
                        });
                    }
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectileState {
    id: u64,
    owner_id: u64,
    owner_team: Team,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerPacket {
    Snapshot {
        your_id: u64,
        players: Vec<PlayerState>,
        projectiles: Vec<ProjectileState>,
        structures: Vec<StructureState>,
        minions: Vec<MinionState>,
        #[serde(default)]
        neutrals: Vec<NeutralState>,
        /// Active boss team buffs (additive field; absent = no buffs).
        #[serde(default)]
        team_buffs: Vec<TeamBuffState>,
        game_state: GameState,
        #[serde(default)]
        rematch_in_secs: Option<u64>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GameState {
    #[default]
    Lobby,
    Running,
    Victory {
        winner: Team,
    },
}

#[derive(Debug, Clone, Copy)]
struct Vec3f {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3f {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
            z: self.z + (other.z - self.z) * t,
        }
    }

    fn add_scaled(self, velocity: Self, dt: f32) -> Self {
        Self {
            x: self.x + velocity.x * dt,
            y: self.y + velocity.y * dt,
            z: self.z + velocity.z * dt,
        }
    }

    fn distance_squared(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }

    fn distance(self, other: Self) -> f32 {
        self.distance_squared(other).sqrt()
    }

    fn normalize_or_zero(self) -> Self {
        let len_sq = self.x * self.x + self.y * self.y + self.z * self.z;
        if len_sq <= 0.000_001 {
            Self::new(0.0, 0.0, 0.0)
        } else {
            let inv_len = len_sq.sqrt().recip();
            Self::new(self.x * inv_len, self.y * inv_len, self.z * inv_len)
        }
    }
}

struct ConnectedPlayer {
    state: PlayerState,
    /// False until the endpoint sends a `Join` packet. Pre-join endpoints are
    /// kept for addressing (snapshots are still sent to them) but are excluded
    /// from the replicated player list and from all gameplay simulation.
    joined: bool,
    session_id: Option<String>,
    last_seen: Instant,
    last_movement_at: Instant,
    /// Per-slot cast timestamps (Q/W/E/R); each ability cools down independently.
    last_cast_at: [Option<Instant>; 4],
    respawn_at: Option<Instant>,
    /// Debug invulnerability toggle (TASK04). Not networked; the requesting
    /// client owns the toggle and the server skips damage while it is set.
    god_mode: bool,
    /// Debug movement multiplier (1.0 = normal). Raises the server's accepted
    /// movement distance so a boosted client is not clamped as a teleport.
    speed_mult: f32,
}

struct DisconnectedSession {
    player: ConnectedPlayer,
    disconnected_at: Instant,
}

struct Projectile {
    state: ProjectileState,
    target: TargetId,
    velocity: Vec3f,
    homing: bool,
    guaranteed_hit: bool,
    damage: f32,
    radius: f32,
    expires_at: Instant,
}

struct Structure {
    state: StructureState,
    role: StructureRole,
    last_attack_at: Option<Instant>,
    attack_range: f32,
    attack_damage: f32,
    attack_cooldown: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StructureRole {
    LaneTower { lane: Lane },
    BaseTower,
}

struct Minion {
    state: MinionState,
    path: Vec<Vec3f>,
    next_waypoint: usize,
    last_attack_at: Option<Instant>,
    aggro_target: Option<MinionAggroTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MinionAggroTarget {
    Player(u64),
    Minion(u64),
}

#[derive(Debug, Clone, Copy)]
struct NeutralTemplate {
    max_hp: f32,
    attack_damage: f32,
    attack_range: f32,
    kill_gold: u32,
    kill_xp: u32,
}

struct Neutral {
    state: NeutralState,
    anchor: Vec3f,
    target_player_id: Option<u64>,
    last_attack_at: Option<Instant>,
    dead_until: Option<Instant>,
}

impl MinionAggroTarget {
    fn id(self) -> u64 {
        match self {
            MinionAggroTarget::Player(id) | MinionAggroTarget::Minion(id) => id,
        }
    }

    fn kind(self) -> MinionTargetKind {
        match self {
            MinionAggroTarget::Player(_) => MinionTargetKind::Player,
            MinionAggroTarget::Minion(_) => MinionTargetKind::Minion,
        }
    }
}

#[derive(Component)]
struct Player;

#[derive(Component)]
struct Transform3D {
    position: Vec3,
    yaw: f32,
}

#[derive(Component)]
struct Health {
    current: f32,
    max: f32,
}

#[derive(Component)]
struct Mana {
    current: f32,
    max: f32,
}

#[derive(Component)]
struct TeamMarker(Team);

#[derive(Resource, Default)]
struct EcsPlayerEntities {
    by_player_id: HashMap<u64, Entity>,
}

#[derive(Resource, Default)]
struct SimulationDeltaSeconds {
    value: f32,
}

#[derive(Resource, Default)]
struct TickContext {
    now: Option<Instant>,
    dt: f32,
}

#[derive(Resource)]
struct ServerRuntime {
    socket: UdpSocket,
    players: HashMap<SocketAddr, ConnectedPlayer>,
    disconnected_sessions: HashMap<String, DisconnectedSession>,
    projectiles: HashMap<u64, Projectile>,
    map_layout: MapLayoutState,
    structures: HashMap<u64, Structure>,
    minions: HashMap<u64, Minion>,
    game_state: GameState,
    victory_at: Option<Instant>,
    next_player_id: u64,
    next_projectile_id: u64,
    next_minion_id: u64,
    neutrals: HashMap<u64, Neutral>,
    team_buffs: TeamBuffs,
    recv_buf: [u8; MAX_PACKET_SIZE],
    last_snapshot_at: Instant,
    last_simulation_at: Instant,
    last_wave_spawn_at: Instant,
}

impl ServerRuntime {
    fn new(socket: UdpSocket) -> Self {
        let map_layout = build_map_layout();
        let mut next_neutral_id: u64 = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        // Raid bosses start dormant; the Lobby -> Running transition arms
        // their spawn schedule (see `schedule_boss_spawns`).
        neutrals.extend(build_boss_neutrals(&mut next_neutral_id));

        Self {
            socket,
            players: HashMap::new(),
            disconnected_sessions: HashMap::new(),
            projectiles: HashMap::new(),
            structures: build_structures(&map_layout),
            minions: HashMap::new(),
            game_state: GameState::Lobby,
            map_layout,
            victory_at: None,
            next_player_id: 1,
            next_projectile_id: 1,
            next_minion_id: 1,
            neutrals,
            team_buffs: TeamBuffs::default(),
            recv_buf: [0_u8; MAX_PACKET_SIZE],
            last_snapshot_at: Instant::now(),
            last_simulation_at: Instant::now(),
            last_wave_spawn_at: Instant::now()
                .checked_sub(MINION_WAVE_INTERVAL)
                .unwrap_or_else(Instant::now),
        }
    }

    fn receive_packets(&mut self) {
        let Self {
            socket,
            recv_buf,
            players,
            disconnected_sessions,
            projectiles,
            map_layout,
            structures,
            minions,
            game_state,
            victory_at,
            next_player_id,
            next_projectile_id,
            last_wave_spawn_at,
            neutrals,
            team_buffs,
            ..
        } = self;

        loop {
            match socket.recv_from(recv_buf) {
                Ok((len, addr)) => {
                    let packet = match serde_json::from_slice::<ClientPacket>(&recv_buf[..len]) {
                        Ok(packet) => packet,
                        Err(error) => {
                            eprintln!("Invalid packet from {addr}: {error}");
                            continue;
                        }
                    };

                    let now = Instant::now();

                    match packet {
                        ClientPacket::Transform { x, y, z, yaw } => {
                            ensure_player_connected(players, map_layout, addr, next_player_id, now);
                            if let Some(player) = players.get_mut(&addr) {
                                player.last_seen = now;
                            }
                            if matches!(game_state, GameState::Running)
                                && let Some(player) = players.get_mut(&addr)
                                && player.state.hp > 0.0
                            {
                                handle_transform_request(player, map_layout, x, y, z, yaw, now);
                            }
                        }
                        ClientPacket::Cast { target, slot } => {
                            ensure_player_connected(players, map_layout, addr, next_player_id, now);
                            if let Some(player) = players.get_mut(&addr) {
                                player.last_seen = now;
                            }
                            handle_cast_request(
                                players,
                                projectiles,
                                minions,
                                structures,
                                neutrals,
                                team_buffs,
                                addr,
                                target,
                                slot,
                                next_projectile_id,
                                game_state,
                                now,
                            );
                        }
                        ClientPacket::Join {
                            team,
                            character,
                            hero_class,
                            avatar,
                            session_id,
                        } => {
                            let session_id = normalize_session_id(session_id);
                            if !ensure_player_for_join(
                                players,
                                disconnected_sessions,
                                map_layout,
                                addr,
                                session_id,
                                next_player_id,
                                now,
                            ) {
                                continue;
                            }
                            if let Some(player) = players.get_mut(&addr) {
                                handle_join_request(
                                    player,
                                    team,
                                    character,
                                    hero_class,
                                    avatar.as_deref(),
                                    map_layout,
                                    now,
                                );
                            }
                            if matches!(game_state, GameState::Lobby) {
                                println!("First player joined - match starting");
                                *game_state = GameState::Running;
                                // Match start: arm the raid-boss spawn schedule.
                                schedule_boss_spawns(neutrals, now);
                                println!(
                                    "Boss schedule armed: wendigo_boss in {}s, king_mutatio_boss in {}s",
                                    BOTTOM_BOSS_SPAWN_DELAY.as_secs(),
                                    TOP_BOSS_SPAWN_DELAY.as_secs()
                                );
                            }
                        }
                        ClientPacket::Ping => {
                            ensure_player_connected(players, map_layout, addr, next_player_id, now);
                            if let Some(player) = players.get_mut(&addr) {
                                player.last_seen = now;
                            }
                        }
                        ClientPacket::RequestRematch => {
                            ensure_player_connected(players, map_layout, addr, next_player_id, now);
                            let mut joined = false;
                            if let Some(player) = players.get_mut(&addr) {
                                player.last_seen = now;
                                joined = player.joined;
                            }
                            if joined && matches!(game_state, GameState::Victory { .. }) {
                                reset_match(
                                    players,
                                    structures,
                                    minions,
                                    projectiles,
                                    neutrals,
                                    team_buffs,
                                    map_layout,
                                    last_wave_spawn_at,
                                    game_state,
                                );
                                *victory_at = None;
                            }
                        }
                        ClientPacket::SetGodMode { enabled } => {
                            ensure_player_connected(players, map_layout, addr, next_player_id, now);
                            if let Some(player) = players.get_mut(&addr) {
                                player.last_seen = now;
                                if !player.joined {
                                    continue;
                                }
                                if player.god_mode != enabled {
                                    println!("Player {} god_mode={}", player.state.id, enabled);
                                }
                                player.god_mode = enabled;
                                if enabled {
                                    player.state.hp = player.state.max_hp;
                                    player.state.mana = player.state.max_mana;
                                    player.respawn_at = None;
                                }
                            }
                        }
                        ClientPacket::SetSpeedBoost { enabled } => {
                            ensure_player_connected(players, map_layout, addr, next_player_id, now);
                            if let Some(player) = players.get_mut(&addr) {
                                player.last_seen = now;
                                if !player.joined {
                                    continue;
                                }
                                let mult = if enabled { DEBUG_SPEED_MULTIPLIER } else { 1.0 };
                                if (player.speed_mult - mult).abs() > f32::EPSILON {
                                    println!("Player {} speed_boost={}", player.state.id, enabled);
                                }
                                player.speed_mult = mult;
                            }
                        }
                        ClientPacket::UpgradeSkill { slot } => {
                            ensure_player_connected(players, map_layout, addr, next_player_id, now);
                            if let Some(player) = players.get_mut(&addr) {
                                player.last_seen = now;
                                if player.joined {
                                    apply_skill_upgrade(player, slot);
                                }
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("Socket receive error: {error}");
                    break;
                }
            }
        }
    }

    fn prepare_tick(&mut self) -> (Instant, f32) {
        self.receive_packets();

        let now = Instant::now();
        let dt = now
            .duration_since(self.last_simulation_at)
            .as_secs_f32()
            .clamp(0.0, 0.1);
        self.last_simulation_at = now;
        (now, dt)
    }

    fn simulate_after_mana(&mut self, now: Instant, dt: f32) {
        let Self {
            socket,
            players,
            disconnected_sessions,
            projectiles,
            map_layout,
            structures,
            minions,
            game_state,
            victory_at,
            next_projectile_id,
            next_minion_id,
            neutrals,
            team_buffs,
            last_wave_spawn_at,
            last_snapshot_at,
            ..
        } = self;

        spawn_minion_waves_if_due(
            map_layout,
            minions,
            next_minion_id,
            game_state,
            now,
            last_wave_spawn_at,
        );
        simulate_minions(players, minions, structures, game_state, dt, now);
        simulate_tower_attacks(
            players,
            minions,
            projectiles,
            structures,
            next_projectile_id,
            game_state,
            now,
        );
        simulate_projectiles(
            players,
            minions,
            structures,
            neutrals,
            team_buffs,
            projectiles,
            game_state,
            dt,
            now,
        );
        simulate_neutrals(players, neutrals, game_state, dt, now);
        regenerate_team_buff_hp(players, team_buffs, game_state, dt, now);
        restore_god_mode_players(players);
        handle_respawns(players, structures, map_layout, game_state, now);

        if let GameState::Victory { .. } = game_state {
            if victory_at.is_none() {
                *victory_at = Some(now);
            }
            if victory_at.is_some_and(|t| now.duration_since(t) >= VICTORY_REMATCH_DELAY) {
                reset_match(
                    players,
                    structures,
                    minions,
                    projectiles,
                    neutrals,
                    team_buffs,
                    map_layout,
                    last_wave_spawn_at,
                    game_state,
                );
                *victory_at = None;
            }
        } else {
            *victory_at = None;
        }

        let timed_out_addrs = players
            .iter()
            .filter(|(_, player)| now.duration_since(player.last_seen) > PLAYER_TIMEOUT)
            .map(|(addr, _)| *addr)
            .collect::<Vec<_>>();
        for addr in timed_out_addrs {
            let Some(player) = players.remove(&addr) else {
                continue;
            };
            println!("Player {} timed out ({addr})", player.state.id);
            if let Some(session_id) = player.session_id.clone() {
                disconnected_sessions.insert(
                    session_id,
                    DisconnectedSession {
                        player,
                        disconnected_at: now,
                    },
                );
            }
        }
        disconnected_sessions.retain(|_, session| {
            now.duration_since(session.disconnected_at) <= SESSION_RECLAIM_WINDOW
        });

        let live_player_ids = players
            .values()
            .map(|player| player.state.id)
            .collect::<HashSet<_>>();
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

        if now.duration_since(*last_snapshot_at) >= SNAPSHOT_INTERVAL {
            let players_snapshot = build_players_snapshot(players);

            let mut projectiles_snapshot = projectiles
                .values()
                .map(|projectile| projectile.state.clone())
                .collect::<Vec<_>>();
            projectiles_snapshot.sort_unstable_by_key(|projectile| projectile.id);

            let mut structures_snapshot = structures
                .values()
                .filter(|structure| structure.state.hp > 0.0)
                .map(|structure| structure.state.clone())
                .collect::<Vec<_>>();
            structures_snapshot.sort_unstable_by_key(|structure| structure.id);

            let mut minions_snapshot = minions
                .values()
                .filter(|minion| minion.state.hp > 0.0)
                .map(|minion| minion.state.clone())
                .collect::<Vec<_>>();
            minions_snapshot.sort_unstable_by_key(|minion| minion.id);

            let mut neutrals_snapshot = neutrals
                .values()
                .filter(|neutral| neutral.dead_until.is_none() && neutral.state.hp > 0.0)
                .map(|neutral| neutral.state.clone())
                .collect::<Vec<_>>();
            neutrals_snapshot.sort_unstable_by_key(|neutral| neutral.id);

            let team_buffs_snapshot = team_buffs.snapshot(now);

            let rematch_in_secs = if let GameState::Victory { .. } = game_state {
                victory_at.map(|t| {
                    VICTORY_REMATCH_DELAY
                        .saturating_sub(now.duration_since(t))
                        .as_secs()
                })
            } else {
                None
            };

            for (addr, player) in &*players {
                let packet = ServerPacket::Snapshot {
                    your_id: player.state.id,
                    players: players_snapshot.clone(),
                    projectiles: projectiles_snapshot.clone(),
                    structures: structures_snapshot.clone(),
                    minions: minions_snapshot.clone(),
                    neutrals: neutrals_snapshot.clone(),
                    team_buffs: team_buffs_snapshot.clone(),
                    game_state: game_state.clone(),
                    rematch_in_secs,
                };

                match serde_json::to_vec(&packet) {
                    Ok(payload) => {
                        if let Err(error) = socket.send_to(&payload, addr) {
                            eprintln!("Failed to send snapshot to {addr}: {error}");
                        }
                    }
                    Err(error) => eprintln!("Failed to serialize snapshot: {error}"),
                }
            }

            *last_snapshot_at = now;
        }
    }
}

/// Replicated player list: only joined players are visible to clients.
/// Pre-join endpoints keep receiving snapshots (they are still addressable)
/// but must not appear in the world as ghost players.
fn build_players_snapshot(players: &HashMap<SocketAddr, ConnectedPlayer>) -> Vec<PlayerState> {
    let mut snapshot = players
        .values()
        .filter(|player| player.joined)
        .map(|player| player.state.clone())
        .collect::<Vec<_>>();
    snapshot.sort_unstable_by_key(|player| player.id);
    snapshot
}

fn server_prepare_tick_system(
    mut runtime: ResMut<ServerRuntime>,
    mut tick: ResMut<TickContext>,
    mut simulation_delta: ResMut<SimulationDeltaSeconds>,
) {
    let (now, dt) = runtime.prepare_tick();
    tick.now = Some(now);
    tick.dt = dt;
    simulation_delta.value = dt;
}

fn sync_players_into_ecs_system(
    runtime: Res<ServerRuntime>,
    mut commands: Commands,
    mut entities: ResMut<EcsPlayerEntities>,
    mut query: Query<(&mut Transform3D, &mut Health, &mut Mana, &mut TeamMarker), With<Player>>,
) {
    let live_ids = runtime
        .players
        .values()
        .filter(|player| player.joined)
        .map(|player| player.state.id)
        .collect::<HashSet<_>>();
    let stale_ids = entities
        .by_player_id
        .keys()
        .copied()
        .filter(|player_id| !live_ids.contains(player_id))
        .collect::<Vec<_>>();

    for stale_player_id in stale_ids {
        if let Some(entity) = entities.by_player_id.remove(&stale_player_id) {
            commands.entity(entity).despawn();
        }
    }

    for connected_player in runtime.players.values() {
        if !connected_player.joined {
            continue;
        }
        let state = &connected_player.state;
        let Some(entity) = entities.by_player_id.get(&state.id).copied() else {
            let entity = commands
                .spawn((
                    Player,
                    Transform3D {
                        position: Vec3::new(state.x, state.y, state.z),
                        yaw: state.yaw,
                    },
                    Health {
                        current: state.hp,
                        max: state.max_hp,
                    },
                    Mana {
                        current: state.mana,
                        max: state.max_mana,
                    },
                    TeamMarker(state.team),
                ))
                .id();
            entities.by_player_id.insert(state.id, entity);
            continue;
        };

        if let Ok((mut transform, mut health, mut mana, mut team)) = query.get_mut(entity) {
            transform.position = Vec3::new(state.x, state.y, state.z);
            transform.yaw = state.yaw;
            health.current = state.hp;
            health.max = state.max_hp;
            mana.current = state.mana;
            mana.max = state.max_mana;
            team.0 = state.team;
        } else {
            commands.entity(entity).despawn();
            let replacement = commands
                .spawn((
                    Player,
                    Transform3D {
                        position: Vec3::new(state.x, state.y, state.z),
                        yaw: state.yaw,
                    },
                    Health {
                        current: state.hp,
                        max: state.max_hp,
                    },
                    Mana {
                        current: state.mana,
                        max: state.max_mana,
                    },
                    TeamMarker(state.team),
                ))
                .id();
            entities.by_player_id.insert(state.id, replacement);
        }
    }
}

fn regenerate_mana_system(
    simulation_delta: Res<SimulationDeltaSeconds>,
    mut query: Query<(&mut Mana, &Health), With<Player>>,
) {
    if simulation_delta.value <= 0.0 {
        return;
    }

    for (mut mana, health) in &mut query {
        if health.current <= 0.0 {
            continue;
        }
        if mana.max <= 0.0 {
            mana.max = MAX_MANA;
        }
        mana.current =
            (mana.current + MANA_REGEN_PER_SECOND * simulation_delta.value).clamp(0.0, mana.max);
    }
}

fn sync_players_from_ecs_system(
    mut runtime: ResMut<ServerRuntime>,
    entities: Res<EcsPlayerEntities>,
    query: Query<&Mana, With<Player>>,
) {
    for connected_player in runtime.players.values_mut() {
        let player_id = connected_player.state.id;
        let Some(entity) = entities.by_player_id.get(&player_id).copied() else {
            continue;
        };
        let Ok(mana) = query.get(entity) else {
            continue;
        };
        connected_player.state.mana = mana.current;
        connected_player.state.max_mana = mana.max;
    }
}

fn server_finalize_tick_system(mut runtime: ResMut<ServerRuntime>, tick: Res<TickContext>) {
    let Some(now) = tick.now else {
        return;
    };
    runtime.simulate_after_mana(now, tick.dt);
}

fn main() -> io::Result<()> {
    let bind_addr = std::env::var("SERVER_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    let socket = UdpSocket::bind(&bind_addr)?;
    socket.set_nonblocking(true)?;
    println!("UDP game server is listening on {bind_addr}");

    App::new()
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(SIMULATION_STEP_SLEEP)))
        .add_plugins(GameplayPlugin)
        .insert_resource(ServerRuntime::new(socket))
        .init_resource::<TickContext>()
        .init_resource::<SimulationDeltaSeconds>()
        .init_resource::<EcsPlayerEntities>()
        .add_systems(
            Update,
            (
                server_prepare_tick_system,
                sync_players_into_ecs_system,
                regenerate_mana_system,
                sync_players_from_ecs_system,
                gameplay::combat::sync_minions_into_ecs_system,
                gameplay::combat::collect_projectile_minion_damage_system,
                gameplay::combat::apply_projectile_minion_damage_system,
                server_finalize_tick_system,
            )
                .chain(),
        )
        .run();

    Ok(())
}

/// Spends a skill point on the given slot, capped by the class ability's max rank.
fn apply_skill_upgrade(player: &mut ConnectedPlayer, slot: u8) {
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return;
    };
    let def = ability_for_class_slot(player.state.hero_class, skill_slot);
    let s = skill_slot.index();
    if player.state.skill_points > 0 && player.state.ranks[s] < def.max_rank {
        player.state.ranks[s] += 1;
        player.state.skill_points -= 1;
        println!(
            "Player {} upgraded {} (slot {}) to rank {}",
            player.state.id, def.id, s, player.state.ranks[s]
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_cast_request(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    projectiles: &mut HashMap<u64, Projectile>,
    minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
    neutrals: &mut HashMap<u64, Neutral>,
    team_buffs: &TeamBuffs,
    caster_addr: SocketAddr,
    target: TargetId,
    slot: u8,
    next_projectile_id: &mut u64,
    game_state: &GameState,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return;
    };
    let Some(caster) = players.get(&caster_addr) else {
        return;
    };
    if !caster.joined || caster.state.hp <= 0.0 {
        return;
    }
    // Authoritative kit resolution: class + slot -> ability definition.
    let def = ability_for_class_slot(caster.state.hero_class, skill_slot);
    if !unlocked_slots_for_level(caster.state.level)[skill_slot.index()] {
        return;
    }
    let rank = caster.state.ranks[skill_slot.index()].clamp(1, def.max_rank);
    let mana_cost = scaled_mana_cost(def, rank);
    if caster.state.mana < mana_cost {
        return;
    }
    if caster.last_cast_at[skill_slot.index()]
        .is_some_and(|last_cast| now.duration_since(last_cast) < scaled_cooldown(def, rank))
    {
        return;
    }

    let effect_scale = rank_effect_scale(rank);
    if def.targeting == TargetingMode::SelfTarget {
        let Some(caster_mut) = players.get_mut(&caster_addr) else {
            return;
        };
        caster_mut.state.mana -= mana_cost;
        caster_mut.last_cast_at[skill_slot.index()] = Some(now);
        if let Some(heal) = def.self_heal {
            caster_mut.state.hp =
                (caster_mut.state.hp + heal * effect_scale).min(caster_mut.state.max_hp);
        }
        if let Some(restore) = def.self_mana_restore {
            caster_mut.state.mana =
                (caster_mut.state.mana + restore * effect_scale).min(caster_mut.state.max_mana);
        }
        return;
    }

    let caster_team = caster.state.team;
    let (target_position, target_radius) = match target.kind {
        TargetKind::Player => {
            let Some(target_player) = players.values().find(|player| {
                player.joined
                    && player.state.id == target.id
                    && player.state.hp > 0.0
                    && player.state.team != caster_team
            }) else {
                return;
            };
            (
                Vec3f::new(
                    target_player.state.x,
                    target_player.state.y + AIM_HEIGHT,
                    target_player.state.z,
                ),
                PLAYER_HIT_RADIUS,
            )
        }
        TargetKind::Minion => {
            let Some(target_minion) = minions.get(&target.id) else {
                return;
            };
            if target_minion.state.hp <= 0.0 || target_minion.state.team == caster_team {
                return;
            }
            (
                Vec3f::new(
                    target_minion.state.x,
                    target_minion.state.y + MINION_RADIUS * 0.8,
                    target_minion.state.z,
                ),
                MINION_RADIUS,
            )
        }
        TargetKind::Structure => {
            let Some(target_structure) = structures.get(&target.id) else {
                return;
            };
            if target_structure.state.hp <= 0.0 || target_structure.state.team == caster_team {
                return;
            }
            (
                Vec3f::new(
                    target_structure.state.x,
                    target_structure.state.y,
                    target_structure.state.z,
                ),
                structure_radius(target_structure.state.kind),
            )
        }
        TargetKind::Neutral => {
            let Some(target_neutral) = neutrals.get(&target.id) else {
                return;
            };
            if target_neutral.dead_until.is_some() || target_neutral.state.hp <= 0.0 {
                return;
            }
            (
                Vec3f::new(
                    target_neutral.state.x,
                    target_neutral.state.y + NEUTRAL_RADIUS * 0.85,
                    target_neutral.state.z,
                ),
                NEUTRAL_RADIUS,
            )
        }
    };

    let caster_position = Vec3f::new(
        caster.state.x,
        caster.state.y + CAST_SPAWN_HEIGHT,
        caster.state.z,
    );
    let direction = Vec3f::new(
        target_position.x - caster_position.x,
        target_position.y - caster_position.y,
        target_position.z - caster_position.z,
    )
    .normalize_or_zero();

    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
        return;
    }

    let dx = target_position.x - caster_position.x;
    let dz = target_position.z - caster_position.z;
    let horizontal_distance = (dx * dx + dz * dz).sqrt();
    if horizontal_distance > scaled_cast_range(def, rank) + target_radius {
        return;
    }

    let Some(caster_mut) = players.get_mut(&caster_addr) else {
        return;
    };
    caster_mut.state.mana -= mana_cost;
    caster_mut.last_cast_at[skill_slot.index()] = Some(now);

    // Higher invested rank = proportionally more projectile damage; active
    // boss team buffs multiply the outgoing ability damage authoritatively.
    let rank_damage = def.projectile_damage.unwrap_or(0.0)
        * effect_scale
        * team_buffs.damage_multiplier(caster_team, now);

    let projectile_id = *next_projectile_id;
    *next_projectile_id += 1;

    projectiles.insert(
        projectile_id,
        Projectile {
            state: ProjectileState {
                id: projectile_id,
                owner_id: caster_mut.state.id,
                owner_team: caster_mut.state.team,
                x: caster_position.x,
                y: caster_position.y,
                z: caster_position.z,
            },
            target,
            velocity: Vec3f::new(
                direction.x * PROJECTILE_SPEED,
                direction.y * PROJECTILE_SPEED,
                direction.z * PROJECTILE_SPEED,
            ),
            homing: true,
            guaranteed_hit: true,
            damage: rank_damage,
            radius: PROJECTILE_RADIUS,
            expires_at: now + PROJECTILE_LIFETIME,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn simulate_projectiles(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    _minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
    neutrals: &mut HashMap<u64, Neutral>,
    team_buffs: &mut TeamBuffs,
    projectiles: &mut HashMap<u64, Projectile>,
    game_state: &mut GameState,
    dt: f32,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    let mut player_damage_events: Vec<(u64, f32)> = Vec::new();
    let mut structure_damage_events: Vec<(u64, f32, Team)> = Vec::new();
    let mut neutral_damage_events: Vec<(u64, f32, u64)> = Vec::new();

    projectiles.retain(|_, projectile| {
        if !projectile.guaranteed_hit && now >= projectile.expires_at {
            return false;
        }

        match projectile.target.kind {
            TargetKind::Player => {
                let Some(target) = players.values().find(|player| {
                    player.state.id == projectile.target.id && player.state.hp > 0.0
                }) else {
                    return false;
                };

                let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
                let target_pos =
                    Vec3f::new(target.state.x, target.state.y + AIM_HEIGHT, target.state.z);
                if projectile.homing {
                    let direction = Vec3f::new(
                        target_pos.x - start.x,
                        target_pos.y - start.y,
                        target_pos.z - start.z,
                    )
                    .normalize_or_zero();
                    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                        player_damage_events.push((projectile.target.id, projectile.damage));
                        return false;
                    }
                    projectile.velocity = Vec3f::new(
                        direction.x * PROJECTILE_SPEED,
                        direction.y * PROJECTILE_SPEED,
                        direction.z * PROJECTILE_SPEED,
                    );
                }
                let end = start.add_scaled(projectile.velocity, dt);
                projectile.state.x = end.x;
                projectile.state.y = end.y;
                projectile.state.z = end.z;

                let combined_radius = projectile.radius + PLAYER_HIT_RADIUS;
                if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
                    player_damage_events.push((projectile.target.id, projectile.damage));
                    return false;
                }
            }
            TargetKind::Minion => {
                // Projectile-vs-minion collisions are handled by ECS combat systems.
            }
            TargetKind::Structure => {
                let Some(structure) = structures.get(&projectile.target.id) else {
                    return false;
                };
                if structure.state.hp <= 0.0 {
                    return false;
                }
                let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
                let target_pos =
                    Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
                if projectile.homing {
                    let direction = Vec3f::new(
                        target_pos.x - start.x,
                        target_pos.y - start.y,
                        target_pos.z - start.z,
                    )
                    .normalize_or_zero();
                    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                        structure_damage_events.push((
                            projectile.target.id,
                            projectile.damage,
                            projectile.state.owner_team,
                        ));
                        return false;
                    }
                    projectile.velocity = Vec3f::new(
                        direction.x * PROJECTILE_SPEED,
                        direction.y * PROJECTILE_SPEED,
                        direction.z * PROJECTILE_SPEED,
                    );
                }
                let end = start.add_scaled(projectile.velocity, dt);
                projectile.state.x = end.x;
                projectile.state.y = end.y;
                projectile.state.z = end.z;

                let target_radius = match structure.state.kind {
                    StructureKind::Tower => TOWER_SIZE * 0.5,
                    StructureKind::BaseTower => BASE_TOWER_SIZE * 0.5,
                };
                let combined_radius = projectile.radius + target_radius;
                if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
                    structure_damage_events.push((
                        projectile.target.id,
                        projectile.damage,
                        projectile.state.owner_team,
                    ));
                    return false;
                }
            }
            TargetKind::Neutral => {
                let Some(target_neutral) = neutrals.get(&projectile.target.id) else {
                    return false;
                };
                if target_neutral.dead_until.is_some() || target_neutral.state.hp <= 0.0 {
                    return false;
                }

                let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
                let target_pos = Vec3f::new(
                    target_neutral.state.x,
                    target_neutral.state.y + NEUTRAL_RADIUS * 0.85,
                    target_neutral.state.z,
                );
                if projectile.homing {
                    let direction = Vec3f::new(
                        target_pos.x - start.x,
                        target_pos.y - start.y,
                        target_pos.z - start.z,
                    )
                    .normalize_or_zero();
                    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                        neutral_damage_events.push((
                            projectile.target.id,
                            projectile.damage,
                            projectile.state.owner_id,
                        ));
                        return false;
                    }
                    projectile.velocity = Vec3f::new(
                        direction.x * PROJECTILE_SPEED,
                        direction.y * PROJECTILE_SPEED,
                        direction.z * PROJECTILE_SPEED,
                    );
                }
                let end = start.add_scaled(projectile.velocity, dt);
                projectile.state.x = end.x;
                projectile.state.y = end.y;
                projectile.state.z = end.z;

                let combined_radius = projectile.radius + NEUTRAL_RADIUS;
                if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
                    neutral_damage_events.push((
                        projectile.target.id,
                        projectile.damage,
                        projectile.state.owner_id,
                    ));
                    return false;
                }
            }
        }

        true
    });

    for (target_id, damage) in player_damage_events {
        if let Some(target_player) = players
            .values_mut()
            .find(|player| player.state.id == target_id && player.state.hp > 0.0)
        {
            if target_player.god_mode {
                continue;
            }
            target_player.state.hp = (target_player.state.hp - damage).max(0.0);
            if target_player.state.hp <= 0.0 && target_player.respawn_at.is_none() {
                target_player.respawn_at = Some(now + RESPAWN_DELAY);
            }
        }
    }

    for (target_id, damage, attacker_team) in structure_damage_events {
        if let Some(target_structure) = structures.get_mut(&target_id) {
            if target_structure.state.hp <= 0.0 {
                continue;
            }
            target_structure.state.hp = (target_structure.state.hp - damage).max(0.0);
            if target_structure.state.hp <= 0.0
                && target_structure.state.kind == StructureKind::BaseTower
            {
                *game_state = GameState::Victory {
                    winner: attacker_team,
                };
            }
        }
    }

    for (target_id, damage, attacker_id) in neutral_damage_events {
        apply_neutral_damage(
            players, neutrals, team_buffs, target_id, damage, attacker_id, now,
        );
    }
}

fn apply_neutral_damage(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    neutrals: &mut HashMap<u64, Neutral>,
    team_buffs: &mut TeamBuffs,
    target_id: u64,
    damage: f32,
    attacker_player_id: u64,
    now: Instant,
) {
    let Some(neutral) = neutrals.get_mut(&target_id) else {
        return;
    };
    if neutral.dead_until.is_some() || neutral.state.hp <= 0.0 {
        return;
    }
    neutral.state.hp = (neutral.state.hp - damage).max(0.0);
    if players
        .values()
        .any(|player| player.state.id == attacker_player_id && player.state.hp > 0.0)
    {
        neutral.target_player_id = Some(attacker_player_id);
        neutral.state.ai_state = NeutralAiState::Aggro;
    }
    if neutral.state.hp <= 0.0 {
        let camp_type = neutral.state.camp_type;
        award_neutral_kill_to_player(players, attacker_player_id, camp_type);
        // Boss kill: the killer's whole team gains the boss buff (refresh on
        // re-kill). Unresolvable killer (already gone) grants no buff.
        if let Some(kind) = camp_type.team_buff_kind() {
            let killer_team = players
                .values()
                .find(|player| player.state.id == attacker_player_id)
                .map(|player| player.state.team);
            if let Some(team) = killer_team {
                team_buffs.grant(team, kind, now);
                println!(
                    "Boss {camp_type:?} slain by player {attacker_player_id}; team {team:?} gains {kind:?} for {}s",
                    kind.duration().as_secs()
                );
            }
        }
        neutral.dead_until = Some(now + neutral_respawn_cooldown(camp_type));
        neutral.target_player_id = None;
        neutral.last_attack_at = None;
        neutral.state.ai_state = NeutralAiState::Idle;
    }
}

/// Applies boss-buff HP regeneration to every alive player of a buffed team,
/// clamped to max HP. Runs each simulation tick (piggybacks the regen cadence).
fn regenerate_team_buff_hp(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    team_buffs: &TeamBuffs,
    game_state: &GameState,
    dt: f32,
    now: Instant,
) {
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

fn award_neutral_kill_to_player(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    killer_id: u64,
    camp_type: NeutralCampType,
) {
    let rewards = neutral_template(camp_type);
    for player in players.values_mut() {
        if player.state.id == killer_id {
            player.state.gold = player.state.gold.saturating_add(rewards.kill_gold);
            grant_player_xp(&mut player.state, rewards.kill_xp);
            break;
        }
    }
}

fn neutral_horizontal_distance_sq_from_anchor(anchor: Vec3f, player: &PlayerState) -> f32 {
    let dx = anchor.x - player.x;
    let dz = anchor.z - player.z;
    dx * dx + dz * dz
}

fn simulate_neutrals(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    neutrals: &mut HashMap<u64, Neutral>,
    game_state: &GameState,
    dt: f32,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }

    let mut player_damage_events: Vec<(u64, f32)> = Vec::new();

    for neutral in neutrals.values_mut() {
        if let Some(dead_until) = neutral.dead_until {
            if now >= dead_until {
                let template = neutral_template(neutral.state.camp_type);
                neutral.dead_until = None;
                neutral.state.hp = template.max_hp;
                neutral.state.max_hp = template.max_hp;
                neutral.state.x = neutral.anchor.x;
                neutral.state.y = neutral.anchor.y;
                neutral.state.z = neutral.anchor.z;
                neutral.state.yaw = 0.0;
                neutral.state.ai_state = NeutralAiState::Idle;
                neutral.target_player_id = None;
                neutral.last_attack_at = None;
            } else {
                continue;
            }
        }

        if neutral.state.hp <= 0.0 {
            continue;
        }

        let template = neutral_template(neutral.state.camp_type);
        let (aggro_radius, leash_distance) = neutral_aggro_and_leash(neutral.state.camp_type);
        let aggro_sq = aggro_radius * aggro_radius;
        let leash_sq = leash_distance * leash_distance;
        let anchor = neutral.anchor;
        let neutral_pos = Vec3f::new(neutral.state.x, neutral.state.y, neutral.state.z);

        if neutral.state.ai_state == NeutralAiState::Idle && neutral.target_player_id.is_none() {
            let best = players
                .values()
                .filter(|player| player.joined && player.state.hp > 0.0)
                .map(|player| {
                    let hit =
                        Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z);
                    (player.state.id, neutral_pos.distance_squared(hit))
                })
                .filter(|(_, dist_sq)| *dist_sq <= aggro_sq)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((player_id, _)) = best {
                neutral.target_player_id = Some(player_id);
                neutral.state.ai_state = NeutralAiState::Aggro;
            }
        }

        let Some(target_id) = neutral.target_player_id else {
            neutral.state.ai_state = NeutralAiState::Idle;
            continue;
        };

        let Some(target_player) = players
            .values()
            .find(|player| player.state.id == target_id && player.state.hp > 0.0)
        else {
            neutral.target_player_id = None;
            neutral.state.ai_state = NeutralAiState::Idle;
            continue;
        };

        if neutral_horizontal_distance_sq_from_anchor(anchor, &target_player.state) > leash_sq {
            neutral.state.x = anchor.x;
            neutral.state.y = anchor.y;
            neutral.state.z = anchor.z;
            neutral.state.hp = neutral.state.max_hp;
            neutral.state.ai_state = NeutralAiState::Idle;
            neutral.target_player_id = None;
            neutral.last_attack_at = None;
            neutral.state.yaw = 0.0;
            continue;
        }

        let target_hit = Vec3f::new(
            target_player.state.x,
            target_player.state.y + AIM_HEIGHT,
            target_player.state.z,
        );
        let dist = neutral_pos.distance(target_hit);

        if dist <= template.attack_range {
            let can_attack = neutral
                .last_attack_at
                .is_none_or(|last| now.duration_since(last) >= NEUTRAL_ATTACK_COOLDOWN);
            if can_attack {
                neutral.last_attack_at = Some(now);
                player_damage_events.push((target_id, template.attack_damage));
            }
            let dir_x = target_hit.x - neutral.state.x;
            let dir_z = target_hit.z - neutral.state.z;
            if dir_x * dir_x + dir_z * dir_z > 0.0001 {
                neutral.state.yaw = dir_x.atan2(dir_z);
            }
        } else {
            neutral.state.ai_state = NeutralAiState::Aggro;
            let dir_x = target_hit.x - neutral.state.x;
            let dir_z = target_hit.z - neutral.state.z;
            let dist_flat_sq = dir_x * dir_x + dir_z * dir_z;
            let dist_flat = dist_flat_sq.sqrt();
            if dist_flat > 0.0001 {
                let travel = (NEUTRAL_CHASE_SPEED * dt).min(dist_flat);
                let inv = dist_flat.recip();
                neutral.state.x += dir_x * inv * travel;
                neutral.state.z += dir_z * inv * travel;
                neutral.state.yaw = dir_x.atan2(dir_z);
            }
        }
    }

    for (target_id, damage) in player_damage_events {
        if let Some(target_player) = players
            .values_mut()
            .find(|player| player.state.id == target_id && player.state.hp > 0.0)
        {
            if target_player.god_mode {
                continue;
            }
            target_player.state.hp = (target_player.state.hp - damage).max(0.0);
            if target_player.state.hp <= 0.0 && target_player.respawn_at.is_none() {
                target_player.respawn_at = Some(now + RESPAWN_DELAY);
            }
        }
    }
}

fn swept_sphere_intersects_target(start: Vec3f, end: Vec3f, target: Vec3f, radius: f32) -> bool {
    let seg_x = end.x - start.x;
    let seg_y = end.y - start.y;
    let seg_z = end.z - start.z;
    let seg_len_sq = seg_x * seg_x + seg_y * seg_y + seg_z * seg_z;
    if seg_len_sq <= 0.000_001 {
        return start.distance_squared(target) <= radius * radius;
    }

    let to_target_x = target.x - start.x;
    let to_target_y = target.y - start.y;
    let to_target_z = target.z - start.z;
    let t = ((to_target_x * seg_x + to_target_y * seg_y + to_target_z * seg_z) / seg_len_sq)
        .clamp(0.0, 1.0);
    let closest = Vec3f::new(
        start.x + seg_x * t,
        start.y + seg_y * t,
        start.z + seg_z * t,
    );

    closest.distance_squared(target) <= radius * radius
}

struct MapLayoutState {
    home: Vec3f,
    away: Vec3f,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
    left_x: f32,
    right_x: f32,
    top_z: f32,
    bottom_z: f32,
}

impl MapLayoutState {
    fn clamp_player_position(&self, position: Vec3f) -> Vec3f {
        Vec3f::new(
            position.x.clamp(self.min_x, self.max_x),
            PLAYER_GROUND_Y,
            position.z.clamp(self.min_z, self.max_z),
        )
    }
}

fn apply_minion_damage(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    target_id: u64,
    damage: f32,
    attacker_team: Team,
) {
    let Some(target_minion) = minions.get_mut(&target_id) else {
        return;
    };
    if target_minion.state.hp <= 0.0 {
        return;
    }
    target_minion.state.hp = (target_minion.state.hp - damage).max(0.0);
    if target_minion.state.hp <= 0.0 {
        target_minion.state.state = MinionBrainState::Dead;
        target_minion.state.target_kind = None;
        target_minion.state.target_id = None;
        award_minion_kill_rewards(players, attacker_team);
    }
}

/// Bulletproof debug invulnerability: after all damage for the tick, force god-mode
/// players back to full HP and cancel any pending respawn, so they never die even if
/// a damage path is missed (TASK04).
fn restore_god_mode_players(players: &mut HashMap<SocketAddr, ConnectedPlayer>) {
    for player in players.values_mut() {
        if player.god_mode {
            player.state.hp = player.state.max_hp;
            player.state.mana = player.state.max_mana;
            player.respawn_at = None;
        }
    }
}

fn award_minion_kill_rewards(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    attacker_team: Team,
) {
    let recipients = players
        .iter()
        .filter(|(_, player)| player.joined && player.state.team == attacker_team)
        .map(|(addr, _)| *addr)
        .collect::<Vec<_>>();
    if recipients.is_empty() {
        return;
    }

    let per_player_gold = MINION_KILL_GOLD / recipients.len() as u32;
    let per_player_xp = MINION_KILL_XP / recipients.len() as u32;
    let bonus_gold_receivers = MINION_KILL_GOLD % recipients.len() as u32;
    let bonus_xp_receivers = MINION_KILL_XP % recipients.len() as u32;

    for (index, addr) in recipients.into_iter().enumerate() {
        let Some(player) = players.get_mut(&addr) else {
            continue;
        };
        let mut gold = per_player_gold;
        if (index as u32) < bonus_gold_receivers {
            gold += 1;
        }
        let mut xp = per_player_xp;
        if (index as u32) < bonus_xp_receivers {
            xp += 1;
        }
        player.state.gold = player.state.gold.saturating_add(gold);
        grant_player_xp(&mut player.state, xp);
    }
}

fn simulate_minions(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
    game_state: &mut GameState,
    dt: f32,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }

    let player_targets = players
        .values()
        .filter(|player| player.joined && player.state.hp > 0.0)
        .map(|player| {
            (
                player.state.id,
                player.state.team,
                Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z),
            )
        })
        .collect::<Vec<_>>();
    let minion_targets = minions
        .values()
        .filter(|minion| minion.state.hp > 0.0)
        .map(|minion| {
            (
                minion.state.id,
                minion.state.team,
                Vec3f::new(minion.state.x, minion.state.y, minion.state.z),
            )
        })
        .collect::<Vec<_>>();

    let mut player_damage_events: Vec<(u64, f32)> = Vec::new();
    let mut minion_damage_events: Vec<(u64, f32, Team)> = Vec::new();
    let mut structure_damage_events: Vec<(u64, f32, Team)> = Vec::new();
    let minion_vision_sq = MINION_VISION_RANGE * MINION_VISION_RANGE;

    for minion in minions.values_mut() {
        if minion.state.hp <= 0.0 {
            minion.state.state = MinionBrainState::Dead;
            minion.state.target_kind = None;
            minion.state.target_id = None;
            continue;
        }
        minion.state.state = MinionBrainState::Marching;
        minion.state.target_kind = None;
        minion.state.target_id = None;

        let minion_position = Vec3f::new(minion.state.x, minion.state.y, minion.state.z);

        // Enemy minions always take priority. A minion never targets a player while
        // any enemy minion is within vision; players are only considered when no
        // enemy minion is in range. This overrides sticky player aggro too.
        let best_minion = minion_targets
            .iter()
            .filter(|(id, team, _)| *id != minion.state.id && *team != minion.state.team)
            .map(|(id, _, position)| {
                (
                    MinionAggroTarget::Minion(*id),
                    *position,
                    MINION_RADIUS,
                    minion_position.distance_squared(*position),
                )
            })
            .filter(|(_, _, _, dist_sq)| *dist_sq <= minion_vision_sq)
            .min_by(|left, right| {
                left.3
                    .partial_cmp(&right.3)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        let aggro_target = if let Some(minion_target) = best_minion {
            Some((minion_target.0, minion_target.1, minion_target.2))
        } else {
            // No enemy minion in range: keep sticky player aggro if still valid,
            // otherwise pick the nearest enemy player in vision.
            let sticky_player = minion.aggro_target.and_then(|target| match target {
                MinionAggroTarget::Player(target_id) => player_targets
                    .iter()
                    .find(|(id, team, position)| {
                        *id == target_id
                            && *team != minion.state.team
                            && minion_position.distance_squared(*position) <= minion_vision_sq
                    })
                    .map(|(_, _, position)| (target, *position, PLAYER_HIT_RADIUS)),
                MinionAggroTarget::Minion(_) => None,
            });

            sticky_player.or_else(|| {
                player_targets
                    .iter()
                    .filter(|(_, team, _)| *team != minion.state.team)
                    .map(|(id, _, position)| {
                        (
                            MinionAggroTarget::Player(*id),
                            *position,
                            PLAYER_HIT_RADIUS,
                            minion_position.distance_squared(*position),
                        )
                    })
                    .filter(|(_, _, _, dist_sq)| *dist_sq <= minion_vision_sq)
                    .min_by(|left, right| {
                        left.3
                            .partial_cmp(&right.3)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(target, position, radius, _)| (target, position, radius))
            })
        };

        if let Some((target, target_pos, target_radius)) = aggro_target {
            minion.aggro_target = Some(target);
            minion.state.target_kind = Some(target.kind());
            minion.state.target_id = Some(target.id());
            let dir_x = target_pos.x - minion.state.x;
            let dir_z = target_pos.z - minion.state.z;
            let distance_sq = dir_x * dir_x + dir_z * dir_z;
            let attack_distance = MINION_ATTACK_RANGE + target_radius;
            if distance_sq <= attack_distance * attack_distance {
                minion.state.state = MinionBrainState::Attacking;
                let can_attack = minion
                    .last_attack_at
                    .is_none_or(|last| now.duration_since(last) >= MINION_ATTACK_COOLDOWN);
                if can_attack {
                    minion.last_attack_at = Some(now);
                    match target {
                        MinionAggroTarget::Player(target_id) => {
                            player_damage_events.push((target_id, MINION_ATTACK_DAMAGE));
                        }
                        MinionAggroTarget::Minion(target_id) => {
                            minion_damage_events.push((
                                target_id,
                                MINION_ATTACK_DAMAGE,
                                minion.state.team,
                            ));
                        }
                    }
                }
            } else {
                minion.state.state = MinionBrainState::Chasing;
                let distance = distance_sq.sqrt();
                let travel = (MINION_SPEED * dt).min(distance);
                if distance > 0.0001 {
                    let inv_distance = distance.recip();
                    minion.state.x += dir_x * inv_distance * travel;
                    minion.state.z += dir_z * inv_distance * travel;
                }
            }
            if distance_sq > 0.0001 {
                minion.state.yaw = dir_x.atan2(dir_z);
            }
            continue;
        }

        minion.aggro_target = None;
        let target = structures
            .values()
            .filter(|structure| {
                if structure.state.hp <= 0.0 || structure.state.team == minion.state.team {
                    return false;
                }
                match structure.role {
                    StructureRole::LaneTower { lane } => lane == minion.state.lane,
                    StructureRole::BaseTower => true,
                }
            })
            .min_by(|left, right| {
                let left_pos = Vec3f::new(left.state.x, left.state.y, left.state.z);
                let right_pos = Vec3f::new(right.state.x, right.state.y, right.state.z);
                minion_position
                    .distance_squared(left_pos)
                    .partial_cmp(&minion_position.distance_squared(right_pos))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|structure| {
                let position = Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
                let distance_sq = minion_position.distance_squared(position);
                (
                    structure.state.id,
                    structure.state.kind,
                    position,
                    distance_sq,
                )
            });

        if let Some((target_id, target_kind, target_pos, distance_sq)) = target {
            minion.state.target_kind = Some(MinionTargetKind::Structure);
            minion.state.target_id = Some(target_id);
            let attack_distance = MINION_ATTACK_RANGE + structure_radius(target_kind);
            if distance_sq <= attack_distance * attack_distance {
                minion.state.state = MinionBrainState::Attacking;
                let can_attack = minion
                    .last_attack_at
                    .is_none_or(|last| now.duration_since(last) >= MINION_ATTACK_COOLDOWN);
                if can_attack {
                    minion.last_attack_at = Some(now);
                    structure_damage_events.push((
                        target_id,
                        MINION_ATTACK_DAMAGE,
                        minion.state.team,
                    ));
                }
                let dir_x = target_pos.x - minion.state.x;
                let dir_z = target_pos.z - minion.state.z;
                if dir_x * dir_x + dir_z * dir_z > 0.0001 {
                    minion.state.yaw = dir_x.atan2(dir_z);
                }
                continue;
            }
            minion.state.state = MinionBrainState::Chasing;
        }

        while minion.next_waypoint < minion.path.len() {
            let waypoint = minion.path[minion.next_waypoint];
            let dir_x = waypoint.x - minion.state.x;
            let dir_z = waypoint.z - minion.state.z;
            let distance_sq = dir_x * dir_x + dir_z * dir_z;
            if distance_sq <= 0.01 {
                minion.next_waypoint += 1;
                continue;
            }

            let distance = distance_sq.sqrt();
            let travel = (MINION_SPEED * dt).min(distance);
            let inv_distance = distance.recip();
            minion.state.x += dir_x * inv_distance * travel;
            minion.state.z += dir_z * inv_distance * travel;
            minion.state.yaw = dir_x.atan2(dir_z);
            if travel >= distance - 0.001 {
                minion.next_waypoint += 1;
            }
            break;
        }
    }

    for (target_id, damage) in player_damage_events {
        if let Some(target_player) = players
            .values_mut()
            .find(|player| player.state.id == target_id && player.state.hp > 0.0)
        {
            if target_player.god_mode {
                continue;
            }
            target_player.state.hp = (target_player.state.hp - damage).max(0.0);
            if target_player.state.hp <= 0.0 && target_player.respawn_at.is_none() {
                target_player.respawn_at = Some(now + RESPAWN_DELAY);
            }
        }
    }

    for (target_id, damage, attacker_team) in minion_damage_events {
        apply_minion_damage(players, minions, target_id, damage, attacker_team);
    }

    for (target_id, damage, attacker_team) in structure_damage_events {
        let Some(target_structure) = structures.get_mut(&target_id) else {
            continue;
        };
        if target_structure.state.hp <= 0.0 {
            continue;
        }
        target_structure.state.hp = (target_structure.state.hp - damage).max(0.0);
        if target_structure.state.hp <= 0.0
            && target_structure.state.kind == StructureKind::BaseTower
        {
            *game_state = GameState::Victory {
                winner: attacker_team,
            };
        }
    }
}

fn simulate_tower_attacks(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    projectiles: &mut HashMap<u64, Projectile>,
    structures: &mut HashMap<u64, Structure>,
    next_projectile_id: &mut u64,
    game_state: &GameState,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    let mut towers_to_fire: Vec<(Team, Vec3f, u64, Vec3f, f32, f32)> = Vec::new();
    let mut minion_damage_events: Vec<(u64, f32, Team)> = Vec::new();

    for structure in structures.values_mut() {
        if structure.state.hp <= 0.0 {
            continue;
        }
        if structure.attack_damage <= 0.0 || structure.attack_range <= 0.0 {
            continue;
        }
        if structure
            .last_attack_at
            .is_some_and(|last| now.duration_since(last) < structure.attack_cooldown)
        {
            continue;
        }

        let tower_position = Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
        let range_sq = structure.attack_range * structure.attack_range;

        let best_minion = minions
            .values()
            .filter(|minion| minion.state.hp > 0.0 && minion.state.team != structure.state.team)
            .map(|minion| {
                let pos = Vec3f::new(minion.state.x, minion.state.y, minion.state.z);
                (minion.state.id, pos, tower_position.distance_squared(pos))
            })
            .filter(|(_, _, dist_sq)| *dist_sq <= range_sq)
            .min_by(|left, right| {
                left.2
                    .partial_cmp(&right.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some((target_id, _, _)) = best_minion {
            structure.last_attack_at = Some(now);
            minion_damage_events.push((target_id, structure.attack_damage, structure.state.team));
            continue;
        }

        let mut best_target: Option<(u64, Vec3f, f32)> = None;
        for player in players.values() {
            if !player.joined || player.state.hp <= 0.0 || player.state.team == structure.state.team
            {
                continue;
            }
            let target_pos =
                Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z);
            let dist_sq = tower_position.distance_squared(target_pos);
            if dist_sq <= range_sq && best_target.is_none_or(|(_, _, best)| dist_sq < best) {
                best_target = Some((player.state.id, target_pos, dist_sq));
            }
        }

        if let Some((target_id, target_pos, _)) = best_target {
            structure.last_attack_at = Some(now);
            towers_to_fire.push((
                structure.state.team,
                tower_position,
                target_id,
                target_pos,
                structure.attack_damage,
                match structure.state.kind {
                    StructureKind::Tower => TOWER_SHOT_HEIGHT,
                    StructureKind::BaseTower => BASE_TOWER_SHOT_HEIGHT,
                },
            ));
        }
    }

    for (target_id, damage, attacker_team) in minion_damage_events {
        apply_minion_damage(players, minions, target_id, damage, attacker_team);
    }

    for (team, tower_position, target_id, target_pos, damage, shot_height) in towers_to_fire {
        let origin = Vec3f::new(
            tower_position.x,
            tower_position.y + shot_height,
            tower_position.z,
        );
        let direction = Vec3f::new(
            target_pos.x - origin.x,
            target_pos.y - origin.y,
            target_pos.z - origin.z,
        )
        .normalize_or_zero();

        if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
            continue;
        }

        let projectile_id = *next_projectile_id;
        *next_projectile_id += 1;

        projectiles.insert(
            projectile_id,
            Projectile {
                state: ProjectileState {
                    id: projectile_id,
                    owner_id: 0,
                    owner_team: team,
                    x: origin.x,
                    y: origin.y,
                    z: origin.z,
                },
                target: TargetId {
                    kind: TargetKind::Player,
                    id: target_id,
                },
                velocity: Vec3f::new(
                    direction.x * PROJECTILE_SPEED,
                    direction.y * PROJECTILE_SPEED,
                    direction.z * PROJECTILE_SPEED,
                ),
                homing: true,
                guaranteed_hit: true,
                damage,
                radius: PROJECTILE_RADIUS,
                expires_at: now + PROJECTILE_LIFETIME,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.0001;

    fn horizontal_distance(a: &PlayerState, b: &PlayerState) -> f32 {
        let dx = a.x - b.x;
        let dz = a.z - b.z;
        (dx * dx + dz * dz).sqrt()
    }

    #[test]
    fn map_layout_is_symmetric() {
        let layout = build_map_layout();
        assert!((layout.home.x + layout.away.x).abs() < EPSILON);
        assert!((layout.home.z + layout.away.z).abs() < EPSILON);
        assert!(layout.left_x < layout.right_x);
        assert!(layout.bottom_z < layout.top_z);
    }

    #[test]
    fn lane_paths_connect_bases() {
        let layout = build_map_layout();
        for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
            let points = lane_control_points(&layout, lane);
            assert!((points.first().unwrap().x - layout.home.x).abs() < EPSILON);
            assert!((points.first().unwrap().z - layout.home.z).abs() < EPSILON);
            assert!((points.last().unwrap().x - layout.away.x).abs() < EPSILON);
            assert!((points.last().unwrap().z - layout.away.z).abs() < EPSILON);
        }
    }

    #[test]
    fn blue_path_is_reverse_of_green() {
        let layout = build_map_layout();
        let green = build_minion_path(&layout, Lane::Mid, Team::Green);
        let blue = build_minion_path(&layout, Lane::Mid, Team::Blue);
        assert!((green.first().unwrap().x - layout.home.x).abs() < EPSILON);
        assert!((blue.first().unwrap().x - layout.away.x).abs() < EPSILON);
        assert!((green.last().unwrap().x - layout.away.x).abs() < EPSILON);
        assert!((blue.last().unwrap().x - layout.home.x).abs() < EPSILON);
    }

    #[test]
    fn mana_regenerates_and_is_clamped() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34567".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let player = players.get_mut(&addr).unwrap();
        player.state.mana = 10.0;
        player.state.max_mana = MAX_MANA;

        regenerate_mana(&mut players, 2.5);
        let expected = 10.0 + MANA_REGEN_PER_SECOND * 2.5;
        let current = players.get(&addr).unwrap().state.mana;
        assert!((current - expected).abs() < EPSILON);

        regenerate_mana(&mut players, 100.0);
        let clamped = players.get(&addr).unwrap().state.mana;
        assert!((clamped - MAX_MANA).abs() < EPSILON);
    }

    #[test]
    fn movement_authority_clamps_teleports_and_accepts_normal_steps() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:35001".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        handle_join_request(
            players.get_mut(&addr).unwrap(),
            Team::Green,
            CharacterChoice::Ipfs,
            HeroClass::default(),
            None,
            &layout,
            now,
        );
        let start = players.get(&addr).unwrap().state.clone();
        let normal_at = now + Duration::from_millis(100);
        let normal_x = start.x + PLAYER_SPEED * 0.1;
        handle_transform_request(
            players.get_mut(&addr).unwrap(),
            &layout,
            normal_x,
            PLAYER_GROUND_Y,
            start.z,
            0.25,
            normal_at,
        );

        let after_normal = players.get(&addr).unwrap().state.clone();
        assert!((after_normal.x - normal_x).abs() < EPSILON);
        assert!((after_normal.y - PLAYER_GROUND_Y).abs() < EPSILON);
        assert!((after_normal.yaw - 0.25).abs() < EPSILON);

        let teleport_at = normal_at + Duration::from_millis(50);
        let before_teleport = players.get(&addr).unwrap().state.clone();
        handle_transform_request(
            players.get_mut(&addr).unwrap(),
            &layout,
            before_teleport.x + 500.0,
            PLAYER_GROUND_Y,
            before_teleport.z,
            0.5,
            teleport_at,
        );

        let after_teleport = players.get(&addr).unwrap().state.clone();
        let accepted_distance = horizontal_distance(&before_teleport, &after_teleport);
        let max_distance = PLAYER_SPEED * 0.05 + MOVEMENT_POSITION_TOLERANCE + EPSILON;
        assert!(accepted_distance <= max_distance);
        assert!(after_teleport.x < before_teleport.x + 500.0);

        let before_invalid = after_teleport.clone();
        handle_transform_request(
            players.get_mut(&addr).unwrap(),
            &layout,
            f32::NAN,
            PLAYER_GROUND_Y,
            before_invalid.z + 1.0,
            1.0,
            teleport_at + Duration::from_millis(50),
        );
        let after_invalid = players.get(&addr).unwrap().state.clone();
        assert!((after_invalid.x - before_invalid.x).abs() < EPSILON);
        assert!((after_invalid.z - before_invalid.z).abs() < EPSILON);
    }

    #[test]
    fn movement_authority_keeps_players_inside_map_bounds() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:35002".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        handle_join_request(
            players.get_mut(&addr).unwrap(),
            Team::Green,
            CharacterChoice::Ipfs,
            HeroClass::default(),
            None,
            &layout,
            now,
        );
        {
            let player = players.get_mut(&addr).unwrap();
            player.state.x = layout.max_x - 0.1;
            player.state.z = layout.max_z - 0.1;
            player.last_movement_at = now;
        }

        handle_transform_request(
            players.get_mut(&addr).unwrap(),
            &layout,
            layout.max_x + 5.0,
            PLAYER_GROUND_Y,
            layout.max_z + 5.0,
            0.0,
            now + Duration::from_secs(1),
        );

        let player = players.get(&addr).unwrap();
        assert!(player.state.x <= layout.max_x);
        assert!(player.state.z <= layout.max_z);
        assert!(player.state.x >= layout.min_x);
        assert!(player.state.z >= layout.min_z);
    }

    #[test]
    fn neutral_template_matches_balance_constants() {
        let sk = neutral_template(NeutralCampType::Skirmisher);
        assert!((sk.max_hp - SKIRMISHER_MAX_HP).abs() < EPSILON);
        assert!((sk.attack_damage - SKIRMISHER_ATTACK_DAMAGE).abs() < EPSILON);
        assert!((sk.attack_range - SKIRMISHER_ATTACK_RANGE).abs() < EPSILON);
        assert_eq!(sk.kill_gold, SKIRMISHER_KILL_GOLD);
        assert_eq!(sk.kill_xp, SKIRMISHER_KILL_XP);

        let br = neutral_template(NeutralCampType::Bruiser);
        assert!((br.max_hp - BRUISER_MAX_HP).abs() < EPSILON);
        assert_eq!(br.kill_gold, BRUISER_KILL_GOLD);

        let sp = neutral_template(NeutralCampType::Spitter);
        assert!((sp.attack_range - SPITTER_ATTACK_RANGE).abs() < EPSILON);
        assert_eq!(sp.kill_xp, SPITTER_KILL_XP);
    }

    #[test]
    fn neutral_camps_spawn_alive_with_distinct_templates() {
        let mut next_neutral_id = 9_001;
        let neutrals = build_neutral_camps(&mut next_neutral_id);

        assert_eq!(neutrals.len(), 3);

        let mut camp_types = Vec::new();
        for neutral in neutrals.values() {
            let template = neutral_template(neutral.state.camp_type);
            assert!(!camp_types.contains(&neutral.state.camp_type));
            camp_types.push(neutral.state.camp_type);
            assert!((neutral.state.x - neutral.anchor.x).abs() < EPSILON);
            assert!((neutral.state.y - neutral.anchor.y).abs() < EPSILON);
            assert!((neutral.state.z - neutral.anchor.z).abs() < EPSILON);
            assert!((neutral.state.hp - template.max_hp).abs() < EPSILON);
            assert!((neutral.state.max_hp - template.max_hp).abs() < EPSILON);
            assert_eq!(neutral.state.ai_state, NeutralAiState::Idle);
            assert!(neutral.dead_until.is_none());
        }
    }

    #[test]
    fn neutral_kills_award_rewards_and_respawn_on_cooldown() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:45678".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let killer_id = players.get(&addr).unwrap().state.id;

        let mut next_neutral_id = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        let neutral_id = *neutrals.keys().next().unwrap();
        let camp_type = neutrals.get(&neutral_id).unwrap().state.camp_type;
        let template = neutral_template(camp_type);
        let kill_damage = neutrals.get(&neutral_id).unwrap().state.hp + 1.0;

        let mut team_buffs = TeamBuffs::default();
        apply_neutral_damage(
            &mut players,
            &mut neutrals,
            &mut team_buffs,
            neutral_id,
            kill_damage,
            killer_id,
            now,
        );

        let killer = players.get(&addr).unwrap();
        assert_eq!(killer.state.gold, template.kill_gold);
        assert_eq!(killer.state.xp, template.kill_xp);

        let neutral = neutrals.get(&neutral_id).unwrap();
        assert_eq!(neutral.state.hp, 0.0);
        assert_eq!(neutral.state.ai_state, NeutralAiState::Idle);
        assert!(neutral.dead_until.is_some());
        assert!(neutral.target_player_id.is_none());

        simulate_neutrals(
            &mut players,
            &mut neutrals,
            &GameState::Running,
            0.1,
            now + NEUTRAL_RESPAWN_COOLDOWN + Duration::from_millis(1),
        );

        let respawned = neutrals.get(&neutral_id).unwrap();
        assert!(respawned.dead_until.is_none());
        assert!((respawned.state.hp - template.max_hp).abs() < EPSILON);
        assert!((respawned.state.x - respawned.anchor.x).abs() < EPSILON);
        assert!((respawned.state.z - respawned.anchor.z).abs() < EPSILON);
        assert_eq!(respawned.state.ai_state, NeutralAiState::Idle);
    }

    #[test]
    fn neutral_leash_reset_restores_anchor_and_full_hp() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:56789".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let player_id = players.get(&addr).unwrap().state.id;

        let mut next_neutral_id = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        let neutral_id = *neutrals.keys().next().unwrap();
        let anchor = neutrals.get(&neutral_id).unwrap().anchor;

        {
            let neutral = neutrals.get_mut(&neutral_id).unwrap();
            neutral.state.hp -= 15.0;
            neutral.state.x = anchor.x + 2.0;
            neutral.state.z = anchor.z + 1.5;
            neutral.state.ai_state = NeutralAiState::Aggro;
            neutral.target_player_id = Some(player_id);
        }

        {
            let player = players.get_mut(&addr).unwrap();
            player.state.x = anchor.x + NEUTRAL_LEASH_DISTANCE + 2.0;
            player.state.z = anchor.z;
        }

        simulate_neutrals(&mut players, &mut neutrals, &GameState::Running, 0.1, now);

        let reset = neutrals.get(&neutral_id).unwrap();
        assert!((reset.state.x - anchor.x).abs() < EPSILON);
        assert!((reset.state.y - anchor.y).abs() < EPSILON);
        assert!((reset.state.z - anchor.z).abs() < EPSILON);
        assert!((reset.state.hp - reset.state.max_hp).abs() < EPSILON);
        assert_eq!(reset.state.ai_state, NeutralAiState::Idle);
        assert!(reset.target_player_id.is_none());
    }

    #[test]
    fn neutrals_do_not_break_minion_waves_or_tower_attacks() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:60000".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        handle_join_request(
            players.get_mut(&addr).unwrap(),
            Team::Green,
            CharacterChoice::Ipfs,
            HeroClass::default(),
            None,
            &layout,
            now,
        );

        let mut next_neutral_id = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        let focus_neutral_id = *neutrals.keys().next().unwrap();
        let focus_anchor = neutrals.get(&focus_neutral_id).unwrap().anchor;
        {
            let player = players.get_mut(&addr).unwrap();
            player.state.x = focus_anchor.x;
            player.state.z = focus_anchor.z;
        }
        simulate_neutrals(&mut players, &mut neutrals, &GameState::Running, 0.1, now);
        assert_eq!(
            neutrals.get(&focus_neutral_id).unwrap().state.ai_state,
            NeutralAiState::Aggro
        );

        let mut minions = HashMap::new();
        let mut next_minion_id = 1;
        let mut last_wave_spawn_at = now - MINION_WAVE_INTERVAL;
        spawn_minion_waves_if_due(
            &layout,
            &mut minions,
            &mut next_minion_id,
            &GameState::Running,
            now,
            &mut last_wave_spawn_at,
        );
        assert!(!minions.is_empty());

        let moving_minion_id = *minions.keys().next().unwrap();
        let before_move = {
            let minion = minions.get(&moving_minion_id).unwrap();
            (minion.state.x, minion.state.z)
        };

        let mut structures = build_structures(&layout);
        let green_tower = structures
            .values()
            .find(|structure| {
                structure.state.team == Team::Green && structure.state.kind == StructureKind::Tower
            })
            .unwrap()
            .state
            .clone();

        let enemy_minion_id = minions
            .values()
            .find(|minion| minion.state.team == Team::Blue)
            .unwrap()
            .state
            .id;
        {
            let enemy_minion = minions.get_mut(&enemy_minion_id).unwrap();
            enemy_minion.state.x = green_tower.x + 1.0;
            enemy_minion.state.z = green_tower.z + 1.0;
        }
        let tower_target_hp = minions.get(&enemy_minion_id).unwrap().state.hp;

        let mut game_state = GameState::Running;
        simulate_minions(
            &mut players,
            &mut minions,
            &mut structures,
            &mut game_state,
            0.5,
            now + Duration::from_millis(250),
        );

        let after_move = {
            let minion = minions.get(&moving_minion_id).unwrap();
            (minion.state.x, minion.state.z)
        };
        assert!(before_move != after_move);

        let mut projectiles = HashMap::new();
        let mut next_projectile_id = 1;
        simulate_tower_attacks(
            &mut players,
            &mut minions,
            &mut projectiles,
            &mut structures,
            &mut next_projectile_id,
            &GameState::Running,
            now + TOWER_COOLDOWN,
        );

        let damaged_hp = minions.get(&enemy_minion_id).unwrap().state.hp;
        assert!(damaged_hp < tower_target_hp);
        assert!(matches!(game_state, GameState::Running));
    }

    #[test]
    fn minion_prefers_enemy_minion_over_closer_player() {
        let layout = build_map_layout();
        let now = Instant::now();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let enemy_addr: SocketAddr = "127.0.0.1:34570".parse().unwrap();
        ensure_player_connected(&mut players, &layout, enemy_addr, &mut next_player_id, now);
        handle_join_request(
            players.get_mut(&enemy_addr).unwrap(),
            Team::Blue,
            CharacterChoice::Ipfs,
            HeroClass::default(),
            None,
            &layout,
            now,
        );
        {
            // Enemy player right next to the acting green minion at the origin.
            let p = players.get_mut(&enemy_addr).unwrap();
            p.state.x = 1.0;
            p.state.z = 0.0;
        }

        let make_minion = |id: u64, team: Team, x: f32| Minion {
            state: MinionState {
                id,
                team,
                lane: Lane::Mid,
                x,
                y: MINION_SPAWN_HEIGHT,
                z: 0.0,
                yaw: 0.0,
                hp: MINION_MAX_HP,
                max_hp: MINION_MAX_HP,
                state: MinionBrainState::Marching,
                target_kind: None,
                target_id: None,
            },
            path: Vec::new(),
            next_waypoint: 0,
            last_attack_at: None,
            aggro_target: None,
        };

        let mut minions = HashMap::new();
        minions.insert(1, make_minion(1, Team::Green, 0.0));
        // Enemy minion farther than the player but still within vision.
        minions.insert(2, make_minion(2, Team::Blue, MINION_VISION_RANGE * 0.5));

        let mut structures = HashMap::new();
        let mut game_state = GameState::Running;
        simulate_minions(
            &mut players,
            &mut minions,
            &mut structures,
            &mut game_state,
            0.1,
            now,
        );

        let green = minions.get(&1).unwrap();
        assert_eq!(
            green.state.target_kind,
            Some(MinionTargetKind::Minion),
            "enemy minion must take priority over the closer player"
        );
        assert_eq!(green.state.target_id, Some(2));
    }

    #[test]
    fn progression_levels_up_and_scales_stats() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34568".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let player = players.get_mut(&addr).unwrap();
        let first_threshold = player.state.next_level_xp;
        let second_threshold = xp_threshold_for_level(STARTING_LEVEL + 1);

        grant_player_xp(&mut player.state, first_threshold + second_threshold + 17);

        assert_eq!(player.state.level, STARTING_LEVEL + 2);
        assert_eq!(player.state.skill_points, 2);
        assert_eq!(player.state.xp, 17);
        assert_eq!(
            player.state.next_level_xp,
            xp_threshold_for_level(STARTING_LEVEL + 2)
        );
        assert!((player.state.max_hp - (MAX_HP + LEVEL_UP_HP_BONUS * 2.0)).abs() < EPSILON);
        assert!((player.state.max_mana - (MAX_MANA + LEVEL_UP_MANA_BONUS * 2.0)).abs() < EPSILON);
    }

    #[test]
    fn respawn_restores_scaled_maximums() {
        let layout = build_map_layout();
        let structures = build_structures(&layout);
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34569".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let player = players.get_mut(&addr).unwrap();
        let first_threshold = player.state.next_level_xp;
        let second_threshold = xp_threshold_for_level(STARTING_LEVEL + 1);
        grant_player_xp(&mut player.state, first_threshold + second_threshold);
        player.state.hp = 0.0;
        player.state.mana = 0.0;
        player.respawn_at = Some(now - Duration::from_millis(1));

        handle_respawns(&mut players, &structures, &layout, &GameState::Running, now);

        let player = players.get(&addr).unwrap();
        assert_eq!(player.state.level, STARTING_LEVEL + 2);
        assert!(player.state.max_hp > MAX_HP);
        assert!(player.state.max_mana > MAX_MANA);
        assert!((player.state.hp - player.state.max_hp).abs() < EPSILON);
        assert!((player.state.mana - player.state.max_mana).abs() < EPSILON);
    }

    #[test]
    fn session_id_reclaims_timed_out_player_from_new_endpoint() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut disconnected_sessions = HashMap::new();
        let mut next_player_id = 1;
        let old_addr: SocketAddr = "127.0.0.1:52001".parse().unwrap();
        let new_addr: SocketAddr = "127.0.0.1:52002".parse().unwrap();
        let now = Instant::now();
        let session_id = "stable-player-1".to_string();

        ensure_player_for_join(
            &mut players,
            &mut disconnected_sessions,
            &layout,
            old_addr,
            Some(session_id.clone()),
            &mut next_player_id,
            now,
        );
        let original_id = players.get(&old_addr).unwrap().state.id;
        players.get_mut(&old_addr).unwrap().last_seen =
            now - PLAYER_TIMEOUT - Duration::from_secs(1);

        assert!(ensure_player_for_join(
            &mut players,
            &mut disconnected_sessions,
            &layout,
            new_addr,
            Some(session_id.clone()),
            &mut next_player_id,
            now,
        ));

        assert!(!players.contains_key(&old_addr));
        let reclaimed = players.get(&new_addr).unwrap();
        assert_eq!(reclaimed.state.id, original_id);
        assert_eq!(reclaimed.session_id.as_deref(), Some(session_id.as_str()));
    }

    #[test]
    fn active_session_id_cannot_be_stolen_by_another_endpoint() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut disconnected_sessions = HashMap::new();
        let mut next_player_id = 1;
        let old_addr: SocketAddr = "127.0.0.1:52101".parse().unwrap();
        let new_addr: SocketAddr = "127.0.0.1:52102".parse().unwrap();
        let now = Instant::now();
        let session_id = "active-player-1".to_string();

        assert!(ensure_player_for_join(
            &mut players,
            &mut disconnected_sessions,
            &layout,
            old_addr,
            Some(session_id.clone()),
            &mut next_player_id,
            now,
        ));
        ensure_player_connected(
            &mut players,
            &layout,
            new_addr,
            &mut next_player_id,
            now + Duration::from_millis(1),
        );
        let placeholder_id = players.get(&new_addr).unwrap().state.id;
        assert!(!ensure_player_for_join(
            &mut players,
            &mut disconnected_sessions,
            &layout,
            new_addr,
            Some(session_id),
            &mut next_player_id,
            now + Duration::from_secs(1),
        ));

        assert!(players.contains_key(&old_addr));
        let placeholder = players.get(&new_addr).unwrap();
        assert_eq!(placeholder.state.id, placeholder_id);
        assert!(placeholder.session_id.is_none());
    }

    #[test]
    fn connected_placeholder_can_reclaim_timed_out_session_id() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut disconnected_sessions = HashMap::new();
        let mut next_player_id = 1;
        let old_addr: SocketAddr = "127.0.0.1:52111".parse().unwrap();
        let new_addr: SocketAddr = "127.0.0.1:52112".parse().unwrap();
        let now = Instant::now();
        let session_id = "placeholder-reclaim-1".to_string();

        assert!(ensure_player_for_join(
            &mut players,
            &mut disconnected_sessions,
            &layout,
            old_addr,
            Some(session_id.clone()),
            &mut next_player_id,
            now,
        ));
        let original_id = players.get(&old_addr).unwrap().state.id;
        players.get_mut(&old_addr).unwrap().last_seen =
            now - PLAYER_TIMEOUT - Duration::from_secs(1);

        ensure_player_connected(
            &mut players,
            &layout,
            new_addr,
            &mut next_player_id,
            now + Duration::from_millis(1),
        );
        let placeholder_id = players.get(&new_addr).unwrap().state.id;
        assert_ne!(placeholder_id, original_id);

        assert!(ensure_player_for_join(
            &mut players,
            &mut disconnected_sessions,
            &layout,
            new_addr,
            Some(session_id.clone()),
            &mut next_player_id,
            now,
        ));

        assert!(!players.contains_key(&old_addr));
        let reclaimed = players.get(&new_addr).unwrap();
        assert_eq!(reclaimed.state.id, original_id);
        assert_eq!(reclaimed.session_id.as_deref(), Some(session_id.as_str()));
    }

    #[test]
    fn stale_disconnected_session_gets_new_player_id() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut disconnected_sessions = HashMap::new();
        let mut next_player_id = 1;
        let old_addr: SocketAddr = "127.0.0.1:52201".parse().unwrap();
        let new_addr: SocketAddr = "127.0.0.1:52202".parse().unwrap();
        let now = Instant::now();
        let session_id = "stale-player-1".to_string();

        ensure_player_connected(&mut players, &layout, old_addr, &mut next_player_id, now);
        let mut old_player = players.remove(&old_addr).unwrap();
        old_player.session_id = Some(session_id.clone());
        let original_id = old_player.state.id;
        disconnected_sessions.insert(
            session_id.clone(),
            DisconnectedSession {
                player: old_player,
                disconnected_at: now - SESSION_RECLAIM_WINDOW - Duration::from_secs(1),
            },
        );

        assert!(ensure_player_for_join(
            &mut players,
            &mut disconnected_sessions,
            &layout,
            new_addr,
            Some(session_id),
            &mut next_player_id,
            now,
        ));

        assert_ne!(players.get(&new_addr).unwrap().state.id, original_id);
        assert!(disconnected_sessions.is_empty());
    }

    #[test]
    fn pre_join_endpoint_is_hidden_and_inert_until_join() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let ghost_addr: SocketAddr = "127.0.0.1:53001".parse().unwrap();
        let enemy_addr: SocketAddr = "127.0.0.1:53002".parse().unwrap();
        let now = Instant::now();

        // A heartbeat-only endpoint (Ping before Join) must not be replicated.
        ensure_player_connected(&mut players, &layout, ghost_addr, &mut next_player_id, now);
        assert!(!players.get(&ghost_addr).unwrap().joined);
        assert!(build_players_snapshot(&players).is_empty());

        // It cannot move...
        let before = players.get(&ghost_addr).unwrap().state.clone();
        handle_transform_request(
            players.get_mut(&ghost_addr).unwrap(),
            &layout,
            before.x + 1.0,
            PLAYER_GROUND_Y,
            before.z,
            1.0,
            now + Duration::from_secs(1),
        );
        let after = players.get(&ghost_addr).unwrap().state.clone();
        assert!((after.x - before.x).abs() < EPSILON);
        assert!((after.yaw - before.yaw).abs() < EPSILON);

        // ...and cannot cast, even at a valid joined enemy in range.
        ensure_player_connected(&mut players, &layout, enemy_addr, &mut next_player_id, now);
        handle_join_request(
            players.get_mut(&enemy_addr).unwrap(),
            Team::Blue,
            CharacterChoice::Wang,
            HeroClass::default(),
            None,
            &layout,
            now,
        );
        let ghost_pos = {
            let ghost = players.get(&ghost_addr).unwrap();
            (ghost.state.x, ghost.state.z)
        };
        {
            let enemy = players.get_mut(&enemy_addr).unwrap();
            enemy.state.x = ghost_pos.0 + 2.0;
            enemy.state.z = ghost_pos.1;
        }
        let enemy_id = players.get(&enemy_addr).unwrap().state.id;
        let mana_before = players.get(&ghost_addr).unwrap().state.mana;
        let mut projectiles = HashMap::new();
        let mut next_projectile_id = 1;
        cast_slot_with_buffs(
            &mut players,
            &mut projectiles,
            &TeamBuffs::default(),
            ghost_addr,
            TargetId {
                kind: TargetKind::Player,
                id: enemy_id,
            },
            0,
            &mut next_projectile_id,
            now,
        );
        assert!(projectiles.is_empty());
        assert!((players.get(&ghost_addr).unwrap().state.mana - mana_before).abs() < EPSILON);

        // Joining flips the flag and the player becomes visible in snapshots.
        handle_join_request(
            players.get_mut(&ghost_addr).unwrap(),
            Team::Green,
            CharacterChoice::Ipfs,
            HeroClass::default(),
            None,
            &layout,
            now,
        );
        let ghost_id = players.get(&ghost_addr).unwrap().state.id;
        let snapshot = build_players_snapshot(&players);
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.iter().any(|player| player.id == ghost_id));
    }

    /// Sets up two joined enemy players (caster at `caster_addr` with the given
    /// class) standing `gap` apart on the x axis, returning the target's id.
    fn setup_caster_and_target(
        players: &mut HashMap<SocketAddr, ConnectedPlayer>,
        layout: &MapLayoutState,
        caster_addr: SocketAddr,
        target_addr: SocketAddr,
        caster_class: HeroClass,
        gap: f32,
        now: Instant,
    ) -> u64 {
        let mut next_player_id = 1;
        ensure_player_connected(players, layout, caster_addr, &mut next_player_id, now);
        ensure_player_connected(players, layout, target_addr, &mut next_player_id, now);
        handle_join_request(
            players.get_mut(&caster_addr).unwrap(),
            Team::Green,
            CharacterChoice::Ipfs,
            caster_class,
            None,
            layout,
            now,
        );
        handle_join_request(
            players.get_mut(&target_addr).unwrap(),
            Team::Blue,
            CharacterChoice::Wang,
            HeroClass::default(),
            None,
            layout,
            now,
        );
        let caster_pos = {
            let caster = players.get(&caster_addr).unwrap();
            (caster.state.x, caster.state.z)
        };
        {
            let target = players.get_mut(&target_addr).unwrap();
            target.state.x = caster_pos.0 + gap;
            target.state.z = caster_pos.1;
        }
        players.get(&target_addr).unwrap().state.id
    }

    #[allow(clippy::too_many_arguments)]
    fn cast_slot_with_buffs(
        players: &mut HashMap<SocketAddr, ConnectedPlayer>,
        projectiles: &mut HashMap<u64, Projectile>,
        team_buffs: &TeamBuffs,
        caster_addr: SocketAddr,
        target: TargetId,
        slot: u8,
        next_projectile_id: &mut u64,
        now: Instant,
    ) {
        let mut minions = HashMap::new();
        let mut structures = HashMap::new();
        let mut neutrals = HashMap::new();
        handle_cast_request(
            players,
            projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            team_buffs,
            caster_addr,
            target,
            slot,
            next_projectile_id,
            &GameState::Running,
            now,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn cast_slot(
        players: &mut HashMap<SocketAddr, ConnectedPlayer>,
        projectiles: &mut HashMap<u64, Projectile>,
        caster_addr: SocketAddr,
        target: TargetId,
        slot: u8,
        next_projectile_id: &mut u64,
        now: Instant,
    ) {
        let team_buffs = TeamBuffs::default();
        cast_slot_with_buffs(
            players,
            projectiles,
            &team_buffs,
            caster_addr,
            target,
            slot,
            next_projectile_id,
            now,
        );
    }

    #[test]
    fn cast_drains_mana_respects_cooldown_and_blocks_empty_mana() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let addr_a: SocketAddr = "127.0.0.1:51001".parse().unwrap();
        let addr_b: SocketAddr = "127.0.0.1:51002".parse().unwrap();
        let now = Instant::now();
        let q = ability_for_class_slot(HeroClass::Warrior, SkillSlot::Q);
        let b_id = setup_caster_and_target(
            &mut players,
            &layout,
            addr_a,
            addr_b,
            HeroClass::Warrior,
            q.cast_range * 0.5,
            now,
        );
        let target = TargetId {
            kind: TargetKind::Player,
            id: b_id,
        };
        let a_mana_before = players.get(&addr_a).unwrap().state.mana;

        let mut projectiles = HashMap::new();
        let mut next_projectile_id = 1_u64;

        cast_slot(
            &mut players,
            &mut projectiles,
            addr_a,
            target,
            0,
            &mut next_projectile_id,
            now,
        );
        assert_eq!(projectiles.len(), 1);
        let mana_after_first = players.get(&addr_a).unwrap().state.mana;
        assert!((mana_after_first - (a_mana_before - scaled_mana_cost(q, 1))).abs() < EPSILON);

        cast_slot(
            &mut players,
            &mut projectiles,
            addr_a,
            target,
            0,
            &mut next_projectile_id,
            now,
        );
        assert_eq!(
            projectiles.len(),
            1,
            "second cast at same instant must be cooldown-blocked"
        );

        let later = now + scaled_cooldown(q, 1) + Duration::from_millis(1);
        cast_slot(
            &mut players,
            &mut projectiles,
            addr_a,
            target,
            0,
            &mut next_projectile_id,
            later,
        );
        assert_eq!(
            projectiles.len(),
            2,
            "cast after cooldown should spawn another projectile"
        );

        players.get_mut(&addr_a).unwrap().state.mana = 0.0;
        let mut next_id = 99_u64;
        cast_slot(
            &mut players,
            &mut projectiles,
            addr_a,
            target,
            0,
            &mut next_id,
            later + scaled_cooldown(q, 1),
        );
        assert_eq!(
            projectiles.len(),
            2,
            "zero mana must not create a projectile"
        );
    }

    #[test]
    fn join_applies_class_and_normalizes_avatar() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:51050".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);

        let valid_slug = shared::avatar_roster()[0].slug.clone();
        handle_join_request(
            players.get_mut(&addr).unwrap(),
            Team::Blue,
            CharacterChoice::Ipfs,
            HeroClass::Cleric,
            Some(valid_slug.as_str()),
            &layout,
            now,
        );
        {
            let state = &players.get(&addr).unwrap().state;
            assert_eq!(state.hero_class, HeroClass::Cleric);
            assert_eq!(state.avatar.as_deref(), Some(valid_slug.as_str()));
        }

        // Unknown or malicious avatar slugs must fall back to None without panicking.
        handle_join_request(
            players.get_mut(&addr).unwrap(),
            Team::Blue,
            CharacterChoice::Ipfs,
            HeroClass::Mage,
            Some("../../etc/passwd"),
            &layout,
            now,
        );
        let state = &players.get(&addr).unwrap().state;
        assert_eq!(state.hero_class, HeroClass::Mage);
        assert_eq!(state.avatar, None);
    }

    #[test]
    fn class_kits_apply_distinct_authoritative_numbers() {
        let layout = build_map_layout();
        let now = Instant::now();
        let mut results = Vec::new();
        for (index, class) in [HeroClass::Warrior, HeroClass::Mage].into_iter().enumerate() {
            let mut players = HashMap::new();
            let caster: SocketAddr = format!("127.0.0.1:5210{index}").parse().unwrap();
            let victim: SocketAddr = format!("127.0.0.1:5220{index}").parse().unwrap();
            let q = ability_for_class_slot(class, SkillSlot::Q);
            let victim_id = setup_caster_and_target(
                &mut players,
                &layout,
                caster,
                victim,
                class,
                q.cast_range * 0.5,
                now,
            );
            let mut projectiles = HashMap::new();
            let mut next_projectile_id = 1_u64;
            cast_slot(
                &mut players,
                &mut projectiles,
                caster,
                TargetId {
                    kind: TargetKind::Player,
                    id: victim_id,
                },
                0,
                &mut next_projectile_id,
                now,
            );
            assert_eq!(projectiles.len(), 1);
            let projectile = projectiles.values().next().unwrap();
            assert!((projectile.damage - q.projectile_damage.unwrap()).abs() < EPSILON);
            let mana_spent = MAX_MANA - players.get(&caster).unwrap().state.mana;
            assert!((mana_spent - q.base_mana_cost).abs() < EPSILON);
            results.push((projectile.damage, mana_spent));
        }
        assert!(
            (results[0].0 - results[1].0).abs() > EPSILON,
            "warrior and mage Q damage must differ"
        );
        assert!(
            (results[0].1 - results[1].1).abs() > EPSILON,
            "warrior and mage Q mana cost must differ"
        );
    }

    #[test]
    fn self_target_abilities_apply_heal_and_respect_unlock_gates() {
        let layout = build_map_layout();
        let now = Instant::now();
        let mut players = HashMap::new();
        let caster: SocketAddr = "127.0.0.1:52301".parse().unwrap();
        let other: SocketAddr = "127.0.0.1:52302".parse().unwrap();
        setup_caster_and_target(
            &mut players,
            &layout,
            caster,
            other,
            HeroClass::Cleric,
            5.0,
            now,
        );
        let self_target = TargetId {
            kind: TargetKind::Player,
            id: players.get(&caster).unwrap().state.id,
        };
        let w = ability_for_class_slot(HeroClass::Cleric, SkillSlot::W);
        assert_eq!(w.targeting, TargetingMode::SelfTarget);

        // Level 1: W is locked -> cast must be a complete no-op.
        {
            let state = &mut players.get_mut(&caster).unwrap().state;
            state.hp = 40.0;
        }
        let mut projectiles = HashMap::new();
        let mut next_projectile_id = 1_u64;
        cast_slot(
            &mut players,
            &mut projectiles,
            caster,
            self_target,
            1,
            &mut next_projectile_id,
            now,
        );
        {
            let state = &players.get(&caster).unwrap().state;
            assert!((state.hp - 40.0).abs() < EPSILON, "locked W must not heal");
            assert!((state.mana - MAX_MANA).abs() < EPSILON);
            assert!(projectiles.is_empty());
        }

        // Level 2 unlocks W: heal appears, mana is drained, still no projectile.
        {
            let state = &mut players.get_mut(&caster).unwrap().state;
            grant_player_xp(state, state.next_level_xp);
            assert_eq!(state.level, 2);
            state.hp = 40.0;
            state.mana = state.max_mana;
        }
        cast_slot(
            &mut players,
            &mut projectiles,
            caster,
            self_target,
            1,
            &mut next_projectile_id,
            now,
        );
        let state = &players.get(&caster).unwrap().state;
        assert!(
            (state.hp - (40.0 + w.self_heal.unwrap())).abs() < EPSILON,
            "unlocked W must heal by the kit amount"
        );
        assert!((state.mana - (state.max_mana - w.base_mana_cost)).abs() < EPSILON);
        assert!(projectiles.is_empty(), "self ability must not spawn a projectile");
    }

    #[test]
    fn rank_scaling_boosts_damage_and_upgrades_cap_at_max_rank() {
        let layout = build_map_layout();
        let now = Instant::now();
        let mut players = HashMap::new();
        let caster: SocketAddr = "127.0.0.1:52401".parse().unwrap();
        let victim: SocketAddr = "127.0.0.1:52402".parse().unwrap();
        let q = ability_for_class_slot(HeroClass::Ranger, SkillSlot::Q);
        let victim_id = setup_caster_and_target(
            &mut players,
            &layout,
            caster,
            victim,
            HeroClass::Ranger,
            q.cast_range * 0.5,
            now,
        );

        // Upgrades consume points and cap at the shared max rank (3).
        {
            let player = players.get_mut(&caster).unwrap();
            player.state.skill_points = 5;
            for _ in 0..5 {
                apply_skill_upgrade(player, 0);
            }
            assert_eq!(player.state.ranks[0], q.max_rank);
            assert_eq!(
                player.state.skill_points,
                5 - u32::from(q.max_rank - 1),
                "only rank-raising upgrades may consume points"
            );
        }

        let mut projectiles = HashMap::new();
        let mut next_projectile_id = 1_u64;
        cast_slot(
            &mut players,
            &mut projectiles,
            caster,
            TargetId {
                kind: TargetKind::Player,
                id: victim_id,
            },
            0,
            &mut next_projectile_id,
            now,
        );
        let projectile = projectiles.values().next().expect("rank-3 cast fires");
        let expected = q.projectile_damage.unwrap() * rank_effect_scale(q.max_rank);
        assert!((projectile.damage - expected).abs() < EPSILON);
        let mana_spent = MAX_MANA - players.get(&caster).unwrap().state.mana;
        assert!((mana_spent - scaled_mana_cost(q, q.max_rank)).abs() < EPSILON);
    }

    #[test]
    fn cast_range_validation_covers_target_types_and_rejects_far_targets() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let caster_addr: SocketAddr = "127.0.0.1:53001".parse().unwrap();
        let target_addr: SocketAddr = "127.0.0.1:53002".parse().unwrap();
        let now = Instant::now();
        // Mage Q has the longest basic range; targets below sit at half range.
        let q = ability_for_class_slot(HeroClass::Mage, SkillSlot::Q);
        let cast_range = q.cast_range;
        let cooldown = scaled_cooldown(q, 1);

        ensure_player_connected(&mut players, &layout, caster_addr, &mut next_player_id, now);
        ensure_player_connected(&mut players, &layout, target_addr, &mut next_player_id, now);
        handle_join_request(
            players.get_mut(&caster_addr).unwrap(),
            Team::Green,
            CharacterChoice::Ipfs,
            HeroClass::Mage,
            None,
            &layout,
            now,
        );
        handle_join_request(
            players.get_mut(&target_addr).unwrap(),
            Team::Blue,
            CharacterChoice::Wang,
            HeroClass::default(),
            None,
            &layout,
            now,
        );
        {
            let caster = players.get_mut(&caster_addr).unwrap();
            caster.state.x = 0.0;
            caster.state.z = 0.0;
        }
        {
            let target = players.get_mut(&target_addr).unwrap();
            target.state.x = cast_range * 0.5;
            target.state.z = 0.0;
        }

        let mut minions = HashMap::new();
        minions.insert(
            10,
            Minion {
                state: MinionState {
                    id: 10,
                    team: Team::Blue,
                    lane: Lane::Mid,
                    x: 0.0,
                    y: MINION_SPAWN_HEIGHT,
                    z: cast_range * 0.5,
                    yaw: 0.0,
                    hp: MINION_MAX_HP,
                    max_hp: MINION_MAX_HP,
                    state: MinionBrainState::Marching,
                    target_kind: None,
                    target_id: None,
                },
                path: Vec::new(),
                next_waypoint: 0,
                last_attack_at: None,
                aggro_target: None,
            },
        );

        let mut structures = HashMap::new();
        let mut structure_id = 20;
        add_structure(
            &mut structures,
            &mut structure_id,
            StructureKind::Tower,
            StructureRole::LaneTower { lane: Lane::Mid },
            Team::Blue,
            Vec3f::new(cast_range * 0.5, 3.0, cast_range * 0.25),
        );

        let mut next_neutral_id = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        let neutral_id = *neutrals.keys().next().unwrap();
        {
            let neutral = neutrals.get_mut(&neutral_id).unwrap();
            neutral.state.x = cast_range * 0.25;
            neutral.state.z = cast_range * 0.5;
        }

        let mut projectiles = HashMap::new();
        let mut next_projectile_id = 1_u64;
        let game_state = GameState::Running;
        let target_player_id = players.get(&target_addr).unwrap().state.id;
        let targets = [
            TargetId {
                kind: TargetKind::Player,
                id: target_player_id,
            },
            TargetId {
                kind: TargetKind::Minion,
                id: 10,
            },
            TargetId {
                kind: TargetKind::Structure,
                id: 20,
            },
            TargetId {
                kind: TargetKind::Neutral,
                id: neutral_id,
            },
        ];

        let team_buffs = TeamBuffs::default();
        for (index, target) in targets.into_iter().enumerate() {
            players.get_mut(&caster_addr).unwrap().state.mana = MAX_MANA;
            handle_cast_request(
                &mut players,
                &mut projectiles,
                &mut minions,
                &mut structures,
                &mut neutrals,
                &team_buffs,
                caster_addr,
                target,
                0,
                &mut next_projectile_id,
                &game_state,
                now + (cooldown + Duration::from_millis(1)) * index as u32,
            );
            assert_eq!(projectiles.len(), index + 1);
        }

        {
            let target = players.get_mut(&target_addr).unwrap();
            target.state.x = cast_range + PLAYER_HIT_RADIUS + 10.0;
            target.state.z = 0.0;
        }
        let mana_before = players.get(&caster_addr).unwrap().state.mana;
        let projectile_count = projectiles.len();
        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            &team_buffs,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: target_player_id,
            },
            0,
            &mut next_projectile_id,
            &game_state,
            now + (cooldown + Duration::from_millis(1)) * 5,
        );

        assert_eq!(projectiles.len(), projectile_count);
        assert!((players.get(&caster_addr).unwrap().state.mana - mana_before).abs() < EPSILON);
    }

    // --- TASK-19 raid bosses -------------------------------------------------

    /// Camp types visible under the snapshot filter (alive and not respawn-gated).
    fn visible_camp_types(neutrals: &HashMap<u64, Neutral>) -> Vec<NeutralCampType> {
        neutrals
            .values()
            .filter(|neutral| neutral.dead_until.is_none() && neutral.state.hp > 0.0)
            .map(|neutral| neutral.state.camp_type)
            .collect()
    }

    fn build_camps_and_bosses() -> HashMap<u64, Neutral> {
        let mut next_neutral_id = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        neutrals.extend(build_boss_neutrals(&mut next_neutral_id));
        neutrals
    }

    fn find_boss_id(neutrals: &HashMap<u64, Neutral>, camp_type: NeutralCampType) -> u64 {
        neutrals
            .values()
            .find(|neutral| neutral.state.camp_type == camp_type)
            .map(|neutral| neutral.state.id)
            .expect("boss neutral must exist")
    }

    #[test]
    fn boss_templates_use_boss_constants_and_outclass_camps() {
        let wendigo = neutral_template(NeutralCampType::WendigoBoss);
        assert!((wendigo.max_hp - WENDIGO_MAX_HP).abs() < EPSILON);
        assert!((wendigo.attack_damage - WENDIGO_ATTACK_DAMAGE).abs() < EPSILON);
        assert!((wendigo.attack_range - WENDIGO_ATTACK_RANGE).abs() < EPSILON);
        assert_eq!(wendigo.kill_gold, WENDIGO_KILL_GOLD);
        assert_eq!(wendigo.kill_xp, WENDIGO_KILL_XP);

        let mutatio = neutral_template(NeutralCampType::KingMutatioBoss);
        assert!((mutatio.max_hp - MUTATIO_MAX_HP).abs() < EPSILON);
        assert!((mutatio.attack_damage - MUTATIO_ATTACK_DAMAGE).abs() < EPSILON);
        assert!((mutatio.attack_range - MUTATIO_ATTACK_RANGE).abs() < EPSILON);
        assert_eq!(mutatio.kill_gold, MUTATIO_KILL_GOLD);
        assert_eq!(mutatio.kill_xp, MUTATIO_KILL_XP);

        for camp in [
            NeutralCampType::Skirmisher,
            NeutralCampType::Bruiser,
            NeutralCampType::Spitter,
        ] {
            let template = neutral_template(camp);
            assert!(wendigo.max_hp > template.max_hp);
            assert!(mutatio.max_hp > template.max_hp);
            assert!(wendigo.attack_damage > template.attack_damage);
            assert!(!camp.is_boss());
        }
        assert!(NeutralCampType::WendigoBoss.is_boss());
        assert!(NeutralCampType::KingMutatioBoss.is_boss());
    }

    #[test]
    fn boss_pits_are_point_symmetric_and_clear_of_camps() {
        let bosses = boss_blueprints();
        assert_eq!(bosses.len(), 2);
        let (wendigo_anchor, wendigo_type) = bosses[0];
        let (mutatio_anchor, mutatio_type) = bosses[1];
        assert_eq!(wendigo_type, NeutralCampType::WendigoBoss);
        assert_eq!(mutatio_type, NeutralCampType::KingMutatioBoss);

        // Bottom boss sits in negative-z (bottom-lane) territory, top boss in
        // positive-z; the pits are 180-degree rotationally symmetric.
        assert!(wendigo_anchor.z < 0.0 && mutatio_anchor.z > 0.0);
        assert!((wendigo_anchor.x + mutatio_anchor.x).abs() < EPSILON);
        assert!((wendigo_anchor.z + mutatio_anchor.z).abs() < EPSILON);

        // Pits stay clear of every jungle camp slot.
        for (camp_anchor, _) in jungle_camp_blueprints() {
            for (boss_anchor, _) in &bosses {
                assert!(
                    camp_anchor.distance(*boss_anchor) > 5.0,
                    "boss pit overlaps a camp"
                );
            }
        }
    }

    #[test]
    fn bosses_are_gated_before_spawn_delays_and_spawn_with_full_stats() {
        let mut players = HashMap::new();
        let mut neutrals = build_camps_and_bosses();
        let now = Instant::now();

        // Before match start (Lobby): bosses dormant, only the 3 camps visible.
        let visible = visible_camp_types(&neutrals);
        assert_eq!(visible.len(), 3);
        assert!(visible.iter().all(|camp_type| !camp_type.is_boss()));

        schedule_boss_spawns(&mut neutrals, now);

        // Just before the bottom-boss delay: still no boss.
        let before_bottom = now + BOTTOM_BOSS_SPAWN_DELAY - Duration::from_millis(1);
        simulate_neutrals(&mut players, &mut neutrals, &GameState::Running, 0.1, before_bottom);
        assert!(
            visible_camp_types(&neutrals)
                .iter()
                .all(|camp_type| !camp_type.is_boss())
        );

        // At/after the bottom delay: Wendigo up at its pit with full boss HP,
        // top boss still gated.
        let after_bottom = now + BOTTOM_BOSS_SPAWN_DELAY + Duration::from_millis(1);
        simulate_neutrals(&mut players, &mut neutrals, &GameState::Running, 0.1, after_bottom);
        let visible = visible_camp_types(&neutrals);
        assert!(visible.contains(&NeutralCampType::WendigoBoss));
        assert!(!visible.contains(&NeutralCampType::KingMutatioBoss));
        let wendigo = neutrals
            .values()
            .find(|neutral| neutral.state.camp_type == NeutralCampType::WendigoBoss)
            .unwrap();
        assert!((wendigo.state.hp - WENDIGO_MAX_HP).abs() < EPSILON);
        assert!((wendigo.state.x - wendigo.anchor.x).abs() < EPSILON);
        assert!((wendigo.state.z - wendigo.anchor.z).abs() < EPSILON);

        // At/after the top delay: King Mutatio up as well.
        let after_top = now + TOP_BOSS_SPAWN_DELAY + Duration::from_millis(1);
        simulate_neutrals(&mut players, &mut neutrals, &GameState::Running, 0.1, after_top);
        let visible = visible_camp_types(&neutrals);
        assert!(visible.contains(&NeutralCampType::KingMutatioBoss));
        let mutatio = neutrals
            .values()
            .find(|neutral| neutral.state.camp_type == NeutralCampType::KingMutatioBoss)
            .unwrap();
        assert!((mutatio.state.hp - MUTATIO_MAX_HP).abs() < EPSILON);
        assert!((mutatio.state.x - mutatio.anchor.x).abs() < EPSILON);
        assert!((mutatio.state.z - mutatio.anchor.z).abs() < EPSILON);

        // Camps were never gated by the boss schedule.
        let camp_count = visible
            .iter()
            .filter(|camp_type| !camp_type.is_boss())
            .count();
        assert_eq!(camp_count, 3);
    }

    #[test]
    fn boss_kill_grants_team_buff_and_respawns_on_boss_cooldown() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let killer_addr: SocketAddr = "127.0.0.1:47001".parse().unwrap();
        let enemy_addr: SocketAddr = "127.0.0.1:47002".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, killer_addr, &mut next_player_id, now);
        ensure_player_connected(&mut players, &layout, enemy_addr, &mut next_player_id, now);
        players.get_mut(&killer_addr).unwrap().state.team = Team::Green;
        players.get_mut(&enemy_addr).unwrap().state.team = Team::Blue;
        let killer_id = players.get(&killer_addr).unwrap().state.id;

        let mut neutrals = build_camps_and_bosses();
        schedule_boss_spawns(&mut neutrals, now);
        let spawn_at = now + BOTTOM_BOSS_SPAWN_DELAY + Duration::from_millis(1);
        simulate_neutrals(&mut players, &mut neutrals, &GameState::Running, 0.1, spawn_at);

        let wendigo_id = find_boss_id(&neutrals, NeutralCampType::WendigoBoss);
        let mut team_buffs = TeamBuffs::default();
        let kill_at = spawn_at + Duration::from_secs(5);
        apply_neutral_damage(
            &mut players,
            &mut neutrals,
            &mut team_buffs,
            wendigo_id,
            WENDIGO_MAX_HP + 1.0,
            killer_id,
            kill_at,
        );

        // Killer got the individual reward; the whole killing team got the buff.
        let killer = players.get(&killer_addr).unwrap();
        assert_eq!(killer.state.gold, WENDIGO_KILL_GOLD);
        assert!(team_buffs.is_active(Team::Green, TeamBuffKind::WendigoFavor, kill_at));
        assert!(!team_buffs.is_active(Team::Blue, TeamBuffKind::WendigoFavor, kill_at));
        assert!(
            (team_buffs.damage_multiplier(Team::Green, kill_at) - BOTTOM_BOSS_BUFF_DAMAGE_MULT)
                .abs()
                < EPSILON
        );
        assert!((team_buffs.damage_multiplier(Team::Blue, kill_at) - 1.0).abs() < EPSILON);

        // The boss stays down through the camp cooldown (40s) and until its own
        // 180s cooldown elapses, then respawns at the pit at full HP.
        let after_camp_cooldown = kill_at + NEUTRAL_RESPAWN_COOLDOWN + Duration::from_millis(1);
        simulate_neutrals(
            &mut players,
            &mut neutrals,
            &GameState::Running,
            0.1,
            after_camp_cooldown,
        );
        assert!(
            !visible_camp_types(&neutrals).contains(&NeutralCampType::WendigoBoss),
            "boss must not reuse the camp respawn cooldown"
        );

        let before_boss_respawn = kill_at + BOSS_RESPAWN_COOLDOWN - Duration::from_millis(1);
        simulate_neutrals(
            &mut players,
            &mut neutrals,
            &GameState::Running,
            0.1,
            before_boss_respawn,
        );
        assert!(!visible_camp_types(&neutrals).contains(&NeutralCampType::WendigoBoss));

        let after_boss_respawn = kill_at + BOSS_RESPAWN_COOLDOWN + Duration::from_millis(1);
        simulate_neutrals(
            &mut players,
            &mut neutrals,
            &GameState::Running,
            0.1,
            after_boss_respawn,
        );
        let wendigo = neutrals.get(&wendigo_id).unwrap();
        assert!(wendigo.dead_until.is_none());
        assert!((wendigo.state.hp - WENDIGO_MAX_HP).abs() < EPSILON);
        assert!((wendigo.state.x - wendigo.anchor.x).abs() < EPSILON);
        assert!((wendigo.state.z - wendigo.anchor.z).abs() < EPSILON);
    }

    #[test]
    fn camp_kills_do_not_grant_team_buffs() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:47003".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let killer_id = players.get(&addr).unwrap().state.id;

        let mut next_neutral_id = 9_001;
        let mut neutrals = build_neutral_camps(&mut next_neutral_id);
        let camp_id = *neutrals.keys().next().unwrap();
        let mut team_buffs = TeamBuffs::default();
        apply_neutral_damage(
            &mut players,
            &mut neutrals,
            &mut team_buffs,
            camp_id,
            10_000.0,
            killer_id,
            now,
        );
        for team in [Team::Green, Team::Blue] {
            assert!((team_buffs.damage_multiplier(team, now) - 1.0).abs() < EPSILON);
            assert!(team_buffs.hp_regen_per_second(team, now) == 0.0);
        }
        assert!(team_buffs.snapshot(now).is_empty());
    }

    #[test]
    fn team_buffs_expire_refresh_and_stack_multiplicatively() {
        let mut buffs = TeamBuffs::default();
        let now = Instant::now();
        buffs.grant(Team::Green, TeamBuffKind::WendigoFavor, now);

        let almost_expired = now + BOTTOM_BOSS_BUFF_DURATION - Duration::from_millis(1);
        assert!(buffs.is_active(Team::Green, TeamBuffKind::WendigoFavor, almost_expired));
        let expired = now + BOTTOM_BOSS_BUFF_DURATION;
        assert!(!buffs.is_active(Team::Green, TeamBuffKind::WendigoFavor, expired));
        assert!((buffs.damage_multiplier(Team::Green, expired) - 1.0).abs() < EPSILON);

        // Re-kill refreshes the expiry from the new kill instant.
        let rekill_at = now + Duration::from_secs(30);
        buffs.grant(Team::Green, TeamBuffKind::WendigoFavor, rekill_at);
        assert!(buffs.is_active(
            Team::Green,
            TeamBuffKind::WendigoFavor,
            rekill_at + BOTTOM_BOSS_BUFF_DURATION - Duration::from_millis(1)
        ));
        assert!(!buffs.is_active(
            Team::Green,
            TeamBuffKind::WendigoFavor,
            rekill_at + BOTTOM_BOSS_BUFF_DURATION
        ));

        // Both buffs active for one team combine multiplicatively.
        buffs.grant(Team::Green, TeamBuffKind::MutatioMight, rekill_at);
        let both_active = rekill_at + Duration::from_secs(1);
        let expected = BOTTOM_BOSS_BUFF_DAMAGE_MULT * TOP_BOSS_BUFF_DAMAGE_MULT;
        assert!((buffs.damage_multiplier(Team::Green, both_active) - expected).abs() < EPSILON);
        assert!(
            (buffs.hp_regen_per_second(Team::Green, both_active)
                - TOP_BOSS_BUFF_HP_REGEN_PER_SECOND)
                .abs()
                < EPSILON
        );
        // Enemy team remains unaffected throughout.
        assert!((buffs.damage_multiplier(Team::Blue, both_active) - 1.0).abs() < EPSILON);
        assert!(buffs.hp_regen_per_second(Team::Blue, both_active) == 0.0);

        // Snapshot carries both active entries with sane remaining times.
        let snapshot = buffs.snapshot(both_active);
        assert_eq!(snapshot.len(), 2);
        for entry in &snapshot {
            assert_eq!(entry.team, Team::Green);
            assert!(entry.remaining_secs > 0.0);
            assert!(entry.remaining_secs <= TOP_BOSS_BUFF_DURATION.as_secs_f32());
        }

        buffs.clear();
        assert!(buffs.snapshot(both_active).is_empty());
    }

    #[test]
    fn team_buff_multiplies_cast_damage_for_buffed_team_only() {
        let layout = build_map_layout();
        let now = Instant::now();
        let mut players = HashMap::new();
        let caster_addr: SocketAddr = "127.0.0.1:47101".parse().unwrap();
        let target_addr: SocketAddr = "127.0.0.1:47102".parse().unwrap();
        let q = ability_for_class_slot(HeroClass::Warrior, SkillSlot::Q);
        let target_id = setup_caster_and_target(
            &mut players,
            &layout,
            caster_addr,
            target_addr,
            HeroClass::Warrior,
            q.cast_range * 0.5,
            now,
        );
        let caster_id = players.get(&caster_addr).unwrap().state.id;
        let base_damage = q.projectile_damage.unwrap();

        // Buff the caster's team (Green): outgoing damage is multiplied.
        let mut team_buffs = TeamBuffs::default();
        team_buffs.grant(Team::Green, TeamBuffKind::WendigoFavor, now);
        let mut projectiles = HashMap::new();
        let mut next_projectile_id = 1_u64;
        cast_slot_with_buffs(
            &mut players,
            &mut projectiles,
            &team_buffs,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: target_id,
            },
            0,
            &mut next_projectile_id,
            now,
        );
        let buffed = projectiles.values().next().expect("buffed cast fires");
        assert!(
            (buffed.damage - base_damage * BOTTOM_BOSS_BUFF_DAMAGE_MULT).abs() < EPSILON,
            "buffed team damage must include the boss multiplier"
        );

        // The enemy (Blue) caster gets no multiplier from Green's buff.
        projectiles.clear();
        cast_slot_with_buffs(
            &mut players,
            &mut projectiles,
            &team_buffs,
            target_addr,
            TargetId {
                kind: TargetKind::Player,
                id: caster_id,
            },
            0,
            &mut next_projectile_id,
            now,
        );
        let unbuffed = projectiles.values().next().expect("enemy cast fires");
        assert!((unbuffed.damage - base_damage).abs() < EPSILON);

        // Both buffs active: multiplicative stacking on the buffed team.
        team_buffs.grant(Team::Green, TeamBuffKind::MutatioMight, now);
        projectiles.clear();
        let later = now + scaled_cooldown(q, 1) + Duration::from_millis(1);
        cast_slot_with_buffs(
            &mut players,
            &mut projectiles,
            &team_buffs,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: target_id,
            },
            0,
            &mut next_projectile_id,
            later,
        );
        let double_buffed = projectiles.values().next().expect("double-buffed cast fires");
        let expected = base_damage * BOTTOM_BOSS_BUFF_DAMAGE_MULT * TOP_BOSS_BUFF_DAMAGE_MULT;
        assert!((double_buffed.damage - expected).abs() < EPSILON);

        // After expiry the multiplier is gone.
        projectiles.clear();
        let after_expiry = now + TOP_BOSS_BUFF_DURATION + scaled_cooldown(q, 1);
        cast_slot_with_buffs(
            &mut players,
            &mut projectiles,
            &team_buffs,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: target_id,
            },
            0,
            &mut next_projectile_id,
            after_expiry,
        );
        let expired = projectiles.values().next().expect("post-expiry cast fires");
        assert!((expired.damage - base_damage).abs() < EPSILON);
    }

    #[test]
    fn top_buff_regenerates_hp_for_alive_buffed_players_only() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let green_addr: SocketAddr = "127.0.0.1:47201".parse().unwrap();
        let blue_addr: SocketAddr = "127.0.0.1:47202".parse().unwrap();
        let dead_addr: SocketAddr = "127.0.0.1:47203".parse().unwrap();
        let now = Instant::now();
        for addr in [green_addr, blue_addr, dead_addr] {
            ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
            // Buff regen only applies to joined players.
            players.get_mut(&addr).unwrap().joined = true;
        }
        players.get_mut(&green_addr).unwrap().state.team = Team::Green;
        players.get_mut(&blue_addr).unwrap().state.team = Team::Blue;
        players.get_mut(&dead_addr).unwrap().state.team = Team::Green;
        players.get_mut(&green_addr).unwrap().state.hp = 50.0;
        players.get_mut(&blue_addr).unwrap().state.hp = 50.0;
        players.get_mut(&dead_addr).unwrap().state.hp = 0.0;

        let mut team_buffs = TeamBuffs::default();
        team_buffs.grant(Team::Green, TeamBuffKind::MutatioMight, now);

        regenerate_team_buff_hp(&mut players, &team_buffs, &GameState::Running, 1.0, now);
        let expected = 50.0 + TOP_BOSS_BUFF_HP_REGEN_PER_SECOND;
        assert!((players.get(&green_addr).unwrap().state.hp - expected).abs() < EPSILON);
        assert!((players.get(&blue_addr).unwrap().state.hp - 50.0).abs() < EPSILON);
        assert!(players.get(&dead_addr).unwrap().state.hp == 0.0);

        // Regen clamps to max HP.
        regenerate_team_buff_hp(&mut players, &team_buffs, &GameState::Running, 10_000.0, now);
        let green = players.get(&green_addr).unwrap();
        assert!((green.state.hp - green.state.max_hp).abs() < EPSILON);

        // No regen after the buff expires.
        players.get_mut(&green_addr).unwrap().state.hp = 50.0;
        let expired = now + TOP_BOSS_BUFF_DURATION;
        regenerate_team_buff_hp(&mut players, &team_buffs, &GameState::Running, 1.0, expired);
        assert!((players.get(&green_addr).unwrap().state.hp - 50.0).abs() < EPSILON);
    }

    #[test]
    fn reset_match_clears_buffs_and_restarts_boss_schedule() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:47301".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);

        let mut structures = build_structures(&layout);
        let mut minions = HashMap::new();
        let mut projectiles = HashMap::new();
        let mut neutrals = build_camps_and_bosses();
        schedule_boss_spawns(&mut neutrals, now);
        // Bring both bosses up, then grant a buff as if one was killed.
        simulate_neutrals(
            &mut players,
            &mut neutrals,
            &GameState::Running,
            0.1,
            now + TOP_BOSS_SPAWN_DELAY + Duration::from_millis(1),
        );
        let mut team_buffs = TeamBuffs::default();
        team_buffs.grant(Team::Green, TeamBuffKind::WendigoFavor, now);

        let mut last_wave_spawn_at = now;
        let mut game_state = GameState::Victory {
            winner: Team::Green,
        };
        reset_match(
            &mut players,
            &mut structures,
            &mut minions,
            &mut projectiles,
            &mut neutrals,
            &mut team_buffs,
            &layout,
            &mut last_wave_spawn_at,
            &mut game_state,
        );

        assert!(matches!(game_state, GameState::Running));
        // Buffs cleared for both teams.
        let later = now + Duration::from_secs(1);
        for team in [Team::Green, Team::Blue] {
            assert!((team_buffs.damage_multiplier(team, later) - 1.0).abs() < EPSILON);
        }
        // Bosses re-gated on the fresh schedule; camps untouched.
        for neutral in neutrals.values() {
            if neutral.state.camp_type.is_boss() {
                assert!(neutral.dead_until.is_some());
                assert!(neutral.state.hp <= 0.0);
            } else {
                assert!(neutral.dead_until.is_none());
                assert!(neutral.state.hp > 0.0);
            }
        }
    }

    #[test]
    fn boss_and_buff_wire_formats_are_snake_case_and_additive() {
        assert_eq!(
            serde_json::to_string(&NeutralCampType::WendigoBoss).unwrap(),
            "\"wendigo_boss\""
        );
        assert_eq!(
            serde_json::to_string(&NeutralCampType::KingMutatioBoss).unwrap(),
            "\"king_mutatio_boss\""
        );
        let entry = TeamBuffState {
            team: Team::Green,
            kind: TeamBuffKind::MutatioMight,
            remaining_secs: 12.5,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"mutatio_might\""));
        assert!(json.contains("\"green\""));

        // The snapshot stays decodable without the new field (serde default).
        let legacy = r#"{"type":"snapshot","your_id":1,"players":[],"projectiles":[],"structures":[],"minions":[],"game_state":{"type":"lobby"}}"#;
        let packet: ServerPacket = serde_json::from_str(legacy).expect("legacy snapshot decodes");
        let ServerPacket::Snapshot { team_buffs, .. } = packet;
        assert!(team_buffs.is_empty());
    }

    #[test]
    fn boss_leash_reset_uses_boss_distance_and_restores_full_hp() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:47401".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let player_id = players.get(&addr).unwrap().state.id;

        let mut neutrals = build_camps_and_bosses();
        schedule_boss_spawns(&mut neutrals, now);
        let spawn_at = now + BOTTOM_BOSS_SPAWN_DELAY + Duration::from_millis(1);
        simulate_neutrals(&mut players, &mut neutrals, &GameState::Running, 0.1, spawn_at);
        let wendigo_id = find_boss_id(&neutrals, NeutralCampType::WendigoBoss);
        let anchor = neutrals.get(&wendigo_id).unwrap().anchor;

        // Aggro the boss, then keep the target INSIDE the camp leash distance
        // but OUTSIDE the (larger) boss leash: the boss must keep chasing.
        {
            let boss = neutrals.get_mut(&wendigo_id).unwrap();
            boss.state.hp -= 50.0;
            boss.state.ai_state = NeutralAiState::Aggro;
            boss.target_player_id = Some(player_id);
        }
        {
            let player = players.get_mut(&addr).unwrap();
            player.state.x = anchor.x + NEUTRAL_LEASH_DISTANCE + 2.0;
            player.state.z = anchor.z;
        }
        const {
            assert!(NEUTRAL_LEASH_DISTANCE + 2.0 < BOSS_LEASH_DISTANCE);
        }
        simulate_neutrals(
            &mut players,
            &mut neutrals,
            &GameState::Running,
            0.1,
            spawn_at + Duration::from_millis(100),
        );
        {
            let boss = neutrals.get(&wendigo_id).unwrap();
            assert_eq!(boss.state.ai_state, NeutralAiState::Aggro);
            assert!(boss.state.hp < WENDIGO_MAX_HP, "no reset inside boss leash");
        }

        // Past the boss leash the boss resets to its pit at full HP.
        {
            let player = players.get_mut(&addr).unwrap();
            player.state.x = anchor.x + BOSS_LEASH_DISTANCE + 2.0;
        }
        simulate_neutrals(
            &mut players,
            &mut neutrals,
            &GameState::Running,
            0.1,
            spawn_at + Duration::from_millis(200),
        );
        let boss = neutrals.get(&wendigo_id).unwrap();
        assert_eq!(boss.state.ai_state, NeutralAiState::Idle);
        assert!((boss.state.hp - WENDIGO_MAX_HP).abs() < EPSILON);
        assert!((boss.state.x - anchor.x).abs() < EPSILON);
        assert!((boss.state.z - anchor.z).abs() < EPSILON);
        assert!(boss.target_player_id.is_none());
    }
}
