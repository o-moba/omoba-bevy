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
const NEXUS_MAX_HP: f32 = 650.0;
const TOWER_SIZE: f32 = 2.6;
const NEXUS_SIZE: f32 = 8.0;
const TOWER_RANGE: f32 = 20.0;
const TOWER_DAMAGE: f32 = 14.0;
const TOWER_COOLDOWN: Duration = Duration::from_millis(900);
const TOWER_SHOT_HEIGHT: f32 = 2.4;

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
    Transform { x: f32, y: f32, z: f32, yaw: f32 },
    Cast { target: TargetId },
    Join { team: Team },
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
enum TargetKind {
    Player,
    Structure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct TargetId {
    kind: TargetKind,
    id: u64,
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StructureKind {
    Tower,
    Nexus,
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
    damage: f32,
    radius: f32,
    expires_at: Instant,
}

struct Structure {
    state: StructureState,
    last_attack_at: Option<Instant>,
    attack_range: f32,
    attack_damage: f32,
    attack_cooldown: Duration,
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
    let mut game_state = GameState::Running;
    let mut next_player_id: u64 = 1;
    let mut next_projectile_id: u64 = 1;
    let mut recv_buf = [0_u8; MAX_PACKET_SIZE];
    let mut last_snapshot_at = Instant::now();
    let mut last_simulation_at = Instant::now();

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
                    ensure_player_connected(&mut players, addr, &mut next_player_id, now);
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
                                &mut structures,
                                addr,
                                target,
                                &mut next_projectile_id,
                                &game_state,
                                now,
                            );
                        }
                        ClientPacket::Join { team } => {
                            if let Some(player) = players.get_mut(&addr) {
                                player.state.team = team;
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
        simulate_tower_attacks(
            &players,
            &mut projectiles,
            &mut structures,
            &mut next_projectile_id,
            &game_state,
            now,
        );
        simulate_projectiles(
            &mut players,
            &mut structures,
            &mut projectiles,
            &mut game_state,
            dt,
            now,
        );
        handle_respawns(&mut players, &map_layout, &game_state, now);

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
            let owner_alive =
                projectile.state.owner_id == 0 || live_player_ids.contains(&projectile.state.owner_id);
            let target_alive = match projectile.target.kind {
                TargetKind::Player => live_player_ids.contains(&projectile.target.id),
                TargetKind::Structure => structures
                    .get(&projectile.target.id)
                    .is_some_and(|structure| structure.state.hp > 0.0),
            };
            owner_alive && target_alive
        });

