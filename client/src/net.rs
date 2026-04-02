use bevy::ecs::query::Or;
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
use crate::persistence::{FileGameServerAddr, ResolvedServerAddressForPrefs};
use crate::player::{PLAYER_SIZE, Player, PlayerBody, VerticalVelocity};
use crate::session_config::{
    DEFAULT_GAME_SERVER_ADDR, T_RETRY, T_STALE_SNAPSHOT, T_WAIT_MAX,
    TRANSPORT_CONSECUTIVE_RECV_ERRORS, TRANSPORT_CONSECUTIVE_SEND_ERRORS, is_stale,
};
use crate::team::TeamSelection;
use crate::team::{CharacterChoice, Team, TeamSelectRoot, spawn_team_select_ui};
use crate::world::{
    NormalizeModelScale, PlayerAssets, PlayerModelCatalog, model_assets_for_choice,
};

const LOCAL_BIND_ADDR: &str = "0.0.0.0:0";
const UPDATE_INTERVAL_SECONDS: f32 = 0.05;
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

/// High-level client session / transport state (TASK-14 frozen connection states).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClientConnectionState {
    /// Binding socket / spawning I/O thread (single-frame or brief).
    #[default]
    Connecting,
    /// Transport up; no qualifying snapshot yet for this connect attempt.
    WaitingForServer,
    /// At least one qualifying snapshot applied; `your_id` is known.
    Connected,
    /// Session not live; user must use Retry or pick team again as documented.
    Disconnected,
}

/// Signal from the UDP thread to the Bevy main thread (failure detection §3 in spec).
#[derive(Debug, Clone, Copy)]
pub enum NetThreadSignal {
    /// Recv/send error streak exceeded fixed thresholds (P3 transport rule).
    TransportFailure,
}

#[derive(Message, Clone, Copy, Debug)]
pub enum SessionUiCommand {
    /// User explicitly resumes waiting for snapshots after **Disconnected** (P2 manual recovery).
    Retry,
}

/// Set by [`ingest_server_snapshot_packets`] when the UDP thread dropped the snapshot sender
/// (failure detection §3: channel implies session ended).
#[derive(Resource, Default)]
pub struct NetIncomingDisconnected(pub bool);

/// Session controller: owns lifecycle flags and join idempotency (single ownership vs UI/net).
#[derive(Resource)]
pub struct ClientSession {
    pub state: ClientConnectionState,
    /// Wall time when **WaitingForServer** began for the current attempt.
    pub waiting_since: Option<Instant>,
    /// Last time a qualifying snapshot was applied while **Connected**.
    pub last_qualifying_snapshot_wall: Option<Instant>,
    /// While true, incoming snapshots are drained and ignored (no silent re-entry to gameplay).
    pub discard_incoming_snapshots: bool,
    /// **P4**: after a join packet is sent, suppress duplicate join until teardown.
    pub join_flow_committed: bool,
    /// Server address used for this process (for UI copy).
    pub server_addr_display: String,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            state: ClientConnectionState::Connecting,
            waiting_since: None,
            last_qualifying_snapshot_wall: None,
            discard_incoming_snapshots: false,
            join_flow_committed: false,
            server_addr_display: String::new(),
        }
    }
}

impl ClientSession {
    pub fn is_connected(&self) -> bool {
        self.state == ClientConnectionState::Connected
    }
}

pub struct NetworkingPlugin;

