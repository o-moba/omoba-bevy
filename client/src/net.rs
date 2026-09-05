use shared::protocol::{JoinRejection, PROTOCOL_VERSION, SnapshotMeta, SnapshotOrder};
use shared::transport::SnapshotAssembler;

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

use crate::bosses::BossVisual;
use crate::camera::{CameraState, MainCamera, locked_camera_offset};
use crate::combat::{CombatStats, MAX_HP, MAX_MANA};
use crate::maps::MapLayout;
use crate::model_scale::{ModelScaleSource, NormalizeModelScale, model_scale_key};
use crate::persistence::{ClientSessionId, FileGameServerAddr, ResolvedServerAddressForPrefs};
use crate::player::{
    DEBUG_SPEED_MULTIPLIER, DebugSpeedBoost, PLAYER_SIZE, Player, PlayerBody, VerticalVelocity,
};
use crate::session_config::{
    DEFAULT_GAME_SERVER_ADDR, T_RETRY, T_STALE_SNAPSHOT, T_WAIT_MAX,
    TRANSPORT_CONSECUTIVE_RECV_ERRORS, TRANSPORT_CONSECUTIVE_SEND_ERRORS, is_stale,
};
use crate::sprite::{PlayerVisualMode, SpriteVisualAssets};
use crate::team::TeamSelection;
use crate::team::{CharacterChoice, Team, TeamSelectRoot, spawn_team_select_ui};
use crate::world::{PlayerAssets, PlayerModelResolver};
use shared::{HeroClass, PlayerActionKind};

const LOCAL_BIND_ADDR: &str = "0.0.0.0:0";
const UPDATE_INTERVAL_SECONDS: f32 = 0.05;
const NETWORK_LOOP_SLEEP: Duration = Duration::from_millis(16);
/// Largest application payload that can be carried by one IPv4 UDP datagram.
const IPV4_UDP_MAX_PAYLOAD_BYTES: usize = 65_507;
/// Storage is deliberately larger than the legal payload ceiling so a valid
/// server datagram can never be silently truncated before JSON decoding.
const SERVER_DATAGRAM_RECEIVE_CAPACITY: usize = 65_536;
const _: () = assert!(SERVER_DATAGRAM_RECEIVE_CAPACITY > IPV4_UDP_MAX_PAYLOAD_BYTES);
const DECODE_ERROR_LOG_INTERVAL: Duration = Duration::from_secs(1);
const PROJECTILE_RADIUS: f32 = 0.22;
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

/// The loadout that was actually sent in a Join packet. Remembered so a
/// transient connection loss can re-join automatically (the server keeps the
/// session reclaimable for `SESSION_RECLAIM_WINDOW`, 30 s) instead of
/// dumping an already-joined player back onto the select screen (TASK-25).
#[derive(Clone)]
pub struct CommittedJoin {
    pub team: Team,
    pub character: CharacterChoice,
    pub hero_class: HeroClass,
    pub avatar: Option<String>,
    pub sprite_character: Option<String>,
}

/// Auto-reconnect bookkeeping for a torn-down session with a committed join.
#[derive(Default, Clone, Copy)]
pub struct ReconnectState {
    pub active: bool,
    pub attempts: u32,
    pub last_attempt: Option<Instant>,
}

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
    /// Loadout of the last committed join (auto-rejoin after reconnect).
    pub last_join: Option<CommittedJoin>,
    /// Auto-reconnect loop state (active after a teardown of a joined session).
    pub reconnect: ReconnectState,
    admitted: bool,
    join_last_sent: Option<Instant>,
    join_attempts: u32,
    join_error: Option<JoinRejection>,
    join_exhausted: bool,
    snapshot_order: SnapshotOrder,
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
            last_join: None,
            reconnect: ReconnectState::default(),
            admitted: false,
            join_last_sent: None,
            join_attempts: 0,
            join_error: None,
            join_exhausted: false,
            snapshot_order: SnapshotOrder::default(),
        }
    }
}

/// Why a network teardown happened; logged so surprise disconnects are
/// diagnosable from the client log.
#[derive(Debug, Clone, Copy)]
enum TeardownReason {
    StaleSnapshot { elapsed_secs: f32 },
    TransportFailure,
    ServerWaitTimeout,
    IncomingChannelClosed,
}

impl std::fmt::Display for TeardownReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleSnapshot { elapsed_secs } => {
                write!(f, "no qualifying snapshot for {elapsed_secs:.1}s")
            }
            Self::TransportFailure => write!(f, "transport failure reported by the UDP thread"),
            Self::ServerWaitTimeout => write!(f, "server did not answer within the wait budget"),
            Self::IncomingChannelClosed => write!(f, "incoming packet channel closed"),
        }
    }
}

/// A teardown only falls back to the team-select screen when the player
/// never committed a join; joined players auto-reconnect instead.
fn teardown_shows_select(has_committed_join: bool) -> bool {
    !has_committed_join
}

/// Auto-reconnect cadence gate: disconnected + active + `T_RETRY` since the
/// last attempt (or no attempt yet).
fn should_attempt_reconnect(
    state: ClientConnectionState,
    reconnect_active: bool,
    elapsed_since_last_attempt: Option<Duration>,
) -> bool {
    state == ClientConnectionState::Disconnected
        && reconnect_active
        && elapsed_since_last_attempt.is_none_or(|elapsed| elapsed >= T_RETRY)
}

impl ClientSession {
    pub fn join_confirmed(&self) -> bool {
        self.is_connected() && self.admitted
    }

    fn clear_join_attempt(&mut self) {
        self.admitted = false;
        self.join_last_sent = None;
        self.join_attempts = 0;
        self.join_error = None;
        self.join_exhausted = false;
    }

    fn join_retry_due(&self, now: Instant) -> bool {
        self.is_connected()
            && self.last_join.is_some()
            && !self.admitted
            && self.join_error.is_none()
            && !self.join_exhausted
            && self
                .join_last_sent
                .is_none_or(|at| now.saturating_duration_since(at) >= T_RETRY)
    }

    pub fn is_connected(&self) -> bool {
        self.state == ClientConnectionState::Connected
    }
}

pub struct NetworkingPlugin;

/// Strict main-thread ordering for networking / snapshot / UI sync (Bevy 0.18: avoid `.after(fn)`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ClientNetPipeline {
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

