use serde::{Deserialize, Serialize};
use shared::{PlayerAbilitySnapshot, SkillSlot, TargetingMode};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::{SocketAddr, UdpSocket},
    thread,
    time::{Duration, Instant},
};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:4000";
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(50);
const PLAYER_TIMEOUT: Duration = Duration::from_secs(5);
const SIMULATION_STEP_SLEEP: Duration = Duration::from_millis(10);
const MAX_PACKET_SIZE: usize = 8 * 1024;

const MAX_HP: f32 = 100.0;
const MAX_MANA: f32 = 100.0;
const MANA_REGEN_PER_SECOND: f32 = 8.0;
const PROJECTILE_SPEED: f32 = 19.0;
const PROJECTILE_RADIUS: f32 = 0.22;

const MELEE_STRIKE_MANA: f32 = 12.0;
const MELEE_STRIKE_COOLDOWN: Duration = Duration::from_millis(450);
const MELEE_STRIKE_DAMAGE: f32 = 14.0;
const MELEE_STRIKE_RANGE: f32 = 3.6;

const RANGED_SHOT_BASE_DAMAGE: f32 = 18.0;
const RANGED_SHOT_DAMAGE_PER_RANK: f32 = 5.0;
const RANGED_SHOT_BASE_RANGE: f32 = 26.0;
const RANGED_SHOT_RANGE_PER_RANK: f32 = 2.0;
const RANGED_SHOT_BASE_SPEED: f32 = 17.0;
const RANGED_SHOT_SPEED_PER_RANK: f32 = 1.2;
const RANGED_SHOT_MANA: f32 = 20.0;
const RANGED_SHOT_BASE_COOLDOWN_MS: u64 = 380;
const RANGED_SHOT_COOLDOWN_REDUCTION_PER_RANK_MS: u64 = 22;
const RANGED_SHOT_MAX_RANK: u8 = 5;
const PROJECTILE_LIFETIME: Duration = Duration::from_secs(3);
const PLAYER_HIT_RADIUS: f32 = 0.62;
const AIM_HEIGHT: f32 = 0.55;
const RESPAWN_DELAY: Duration = Duration::from_secs(5);

const TOWER_MAX_HP: f32 = 240.0;
const BASE_TOWER_MAX_HP: f32 = 650.0;
const TOWER_SIZE: f32 = 2.6;
const BASE_TOWER_SIZE: f32 = 6.0;
const TOWER_RANGE: f32 = 20.0;
const TOWER_DAMAGE: f32 = 14.0;
const TOWER_COOLDOWN: Duration = Duration::from_millis(900);
const TOWER_SHOT_HEIGHT: f32 = 2.4;
const BASE_TOWER_RANGE: f32 = 24.0;
const BASE_TOWER_DAMAGE: f32 = 18.0;
const BASE_TOWER_COOLDOWN: Duration = Duration::from_millis(850);
const BASE_TOWER_SHOT_HEIGHT: f32 = 3.2;

const MINION_MAX_HP: f32 = 65.0;
const MINION_SPEED: f32 = 3.1;
const MINION_ATTACK_RANGE: f32 = 2.4;
const MINION_ATTACK_DAMAGE: f32 = 8.0;
const MINION_ATTACK_COOLDOWN: Duration = Duration::from_millis(950);
const MINION_VISION_RANGE: f32 = 10.0;
const MINION_RADIUS: f32 = 0.55;
const MINION_SPAWN_HEIGHT: f32 = 0.5;
const MINION_WAVE_INTERVAL: Duration = Duration::from_secs(60);
const MINIONS_PER_WAVE: usize = 3;
const MINION_KILL_GOLD: u32 = 18;
const MINION_KILL_XP: u32 = 32;
const PLAYER_SPAWN_OFFSET: f32 = 7.0;
const STARTING_LEVEL: u32 = 1;
const MAX_LEVEL: u32 = 10;
const LEVEL_UP_HP_BONUS: f32 = 18.0;
const LEVEL_UP_MANA_BONUS: f32 = 12.0;
const LEVEL_XP_THRESHOLDS: [u32; 9] = [120, 150, 180, 220, 260, 300, 340, 380, 420];

const NEUTRAL_RADIUS: f32 = 0.62;
const NEUTRAL_SPAWN_HEIGHT: f32 = 0.5;
const NEUTRAL_AGGRO_RADIUS: f32 = 7.5;
const NEUTRAL_LEASH_DISTANCE: f32 = 13.0;
const NEUTRAL_ATTACK_COOLDOWN: Duration = Duration::from_millis(850);
const NEUTRAL_CHASE_SPEED: f32 = 2.9;
const NEUTRAL_RESPAWN_COOLDOWN: Duration = Duration::from_secs(40);
const VICTORY_REMATCH_DELAY: Duration = Duration::from_secs(10);

