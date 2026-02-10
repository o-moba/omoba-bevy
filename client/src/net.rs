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

use crate::camera::{CameraState, MainCamera, CAMERA_DISTANCE, CAMERA_HEIGHT};
use crate::combat::{CombatStats, MAX_HP, MAX_MANA};
use crate::player::{Player, PlayerBody, VerticalVelocity, PLAYER_SIZE};
use crate::team::Team;
use crate::world::PlayerAssets;

const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:4000";
const LOCAL_BIND_ADDR: &str = "0.0.0.0:0";
const UPDATE_INTERVAL_SECONDS: f32 = 0.05;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const NETWORK_LOOP_SLEEP: Duration = Duration::from_millis(16);
const MAX_PACKET_SIZE: usize = 8 * 1024;
const PROJECTILE_RADIUS: f32 = 0.22;
const TOWER_SIZE: f32 = 2.6;
const TOWER_HEIGHT: f32 = 6.0;
const NEXUS_SIZE: f32 = 8.0;

pub struct NetworkingPlugin;

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<NetworkCommand>()
            .init_resource::<NetworkState>()
            .init_resource::<GameStateSnapshot>()
            .insert_resource(LocalStateSendTimer(Timer::from_seconds(
                UPDATE_INTERVAL_SECONDS,
                TimerMode::Repeating,
            )))
            .add_systems(Startup, (setup_network_visual_assets, start_networking))
            .add_systems(
                Update,
                (
                    send_local_state,
                    send_network_commands,
                    apply_server_snapshot,
                ),
            );
    }
}

#[derive(Event, Clone, Copy, Debug)]
pub enum NetworkCommand {
    Cast { target: TargetId },
    Join { team: Team },
}

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
pub enum TargetKind {
    Player,
    Structure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetId {
    pub kind: TargetKind,
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlayerState {
    id: u64,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    #[serde(default = "default_team")]
    team: Team,
    #[serde(default = "default_hp")]
    hp: f32,
    #[serde(default = "default_max_hp")]
    max_hp: f32,
    #[serde(default = "default_mana")]
    mana: f32,
    #[serde(default = "default_max_mana")]
    max_mana: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectileState {
    id: u64,
    owner_id: u64,
    #[serde(default = "default_team")]
    owner_team: Team,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Component)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
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
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerPacket {
    Snapshot {
        your_id: u64,
        players: Vec<PlayerState>,
        #[serde(default)]
        projectiles: Vec<ProjectileState>,
        #[serde(default)]
        structures: Vec<StructureState>,
        #[serde(default)]
        game_state: GameState,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameState {
    Running,
    Victory { winner: Team },
}

impl Default for GameState {
    fn default() -> Self {
        GameState::Running
    }
}

#[derive(Resource, Default, Clone)]
pub struct GameStateSnapshot {
    pub state: GameState,
}

#[derive(Resource)]
struct NetworkChannels {
    outgoing: Sender<ClientPacket>,
    incoming: Receiver<ServerPacket>,
}

#[derive(Resource, Default)]
struct NetworkState {
    local_id: Option<u64>,
    local_team: Option<Team>,
    remote_players: HashMap<u64, Entity>,
    projectiles: HashMap<u64, Entity>,
    structures: HashMap<u64, Entity>,
}

#[derive(Resource)]
struct LocalStateSendTimer(Timer);

#[derive(Component)]
pub struct RemotePlayer;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkPlayerId(pub u64);

#[derive(Component)]
struct NetworkProjectile;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkStructureId(pub u64);

#[derive(Component)]
pub struct NetworkStructure;

#[derive(Resource)]
struct NetworkVisualAssets {
    projectile_mesh: Handle<Mesh>,
    friendly_projectile_material: Handle<StandardMaterial>,
    hostile_projectile_material: Handle<StandardMaterial>,
    tower_mesh: Handle<Mesh>,
    nexus_mesh: Handle<Mesh>,
    green_structure_material: Handle<StandardMaterial>,
    blue_structure_material: Handle<StandardMaterial>,
}

fn setup_network_visual_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let projectile_mesh = meshes.add(Mesh::from(Sphere::new(PROJECTILE_RADIUS)));
    let tower_mesh = meshes.add(Mesh::from(Cuboid::new(
        TOWER_SIZE,
        TOWER_HEIGHT,
        TOWER_SIZE,
    )));
    let nexus_mesh = meshes.add(Mesh::from(Cuboid::new(
        NEXUS_SIZE,
        NEXUS_SIZE,
        NEXUS_SIZE,
    )));
    let friendly_projectile_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.92, 1.0),
        unlit: true,
        ..default()
    });
    let hostile_projectile_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.36, 0.36),
        unlit: true,
        ..default()
    });
    let green_structure_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.14, 0.55, 0.22),
        perceptual_roughness: 0.75,
        ..default()
    });
    let blue_structure_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.18, 0.35, 0.75),
        perceptual_roughness: 0.75,
        ..default()
    });

    commands.insert_resource(NetworkVisualAssets {
        projectile_mesh,
        friendly_projectile_material,
        hostile_projectile_material,
        tower_mesh,
        nexus_mesh,
        green_structure_material,
        blue_structure_material,
    });
}

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

