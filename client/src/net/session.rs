//! Session controller: connection states, join bookkeeping, teardown and reconnect.

use bevy::prelude::*;
use std::time::{Duration, Instant};

use shared::HeroClass;
use shared::protocol::{JoinRejection, SnapshotOrder};
use shared::wire::ClientPacket;

use crate::camera::CameraState;
use crate::domain::RoundId;
use crate::persistence::{ClientSessionId, FileGameServerAddr};
use crate::player::Player;
use crate::session_config::{
    DEFAULT_GAME_SERVER_ADDR, T_RETRY, T_STALE_SNAPSHOT, T_WAIT_MAX, is_stale,
};
use crate::team::{CharacterChoice, Team, TeamSelection};

use super::components::{
    GameStateSnapshot, NetworkMinion, NetworkNeutral, NetworkProjectile, NetworkState,
    NetworkStructure, RemotePlayer,
};
use super::ingest::PendingServerSnapshotFrame;
use super::offline;
use super::transport::{NetThreadSignal, NetworkChannels, spawn_network_transport};

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

#[derive(Message, Clone, Debug)]
pub enum SessionUiCommand {
    /// User explicitly resumes waiting for snapshots after **Disconnected** (P2 manual recovery).
    Retry,
    /// Replace remote I/O with an in-process, unrated character practice.
    StartOffline,
    /// Validated address chosen in the pre-join UI. Never transfers an active match.
    ConnectTo(String),
    /// Switch to a trusted lobby allocation without overwriting the saved lobby.
    ConnectAllocated(String),
    /// Leave the current match or queue and return to the front end. The
    /// server is told to release the seat; the connection stays up.
    LeaveMatch,
}

/// A lifecycle edge of the session, announced once when it happens.
///
/// `net` queues these in `ClientSession.outbox` wherever the edge is taken
/// (plain functions and systems at the 16-parameter limit alike) and
/// `flush_session_events` writes them as messages at the end of
/// `ClientNetPipeline::ApplySnapshot` and again at the end of
/// `ClientNetPipeline::SessionLifecycle`. Readers that react in the same frame
/// belong in [`super::SessionReactions`]. Level-based readers (`join_confirmed`,
/// `is_connected`, ...) stay the source of truth for "what is the state now".
#[derive(Message, Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// A new transport (UDP thread or in-process practice) replaced the old one.
    TransportStarted { addr: String, offline: bool },
    /// The first qualifying snapshot of a connect attempt was applied.
    Connected,
    /// The server lists this client in a snapshot of a connected session
    /// (`join_confirmed()` became true), once per join attempt.
    Joined { your_id: u64 },
    /// The server (or the client's own admission check) refused the join; sent
    /// when the rejection changes to a new value.
    Rejected(JoinRejection),
    /// The join retry budget is spent (or the join could not be sent).
    JoinExhausted,
    /// The session was torn down; `reconnecting` says whether the committed
    /// join is kept and the auto-reconnect loop will retry it.
    Disconnected {
        reason: TeardownReason,
        reconnecting: bool,
    },
    /// The player left the match or the queue; `returning_to` is the lobby
    /// address the client reconnects to, if any. The front end goes Home.
    Left { returning_to: Option<String> },
    /// Career and social views belong to the previous server and are stale;
    /// `CareerClient` and `SocialClient` clear themselves in
    /// `SessionReactions`, before the next frame's ingest.
    ServerScopeReset,
    /// A snapshot of a different round than the last one was applied. Rounds
    /// with a zero epoch or match id are skipped and a reconnect to the same
    /// round is not a change. Read after `ApplySnapshot` (same frame) by the
    /// combat round reset and the mobile-controls clear.
    RoundChanged { previous: RoundId, current: RoundId },
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
    pub prematch: bool,
    pub team: Team,
    pub character: CharacterChoice,
    pub hero_class: HeroClass,
    pub avatar: Option<String>,
    pub sprite_character: Option<String>,
}

#[cfg(test)]
impl CommittedJoin {
    pub(crate) fn for_test() -> Self {
        Self {
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::default(),
            hero_class: HeroClass::default(),
            avatar: None,
            sprite_character: None,
        }
    }
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
    pub(in crate::net) state: ClientConnectionState,
    /// Wall time when **WaitingForServer** began for the current attempt.
    pub(in crate::net) waiting_since: Option<Instant>,
    /// Last time a qualifying snapshot was applied while **Connected**.
    pub(in crate::net) last_qualifying_snapshot_wall: Option<Instant>,
    /// While true, incoming snapshots are drained and ignored (no silent re-entry to gameplay).
    pub(in crate::net) discard_incoming_snapshots: bool,
    /// **P4**: after a join packet is sent, suppress duplicate join until teardown.
    pub(in crate::net) join_flow_committed: bool,
    /// Server address used for this process (for UI copy).
    pub(in crate::net) server_addr_display: String,
    /// Loadout of the last committed join (auto-rejoin after reconnect).
    pub(in crate::net) last_join: Option<CommittedJoin>,
    /// Auto-reconnect loop state (active after a teardown of a joined session).
    pub(in crate::net) reconnect: ReconnectState,
    pub(in crate::net) admitted: bool,
    pub(in crate::net) join_last_sent: Option<Instant>,
    pub(in crate::net) join_attempts: u32,
    pub(in crate::net) join_error: Option<JoinRejection>,
    pub(in crate::net) join_exhausted: bool,
    pub(in crate::net) snapshot_order: SnapshotOrder,
    pub(in crate::net) career_server_epoch: u64,
    pub(in crate::net) career_packet_sequence: u64,
    pub(in crate::net) ephemeral_endpoint: bool,
    pub(in crate::net) offline_return_addr: Option<String>,
    /// `SessionEvent::Joined` was sent for the current join attempt.
    pub(in crate::net) announced_join: bool,
    /// Lifecycle edges queued for `flush_session_events`.
    pub(in crate::net) outbox: Vec<SessionEvent>,
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
            career_server_epoch: 0,
            career_packet_sequence: 0,
            ephemeral_endpoint: false,
            offline_return_addr: None,
            announced_join: false,
            outbox: Vec::new(),
        }
    }
}

