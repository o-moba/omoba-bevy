use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, UdpSocket},
    thread,
    time::{Duration, Instant},
};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:4000";
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(50);
const PLAYER_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PACKET_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientPacket {
    Transform { x: f32, y: f32, z: f32, yaw: f32 },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlayerState {
    id: u64,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerPacket {
    Snapshot { your_id: u64, players: Vec<PlayerState> },
}

struct ConnectedPlayer {
    state: PlayerState,
    last_seen: Instant,
}

fn main() -> io::Result<()> {
    let bind_addr = std::env::var("SERVER_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    let socket = UdpSocket::bind(&bind_addr)?;
    socket.set_nonblocking(true)?;

    println!("UDP game server is listening on {bind_addr}");

    let mut players = HashMap::<SocketAddr, ConnectedPlayer>::new();
    let mut next_player_id: u64 = 1;
    let mut recv_buf = [0_u8; MAX_PACKET_SIZE];
    let mut last_snapshot_at = Instant::now();

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

                    let player = players.entry(addr).or_insert_with(|| {
                        let player_id = next_player_id;
                        next_player_id += 1;
                        println!("Player {player_id} connected from {addr}");

                        ConnectedPlayer {
                            state: PlayerState {
                                id: player_id,
                                x: 0.0,
                                y: 0.5,
                                z: 0.0,
                                yaw: 0.0,
                            },
                            last_seen: Instant::now(),
                        }
                    });

                    player.last_seen = Instant::now();
                    if let ClientPacket::Transform { x, y, z, yaw } = packet {
                        player.state.x = x;
                        player.state.y = y;
                        player.state.z = z;
                        player.state.yaw = yaw;
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
        players.retain(|addr, player| {
            let is_alive = now.duration_since(player.last_seen) <= PLAYER_TIMEOUT;
            if !is_alive {
                println!("Player {} timed out ({addr})", player.state.id);
            }
            is_alive
        });

        if now.duration_since(last_snapshot_at) >= SNAPSHOT_INTERVAL {
            let players_snapshot = players
                .values()
                .map(|player| player.state.clone())
                .collect::<Vec<_>>();

            for (addr, player) in &players {
                let packet = ServerPacket::Snapshot {
                    your_id: player.state.id,
                    players: players_snapshot.clone(),
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

        thread::sleep(Duration::from_millis(10));
    }
}