/// Shared production scheduling contract, also used by the isolated ECS regression.
pub(crate) fn configure_network_pipeline(app: &mut App) {
    app.configure_sets(
        Update,
        (
            // Resolve current authoritative state before reading modal/gameplay input.
            // Commands created by those inputs leave in the same frame.
            crate::input_context::InputContextSet::Modal
                .after(ClientNetPipeline::InterpolateRemotePlayers),
            ClientNetPipeline::SendLocalState.after(crate::input_context::InputContextSet::Actions),
            ClientNetPipeline::SendCommands.after(ClientNetPipeline::SendLocalState),
            ClientNetPipeline::ApplySnapshot.after(ClientNetPipeline::IngestSnapshot),
            ClientNetPipeline::InterpolateNetEntities.after(ClientNetPipeline::ApplySnapshot),
            ClientNetPipeline::InterpolateRemotePlayers
                .after(ClientNetPipeline::InterpolateNetEntities),
            ClientNetPipeline::SessionRetryInput.after(ClientNetPipeline::SendCommands),
            ClientNetPipeline::SessionLifecycle.after(ClientNetPipeline::SessionRetryInput),
            ClientNetPipeline::SyncConnectionUi.after(ClientNetPipeline::SessionLifecycle),
        ),
    );
}

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        configure_network_pipeline(app);
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
            .add_systems(
                Update,
                send_local_state.in_set(ClientNetPipeline::SendLocalState),
            )
            .add_systems(
                Update,
                send_network_commands.in_set(ClientNetPipeline::SendCommands),
            )
            .add_systems(Update, mirror_debug_flags_to_network_state)
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
                (update_session_lifecycle, retry_pending_join)
                    .chain()
                    .in_set(ClientNetPipeline::SessionLifecycle),
            )
            .add_systems(
                Update,
                sync_connection_status_ui.in_set(ClientNetPipeline::SyncConnectionUi),
            )
            .add_systems(
                PostUpdate,
                ground_networked_entities.before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// Client-side terrain lift for server-driven entities. The server simulates
/// a flat ground plane; after interpolation writes the (flat) server Y, this
/// re-bases remote players and minions onto the local terrain (base pads and
/// their ramps), matching how the local player is grounded.
fn ground_networked_entities(
    map_layout: Res<MapLayout>,
    visual_mode: Res<PlayerVisualMode>,
    mut remote_players: Query<
        (&mut Transform, Option<&NormalizeModelScale>),
        (With<RemotePlayer>, Without<NetworkMinion>),
    >,
    mut minions: Query<
        (&mut Transform, Option<&NormalizeModelScale>),
        (With<NetworkMinion>, Without<RemotePlayer>),
    >,
) {
    for (mut transform, normalization) in &mut remote_players {
        transform.translation.y = crate::player::ground_origin_y(
            &map_layout,
            *visual_mode,
            normalization,
            transform.translation.x,
            transform.translation.z,
        );
    }
    for (mut transform, normalization) in &mut minions {
        let terrain = if *visual_mode == PlayerVisualMode::Models3d {
            map_layout.terrain_height_3d(transform.translation.x, transform.translation.z)
        } else {
            map_layout.terrain_height(transform.translation.x, transform.translation.z)
        };
        // Slime models rest on their measured foot offset; the sphere radius
        // remains the fallback until the model is measured.
        let offset = match normalization.and_then(NormalizeModelScale::foot_local_y) {
            Some(foot_local_y) => -foot_local_y,
            None => MINION_RADIUS,
        };
        transform.translation.y = terrain + offset;
    }
}

#[derive(Message, Clone, Debug)]
pub enum NetworkCommand {
    Cast {
        target: TargetId,
        /// Hotbar slot index (0=Q .. 3=R).
        slot: u8,
    },
    Join {
        team: Team,
        character: CharacterChoice,
        hero_class: HeroClass,
        /// Selected roster avatar slug (cosmetic), if any.
        avatar: Option<String>,
        /// Selected 2D sprite cosmetic. The renderer mode remains client-local.
        sprite_character: Option<String>,
    },
    #[allow(dead_code)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientPacket {
    Hello {
        protocol_version: u16,
    },
    Transform {
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
    },
    Cast {
        target: TargetId,
        #[serde(default)]
        slot: u8,
    },
    Join {
        team: Team,
        #[serde(default = "default_character_choice")]
        character: CharacterChoice,
        #[serde(default)]
        hero_class: HeroClass,
        #[serde(default)]
        avatar: Option<String>,
        #[serde(default)]
        sprite_character: Option<String>,
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
    #[serde(default = "default_skill_ranks")]
    ranks: [u8; 4],
    #[serde(default = "default_character_choice")]
    character: CharacterChoice,
    /// Authoritative class the server resolved for this player.
    #[serde(default)]
    hero_class: HeroClass,
    /// Cosmetic roster avatar slug; `None` falls back to `character`.
    #[serde(default)]
    avatar: Option<String>,
    /// Cosmetic sprite id; absent legacy snapshots use the manifest default.
    #[serde(default)]
    sprite_character: Option<String>,
    #[serde(default)]
    action_sequence: u64,
    #[serde(default)]
    action_kind: PlayerActionKind,
    #[serde(default)]
    action_slot: u8,
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
    #[serde(default)]
    protected: bool,
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
pub enum MinionBrainState {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NeutralCampType {
    Skirmisher,
    Bruiser,
    Spitter,
    /// Bottom raid boss ("Wendigo").
    WendigoBoss,
    /// Top raid boss ("King Mutatio").
    KingMutatioBoss,
}

impl NeutralCampType {
    pub fn is_boss(self) -> bool {
        matches!(
            self,
            NeutralCampType::WendigoBoss | NeutralCampType::KingMutatioBoss
        )
    }
}

/// Team-wide boss buff kinds replicated from the server (TASK-19).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeamBuffKind {
    /// Bottom boss (Wendigo): +ability damage.
    WendigoFavor,
    /// Top boss (King Mutatio): +ability damage and HP regen.
    MutatioMight,
}

/// One active team buff from the snapshot (`serde(default)` additive field).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamBuffState {
    pub team: Team,
    pub kind: TeamBuffKind,
    #[serde(default)]
    pub remaining_secs: f32,
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
        #[serde(flatten, default)]
        meta: SnapshotMeta,
        #[serde(default)]
        join_error: Option<JoinRejection>,
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
        team_buffs: Vec<TeamBuffState>,
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
    /// Release-mode matchmaking: players joined so far vs. roster size.
    Forming {
        ready: u32,
        needed: u32,
    },
    /// Full roster found; the match starts when the countdown elapses.
    Starting {
        countdown_ms: u32,
    },
    Running,
    Victory {
        winner: Team,
    },
}

#[derive(Resource, Default, Clone)]
pub struct GameStateSnapshot {
    pub meta: SnapshotMeta,
    pub state: GameState,
    pub rematch_in_secs: Option<u64>,
    /// Active boss team buffs replicated from the server.
    pub team_buffs: Vec<TeamBuffState>,
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
    /// Mirror of `DebugSpeedBoost`, so snapshot reconcile can widen the snap
    /// threshold while boosting without exceeding the 16-param system limit.
    speed_boost_active: bool,
}

/// Latest drained snapshot for this frame (filled by [`ingest_server_snapshot_packets`]).
#[derive(Resource, Default)]
struct PendingServerSnapshotFrame {
    frame: Option<PendingSnapshotData>,
}

struct PendingSnapshotData {
    meta: SnapshotMeta,
    wall_time: Instant,
    your_id: u64,
    players: Vec<PlayerState>,
    projectiles: Vec<ProjectileState>,
    structures: Vec<StructureState>,
    minions: Vec<MinionState>,
    neutrals: Vec<NeutralState>,
    team_buffs: Vec<TeamBuffState>,
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

/// Roster avatar slug replicated from the server (`None` = legacy character model).
#[derive(Component, Clone, Debug)]
pub struct NetworkAvatar(pub Option<String>);

/// Sprite character id replicated from the server (`None` = manifest default).
#[derive(Component, Clone, Debug)]
pub struct NetworkSpriteCharacter(pub Option<String>);

/// Latest authoritative cosmetic action for a player. The sequence lets
/// renderers distinguish a new cast from repeated delivery in snapshots.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerCosmeticAction {
    pub sequence: u64,
    pub kind: PlayerActionKind,
    pub slot: u8,
}

impl From<&PlayerState> for PlayerCosmeticAction {
    fn from(player: &PlayerState) -> Self {
        Self {
            sequence: player.action_sequence,
            kind: player.action_kind,
            slot: player.action_slot,
        }
    }
}

/// Authoritative hero class replicated from the server.
#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkHeroClass(pub HeroClass);

#[derive(Component, Clone, Copy, Debug)]
pub struct PlayerProgression {
    pub level: u32,
    pub xp: u32,
    pub next_level_xp: u32,
    pub skill_points: u32,
    /// Per-slot ability ranks (1-based) mirrored from the server snapshot (TASK03).
    pub ranks: [u8; 4],
}

impl Default for PlayerProgression {
    fn default() -> Self {
        Self {
            level: 1,
            xp: 0,
            next_level_xp: 0,
            skill_points: 0,
            ranks: [1; 4],
        }
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkProjectile {
    #[allow(dead_code)] // Consumed by the optional 2D presentation plugin.
    pub owner_team: Team,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkStructureId(pub u64);

#[derive(Component)]
pub struct NetworkStructure;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NetworkStructureProtected(pub bool);

#[derive(Component)]
pub struct NetworkMinion;

/// Replicated minion AI state, mirrored onto the entity so the minion
/// presentation systems can animate walking and attacks.
#[derive(Component, Clone, Copy)]
pub struct NetworkMinionBrainState(pub MinionBrainState);

/// Minions render at this multiple of the shared normalized hero height.
const MINION_MODEL_HEIGHT_SCALE: f32 = 0.6;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkMinionId(pub u64);

#[derive(Component)]
pub struct NetworkNeutral;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkNeutralId(pub u64);

/// Replicated neutral AI state (drives the boss idle/walk animation switch).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralAiStateTag(pub NeutralAiState);

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
pub struct NetworkVisualAssets {
    projectile_mesh: Handle<Mesh>,
    friendly_projectile_material: Handle<StandardMaterial>,
    hostile_projectile_material: Handle<StandardMaterial>,
    neutral_mesh: Handle<Mesh>,
    neutral_material: Handle<StandardMaterial>,
}

fn setup_network_visual_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let projectile_mesh = meshes.add(Mesh::from(Sphere::new(PROJECTILE_RADIUS)));
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
    client_session.clear_join_attempt();
    client_session.snapshot_order = SnapshotOrder::default();
    client_session.server_addr_display.clone_from(&server_addr);
    commands.insert_resource(ResolvedServerAddressForPrefs(server_addr.clone()));
    let (outgoing_tx, outgoing_rx) = unbounded::<ClientPacket>();
    let (incoming_tx, incoming_rx) = crossbeam_channel::bounded::<ServerPacket>(8);
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

    let mut assembler = SnapshotAssembler::default();
    let mut recv_buf = vec![0_u8; SERVER_DATAGRAM_RECEIVE_CAPACITY];
    let mut last_heartbeat_at = Instant::now();
    let mut last_receive_error_log_at: Option<Instant> = None;
    let mut last_decode_error_log_at: Option<Instant> = None;
    let mut suppressed_decode_errors = 0_u64;
    let mut first_snapshot_received = false;
    let mut consecutive_recv_errors: u32 = 0;
    let mut consecutive_send_errors: u32 = 0;
    let mut transport_failure_reported = false;

    let _ = udp_try_send(
        &socket,
        &ClientPacket::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
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
            assembler.expire(Instant::now());
            udp_try_send(
                &socket,
                &ClientPacket::Hello {
                    protocol_version: PROTOCOL_VERSION,
                },
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
                    let payload = match assembler.push(&recv_buf[..len], Instant::now()) {
                        Ok(Some(payload)) => payload,
                        Ok(None) => continue,
                        Err(error) => {
                            if last_decode_error_log_at
                                .is_none_or(|at: Instant| at.elapsed() >= T_RETRY)
                            {
                                warn!("Snapshot frame rejected: {error}");
                                last_decode_error_log_at = Some(Instant::now());
                            }
                            continue;
                        }
                    };
                    match forward_complete_server_datagram(&payload, &incoming) {
                        Ok(published) => {
                            if published && !first_snapshot_received {
                                println!(
                                    "First snapshot received from {server_addr}; connection is live"
                                );
                                first_snapshot_received = true;
                            }
                        }
                        Err(error) => {
                            let now = Instant::now();
                            if last_decode_error_log_at.is_none_or(|last| {
                                now.duration_since(last) >= DECODE_ERROR_LOG_INTERVAL
                            }) {
                                let suppressed = std::mem::take(&mut suppressed_decode_errors);
                                eprintln!(
                                    "Failed to decode complete server datagram ({len} bytes): \
                                     {error}; suppressed {suppressed} similar errors"
                                );
                                last_decode_error_log_at = Some(now);
                            } else {
                                suppressed_decode_errors =
                                    suppressed_decode_errors.saturating_add(1);
                            }
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

fn decode_server_packet(payload: &[u8]) -> Result<ServerPacket, serde_json::Error> {
    serde_json::from_slice(payload)
}

/// Decode one complete received datagram and publish only the resulting whole
/// packet. Keeping this boundary shared by the socket loop and tests proves a
/// malformed/truncated JSON prefix can never enter snapshot staging.
fn forward_complete_server_datagram(
    payload: &[u8],
    incoming: &Sender<ServerPacket>,
) -> io::Result<bool> {
    let packet = decode_server_packet(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    match incoming.try_send(packet) {
        Ok(()) => Ok(true),
        // A stalled frame must not grow an unbounded snapshot queue.
        Err(crossbeam_channel::TrySendError::Full(_)) => Ok(false),
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "snapshot receiver closed",
        )),
    }
}

fn send_packet(socket: &UdpSocket, packet: &ClientPacket) -> io::Result<()> {
    let payload = serde_json::to_vec(packet)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    validate_client_payload_size(payload.len())?;
    socket.send(&payload)?;
    Ok(())
}

fn validate_client_payload_size(payload_len: usize) -> io::Result<()> {
    if payload_len > IPV4_UDP_MAX_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "client datagram is {} bytes; IPv4 UDP payload limit is {} bytes",
                payload_len, IPV4_UDP_MAX_PAYLOAD_BYTES
            ),
        ));
    }
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

    if !client_session.join_confirmed() {
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

fn mirror_debug_flags_to_network_state(
    speed_boost: Res<DebugSpeedBoost>,
    mut network_state: ResMut<NetworkState>,
) {
    if network_state.speed_boost_active != speed_boost.0 {
        network_state.speed_boost_active = speed_boost.0;
    }
}

fn send_network_commands(
    mut command_events: MessageReader<NetworkCommand>,
    channels: Option<Res<NetworkChannels>>,
    mut client_session: ResMut<ClientSession>,
    client_session_id: Res<ClientSessionId>,
) {
    let Some(channels) = channels else {
        return;
    };

    for command in command_events.read() {
        match command {
            NetworkCommand::Cast { target, slot } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels.outgoing.send(ClientPacket::Cast {
                    target: *target,
                    slot: *slot,
                });
            }
            NetworkCommand::Join {
                team,
                character,
                hero_class,
                avatar,
                sprite_character,
            } => {
                if client_session.admitted {
                    continue;
                }
                client_session.clear_join_attempt();
                client_session.join_flow_committed = true;
                client_session.last_join = Some(CommittedJoin {
                    team: *team,
                    character: *character,
                    hero_class: *hero_class,
                    avatar: avatar.clone(),
                    sprite_character: sprite_character.clone(),
                });
                send_join_attempt(&channels, &mut client_session, &client_session_id);
            }
            NetworkCommand::RequestRematch => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels.outgoing.send(ClientPacket::RequestRematch);
            }
            NetworkCommand::SetGodMode { enabled } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels
                    .outgoing
                    .send(ClientPacket::SetGodMode { enabled: *enabled });
            }
            NetworkCommand::SetSpeedBoost { enabled } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels
                    .outgoing
                    .send(ClientPacket::SetSpeedBoost { enabled: *enabled });
            }
            NetworkCommand::UpgradeSkill { slot } => {
                if !client_session.join_confirmed() {
                    continue;
                }
                let _ = channels
                    .outgoing
                    .send(ClientPacket::UpgradeSkill { slot: *slot });
            }
        }
    }
}

fn retry_pending_join(
    channels: Option<Res<NetworkChannels>>,
    mut session: ResMut<ClientSession>,
    identity: Res<ClientSessionId>,
) {
    if let Some(channels) = channels {
        if session.join_retry_due(Instant::now()) {
            send_join_attempt(&channels, &mut session, &identity);
        }
    }
}

const MAX_JOIN_ATTEMPTS: u32 = 15;

fn send_join_attempt(
    channels: &NetworkChannels,
    session: &mut ClientSession,
    session_id: &ClientSessionId,
) {
    let Some(join) = session.last_join.clone() else {
        return;
    };
    if session.join_attempts >= MAX_JOIN_ATTEMPTS {
        session.join_exhausted = true;
        return;
    }
    let result = channels.outgoing.send(ClientPacket::Join {
        team: join.team,
        character: join.character,
        hero_class: join.hero_class,
        avatar: join.avatar,
        sprite_character: join.sprite_character,
        session_id: Some(session_id.0.clone()),
    });
    session.join_last_sent = Some(Instant::now());
    session.join_attempts += 1;
    session.join_flow_committed = true;
    if result.is_err() {
        session.join_exhausted = true;
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
    mut client_session: ResMut<ClientSession>,
    mut pending: ResMut<PendingServerSnapshotFrame>,
    mut incoming_dead: ResMut<NetIncomingDisconnected>,
    team_selection: Res<TeamSelection>,
) {
    pending.frame = None;
    let Some(channels) = channels.as_ref() else {
        return;
    };

    if client_session.discard_incoming_snapshots {
        while let Ok(_ignored) = channels.incoming.try_recv() {}
        return;
    }

    let mut latest_snapshot: Option<PendingSnapshotData> = None;

    loop {
        match channels.incoming.try_recv() {
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                incoming_dead.0 = true;
                break;
            }
            Ok(packet) => match packet {
                ServerPacket::Snapshot {
                    meta,
                    join_error,
                    your_id,
                    players,
                    projectiles,
                    structures,
                    minions,
                    neutrals,
                    team_buffs,
                    game_state,
                    rematch_in_secs,
                } => {
                    if meta.protocol_version != PROTOCOL_VERSION {
                        client_session.join_error = Some(JoinRejection::ProtocolMismatch);
                        continue;
                    }
                    if !client_session.snapshot_order.accept(meta) {
                        continue;
                    }
                    client_session.join_error = join_error;
                    client_session.admitted =
                        join_error.is_none() && players.iter().any(|player| player.id == your_id);
                    if client_session.admitted {
                        client_session.reconnect = ReconnectState::default();
                        client_session.join_exhausted = false;
                    }
                    latest_snapshot = Some(PendingSnapshotData {
                        meta,
                        wall_time: Instant::now(),
                        your_id,
                        players,
                        projectiles,
                        structures,
                        minions,
                        neutrals,
                        team_buffs,
                        game_state,
                        rematch_in_secs,
                        selected_team_for_spawn: team_selection.team,
                    });
                }
            },
        }
    }

    pending.frame = latest_snapshot;
}

/// Grouped UI-side resources for [`apply_server_snapshot`] (Bevy caps system
/// functions at 16 parameters).
#[derive(bevy::ecs::system::SystemParam)]
struct SnapshotUiState<'w> {
    game_state_snapshot: ResMut<'w, GameStateSnapshot>,
    cam_state: ResMut<'w, CameraState>,
    team_selection: ResMut<'w, TeamSelection>,
    visual_mode: Res<'w, PlayerVisualMode>,
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
    action_query: Query<Option<&PlayerCosmeticAction>>,
    player_assets: Res<PlayerAssets>,
    mut models: PlayerModelResolver,
    visuals: Res<NetworkVisualAssets>,
    mut ui_state: SnapshotUiState,
) {
    let SnapshotUiState {
        game_state_snapshot,
        cam_state,
        team_selection,
        visual_mode,
    } = &mut ui_state;
    let Some(data) = pending.frame.take() else {
        return;
    };
    let PendingSnapshotData {
        meta,
        wall_time: snapshot_wall_time,
        your_id,
        players,
        projectiles,
        structures,
        minions,
        neutrals,
        team_buffs,
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
    game_state_snapshot.meta = meta;
    game_state_snapshot.state = game_state;
    game_state_snapshot.rematch_in_secs = rematch_in_secs;
    game_state_snapshot.team_buffs = team_buffs;

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
                NetworkAvatar(local_player_state.avatar.clone()),
                NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                NetworkHeroClass(local_player_state.hero_class),
                player_state_to_progression(local_player_state),
            ));
            let next_action = PlayerCosmeticAction::from(local_player_state);
            if action_query.get(local_entity).ok().flatten().copied() != Some(next_action) {
                commands.entity(local_entity).insert(next_action);
            }
            network_state.local_team = Some(local_player_state.team);

            let server_translation = Vec3::new(
                local_player_state.x,
                local_player_state.y,
                local_player_state.z,
            );
            if let Ok(mut local_transform) = transform_sets.p0().get_mut(local_entity) {
                // Snap on meaningful server corrections (first team spawn, respawn, etc.).
                // While speed-boosting, the local player legitimately leads the last
                // server-acked position further, so widen the threshold to avoid
                // rubber-banding the boosted movement.
                let snap_distance = if network_state.speed_boost_active {
                    LOCAL_SNAP_DISTANCE * DEBUG_SPEED_MULTIPLIER
                } else {
                    LOCAL_SNAP_DISTANCE
                };
                if local_transform
                    .translation
                    .distance_squared(server_translation)
                    > snap_distance * snap_distance
                {
                    local_transform.translation = server_translation;
                    local_transform.rotation = Quat::from_rotation_y(local_player_state.yaw);
                }
            }
        }
    } else if let Some(local_player_state) = local_player_state {
        // Spawn only after the local join was committed (a team was picked).
        // Snapshots list joined players only, so our presence in the list is
        // the server's join ack. The server may have assigned a different
        // team than requested (release-mode balancing) - adopt it as truth.
        if selected_team_for_spawn.is_none() {
            return;
        }
        if team_selection.team != Some(local_player_state.team) {
            info!(
                "Server assigned team {} (matchmaking)",
                local_player_state.team.as_str()
            );
            team_selection.team = Some(local_player_state.team);
        }
        let spawn = Vec3::new(
            local_player_state.x,
            local_player_state.y,
            local_player_state.z,
        );
        let (local_scene, local_gltf) = if **visual_mode == PlayerVisualMode::Models3d {
            models.resolve(
                local_player_state.character,
                local_player_state.avatar.as_deref(),
            )
        } else {
            (None, None)
        };
        let entity = if **visual_mode == PlayerVisualMode::Sprite2d {
            commands
                .spawn((
                    Transform::from_translation(spawn),
                    Visibility::default(),
                    Player,
                    PlayerBody,
                    VerticalVelocity::default(),
                    local_player_state.team,
                    (
                        NetworkPlayerId(your_id),
                        NetworkCharacterChoice(local_player_state.character),
                        NetworkAvatar(local_player_state.avatar.clone()),
                        NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                        PlayerCosmeticAction::from(local_player_state),
                        NetworkHeroClass(local_player_state.hero_class),
                    ),
                    player_state_to_combat_stats(local_player_state),
                    player_state_to_progression(local_player_state),
                    Name::new("Player"),
                ))
                .id()
        } else if let Some(scene_handle) = local_scene {
            let mut entity_commands = commands.spawn((
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
                (
                    NetworkPlayerId(your_id),
                    NetworkCharacterChoice(local_player_state.character),
                    NetworkAvatar(local_player_state.avatar.clone()),
                    NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                    PlayerCosmeticAction::from(local_player_state),
                    NetworkHeroClass(local_player_state.hero_class),
                ),
                player_state_to_combat_stats(local_player_state),
                player_state_to_progression(local_player_state),
                Name::new("Player"),
            ));
            if let Some(gltf) = local_gltf {
                entity_commands.insert(ModelScaleSource {
                    gltf,
                    key: model_scale_key(
                        local_player_state.character,
                        local_player_state.avatar.as_deref(),
                    ),
                });
            }
            entity_commands.id()
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
                    (
                        NetworkPlayerId(your_id),
                        NetworkCharacterChoice(local_player_state.character),
                        NetworkAvatar(local_player_state.avatar.clone()),
                        NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                        PlayerCosmeticAction::from(local_player_state),
                        NetworkHeroClass(local_player_state.hero_class),
                    ),
                    player_state_to_combat_stats(local_player_state),
                    player_state_to_progression(local_player_state),
                    Name::new("Player"),
                ))
                .id()
        };

        network_state.local_team = Some(local_player_state.team);
        if let Ok(mut camera_transform) = transform_sets.p1().single_mut() {
            cam_state.locked = true;
            if **visual_mode == PlayerVisualMode::Sprite2d {
                let xy = crate::world2d::simulation_xz_to_render_xy(spawn);
                camera_transform.translation.x = xy.x;
                camera_transform.translation.y = xy.y;
            } else {
                let zoom = cam_state.zoom;
                camera_transform.translation = spawn + locked_camera_offset(zoom);
                let look_target = Vec3::new(spawn.x, PLAYER_SIZE * 0.5, spawn.z);
                *camera_transform = camera_transform.looking_at(look_target, Vec3::Y);
            }
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
                NetworkAvatar(player.avatar.clone()),
                NetworkSpriteCharacter(player.sprite_character.clone()),
                NetworkHeroClass(player.hero_class),
                player_state_to_combat_stats(player),
                player_state_to_progression(player),
            ));
            let next_action = PlayerCosmeticAction::from(player);
            if action_query.get(entity).ok().flatten().copied() != Some(next_action) {
                commands.entity(entity).insert(next_action);
            }
            continue;
        }

        let (scene_handle, gltf_handle) = if **visual_mode == PlayerVisualMode::Models3d {
            models.resolve(player.character, player.avatar.as_deref())
        } else {
            (None, None)
        };
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
            NetworkAvatar(player.avatar.clone()),
            NetworkSpriteCharacter(player.sprite_character.clone()),
            PlayerCosmeticAction::from(player),
            NetworkHeroClass(player.hero_class),
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
        if **visual_mode == PlayerVisualMode::Models3d {
            entity_commands.insert(NormalizeModelScale::for_player_model());
            if let Some(gltf) = gltf_handle {
                entity_commands.insert(ModelScaleSource {
                    gltf,
                    key: model_scale_key(player.character, player.avatar.as_deref()),
                });
            }
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
        }
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
            commands.entity(entity).insert(NetworkProjectile {
                owner_team: projectile.owner_team,
            });
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

        let mut entity_commands = commands.spawn((
            Transform::from_xyz(projectile.x, projectile.y, projectile.z),
            Visibility::default(),
            NetworkProjectile {
                owner_team: projectile.owner_team,
            },
            Name::new(format!("Projectile-{}", projectile.id)),
        ));
        if **visual_mode == PlayerVisualMode::Models3d {
            entity_commands.insert((
                Mesh3d(visuals.projectile_mesh.clone()),
                MeshMaterial3d(material),
            ));
        }
        let entity = entity_commands.id();
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
                NetworkStructureProtected(structure.protected),
                structure_state_to_combat_stats(structure),
            ));
            continue;
        }

        let entity_commands = commands.spawn((
            Transform::from_xyz(structure.x, structure.y, structure.z),
            Visibility::default(),
            NetworkStructure,
            NetworkStructureId(structure.id),
            NetworkStructureProtected(structure.protected),
            structure.kind,
            structure.team,
            structure_state_to_combat_stats(structure),
            Name::new(format!("Structure-{}", structure.id)),
        ));
        let entity = entity_commands.id();

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
                NetworkMinionBrainState(minion.state),
            ));
            continue;
        }

        let mut entity_commands = commands.spawn((
            Transform::from_translation(target_translation).with_rotation(target_rotation),
            Visibility::default(),
            NetworkMinion,
            NetworkMinionId(minion.id),
            NetworkMinionBrainState(minion.state),
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
        ));
        if **visual_mode == PlayerVisualMode::Models3d {
            // Original procedural meshes attach once in MinionVisualsPlugin.
            entity_commands.insert(NormalizeModelScale::scaled_by(MINION_MODEL_HEIGHT_SCALE));
        }
        let entity = entity_commands.id();
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
                NeutralAiStateTag(neutral.ai_state),
            ));
            continue;
        }

        let base_components = (
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
            NeutralAiStateTag(neutral.ai_state),
        );

        // Raid bosses render their staged GLB model (attached by the bosses
        // module) instead of the generic neutral sphere.
        let entity = if neutral.camp_type.is_boss() {
            commands
                .spawn((
                    base_components,
                    BossVisual {
                        camp_type: neutral.camp_type,
                    },
                    Name::new(format!(
                        "Boss-{}-{}",
                        neutral.id,
                        crate::bosses::boss_display_name(neutral.camp_type)
                    )),
                ))
                .id()
        } else if **visual_mode == PlayerVisualMode::Models3d {
            commands
                .spawn((
                    base_components,
                    Mesh3d(visuals.neutral_mesh.clone()),
                    MeshMaterial3d(visuals.neutral_material.clone()),
                    Name::new(format!("Neutral-{}", neutral.id)),
                ))
                .id()
        } else {
            commands
                .spawn((
                    base_components,
                    Name::new(format!("Neutral-{}", neutral.id)),
                ))
                .id()
        };
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
            text.0 = if client_session.join_confirmed() {
                "Joined - connected to match.".to_owned()
            } else if client_session.last_join.is_some() {
                format!(
                    "Joining match… attempt {}/{}. Waiting for server admission.",
                    client_session.join_attempts, MAX_JOIN_ATTEMPTS
                )
            } else {
                "Connected - choose a hero and team to join.".to_owned()
            };
        }
        ClientConnectionState::Disconnected if client_session.reconnect.active => {
            text.0 = format!(
                "Connection lost - reconnecting (attempt {})...",
                client_session.reconnect.attempts.max(1)
            );
        }
        ClientConnectionState::Disconnected => {
            text.0 = "Disconnected - connection lost or timed out. Use Retry when the server is back, then choose your team again."
                .to_string();
        }
    }

    if let Some(reason) = client_session.join_error {
        text.0 = reason.message().to_owned();
    } else if client_session.join_exhausted {
        text.0 = "The server did not confirm your Join. Use Retry to try again.".to_owned();
    }

    let Ok(mut retry_vis) = retry_vis.single_mut() else {
        return;
    };
    *retry_vis = if client_session.state == ClientConnectionState::Disconnected
        || client_session.join_error.is_some()
        || client_session.join_exhausted
    {
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
    reason: TeardownReason,
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
    visual_mode: PlayerVisualMode,
    sprite_assets: &SpriteVisualAssets,
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

    warn!("Network teardown: {reason}");
    if teardown_shows_select(client_session.last_join.is_some()) {
        team_selection.team = None;
        if overlay_query.iter().next().is_none() {
            spawn_team_select_ui(commands, team_selection, visual_mode, sprite_assets);
        }
    } else if !client_session.reconnect.active {
        // Joined session: keep the team selection and reconnect quietly
        // instead of dumping the player back onto the select screen.
        client_session.reconnect = ReconnectState {
            active: true,
            attempts: 0,
            last_attempt: None,
        };
    }

    client_session.clear_join_attempt();
    client_session.state = ClientConnectionState::Disconnected;
    client_session.discard_incoming_snapshots = true;
    client_session.join_flow_committed = false;
    client_session.waiting_since = None;
    client_session.last_qualifying_snapshot_wall = None;
}