const TARGET_BASE_RUN_TIME_SECONDS: f32 = 45.0;
const PLAYER_SPEED: f32 = 5.0;
const TARGET_BASE_DISTANCE: f32 = PLAYER_SPEED * TARGET_BASE_RUN_TIME_SECONDS;
const BASE_PAD_SIZE: f32 = 46.0;
const BASE_EDGE_MARGIN: f32 = 6.0;
const LANE_WIDTH: f32 = 12.0;
const LANE_EDGE_PADDING: f32 = 6.0;

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
        #[serde(default)]
        slot: u8,
        target: TargetId,
    },
    UpgradeSkill {
        slot: u8,
    },
    Join {
        team: Team,
        #[serde(default = "default_character_choice")]
        character: CharacterChoice,
    },
    Ping,
    RequestRematch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Team {
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CharacterChoice {
    Ipfs,
    Toka,
    Wang,
    Cube,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum HeroAbility {
    MeleeStrike,
    RangedShot,
}

fn default_character_choice() -> CharacterChoice {
    CharacterChoice::Ipfs
}

fn sync_connected_player_abilities(player: &mut ConnectedPlayer, now: Instant) {
    let state = &mut player.state;
    let level = state.level;
    state.abilities.unlocked = shared::unlocked_slots_for_level(level);
    for i in 0..4 {
        let def = &shared::ABILITIES[i];
        let rank = state.abilities.ranks[i].clamp(1, def.max_rank);
        state.abilities.ranks[i] = rank;
        state.abilities.cooldown_remaining[i] =
            if let Some(last) = player.last_ability_cast_at[i] {
                let cd = shared::scaled_cooldown(def, rank);
                let elapsed = now.duration_since(last);
                if elapsed >= cd {
                    0.0
                } else {
                    (cd - elapsed).as_secs_f32()
                }
            } else {
                0.0
            };
        state.abilities.rank_upgrade_available[i] = state.skill_points > 0
            && state.abilities.unlocked[i]
            && rank < def.max_rank;
    }
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
    skill_ranks: [u8; skills::SLOT_COUNT],
    #[serde(default = "default_character_choice")]
    character: CharacterChoice,
    #[serde(default)]
    abilities: PlayerAbilitySnapshot,
}

fn default_melee_skill_rank() -> u32 {
    1
}

fn melee_skill_rank_clamped(rank: u32) -> u32 {
    rank.clamp(1, MAX_MELEE_SKILL_RANK)
}

fn melee_damage_for_rank(rank: u32) -> f32 {
    let r = melee_skill_rank_clamped(rank);
    MELEE_BASE_DAMAGE + MELEE_DAMAGE_PER_RANK * (r - 1) as f32
}

fn melee_cooldown_for_rank(rank: u32) -> Duration {
    let r = melee_skill_rank_clamped(rank);
    let steps = r - 1;
    let mut cd = MELEE_COOLDOWN_BASE;
    for _ in 0..steps {
        cd = cd.saturating_sub(MELEE_COOLDOWN_PER_RANK_REDUCTION);
        if cd <= MELEE_COOLDOWN_MIN {
            return MELEE_COOLDOWN_MIN;
        }
    }
    cd.max(MELEE_COOLDOWN_MIN)
}

fn horizontal_distance_squared_xz(a: Vec3f, b: Vec3f) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx * dx + dz * dz
}

fn default_ranged_shot_rank() -> u8 {
    1
}

fn ranged_shot_rank_clamped(rank: u8) -> u8 {
    rank.clamp(1, RANGED_SHOT_MAX_RANK)
}

fn ranged_shot_damage_for_rank(rank: u8) -> f32 {
    let r = (ranged_shot_rank_clamped(rank) - 1) as f32;
    RANGED_SHOT_BASE_DAMAGE + r * RANGED_SHOT_DAMAGE_PER_RANK
}

fn ranged_shot_range_for_rank(rank: u8) -> f32 {
    let r = (ranged_shot_rank_clamped(rank) - 1) as f32;
    RANGED_SHOT_BASE_RANGE + r * RANGED_SHOT_RANGE_PER_RANK
}

fn ranged_shot_speed_for_rank(rank: u8) -> f32 {
    let r = (ranged_shot_rank_clamped(rank) - 1) as f32;
    RANGED_SHOT_BASE_SPEED + r * RANGED_SHOT_SPEED_PER_RANK
}

fn ranged_shot_cooldown_for_rank(rank: u8) -> Duration {
    let rank = ranged_shot_rank_clamped(rank) as u64;
    let reduce = (rank - 1) * RANGED_SHOT_COOLDOWN_REDUCTION_PER_RANK_MS;
    let ms = RANGED_SHOT_BASE_COOLDOWN_MS.saturating_sub(reduce);
    Duration::from_millis(ms.max(120))
}

fn default_skill_ranks() -> [u8; skills::SLOT_COUNT] {
    [skills::STARTING_RANK; skills::SLOT_COUNT]
}

fn try_upgrade_skill(state: &mut PlayerState, slot: u8) -> bool {
    let slot_usize = slot as usize;
    if slot_usize >= skills::SLOT_COUNT {
        return false;
    }
    if !skills::can_upgrade_slot(&state.skill_ranks, slot_usize, state.skill_points) {
        return false;
    }
    state.skill_points -= 1;
    state.skill_ranks[slot_usize] = state.skill_ranks[slot_usize].saturating_add(1);
    true
}

fn xp_threshold_for_level(level: u32) -> u32 {
    if level >= MAX_LEVEL {
        0
    } else {
        let index = level.saturating_sub(STARTING_LEVEL) as usize;
        LEVEL_XP_THRESHOLDS[index]
    }
}

fn apply_level_up(state: &mut PlayerState) {
    state.level = state.level.saturating_add(1);
    state.skill_points = state.skill_points.saturating_add(1);
    state.max_hp += LEVEL_UP_HP_BONUS;
    state.max_mana += LEVEL_UP_MANA_BONUS;
    state.hp = (state.hp + LEVEL_UP_HP_BONUS).clamp(0.0, state.max_hp);
    state.mana = (state.mana + LEVEL_UP_MANA_BONUS).clamp(0.0, state.max_mana);
    state.next_level_xp = xp_threshold_for_level(state.level);
}

fn grant_player_xp(state: &mut PlayerState, amount: u32) {
    if amount == 0 {
        return;
    }
    if state.level >= MAX_LEVEL {
        state.xp = 0;
        state.next_level_xp = 0;
        return;
    }

    state.xp = state.xp.saturating_add(amount);
    while state.level < MAX_LEVEL && state.next_level_xp > 0 && state.xp >= state.next_level_xp {
        state.xp -= state.next_level_xp;
        apply_level_up(state);
    }

    if state.level >= MAX_LEVEL {
        state.xp = 0;
        state.next_level_xp = 0;
    }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum ProjectileVisual {
    #[default]
    TowerBolt,
    RangedShot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectileState {
    id: u64,
    owner_id: u64,
    owner_team: Team,
    x: f32,
    y: f32,
    z: f32,
    #[serde(default)]
    visual: ProjectileVisual,
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
        game_state: GameState,
        #[serde(default)]
        rematch_in_secs: Option<u64>,
        #[serde(default)]
        ability_feedback: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GameState {
    #[default]
    Lobby,
    Running,
    Victory { winner: Team },
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
    last_seen: Instant,
    last_melee_at: Option<Instant>,
    last_ranged_shot_at: Option<Instant>,
    respawn_at: Option<Instant>,
    pending_ability_feedback: Option<String>,
}

struct Projectile {
    state: ProjectileState,
    target: TargetId,
    velocity: Vec3f,
    move_speed: f32,
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

fn neutral_template(camp_type: NeutralCampType) -> NeutralTemplate {
    match camp_type {
        NeutralCampType::Skirmisher => NeutralTemplate {
            max_hp: 72.0,
            attack_damage: 7.0,
            attack_range: 2.45,
            kill_gold: 28,
            kill_xp: 50,
        },
        NeutralCampType::Bruiser => NeutralTemplate {
            max_hp: 130.0,
            attack_damage: 11.0,
            attack_range: 2.65,
            kill_gold: 52,
            kill_xp: 85,
        },
        NeutralCampType::Spitter => NeutralTemplate {
            max_hp: 58.0,
            attack_damage: 9.0,
            attack_range: 7.6,
            kill_gold: 35,
            kill_xp: 55,
        },
    }
}

fn jungle_camp_blueprints() -> Vec<(Vec3f, NeutralCampType)> {
    let inner_side = TARGET_BASE_DISTANCE / 2.0_f32.sqrt();
    let half_inner_side = inner_side * 0.5;
    let base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
    let half_map_size = half_inner_side + base_padding;
    let map_size = half_map_size * 2.0;
    let jungle_outer = map_size * 0.34;
    let jungle_inner = map_size * 0.22;
    let y = NEUTRAL_SPAWN_HEIGHT;
    vec![
        (
            Vec3f::new(-jungle_outer, y, jungle_inner),
            NeutralCampType::Skirmisher,
        ),
        (
            Vec3f::new(jungle_outer, y, -jungle_inner),
            NeutralCampType::Bruiser,
        ),
        (
            Vec3f::new(-jungle_inner, y, -jungle_outer),
            NeutralCampType::Spitter,
        ),
    ]
}

fn build_neutral_camps(next_id: &mut u64) -> HashMap<u64, Neutral> {
    let mut out = HashMap::new();
    for (anchor, camp_type) in jungle_camp_blueprints() {
        let template = neutral_template(camp_type);
        let id = *next_id;
        *next_id += 1;
        out.insert(
            id,
            Neutral {
                state: NeutralState {
                    id,
                    camp_type,
                    x: anchor.x,
                    y: anchor.y,
                    z: anchor.z,
                    yaw: 0.0,
                    hp: template.max_hp,
                    max_hp: template.max_hp,
                    ai_state: NeutralAiState::Idle,
                },
                anchor,
                target_player_id: None,
                last_attack_at: None,
                dead_until: None,
            },
        );
    }
    out
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

fn main() -> io::Result<()> {
    let bind_addr = std::env::var("SERVER_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    let socket = UdpSocket::bind(&bind_addr)?;
    socket.set_nonblocking(true)?;

    println!("UDP game server is listening on {bind_addr}");

    let mut players = HashMap::<SocketAddr, ConnectedPlayer>::new();
    let mut projectiles = HashMap::<u64, Projectile>::new();
    let map_layout = build_map_layout();
    let mut structures = build_structures(&map_layout);
    let mut minions = HashMap::<u64, Minion>::new();
    let mut game_state = GameState::Lobby;
    let mut victory_at: Option<Instant> = None;
    let mut next_player_id: u64 = 1;
    let mut next_projectile_id: u64 = 1;
    let mut next_minion_id: u64 = 1;
    let mut next_neutral_id: u64 = 9001;
    let mut neutrals = build_neutral_camps(&mut next_neutral_id);
    let mut recv_buf = [0_u8; MAX_PACKET_SIZE];
    let mut last_snapshot_at = Instant::now();
    let mut last_simulation_at = Instant::now();
    let mut last_wave_spawn_at = Instant::now()
        .checked_sub(MINION_WAVE_INTERVAL)
        .unwrap_or_else(Instant::now);

    loop {
        loop {
            match socket.recv_from(&mut recv_buf) {
                Ok((len, addr)) => {
                    let packet = match serde_json::from_slice::<ClientPacket>(&recv_buf[..len]) {
                        Ok(packet) => packet,
                        Err(error) => {
                            eprintln!("Invalid packet from {addr}: {error}");
                            continue;
                        }
                    };

                    let now = Instant::now();
                    ensure_player_connected(
                        &mut players,
                        &map_layout,
                        addr,
                        &mut next_player_id,
                        now,
                    );
                    if let Some(player) = players.get_mut(&addr) {
                        player.last_seen = now;
                    }

                    match packet {
                        ClientPacket::Transform { x, y, z, yaw } => {
                            if matches!(game_state, GameState::Running)
                                && let Some(player) = players.get_mut(&addr)
                                && player.state.hp > 0.0
                            {
                                player.state.x = x;
                                player.state.y = y;
                                player.state.z = z;
                                player.state.yaw = yaw;
                            }
                        }
                        ClientPacket::Cast { slot, target } => {
                            handle_cast_request(
                                &mut players,
                                &mut minions,
                                &mut neutrals,
                                &mut game_state,
                                addr,
                                slot,
                                target,
                                &mut next_projectile_id,
                                now,
                            );
                        }
                        ClientPacket::UpgradeSkill { slot } => {
                            if matches!(game_state, GameState::Running)
                                && let Some(player) = players.get_mut(&addr)
                            {
                                let _applied = try_upgrade_skill(&mut player.state, slot);
                            }
                        }
                        ClientPacket::Join { team, character } => {
                            if let Some(player) = players.get_mut(&addr) {
                                handle_join_request(player, team, character, &map_layout);
                            }
                            if matches!(game_state, GameState::Lobby) {
                                println!("First player joined – match starting");
                                game_state = GameState::Running;
                            }
                        }
                        ClientPacket::Ping => {}
                        ClientPacket::RequestRematch => {
                            if matches!(game_state, GameState::Victory { .. }) {
                                reset_match(
                                    &mut players,
                                    &mut structures,
                                    &mut minions,
                                    &mut projectiles,
                                    &map_layout,
                                    &mut last_wave_spawn_at,
                                    &mut game_state,
                                );
                                victory_at = None;
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

        let now = Instant::now();
        let dt = now
            .duration_since(last_simulation_at)
            .as_secs_f32()
            .clamp(0.0, 0.1);
        last_simulation_at = now;

        regenerate_mana(&mut players, dt);
        apply_vitality_regen(&mut players, dt);
        spawn_minion_waves_if_due(
            &map_layout,
            &mut minions,
            &mut next_minion_id,
            &game_state,
            now,
            &mut last_wave_spawn_at,
        );
        simulate_minions(
            &mut players,
            &mut minions,
            &mut structures,
            &mut game_state,
            dt,
            now,
        );
        simulate_tower_attacks(
            &mut players,
            &mut minions,
            &mut projectiles,
            &mut structures,
            &mut next_projectile_id,
            &game_state,
            now,
        );
        simulate_projectiles(
            &mut players,
            &mut minions,
            &mut structures,
            &mut neutrals,
            &mut projectiles,
            &mut game_state,
            dt,
            now,
        );
        simulate_neutrals(&mut players, &mut neutrals, &game_state, dt, now);
        handle_respawns(&mut players, &structures, &map_layout, &game_state, now);

        // Rematch countdown
        if let GameState::Victory { .. } = game_state {
            if victory_at.is_none() {
                victory_at = Some(now);
            }
            if victory_at.is_some_and(|t| now.duration_since(t) >= VICTORY_REMATCH_DELAY) {
                reset_match(
                    &mut players,
                    &mut structures,
                    &mut minions,
                    &mut projectiles,
                    &map_layout,
                    &mut last_wave_spawn_at,
                    &mut game_state,
                );
                victory_at = None;
            }
        } else {
            victory_at = None;
        }

        players.retain(|addr, player| {
            let is_alive = now.duration_since(player.last_seen) <= PLAYER_TIMEOUT;
            if !is_alive {
                println!("Player {} timed out ({addr})", player.state.id);
            }
            is_alive
        });

        let live_player_ids = players
            .values()
            .map(|player| player.state.id)
            .collect::<HashSet<_>>();
        projectiles.retain(|_, projectile| {
            match projectile.target.kind {
                TargetKind::Player => live_player_ids.contains(&projectile.target.id),
                TargetKind::Minion => minions
                    .get(&projectile.target.id)
                    .is_some_and(|minion| minion.state.hp > 0.0),
                TargetKind::Structure => structures
                    .get(&projectile.target.id)
                    .is_some_and(|structure| structure.state.hp > 0.0),
                TargetKind::Neutral => neutrals.get(&projectile.target.id).is_some_and(|neutral| {
                    neutral.dead_until.is_none() && neutral.state.hp > 0.0
                }),
            }
        });

        minions.retain(|_, minion| minion.state.hp > 0.0);

        if now.duration_since(last_snapshot_at) >= SNAPSHOT_INTERVAL {
            let mut players_snapshot = players
                .values()
                .map(|player| player.state.clone())
                .collect::<Vec<_>>();
            players_snapshot.sort_unstable_by_key(|player| player.id);

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

            let rematch_in_secs = if let GameState::Victory { .. } = game_state {
                victory_at.map(|t| {
                    VICTORY_REMATCH_DELAY
                        .saturating_sub(now.duration_since(t))
                        .as_secs()
                })
            } else {
                None
            };

            for (addr, connected) in players.iter_mut() {
                let ability_feedback = connected.pending_ability_feedback.take();
                let packet = ServerPacket::Snapshot {
                    your_id: connected.state.id,
                    players: players_snapshot.clone(),
                    projectiles: projectiles_snapshot.clone(),
                    structures: structures_snapshot.clone(),
                    minions: minions_snapshot.clone(),
                    neutrals: neutrals_snapshot.clone(),
                    game_state: game_state.clone(),
                    rematch_in_secs,
                    ability_feedback,
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

            last_snapshot_at = now;
        }

        thread::sleep(SIMULATION_STEP_SLEEP);
    }
}

fn ensure_player_connected(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    map_layout: &MapLayoutState,
    addr: SocketAddr,
    next_player_id: &mut u64,
    now: Instant,
) {
    players.entry(addr).or_insert_with(|| {
        let player_id = *next_player_id;
        *next_player_id += 1;
        println!("Player {player_id} connected from {addr}");
        let spawn = spawn_position_for_team(map_layout, Team::Green);

        ConnectedPlayer {
            state: PlayerState {
                id: player_id,
                x: spawn.x,
                y: 0.5,
                z: spawn.z,
                yaw: 0.0,
                team: Team::Green,
                hp: MAX_HP,
                max_hp: MAX_HP,
                mana: MAX_MANA,
                max_mana: MAX_MANA,
                gold: 0,
                xp: 0,
                level: STARTING_LEVEL,
                next_level_xp: xp_threshold_for_level(STARTING_LEVEL),
                skill_points: 0,
                skill_ranks: default_skill_ranks(),
                character: default_character_choice(),
                abilities: PlayerAbilitySnapshot::fresh_for_level(STARTING_LEVEL),
            },
            last_seen: now,
            last_melee_at: None,
            last_ranged_shot_at: None,
            respawn_at: None,
            pending_ability_feedback: None,
        }
    });
}

fn regenerate_mana(players: &mut HashMap<SocketAddr, ConnectedPlayer>, dt: f32) {
    for player in players.values_mut() {
        if player.state.hp <= 0.0 {
            continue;
        }
        if player.state.max_mana <= 0.0 {
            player.state.max_mana = MAX_MANA;
        }
        let focus_bonus = skills::focus_mana_regen_bonus(player.state.skill_ranks[2]);
        player.state.mana = (player.state.mana
            + (MANA_REGEN_PER_SECOND + focus_bonus) * dt)
            .clamp(0.0, player.state.max_mana);
    }
}

fn apply_vitality_regen(players: &mut HashMap<SocketAddr, ConnectedPlayer>, dt: f32) {
    for player in players.values_mut() {
        if player.state.hp <= 0.0 || player.state.hp >= player.state.max_hp {
            continue;
        }
        let regen = skills::vitality_hp_per_second(player.state.skill_ranks[1]);
        player.state.hp = (player.state.hp + regen * dt).min(player.state.max_hp);
    }
}

fn handle_join_request(
    player: &mut ConnectedPlayer,
    team: Team,
    character: CharacterChoice,
    map_layout: &MapLayoutState,
) {
    println!(
        "Player {} joined team {:?} as {:?}",
        player.state.id, team, character
    );
    player.state.team = team;
    player.state.character = character;
    let spawn = spawn_position_for_team(map_layout, team);
    player.state.x = spawn.x;
    player.state.y = 0.5;
    player.state.z = spawn.z;
    player.state.yaw = 0.0;
    player.state.hp = MAX_HP;
    player.state.max_hp = MAX_HP;
    player.state.mana = MAX_MANA;
    player.state.max_mana = MAX_MANA;
    player.state.gold = 0;
    player.state.xp = 0;
    player.state.level = STARTING_LEVEL;
    player.state.next_level_xp = xp_threshold_for_level(STARTING_LEVEL);
    player.state.skill_points = 0;
    player.state.skill_ranks = default_skill_ranks();
    player.last_cast_at = None;
    player.respawn_at = None;
}

fn queue_ability_feedback(players: &mut HashMap<SocketAddr, ConnectedPlayer>, addr: SocketAddr, msg: String) {
    if let Some(player) = players.get_mut(&addr) {
        player.pending_ability_feedback = Some(msg);
    }
}

fn resolve_hero_ability_target_world(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    minions: &HashMap<u64, Minion>,
    structures: &HashMap<u64, Structure>,
    neutrals: &HashMap<u64, Neutral>,
    caster_team: Team,
    target: TargetId,
) -> Option<Vec3f> {
    match target.kind {
        TargetKind::Player => {
            let target_player = players.values().find(|player| {
                player.state.id == target.id
                    && player.state.hp > 0.0
                    && player.state.team != caster_team
            })?;
            Some(Vec3f::new(
                target_player.state.x,
                target_player.state.y + AIM_HEIGHT,
                target_player.state.z,
            ))
        }
        TargetKind::Minion => {
            let target_minion = minions.get(&target.id)?;
            if target_minion.state.hp <= 0.0 || target_minion.state.team == caster_team {
                return None;
            }
            Some(Vec3f::new(
                target_minion.state.x,
                target_minion.state.y + MINION_RADIUS * 0.8,
                target_minion.state.z,
            ))
        }
        TargetKind::Structure => {
            let target_structure = structures.get(&target.id)?;
            if target_structure.state.hp <= 0.0 || target_structure.state.team == caster_team {
                return None;
            }
            Some(Vec3f::new(
                target_structure.state.x,
                target_structure.state.y,
                target_structure.state.z,
            ))
        }
        TargetKind::Neutral => {
            let target_neutral = neutrals.get(&target.id)?;
            if target_neutral.dead_until.is_some() || target_neutral.state.hp <= 0.0 {
                return None;
            }
            Some(Vec3f::new(
                target_neutral.state.x,
                target_neutral.state.y + NEUTRAL_RADIUS * 0.85,
                target_neutral.state.z,
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_hero_direct_damage(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
    neutrals: &mut HashMap<u64, Neutral>,
    game_state: &mut GameState,
    caster_player_id: u64,
    caster_team: Team,
    target: TargetId,
    damage: f32,
    now: Instant,
) {
    match target.kind {
        TargetKind::Player => {
            if let Some(target_player) = players.values_mut().find(|player| {
                player.state.id == target.id && player.state.hp > 0.0 && player.state.team != caster_team
            }) {
                target_player.state.hp = (target_player.state.hp - damage).max(0.0);
                if target_player.state.hp <= 0.0 && target_player.respawn_at.is_none() {
                    target_player.respawn_at = Some(now + RESPAWN_DELAY);
                }
            }
        }
        TargetKind::Minion => {
            apply_minion_damage(players, minions, target.id, damage, caster_team);
        }
        TargetKind::Structure => {
            if let Some(target_structure) = structures.get_mut(&target.id) {
                if target_structure.state.hp <= 0.0 {
                    return;
                }
                target_structure.state.hp = (target_structure.state.hp - damage).max(0.0);
                if target_structure.state.hp <= 0.0
                    && target_structure.state.kind == StructureKind::BaseTower
                {
                    *game_state = GameState::Victory {
                        winner: caster_team,
                    };
                }
            }
        }
        TargetKind::Neutral => {
            apply_neutral_damage(players, neutrals, target.id, damage, caster_player_id, now);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_use_ability_request(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    projectiles: &mut HashMap<u64, Projectile>,
    minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
    neutrals: &mut HashMap<u64, Neutral>,
    game_state: &mut GameState,
    caster_addr: SocketAddr,
    slot: u8,
    target: TargetId,
    next_projectile_id: &mut u64,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    if slot != 0 {
        return;
    }
    let Some(caster) = players.get(&caster_addr) else {
        return;
    };
    if caster.state.hp <= 0.0 {
        queue_ability_feedback(players, caster_addr, "Cannot use abilities while dead.".to_string());
        return;
    }
    let mana_cost = skills::slot0_mana_cost(caster.state.skill_ranks[0]);
    let cooldown = skills::slot0_cooldown(caster.state.skill_ranks[0]);
    if caster.state.mana < mana_cost {
        return;
    }
    if caster
        .last_cast_at
        .is_some_and(|last_cast| now.duration_since(last_cast) < cooldown)
    {
        queue_ability_feedback(
            players,
            caster_addr,
            "Melee Strike is on cooldown.".to_string(),
        );
        return;
    }

    let caster_team = caster.state.team;
    let caster_id = caster.state.id;
    let caster_position = Vec3f::new(caster.state.x, caster.state.y + AIM_HEIGHT, caster.state.z);

    let Some(target_position) = resolve_hero_ability_target_world(
        players,
        minions,
        structures,
        neutrals,
        caster_team,
        target,
    ) else {
        queue_ability_feedback(
            players,
            caster_addr,
            "Melee Strike: invalid or dead target.".to_string(),
        );
        return;
    };

    let dist = caster_position.distance(target_position);
    if dist > MELEE_STRIKE_RANGE {
        queue_ability_feedback(
            players,
            caster_addr,
            "Target is out of range for Melee Strike.".to_string(),
        );
        return;
    }

    let Some(caster_mut) = players.get_mut(&caster_addr) else {
        return;
    };
    caster_mut.state.mana -= MELEE_STRIKE_MANA;
    caster_mut.last_melee_at = Some(now);

    apply_hero_direct_damage(
        players,
        minions,
        structures,
        neutrals,
        game_state,
        caster_id,
        caster_team,
        target,
        MELEE_STRIKE_DAMAGE,
        now,
    );
}

#[allow(clippy::too_many_arguments)]
fn try_ranged_shot_ability(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    neutrals: &mut HashMap<u64, Neutral>,
    caster_addr: SocketAddr,
    target: TargetId,
    next_projectile_id: &mut u64,
    now: Instant,
) {
    let Some(caster) = players.get(&caster_addr) else {
        return;
    };
    if caster.state.hp <= 0.0 {
        queue_ability_feedback(players, caster_addr, "Cannot use abilities while dead.".to_string());
        return;
    }

    let rank = caster.state.ranged_shot_rank;
    let mana_cost = RANGED_SHOT_MANA;
    let cooldown = ranged_shot_cooldown_for_rank(rank);
    let max_range = ranged_shot_range_for_rank(rank);

    if caster.state.mana < mana_cost {
        queue_ability_feedback(
            players,
            caster_addr,
            "Not enough mana for Ranged Shot.".to_string(),
        );
        return;
    }
    if caster
        .last_ranged_shot_at
        .is_some_and(|t| now.duration_since(t) < cooldown)
    {
        queue_ability_feedback(
            players,
            caster_addr,
            "Ranged Shot is on cooldown.".to_string(),
        );
        return;
    }

    let caster_team = caster.state.team;
    let Some(target_position) = resolve_hero_ability_target_world(
        players,
        minions,
        structures,
        neutrals,
        caster_team,
        target,
    ) else {
        queue_ability_feedback(
            players,
            caster_addr,
            "Ranged Shot: invalid or dead target.".to_string(),
        );
        return;
    };

    let caster_position = Vec3f::new(
        caster.state.x,
        caster.state.y + CAST_SPAWN_HEIGHT,
        caster.state.z,
    );
    let dist = caster_position.distance(target_position);
    if dist > max_range {
        queue_ability_feedback(
            players,
            caster_addr,
            "Target is out of range for Ranged Shot.".to_string(),
        );
        return;
    }

    let direction = Vec3f::new(
        target_position.x - caster_position.x,
        target_position.y - caster_position.y,
        target_position.z - caster_position.z,
    )
    .normalize_or_zero();

    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
        queue_ability_feedback(
            players,
            caster_addr,
            "Cannot resolve Ranged Shot direction.".to_string(),
        );
        return;
    }

    let move_speed = ranged_shot_speed_for_rank(rank);
    let damage = ranged_shot_damage_for_rank(rank);

    let Some(caster_mut) = players.get_mut(&caster_addr) else {
        return;
    };
    let projectile_speed = skills::effective_projectile_speed(
        caster_mut.state.skill_ranks[0],
        caster_mut.state.skill_ranks[3],
    );
    let projectile_damage = skills::slot0_damage(caster_mut.state.skill_ranks[0]);
    caster_mut.state.mana -= mana_cost;
    caster_mut.last_cast_at = Some(now);

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
                visual: ProjectileVisual::RangedShot,
            },
            target,
            velocity: Vec3f::new(
                direction.x * projectile_speed,
                direction.y * projectile_speed,
                direction.z * projectile_speed,
            ),
            move_speed,
            homing: true,
            guaranteed_hit: true,
            damage: projectile_damage,
            radius: PROJECTILE_RADIUS,
            expires_at: now + PROJECTILE_LIFETIME,
        },
    );
}

fn handle_upgrade_ability_request(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    addr: SocketAddr,
    ability: HeroAbility,
    _now: Instant,
) {
    let Some(player) = players.get_mut(&addr) else {
        return;
    };
    match ability {
        HeroAbility::RangedShot => {
            if player.state.skill_points == 0 {
                queue_ability_feedback(
                    players,
                    addr,
                    "No skill points available to upgrade Ranged Shot.".to_string(),
                );
                return;
            }
            if player.state.ranged_shot_rank >= RANGED_SHOT_MAX_RANK {
                queue_ability_feedback(
                    players,
                    addr,
                    "Ranged Shot is already at max rank.".to_string(),
                );
                return;
            }
            player.state.skill_points = player.state.skill_points.saturating_sub(1);
            player.state.ranged_shot_rank = (player.state.ranged_shot_rank + 1).min(RANGED_SHOT_MAX_RANK);
        }
        HeroAbility::MeleeStrike => {
            queue_ability_feedback(
                players,
                addr,
                "Melee Strike has no rank upgrades yet.".to_string(),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn simulate_projectiles(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
    neutrals: &mut HashMap<u64, Neutral>,
    projectiles: &mut HashMap<u64, Projectile>,
    game_state: &mut GameState,
    dt: f32,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    let mut player_damage_events: Vec<(u64, f32)> = Vec::new();
    let mut minion_damage_events: Vec<(u64, f32, Team)> = Vec::new();
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
                        direction.x * projectile.move_speed,
                        direction.y * projectile.move_speed,
                        direction.z * projectile.move_speed,
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
                let Some(target_minion) = minions.get(&projectile.target.id) else {
                    return false;
                };
                if target_minion.state.hp <= 0.0 {
                    return false;
                }

                let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
                let target_pos = Vec3f::new(
                    target_minion.state.x,
                    target_minion.state.y + MINION_RADIUS * 0.8,
                    target_minion.state.z,
                );
                if projectile.homing {
                    let direction = Vec3f::new(
                        target_pos.x - start.x,
                        target_pos.y - start.y,
                        target_pos.z - start.z,
                    )
                    .normalize_or_zero();
                    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                        minion_damage_events.push((
                            projectile.target.id,
                            projectile.damage,
                            projectile.state.owner_team,
                        ));
                        return false;
                    }
                    projectile.velocity = Vec3f::new(
                        direction.x * projectile.move_speed,
                        direction.y * projectile.move_speed,
                        direction.z * projectile.move_speed,
                    );
                }
                let end = start.add_scaled(projectile.velocity, dt);
                projectile.state.x = end.x;
                projectile.state.y = end.y;
                projectile.state.z = end.z;

                let combined_radius = projectile.radius + MINION_RADIUS;
                if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
                    minion_damage_events.push((
                        projectile.target.id,
                        projectile.damage,
                        projectile.state.owner_team,
                    ));
                    return false;
                }
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
                        direction.x * projectile.move_speed,
                        direction.y * projectile.move_speed,
                        direction.z * projectile.move_speed,
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

                let start =
                    Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
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
                        direction.x * projectile.move_speed,
                        direction.y * projectile.move_speed,
                        direction.z * projectile.move_speed,
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
        apply_neutral_damage(players, neutrals, target_id, damage, attacker_id, now);
    }
}

fn apply_neutral_damage(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    neutrals: &mut HashMap<u64, Neutral>,
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
    if players.values().any(|player| {
        player.state.id == attacker_player_id && player.state.hp > 0.0
    }) {
        neutral.target_player_id = Some(attacker_player_id);
        neutral.state.ai_state = NeutralAiState::Aggro;
    }
    if neutral.state.hp <= 0.0 {
        let camp_type = neutral.state.camp_type;
        award_neutral_kill_to_player(players, attacker_player_id, camp_type);
        neutral.dead_until = Some(now + NEUTRAL_RESPAWN_COOLDOWN);
        neutral.target_player_id = None;
        neutral.last_attack_at = None;
        neutral.state.ai_state = NeutralAiState::Idle;
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
            player.state.xp = player.state.xp.saturating_add(rewards.kill_xp);
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

    let aggro_sq = NEUTRAL_AGGRO_RADIUS * NEUTRAL_AGGRO_RADIUS;
    let leash_sq = NEUTRAL_LEASH_DISTANCE * NEUTRAL_LEASH_DISTANCE;
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
        let anchor = neutral.anchor;
        let neutral_pos = Vec3f::new(neutral.state.x, neutral.state.y, neutral.state.z);

        if neutral.state.ai_state == NeutralAiState::Idle && neutral.target_player_id.is_none() {
            let best = players
                .values()
                .filter(|player| player.state.hp > 0.0)
                .map(|player| {
                    let hit = Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z);
                    (player.state.id, neutral_pos.distance_squared(hit))
                })
                .filter(|(_, dist_sq)| *dist_sq <= aggro_sq)
                .min_by(|a, b| {
                    a.1
                        .partial_cmp(&b.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
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

fn build_structures(layout: &MapLayoutState) -> HashMap<u64, Structure> {
    let mut structures = HashMap::new();
    let mut next_id: u64 = 1;

    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
        let lane_points = lane_control_points(layout, lane);
        let green_tower = sample_polyline_position(&lane_points, 0.30);
        let blue_tower = sample_polyline_position(&lane_points, 0.70);
        add_structure(
            &mut structures,
            &mut next_id,
            StructureKind::Tower,
            StructureRole::LaneTower { lane },
            Team::Green,
            Vec3f::new(green_tower.x, 3.0, green_tower.z),
        );
        add_structure(
            &mut structures,
            &mut next_id,
            StructureKind::Tower,
            StructureRole::LaneTower { lane },
            Team::Blue,
            Vec3f::new(blue_tower.x, 3.0, blue_tower.z),
        );
    }

    let home_nexus = Vec3f::new(layout.home.x, 4.0, layout.home.z);
    let away_nexus = Vec3f::new(layout.away.x, 4.0, layout.away.z);
    add_structure(
        &mut structures,
        &mut next_id,
        StructureKind::BaseTower,
        StructureRole::BaseTower,
        Team::Green,
        home_nexus,
    );
    add_structure(
        &mut structures,
        &mut next_id,
        StructureKind::BaseTower,
        StructureRole::BaseTower,
        Team::Blue,
        away_nexus,
    );

    structures
}

fn add_structure(
    structures: &mut HashMap<u64, Structure>,
    next_id: &mut u64,
    kind: StructureKind,
    role: StructureRole,
    team: Team,
    position: Vec3f,
) {
    let (max_hp, attack_range, attack_damage, attack_cooldown) = match kind {
        StructureKind::Tower => (TOWER_MAX_HP, TOWER_RANGE, TOWER_DAMAGE, TOWER_COOLDOWN),
        StructureKind::BaseTower => (
            BASE_TOWER_MAX_HP,
            BASE_TOWER_RANGE,
            BASE_TOWER_DAMAGE,
            BASE_TOWER_COOLDOWN,
        ),
    };
    let id = *next_id;
    *next_id += 1;
    structures.insert(
        id,
        Structure {
            state: StructureState {
                id,
                kind,
                team,
                x: position.x,
                y: position.y,
                z: position.z,
                hp: max_hp,
                max_hp,
            },
            role,
            last_attack_at: None,
            attack_range,
            attack_damage,
            attack_cooldown,
        },
    );
}

fn build_map_layout() -> MapLayoutState {
    let inner_side = TARGET_BASE_DISTANCE / 2.0_f32.sqrt();
    let half_inner_side = inner_side * 0.5;
    let base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
    let half_map_size = half_inner_side + base_padding;
    let home = Vec3f::new(-half_inner_side, 0.0, -half_inner_side);
    let away = Vec3f::new(half_inner_side, 0.0, half_inner_side);

    let lane_edge_offset = LANE_EDGE_PADDING + LANE_WIDTH * 0.5;
    let left_x = -half_map_size + lane_edge_offset;
    let right_x = half_map_size - lane_edge_offset;
    let top_z = half_map_size - lane_edge_offset;
    let bottom_z = -half_map_size + lane_edge_offset;

    MapLayoutState {
        home,
        away,
        left_x,
        right_x,
        top_z,
        bottom_z,
    }
}

struct MapLayoutState {
    home: Vec3f,
    away: Vec3f,
    left_x: f32,
    right_x: f32,
    top_z: f32,
    bottom_z: f32,
}

fn spawn_position_for_team(map_layout: &MapLayoutState, team: Team) -> Vec3f {
    let base = match team {
        Team::Green => map_layout.home,
        Team::Blue => map_layout.away,
    };
    let dir = Vec3f::new(-base.x, 0.0, -base.z).normalize_or_zero();
    Vec3f::new(
        base.x + dir.x * PLAYER_SPAWN_OFFSET,
        base.y,
        base.z + dir.z * PLAYER_SPAWN_OFFSET,
    )
}

fn spawn_position_for_team_from_base(
    structures: &HashMap<u64, Structure>,
    map_layout: &MapLayoutState,
    team: Team,
) -> Vec3f {
    let Some(base_tower) = structures.values().find(|structure| {
        structure.state.team == team
            && structure.state.kind == StructureKind::BaseTower
            && structure.state.hp > 0.0
    }) else {
        return spawn_position_for_team(map_layout, team);
    };

    let base = Vec3f::new(base_tower.state.x, 0.0, base_tower.state.z);
    let dir = Vec3f::new(-base.x, 0.0, -base.z).normalize_or_zero();
    Vec3f::new(
        base.x + dir.x * PLAYER_SPAWN_OFFSET,
        0.0,
        base.z + dir.z * PLAYER_SPAWN_OFFSET,
    )
}

fn lane_control_points(layout: &MapLayoutState, lane: Lane) -> Vec<Vec3f> {
    match lane {
        Lane::Mid => vec![layout.home, layout.away],
        Lane::Top => vec![
            layout.home,
            Vec3f::new(layout.left_x, 0.0, layout.home.z),
            Vec3f::new(layout.left_x, 0.0, layout.top_z),
            Vec3f::new(layout.right_x, 0.0, layout.top_z),
            Vec3f::new(layout.away.x, 0.0, layout.top_z),
            layout.away,
        ],
        Lane::Bot => vec![
            layout.home,
            Vec3f::new(layout.home.x, 0.0, layout.bottom_z),
            Vec3f::new(layout.left_x, 0.0, layout.bottom_z),
            Vec3f::new(layout.right_x, 0.0, layout.bottom_z),
            Vec3f::new(layout.right_x, 0.0, layout.away.z),
            layout.away,
        ],
    }
}

fn sample_polyline_position(points: &[Vec3f], t: f32) -> Vec3f {
    if points.len() <= 1 {
        return points.first().copied().unwrap_or(Vec3f::new(0.0, 0.0, 0.0));
    }

    let segment_lengths = points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .collect::<Vec<_>>();
    let total_length: f32 = segment_lengths.iter().sum();
    if total_length <= 0.0001 {
        return points[0];
    }

    let mut remaining = total_length * t.clamp(0.0, 1.0);
    for (index, length) in segment_lengths.into_iter().enumerate() {
        if remaining <= length {
            let local_t = if length <= 0.0001 {
                0.0
            } else {
                remaining / length
            };
            return points[index].lerp(points[index + 1], local_t);
        }
        remaining -= length;
    }

    points.last().copied().unwrap_or(points[0])
}

fn build_minion_path(layout: &MapLayoutState, lane: Lane, team: Team) -> Vec<Vec3f> {
    let mut points = lane_control_points(layout, lane);
    if team == Team::Blue {
        points.reverse();
    }
    for point in &mut points {
        point.y = MINION_SPAWN_HEIGHT;
    }
    points
}

fn spawn_minion_waves_if_due(
    map_layout: &MapLayoutState,
    minions: &mut HashMap<u64, Minion>,
    next_minion_id: &mut u64,
    game_state: &GameState,
    now: Instant,
    last_wave_spawn_at: &mut Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    if now.duration_since(*last_wave_spawn_at) < MINION_WAVE_INTERVAL {
        return;
    }
    *last_wave_spawn_at = now;

    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
        spawn_minion_wave_for_team_lane(map_layout, minions, next_minion_id, Team::Green, lane);
        spawn_minion_wave_for_team_lane(map_layout, minions, next_minion_id, Team::Blue, lane);
    }
}

fn spawn_minion_wave_for_team_lane(
    map_layout: &MapLayoutState,
    minions: &mut HashMap<u64, Minion>,
    next_minion_id: &mut u64,
    team: Team,
    lane: Lane,
) {
    let path = build_minion_path(map_layout, lane, team);
    if path.is_empty() {
        return;
    }
    let spawn = path[0];

    for wave_index in 0..MINIONS_PER_WAVE {
        let minion_id = *next_minion_id;
        *next_minion_id += 1;

        let offset = wave_index as f32 * (MINION_RADIUS * 2.0 + 0.4);
        let mut spawn_x = spawn.x;
        let mut spawn_z = spawn.z;
        let mut yaw = 0.0;
        if let Some(next_point) = path.get(1) {
            let dir_x = next_point.x - spawn.x;
            let dir_z = next_point.z - spawn.z;
            let len_sq = dir_x * dir_x + dir_z * dir_z;
            if len_sq > 0.0001 {
                let inv_len = len_sq.sqrt().recip();
                spawn_x -= dir_x * inv_len * offset;
                spawn_z -= dir_z * inv_len * offset;
                yaw = dir_x.atan2(dir_z);
            }
        }

        minions.insert(
            minion_id,
            Minion {
                state: MinionState {
                    id: minion_id,
                    team,
                    lane,
                    x: spawn_x,
                    y: MINION_SPAWN_HEIGHT,
                    z: spawn_z,
                    yaw,
                    hp: MINION_MAX_HP,
                    max_hp: MINION_MAX_HP,
                    state: MinionBrainState::Marching,
                    target_kind: None,
                    target_id: None,
                },
                path: path.clone(),
                next_waypoint: 1,
                last_attack_at: None,
                aggro_target: None,
            },
        );
    }
}

fn structure_radius(kind: StructureKind) -> f32 {
    match kind {
        StructureKind::Tower => TOWER_SIZE * 0.5,
        StructureKind::BaseTower => BASE_TOWER_SIZE * 0.5,
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
    if target_minion.state.hp <= 0.0 || target_minion.state.team == attacker_team {
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

fn award_minion_kill_rewards(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    attacker_team: Team,
) {
    let recipients = players
        .iter()
        .filter(|(_, player)| player.state.team == attacker_team)
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
        .filter(|player| player.state.hp > 0.0)
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

        let mut aggro_target = minion.aggro_target.and_then(|target| match target {
            MinionAggroTarget::Player(target_id) => player_targets
                .iter()
                .find(|(id, team, position)| {
                    *id == target_id
                        && *team != minion.state.team
                        && minion_position.distance_squared(*position) <= minion_vision_sq
                })
                .map(|(_, _, position)| (target, *position, PLAYER_HIT_RADIUS)),
            MinionAggroTarget::Minion(target_id) => minion_targets
                .iter()
                .find(|(id, team, position)| {
                    *id == target_id
                        && *id != minion.state.id
                        && *team != minion.state.team
                        && minion_position.distance_squared(*position) <= minion_vision_sq
                })
                .map(|(_, _, position)| (target, *position, MINION_RADIUS)),
        });

        if aggro_target.is_none() {
            let best_player = player_targets
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
                });

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

            aggro_target = match (best_player, best_minion) {
                (Some(player_target), Some(minion_target)) => {
                    if player_target.3 <= minion_target.3 {
                        Some((player_target.0, player_target.1, player_target.2))
                    } else {
                        Some((minion_target.0, minion_target.1, minion_target.2))
                    }
                }
                (Some(player_target), None) => {
                    Some((player_target.0, player_target.1, player_target.2))
                }
                (None, Some(minion_target)) => {
                    Some((minion_target.0, minion_target.1, minion_target.2))
                }
                (None, None) => None,
            };
        }

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
            if player.state.hp <= 0.0 || player.state.team == structure.state.team {
                continue;
            }
            let target_pos =
                Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z);
            let dist_sq = tower_position.distance_squared(target_pos);
            if dist_sq <= range_sq
                && best_target.is_none_or(|(_, _, best)| dist_sq < best)
            {
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
                    visual: ProjectileVisual::TowerBolt,
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
                move_speed: PROJECTILE_SPEED,
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
        let bonus = skills::focus_mana_regen_bonus(
            players.get(&addr).unwrap().state.skill_ranks[2],
        );
        let expected = 10.0 + (MANA_REGEN_PER_SECOND + bonus) * 2.5;
        let current = players.get(&addr).unwrap().state.mana;
        assert!((current - expected).abs() < EPSILON);

        regenerate_mana(&mut players, 100.0);
        let clamped = players.get(&addr).unwrap().state.mana;
        assert!((clamped - MAX_MANA).abs() < EPSILON);
    }

    #[test]
    fn upgrade_skill_spends_point_and_increments_rank() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34571".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let state = &mut players.get_mut(&addr).unwrap().state;
        state.skill_points = 1;
        assert!(try_upgrade_skill(state, 0));
        assert_eq!(state.skill_points, 0);
        assert_eq!(state.skill_ranks[0], skills::STARTING_RANK + 1);
    }

    #[test]
    fn upgrade_skill_rejects_without_points_or_invalid_slot() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34572".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let state = &mut players.get_mut(&addr).unwrap().state;
        state.skill_points = 0;
        let ranks_before = state.skill_ranks;
        assert!(!try_upgrade_skill(state, 0));
        assert_eq!(state.skill_ranks, ranks_before);
        assert!(!try_upgrade_skill(state, 9));
    }

    #[test]
    fn apply_level_up_grants_one_skill_point() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34573".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let state = &mut players.get_mut(&addr).unwrap().state;
        let before = state.skill_points;
        apply_level_up(state);
        assert_eq!(state.skill_points, before.saturating_add(1));
    }

    #[test]
    fn grant_xp_over_threshold_levels_and_awards_skill_point() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34574".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let state = &mut players.get_mut(&addr).unwrap().state;
        assert_eq!(state.skill_points, 0);
        let before_level = state.level;
        state.xp = state.next_level_xp.saturating_sub(1);
        grant_player_xp(state, 1);
        assert_eq!(state.level, before_level + 1);
        assert_eq!(state.skill_points, 1);
    }

    #[test]
    fn upgrade_skill_rejects_at_max_rank_even_with_points() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34575".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let state = &mut players.get_mut(&addr).unwrap().state;
        state.skill_points = 3;
        state.skill_ranks[0] = skills::MAX_SKILL_RANK;
        let ranks_before = state.skill_ranks;
        let points_before = state.skill_points;
        assert!(!try_upgrade_skill(state, 0));
        assert_eq!(state.skill_ranks, ranks_before);
        assert_eq!(state.skill_points, points_before);
    }

    #[test]
    fn upgrade_slot0_updates_authoritative_cast_mana_and_cooldown() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:34576".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let state = &mut players.get_mut(&addr).unwrap().state;
        state.skill_points = 1;
        let rank_before = state.skill_ranks[0];
        let mana_before = skills::slot0_mana_cost(rank_before);
        let cd_before = skills::slot0_cooldown(rank_before);
        assert!(try_upgrade_skill(state, 0));
        let rank_after = state.skill_ranks[0];
        assert_eq!(rank_after, rank_before + 1);
        let mana_after = skills::slot0_mana_cost(rank_after);
        let cd_after = skills::slot0_cooldown(rank_after);
        assert!(mana_after <= mana_before);
        assert!(cd_after <= cd_before);
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

        apply_neutral_damage(
            &mut players,
            &mut neutrals,
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

    fn build_snapshot_for_addr(
        players: &HashMap<SocketAddr, ConnectedPlayer>,
        structures: &HashMap<u64, Structure>,
        minions: &HashMap<u64, Minion>,
        neutrals: &HashMap<u64, Neutral>,
        game_state: &GameState,
        addr: SocketAddr,
    ) -> ServerPacket {
        let your_id = players
            .get(&addr)
            .expect("snapshot requested for connected player")
            .state
            .id;

        let mut players_snapshot = players
            .values()
            .map(|player| player.state.clone())
            .collect::<Vec<_>>();
        players_snapshot.sort_unstable_by_key(|player| player.id);

        let structures_snapshot = structures
            .values()
            .filter(|structure| structure.state.hp > 0.0)
            .map(|structure| structure.state.clone())
            .collect::<Vec<_>>();
        let minions_snapshot = minions
            .values()
            .filter(|minion| minion.state.hp > 0.0)
            .map(|minion| minion.state.clone())
            .collect::<Vec<_>>();
        let neutrals_snapshot = neutrals
            .values()
            .filter(|neutral| neutral.dead_until.is_none() && neutral.state.hp > 0.0)
            .map(|neutral| neutral.state.clone())
            .collect::<Vec<_>>();

        ServerPacket::Snapshot {
            your_id,
            players: players_snapshot,
            projectiles: Vec::new(),
            structures: structures_snapshot,
            minions: minions_snapshot,
            neutrals: neutrals_snapshot,
            game_state: game_state.clone(),
            rematch_in_secs: None,
        }
    }

    #[test]
    fn upgrade_ability_consumes_skill_point_and_increments_rank() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:49999".parse().unwrap();
        let now = Instant::now();

        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        {
            let p = players.get_mut(&addr).unwrap();
            p.state.skill_points = 2;
            p.state.level = 6;
            p.state.abilities.unlocked = shared::unlocked_slots_for_level(6);
        }

        handle_upgrade_ability_request(&mut players, addr, SkillSlot::Q, &GameState::Running);

        let p = players.get(&addr).unwrap();
        assert_eq!(p.state.skill_points, 1);
        assert_eq!(p.state.abilities.ranks[0], 2);
    }

    /// Hostile player next to the green spawn for targeted-ability checks.
    fn connect_enemy_player_adjacent(
        players: &mut HashMap<SocketAddr, ConnectedPlayer>,
        layout: &MapLayoutState,
        green_addr: SocketAddr,
        enemy_addr: SocketAddr,
        next_player_id: &mut u64,
        now: Instant,
    ) -> u64 {
        ensure_player_connected(players, layout, green_addr, next_player_id, now);
        ensure_player_connected(players, layout, enemy_addr, next_player_id, now);
        let (gx, gz) = {
            let g = players.get(&green_addr).unwrap();
            (g.state.x, g.state.z)
        };
        let enemy_id = {
            let enemy = players.get_mut(&enemy_addr).unwrap();
            enemy.state.team = Team::Blue;
            enemy.state.x = gx + 3.0;
            enemy.state.y = 0.5;
            enemy.state.z = gz;
            enemy.state.id
        };
        enemy_id
    }

    #[test]
    fn cast_rejects_insufficient_mana_for_targeted_ability() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let green_addr: SocketAddr = "127.0.0.1:50201".parse().unwrap();
        let blue_addr: SocketAddr = "127.0.0.1:50202".parse().unwrap();
        let now = Instant::now();
        let blue_id = connect_enemy_player_adjacent(
            &mut players,
            &layout,
            green_addr,
            blue_addr,
            &mut next_player_id,
            now,
        );
        players.get_mut(&green_addr).unwrap().state.mana = 5.0;

        let mut projectiles = HashMap::new();
        let mut next_pid = 1_u64;
        let mut minions = HashMap::new();
        let mut structures = build_structures(&layout);
        let mut neutrals = HashMap::new();

        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            green_addr,
            SkillSlot::Q,
            Some(TargetId {
                kind: TargetKind::Player,
                id: blue_id,
            }),
            &mut next_pid,
            &GameState::Running,
            now,
        );

        assert!(projectiles.is_empty());
        assert!((players.get(&green_addr).unwrap().state.mana - 5.0).abs() < EPSILON);
    }

    #[test]
    fn cast_rejects_unit_target_without_target() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:50203".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let mana_before = players.get(&addr).unwrap().state.mana;

        let mut projectiles = HashMap::new();
        let mut next_pid = 1_u64;
        let mut minions = HashMap::new();
        let mut structures = build_structures(&layout);
        let mut neutrals = HashMap::new();

        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            addr,
            SkillSlot::Q,
            None,
            &mut next_pid,
            &GameState::Running,
            now,
        );

        assert!(projectiles.is_empty());
        assert!((players.get(&addr).unwrap().state.mana - mana_before).abs() < EPSILON);
    }

    #[test]
    fn cast_rejects_friendly_minion_target() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:50209".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        let (px, pz, mana_before) = {
            let p = players.get(&addr).unwrap();
            (p.state.x, p.state.z, p.state.mana)
        };

        let mut projectiles = HashMap::new();
        let mut next_pid = 1_u64;
        let mut minions = HashMap::new();
        let friendly_minion_id = 9001;
        minions.insert(
            friendly_minion_id,
            Minion {
                state: MinionState {
                    id: friendly_minion_id,
                    team: Team::Green,
                    lane: Lane::Mid,
                    x: px + 1.0,
                    y: MINION_SPAWN_HEIGHT,
                    z: pz,
                    yaw: 0.0,
                    hp: MINION_MAX_HP,
                    max_hp: MINION_MAX_HP,
                    state: MinionBrainState::Marching,
                    target_kind: None,
                    target_id: None,
                },
                path: vec![Vec3f::new(px + 1.0, MINION_SPAWN_HEIGHT, pz)],
                next_waypoint: 0,
                last_attack_at: None,
                aggro_target: None,
            },
        );
        let mut structures = build_structures(&layout);
        let mut neutrals = HashMap::new();

        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            addr,
            SkillSlot::Q,
            Some(TargetId {
                kind: TargetKind::Minion,
                id: friendly_minion_id,
            }),
            &mut next_pid,
            &GameState::Running,
            now,
        );

        assert!(projectiles.is_empty());
        assert!((players.get(&addr).unwrap().state.mana - mana_before).abs() < EPSILON);
    }

    #[test]
    fn cast_rejects_second_cast_while_on_cooldown() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let green_addr: SocketAddr = "127.0.0.1:50204".parse().unwrap();
        let blue_addr: SocketAddr = "127.0.0.1:50205".parse().unwrap();
        let now = Instant::now();
        let blue_id = connect_enemy_player_adjacent(
            &mut players,
            &layout,
            green_addr,
            blue_addr,
            &mut next_player_id,
            now,
        );
        let target = Some(TargetId {
            kind: TargetKind::Player,
            id: blue_id,
        });

        let mut projectiles = HashMap::new();
        let mut next_pid = 1_u64;
        let mut minions = HashMap::new();
        let mut structures = build_structures(&layout);
        let mut neutrals = HashMap::new();

        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            green_addr,
            SkillSlot::Q,
            target,
            &mut next_pid,
            &GameState::Running,
            now,
        );
        assert_eq!(projectiles.len(), 1);

        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            green_addr,
            SkillSlot::Q,
            target,
            &mut next_pid,
            &GameState::Running,
            now,
        );
        assert_eq!(projectiles.len(), 1);
    }

    #[test]
    fn cast_self_target_rejects_spurious_network_target() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let green_addr: SocketAddr = "127.0.0.1:50207".parse().unwrap();
        let blue_addr: SocketAddr = "127.0.0.1:50208".parse().unwrap();
        let now = Instant::now();
        let blue_id = connect_enemy_player_adjacent(
            &mut players,
            &layout,
            green_addr,
            blue_addr,
            &mut next_player_id,
            now,
        );
        {
            let p = players.get_mut(&green_addr).unwrap();
            p.state.level = 2;
            p.state.abilities = PlayerAbilitySnapshot::fresh_for_level(2);
            p.state.hp = 70.0;
            p.state.mana = MAX_MANA;
        }
        let hp_before = players.get(&green_addr).unwrap().state.hp;
        let mana_before = players.get(&green_addr).unwrap().state.mana;

        let mut projectiles = HashMap::new();
        let mut next_pid = 1_u64;
        let mut minions = HashMap::new();
        let mut structures = build_structures(&layout);
        let mut neutrals = HashMap::new();

        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            green_addr,
            SkillSlot::W,
            Some(TargetId {
                kind: TargetKind::Player,
                id: blue_id,
            }),
            &mut next_pid,
            &GameState::Running,
            now,
        );

        let p = players.get(&green_addr).unwrap();
        assert!(projectiles.is_empty());
        assert!((p.state.hp - hp_before).abs() < EPSILON);
        assert!((p.state.mana - mana_before).abs() < EPSILON);
    }

    #[test]
    fn cast_self_target_applies_without_network_target() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let addr: SocketAddr = "127.0.0.1:50206".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, addr, &mut next_player_id, now);
        {
            let p = players.get_mut(&addr).unwrap();
            p.state.level = 2;
            p.state.abilities = PlayerAbilitySnapshot::fresh_for_level(2);
            p.state.hp = 70.0;
            p.state.mana = MAX_MANA;
        }

        let mut projectiles = HashMap::new();
        let mut next_pid = 1_u64;
        let mut minions = HashMap::new();
        let mut structures = build_structures(&layout);
        let mut neutrals = HashMap::new();

        handle_cast_request(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            addr,
            SkillSlot::W,
            None,
            &mut next_pid,
            &GameState::Running,
            now,
        );

        let p = players.get(&addr).unwrap();
        assert!(projectiles.is_empty());
        assert!(p.state.hp > 70.0);
        assert!(p.state.mana < MAX_MANA);
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
    fn ranged_shot_rank_improves_damage_range_speed_and_cooldown() {
        assert!(ranged_shot_damage_for_rank(2) > ranged_shot_damage_for_rank(1));
        assert!(ranged_shot_range_for_rank(2) > ranged_shot_range_for_rank(1));
        assert!(ranged_shot_speed_for_rank(2) > ranged_shot_speed_for_rank(1));
        assert!(ranged_shot_cooldown_for_rank(2) < ranged_shot_cooldown_for_rank(1));
    }

    #[test]
    fn ranged_shot_respects_max_range_on_server() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let caster_addr: SocketAddr = "127.0.0.1:50001".parse().unwrap();
        let victim_addr: SocketAddr = "127.0.0.1:50002".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, caster_addr, &mut next_player_id, now);
        ensure_player_connected(&mut players, &layout, victim_addr, &mut next_player_id, now);

        let victim_id = players.get(&victim_addr).unwrap().state.id;
        {
            let v = players.get_mut(&victim_addr).unwrap();
            v.state.team = Team::Blue;
            v.state.x = 500.0;
            v.state.z = 500.0;
        }
        {
            let c = players.get_mut(&caster_addr).unwrap();
            c.state.team = Team::Green;
            c.state.x = 0.0;
            c.state.z = 0.0;
            c.state.mana = MAX_MANA;
            c.state.ranged_shot_rank = 1;
        }

        let mut projectiles = HashMap::new();
        let mut minions = HashMap::new();
        let mut structures = HashMap::new();
        let mut neutrals = HashMap::new();
        let mut next_projectile_id = 1_u64;

        try_ranged_shot_ability(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: victim_id,
            },
            &mut next_projectile_id,
            now,
        );

        assert!(
            projectiles.is_empty(),
            "Ranged Shot must not spawn a projectile when target exceeds max range"
        );
        let caster = players.get(&caster_addr).unwrap();
        assert!(
            caster.pending_ability_feedback.is_some(),
            "expected server feedback when out of range"
        );
    }

    #[test]
    fn ranged_shot_ability_spawns_projectile_with_rank_scaled_damage() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let caster_addr: SocketAddr = "127.0.0.1:50011".parse().unwrap();
        let victim_addr: SocketAddr = "127.0.0.1:50012".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, caster_addr, &mut next_player_id, now);
        ensure_player_connected(&mut players, &layout, victim_addr, &mut next_player_id, now);

        let victim_id = players.get(&victim_addr).unwrap().state.id;
        let caster_id = players.get(&caster_addr).unwrap().state.id;
        {
            let v = players.get_mut(&victim_addr).unwrap();
            v.state.team = Team::Blue;
            v.state.x = 10.0;
            v.state.z = 0.0;
        }
        {
            let c = players.get_mut(&caster_addr).unwrap();
            c.state.team = Team::Green;
            c.state.x = 5.0;
            c.state.z = 0.0;
            c.state.mana = MAX_MANA;
            c.state.ranged_shot_rank = 3;
        }

        let mut projectiles = HashMap::new();
        let mut minions = HashMap::new();
        let mut structures = HashMap::new();
        let mut neutrals = HashMap::new();
        let mut next_projectile_id = 1_u64;

        try_ranged_shot_ability(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: victim_id,
            },
            &mut next_projectile_id,
            now,
        );

        assert_eq!(projectiles.len(), 1);
        let projectile = projectiles.values().next().unwrap();
        assert_eq!(projectile.damage, ranged_shot_damage_for_rank(3));
        assert_eq!(projectile.state.owner_id, caster_id);
        assert!(matches!(
            projectile.state.visual,
            ProjectileVisual::RangedShot
        ));

        let caster = players.get(&caster_addr).unwrap();
        assert!((caster.state.mana - (MAX_MANA - RANGED_SHOT_MANA)).abs() < EPSILON);
        assert!(caster.last_ranged_shot_at.is_some());
        assert!(caster.pending_ability_feedback.is_none());
    }

    #[test]
    fn ranged_shot_succeeds_when_melee_strike_is_on_cooldown() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let caster_addr: SocketAddr = "127.0.0.1:50021".parse().unwrap();
        let victim_addr: SocketAddr = "127.0.0.1:50022".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, caster_addr, &mut next_player_id, now);
        ensure_player_connected(&mut players, &layout, victim_addr, &mut next_player_id, now);

        let victim_id = players.get(&victim_addr).unwrap().state.id;
        {
            let v = players.get_mut(&victim_addr).unwrap();
            v.state.team = Team::Blue;
            v.state.x = 10.0;
            v.state.z = 0.0;
        }
        {
            let c = players.get_mut(&caster_addr).unwrap();
            c.state.team = Team::Green;
            c.state.x = 5.0;
            c.state.z = 0.0;
            c.state.mana = MAX_MANA;
            c.state.ranged_shot_rank = 1;
            c.last_melee_at = Some(now);
            c.last_ranged_shot_at = None;
        }

        let mut projectiles = HashMap::new();
        let mut minions = HashMap::new();
        let mut structures = HashMap::new();
        let mut neutrals = HashMap::new();
        let mut next_projectile_id = 1_u64;

        try_ranged_shot_ability(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: victim_id,
            },
            &mut next_projectile_id,
            now,
        );

        assert_eq!(projectiles.len(), 1);
    }

    #[test]
    fn melee_strike_succeeds_when_ranged_shot_is_on_cooldown() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let caster_addr: SocketAddr = "127.0.0.1:50031".parse().unwrap();
        let victim_addr: SocketAddr = "127.0.0.1:50032".parse().unwrap();
        let now = Instant::now();
        let mut game_state = GameState::Running;
        ensure_player_connected(&mut players, &layout, caster_addr, &mut next_player_id, now);
        ensure_player_connected(&mut players, &layout, victim_addr, &mut next_player_id, now);

        let victim_id = players.get(&victim_addr).unwrap().state.id;
        let victim_hp_before = players.get(&victim_addr).unwrap().state.hp;
        {
            let v = players.get_mut(&victim_addr).unwrap();
            v.state.team = Team::Blue;
            v.state.x = 5.5;
            v.state.z = 0.0;
        }
        {
            let c = players.get_mut(&caster_addr).unwrap();
            c.state.team = Team::Green;
            c.state.x = 5.0;
            c.state.z = 0.0;
            c.state.mana = MAX_MANA;
            c.last_ranged_shot_at = Some(now);
            c.last_melee_at = None;
        }

        let mut minions = HashMap::new();
        let mut structures = HashMap::new();
        let mut neutrals = HashMap::new();

        try_melee_strike(
            &mut players,
            &mut minions,
            &mut structures,
            &mut neutrals,
            &mut game_state,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: victim_id,
            },
            now,
        );

        let victim = players.get(&victim_addr).unwrap();
        assert!(victim.state.hp < victim_hp_before);
        let caster = players.get(&caster_addr).unwrap();
        assert!((caster.state.mana - (MAX_MANA - MELEE_STRIKE_MANA)).abs() < EPSILON);
        assert!(caster.last_melee_at.is_some());
    }

    #[test]
    fn ranged_shot_invalid_dead_player_target_queues_feedback() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let caster_addr: SocketAddr = "127.0.0.1:50041".parse().unwrap();
        let victim_addr: SocketAddr = "127.0.0.1:50042".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, caster_addr, &mut next_player_id, now);
        ensure_player_connected(&mut players, &layout, victim_addr, &mut next_player_id, now);

        let victim_id = players.get(&victim_addr).unwrap().state.id;
        {
            let v = players.get_mut(&victim_addr).unwrap();
            v.state.team = Team::Blue;
            v.state.hp = 0.0;
        }
        {
            let c = players.get_mut(&caster_addr).unwrap();
            c.state.team = Team::Green;
            c.state.mana = MAX_MANA;
        }

        let mut projectiles = HashMap::new();
        let mut minions = HashMap::new();
        let mut structures = HashMap::new();
        let mut neutrals = HashMap::new();
        let mut next_projectile_id = 1_u64;

        try_ranged_shot_ability(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            caster_addr,
            TargetId {
                kind: TargetKind::Player,
                id: victim_id,
            },
            &mut next_projectile_id,
            now,
        );

        assert!(projectiles.is_empty());
        let caster = players.get(&caster_addr).unwrap();
        assert!(caster
            .pending_ability_feedback
            .as_ref()
            .is_some_and(|m| m.contains("invalid") || m.contains("dead")));
    }

    #[test]
    fn ranged_shot_rejects_friendly_minion_without_resource_spend() {
        let layout = build_map_layout();
        let mut players = HashMap::new();
        let mut next_player_id = 1;
        let caster_addr: SocketAddr = "127.0.0.1:50051".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(&mut players, &layout, caster_addr, &mut next_player_id, now);
        let (cx, cz, mana_before) = {
            let caster = players.get(&caster_addr).unwrap();
            (caster.state.x, caster.state.z, caster.state.mana)
        };

        let mut projectiles = HashMap::new();
        let mut minions = HashMap::new();
        let friendly_minion_id = 8801;
        minions.insert(
            friendly_minion_id,
            Minion {
                state: MinionState {
                    id: friendly_minion_id,
                    team: Team::Green,
                    lane: Lane::Mid,
                    x: cx + 2.0,
                    y: MINION_SPAWN_HEIGHT,
                    z: cz,
                    yaw: 0.0,
                    hp: MINION_MAX_HP,
                    max_hp: MINION_MAX_HP,
                    state: MinionBrainState::Marching,
                    target_kind: None,
                    target_id: None,
                },
                path: vec![Vec3f::new(cx + 2.0, MINION_SPAWN_HEIGHT, cz)],
                next_waypoint: 0,
                last_attack_at: None,
                aggro_target: None,
            },
        );
        let mut structures = HashMap::new();
        let mut neutrals = HashMap::new();
        let mut next_projectile_id = 1_u64;

        try_ranged_shot_ability(
            &mut players,
            &mut projectiles,
            &mut minions,
            &mut structures,
            &mut neutrals,
            caster_addr,
            TargetId {
                kind: TargetKind::Minion,
                id: friendly_minion_id,
            },
            &mut next_projectile_id,
            now,
        );

        let caster = players.get(&caster_addr).unwrap();
        assert!(projectiles.is_empty());
        assert!((caster.state.mana - mana_before).abs() < EPSILON);
        assert!(caster.last_ranged_shot_at.is_none());
        assert!(
            caster
                .pending_ability_feedback
                .as_ref()
                .is_some_and(|m| m.contains("invalid"))
        );
    }
}