/// Why a network teardown happened; logged so surprise disconnects are
/// diagnosable from the client log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TeardownReason {
    StaleSnapshot { elapsed_secs: f32 },
    TransportFailure,
    ServerWaitTimeout,
    IncomingChannelClosed,
    ProtocolMismatch,
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
            Self::ProtocolMismatch => write!(f, "client and server protocol versions differ"),
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
    pub(crate) fn is_offline(&self) -> bool {
        self.offline_return_addr.is_some()
    }

    #[cfg(test)]
    pub(crate) fn admitted_for_test() -> Self {
        Self {
            state: ClientConnectionState::Connected,
            admitted: true,
            join_flow_committed: true,
            last_join: Some(CommittedJoin::for_test()),
            ..default()
        }
    }

    /// A joined session right after a transport teardown: the join is still
    /// the player's intent, the connection is gone, the hero is despawned.
    #[cfg(test)]
    pub(crate) fn reconnecting_for_test() -> Self {
        Self {
            state: ClientConnectionState::Disconnected,
            admitted: false,
            join_flow_committed: false,
            last_join: Some(CommittedJoin::for_test()),
            reconnect: ReconnectState {
                active: true,
                attempts: 0,
                last_attempt: None,
            },
            ..default()
        }
    }

    /// Locked in, waiting for the server: what the picker leaves behind.
    #[cfg(test)]
    pub(crate) fn queued_for_test() -> Self {
        Self {
            state: ClientConnectionState::Connected,
            join_flow_committed: true,
            last_join: Some(CommittedJoin::for_test()),
            ..default()
        }
    }

    #[cfg(test)]
    pub(crate) fn reject_for_test(&mut self, rejection: JoinRejection) {
        self.join_error = Some(rejection);
    }

    /// Overrides the connection state (test fixtures only).
    #[cfg(test)]
    pub(crate) fn set_state_for_test(&mut self, state: ClientConnectionState) {
        self.state = state;
    }

    /// Overrides the in-flight join flag (test fixtures only).
    #[cfg(test)]
    pub(crate) fn set_join_in_flight_for_test(&mut self, in_flight: bool) {
        self.join_flow_committed = in_flight;
    }

    /// Overrides the server address shown in the UI (test fixtures only).
    #[cfg(test)]
    pub(crate) fn set_server_addr_for_test(&mut self, addr: impl Into<String>) {
        self.server_addr_display = addr.into();
    }

    /// Marks the committed join as a prematch join or not (test fixtures
    /// only; panics without a committed join).
    #[cfg(test)]
    pub(crate) fn set_joined_prematch_for_test(&mut self, prematch: bool) {
        self.last_join.as_mut().expect("a committed join").prematch = prematch;
    }

    /// Drops the committed join without touching anything else (test
    /// fixtures only; production code uses `abandon_join`).
    #[cfg(test)]
    pub(crate) fn clear_last_join_for_test(&mut self) {
        self.last_join = None;
    }

    /// Connection lifecycle state.
    pub fn state(&self) -> ClientConnectionState {
        self.state
    }

    /// A join packet was sent for the current attempt and duplicates are
    /// suppressed until teardown (`join_flow_committed`).
    pub fn join_in_flight(&self) -> bool {
        self.join_flow_committed
    }

    /// The server address of this session (for UI copy and signing scope).
    pub fn server_addr(&self) -> &str {
        &self.server_addr_display
    }

    /// The committed join is a prematch (draft) join.
    pub fn joined_prematch(&self) -> bool {
        self.last_join.as_ref().is_some_and(|join| join.prematch)
    }

    pub fn join_confirmed(&self) -> bool {
        self.is_connected() && self.admitted
    }

    /// A join the client still stands behind: locked in, and neither left nor
    /// abandoned. It survives a transport teardown, which is what tells a
    /// reconnect apart from "the player is back in the menus".
    pub fn has_committed_join(&self) -> bool {
        self.last_join.is_some()
    }

    /// Gives up the current join locally: no more retries, the lock-in works
    /// again, and a stale snapshot cannot count as an admission.
    pub fn abandon_join(&mut self) {
        self.last_join = None;
        self.join_flow_committed = false;
        self.reconnect = ReconnectState::default();
        self.clear_join_attempt();
    }

    /// The rejection the server returned for the last join attempt.
    pub fn join_rejection(&self) -> Option<JoinRejection> {
        self.join_error
    }

    /// A committed join that can no longer make progress on its own: the
    /// server rejected it, or the retry budget is spent.
    pub fn join_blocked(&self) -> bool {
        self.join_error.is_some() || self.join_exhausted
    }

    pub(crate) fn is_choosing_loadout(&self) -> bool {
        self.is_connected()
            && !self.join_flow_committed
            && self.join_error.is_none()
            && !self.join_exhausted
    }

    pub(in crate::net) fn clear_join_attempt(&mut self) {
        crate::passport::clear_tickets();
        self.admitted = false;
        self.join_last_sent = None;
        self.join_attempts = 0;
        self.join_error = None;
        self.join_exhausted = false;
        self.announced_join = false;
    }

    /// Records the join rejection and announces it when it changes to a new
    /// `Some` value (a repeated rejection in every snapshot is one event).
    pub(in crate::net) fn set_join_error(&mut self, error: Option<JoinRejection>) {
        if let Some(rejection) = error
            && self.join_error != error
        {
            self.outbox.push(SessionEvent::Rejected(rejection));
        }
        self.join_error = error;
    }

    /// Marks the join retry budget as spent, announcing the edge once.
    fn exhaust_join(&mut self) {
        if !self.join_exhausted {
            self.outbox.push(SessionEvent::JoinExhausted);
        }
        self.join_exhausted = true;
    }

    /// Stops the auto-reconnect loop. A teardown announced in the same frame
    /// no longer reconnects, so its queued event says so.
    fn stop_reconnecting(&mut self) {
        self.reconnect = ReconnectState::default();
        for event in &mut self.outbox {
            if let SessionEvent::Disconnected { reconnecting, .. } = event {
                *reconnecting = false;
            }
        }
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

pub(in crate::net) fn start_networking(
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
    if std::env::var("OMOBA_OFFLINE_PRACTICE").is_ok_and(|v| v == "1") {
        client_session.offline_return_addr = Some(server_addr);
        client_session.ephemeral_endpoint = true;
        spawn_network_transport(&mut commands, &mut client_session, offline::ADDRESS.into());
    } else {
        spawn_network_transport(&mut commands, &mut client_session, server_addr);
    }
}

fn validated_server_addr_or_default(raw: &str) -> String {
    crate::persistence::validate_game_server_addr(raw)
        .or_else(|| crate::persistence::validate_game_server_addr(DEFAULT_GAME_SERVER_ADDR))
        .unwrap_or_else(|| "127.0.0.1:4000".to_owned())
}

pub(in crate::net) fn retry_pending_join(
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

/// Writes the queued [`SessionEvent`]s as messages. Runs at the end of
/// `ClientNetPipeline::ApplySnapshot` (snapshot edges, transport starts from
/// `Startup`) and chained after `retry_pending_join` in
/// `ClientNetPipeline::SessionLifecycle` (lifecycle commands, teardowns and the
/// join attempts sent from `SendCommands`).
pub(in crate::net) fn flush_session_events(
    mut session: ResMut<ClientSession>,
    mut events: MessageWriter<SessionEvent>,
) {
    if session.outbox.is_empty() {
        return;
    }
    // Draining the queue is bookkeeping, not a session change.
    events.write_batch(session.bypass_change_detection().outbox.drain(..));
}

pub(in crate::net) const MAX_JOIN_ATTEMPTS: u32 = 15;

pub(in crate::net) fn send_join_attempt(
    channels: &NetworkChannels,
    session: &mut ClientSession,
    session_id: &ClientSessionId,
) {
    let Some(join) = session.last_join.clone() else {
        return;
    };
    if session.join_attempts >= MAX_JOIN_ATTEMPTS {
        session.exhaust_join();
        return;
    }
    let passport_ticket = if session.is_offline() {
        None
    } else {
        match crate::passport::ticket_for_slug(join.avatar.as_deref(), &session_id.0) {
            crate::passport::TicketPoll::Free => None,
            crate::passport::TicketPoll::Ready(ticket) => Some(ticket),
            crate::passport::TicketPoll::Pending => {
                session.join_last_sent = Some(Instant::now());
                return;
            }
            crate::passport::TicketPoll::Denied(error) => {
                warn!("Purchased avatar admission unavailable: {error}");
                session.set_join_error(Some(JoinRejection::AvatarNotAuthorized));
                session.join_exhausted = true;
                return;
            }
        }
    };
    let result = channels.outgoing.try_send(ClientPacket::Join {
        prematch: join.prematch,
        team: join.team.into(),
        character: join.character,
        hero_class: join.hero_class,
        avatar: join.avatar,
        sprite_character: join.sprite_character,
        session_id: Some(session_id.0.clone()),
        passport_ticket,
    });
    session.join_last_sent = Some(Instant::now());
    session.join_attempts += 1;
    session.join_flow_committed = true;
    if result.is_err() {
        session.exhaust_join();
    }
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

pub(in crate::net) fn perform_network_teardown(
    reason: TeardownReason,
    commands: &mut Commands,
    client_session: &mut ClientSession,
    network_state: &mut NetworkState,
    game_state_snapshot: &mut GameStateSnapshot,
    team_selection: &mut TeamSelection,
    cam_state: &mut CameraState,
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

    warn!("Network teardown: {reason}");
    if teardown_shows_select(client_session.last_join.is_some()) {
        // Nothing was committed: the player is somewhere in the front end and
        // stays there. Screens belong to the shell, not to the transport.
        team_selection.team = None;
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
    let reconnecting = client_session.reconnect.active;
    client_session.outbox.push(SessionEvent::Disconnected {
        reason,
        reconnecting,
    });
}

/// Entity queries a network teardown needs, grouped (Bevy caps system
/// functions at 16 parameters).
#[derive(bevy::ecs::system::SystemParam)]
pub(in crate::net) struct TeardownQueries<'w, 's> {
    pub(in crate::net) remote_query: Query<'w, 's, Entity, With<RemotePlayer>>,
    pub(in crate::net) projectile_query: Query<'w, 's, Entity, With<NetworkProjectile>>,
    pub(in crate::net) structure_query: Query<'w, 's, Entity, With<NetworkStructure>>,
    pub(in crate::net) minion_query: Query<'w, 's, Entity, With<NetworkMinion>>,
    pub(in crate::net) neutral_query: Query<'w, 's, Entity, With<NetworkNeutral>>,
    pub(in crate::net) player_query: Query<'w, 's, Entity, With<Player>>,
}

/// Session commands from the UI, the auto-reconnect and the teardowns. Other
/// modules react to the [`SessionEvent`]s it queues (career and social clear
/// on `ServerScopeReset`, the front end goes Home on `Left`, all in
/// `SessionReactions`). Three outside writes stay here on purpose:
/// `TeamSelection.team = None` is join intent (an in-flight snapshot must not
/// respawn the hero), and `LeaveMatch` needs the lobby address from
/// `MatchServiceClient::take_return_to_lobby` and `CareerIdentity` to sign
/// `CancelQueue` before it sends `Leave`.
pub(in crate::net) fn update_session_lifecycle(
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
    mut match_service: Option<ResMut<crate::match_service::MatchServiceClient>>,
    mut career_identity: Option<ResMut<crate::career_identity::CareerIdentity>>,
    session_id: Option<Res<ClientSessionId>>,
) {
    let TeardownQueries {
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
            SessionUiCommand::StartOffline => {
                if client_session.has_committed_join() || client_session.is_offline() {
                    continue;
                }
                if let Some(channels) = channels.as_ref() {
                    let _ = channels.outgoing.try_send(ClientPacket::Leave);
                }
                client_session.offline_return_addr =
                    Some(client_session.server_addr_display.clone());
                client_session.ephemeral_endpoint = true;
                client_session.abandon_join();
                team_selection.team = None;
                // Practice only uses avatars bundled with the app; no wallet or download.
                if !offline::shipped_avatar(team_selection.avatar.as_deref()) {
                    team_selection.avatar = None;
                }
                despawn_tracked_net_entities(
                    &mut commands,
                    &mut network_state,
                    remote_query,
                    projectile_query,
                    structure_query,
                    minion_query,
                    neutral_query,
                );
                despawn_local_players(&mut commands, player_query);
                *game_state_snapshot = GameStateSnapshot::default();
                commands.insert_resource(PendingServerSnapshotFrame::default());
                if let Some(service) = match_service.as_mut() {
                    service.take_return_to_lobby();
                }
                client_session.outbox.push(SessionEvent::ServerScopeReset);
                spawn_network_transport(
                    &mut commands,
                    &mut client_session,
                    offline::ADDRESS.into(),
                );
                incoming_dead.0 = false;
                retried_this_frame = true;
            }
            SessionUiCommand::ConnectTo(raw) => {
                if client_session.join_flow_committed || client_session.last_join.is_some() {
                    continue;
                }
                let Some(address) = crate::persistence::validate_game_server_addr(raw) else {
                    continue;
                };
                // Checked before `spawn_network_transport` overwrites the address.
                if address != client_session.server_addr_display {
                    client_session.outbox.push(SessionEvent::ServerScopeReset);
                }
                client_session.offline_return_addr = None;
                client_session.ephemeral_endpoint = false;
                commands.remove_resource::<NetworkChannels>();
                spawn_network_transport(&mut commands, &mut client_session, address);
                incoming_dead.0 = false;
                retried_this_frame = true;
            }
            SessionUiCommand::ConnectAllocated(raw) => {
                let Some(address) = crate::persistence::validate_game_server_addr(raw) else {
                    continue;
                };
                client_session.abandon_join();
                despawn_tracked_net_entities(
                    &mut commands,
                    &mut network_state,
                    remote_query,
                    projectile_query,
                    structure_query,
                    minion_query,
                    neutral_query,
                );
                despawn_local_players(&mut commands, player_query);
                *game_state_snapshot = GameStateSnapshot::default();
                commands.insert_resource(PendingServerSnapshotFrame::default());
                client_session.outbox.push(SessionEvent::ServerScopeReset);
                cam_state.locked = false;
                client_session.ephemeral_endpoint = true;
                commands.remove_resource::<NetworkChannels>();
                spawn_network_transport(&mut commands, &mut client_session, address);
                incoming_dead.0 = false;
                retried_this_frame = true;
            }
            SessionUiCommand::LeaveMatch => {
                if match_service.as_ref().is_some_and(|service| service.active)
                    && let (Some(channels), Some(identity), Some(session_id)) = (
                        channels.as_ref(),
                        career_identity.as_mut(),
                        session_id.as_ref(),
                    )
                    && let Ok(request) = identity.prepare_request(
                        &shared::career::CareerRequest::CancelQueue,
                        &client_session.server_addr_display,
                        game_state_snapshot.meta.server_epoch,
                        &session_id.0,
                    )
                {
                    let _ = channels.outgoing.try_send(ClientPacket::Career { request });
                }
                let return_to_lobby = client_session.offline_return_addr.take().or_else(|| {
                    match_service
                        .as_mut()
                        .and_then(|service| service.take_return_to_lobby())
                });
                // The server releases the seat (or the queue entry) on `Leave`;
                // the connection itself stays up for the menus and the career.
                if let Some(channels) = channels.as_ref() {
                    let _ = channels.outgoing.try_send(ClientPacket::Leave);
                }
                client_session.abandon_join();
                client_session.outbox.push(SessionEvent::Left {
                    returning_to: return_to_lobby.clone(),
                });
                // Snapshots already in flight may still list this player; with
                // no team selected they cannot respawn the local hero.
                team_selection.team = None;
                despawn_local_players(&mut commands, player_query);
                cam_state.locked = false;
                if let Some(address) = return_to_lobby {
                    despawn_tracked_net_entities(
                        &mut commands,
                        &mut network_state,
                        remote_query,
                        projectile_query,
                        structure_query,
                        minion_query,
                        neutral_query,
                    );
                    *game_state_snapshot = GameStateSnapshot::default();
                    commands.insert_resource(PendingServerSnapshotFrame::default());
                    client_session.outbox.push(SessionEvent::ServerScopeReset);
                    client_session.ephemeral_endpoint = false;
                    commands.remove_resource::<NetworkChannels>();
                    spawn_network_transport(&mut commands, &mut client_session, address);
                    incoming_dead.0 = false;
                    retried_this_frame = true;
                }
            }
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
                remote_query,
                projectile_query,
                structure_query,
                minion_query,
                neutral_query,
                player_query,
            );
        }
    }

    while let Ok(signal) = channels.signals.try_recv() {
        match signal {
            NetThreadSignal::ProtocolMismatch => {
                if client_session.state != ClientConnectionState::Disconnected {
                    perform_network_teardown(
                        TeardownReason::ProtocolMismatch,
                        &mut commands,
                        &mut client_session,
                        &mut network_state,
                        &mut game_state_snapshot,
                        &mut team_selection,
                        &mut cam_state,
                        remote_query,
                        projectile_query,
                        structure_query,
                        minion_query,
                        neutral_query,
                        player_query,
                    );
                }
                // Retrying the same incompatible release cannot recover a
                // joined session. Preserve the actionable message until Retry.
                client_session.set_join_error(Some(JoinRejection::ProtocolMismatch));
                client_session.stop_reconnecting();
            }
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
                        remote_query,
                        projectile_query,
                        structure_query,
                        minion_query,
                        neutral_query,
                        player_query,
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
                    remote_query,
                    projectile_query,
                    structure_query,
                    minion_query,
                    neutral_query,
                    player_query,
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
                remote_query,
                projectile_query,
                structure_query,
                minion_query,
                neutral_query,
                player_query,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::apply::{SnapshotApplied, StagedSnapshot, snapshot_apply_systems};
    use crate::net::ingest::ingest_server_snapshot_packets;
    use crate::net::status_ui::{
        ConnectionRetryButton, ConnectionStatusLabel, setup_connection_status_ui,
        sync_connection_status_ui,
    };
    use crate::net::test_fixtures::*;
    use crate::net::transport::run_udp_client;
    use crate::persistence::ResolvedServerAddressForPrefs;
    use crate::sprite::{PlayerVisualMode, SpriteVisualAssets};
    use crate::world::PlayerAssets;
    use shared::protocol::PROTOCOL_VERSION;
    use std::net::UdpSocket;

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
        assert!(!app.world().resource::<ClientSession>().join_confirmed());
        app.update();
        assert!(
            outgoing.try_recv().is_err(),
            "retry interval must be bounded"
        );
        app.world_mut()
            .resource_mut::<ClientSession>()
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
        assert!(app.world().resource::<ClientSession>().join_confirmed());
        app.world_mut()
            .resource_mut::<ClientSession>()
            .join_last_sent = Some(Instant::now() - T_RETRY);
        app.update();
        assert!(outgoing.try_recv().is_err(), "admission ends retries");
    }

    // Session events (roadmap step 15a): emitted through the outbox, written
    // as messages by the flush at the end of `ApplySnapshot`.

    #[test]
    fn first_admitted_snapshot_announces_connected_before_joined_once() {
        let (mut app, incoming) = snapshot_app();
        app.world_mut().resource_mut::<ClientSession>().state =
            ClientConnectionState::WaitingForServer;
        incoming.send(admission_snapshot(1, 1, true, None)).unwrap();
        app.update();
        assert_eq!(
            drain_session_events(&mut app),
            vec![SessionEvent::Connected, SessionEvent::Joined { your_id: 1 }]
        );
        incoming.send(admission_snapshot(1, 2, true, None)).unwrap();
        app.update();
        assert_eq!(drain_session_events(&mut app), Vec::new(), "edges only");
    }

    #[test]
    fn teardown_of_a_committed_join_announces_a_reconnecting_disconnect() {
        let (mut app, incoming) = snapshot_app();
        app.world_mut().resource_mut::<ClientSession>().last_join = Some(CommittedJoin::for_test());
        incoming.send(admission_snapshot(1, 1, true, None)).unwrap();
        app.update();
        drain_session_events(&mut app);
        tear_down(&mut app, TeardownReason::TransportFailure);
        app.update();
        assert_eq!(
            drain_session_events(&mut app),
            vec![SessionEvent::Disconnected {
                reason: TeardownReason::TransportFailure,
                reconnecting: true,
            }]
        );
    }

    #[test]
    fn reconnect_to_the_same_round_is_no_round_change_but_a_new_match_is() {
        let (mut app, incoming) = snapshot_app();
        app.world_mut().resource_mut::<ClientSession>().last_join = Some(CommittedJoin::for_test());
        incoming.send(admission_snapshot(1, 1, true, None)).unwrap();
        app.update();
        tear_down(&mut app, TeardownReason::TransportFailure);
        // What a reconnect's transport swap does to the session, on the same
        // channels (`spawn_network_transport` would start a UDP thread).
        {
            let mut session = app.world_mut().resource_mut::<ClientSession>();
            session.discard_incoming_snapshots = false;
            session.state = ClientConnectionState::WaitingForServer;
        }
        app.update();
        drain_session_events(&mut app);
        incoming.send(admission_snapshot(1, 2, true, None)).unwrap();
        app.update();
        assert_eq!(
            drain_session_events(&mut app),
            vec![SessionEvent::Connected, SessionEvent::Joined { your_id: 1 }],
            "the same round after a teardown is not a round change"
        );
        incoming.send(admission_snapshot(2, 1, true, None)).unwrap();
        app.update();
        let round = |match_id| RoundId {
            server_epoch: 1,
            match_id,
        };
        assert_eq!(
            drain_session_events(&mut app),
            vec![SessionEvent::RoundChanged {
                previous: round(1),
                current: round(2),
            }]
        );
    }

    #[test]
    fn map_geometry_mismatch_announces_one_rejection() {
        let (mut app, incoming) = snapshot_app();
        let mut snapshot = serde_json::to_value(admission_snapshot(1, 1, true, None)).unwrap();
        snapshot["geometry_id"] = serde_json::json!("another-map");
        for tick in 1..=2 {
            snapshot["snapshot_tick"] = serde_json::json!(tick);
            incoming
                .send(serde_json::from_value(snapshot.clone()).unwrap())
                .unwrap();
            app.update();
            let expected = if tick == 1 {
                vec![SessionEvent::Rejected(JoinRejection::MapGeometryMismatch)]
            } else {
                Vec::new()
            };
            assert_eq!(drain_session_events(&mut app), expected);
        }
        assert_eq!(
            app.world().resource::<ClientSession>().join_rejection(),
            Some(JoinRejection::MapGeometryMismatch)
        );
    }

    #[test]
    fn protocol_mismatch_teardown_announces_no_reconnect_then_the_rejection() {
        let (outgoing, _outgoing_rx) = crossbeam_channel::unbounded();
        let (_incoming_tx, incoming) = crossbeam_channel::unbounded();
        let (signals_tx, signals) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.insert_resource(NetworkChannels {
            gameplay_signer: Default::default(),
            outgoing,
            incoming,
            signals,
        })
        .insert_resource(ClientSession::admitted_for_test())
        .init_resource::<NetIncomingDisconnected>()
        .init_resource::<PendingServerSnapshotFrame>()
        .init_resource::<NetworkState>()
        .init_resource::<GameStateSnapshot>()
        .init_resource::<TeamSelection>()
        .init_resource::<CameraState>()
        .add_message::<SessionUiCommand>()
        .add_message::<SessionEvent>()
        .add_systems(
            Update,
            (update_session_lifecycle, flush_session_events).chain(),
        );
        signals_tx.send(NetThreadSignal::ProtocolMismatch).unwrap();
        app.update();
        assert_eq!(
            drain_session_events(&mut app),
            vec![
                SessionEvent::Disconnected {
                    reason: TeardownReason::ProtocolMismatch,
                    reconnecting: false,
                },
                SessionEvent::Rejected(JoinRejection::ProtocolMismatch),
            ]
        );
        assert!(!app.world().resource::<ClientSession>().reconnect.active);
    }

    #[test]
    fn connect_and_leave_announce_scope_reset_transport_and_left() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap().to_string();
        let mut app = App::new();
        app.insert_resource(ClientSession {
            state: ClientConnectionState::Connected,
            server_addr_display: "127.0.0.1:9".into(),
            ..default()
        })
        .init_resource::<NetIncomingDisconnected>()
        .init_resource::<PendingServerSnapshotFrame>()
        .init_resource::<NetworkState>()
        .init_resource::<GameStateSnapshot>()
        .init_resource::<TeamSelection>()
        .init_resource::<CameraState>()
        .add_message::<SessionUiCommand>()
        .add_message::<SessionEvent>()
        .add_systems(
            Update,
            (update_session_lifecycle, flush_session_events).chain(),
        );
        app.world_mut()
            .write_message(SessionUiCommand::ConnectTo(address.clone()));
        app.update();
        assert_eq!(
            drain_session_events(&mut app),
            vec![
                SessionEvent::ServerScopeReset,
                SessionEvent::TransportStarted {
                    addr: address.clone(),
                    offline: false,
                },
            ]
        );
        app.world_mut().write_message(SessionUiCommand::LeaveMatch);
        app.update();
        assert_eq!(
            drain_session_events(&mut app),
            vec![SessionEvent::Left { returning_to: None }]
        );
    }

    // Session reactions (roadmap step 15d, hazard 7): consumers of
    // `ServerScopeReset` run in `SessionReactions`, in the frame of the reset.

    #[test]
    fn scope_reset_clears_career_and_social_in_its_frame_so_the_next_view_survives() {
        use crate::career::CareerClient;
        use crate::net::{ClientNetPipeline, SessionReactions, configure_network_pipeline};
        use crate::social::SocialClient;
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap().to_string();
        let mut career = CareerClient::default();
        career.public_profile_id = Some("old-server-profile".into());
        let mut social = SocialClient::default();
        social.chat_open = true;
        let mut app = App::new();
        app.insert_resource(ClientSession {
            state: ClientConnectionState::Connected,
            server_addr_display: "127.0.0.1:9".into(),
            career_server_epoch: 7,
            ..default()
        })
        .insert_resource(career)
        .insert_resource(social)
        .init_resource::<NetIncomingDisconnected>()
        .init_resource::<PendingServerSnapshotFrame>()
        .init_resource::<NetworkState>()
        .init_resource::<GameStateSnapshot>()
        .init_resource::<TeamSelection>()
        .init_resource::<CameraState>()
        .add_message::<SessionUiCommand>()
        .add_message::<SessionEvent>()
        .add_systems(
            Update,
            (
                ingest_server_snapshot_packets.in_set(ClientNetPipeline::IngestSnapshot),
                (update_session_lifecycle, flush_session_events)
                    .chain()
                    .in_set(ClientNetPipeline::SessionLifecycle),
                crate::career::clear_account_on_scope_reset.in_set(SessionReactions),
                crate::social::clear_on_scope_reset.in_set(SessionReactions),
            ),
        );
        configure_network_pipeline(&mut app);

        app.world_mut()
            .write_message(SessionUiCommand::ConnectTo(address));
        app.update();
        assert!(
            app.world()
                .resource::<CareerClient>()
                .public_profile_id
                .is_none(),
            "the old server's account is cleared in the frame of the reset"
        );
        assert!(!app.world().resource::<SocialClient>().chat_open);

        // The next frame's ingest applies the new server's first career view
        // (on the new transport's channels; the test stands in for its thread).
        let (outgoing, _outgoing_rx) = crossbeam_channel::unbounded();
        let (incoming_tx, incoming) = crossbeam_channel::unbounded();
        let (_signals_tx, signals) = crossbeam_channel::unbounded();
        app.insert_resource(NetworkChannels {
            gameplay_signer: Default::default(),
            outgoing,
            incoming,
            signals,
        });
        incoming_tx
            .send(shared::wire::ServerPacket::Career {
                server_epoch: 7,
                sequence: 1,
                career: shared::career::CareerView {
                    storage_enabled: true,
                    ..default()
                },
            })
            .unwrap();
        app.update();
        let career = app.world().resource::<CareerClient>();
        assert!(
            career.view.storage_enabled,
            "the new view survives: the reset was consumed in its own frame"
        );
        assert!(career.public_profile_id.is_none());
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
    fn offline_leave_restores_saved_endpoint_and_next_join_uses_real_udp_transport() {
        use crate::world::{AvatarAssetCache, PlayerModelCatalog};
        use bevy::{asset::AssetApp, ecs::system::RunSystemOnce};

        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let saved_address = server.local_addr().unwrap().to_string();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Scene>()
            .init_asset::<bevy::gltf::Gltf>()
            .insert_resource(ClientSession {
                server_addr_display: saved_address.clone(),
                ..default()
            })
            .insert_resource(ResolvedServerAddressForPrefs(saved_address.clone()))
            .insert_resource(ClientSessionId("offline-online-lifecycle-test".into()))
            .insert_resource(PlayerVisualMode::Models3d)
            .insert_resource(PlayerAssets {
                scene: None,
                gltf: None,
                mesh: default(),
                material: default(),
            })
            .init_resource::<PlayerModelCatalog>()
            .init_resource::<AvatarAssetCache>()
            .init_resource::<NetIncomingDisconnected>()
            .init_resource::<PendingServerSnapshotFrame>()
            .init_resource::<StagedSnapshot>()
            .init_resource::<NetworkState>()
            .init_resource::<GameStateSnapshot>()
            .init_resource::<TeamSelection>()
            .init_resource::<CameraState>()
            .init_resource::<crate::maps::MapLayout>()
            .init_resource::<crate::frontend::PendingScreen>()
            .init_resource::<crate::debug::DebugToggles>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_message::<SessionUiCommand>()
            .add_message::<crate::game_vfx::UtilityVfx>()
            .add_message::<SessionEvent>()
            .add_message::<SnapshotApplied>()
            .add_systems(
                Update,
                (
                    update_session_lifecycle,
                    offline::step,
                    ingest_server_snapshot_packets,
                    snapshot_apply_systems(),
                    // The front end's reaction to `Left` (in `SessionReactions`
                    // in the app), after the flush that writes it.
                    crate::frontend::return_home_on_leave,
                )
                    .chain(),
            );

        app.world_mut()
            .write_message(SessionUiCommand::StartOffline);
        app.update();
        assert!(app.world().resource::<ClientSession>().is_offline());
        assert!(app.world().contains_resource::<offline::LocalPractice>());
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            saved_address
        );

        fn send_test_join(app: &mut App, hero_class: HeroClass) {
            app.world_mut()
                .run_system_once(
                    move |channels: Res<NetworkChannels>,
                          mut session: ResMut<ClientSession>,
                          identity: Res<ClientSessionId>,
                          mut selection: ResMut<TeamSelection>| {
                        selection.team = Some(Team::Green);
                        session.last_join = Some(CommittedJoin {
                            hero_class,
                            ..CommittedJoin::for_test()
                        });
                        send_join_attempt(&channels, &mut session, &identity);
                    },
                )
                .unwrap();
        }
        send_test_join(&mut app, HeroClass::Warrior);
        app.update();
        assert!(app.world().resource::<ClientSession>().join_confirmed());
        assert_eq!(
            app.world().resource::<GameStateSnapshot>().match_mode,
            "offline_practice"
        );
        let local = app
            .world_mut()
            .query_filtered::<Entity, With<Player>>()
            .single(app.world())
            .unwrap();
        let targets: Vec<_> = app
            .world()
            .resource::<NetworkState>()
            .remote_players
            .values()
            .copied()
            .collect();
        assert!(
            !targets.is_empty(),
            "the real local snapshot must spawn practice targets"
        );
        let visual = app
            .world_mut()
            .spawn(Name::new("practice target child"))
            .id();
        app.world_mut().entity_mut(targets[0]).add_child(visual);

        app.world_mut().write_message(SessionUiCommand::LeaveMatch);
        app.update();
        assert!(!app.world().contains_resource::<offline::LocalPractice>());
        assert!(app.world().get_entity(local).is_err());
        assert!(app.world().get_entity(visual).is_err());
        assert!(
            targets
                .iter()
                .all(|entity| app.world().get_entity(*entity).is_err())
        );
        assert!(
            app.world()
                .resource::<NetworkState>()
                .remote_players
                .is_empty()
        );
        assert!(!app.world().resource::<ClientSession>().is_offline());
        assert!(!app.world().resource::<ClientSession>().ephemeral_endpoint);
        assert!(!app.world().resource::<ClientSession>().has_committed_join());
        assert_eq!(
            app.world().resource::<ClientSession>().server_addr_display,
            saved_address
        );
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            saved_address
        );
        assert_eq!(
            app.world().resource::<crate::frontend::PendingScreen>().0,
            Some(crate::frontend::AppScreen::Home)
        );

        send_test_join(&mut app, HeroClass::Mage);
        let mut packet = [0_u8; 4096];
        let mut received_join = false;
        for _ in 0..4 {
            let (length, _) = server
                .recv_from(&mut packet)
                .expect("the restored production UDP transport must reach the saved endpoint");
            if matches!(
                serde_json::from_slice::<shared::public_transport::PublicClientDatagram>(
                    &packet[..length]
                ),
                Ok(shared::public_transport::PublicClientDatagram::TransportProbe { .. })
            ) {
                continue;
            }
            match serde_json::from_slice::<ClientPacket>(&packet[..length]).unwrap() {
                ClientPacket::Hello { .. } => {}
                ClientPacket::Join {
                    session_id,
                    team,
                    hero_class,
                    passport_ticket,
                    ..
                } => {
                    assert_eq!(session_id.as_deref(), Some("offline-online-lifecycle-test"));
                    assert_eq!(team, shared::map::Team::Green);
                    assert_eq!(
                        hero_class,
                        HeroClass::Mage,
                        "must be the new online join, not a queued offline join"
                    );
                    assert!(passport_ticket.is_none());
                    received_join = true;
                    break;
                }
                other => {
                    panic!("offline packets must not leak to the restored endpoint: {other:?}")
                }
            }
        }
        assert!(
            received_join,
            "online Join was not received by the UDP fixture"
        );
        assert!(!app.world().contains_resource::<offline::LocalPractice>());
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            saved_address
        );
    }

    #[test]
    fn allocated_handoff_retry_and_leave_preserve_saved_lobby_and_clear_old_scene() {
        let lobby = UdpSocket::bind("127.0.0.1:0").unwrap();
        let arena = UdpSocket::bind("127.0.0.1:0").unwrap();
        let lobby_address = lobby.local_addr().unwrap().to_string();
        let arena_address = arena.local_addr().unwrap().to_string();
        let mut flow = crate::match_service::MatchServiceClient::default();
        flow.active = true;
        flow.lobby_addr = Some(lobby_address.clone());
        flow.allocation = Some(shared::match_service::MatchAllocation {
            allocation_id: "handoff-test".into(),
            endpoint: arena_address.clone(),
            preference: shared::match_service::MatchPreference::Quick,
            team: shared::map::Team::Green,
            human_count: 1,
            bot_count: 9,
            rated: true,
            join_deadline_ms: 1,
        });
        let mut app = App::new();
        app.insert_resource(flow)
            .insert_resource(ResolvedServerAddressForPrefs(lobby_address.clone()))
            .insert_resource(ClientSession {
                state: ClientConnectionState::Connected,
                server_addr_display: lobby_address.clone(),
                ..default()
            })
            .init_resource::<NetIncomingDisconnected>()
            .init_resource::<PendingServerSnapshotFrame>()
            .init_resource::<NetworkState>()
            .init_resource::<GameStateSnapshot>()
            .init_resource::<TeamSelection>()
            .init_resource::<CameraState>()
            .add_message::<SessionUiCommand>()
            .add_systems(Update, update_session_lifecycle);
        let old_player = app.world_mut().spawn(Player).id();
        app.world_mut()
            .write_message(SessionUiCommand::ConnectAllocated(arena_address.clone()));
        app.update();
        assert!(app.world().get_entity(old_player).is_err());
        assert_eq!(
            app.world().resource::<ClientSession>().server_addr_display,
            arena_address
        );
        assert!(app.world().resource::<ClientSession>().ephemeral_endpoint);
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            lobby_address
        );
        app.world_mut().resource_mut::<ClientSession>().state = ClientConnectionState::Disconnected;
        app.world_mut().write_message(SessionUiCommand::Retry);
        app.update();
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            lobby_address
        );
        app.world_mut().write_message(SessionUiCommand::LeaveMatch);
        app.update();
        assert_eq!(
            app.world().resource::<ClientSession>().server_addr_display,
            lobby_address
        );
        assert!(!app.world().resource::<ClientSession>().ephemeral_endpoint);
        assert!(
            !app.world()
                .resource::<crate::match_service::MatchServiceClient>()
                .active
        );
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            lobby_address
        );
    }

    #[test]
    fn old_framed_server_reports_protocol_mismatch_and_stops_automatic_reconnect() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let server_addr = server.local_addr().unwrap().to_string();
        let (outgoing, outgoing_rx) = crossbeam_channel::unbounded();
        let (incoming_tx, incoming) = crossbeam_channel::unbounded();
        let (signals_tx, signals_rx) = crossbeam_channel::unbounded();
        let worker = std::thread::spawn(move || {
            run_udp_client(
                server_addr,
                outgoing_rx,
                incoming_tx,
                signals_tx,
                Default::default(),
            );
        });
        let mut packet = [0_u8; 1200];
        let (length, client_addr) = server.recv_from(&mut packet).unwrap();
        assert!(matches!(
            serde_json::from_slice::<ClientPacket>(&packet[..length]).unwrap(),
            ClientPacket::Hello {
                protocol_version: PROTOCOL_VERSION
            }
        ));
        let mut old_frame = shared::transport::encode_snapshot(
            br#"{"type":"snapshot","protocol_version":1,"players":[]}"#,
            7,
            1,
        )
        .unwrap()
        .remove(0);
        old_frame[4..6].copy_from_slice(&1_u16.to_le_bytes());
        server.send_to(&old_frame, client_addr).unwrap();
        let signal = match signals_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(signal) => signal,
            Err(error) => {
                drop(outgoing);
                worker.join().unwrap();
                panic!("old framed server did not report incompatibility: {error}");
            }
        };
        worker.join().unwrap();
        assert!(matches!(signal, NetThreadSignal::ProtocolMismatch));
        assert!(
            incoming.try_recv().is_err(),
            "incompatible JSON must never be applied"
        );

        // Feed the real thread's signal through the production lifecycle and UI.
        let (signals_tx, signals) = crossbeam_channel::unbounded();
        signals_tx.send(signal).unwrap();
        let mut app = App::new();
        app.insert_resource(NetworkChannels {
            gameplay_signer: Default::default(),
            outgoing,
            incoming,
            signals,
        })
        .insert_resource(ClientSession {
            state: ClientConnectionState::Connected,
            admitted: true,
            last_join: Some(CommittedJoin {
                prematch: false,
                team: Team::Green,
                character: CharacterChoice::Ipfs,
                hero_class: shared::HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
            }),
            ..default()
        })
        .init_resource::<NetIncomingDisconnected>()
        .init_resource::<PendingServerSnapshotFrame>()
        .init_resource::<NetworkState>()
        .init_resource::<GameStateSnapshot>()
        .init_resource::<TeamSelection>()
        .init_resource::<CameraState>()
        .init_resource::<PlayerVisualMode>()
        .init_resource::<SpriteVisualAssets>()
        .add_message::<SessionUiCommand>()
        .add_systems(Startup, setup_connection_status_ui)
        .add_systems(
            Update,
            (
                ingest_server_snapshot_packets,
                update_session_lifecycle,
                sync_connection_status_ui,
            )
                .chain(),
        );
        let actor = app.world_mut().spawn(Player).id();
        for _ in 0..2 {
            app.update();
            let session = app.world().resource::<ClientSession>();
            assert_eq!(session.state, ClientConnectionState::Disconnected);
            assert_eq!(session.join_error, Some(JoinRejection::ProtocolMismatch));
            assert!(!session.join_confirmed());
            assert!(!session.reconnect.active);
            let label = app
                .world_mut()
                .query_filtered::<&Text, With<ConnectionStatusLabel>>()
                .single(app.world())
                .unwrap();
            assert_eq!(label.0, JoinRejection::ProtocolMismatch.message());
            let retry = app
                .world_mut()
                .query_filtered::<&Node, With<ConnectionRetryButton>>()
                .single(app.world())
                .unwrap();
            assert_eq!(retry.display, Display::Flex);
        }
        assert!(app.world().get_entity(actor).is_err());
    }
}
