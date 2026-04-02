use bevy::ecs::query::Or;
use bevy::ecs::system::SystemParam;
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

use crate::camera::{CameraState, MainCamera, locked_camera_offset};
use crate::combat::{CombatStats, MAX_HP, MAX_MANA};
use crate::debug_console::DebugConsole;
use crate::player::{PLAYER_SIZE, Player, PlayerBody, VerticalVelocity};
use crate::team::TeamSelection;
use crate::team::{CharacterChoice, Team};
use crate::world::{
    NormalizeModelScale, PlayerAssets, PlayerModelCatalog, model_assets_for_choice,
};

const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:4000";
const LOCAL_BIND_ADDR: &str = "0.0.0.0:0";
const UPDATE_INTERVAL_SECONDS: f32 = 0.05;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const NETWORK_LOOP_SLEEP: Duration = Duration::from_millis(16);
const MAX_PACKET_SIZE: usize = 8 * 1024;
const PROJECTILE_RADIUS: f32 = 0.22;
const TOWER_SIZE: f32 = 2.6;
const TOWER_HEIGHT: f32 = 6.0;
const BASE_TOWER_SIZE: f32 = 6.0;
const BASE_TOWER_HEIGHT: f32 = 8.0;
const MINION_RADIUS: f32 = 0.55;
const NEUTRAL_RADIUS: f32 = 0.62;
const LOCAL_SNAP_DISTANCE: f32 = 4.0;
const DEFAULT_PLAYER_LEVEL: u32 = 1;
const DEFAULT_NEXT_LEVEL_XP: u32 = 120;

/// Latest skill-3 fields from the server snapshot (local hero), for HUD prechecks and tooltips.
#[derive(Resource, Clone, Copy, Debug)]
pub struct LocalSkill3Hud {
    pub rank: u8,
    pub cooldown_remaining_secs: f32,
}

impl Default for LocalSkill3Hud {
    fn default() -> Self {
        Self {
            rank: 1,
            cooldown_remaining_secs: 0.0,
        }
    }
}

pub struct NetworkingPlugin;

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NetworkCommand>()
            .init_resource::<NetworkState>()
            .init_resource::<GameStateSnapshot>()
            .init_resource::<LocalSkill3Hud>()
            .init_resource::<QueuedServerSnapshot>()
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
                    drain_server_snapshots,
                    apply_server_snapshot,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                interpolate_snapshot_entities.after(apply_server_snapshot),
            )
            .add_systems(
                Update,
                interpolate_remote_players.after(apply_server_snapshot),
            );
    }
}