fn send_network_commands(
    mut command_events: EventReader<NetworkCommand>,
    channels: Option<Res<NetworkChannels>>,
) {
    let Some(channels) = channels else {
        return;
    };

    for command in command_events.read() {
        match command {
            NetworkCommand::Cast { target } => {
                let _ = channels
                    .outgoing
                    .send(ClientPacket::Cast { target: *target });
            }
            NetworkCommand::Join { team } => {
                let _ = channels.outgoing.send(ClientPacket::Join { team: *team });
            }
        }
    }
}

fn apply_server_snapshot(
    mut commands: Commands,
    channels: Option<Res<NetworkChannels>>,
    mut network_state: ResMut<NetworkState>,
    mut transform_sets: ParamSet<(Query<&mut Transform>, Query<&mut Transform, With<MainCamera>>)>,
    remote_query: Query<&RemotePlayer>,
    projectile_query: Query<&NetworkProjectile>,
    structure_query: Query<&NetworkStructure>,
    local_player_query: Query<Entity, With<Player>>,
    player_assets: Res<PlayerAssets>,
    visuals: Res<NetworkVisualAssets>,
    mut game_state_snapshot: ResMut<GameStateSnapshot>,
    mut cam_state: ResMut<CameraState>,
) {
    let Some(channels) = channels else {
        return;
    };

    let mut latest_snapshot: Option<(
        u64,
        Vec<PlayerState>,
        Vec<ProjectileState>,
        Vec<StructureState>,
        GameState,
    )> = None;
    while let Ok(packet) = channels.incoming.try_recv() {
        match packet {
            ServerPacket::Snapshot {
                your_id,
                players,
                projectiles,
                structures,
                game_state,
            } => {
                latest_snapshot = Some((your_id, players, projectiles, structures, game_state));
            }
        }
    }

    let Some((your_id, players, projectiles, structures, game_state)) = latest_snapshot else {
        return;
    };

    network_state.local_id = Some(your_id);
    game_state_snapshot.state = game_state;

    let local_player_state = players.iter().find(|player| player.id == your_id);
    if let Ok(local_entity) = local_player_query.get_single() {
        commands
            .entity(local_entity)
            .insert(NetworkPlayerId(your_id));
        if let Some(local_player_state) = local_player_state {
            commands
                .entity(local_entity)
                .insert((
                    local_player_state.team,
                    player_state_to_combat_stats(local_player_state),
                ));
            network_state.local_team = Some(local_player_state.team);
        }
    } else if let Some(local_player_state) = local_player_state {
        let spawn = Vec3::new(local_player_state.x, local_player_state.y, local_player_state.z);
        let entity = if let Some(scene_handle) = player_assets.scene.clone() {
            commands
                .spawn((
                    SceneRoot(scene_handle),
                    Transform {
                        translation: spawn,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::splat(1.0),
                    },
                    GlobalTransform::default(),
                    Visibility::default(),
                    Player,
                    PlayerBody,
                    VerticalVelocity::default(),
                    local_player_state.team,
                    NetworkPlayerId(your_id),
                    player_state_to_combat_stats(local_player_state),
                    Name::new("Player"),
                ))
                .id()
        } else {
            commands
                .spawn((
                    Mesh3d(player_assets.mesh.clone()),
                    MeshMaterial3d(player_assets.material.clone()),
                    Transform::from_translation(spawn),
                    Player,
                    PlayerBody,
                    VerticalVelocity::default(),
                    local_player_state.team,
                    NetworkPlayerId(your_id),
                    player_state_to_combat_stats(local_player_state),
                    Name::new("Player"),
                ))
                .id()
        };

        network_state.local_team = Some(local_player_state.team);
        if let Ok(mut camera_transform) = transform_sets.p1().get_single_mut() {
            cam_state.locked = true;
            camera_transform.translation =
                spawn + Vec3::new(0.0, CAMERA_HEIGHT, CAMERA_DISTANCE);
            let look_target = Vec3::new(spawn.x, PLAYER_SIZE * 0.5, spawn.z);
            *camera_transform = camera_transform.looking_at(look_target, Vec3::Y);
        }

        commands.entity(entity);
    }

    let mut seen_remote_ids = HashSet::new();

    for player in &players {
        if player.id == your_id {
            continue;
        }
        seen_remote_ids.insert(player.id);

        if let Some(entity) = network_state.remote_players.get(&player.id).copied() {
            if let Ok(mut transform) = transform_sets.p0().get_mut(entity) {
                transform.translation = Vec3::new(player.x, player.y, player.z);
                transform.rotation = Quat::from_rotation_y(player.yaw);
            }
            commands.entity(entity).insert((
                NetworkPlayerId(player.id),
                player.team,
                player_state_to_combat_stats(player),
            ));
            continue;
        }

        let scene_handle = player_assets.scene.clone();
        let mesh_handle = player_assets.mesh.clone();
        let material_handle = player_assets.material.clone();
        let mut entity_commands = commands.spawn((
            Transform::from_xyz(player.x, player.y, player.z),
            Visibility::default(),
            RemotePlayer,
            PlayerBody,
            player.team,
            NetworkPlayerId(player.id),
            player_state_to_combat_stats(player),
            Name::new(format!("RemotePlayer-{}", player.id)),
        ));
        entity_commands.with_children(|parent| {
            if let Some(scene_handle) = scene_handle {
                parent.spawn((
                    SceneRoot(scene_handle),
                    Transform::default(),
                    Visibility::default(),
                ));
            } else {
                parent.spawn((
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material_handle),
                    Transform::default(),
                    Visibility::default(),
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

    let mut seen_projectile_ids = HashSet::new();
    for projectile in &projectiles {
        seen_projectile_ids.insert(projectile.id);

        if let Some(entity) = network_state.projectiles.get(&projectile.id).copied() {
            if let Ok(mut transform) = transform_sets.p0().get_mut(entity) {
                transform.translation = Vec3::new(projectile.x, projectile.y, projectile.z);
            }
            continue;
        }

        let is_friendly = network_state
            .local_team
            .is_some_and(|team| team == projectile.owner_team);
        let material = if is_friendly {
            visuals.friendly_projectile_material.clone()
        } else {
            visuals.hostile_projectile_material.clone()
        };

        let entity = commands
            .spawn((
                Mesh3d(visuals.projectile_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(projectile.x, projectile.y, projectile.z),
                Visibility::default(),
                NetworkProjectile,
                Name::new(format!("Projectile-{}", projectile.id)),
            ))
            .id();
        network_state.projectiles.insert(projectile.id, entity);
    }

    let stale_projectile_ids = network_state
        .projectiles
        .keys()
        .copied()
        .filter(|id| !seen_projectile_ids.contains(id))
        .collect::<Vec<_>>();
    for projectile_id in stale_projectile_ids {
        if let Some(entity) = network_state.projectiles.remove(&projectile_id) {
            if projectile_query.get(entity).is_ok() {
                commands.entity(entity).despawn_recursive();
            }
        }
    }

    let mut seen_structure_ids = HashSet::new();
    for structure in &structures {
        seen_structure_ids.insert(structure.id);

        if let Some(entity) = network_state.structures.get(&structure.id).copied() {
            if let Ok(mut transform) = transform_sets.p0().get_mut(entity) {
                transform.translation = Vec3::new(structure.x, structure.y, structure.z);
            }
            commands.entity(entity).insert((
                structure.kind,
                structure.team,
                NetworkStructureId(structure.id),
                structure_state_to_combat_stats(structure),
            ));
            continue;
        }

        let material = match structure.team {
            Team::Green => visuals.green_structure_material.clone(),
            Team::Blue => visuals.blue_structure_material.clone(),
        };
        let mesh = match structure.kind {
            StructureKind::Tower => visuals.tower_mesh.clone(),
            StructureKind::Nexus => visuals.nexus_mesh.clone(),
        };

        let entity = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(structure.x, structure.y, structure.z),
                Visibility::default(),
                NetworkStructure,
                NetworkStructureId(structure.id),
                structure.kind,
                structure.team,
                structure_state_to_combat_stats(structure),
                Name::new(format!("Structure-{}", structure.id)),
            ))
            .id();

        network_state.structures.insert(structure.id, entity);
    }

    let stale_structure_ids = network_state
        .structures
        .keys()
        .copied()
        .filter(|id| !seen_structure_ids.contains(id))
        .collect::<Vec<_>>();
    for structure_id in stale_structure_ids {
        if let Some(entity) = network_state.structures.remove(&structure_id) {
            if structure_query.get(entity).is_ok() {
                commands.entity(entity).despawn_recursive();
            }
        }
    }
}

fn player_state_to_combat_stats(player: &PlayerState) -> CombatStats {
    CombatStats {
        hp: player.hp,
        max_hp: player.max_hp.max(1.0),
        mana: player.mana,
        max_mana: player.max_mana.max(1.0),
    }
}

fn structure_state_to_combat_stats(structure: &StructureState) -> CombatStats {
    CombatStats {
        hp: structure.hp,
        max_hp: structure.max_hp.max(1.0),
        mana: 0.0,
        max_mana: 1.0,
    }
}

fn default_hp() -> f32 {
    MAX_HP
}

fn default_team() -> Team {
    Team::Green
}

fn default_max_hp() -> f32 {
    MAX_HP
}

fn default_mana() -> f32 {
    MAX_MANA
}

fn default_max_mana() -> f32 {
    MAX_MANA
}