/// Entity queries a network teardown needs, grouped (Bevy caps system
/// functions at 16 parameters).
#[derive(bevy::ecs::system::SystemParam)]
struct TeardownQueries<'w, 's> {
    overlay_query: Query<'w, 's, Entity, With<TeamSelectRoot>>,
    remote_query: Query<'w, 's, Entity, With<RemotePlayer>>,
    projectile_query: Query<'w, 's, Entity, With<NetworkProjectile>>,
    structure_query: Query<'w, 's, Entity, With<NetworkStructure>>,
    minion_query: Query<'w, 's, Entity, With<NetworkMinion>>,
    neutral_query: Query<'w, 's, Entity, With<NetworkNeutral>>,
    player_query: Query<'w, 's, Entity, With<Player>>,
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
    queries: TeardownQueries,
    mut session_ui: MessageReader<SessionUiCommand>,
    visual_mode: Res<PlayerVisualMode>,
    sprite_assets: Res<SpriteVisualAssets>,
) {
    let TeardownQueries {
        overlay_query,
        remote_query,
        projectile_query,
        structure_query,
        minion_query,
        neutral_query,
        player_query,
    } = &queries;
    let mut retried_this_frame = false;
    for event in session_ui.read() {
        match event {
            SessionUiCommand::Retry => {
                if client_session.state == ClientConnectionState::Disconnected
                    || client_session.join_error.is_some()
                    || client_session.join_exhausted
                {
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

    // Auto-reconnect (TASK-25): a torn-down session with a committed join
    // retries the transport on the shared retry cadence, no user input.
    if should_attempt_reconnect(
        client_session.state,
        client_session.reconnect.active,
        client_session.reconnect.last_attempt.map(|at| at.elapsed()),
    ) {
        commands.remove_resource::<NetworkChannels>();
        let retry_addr = validated_server_addr_or_default(&client_session.server_addr_display);
        spawn_network_transport(&mut commands, &mut client_session, retry_addr);
        incoming_dead.0 = false;
        client_session.reconnect.attempts = client_session.reconnect.attempts.saturating_add(1);
        client_session.reconnect.last_attempt = Some(Instant::now());
        info!(
            "Auto-reconnect attempt {} to {}",
            client_session.reconnect.attempts, client_session.server_addr_display
        );
        return;
    }

    let Some(channels) = channels.as_ref() else {
        return;
    };

    if incoming_dead.0 {
        incoming_dead.0 = false;
        if client_session.state != ClientConnectionState::Disconnected {
            perform_network_teardown(
                TeardownReason::IncomingChannelClosed,
                &mut commands,
                &mut client_session,
                &mut network_state,
                &mut game_state_snapshot,
                &mut team_selection,
                &mut cam_state,
                overlay_query,
                remote_query,
                projectile_query,
                structure_query,
                minion_query,
                neutral_query,
                player_query,
                *visual_mode,
                &sprite_assets,
            );
        }
    }

    while let Ok(signal) = channels.signals.try_recv() {
        match signal {
            NetThreadSignal::TransportFailure => {
                if client_session.state != ClientConnectionState::Disconnected {
                    perform_network_teardown(
                        TeardownReason::TransportFailure,
                        &mut commands,
                        &mut client_session,
                        &mut network_state,
                        &mut game_state_snapshot,
                        &mut team_selection,
                        &mut cam_state,
                        overlay_query,
                        remote_query,
                        projectile_query,
                        structure_query,
                        minion_query,
                        neutral_query,
                        player_query,
                        *visual_mode,
                        &sprite_assets,
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
                    TeardownReason::ServerWaitTimeout,
                    &mut commands,
                    &mut client_session,
                    &mut network_state,
                    &mut game_state_snapshot,
                    &mut team_selection,
                    &mut cam_state,
                    overlay_query,
                    remote_query,
                    projectile_query,
                    structure_query,
                    minion_query,
                    neutral_query,
                    player_query,
                    *visual_mode,
                    &sprite_assets,
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
            let elapsed_secs = client_session
                .last_qualifying_snapshot_wall
                .map(|at| at.elapsed().as_secs_f32())
                .unwrap_or(f32::NAN);
            perform_network_teardown(
                TeardownReason::StaleSnapshot { elapsed_secs },
                &mut commands,
                &mut client_session,
                &mut network_state,
                &mut game_state_snapshot,
                &mut team_selection,
                &mut cam_state,
                overlay_query,
                remote_query,
                projectile_query,
                structure_query,
                minion_query,
                neutral_query,
                player_query,
                *visual_mode,
                &sprite_assets,
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
        ranks: player.ranks,
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

fn default_skill_ranks() -> [u8; 4] {
    [1; 4]
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
    use super::{
        ClientConnectionState, IPV4_UDP_MAX_PAYLOAD_BYTES, SERVER_DATAGRAM_RECEIVE_CAPACITY,
        ServerPacket, T_RETRY, TeardownReason, decode_server_packet,
        forward_complete_server_datagram, should_attempt_reconnect, teardown_shows_select,
        validate_client_payload_size,
    };
    use crate::team::TeamSelection;
    use bevy::prelude::*;
    use serde_json::json;
    use std::{
        net::UdpSocket,
        time::{Duration, Instant},
    };

    fn admission_app() -> (
        App,
        crossbeam_channel::Sender<ServerPacket>,
        crossbeam_channel::Receiver<super::ClientPacket>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (incoming, incoming_rx) = crossbeam_channel::unbounded();
        let (_, signals) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.insert_resource(super::NetworkChannels {
            outgoing: tx,
            incoming: incoming_rx,
            signals,
        })
        .insert_resource(super::ClientSession {
            state: ClientConnectionState::Connected,
            last_join: Some(super::CommittedJoin {
                team: crate::team::Team::Green,
                character: ekza_bevy_sdk::EkzaCharacter::Ipfs,
                hero_class: shared::HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
            }),
            ..default()
        })
        .init_resource::<crate::persistence::ClientSessionId>()
        .init_resource::<super::PendingServerSnapshotFrame>()
        .init_resource::<super::NetIncomingDisconnected>()
        .init_resource::<TeamSelection>()
        .add_systems(
            Update,
            (
                super::ingest_server_snapshot_packets,
                super::retry_pending_join,
            )
                .chain(),
        );
        (app, incoming, rx)
    }

    fn admission_snapshot(
        round: u64,
        tick: u64,
        admitted: bool,
        error: Option<shared::protocol::JoinRejection>,
    ) -> ServerPacket {
        let mut value: serde_json::Value =
            serde_json::from_slice(&populated_snapshot_fixture()).unwrap();
        value["your_id"] = json!(1);
        value["match_id"] = json!(round);
        value["snapshot_tick"] = json!(tick);
        value["join_error"] = json!(error);
        if !admitted {
            value["players"] = json!([]);
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn admitted_snapshot_and_world_fallback_keep_one_local_root_in_the_same_frame() {
        use crate::{
            camera::CameraState,
            maps::MapLayout,
            player::Player,
            sprite::PlayerVisualMode,
            world::{AvatarAssetCache, PlayerAssets, PlayerModelCatalog},
        };
        let (outgoing, _outgoing_rx) = crossbeam_channel::unbounded();
        let (incoming, incoming_rx) = crossbeam_channel::unbounded();
        let (_signal_tx, signals) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .insert_resource(super::NetworkChannels {
                outgoing,
                incoming: incoming_rx,
                signals,
            })
            .insert_resource(super::ClientSession {
                state: ClientConnectionState::Connected,
                ..default()
            })
            .insert_resource(TeamSelection {
                team: Some(crate::team::Team::Green),
                ..default()
            })
            .insert_resource(PlayerVisualMode::Models3d)
            .insert_resource(PlayerAssets {
                scene: None,
                gltf: None,
                mesh: default(),
                material: default(),
            })
            .init_resource::<PlayerModelCatalog>()
            .init_resource::<AvatarAssetCache>()
            .init_resource::<CameraState>()
            .init_resource::<MapLayout>()
            .init_resource::<super::NetworkState>()
            .init_resource::<super::GameStateSnapshot>()
            .init_resource::<super::PendingServerSnapshotFrame>()
            .init_resource::<super::NetIncomingDisconnected>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_systems(Startup, super::setup_network_visual_assets)
            .add_systems(
                Update,
                (
                    super::ingest_server_snapshot_packets
                        .in_set(super::ClientNetPipeline::IngestSnapshot),
                    super::apply_server_snapshot.in_set(super::ClientNetPipeline::ApplySnapshot),
                ),
            );
        super::configure_network_pipeline(&mut app);
        crate::world::register_local_player_spawn(&mut app);
        for tick in 1..=3 {
            let mut snapshot =
                serde_json::to_value(admission_snapshot(1, tick, true, None)).unwrap();
            let mut hero = snapshot["players"][0].clone();
            hero["avatar"] = serde_json::Value::Null;
            snapshot["players"] = json!([hero]);
            for field in ["structures", "minions", "neutrals", "projectiles"] {
                snapshot[field] = json!([]);
            }
            incoming
                .send(serde_json::from_value(snapshot).unwrap())
                .unwrap();
            app.update();
            assert!(
                app.world()
                    .resource::<super::ClientSession>()
                    .join_confirmed()
            );
            let mut players = app
                .world_mut()
                .query_filtered::<Option<&super::NetworkPlayerId>, With<Player>>();
            let ids: Vec<_> = players
                .iter(app.world())
                .map(|id| id.map(|id| id.0))
                .collect();
            assert_eq!(
                ids,
                vec![Some(1)],
                "the admitted snapshot must be the sole creator; no transient untagged world duplicate"
            );
        }
    }

    #[test]
    fn first_join_datagram_loss_retries_until_authoritative_admission() {
        let (mut app, incoming, outgoing) = admission_app();
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut buf = [0; 2048];
        incoming
            .send(admission_snapshot(1, 1, false, None))
            .unwrap();
        app.update();
        sender
            .send_to(
                &serde_json::to_vec(&outgoing.try_recv().unwrap()).unwrap(),
                receiver.local_addr().unwrap(),
            )
            .unwrap();
        let _dropped = receiver.recv(&mut buf).unwrap(); // Deliberately discard first Join at UDP boundary.
        assert!(
            !app.world()
                .resource::<super::ClientSession>()
                .join_confirmed()
        );
        app.update();
        assert!(
            outgoing.try_recv().is_err(),
            "retry interval must be bounded"
        );
        app.world_mut()
            .resource_mut::<super::ClientSession>()
            .join_last_sent = Some(Instant::now() - T_RETRY);
        app.update();
        sender
            .send_to(
                &serde_json::to_vec(&outgoing.try_recv().unwrap()).unwrap(),
                receiver.local_addr().unwrap(),
            )
            .unwrap();
        let len = receiver.recv(&mut buf).unwrap();
        let retry: serde_json::Value = serde_json::from_slice(&buf[..len]).unwrap();
        assert_eq!(retry["type"], "join");
        assert_eq!(retry["team"], "green");
        incoming.send(admission_snapshot(1, 2, true, None)).unwrap();
        app.update();
        assert!(
            app.world()
                .resource::<super::ClientSession>()
                .join_confirmed()
        );
        app.world_mut()
            .resource_mut::<super::ClientSession>()
            .join_last_sent = Some(Instant::now() - T_RETRY);
        app.update();
        assert!(outgoing.try_recv().is_err(), "admission ends retries");
    }

    #[test]
    fn admission_rejection_exhaustion_and_snapshot_order_are_actionable() {
        use shared::protocol::JoinRejection;
        let (mut app, incoming, outgoing) = admission_app();
        for (tick, error) in [
            (1, JoinRejection::MatchFull),
            (2, JoinRejection::SessionActive),
            (3, JoinRejection::ProtocolMismatch),
        ] {
            incoming
                .send(admission_snapshot(1, tick, false, Some(error)))
                .unwrap();
            app.update();
            let session = app.world().resource::<super::ClientSession>();
            assert_eq!(session.join_error, Some(error));
            assert!(!session.join_confirmed());
            assert!(!error.message().is_empty());
            assert!(outgoing.try_recv().is_err());
        }
        incoming.send(admission_snapshot(2, 0, true, None)).unwrap();
        incoming
            .send(admission_snapshot(
                1,
                999,
                false,
                Some(JoinRejection::MatchFull),
            ))
            .unwrap();
        incoming
            .send(admission_snapshot(
                2,
                0,
                false,
                Some(JoinRejection::MatchFull),
            ))
            .unwrap();
        app.update();
        assert!(
            app.world()
                .resource::<super::ClientSession>()
                .join_confirmed(),
            "old round/duplicate cannot undo admission"
        );
        assert!(
            app.world()
                .resource::<super::PendingServerSnapshotFrame>()
                .frame
                .is_some()
        );
        incoming
            .send(admission_snapshot(1, 1000, false, None))
            .unwrap();
        app.update();
        assert!(
            app.world()
                .resource::<super::PendingServerSnapshotFrame>()
                .frame
                .is_none()
        );
        {
            let mut session = app.world_mut().resource_mut::<super::ClientSession>();
            session.clear_join_attempt();
            session.join_attempts = super::MAX_JOIN_ATTEMPTS;
        }
        app.update();
        assert!(
            app.world()
                .resource::<super::ClientSession>()
                .join_exhausted
        );
        assert!(outgoing.try_recv().is_err());
    }

    fn exact_size_snapshot_fixture(size: usize, sentinel: u64) -> Vec<u8> {
        let prefix = String::from(
            r#"{"type":"snapshot","protocol_version":1,"server_epoch":1,"match_id":1,"snapshot_tick":2,"your_id":7,"padding":""#,
        );
        let suffix = format!(
            r#"","players":[],"structures":[{{"id":808,"kind":"tower","team":"blue","x":1.0,"y":2.0,"z":3.0,"hp":4.0,"max_hp":5.0}}],"minions":[{{"id":909,"team":"green","lane":"bot","x":6.0,"y":0.5,"z":7.0,"yaw":0.0,"hp":8.0,"max_hp":9.0,"state":"marching","target_kind":null,"target_id":null}}],"rematch_in_secs":{sentinel}}}"#
        );
        let fixed_len = prefix.len() + suffix.len();
        assert!(
            size >= fixed_len,
            "fixture size {size} is smaller than fixed JSON length {fixed_len}"
        );
        let mut payload = Vec::with_capacity(size);
        payload.extend_from_slice(prefix.as_bytes());
        payload.resize(size - suffix.len(), b'x');
        payload.extend_from_slice(suffix.as_bytes());
        assert_eq!(payload.len(), size);
        payload
    }

    fn assert_fixture_sentinel(payload: &[u8], expected_sentinel: u64) {
        let packet = decode_server_packet(payload).expect("complete fixture should decode");
        let ServerPacket::Snapshot {
            structures,
            minions,
            rematch_in_secs,
            ..
        } = packet;
        assert_eq!(structures.last().map(|structure| structure.id), Some(808));
        assert_eq!(minions.last().map(|minion| minion.id), Some(909));
        assert_eq!(rematch_in_secs, Some(expected_sentinel));
    }

    fn populated_snapshot_fixture() -> Vec<u8> {
        let players = (1_u64..=10)
            .map(|id| {
                json!({
                    "id": id,
                    "x": id as f32 * 1.125,
                    "y": 0.5,
                    "z": id as f32 * -2.25,
                    "yaw": 1.75,
                    "team": if id % 2 == 0 { "blue" } else { "green" },
                    "hp": 100.0,
                    "max_hp": 100.0,
                    "mana": 87.25,
                    "max_mana": 100.0,
                    "gold": 123,
                    "xp": 45,
                    "level": 2,
                    "next_level_xp": 180,
                    "skill_points": 1,
                    "ranks": [1, 1, 1, 1],
                    "character": "ipfs",
                    "hero_class": "warrior",
                    "avatar": "osa-kardialtheconsumer-00bea9121db1",
                    "sprite_character": "cathedral-moth-bellringer",
                    "action_sequence": id,
                    "action_kind": "attack",
                    "action_slot": 0
                })
            })
            .collect::<Vec<_>>();
        let structures = (1_u64..=8)
            .map(|id| {
                json!({
                    "id": id,
                    "kind": if id > 6 { "base_tower" } else { "tower" },
                    "team": if id % 2 == 0 { "blue" } else { "green" },
                    "x": id as f32 * 11.25,
                    "y": 3.0,
                    "z": id as f32 * -9.75,
                    "hp": 240.0,
                    "max_hp": 240.0
                })
            })
            .collect::<Vec<_>>();
        let minions = (1_u64..=18)
            .map(|id| {
                let lane = ["top", "mid", "bot"][(id as usize - 1) % 3];
                json!({
                    "id": id,
                    "team": if id % 2 == 0 { "blue" } else { "green" },
                    "lane": lane,
                    "x": id as f32 * 3.125,
                    "y": 0.5,
                    "z": id as f32 * -4.25,
                    "yaw": 2.75,
                    "hp": 65.0,
                    "max_hp": 65.0,
                    "state": "chasing",
                    "target_kind": "minion",
                    "target_id": id + 100
                })
            })
            .collect::<Vec<_>>();
        let projectiles = (1_u64..=4)
            .map(|id| {
                json!({
                    "id": id,
                    "owner_id": id,
                    "owner_team": if id % 2 == 0 { "blue" } else { "green" },
                    "x": id as f32 * 7.125,
                    "y": 1.35,
                    "z": id as f32 * -8.75
                })
            })
            .collect::<Vec<_>>();
        let neutrals = (1_u64..=5)
            .map(|id| {
                let camp_type = [
                    "skirmisher",
                    "bruiser",
                    "spitter",
                    "wendigo_boss",
                    "king_mutatio_boss",
                ][id as usize - 1];
                json!({
                    "id": 9_000 + id,
                    "camp_type": camp_type,
                    "x": id as f32 * 13.25,
                    "y": 0.7,
                    "z": id as f32 * -14.5,
                    "yaw": 0.25,
                    "hp": 900.0,
                    "max_hp": 1500.0,
                    "ai_state": "idle"
                })
            })
            .collect::<Vec<_>>();

        serde_json::to_vec(&json!({
            "type": "snapshot",
            "protocol_version": 1, "server_epoch": 1, "match_id": 1, "snapshot_tick": 1,
            "your_id": 10,
            "players": players,
            "projectiles": projectiles,
            "structures": structures,
            "minions": minions,
            "neutrals": neutrals,
            "team_buffs": [],
            "game_state": { "type": "running" },
            "rematch_in_secs": 4242
        }))
        .expect("populated fixture should serialize")
    }

    #[test]
    fn teardown_shows_select_only_without_committed_join() {
        assert!(teardown_shows_select(false));
        assert!(!teardown_shows_select(true));
    }

    #[test]
    fn teardown_reasons_render_distinct_messages() {
        let reasons = [
            TeardownReason::StaleSnapshot { elapsed_secs: 3.2 }.to_string(),
            TeardownReason::TransportFailure.to_string(),
            TeardownReason::ServerWaitTimeout.to_string(),
            TeardownReason::IncomingChannelClosed.to_string(),
        ];
        for (index, reason) in reasons.iter().enumerate() {
            assert!(!reason.is_empty());
            for other in reasons.iter().skip(index + 1) {
                assert_ne!(reason, other);
            }
        }
        assert!(reasons[0].contains("3.2"));
    }

    #[test]
    fn reconnect_gate_requires_disconnected_active_and_cooldown() {
        use ClientConnectionState as S;
        // Fires immediately on the first attempt, then respects T_RETRY.
        assert!(should_attempt_reconnect(S::Disconnected, true, None));
        assert!(should_attempt_reconnect(
            S::Disconnected,
            true,
            Some(T_RETRY)
        ));
        assert!(!should_attempt_reconnect(
            S::Disconnected,
            true,
            Some(T_RETRY / 2)
        ));
        // Inactive or non-disconnected states never auto-reconnect.
        assert!(!should_attempt_reconnect(S::Disconnected, false, None));
        assert!(!should_attempt_reconnect(S::Connected, true, None));
        assert!(!should_attempt_reconnect(S::Connecting, true, None));
        assert!(!should_attempt_reconnect(S::WaitingForServer, true, None));
    }

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

    #[test]
    fn old_8_kib_boundary_decodes_complete_trailing_entities() {
        for size in [8_191, 8_192, 8_193] {
            let payload = exact_size_snapshot_fixture(size, size as u64);
            assert_fixture_sentinel(&payload, size as u64);
        }
    }

    #[test]
    fn representative_populated_snapshot_larger_than_8_kib_decodes_completely() {
        let payload = populated_snapshot_fixture();
        assert!(
            payload.len() > 8 * 1024,
            "representative snapshot unexpectedly shrank to {} bytes",
            payload.len()
        );
        let packet = decode_server_packet(&payload).expect("populated snapshot should decode");
        let ServerPacket::Snapshot {
            players,
            projectiles,
            structures,
            minions,
            neutrals,
            rematch_in_secs,
            ..
        } = packet;
        assert_eq!(players.len(), 10);
        assert_eq!(projectiles.len(), 4);
        assert_eq!(structures.last().map(|structure| structure.id), Some(8));
        assert_eq!(minions.last().map(|minion| minion.id), Some(18));
        assert_eq!(neutrals.len(), 5);
        assert_eq!(rematch_in_secs, Some(4242));
    }

    #[test]
    fn production_datagram_forwarding_stages_complete_large_snapshots_only() {
        let (outgoing_tx, _outgoing_rx) = crossbeam_channel::unbounded();
        let (incoming_tx, incoming_rx) = crossbeam_channel::unbounded();
        let (_signal_tx, signal_rx) = crossbeam_channel::unbounded();
        let baseline = Instant::now() - Duration::from_secs(1);
        let mut app = App::new();
        app.insert_resource(super::NetworkChannels {
            outgoing: outgoing_tx,
            incoming: incoming_rx,
            signals: signal_rx,
        })
        .insert_resource(super::ClientSession {
            state: ClientConnectionState::Connected,
            last_qualifying_snapshot_wall: Some(baseline),
            ..default()
        })
        .init_resource::<super::PendingServerSnapshotFrame>()
        .init_resource::<super::NetIncomingDisconnected>()
        .init_resource::<TeamSelection>()
        .add_systems(Update, super::ingest_server_snapshot_packets);

        let populated = populated_snapshot_fixture();
        assert!(populated.len() > 8 * 1024);
        forward_complete_server_datagram(&populated, &incoming_tx)
            .expect("complete populated datagram enters the production channel");
        app.update();
        {
            let pending = app.world().resource::<super::PendingServerSnapshotFrame>();
            let staged = pending.frame.as_ref().expect("large snapshot is staged");
            assert_eq!(
                staged.structures.last().map(|structure| structure.id),
                Some(8)
            );
            assert_eq!(staged.minions.last().map(|minion| minion.id), Some(18));
            assert_eq!(staged.rematch_in_secs, Some(4242));
        }
        assert_eq!(
            app.world()
                .resource::<super::ClientSession>()
                .last_qualifying_snapshot_wall,
            Some(baseline),
            "staging alone must not advance the last-applied snapshot timestamp"
        );

        let malformed = br#"{"type":"snapshot","your_id":7,"players":["#;
        assert!(forward_complete_server_datagram(malformed, &incoming_tx).is_err());
        app.update();
        assert!(
            app.world()
                .resource::<super::PendingServerSnapshotFrame>()
                .frame
                .is_none(),
            "malformed JSON must not publish a partial pending snapshot"
        );
        assert_eq!(
            app.world()
                .resource::<super::ClientSession>()
                .last_qualifying_snapshot_wall,
            Some(baseline),
            "malformed JSON must not advance the qualifying timestamp"
        );

        let recovery = exact_size_snapshot_fixture(9_000, 9_000);
        forward_complete_server_datagram(&recovery, &incoming_tx)
            .expect("a complete datagram after malformed traffic is accepted");
        app.update();
        let pending = app.world().resource::<super::PendingServerSnapshotFrame>();
        let staged = pending.frame.as_ref().expect("recovery snapshot is staged");
        assert_eq!(
            staged.structures.last().map(|structure| structure.id),
            Some(808)
        );
        assert_eq!(staged.minions.last().map(|minion| minion.id), Some(909));
        assert_eq!(staged.rematch_in_secs, Some(9_000));
    }

    #[test]
    fn near_ipv4_limit_decodes_without_prefix_truncation() {
        let payload = exact_size_snapshot_fixture(IPV4_UDP_MAX_PAYLOAD_BYTES, 65_507);
        assert_fixture_sentinel(&payload, 65_507);
    }

    #[test]
    fn outbound_payload_guard_uses_ipv4_udp_ceiling() {
        assert!(validate_client_payload_size(IPV4_UDP_MAX_PAYLOAD_BYTES).is_ok());
        let error = validate_client_payload_size(IPV4_UDP_MAX_PAYLOAD_BYTES + 1).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn loopback_receives_8191_8192_8193_and_malformed_between_good_datagrams() {
        let receiver = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("set receiver timeout");
        let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender");
        sender.connect(receiver.local_addr().unwrap()).unwrap();

        let datagrams = [
            exact_size_snapshot_fixture(8_191, 1),
            exact_size_snapshot_fixture(8_192, 2),
            exact_size_snapshot_fixture(8_193, 3),
            br#"{"type":"snapshot","your_id":7,"players":["#.to_vec(),
            exact_size_snapshot_fixture(9_000, 4),
        ];
        for payload in &datagrams {
            assert_eq!(sender.send(payload).expect("loopback send"), payload.len());
        }

        let mut receive_storage = vec![0_u8; SERVER_DATAGRAM_RECEIVE_CAPACITY];
        let mut decoded_sentinels = Vec::new();
        for expected_len in datagrams.iter().map(Vec::len) {
            let len = receiver
                .recv(&mut receive_storage)
                .expect("loopback receive should complete");
            assert_eq!(len, expected_len);
            if let Ok(ServerPacket::Snapshot {
                rematch_in_secs, ..
            }) = decode_server_packet(&receive_storage[..len])
            {
                decoded_sentinels.push(rematch_in_secs.expect("fixture sentinel"));
            }
        }
        assert_eq!(decoded_sentinels, [1, 2, 3, 4]);
    }

    #[test]
    fn platform_near_limit_loopback_behavior_is_explicit() {
        let receiver = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("set receiver timeout");
        let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender");
        sender.connect(receiver.local_addr().unwrap()).unwrap();
        let payload = exact_size_snapshot_fixture(IPV4_UDP_MAX_PAYLOAD_BYTES, 55);

        match sender.send(&payload) {
            Ok(len) => {
                assert_eq!(len, payload.len());
                let mut storage = vec![0_u8; SERVER_DATAGRAM_RECEIVE_CAPACITY];
                let received = receiver
                    .recv(&mut storage)
                    .expect("receive near-limit payload");
                assert_eq!(received, payload.len());
                assert_fixture_sentinel(&storage[..received], 55);
            }
            Err(error) if cfg!(target_os = "macos") => {
                // The task's Darwin runner was measured at a 9,216-byte
                // send ceiling. errno 40 (EMSGSIZE) is that lower kernel
                // sender limit, not receive truncation; this change does not
                // claim to raise or bypass it.
                assert_eq!(
                    error.raw_os_error(),
                    Some(40),
                    "unexpected macOS error: {error}"
                );
            }
            Err(error) => panic!("legal IPv4 UDP payload failed on loopback: {error}"),
        }
    }
}