/// Strict main-thread ordering for networking / snapshot / UI sync (Bevy 0.18: avoid `.after(fn)`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum ClientNetPipeline {
    SendLocalState,
    SendCommands,
    IngestSnapshot,
    ApplySnapshot,
    InterpolateNetEntities,
    InterpolateRemotePlayers,
    SessionRetryInput,
    SessionLifecycle,
    SyncConnectionUi,
}

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .init_resource::<NetworkState>()
            .init_resource::<GameStateSnapshot>()
            .init_resource::<PendingServerSnapshotFrame>()
            .init_resource::<ClientSession>()
            .init_resource::<NetIncomingDisconnected>()
            .insert_resource(LocalStateSendTimer(Timer::from_seconds(
                UPDATE_INTERVAL_SECONDS,
                TimerMode::Repeating,
            )))
            .add_systems(
                Startup,
                (
                    setup_network_visual_assets,
                    start_networking.after(crate::persistence::load_persistent_client_settings),
                    setup_connection_status_ui,
                ),
            )
            .configure_sets(
                Update,
                (
                    ClientNetPipeline::SendCommands.after(ClientNetPipeline::SendLocalState),
                    ClientNetPipeline::IngestSnapshot.after(ClientNetPipeline::SendCommands),
                    ClientNetPipeline::ApplySnapshot.after(ClientNetPipeline::IngestSnapshot),
                    ClientNetPipeline::InterpolateNetEntities
                        .after(ClientNetPipeline::ApplySnapshot),
                    ClientNetPipeline::InterpolateRemotePlayers
                        .after(ClientNetPipeline::InterpolateNetEntities),
                    ClientNetPipeline::SessionRetryInput
                        .after(ClientNetPipeline::InterpolateRemotePlayers),
                    ClientNetPipeline::SessionLifecycle.after(ClientNetPipeline::SessionRetryInput),
                    ClientNetPipeline::SyncConnectionUi.after(ClientNetPipeline::SessionLifecycle),
                ),
            )
            .add_systems(
                Update,
                send_local_state.in_set(ClientNetPipeline::SendLocalState),
            )
            .add_systems(
                Update,
                send_network_commands.in_set(ClientNetPipeline::SendCommands),
            )
            .add_systems(
                Update,
                ingest_server_snapshot_packets.in_set(ClientNetPipeline::IngestSnapshot),
            )
            .add_systems(
                Update,
                apply_server_snapshot.in_set(ClientNetPipeline::ApplySnapshot),
            )
            .add_systems(
                Update,
                interpolate_snapshot_entities.in_set(ClientNetPipeline::InterpolateNetEntities),
            )
            .add_systems(
                Update,
                interpolate_remote_players.in_set(ClientNetPipeline::InterpolateRemotePlayers),
            )
            .add_systems(
                Update,
                handle_connection_retry_button.in_set(ClientNetPipeline::SessionRetryInput),
            )
            .add_systems(
                Update,
                update_session_lifecycle.in_set(ClientNetPipeline::SessionLifecycle),
            )
            .add_systems(
                Update,
                sync_connection_status_ui.in_set(ClientNetPipeline::SyncConnectionUi),
            );
    }
}

