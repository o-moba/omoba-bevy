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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientPacket {
    Transform { x: f32, y: f32, z: f32, yaw: f32 },
    Cast { target_id: u64 },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlayerState {
    id: u64,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    hp: f32,
    max_hp: f32,
    mana: f32,
    max_mana: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectileState {
    id: u64,
    owner_id: u64,
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
}

struct Projectile {
    state: ProjectileState,
    target_id: u64,
    velocity: Vec3f,
    damage: f32,
    radius: f32,
    expires_at: Instant,
}

fn main() -> io::Result<()> {
    let bind_addr = std::env::var("SERVER_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    let socket = UdpSocket::bind(&bind_addr)?;
    socket.set_nonblocking(true)?;

    println!("UDP game server is listening on {bind_addr}");

    let mut players = HashMap::<SocketAddr, ConnectedPlayer>::new();
    let mut projectiles = HashMap::<u64, Projectile>::new();
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
                        ClientPacket::Cast { target_id } => {
                            handle_cast_request(
                                &mut players,
                                &mut projectiles,
                                addr,
                                target_id,
                                &mut next_projectile_id,
                                now,
                            );
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
        simulate_projectiles(&mut players, &mut projectiles, dt, now);

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
            live_player_ids.contains(&projectile.state.owner_id)
                && live_player_ids.contains(&projectile.target_id)
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

            for (addr, player) in &players {
                let packet = ServerPacket::Snapshot {
                    your_id: player.state.id,
                    players: players_snapshot.clone(),
                    projectiles: projectiles_snapshot.clone(),
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
                hp: MAX_HP,
                max_hp: MAX_HP,
                mana: MAX_MANA,
                max_mana: MAX_MANA,
            },
            last_seen: now,
            last_cast_at: None,
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
    caster_addr: SocketAddr,
    target_id: u64,
    next_projectile_id: &mut u64,
    now: Instant,
) {
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

    let Some(target) = players
        .values()
        .find(|player| player.state.id == target_id && player.state.hp > 0.0)
    else {
        return;
    };

    let caster_position = Vec3f::new(
        caster.state.x,
        caster.state.y + CAST_SPAWN_HEIGHT,
        caster.state.z,
    );
    let target_position = Vec3f::new(target.state.x, target.state.y + AIM_HEIGHT, target.state.z);
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
                x: caster_position.x,
                y: caster_position.y,
                z: caster_position.z,
            },
            target_id,
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
    projectiles: &mut HashMap<u64, Projectile>,
    dt: f32,
    now: Instant,
) {
    let mut damage_events: Vec<(u64, f32)> = Vec::new();

    projectiles.retain(|_, projectile| {
        if now >= projectile.expires_at {
            return false;
        }

        let position = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z)
            .add_scaled(projectile.velocity, dt);
        projectile.state.x = position.x;
        projectile.state.y = position.y;
        projectile.state.z = position.z;

        let Some(target) = players
            .values()
            .find(|player| player.state.id == projectile.target_id && player.state.hp > 0.0)
        else {
            return false;
        };

        let target_pos = Vec3f::new(target.state.x, target.state.y + AIM_HEIGHT, target.state.z);
        let combined_radius = projectile.radius + PLAYER_HIT_RADIUS;
        if position.distance_squared(target_pos) <= combined_radius * combined_radius {
            damage_events.push((projectile.target_id, projectile.damage));
            return false;
        }

        true
    });

    for (target_id, damage) in damage_events {
        if let Some(target_player) = players
            .values_mut()
            .find(|player| player.state.id == target_id && player.state.hp > 0.0)
        {
            target_player.state.hp = (target_player.state.hp - damage).max(0.0);
        }
    }
}
