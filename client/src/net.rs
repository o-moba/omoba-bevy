use bevy::prelude::*;
use bevy::scene::SceneRoot;
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::UdpSocket,
    thread,
    time::{Duration, Instant},
};

use crate::player::Player;
use crate::world::PlayerAssets;

const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:4000";
const LOCAL_BIND_ADDR: &str = "0.0.0.0:0";
const UPDATE_INTERVAL_SECONDS: f32 = 0.05;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const NETWORK_LOOP_SLEEP: Duration = Duration::from_millis(16);
const MAX_PACKET_SIZE: usize = 8 * 1024;

pub struct NetworkingPlugin;

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetworkState>()
            .insert_resource(LocalStateSendTimer(Timer::from_seconds(
                UPDATE_INTERVAL_SECONDS,
                TimerMode::Repeating,
            )))
            .add_systems(Startup, start_networking)
            .add_systems(Update, (send_local_state, apply_server_snapshot));
    }
}

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

#[derive(Resource)]
struct NetworkChannels {
    outgoing: Sender<ClientPacket>,
    incoming: Receiver<ServerPacket>,
}

#[derive(Resource, Default)]
struct NetworkState {
    local_id: Option<u64>,
    remote_players: HashMap<u64, Entity>,
}

#[derive(Resource)]
struct LocalStateSendTimer(Timer);

#[derive(Component)]
struct RemotePlayer;

fn start_networking(mut commands: Commands) {
    let server_addr =
        std::env::var("GAME_SERVER_ADDR").unwrap_or_else(|_| DEFAULT_SERVER_ADDR.to_owned());
    let (outgoing_tx, outgoing_rx) = unbounded::<ClientPacket>();
    let (incoming_tx, incoming_rx) = unbounded::<ServerPacket>();

    thread::spawn(move || {
        run_udp_client(server_addr, outgoing_rx, incoming_tx);
    });

    commands.insert_resource(NetworkChannels {
        outgoing: outgoing_tx,
        incoming: incoming_rx,
    });
}

fn run_udp_client(
    server_addr: String,
    outgoing: Receiver<ClientPacket>,
    incoming: Sender<ServerPacket>,
) {
    let socket = match UdpSocket::bind(LOCAL_BIND_ADDR) {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("Failed to bind client UDP socket: {error}");
            return;
        }
    };

    if let Err(error) = socket.connect(&server_addr) {
        eprintln!("Failed to connect UDP socket to {server_addr}: {error}");
        return;
    }
    if let Err(error) = socket.set_nonblocking(true) {
        eprintln!("Failed to set UDP client socket nonblocking: {error}");
        return;
    }

    let mut recv_buf = [0_u8; MAX_PACKET_SIZE];
    let mut last_heartbeat_at = Instant::now();

    let _ = send_packet(&socket, &ClientPacket::Ping);

    loop {
        loop {
            match outgoing.try_recv() {
                Ok(packet) => {
                    if send_packet(&socket, &packet).is_err() {
                        eprintln!("Failed to send packet to server");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        if last_heartbeat_at.elapsed() >= HEARTBEAT_INTERVAL {
            let _ = send_packet(&socket, &ClientPacket::Ping);
            last_heartbeat_at = Instant::now();
        }

        loop {
            match socket.recv(&mut recv_buf) {
                Ok(len) => match serde_json::from_slice::<ServerPacket>(&recv_buf[..len]) {
                    Ok(packet) => {
                        let _ = incoming.send(packet);
                    }
                    Err(error) => {
                        eprintln!("Failed to decode server packet: {error}");
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("Client socket receive error: {error}");
                    break;
                }
            }
        }

        thread::sleep(NETWORK_LOOP_SLEEP);
    }
}

fn send_packet(socket: &UdpSocket, packet: &ClientPacket) -> io::Result<()> {
    let payload = serde_json::to_vec(packet)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    socket.send(&payload)?;
    Ok(())
}

fn send_local_state(
    time: Res<Time>,
    mut timer: ResMut<LocalStateSendTimer>,
    channels: Option<Res<NetworkChannels>>,
    player_query: Query<&Transform, With<Player>>,
) {
    let Some(channels) = channels else {
        return;
    };

    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    let Ok(player_transform) = player_query.get_single() else {
        return;
    };

    let (yaw, _pitch, _roll) = player_transform.rotation.to_euler(EulerRot::YXZ);
    let packet = ClientPacket::Transform {
        x: player_transform.translation.x,
        y: player_transform.translation.y,
        z: player_transform.translation.z,
        yaw,
    };
    let _ = channels.outgoing.send(packet);
}

fn apply_server_snapshot(
    mut commands: Commands,
    channels: Option<Res<NetworkChannels>>,
    mut network_state: ResMut<NetworkState>,
    mut transforms: Query<&mut Transform>,
    remote_query: Query<&RemotePlayer>,
    player_assets: Res<PlayerAssets>,
) {
    let Some(channels) = channels else {
        return;
    };

    let mut latest_snapshot: Option<(u64, Vec<PlayerState>)> = None;
    while let Ok(packet) = channels.incoming.try_recv() {
        match packet {
            ServerPacket::Snapshot { your_id, players } => {
                latest_snapshot = Some((your_id, players));
            }
        }
    }

    let Some((your_id, players)) = latest_snapshot else {
        return;
    };

    network_state.local_id = Some(your_id);
    let mut seen_remote_ids = HashSet::new();

    for player in players {
        if player.id == your_id {
            continue;
        }
        seen_remote_ids.insert(player.id);

        if let Some(entity) = network_state.remote_players.get(&player.id).copied() {
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.translation = Vec3::new(player.x, player.y, player.z);
                transform.rotation = Quat::from_rotation_y(player.yaw);
            }
            continue;
        }

        let scene_handle = player_assets.scene.clone();
        let mesh_handle = player_assets.mesh.clone();
        let material_handle = player_assets.material.clone();
        let mut entity_commands = commands.spawn((
            SpatialBundle::from_transform(Transform::from_xyz(player.x, player.y, player.z)),
            RemotePlayer,
            Name::new(format!("RemotePlayer-{}", player.id)),
        ));
        entity_commands.with_children(|parent| {
            if let Some(scene_handle) = scene_handle {
                parent.spawn((SceneRoot(scene_handle), SpatialBundle::default()));
            } else {
                parent.spawn((
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material_handle),
                    SpatialBundle::default(),
                ));
            }
        });
        let entity = entity_commands.id();

        network_state.remote_players.insert(player.id, entity);
    }

    let stale_ids = network_state
        .remote_players
        .keys()
        .copied()
        .filter(|id| !seen_remote_ids.contains(id))
        .collect::<Vec<_>>();

    for player_id in stale_ids {
        if let Some(entity) = network_state.remote_players.remove(&player_id) {
            if remote_query.get(entity).is_ok() {
                commands.entity(entity).despawn_recursive();
            }
        }
    }
}