#[derive(Message, Clone, Copy, Debug)]
pub enum NetworkCommand {
    Cast {
        target: TargetId,
    },
    Join {
        team: Team,
        character: CharacterChoice,
    },
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
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameState {
    #[default]
    Lobby,
    Running,
    Victory {
        winner: Team,
    },
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
    signals: Receiver<NetThreadSignal>,
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

/// Latest drained snapshot for this frame (filled by [`ingest_server_snapshot_packets`]).
#[derive(Resource, Default)]
struct PendingServerSnapshotFrame {
    frame: Option<PendingSnapshotData>,
}

struct PendingSnapshotData {
    wall_time: Instant,
    your_id: u64,
    players: Vec<PlayerState>,
    projectiles: Vec<ProjectileState>,
    structures: Vec<StructureState>,
    minions: Vec<MinionState>,
    neutrals: Vec<NeutralState>,
    game_state: GameState,
    rematch_in_secs: Option<u64>,
    /// Local team choice at ingest time (spawn gate when server has not yet mirrored selection).
    selected_team_for_spawn: Option<Team>,
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

fn start_networking(
    mut commands: Commands,
    mut client_session: ResMut<ClientSession>,
    file_addr: Res<FileGameServerAddr>,
) {
    let preferred_addr = std::env::var("GAME_SERVER_ADDR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| file_addr.0.clone())
        .unwrap_or_else(|| DEFAULT_GAME_SERVER_ADDR.to_owned());
    let server_addr = validated_server_addr_or_default(&preferred_addr);
    if server_addr != preferred_addr {
        warn!(
            "Ignoring invalid GAME_SERVER_ADDR/file value {preferred_addr:?}; falling back to {server_addr}"
        );
    }
    spawn_network_transport(&mut commands, &mut client_session, server_addr);
}

fn validated_server_addr_or_default(raw: &str) -> String {
    crate::persistence::validate_game_server_addr(raw)
        .or_else(|| crate::persistence::validate_game_server_addr(DEFAULT_GAME_SERVER_ADDR))
        .expect("default game server address must validate")
}

fn spawn_network_transport(
    commands: &mut Commands,
    client_session: &mut ClientSession,
    server_addr: String,
) {
    client_session.server_addr_display.clone_from(&server_addr);
    commands.insert_resource(ResolvedServerAddressForPrefs(server_addr.clone()));
    let (outgoing_tx, outgoing_rx) = unbounded::<ClientPacket>();
    let (incoming_tx, incoming_rx) = unbounded::<ServerPacket>();
    let (signal_tx, signal_rx) = unbounded::<NetThreadSignal>();

    let addr_for_thread = server_addr;
    thread::spawn(move || {
        run_udp_client(addr_for_thread, outgoing_rx, incoming_tx, signal_tx);
    });

    client_session.state = ClientConnectionState::WaitingForServer;
    client_session.waiting_since = Some(Instant::now());
    client_session.discard_incoming_snapshots = false;
    client_session.join_flow_committed = false;
    client_session.last_qualifying_snapshot_wall = None;

    commands.insert_resource(NetworkChannels {
        outgoing: outgoing_tx,
        incoming: incoming_rx,
        signals: signal_rx,
    });
}

fn run_udp_client(
    server_addr: String,
    outgoing: Receiver<ClientPacket>,
    incoming: Sender<ServerPacket>,
    signals: Sender<NetThreadSignal>,
) {
    println!("Connecting to server at {server_addr}");
    let socket = match UdpSocket::bind(LOCAL_BIND_ADDR) {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("Failed to bind client UDP socket: {error}");
            let _ = signals.send(NetThreadSignal::TransportFailure);
            return;
        }
    };

    if let Err(error) = socket.connect(&server_addr) {
        eprintln!("Failed to connect UDP socket to {server_addr}: {error}");
        let _ = signals.send(NetThreadSignal::TransportFailure);
        return;
    }
    if let Err(error) = socket.set_nonblocking(true) {
        eprintln!("Failed to set UDP client socket nonblocking: {error}");
        let _ = signals.send(NetThreadSignal::TransportFailure);
        return;
    }
    println!("UDP socket connected to {server_addr}; waiting for first snapshot");

    let mut recv_buf = [0_u8; MAX_PACKET_SIZE];
    let mut last_heartbeat_at = Instant::now();
    let mut last_receive_error_log_at: Option<Instant> = None;
    let mut first_snapshot_received = false;
    let mut consecutive_recv_errors: u32 = 0;
    let mut consecutive_send_errors: u32 = 0;
    let mut transport_failure_reported = false;

    let _ = udp_try_send(
        &socket,
        &ClientPacket::Ping,
        &mut consecutive_send_errors,
        &mut transport_failure_reported,
        &signals,
    );

    loop {
        loop {
            match outgoing.try_recv() {
                Ok(packet) => {
                    udp_try_send(
                        &socket,
                        &packet,
                        &mut consecutive_send_errors,
                        &mut transport_failure_reported,
                        &signals,
                    );
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        if last_heartbeat_at.elapsed() >= T_RETRY {
            udp_try_send(
                &socket,
                &ClientPacket::Ping,
                &mut consecutive_send_errors,
                &mut transport_failure_reported,
                &signals,
            );
            last_heartbeat_at = Instant::now();
        }

        loop {
            match socket.recv(&mut recv_buf) {
                Ok(len) => {
                    consecutive_recv_errors = 0;
                    match serde_json::from_slice::<ServerPacket>(&recv_buf[..len]) {
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
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    consecutive_recv_errors = consecutive_recv_errors.saturating_add(1);
                    let now = Instant::now();
                    if last_receive_error_log_at
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
                    {
                        eprintln!("Client socket receive error: {error}");
                        last_receive_error_log_at = Some(now);
                    }
                    if consecutive_recv_errors >= TRANSPORT_CONSECUTIVE_RECV_ERRORS
                        && !transport_failure_reported
                    {
                        transport_failure_reported = true;
                        let _ = signals.send(NetThreadSignal::TransportFailure);
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

fn udp_try_send(
    socket: &UdpSocket,
    packet: &ClientPacket,
    consecutive_send_errors: &mut u32,
    transport_failure_reported: &mut bool,
    signals: &Sender<NetThreadSignal>,
) -> bool {
    match send_packet(socket, packet) {
        Ok(()) => {
            *consecutive_send_errors = 0;
            true
        }
        Err(error) => {
            *consecutive_send_errors = consecutive_send_errors.saturating_add(1);
            eprintln!("Client UDP send error: {error}");
            if *consecutive_send_errors >= TRANSPORT_CONSECUTIVE_SEND_ERRORS
                && !*transport_failure_reported
            {
                *transport_failure_reported = true;
                let _ = signals.send(NetThreadSignal::TransportFailure);
            }
            false
        }
    }
}

fn send_local_state(
    time: Res<Time>,
    mut timer: ResMut<LocalStateSendTimer>,
    channels: Option<Res<NetworkChannels>>,
    client_session: Res<ClientSession>,
    player_query: Query<&Transform, With<Player>>,
) {
    let Some(channels) = channels else {
        return;
    };

    if !client_session.is_connected() {
        return;
    }

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
    mut client_session: ResMut<ClientSession>,
) {
    let Some(channels) = channels else {
        return;
    };

    for command in command_events.read() {
        match command {
            NetworkCommand::Cast { target } => {
                if !client_session.is_connected() {
                    continue;
                }
                let _ = channels
                    .outgoing
                    .send(ClientPacket::Cast { target: *target });
            }
            NetworkCommand::Join { team, character } => {
                if client_session.join_flow_committed {
                    continue;
                }
                client_session.join_flow_committed = true;
                let _ = channels.outgoing.send(ClientPacket::Join {
                    team: *team,
                    character: *character,
                });
            }
            NetworkCommand::RequestRematch => {
                if !client_session.is_connected() {
                    continue;
                }
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

fn ingest_server_snapshot_packets(
    channels: Option<Res<NetworkChannels>>,
    client_session: Res<ClientSession>,
    mut pending: ResMut<PendingServerSnapshotFrame>,
    mut incoming_dead: ResMut<NetIncomingDisconnected>,
    team_selection: Res<TeamSelection>,
) {
    pending.frame = None;
    let Some(channels) = channels.as_ref() else {
        return;
    };

    if client_session.discard_incoming_snapshots {
        loop {
            match channels.incoming.try_recv() {
                Ok(_) => {}
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        return;
    }

    let mut latest_snapshot: Option<(
        u64,
        Vec<PlayerState>,
        Vec<ProjectileState>,
        Vec<StructureState>,
        Vec<MinionState>,
        Vec<NeutralState>,
        GameState,
        Option<u64>,
    )> = None;

    loop {
        match channels.incoming.try_recv() {
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                incoming_dead.0 = true;
                break;
            }
            Ok(packet) => match packet {
                ServerPacket::Snapshot {
                    your_id,
                    players,
                    projectiles,
                    structures,
                    minions,
                    neutrals,
                    game_state,
                    rematch_in_secs,
                } => {
                    latest_snapshot = Some((
                        your_id,
                        players,
                        projectiles,
                        structures,
                        minions,
                        neutrals,
                        game_state,
                        rematch_in_secs,
                    ));
                }
            },
        }
    }

    let Some((
        your_id,
        players,
        projectiles,
        structures,
        minions,
        neutrals,
        game_state,
        rematch_in_secs,
    )) = latest_snapshot
    else {
        return;
    };

    pending.frame = Some(PendingSnapshotData {
        wall_time: Instant::now(),
        your_id,
        players,
        projectiles,
        structures,
        minions,
        neutrals,
        game_state,
        rematch_in_secs,
        selected_team_for_spawn: team_selection.team,
    });
}

fn apply_server_snapshot(
    mut commands: Commands,
    mut pending: ResMut<PendingServerSnapshotFrame>,
    mut client_session: ResMut<ClientSession>,
    mut network_state: ResMut<NetworkState>,
    mut transform_sets: ParamSet<(
        Query<&mut Transform>,
        Query<&mut Transform, With<MainCamera>>,
    )>,
    remote_query: Query<&RemotePlayer>,
    projectile_query: Query<&NetworkProjectile>,
    structure_query: Query<&NetworkStructure>,
    minion_query: Query<&NetworkMinion>,
    neutral_query: Query<&NetworkNeutral>,
    local_player_query: Query<(Entity, Option<&NetworkPlayerId>), With<Player>>,
    player_assets: Res<PlayerAssets>,
    model_catalog: Res<PlayerModelCatalog>,
    visuals: Res<NetworkVisualAssets>,
    mut game_state_snapshot: ResMut<GameStateSnapshot>,
    mut cam_state: ResMut<CameraState>,
) {
    let Some(data) = pending.frame.take() else {
        return;
    };
    let PendingSnapshotData {
        wall_time: snapshot_wall_time,
        your_id,
        players,
        projectiles,
        structures,
        minions,
        neutrals,
        game_state,
        rematch_in_secs,
        selected_team_for_spawn,
    } = data;

    if client_session.state != ClientConnectionState::Connected {
        client_session.state = ClientConnectionState::Connected;
        client_session.waiting_since = None;
    }
    client_session.last_qualifying_snapshot_wall = Some(snapshot_wall_time);

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
    let chosen_local = choose_authoritative_local_player(&local_players, your_id);

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
        let Some(selected_team) = selected_team_for_spawn else {
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
            if remote_query.get(entity).is_ok() {
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
            if projectile_query.get(entity).is_ok() {
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
            if structure_query.get(entity).is_ok() {
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
            if neutral_query.get(entity).is_ok() {
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
            if minion_query.get(entity).is_ok() {
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

#[derive(Component)]
struct ConnectionStatusRoot;

#[derive(Component)]
struct ConnectionStatusLabel;

#[derive(Component)]
struct ConnectionRetryButton;

const CONNECTION_PANEL_BG: Color = Color::srgba(0.02, 0.02, 0.06, 0.72);

fn setup_connection_status_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                top: Val::Px(12.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(CONNECTION_PANEL_BG),
            ConnectionStatusRoot,
            Visibility::Visible,
            ZIndex(20),
            Name::new("ConnectionStatusPanel"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                ConnectionStatusLabel,
            ));
            parent
                .spawn((
                    Button,
                    Node {
                        width: Val::Px(120.0),
                        height: Val::Px(36.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.25, 0.42, 0.32)),
                    Visibility::Hidden,
                    ConnectionRetryButton,
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new("Retry"),
                        TextFont {
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });
        });
}

fn handle_connection_retry_button(
    interaction: Query<&Interaction, (With<ConnectionRetryButton>, Changed<Interaction>)>,
    mut writer: MessageWriter<SessionUiCommand>,
) {
    for i in &interaction {
        if *i == Interaction::Pressed {
            writer.write(SessionUiCommand::Retry);
        }
    }
}

fn sync_connection_status_ui(
    client_session: Res<ClientSession>,
    mut label_q: Query<&mut Text, With<ConnectionStatusLabel>>,
    mut retry_vis: Query<&mut Visibility, With<ConnectionRetryButton>>,
) {
    let Ok(mut text) = label_q.single_mut() else {
        return;
    };
    match client_session.state {
        ClientConnectionState::Connecting => {
            text.0 = format!("Connecting… ({})", client_session.server_addr_display);
        }
        ClientConnectionState::WaitingForServer => {
            text.0 = format!(
                "Waiting for server at {}. Start the server or check GAME_SERVER_ADDR. Retries every {}s (max wait {}s).",
                client_session.server_addr_display,
                T_RETRY.as_secs(),
                T_WAIT_MAX.as_secs()
            );
        }
        ClientConnectionState::Connected => {
            text.0 = format!("Connected ({}).", client_session.server_addr_display);
        }
        ClientConnectionState::Disconnected => {
            text.0 = "Disconnected — connection lost or timed out. Use Retry when the server is back, then choose your team again."
                .to_string();
        }
    }

    let Ok(mut retry_vis) = retry_vis.single_mut() else {
        return;
    };
    *retry_vis = if client_session.state == ClientConnectionState::Disconnected {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

fn despawn_tracked_net_entities(
    commands: &mut Commands,
    network_state: &mut NetworkState,
    remote_query: &Query<Entity, With<RemotePlayer>>,
    projectile_query: &Query<Entity, With<NetworkProjectile>>,
    structure_query: &Query<Entity, With<NetworkStructure>>,
    minion_query: &Query<Entity, With<NetworkMinion>>,
    neutral_query: &Query<Entity, With<NetworkNeutral>>,
) {
    for &entity in network_state.remote_players.values() {
        if remote_query.get(entity).is_ok() {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
    }
    network_state.remote_players.clear();

    for &entity in network_state.projectiles.values() {
        if projectile_query.get(entity).is_ok() {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
    }
    network_state.projectiles.clear();

    for &entity in network_state.structures.values() {
        if structure_query.get(entity).is_ok() {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
    }
    network_state.structures.clear();

    for &entity in network_state.minions.values() {
        if minion_query.get(entity).is_ok() {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
    }
    network_state.minions.clear();

    for &entity in network_state.neutrals.values() {
        if neutral_query.get(entity).is_ok() {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
    }
    network_state.neutrals.clear();

    network_state.local_id = None;
    network_state.local_team = None;
}

fn despawn_local_players(commands: &mut Commands, player_query: &Query<Entity, With<Player>>) {
    for entity in player_query.iter() {
        commands
            .entity(entity)
            .despawn_related::<Children>()
            .despawn();
    }
}

fn perform_network_teardown(
    commands: &mut Commands,
    client_session: &mut ClientSession,
    network_state: &mut NetworkState,
    game_state_snapshot: &mut GameStateSnapshot,
    team_selection: &mut TeamSelection,
    cam_state: &mut CameraState,
    overlay_query: &Query<Entity, With<TeamSelectRoot>>,
    remote_query: &Query<Entity, With<RemotePlayer>>,
    projectile_query: &Query<Entity, With<NetworkProjectile>>,
    structure_query: &Query<Entity, With<NetworkStructure>>,
    minion_query: &Query<Entity, With<NetworkMinion>>,
    neutral_query: &Query<Entity, With<NetworkNeutral>>,
    player_query: &Query<Entity, With<Player>>,
) {
    despawn_tracked_net_entities(
        commands,
        network_state,
        remote_query,
        projectile_query,
        structure_query,
        minion_query,
        neutral_query,
    );
    despawn_local_players(commands, player_query);

    *game_state_snapshot = GameStateSnapshot::default();
    cam_state.locked = false;
    team_selection.team = None;

    if overlay_query.iter().next().is_none() {
        spawn_team_select_ui(commands, team_selection.character);
    }

    client_session.state = ClientConnectionState::Disconnected;
    client_session.discard_incoming_snapshots = true;
    client_session.join_flow_committed = false;
    client_session.waiting_since = None;
    client_session.last_qualifying_snapshot_wall = None;
}

fn update_session_lifecycle(
    mut commands: Commands,
    mut client_session: ResMut<ClientSession>,
    mut incoming_dead: ResMut<NetIncomingDisconnected>,
    channels: Option<Res<NetworkChannels>>,
    mut network_state: ResMut<NetworkState>,
    mut game_state_snapshot: ResMut<GameStateSnapshot>,
    mut team_selection: ResMut<TeamSelection>,
    mut cam_state: ResMut<CameraState>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut session_ui: MessageReader<SessionUiCommand>,
    remote_query: Query<Entity, With<RemotePlayer>>,
    projectile_query: Query<Entity, With<NetworkProjectile>>,
    structure_query: Query<Entity, With<NetworkStructure>>,
    minion_query: Query<Entity, With<NetworkMinion>>,
    neutral_query: Query<Entity, With<NetworkNeutral>>,
    player_query: Query<Entity, With<Player>>,
) {
    let mut retried_this_frame = false;
    for event in session_ui.read() {
        match event {
            SessionUiCommand::Retry => {
                if client_session.state == ClientConnectionState::Disconnected {
                    commands.remove_resource::<NetworkChannels>();
                    let retry_addr =
                        validated_server_addr_or_default(&client_session.server_addr_display);
                    spawn_network_transport(&mut commands, &mut client_session, retry_addr);
                    incoming_dead.0 = false;
                    retried_this_frame = true;
                }
            }
        }
    }

    // Ignore stale transport signals in the same frame as a Retry-triggered transport swap.
    if retried_this_frame {
        return;
    }

    let Some(channels) = channels.as_ref() else {
        return;
    };

    if incoming_dead.0 {
        incoming_dead.0 = false;
        if client_session.state != ClientConnectionState::Disconnected {
            perform_network_teardown(
                &mut commands,
                &mut client_session,
                &mut network_state,
                &mut game_state_snapshot,
                &mut team_selection,
                &mut cam_state,
                &overlay_query,
                &remote_query,
                &projectile_query,
                &structure_query,
                &minion_query,
                &neutral_query,
                &player_query,
            );
        }
    }

    while let Ok(signal) = channels.signals.try_recv() {
        match signal {
            NetThreadSignal::TransportFailure => {
                if client_session.state != ClientConnectionState::Disconnected {
                    perform_network_teardown(
                        &mut commands,
                        &mut client_session,
                        &mut network_state,
                        &mut game_state_snapshot,
                        &mut team_selection,
                        &mut cam_state,
                        &overlay_query,
                        &remote_query,
                        &projectile_query,
                        &structure_query,
                        &minion_query,
                        &neutral_query,
                        &player_query,
                    );
                }
            }
        }
    }

    let now = Instant::now();

    if client_session.state == ClientConnectionState::WaitingForServer {
        if let Some(since) = client_session.waiting_since {
            if since.elapsed() >= T_WAIT_MAX {
                perform_network_teardown(
                    &mut commands,
                    &mut client_session,
                    &mut network_state,
                    &mut game_state_snapshot,
                    &mut team_selection,
                    &mut cam_state,
                    &overlay_query,
                    &remote_query,
                    &projectile_query,
                    &structure_query,
                    &minion_query,
                    &neutral_query,
                    &player_query,
                );
            }
        }
    }

    if client_session.state == ClientConnectionState::Connected {
        if is_stale(
            client_session.last_qualifying_snapshot_wall,
            now,
            T_STALE_SNAPSHOT,
        ) {
            perform_network_teardown(
                &mut commands,
                &mut client_session,
                &mut network_state,
                &mut game_state_snapshot,
                &mut team_selection,
                &mut cam_state,
                &overlay_query,
                &remote_query,
                &projectile_query,
                &structure_query,
                &minion_query,
                &neutral_query,
                &player_query,
            );
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