fn handle_respawns(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    structures: &HashMap<u64, Structure>,
    map_layout: &MapLayoutState,
    game_state: &GameState,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    for player in players.values_mut() {
        let Some(respawn_at) = player.respawn_at else {
            continue;
        };
        if now < respawn_at {
            continue;
        }
        let spawn = spawn_position_for_team_from_base(structures, map_layout, player.state.team);
        player.state.x = spawn.x;
        player.state.y = 0.5;
        player.state.z = spawn.z;
        player.state.yaw = 0.0;
        player.state.hp = player.state.max_hp;
        player.state.mana = player.state.max_mana;
        player.respawn_at = None;
        player.last_melee_at = None;
        player.last_ranged_shot_at = None;
    }
}

fn reset_match(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    structures: &mut HashMap<u64, Structure>,
    minions: &mut HashMap<u64, Minion>,
    projectiles: &mut HashMap<u64, Projectile>,
    map_layout: &MapLayoutState,
    last_wave_spawn_at: &mut Instant,
    game_state: &mut GameState,
) {
    println!("Resetting match for rematch");
    // Reset structures HP
    for structure in structures.values_mut() {
        let max = structure.state.max_hp;
        structure.state.hp = max;
        structure.last_attack_at = None;
    }
    // Clear minions and projectiles
    minions.clear();
    projectiles.clear();
    // Reset wave timer so first wave isn't immediate
    *last_wave_spawn_at = Instant::now();
    // Reset all players to spawn
    for player in players.values_mut() {
        let spawn = spawn_position_for_team(map_layout, player.state.team);
        player.state.x = spawn.x;
        player.state.y = 0.5;
        player.state.z = spawn.z;
        player.state.yaw = 0.0;
        player.state.hp = MAX_HP;
        player.state.max_hp = MAX_HP;
        player.state.mana = MAX_MANA;
        player.state.max_mana = MAX_MANA;
        player.state.gold = 0;
        player.state.xp = 0;
        player.state.skill_points = 0;
        player.state.ranged_shot_rank = 1;
        player.last_melee_at = None;
        player.last_ranged_shot_at = None;
        player.pending_ability_feedback = None;
        player.respawn_at = None;
    }
    *game_state = GameState::Running;
}