        if now.duration_since(last_snapshot_at) >= SNAPSHOT_INTERVAL {
            let players_snapshot = players
                .values()
                .map(|player| player.state.clone())
                .collect::<Vec<_>>();
            let projectiles_snapshot = projectiles
                .values()
                .map(|projectile| projectile.state.clone())
                .collect::<Vec<_>>();
            let structures_snapshot = structures
                .values()
                .filter(|structure| structure.state.hp > 0.0)
                .map(|structure| structure.state.clone())
                .collect::<Vec<_>>();

            for (addr, player) in &players {
                let packet = ServerPacket::Snapshot {
                    your_id: player.state.id,
                    players: players_snapshot.clone(),
                    projectiles: projectiles_snapshot.clone(),
                    structures: structures_snapshot.clone(),
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
    addr: SocketAddr,
    next_player_id: &mut u64,
    now: Instant,
) {
    players.entry(addr).or_insert_with(|| {
        let player_id = *next_player_id;
        *next_player_id += 1;
        println!("Player {player_id} connected from {addr}");

        ConnectedPlayer {
            state: PlayerState {
                id: player_id,
                x: 0.0,
                y: 0.5,
                z: 0.0,
                yaw: 0.0,
                team: Team::Green,
                hp: MAX_HP,
                max_hp: MAX_HP,
                mana: MAX_MANA,
                max_mana: MAX_MANA,
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
        player.state.mana =
            (player.state.mana + MANA_REGEN_PER_SECOND * dt).clamp(0.0, player.state.max_mana);
    }
}

fn handle_cast_request(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    projectiles: &mut HashMap<u64, Projectile>,
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
            let Some(target_player) = players
                .values()
                .find(|player| {
                    player.state.id == target.id
                        && player.state.hp > 0.0
                        && player.state.team != caster_team
                })
            else {
                return;
            };
            Vec3f::new(
                target_player.state.x,
                target_player.state.y + AIM_HEIGHT,
                target_player.state.z,
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
            damage: PROJECTILE_DAMAGE,
            radius: PROJECTILE_RADIUS,
            expires_at: now + PROJECTILE_LIFETIME,
        },
    );
}

fn simulate_projectiles(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
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
    let mut structure_damage_events: Vec<(u64, f32, Team)> = Vec::new();

    projectiles.retain(|_, projectile| {
        if now >= projectile.expires_at {
            return false;
        }

        let position = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z)
            .add_scaled(projectile.velocity, dt);
        projectile.state.x = position.x;
        projectile.state.y = position.y;
        projectile.state.z = position.z;

        match projectile.target.kind {
            TargetKind::Player => {
                let Some(target) = players
                    .values()
                    .find(|player| player.state.id == projectile.target.id && player.state.hp > 0.0)
                else {
                    return false;
                };

                let target_pos =
                    Vec3f::new(target.state.x, target.state.y + AIM_HEIGHT, target.state.z);
                let combined_radius = projectile.radius + PLAYER_HIT_RADIUS;
                if position.distance_squared(target_pos) <= combined_radius * combined_radius {
                    player_damage_events.push((projectile.target.id, projectile.damage));
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
                let target_pos = Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
                let target_radius = match structure.state.kind {
                    StructureKind::Tower => TOWER_SIZE * 0.5,
                    StructureKind::Nexus => NEXUS_SIZE * 0.5,
                };
                let combined_radius = projectile.radius + target_radius;
                if position.distance_squared(target_pos) <= combined_radius * combined_radius {
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

    for (target_id, damage, attacker_team) in structure_damage_events {
        if let Some(target_structure) = structures.get_mut(&target_id) {
            if target_structure.state.hp <= 0.0 {
                continue;
            }
            target_structure.state.hp = (target_structure.state.hp - damage).max(0.0);
            if target_structure.state.hp <= 0.0 {
                if target_structure.state.kind == StructureKind::Nexus {
                    *game_state = GameState::Victory {
                        winner: attacker_team,
                    };
                }
            }
        }
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
    fn team_assignment_by_distance() {
        let layout = build_map_layout();
        let home_team = team_for_position(layout.home, layout.away, layout.home);
        let away_team = team_for_position(layout.home, layout.away, layout.away);
        assert_eq!(home_team, Team::Green);
        assert_eq!(away_team, Team::Blue);

        let near_home = layout.home.lerp(layout.away, 0.2);
        let near_away = layout.home.lerp(layout.away, 0.8);
        assert_eq!(team_for_position(layout.home, layout.away, near_home), Team::Green);
        assert_eq!(team_for_position(layout.home, layout.away, near_away), Team::Blue);
    }
}

fn build_structures(layout: &MapLayoutState) -> HashMap<u64, Structure> {
    let mut structures = HashMap::new();
    let mut next_id: u64 = 1;

    let mid_towers = [0.2, 0.4, 0.6, 0.8];
    for t in mid_towers {
        let position = layout.home.lerp(layout.away, t);
        let tower_pos = Vec3f::new(position.x, 3.0, position.z);
        let team = team_for_position(layout.home, layout.away, tower_pos);
        add_structure(&mut structures, &mut next_id, StructureKind::Tower, team, tower_pos);
    }

    let lane_span_x = layout.right_x - layout.left_x;
    let top_positions = [
        Vec3f::new(layout.left_x + lane_span_x * 0.25, 3.0, layout.top_z),
        Vec3f::new(layout.left_x + lane_span_x * 0.75, 3.0, layout.top_z),
    ];
    let bot_positions = [
        Vec3f::new(layout.left_x + lane_span_x * 0.25, 3.0, layout.bottom_z),
        Vec3f::new(layout.left_x + lane_span_x * 0.75, 3.0, layout.bottom_z),
    ];
    for position in top_positions.into_iter().chain(bot_positions) {
        let team = team_for_position(layout.home, layout.away, position);
        add_structure(&mut structures, &mut next_id, StructureKind::Tower, team, position);
    }

    let home_nexus = Vec3f::new(layout.home.x, 4.0, layout.home.z);
    let away_nexus = Vec3f::new(layout.away.x, 4.0, layout.away.z);
    add_structure(
        &mut structures,
        &mut next_id,
        StructureKind::Nexus,
        Team::Green,
        home_nexus,
    );
    add_structure(
        &mut structures,
        &mut next_id,
        StructureKind::Nexus,
        Team::Blue,
        away_nexus,
    );

    structures
}

fn add_structure(
    structures: &mut HashMap<u64, Structure>,
    next_id: &mut u64,
    kind: StructureKind,
    team: Team,
    position: Vec3f,
) {
    let (max_hp, attack_range, attack_damage, attack_cooldown) = match kind {
        StructureKind::Tower => (TOWER_MAX_HP, TOWER_RANGE, TOWER_DAMAGE, TOWER_COOLDOWN),
        StructureKind::Nexus => (NEXUS_MAX_HP, 0.0, 0.0, Duration::from_secs(0)),
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

fn team_for_position(home: Vec3f, away: Vec3f, position: Vec3f) -> Team {
    let home_dist = position.distance_squared(home);
    let away_dist = position.distance_squared(away);
    if home_dist <= away_dist {
        Team::Green
    } else {
        Team::Blue
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

fn simulate_tower_attacks(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    projectiles: &mut HashMap<u64, Projectile>,
    structures: &mut HashMap<u64, Structure>,
    next_projectile_id: &mut u64,
    game_state: &GameState,
    now: Instant,
) {
    if !matches!(game_state, GameState::Running) {
        return;
    }
    let mut towers_to_fire: Vec<(Team, Vec3f, u64, Vec3f, f32)> = Vec::new();

    for structure in structures.values_mut() {
        if structure.state.hp <= 0.0 {
            continue;
        }
        if structure.state.kind != StructureKind::Tower {
            continue;
        }
        if structure
            .last_attack_at
            .is_some_and(|last| now.duration_since(last) < structure.attack_cooldown)
        {
            continue;
        }

        let tower_position = Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
        let mut best_target: Option<(u64, Vec3f, f32)> = None;
        let range_sq = structure.attack_range * structure.attack_range;
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
            ));
        }
    }

    for (team, tower_position, target_id, target_pos, damage) in towers_to_fire {
        let origin = Vec3f::new(
            tower_position.x,
            tower_position.y + TOWER_SHOT_HEIGHT,
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
                damage,
                radius: PROJECTILE_RADIUS,
                expires_at: now + PROJECTILE_LIFETIME,
            },
        );
    }
}

fn handle_respawns(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
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
        let spawn = match player.state.team {
            Team::Green => map_layout.home,
            Team::Blue => map_layout.away,
        };
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