#[derive(Message, Clone, Copy, Debug)]
pub enum NetworkCommand {
    Cast { target: TargetId },
    CastSkill { skill_slot: u8 },
    Join { team: Team, character: CharacterChoice },
    #[allow(dead_code)]
    RequestRematch,
}

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
    CastSkill {
        skill_slot: u8,
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
pub enum TargetKind {
    Player,
    Minion,
    Structure,
    Neutral,
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
    #[serde(default)]
    gold: u32,
    #[serde(default)]
    xp: u32,
    #[serde(default = "default_player_level")]
    level: u32,
    #[serde(default = "default_next_level_xp")]
    next_level_xp: u32,
    #[serde(default)]
    skill_points: u32,
    #[serde(default = "default_skill3_rank")]
    skill3_rank: u8,
    #[serde(default)]
    skill3_cooldown_remaining_secs: f32,
    #[serde(default = "default_character_choice")]
    character: CharacterChoice,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Top,
    Mid,
    Bot,
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
    #[serde(default = "default_minion_brain_state")]
    state: MinionBrainState,
    #[serde(default)]
    target_kind: Option<MinionTargetKind>,
    #[serde(default)]
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
pub enum NeutralCampType {
    Skirmisher,
    Bruiser,
    Spitter,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NeutralAiState {
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
        minions: Vec<MinionState>,
        #[serde(default)]
        neutrals: Vec<NeutralState>,
        #[serde(default)]
        game_state: GameState,
        #[serde(default)]
        rematch_in_secs: Option<u64>,
        #[serde(default)]
        your_skill_feedback: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameState {
    #[default]
    Lobby,
    Running,
    Victory { winner: Team },
}

#[derive(Resource, Default, Clone)]
pub struct GameStateSnapshot {
    pub state: GameState,
    pub rematch_in_secs: Option<u64>,
}

#[derive(Resource)]
struct NetworkChannels {
    outgoing: Sender<ClientPacket>,
    incoming: Receiver<ServerPacket>,
}

/// Latest snapshot drained from UDP; applied on the same frame by `apply_server_snapshot`.
#[derive(Resource, Default)]
struct QueuedServerSnapshot {
    latest: Option<ServerSnapshotPayload>,
}

struct ServerSnapshotPayload {
    your_id: u64,
    players: Vec<PlayerState>,
    projectiles: Vec<ProjectileState>,
    structures: Vec<StructureState>,
    minions: Vec<MinionState>,
    neutrals: Vec<NeutralState>,
    game_state: GameState,
    rematch_in_secs: Option<u64>,
    your_skill_feedback: Option<String>,
}

/// Bundles stale-entity checks so `apply_server_snapshot` stays within Bevy's system param limit.
#[derive(SystemParam)]
struct SnapshotStaleQueries<'w, 's> {
    remote: Query<'w, 's, &'static RemotePlayer>,
    projectile: Query<'w, 's, &'static NetworkProjectile>,
    structure: Query<'w, 's, &'static NetworkStructure>,
    minion: Query<'w, 's, &'static NetworkMinion>,
    neutral: Query<'w, 's, &'static NetworkNeutral>,
}

#[derive(Resource, Default)]
struct NetworkState {
    local_id: Option<u64>,
    local_team: Option<Team>,
    remote_players: HashMap<u64, Entity>,
    projectiles: HashMap<u64, Entity>,
    structures: HashMap<u64, Entity>,
    minions: HashMap<u64, Entity>,
    neutrals: HashMap<u64, Entity>,
}

#[derive(Resource)]
struct LocalStateSendTimer(Timer);

#[derive(Component)]
pub struct RemotePlayer;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkPlayerId(pub u64);

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkCharacterChoice(pub CharacterChoice);

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlayerProgression {
    pub level: u32,
    pub xp: u32,
    pub next_level_xp: u32,
    pub skill_points: u32,
}

#[derive(Component)]
struct NetworkProjectile;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkStructureId(pub u64);

#[derive(Component)]
pub struct NetworkStructure;

#[derive(Component)]
pub struct NetworkMinion;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkMinionId(pub u64);

#[derive(Component)]
pub struct NetworkNeutral;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkNeutralId(pub u64);

#[derive(Component, Clone, Copy, Debug)]
struct NetEntityInterpolation {
    from_translation: Vec3,
    to_translation: Vec3,
    from_rotation: Quat,
    to_rotation: Quat,
    elapsed: f32,
    duration: f32,
}

#[derive(Component, Clone, Copy, Debug)]
struct RemotePlayerInterpolation {
    from_translation: Vec3,
    to_translation: Vec3,
    from_rotation: Quat,
    to_rotation: Quat,
    elapsed: f32,
    duration: f32,
}

#[derive(Resource)]
struct NetworkVisualAssets {
    projectile_mesh: Handle<Mesh>,
    friendly_projectile_material: Handle<StandardMaterial>,
    hostile_projectile_material: Handle<StandardMaterial>,
    tower_mesh: Handle<Mesh>,
    base_tower_mesh: Handle<Mesh>,
    minion_mesh: Handle<Mesh>,
    green_structure_material: Handle<StandardMaterial>,
    blue_structure_material: Handle<StandardMaterial>,
    green_minion_material: Handle<StandardMaterial>,
    blue_minion_material: Handle<StandardMaterial>,
    neutral_mesh: Handle<Mesh>,
    neutral_material: Handle<StandardMaterial>,
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
    let base_tower_mesh = meshes.add(Mesh::from(Cuboid::new(
        BASE_TOWER_SIZE,
        BASE_TOWER_HEIGHT,
        BASE_TOWER_SIZE,
    )));
    let minion_mesh = meshes.add(Mesh::from(Sphere::new(MINION_RADIUS)));
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
    let green_minion_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.24, 0.72, 0.32),
        perceptual_roughness: 0.75,
        ..default()
    });
    let blue_minion_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.32, 0.48, 0.92),
        perceptual_roughness: 0.75,
        ..default()
    });
    let neutral_mesh = meshes.add(Mesh::from(Sphere::new(NEUTRAL_RADIUS)));
    let neutral_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.38, 0.18),
        perceptual_roughness: 0.8,
        ..default()
    });

    commands.insert_resource(NetworkVisualAssets {
        projectile_mesh,
        friendly_projectile_material,
        hostile_projectile_material,
        tower_mesh,
        base_tower_mesh,
        minion_mesh,
        green_structure_material,
        blue_structure_material,
        green_minion_material,
        blue_minion_material,
        neutral_mesh,
        neutral_material,
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
    println!("Connecting to server at {server_addr}");
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
    println!("UDP socket connected to {server_addr}; waiting for first snapshot");

    let mut recv_buf = [0_u8; MAX_PACKET_SIZE];
    let mut last_heartbeat_at = Instant::now();
    let mut last_receive_error_log_at: Option<Instant> = None;
    let mut first_snapshot_received = false;

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
                        if !first_snapshot_received {
                            println!(
                                "First snapshot received from {server_addr}; connection is live"
                            );
                            first_snapshot_received = true;
                        }
                        let _ = incoming.send(packet);
                    }
                    Err(error) => {
                        eprintln!("Failed to decode server packet: {error}");
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    let now = Instant::now();
                    if last_receive_error_log_at
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
                    {
                        eprintln!("Client socket receive error: {error}");
                        last_receive_error_log_at = Some(now);
                    }
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

    let Ok(player_transform) = player_query.single() else {
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
    mut command_events: MessageReader<NetworkCommand>,
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
            NetworkCommand::CastSkill { skill_slot } => {
                let _ = channels.outgoing.send(ClientPacket::CastSkill {
                    skill_slot: *skill_slot,
                });
            }
            NetworkCommand::Join { team, character } => {
                let _ = channels.outgoing.send(ClientPacket::Join {
                    team: *team,
                    character: *character,
                });
            }
            NetworkCommand::RequestRematch => {
                let _ = channels.outgoing.send(ClientPacket::RequestRematch);
            }
        }
    }
}

