use serde::{Deserialize, Serialize};
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
const SPELL_MANA_COST: f32 = 20.0;
const SPELL_COOLDOWN: Duration = Duration::from_millis(350);

const PROJECTILE_SPEED: f32 = 19.0;
const PROJECTILE_DAMAGE: f32 = 20.0;
const PROJECTILE_RADIUS: f32 = 0.22;
const PROJECTILE_LIFETIME: Duration = Duration::from_secs(3);
const PLAYER_HIT_RADIUS: f32 = 0.62;
const CAST_SPAWN_HEIGHT: f32 = 0.85;
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
        target: TargetId,
    },
    Join {
        team: Team,
        #[serde(default = "default_character_choice")]
        character: CharacterChoice,
    },
    Ping,
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct TargetId {
    kind: TargetKind,
    id: u64,
}

fn default_character_choice() -> CharacterChoice {
    CharacterChoice::Ipfs
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
    #[serde(default = "default_character_choice")]
    character: CharacterChoice,
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
        game_state: GameState,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GameState {
    Running,
    Victory { winner: Team },
}

impl Default for GameState {
    fn default() -> Self {
        GameState::Running
    }
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
    last_cast_at: Option<Instant>,
    respawn_at: Option<Instant>,
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
    let mut game_state = GameState::Running;
    let mut next_player_id: u64 = 1;
    let mut next_projectile_id: u64 = 1;
    let mut next_minion_id: u64 = 1;
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
                            if let Some(player) = players.get_mut(&addr) {
                                if player.state.hp > 0.0 {
                                    player.state.x = x;
                                    player.state.y = y;
                                    player.state.z = z;
                                    player.state.yaw = yaw;
                                }
                            }
                        }
                        ClientPacket::Cast { target } => {
                            handle_cast_request(
                                &mut players,
                                &mut projectiles,
                                &mut minions,
                                &mut structures,
                                addr,
                                target,
                                &mut next_projectile_id,
                                &game_state,
                                now,
                            );
                        }
                        ClientPacket::Join { team, character } => {
                            if let Some(player) = players.get_mut(&addr) {
                                handle_join_request(player, team, character, &map_layout);
                            }
                        }
                        ClientPacket::Ping => {}
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
            &mut projectiles,
            &mut game_state,
            dt,
            now,
        );
        handle_respawns(&mut players, &structures, &map_layout, &game_state, now);

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
            let target_alive = match projectile.target.kind {
                TargetKind::Player => live_player_ids.contains(&projectile.target.id),
                TargetKind::Minion => minions
                    .get(&projectile.target.id)
                    .is_some_and(|minion| minion.state.hp > 0.0),
                TargetKind::Structure => structures
                    .get(&projectile.target.id)
                    .is_some_and(|structure| structure.state.hp > 0.0),
            };
            target_alive
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

            for (addr, player) in &players {
                let packet = ServerPacket::Snapshot {
                    your_id: player.state.id,
                    players: players_snapshot.clone(),
                    projectiles: projectiles_snapshot.clone(),
                    structures: structures_snapshot.clone(),
                    minions: minions_snapshot.clone(),
                    game_state: game_state.clone(),
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
                character: default_character_choice(),
            },
            last_seen: now,
            last_cast_at: None,
            respawn_at: None,
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
        player.state.mana =
            (player.state.mana + MANA_REGEN_PER_SECOND * dt).clamp(0.0, player.state.max_mana);
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
    player.last_cast_at = None;
    player.respawn_at = None;
}

fn handle_cast_request(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    projectiles: &mut HashMap<u64, Projectile>,
    minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
    caster_addr: SocketAddr,
    target: TargetId,
    next_projectile_id: &mut u64,
    game_state: &GameState,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    let Some(caster) = players.get(&caster_addr) else {
        return;
    };
    if caster.state.hp <= 0.0 {
        return;
    }
    if caster.state.mana < SPELL_MANA_COST {
        return;
    }
    if caster
        .last_cast_at
        .is_some_and(|last_cast| now.duration_since(last_cast) < SPELL_COOLDOWN)
    {
        return;
    }

    let caster_team = caster.state.team;
    let target_position = match target.kind {
        TargetKind::Player => {
            let Some(target_player) = players.values().find(|player| {
                player.state.id == target.id
                    && player.state.hp > 0.0
                    && player.state.team != caster_team
            }) else {
                return;
            };
            Vec3f::new(
                target_player.state.x,
                target_player.state.y + AIM_HEIGHT,
                target_player.state.z,
            )
        }
        TargetKind::Minion => {
            let Some(target_minion) = minions.get(&target.id) else {
                return;
            };
            if target_minion.state.hp <= 0.0 {
                return;
            }
            Vec3f::new(
                target_minion.state.x,
                target_minion.state.y + MINION_RADIUS * 0.8,
                target_minion.state.z,
            )
        }
        TargetKind::Structure => {
            let Some(target_structure) = structures.get(&target.id) else {
                return;
            };
            if target_structure.state.hp <= 0.0 || target_structure.state.team == caster_team {
                return;
            }
            Vec3f::new(
                target_structure.state.x,
                target_structure.state.y,
                target_structure.state.z,
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

    let Some(caster_mut) = players.get_mut(&caster_addr) else {
        return;
    };
    caster_mut.state.mana -= SPELL_MANA_COST;
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
            },
            target,
            velocity: Vec3f::new(
                direction.x * PROJECTILE_SPEED,
                direction.y * PROJECTILE_SPEED,
                direction.z * PROJECTILE_SPEED,
            ),
            homing: true,
            guaranteed_hit: true,
            damage: PROJECTILE_DAMAGE,
            radius: PROJECTILE_RADIUS,
            expires_at: now + PROJECTILE_LIFETIME,
        },
    );
}

fn simulate_projectiles(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    structures: &mut HashMap<u64, Structure>,
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
                        direction.x * PROJECTILE_SPEED,
                        direction.y * PROJECTILE_SPEED,
                        direction.z * PROJECTILE_SPEED,
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
            if target_structure.state.hp <= 0.0 {
                if target_structure.state.kind == StructureKind::BaseTower {
                    *game_state = GameState::Victory {
                        winner: attacker_team,
                    };
                }
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
            if dist_sq <= range_sq {
                if best_target.map_or(true, |(_, _, best)| dist_sq < best) {
                    best_target = Some((player.state.id, target_pos, dist_sq));
                }
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
        player.last_cast_at = None;
    }
}