fn choose_authoritative_local_player<T: Copy + Eq>(
    candidates: &[(T, Option<u64>)],
    your_id: u64,
) -> Option<T> {
    let mut chosen = None;
    for (entity, maybe_id) in candidates {
        if maybe_id.is_some_and(|id| id == your_id) {
            return Some(*entity);
        }
        chosen.get_or_insert(*entity);
    }
    chosen
}

fn drain_server_snapshots(
    channels: Option<Res<NetworkChannels>>,
    mut queued: ResMut<QueuedServerSnapshot>,
) {
    let Some(channels) = channels else {
        return;
    };

    while let Ok(packet) = channels.incoming.try_recv() {
        let ServerPacket::Snapshot {
            your_id,
            players,
            projectiles,
            structures,
            minions,
            neutrals,
            game_state,
            rematch_in_secs,
            your_skill_feedback,
        } = packet;
        queued.latest = Some(ServerSnapshotPayload {
            your_id,
            players,
            projectiles,
            structures,
            minions,
            neutrals,
            game_state,
            rematch_in_secs,
            your_skill_feedback,
        });
    }
}

fn apply_server_snapshot(
    mut commands: Commands,
    mut queued: ResMut<QueuedServerSnapshot>,
    mut console: ResMut<DebugConsole>,
    mut network_state: ResMut<NetworkState>,
    mut transform_sets: ParamSet<(
        Query<&mut Transform>,
        Query<&mut Transform, With<MainCamera>>,
    )>,
    stale_queries: SnapshotStaleQueries,
    local_player_query: Query<(Entity, Option<&NetworkPlayerId>), With<Player>>,
    player_assets: Res<PlayerAssets>,
    model_catalog: Res<PlayerModelCatalog>,
    visuals: Res<NetworkVisualAssets>,
    mut game_state_snapshot: ResMut<GameStateSnapshot>,
    mut cam_state: ResMut<CameraState>,
    team_selection: Res<TeamSelection>,
) {
    let Some(ServerSnapshotPayload {
        your_id,
        players,
        projectiles,
        structures,
        minions,
        neutrals,
        game_state,
        rematch_in_secs,
        your_skill_feedback,
    }) = queued.latest.take()
    else {
        return;
    };

    if let Some(msg) = your_skill_feedback {
        console.push_line(msg.clone());
        info!("{msg}");
    }

    network_state.local_id = Some(your_id);
    game_state_snapshot.state = game_state;
    game_state_snapshot.rematch_in_secs = rematch_in_secs;

    let local_player_state = players.iter().find(|player| player.id == your_id);
    let local_players = local_player_query
        .iter()
        .map(|(entity, maybe_id)| (entity, maybe_id.map(|id| id.0)))
        .collect::<Vec<_>>();
    // IMPORTANT: we must tolerate temporary duplication of `Player` entities (e.g. during loading /
    // restart races). Many gameplay systems use `Query::single()` and will break if we allow >1.
    let chosen_local =
        choose_authoritative_local_player(&local_players, your_id);

    if let Some(local_entity) = chosen_local {
        // Keep exactly one local `Player` alive to avoid `single()` query failures.
        for &(entity, maybe_id) in &local_players {
            if entity == local_entity {
                continue;
            }
            // If this extra player happens to have our id, prefer the chosen one anyway and despawn.
            if maybe_id.is_some_and(|id| id == your_id) {
                warn!("Found duplicate local Player for id={your_id}; despawning extra entity");
            }
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }

        // Always ensure the local player is tagged with the server-provided id.
        commands
            .entity(local_entity)
            .insert(NetworkPlayerId(your_id));

        // Only apply character/team/stats once we actually have a state entry for our id.
        // Otherwise we'd oscillate between the locally selected character and the server default.
        if let Some(local_player_state) = local_player_state {
            commands.insert_resource(LocalSkill3Hud {
                rank: local_player_state.skill3_rank,
                cooldown_remaining_secs: local_player_state.skill3_cooldown_remaining_secs,
            });
            commands.entity(local_entity).insert((
                local_player_state.team,
                player_state_to_combat_stats(local_player_state),
                NetworkCharacterChoice(local_player_state.character),
                player_state_to_progression(local_player_state),
            ));
            network_state.local_team = Some(local_player_state.team);

            let server_translation = Vec3::new(
                local_player_state.x,
                local_player_state.y,
                local_player_state.z,
            );
            if let Ok(mut local_transform) = transform_sets.p0().get_mut(local_entity) {
                // Snap on meaningful server corrections (first team spawn, respawn, etc.)
                if local_transform
                    .translation
                    .distance_squared(server_translation)
                    > LOCAL_SNAP_DISTANCE * LOCAL_SNAP_DISTANCE
                {
                    local_transform.translation = server_translation;
                    local_transform.rotation = Quat::from_rotation_y(local_player_state.yaw);
                }
            }
        }
    } else if let Some(local_player_state) = local_player_state {
        // Wait for server ack of selected team to avoid first spawn on default Green.
        let Some(selected_team) = team_selection.team else {
            return;
        };
        if local_player_state.team != selected_team {
            return;
        }
        let spawn = Vec3::new(
            local_player_state.x,
            local_player_state.y,
            local_player_state.z,
        );
        let (local_scene, _local_gltf) =
            model_assets_for_choice(&model_catalog, local_player_state.character);
        let entity = if let Some(scene_handle) = local_scene {
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
                    NormalizeModelScale::for_player_model(),
                    NetworkPlayerId(your_id),
                    NetworkCharacterChoice(local_player_state.character),
                    player_state_to_combat_stats(local_player_state),
                    player_state_to_progression(local_player_state),
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
                    NetworkCharacterChoice(local_player_state.character),
                    player_state_to_combat_stats(local_player_state),
                    player_state_to_progression(local_player_state),
                    Name::new("Player"),
                ))
                .id()
        };

        network_state.local_team = Some(local_player_state.team);
        commands.insert_resource(LocalSkill3Hud {
            rank: local_player_state.skill3_rank,
            cooldown_remaining_secs: local_player_state.skill3_cooldown_remaining_secs,
        });
        if let Ok(mut camera_transform) = transform_sets.p1().single_mut() {
            cam_state.locked = true;
            let zoom = cam_state.zoom;
            camera_transform.translation = spawn + locked_camera_offset(zoom);
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
            if let Ok(transform) = transform_sets.p0().get_mut(entity) {
                let interpolation = RemotePlayerInterpolation {
                    from_translation: transform.translation,
                    to_translation: Vec3::new(player.x, player.y, player.z),
                    from_rotation: transform.rotation,
                    to_rotation: Quat::from_rotation_y(player.yaw),
                    elapsed: 0.0,
                    duration: UPDATE_INTERVAL_SECONDS.max(0.001),
                };
                commands.entity(entity).insert(interpolation);
            }
            commands.entity(entity).insert((
                NetworkPlayerId(player.id),
                player.team,
                NetworkCharacterChoice(player.character),
                player_state_to_combat_stats(player),
                player_state_to_progression(player),
            ));
            continue;
        }

        let (scene_handle, _gltf_handle) =
            model_assets_for_choice(&model_catalog, player.character);
        let mesh_handle = player_assets.mesh.clone();
        let material_handle = player_assets.material.clone();
        let spawn_translation = Vec3::new(player.x, player.y, player.z);
        let spawn_rotation = Quat::from_rotation_y(player.yaw);
        let mut entity_commands = commands.spawn((
            Transform::from_translation(spawn_translation).with_rotation(spawn_rotation),
            Visibility::default(),
            RemotePlayer,
            PlayerBody,
            player.team,
            NetworkPlayerId(player.id),
            NetworkCharacterChoice(player.character),
            NormalizeModelScale::for_player_model(),
            player_state_to_combat_stats(player),
            player_state_to_progression(player),
            RemotePlayerInterpolation {
                from_translation: spawn_translation,
                to_translation: spawn_translation,
                from_rotation: spawn_rotation,
                to_rotation: spawn_rotation,
                elapsed: UPDATE_INTERVAL_SECONDS,
                duration: UPDATE_INTERVAL_SECONDS.max(0.001),
            },
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
            if stale_queries.remote.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
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
            if stale_queries.projectile.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
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
            StructureKind::BaseTower => visuals.base_tower_mesh.clone(),
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
            if stale_queries.structure.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }

    let mut seen_minion_ids = HashSet::new();
    for minion in &minions {
        seen_minion_ids.insert(minion.id);
        let target_translation = Vec3::new(minion.x, minion.y, minion.z);
        let target_rotation = Quat::from_rotation_y(minion.yaw);

        if let Some(entity) = network_state.minions.get(&minion.id).copied() {
            if let Ok(transform) = transform_sets.p0().get_mut(entity) {
                let interpolation = NetEntityInterpolation {
                    from_translation: transform.translation,
                    to_translation: target_translation,
                    from_rotation: transform.rotation,
                    to_rotation: target_rotation,
                    elapsed: 0.0,
                    duration: UPDATE_INTERVAL_SECONDS.max(0.001),
                };
                commands.entity(entity).insert(interpolation);
            }
            commands.entity(entity).insert((
                NetworkMinionId(minion.id),
                minion.team,
                minion_state_to_combat_stats(minion),
            ));
            continue;
        }

        let material = match minion.team {
            Team::Green => visuals.green_minion_material.clone(),
            Team::Blue => visuals.blue_minion_material.clone(),
        };

        let entity = commands
            .spawn((
                Mesh3d(visuals.minion_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(target_translation).with_rotation(target_rotation),
                Visibility::default(),
                NetworkMinion,
                NetworkMinionId(minion.id),
                NetEntityInterpolation {
                    from_translation: target_translation,
                    to_translation: target_translation,
                    from_rotation: target_rotation,
                    to_rotation: target_rotation,
                    elapsed: UPDATE_INTERVAL_SECONDS,
                    duration: UPDATE_INTERVAL_SECONDS.max(0.001),
                },
                minion.team,
                minion_state_to_combat_stats(minion),
                Name::new(format!("Minion-{}-{:?}", minion.id, minion.lane)),
            ))
            .id();
        network_state.minions.insert(minion.id, entity);
    }

    let mut seen_neutral_ids = HashSet::new();
    for neutral in &neutrals {
        seen_neutral_ids.insert(neutral.id);
        let target_translation = Vec3::new(neutral.x, neutral.y, neutral.z);
        let target_rotation = Quat::from_rotation_y(neutral.yaw);

        if let Some(entity) = network_state.neutrals.get(&neutral.id).copied() {
            if let Ok(transform) = transform_sets.p0().get_mut(entity) {
                let interpolation = NetEntityInterpolation {
                    from_translation: transform.translation,
                    to_translation: target_translation,
                    from_rotation: transform.rotation,
                    to_rotation: target_rotation,
                    elapsed: 0.0,
                    duration: UPDATE_INTERVAL_SECONDS.max(0.001),
                };
                commands.entity(entity).insert(interpolation);
            }
            commands.entity(entity).insert((
                NetworkNeutralId(neutral.id),
                neutral_state_to_combat_stats(neutral),
            ));
            continue;
        }

        let entity = commands
            .spawn((
                Mesh3d(visuals.neutral_mesh.clone()),
                MeshMaterial3d(visuals.neutral_material.clone()),
                Transform::from_translation(target_translation).with_rotation(target_rotation),
                Visibility::default(),
                NetworkNeutral,
                NetworkNeutralId(neutral.id),
                NetEntityInterpolation {
                    from_translation: target_translation,
                    to_translation: target_translation,
                    from_rotation: target_rotation,
                    to_rotation: target_rotation,
                    elapsed: UPDATE_INTERVAL_SECONDS,
                    duration: UPDATE_INTERVAL_SECONDS.max(0.001),
                },
                neutral_state_to_combat_stats(neutral),
                Name::new(format!("Neutral-{}", neutral.id)),
            ))
            .id();
        network_state.neutrals.insert(neutral.id, entity);
    }

    let stale_neutral_ids = network_state
        .neutrals
        .keys()
        .copied()
        .filter(|id| !seen_neutral_ids.contains(id))
        .collect::<Vec<_>>();
    for neutral_id in stale_neutral_ids {
        if let Some(entity) = network_state.neutrals.remove(&neutral_id) {
            if stale_queries.neutral.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }

    let stale_minion_ids = network_state
        .minions
        .keys()
        .copied()
        .filter(|id| !seen_minion_ids.contains(id))
        .collect::<Vec<_>>();
    for minion_id in stale_minion_ids {
        if let Some(entity) = network_state.minions.remove(&minion_id) {
            if stale_queries.minion.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }
}

fn interpolate_snapshot_entities(
    time: Res<Time>,
    mut entity_query: Query<
        (&mut Transform, &mut NetEntityInterpolation),
        Or<(With<NetworkMinion>, With<NetworkNeutral>)>,
    >,
) {
    for (mut transform, mut interpolation) in &mut entity_query {
        let duration = interpolation.duration.max(0.001);
        interpolation.elapsed = (interpolation.elapsed + time.delta_secs()).min(duration);
        let t = (interpolation.elapsed / duration).clamp(0.0, 1.0);
        transform.translation = interpolation
            .from_translation
            .lerp(interpolation.to_translation, t);
        transform.rotation = interpolation
            .from_rotation
            .slerp(interpolation.to_rotation, t);
    }
}

fn interpolate_remote_players(
    time: Res<Time>,
    mut player_query: Query<(&mut Transform, &mut RemotePlayerInterpolation), With<RemotePlayer>>,
) {
    for (mut transform, mut interpolation) in &mut player_query {
        let duration = interpolation.duration.max(0.001);
        interpolation.elapsed = (interpolation.elapsed + time.delta_secs()).min(duration);
        let t = (interpolation.elapsed / duration).clamp(0.0, 1.0);
        transform.translation = interpolation
            .from_translation
            .lerp(interpolation.to_translation, t);
        transform.rotation = interpolation
            .from_rotation
            .slerp(interpolation.to_rotation, t);
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

fn player_state_to_progression(player: &PlayerState) -> PlayerProgression {
    PlayerProgression {
        level: player.level.max(1),
        xp: player.xp,
        next_level_xp: player.next_level_xp,
        skill_points: player.skill_points,
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

fn minion_state_to_combat_stats(minion: &MinionState) -> CombatStats {
    CombatStats {
        hp: minion.hp,
        max_hp: minion.max_hp.max(1.0),
        mana: 0.0,
        max_mana: 1.0,
    }
}

fn neutral_state_to_combat_stats(neutral: &NeutralState) -> CombatStats {
    CombatStats {
        hp: neutral.hp,
        max_hp: neutral.max_hp.max(1.0),
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

fn default_character_choice() -> CharacterChoice {
    CharacterChoice::Ipfs
}

fn default_player_level() -> u32 {
    DEFAULT_PLAYER_LEVEL
}

fn default_next_level_xp() -> u32 {
    DEFAULT_NEXT_LEVEL_XP
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

fn default_skill3_rank() -> u8 {
    1
}

fn default_minion_brain_state() -> MinionBrainState {
    MinionBrainState::Marching
}

#[cfg(test)]
mod tests {
    use super::choose_authoritative_local_player;

    #[test]
    fn choose_authoritative_local_player_prefers_matching_network_id() {
        let candidates = [(1_u32, None), (2_u32, Some(77)), (3_u32, Some(13))];

        let chosen = choose_authoritative_local_player(&candidates, 77);

        assert_eq!(chosen, Some(2));
    }

    #[test]
    fn choose_authoritative_local_player_falls_back_to_first_candidate() {
        let candidates = [(11_u32, None), (12_u32, Some(7)), (13_u32, None)];

        let chosen = choose_authoritative_local_player(&candidates, 99);

        assert_eq!(chosen, Some(11));
    }

    #[test]
    fn choose_authoritative_local_player_returns_none_when_empty() {
        let candidates: [(u32, Option<u64>); 0] = [];

        let chosen = choose_authoritative_local_player(&candidates, 1);

        assert_eq!(chosen, None);
    }
}
