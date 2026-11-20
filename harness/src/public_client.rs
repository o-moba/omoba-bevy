//! Ordinary signed public admission for disposable fill clients. Keys live only
//! in memory; this helper has no server privileges or gameplay shortcuts.
use std::{
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    time::{Duration, Instant},
};

use ed25519_dalek::{Signer, SigningKey};
use shared::{
    HeroClass,
    career::{
        AuthChallenge, CareerAction, CareerRequest, CareerView, MatchResult, MatchSummary,
        ProfileSummary, authorized_signing_bytes,
    },
    match_service::{MatchAllocation, MatchPreference, MatchServiceView},
    prematch::{DraftPlayer, PrematchAction, PrematchPhase, PrematchRequest},
    public_transport::{
        PROBE_PADDING_BYTES, PublicClientDatagram, PublicServerDatagram, SignedCommand, hex,
    },
    wire::{CharacterChoice, ClientPacket, GameState, ServerPacket},
};

use crate::SnapshotView;

const RETRY: Duration = Duration::from_secs(1);
const ACTION_RETRY: Duration = Duration::from_millis(300);
const SEARCH_ID: u64 = 1;

/// Safe diagnostics: never includes signing keys, challenges or session nonces.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PublicBotStatus {
    pub phase: &'static str,
    pub auth_age_ms: Option<u64>,
    pub profile_id: Option<String>,
    pub initial_profile: Option<ProfileSummary>,
    pub profile: Option<ProfileSummary>,
    pub result: Option<MatchResult>,
    pub history: Option<Vec<MatchSummary>>,
    pub allocation: Option<MatchAllocation>,
    pub error: Option<String>,
    pub ranked_denied_counts: Vec<u32>,
    pub ranked_accepted_count: Option<u32>,
    pub stale_career_packets: u64,
    pub authority_resets: u64,
    pub sent_gameplay_commands: u64,
    pub sent_transforms: u64,
    pub transform_gaps: crate::cadence::TimingHistogram,
    pub command_work: crate::cadence::CommandTiming,
}

pub(crate) struct PublicSession {
    key: SigningKey,
    public_key: String,
    session_id: String,
    nickname: String,
    client_nonce: String,
    epoch: u64,
    match_id: u64,
    path: Option<String>,
    auth: Option<String>,
    auth_confirmed_at: Option<Instant>,
    challenge: Option<AuthChallenge>,
    account_sequence: u64,
    sequence: u64,
    prematch_sequence: u64,
    career_packet_sequence: u64,
    lobby: bool,
    preference: MatchPreference,
    class: HeroClass,
    view: Option<MatchServiceView>,
    joined: bool,
    search_enabled: bool,
    next_probe: Instant,
    next_auth: Instant,
    next_action: Instant,
    next_ping: Instant,
    last_transform_at: Option<Instant>,
    next_join: Instant,
    next_result_query: Instant,
    result_query: u64,
    pub status: PublicBotStatus,
    eligibility_lobby: Option<SocketAddr>,
    match_vote: Option<shared::match_vote::View>,
}

fn random_hex() -> String {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).expect("OS entropy for disposable bot session");
    hex(&bytes)
}

/// Plan unlocked seats in stable identity order, after honoring real human locks.
/// Bots do not reserve classes; the server fills their remaining kits at freeze.
fn available_draft_class(players: &[DraftPlayer], id: u64) -> Option<HeroClass> {
    let local = players.iter().find(|p| p.player_id == id)?;
    let mut team: Vec<_> = players
        .iter()
        .filter(|p| p.team == local.team && !p.is_bot)
        .collect();
    team.sort_unstable_by_key(|p| (u8::from(!p.locked), p.player_id));
    let mut used = Vec::new();
    for p in team {
        let class = if !used.contains(&p.hero_class) {
            p.hero_class
        } else {
            HeroClass::ALL.into_iter().find(|c| !used.contains(c))?
        };
        used.push(class);
        if p.player_id == id {
            return Some(class);
        }
    }
    None
}

impl PublicSession {
    pub fn new(preference: MatchPreference, class: HeroClass, nickname: String) -> Self {
        let mut seed = [0; 32];
        getrandom::fill(&mut seed).expect("OS entropy for disposable bot key");
        let key = SigningKey::from_bytes(&seed);
        let now = Instant::now();
        Self {
            public_key: hex(key.verifying_key().as_bytes()),
            key,
            session_id: format!("fill-{}", random_hex()),
            nickname,
            client_nonce: random_hex(),
            epoch: 0,
            match_id: 0,
            path: None,
            auth: None,
            auth_confirmed_at: None,
            challenge: None,
            account_sequence: 0,
            sequence: 0,
            prematch_sequence: 0,
            career_packet_sequence: 0,
            lobby: true,
            preference,
            class,
            view: None,
            joined: false,
            search_enabled: true,
            next_probe: now,
            next_auth: now,
            next_action: now,
            next_ping: now,
            last_transform_at: None,
            next_join: now,
            next_result_query: now,
            result_query: 100,
            eligibility_lobby: None,
            match_vote: None,
            status: PublicBotStatus {
                phase: "probing",
                auth_age_ms: None,
                profile_id: None,
                initial_profile: None,
                profile: None,
                result: None,
                history: None,
                allocation: None,
                error: None,
                ranked_denied_counts: Vec::new(),
                ranked_accepted_count: None,
                stale_career_packets: 0,
                authority_resets: 0,
                sent_gameplay_commands: 0,
                sent_transforms: 0,
                transform_gaps: Default::default(),
                command_work: Default::default(),
            },
        }
    }

    pub fn enable_eligibility_probe(&mut self, lobby: SocketAddr) {
        assert!(
            lobby.ip().is_loopback(),
            "eligibility rehearsal is loopback only"
        );
        self.eligibility_lobby = Some(lobby);
    }

    pub fn set_search_enabled(&mut self, enabled: bool) {
        self.search_enabled = enabled;
        if self.lobby && self.auth.is_some() {
            self.status.phase = if enabled {
                "searching"
            } else {
                "authenticated"
            };
        }
        self.next_action = Instant::now();
    }

    pub fn status_view(&self) -> PublicBotStatus {
        let mut status = self.status.clone();
        status.auth_age_ms = self
            .auth_confirmed_at
            .filter(|_| self.auth.is_some())
            .map(|at| u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX));
        status
    }

    fn reset_transport(&mut self, lobby: bool) {
        self.status.phase = "probing";
        self.client_nonce = random_hex();
        self.epoch = 0;
        self.match_id = 0;
        self.path = None;
        self.auth = None;
        self.auth_confirmed_at = None;
        self.challenge = None;
        self.account_sequence = 0;
        self.sequence = 0;
        self.prematch_sequence = 0;
        self.career_packet_sequence = 0;
        self.view = None;
        self.match_vote = None;
        self.joined = false;
        self.lobby = lobby;
        self.next_probe = Instant::now();
        self.next_auth = Instant::now();
        self.next_join = Instant::now();
        self.next_action = Instant::now();
    }

    fn datagram(socket: &UdpSocket, value: &PublicClientDatagram) {
        let bytes = serde_json::to_vec(value).expect("public bot datagram serializes");
        assert_eq!(
            socket.send(&bytes).expect("send public bot datagram"),
            bytes.len(),
            "incomplete public bot datagram"
        );
    }

    fn bootstrap(&self, socket: &UdpSocket, request: CareerRequest) {
        let Some(path) = &self.path else { return };
        Self::datagram(
            socket,
            &PublicClientDatagram::TransportBootstrap {
                path_nonce: path.clone(),
                payload: serde_json::to_string(&ClientPacket::Career { request })
                    .expect("career request serializes"),
            },
        );
    }

    fn account(&mut self, socket: &UdpSocket, action: CareerAction) {
        let Some(nonce) = &self.auth else { return };
        self.account_sequence += 1;
        let signature = hex(&self
            .key
            .sign(&authorized_signing_bytes(
                self.epoch,
                nonce,
                self.account_sequence,
                &action,
            ))
            .to_bytes());
        self.bootstrap(
            socket,
            CareerRequest::Authorized {
                session_nonce: nonce.clone(),
                sequence: self.account_sequence,
                action,
                signature,
            },
        );
    }

    pub fn send(&mut self, socket: &UdpSocket, packet: &ClientPacket) -> bool {
        let (Some(auth), Some(path)) = (&self.auth, &self.path) else {
            return false;
        };
        let prepare_started = Instant::now();
        self.sequence += 1;
        let mut command = SignedCommand {
            server_epoch: self.epoch,
            match_id: self.match_id,
            session_id: self.session_id.clone(),
            session_nonce: auth.clone(),
            path_nonce: path.clone(),
            sequence: self.sequence,
            payload: serde_json::to_string(packet).expect("gameplay command serializes"),
            signature: String::new(),
        };
        self.status
            .command_work
            .prepare
            .record(prepare_started.elapsed());
        let sign_started = Instant::now();
        command.signature = hex(&self.key.sign(&command.signing_bytes()).to_bytes());
        self.status.command_work.sign.record(sign_started.elapsed());
        let encode_started = Instant::now();
        let bytes = serde_json::to_vec(&PublicClientDatagram::SignedCommand { command })
            .expect("public bot datagram serializes");
        self.status
            .command_work
            .encode
            .record(encode_started.elapsed());
        let send_started = Instant::now();
        assert_eq!(
            socket.send(&bytes).expect("send public bot datagram"),
            bytes.len(),
            "incomplete public bot datagram"
        );
        self.status.command_work.send.record(send_started.elapsed());
        self.status.sent_gameplay_commands = self.status.sent_gameplay_commands.saturating_add(1);
        if matches!(packet, ClientPacket::Transform { .. }) {
            self.status.sent_transforms = self.status.sent_transforms.saturating_add(1);
            let now = Instant::now();
            if let Some(previous) = self.last_transform_at.replace(now) {
                self.status
                    .transform_gaps
                    .record(now.duration_since(previous));
            }
        }
        true
    }

    pub fn transport(&mut self, socket: &UdpSocket, packet: PublicServerDatagram) {
        match packet {
            PublicServerDatagram::TransportChallenge {
                client_nonce,
                server_epoch,
                path_nonce,
                lobby,
            } if client_nonce == self.client_nonce => {
                // Admission retries may repeat a challenge. Keep an authenticated
                // session on repeats, but never carry it across a server restart.
                if self.epoch != server_epoch {
                    self.auth = None;
                    self.auth_confirmed_at = None;
                    self.challenge = None;
                    self.sequence = 0;
                    self.account_sequence = 0;
                    self.career_packet_sequence = 0;
                    self.joined = false;
                }
                self.epoch = server_epoch;
                self.path = Some(path_nonce.clone());
                self.lobby = lobby;
                Self::datagram(
                    socket,
                    &PublicClientDatagram::TransportProof {
                        server_epoch,
                        path_nonce,
                    },
                );
                self.status.phase = "authenticating";
                self.next_auth = Instant::now();
            }
            PublicServerDatagram::ServerFull { client_nonce }
                if client_nonce == self.client_nonce =>
            {
                self.status.phase = "server_full";
            }
            _ => {}
        }
    }

    fn valid_challenge(&self, challenge: &AuthChallenge) -> bool {
        challenge.server_epoch == self.epoch
            && challenge.public_key == self.public_key
            && challenge.session_id == self.session_id
            && challenge.nickname == self.nickname
    }

    pub fn career_packet(
        &mut self,
        epoch: u64,
        sequence: u64,
        view: CareerView,
    ) -> Option<SocketAddr> {
        // Mirror native net/ingest: an old auth error or assignment must never
        // roll a newer career view back, even within the same worker epoch.
        if epoch == 0 || epoch != self.epoch || self.path.is_none() {
            return None;
        }
        if sequence <= self.career_packet_sequence {
            self.status.stale_career_packets = self.status.stale_career_packets.saturating_add(1);
            return None;
        }
        self.career_packet_sequence = sequence;
        self.career(epoch, view)
    }

    /// Returns a worker endpoint only for this search's authenticated handoff.
    fn career(&mut self, epoch: u64, view: CareerView) -> Option<SocketAddr> {
        if self.path.is_none() || epoch != self.epoch {
            return None;
        }
        if view.storage_enabled && view.auth_nonce.is_none() {
            if self.auth.is_some()
                && matches!(
                    view.error.as_deref(),
                    Some(
                        "Sign in before using your profile."
                            | "Account authorization could not be refreshed. Reconnect when the account service is available."
                    )
                )
            {
                // Match the native client's explicit authority-loss recovery.
                // A new path also resets signed-command replay scope safely.
                self.status.authority_resets = self.status.authority_resets.saturating_add(1);
                self.reset_transport(self.lobby);
                self.status.error = view.error_code.or(view.error);
                return None;
            }
            if self.auth.is_none()
                && view.error.as_deref()
                    == Some("Authentication failed. Request a fresh challenge.")
            {
                // Do not sign the rejected nonce forever under a congested
                // first login. Ignore its retained view until Challenge replies.
                if self.challenge.take().is_some() {
                    self.next_auth = Instant::now();
                }
                self.status.error = view.error_code.or(view.error);
                return None;
            }
        }
        if self.auth.is_none()
            && let Some(challenge) = view.challenge
            && self.valid_challenge(&challenge)
            && self.challenge.as_ref() != Some(&challenge)
        {
            self.challenge = Some(challenge);
            self.next_auth = Instant::now();
        }
        if let (Some(nonce), Some(profile)) = (view.auth_nonce, view.profile)
            && (self.auth.as_ref() == Some(&nonce)
                || self.challenge.as_ref().is_some_and(|c| c.nonce == nonce))
        {
            self.auth = Some(nonce);
            self.auth_confirmed_at = Some(Instant::now());
            self.status.profile_id = Some(profile.profile_id.clone());
            if self.lobby && self.status.allocation.is_none() {
                self.status.initial_profile = Some(profile.clone());
            } else {
                self.status
                    .initial_profile
                    .get_or_insert_with(|| profile.clone());
            }
            self.status.profile = Some(profile);
            if matches!(self.status.phase, "authenticating" | "probing") {
                self.status.phase = if self.lobby {
                    if self.search_enabled {
                        "searching"
                    } else {
                        "authenticated"
                    }
                } else {
                    "joining"
                };
            }
        }
        self.match_vote = view.match_vote;
        if view.history_loaded {
            self.status.history = Some(view.history);
        }
        if let Some(result) = view.last_result.or(view.detail) {
            self.status.result = Some(result);
        }
        self.status.error = view.error_code.or(view.error);
        if self.eligibility_lobby.is_some() {
            let count = self
                .status
                .profile
                .as_ref()
                .map_or(0, |p| p.completed_public_matches);
            if self.lobby && view.match_service_request_id == Some(9000 + u64::from(count)) {
                if matches!(&view.match_service, Some(MatchServiceView::Failed { code }) if code == shared::career::error_code::RANKED_LOCKED)
                    && [0, 9].contains(&count)
                    && !self.status.ranked_denied_counts.contains(&count)
                {
                    self.status.ranked_denied_counts.push(count);
                }
                if count == 10
                    && matches!(
                        &view.match_service,
                        Some(MatchServiceView::Waiting {
                            preference: MatchPreference::Ranked,
                            ..
                        })
                    )
                {
                    self.status.ranked_accepted_count = Some(count);
                }
            }
            // A retained request1 can still describe the just-finished bot
            // allocation. The final probe asks Ranked explicitly; never follow
            // that old handoff while waiting for the lobby to retire it.
            if self.lobby && count == 10 && self.status.result.as_ref().is_some_and(|r| r.saved) {
                return None;
            }
            if !self.lobby
                && count == 10
                && self.status.result.as_ref().is_some_and(|r| {
                    r.saved
                        && r.outcome == shared::career::MatchOutcome::Completed
                        && self
                            .status
                            .history
                            .as_ref()
                            .is_some_and(|h| h.iter().any(|row| row.result_id == r.result_id))
                })
            {
                let lobby = self.eligibility_lobby.unwrap();
                self.reset_transport(true);
                return Some(lobby);
            }
        }
        if self.auth.is_none() || !self.lobby || view.match_service_request_id != Some(SEARCH_ID) {
            return None;
        }
        self.view = view.match_service;
        if let Some(MatchServiceView::Assigned { allocation, .. }) = &self.view {
            let endpoint = allocation.endpoint.to_socket_addrs().ok()?.find(|addr| {
                addr.port() != 0 && !addr.ip().is_unspecified() && !addr.ip().is_multicast()
            })?;
            self.status.allocation = Some(allocation.clone());
            self.reset_transport(false);
            return Some(endpoint);
        }
        self.status.phase = match self.view {
            Some(MatchServiceView::ReadyCheck { .. }) => "ready_check",
            Some(MatchServiceView::Allocating) => "allocating",
            Some(MatchServiceView::Failed { ref code }) => {
                self.status.error = Some(code.clone());
                "failed"
            }
            _ => "searching",
        };
        None
    }

    pub fn accepts(&self, epoch: u64) -> bool {
        self.path.is_some() && epoch == self.epoch
    }

    pub fn snapshot(&mut self, socket: &UdpSocket, packet: &ServerPacket) {
        self.match_id = packet.meta().match_id;
        self.joined = packet.player(packet.your_id()).is_some();
        if let Some(draft) = packet.prematch() {
            self.status.phase = "draft";
            if let Some(error) = &draft.error {
                self.status.error = Some(error.clone());
            }
            let me = draft
                .players
                .iter()
                .find(|p| p.player_id == packet.your_id());
            let action = match (draft.phase, me) {
                (PrematchPhase::Draft, Some(me)) if !me.locked => {
                    available_draft_class(&draft.players, me.player_id).map(|class| {
                        if class == me.hero_class {
                            PrematchAction::Lock { locked: true }
                        } else {
                            PrematchAction::Select {
                                handheld: Default::default(),
                                character: CharacterChoice::Ipfs,
                                hero_class: class,
                                avatar: None,
                                sprite_character: None,
                                role: class.primary_role(),
                                passport_ticket: None,
                            }
                        }
                    })
                }
                (PrematchPhase::Loading, Some(me)) if !me.loaded => Some(PrematchAction::Loaded),
                _ => None,
            };
            if let Some(action) = action
                && Instant::now() >= self.next_action
            {
                self.next_action = Instant::now() + ACTION_RETRY;
                self.prematch_sequence += 1;
                self.send(
                    socket,
                    &ClientPacket::Prematch {
                        request: PrematchRequest {
                            server_epoch: self.epoch,
                            match_id: self.match_id,
                            generation: draft.generation,
                            request_id: self.prematch_sequence,
                            action,
                        },
                    },
                );
            }
        } else if self.joined {
            self.status.phase = match packet.game_state() {
                GameState::Running => "running",
                GameState::Victory { .. } => "victory",
                _ => "joining",
            };
        }
        if let Some(error) = packet.join_error() {
            self.status.error = Some(format!("{error:?}"));
        }
    }

    pub fn tick(&mut self, socket: &UdpSocket) {
        let now = Instant::now();
        // The Home warmup has no game snapshots to drive network recovery.
        // A silent dead path must not keep its cached login forever. Re-probe
        // with the same account key; never treat stale status as fresh authority.
        if self.lobby
            && !self.search_enabled
            && self.auth.is_some()
            && self
                .auth_confirmed_at
                .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(25))
        {
            self.reset_transport(true);
        }
        if self.auth.is_none() && now >= self.next_probe {
            self.next_probe = now + RETRY;
            Self::datagram(
                socket,
                &PublicClientDatagram::TransportProbe {
                    protocol_version: shared::protocol::PROTOCOL_VERSION,
                    client_nonce: self.client_nonce.clone(),
                    padding: "0".repeat(PROBE_PADDING_BYTES),
                },
            );
        }
        if self.path.is_none() {
            return;
        }
        if self.auth.is_none() {
            if now >= self.next_auth {
                self.next_auth = now + RETRY * 2;
                let request = match &self.challenge {
                    Some(challenge) => CareerRequest::Authenticate {
                        challenge: challenge.clone(),
                        signature: hex(&self.key.sign(&challenge.signing_bytes()).to_bytes()),
                    },
                    None => CareerRequest::Challenge {
                        public_key: self.public_key.clone(),
                        nickname: self.nickname.clone(),
                        session_id: self.session_id.clone(),
                    },
                };
                self.bootstrap(socket, request);
            }
            return;
        }
        if now >= self.next_ping {
            self.next_ping = now + ACTION_RETRY;
            self.send(socket, &ClientPacket::Ping);
        }
        if self.lobby && !self.search_enabled {
            self.status.phase = "authenticated";
            return;
        }
        // The eligibility proof needs a legitimate completed public game, not
        // a second base-destruction benchmark. If it is still running after the
        // ordinary ten-minute window, exercise the server-offered surrender.
        // The independent Ranked victory stage still requires base destruction.
        if self.eligibility_lobby.is_some()
            && !self.lobby
            && self.status.phase == "running"
            && now >= self.next_action
            && let Some(vote) = self.match_vote.as_ref().filter(|v| {
                v.server_epoch == self.epoch
                    && v.match_id == self.match_id
                    && v.surrender.available
                    && v.cooldown_secs == 0
                    && v.active.is_none()
            })
            && let Some(generation) = vote.next_generation
        {
            self.next_action = now + RETRY * 2;
            self.send(
                socket,
                &ClientPacket::MatchVote {
                    server_epoch: vote.server_epoch,
                    match_id: vote.match_id,
                    generation,
                    kind: shared::match_vote::Kind::Surrender,
                    action: shared::match_vote::Action::Start,
                },
            );
        }
        if !self.lobby && self.status.phase == "victory" && now >= self.next_result_query {
            self.next_result_query = now + ACTION_RETRY;
            self.result_query += 1;
            if self.result_query.is_multiple_of(2) {
                if let Some(profile_id) = &self.status.profile_id {
                    self.account(
                        socket,
                        CareerAction::Profile {
                            request_id: self.result_query,
                            profile_id: profile_id.clone(),
                        },
                    );
                }
            } else {
                self.account(
                    socket,
                    CareerAction::History {
                        request_id: self.result_query,
                        before: None,
                    },
                );
            }
        }
        if self.lobby && now >= self.next_action {
            self.next_action = now + ACTION_RETRY;
            if self.eligibility_lobby.is_some() {
                let count = self
                    .status
                    .profile
                    .as_ref()
                    .map_or(0, |p| p.completed_public_matches);
                if ([0, 9].contains(&count) && !self.status.ranked_denied_counts.contains(&count))
                    || (count == 10 && self.status.ranked_accepted_count.is_none())
                {
                    self.next_action = now + RETRY * 2;
                    self.account(
                        socket,
                        CareerAction::FindMatch {
                            request_id: 9000 + u64::from(count),
                            preference: MatchPreference::Ranked,
                            also_casual: false,
                        },
                    );
                    return;
                }
                if count == 0 && self.status.ranked_denied_counts.contains(&0) {
                    self.next_action = now + RETRY * 2;
                    if let Some(id) = &self.status.profile_id {
                        self.account(
                            socket,
                            CareerAction::Profile {
                                request_id: 900_000_001,
                                profile_id: id.clone(),
                            },
                        );
                    }
                    return;
                }
                if self.status.ranked_accepted_count.is_some() {
                    self.account(socket, CareerAction::CancelQueue);
                    return;
                }
            }
            if self.preference == MatchPreference::Ranked
                && self
                    .status
                    .profile
                    .as_ref()
                    .is_none_or(|p| !p.ranked_unlocked())
            {
                // Ordinary signed reads only. A process-test coordinator may
                // prepare disclosed prior games separately in its disposable DB.
                self.next_action = now + RETRY * 2;
                if let Some(id) = &self.status.profile_id {
                    self.account(
                        socket,
                        CareerAction::Profile {
                            request_id: 900_000_000,
                            profile_id: id.clone(),
                        },
                    );
                }
                return;
            }
            match &self.view {
                Some(MatchServiceView::ReadyCheck {
                    check_id,
                    accepted: false,
                    ..
                }) => {
                    self.account(
                        socket,
                        CareerAction::ReadyMatch {
                            request_id: SEARCH_ID,
                            check_id: check_id.clone(),
                            accepted: true,
                        },
                    );
                }
                Some(MatchServiceView::ReadyCheck { .. } | MatchServiceView::Allocating) => {}
                _ => {
                    self.next_action = now + RETRY * 2;
                    self.account(
                        socket,
                        CareerAction::FindMatch {
                            request_id: SEARCH_ID,
                            preference: self.preference,
                            also_casual: false,
                        },
                    );
                }
            }
        } else if !self.lobby && !self.joined && now >= self.next_join {
            self.next_join = now + RETRY;
            self.send(
                socket,
                &ClientPacket::Join {
                    handheld: Default::default(),
                    prematch: true,
                    team: shared::map::Team::Green,
                    character: CharacterChoice::Ipfs,
                    hero_class: self.class,
                    avatar: None,
                    sprite_character: None,
                    session_id: Some(self.session_id.clone()),
                    passport_ticket: None,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};
    use shared::public_transport::decode_hex;

    #[test]
    fn transform_counts_follow_actual_signed_sends_and_idle_input_stays_idle() {
        let (socket, peer) = sockets();
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Warden, "Fixture".into());
        let pose = ClientPacket::Transform {
            input_kind: Default::default(),
            input_active: false,
            dash_sequence: 0,
            x: 1.0,
            y: 0.5,
            z: 2.0,
            yaw: 0.0,
        };
        assert!(!session.send(&socket, &pose));
        assert_eq!(session.status.sent_transforms, 0);
        session.epoch = 19;
        session.match_id = 7;
        session.auth = Some("auth".into());
        session.path = Some("path".into());
        assert!(session.send(&socket, &pose));
        let PublicClientDatagram::SignedCommand { command } = receive(&peer) else {
            panic!("signed input required")
        };
        session
            .key
            .verifying_key()
            .verify(
                &command.signing_bytes(),
                &Signature::from_bytes(&decode_hex(&command.signature).unwrap()),
            )
            .unwrap();
        assert!(matches!(
            serde_json::from_str::<ClientPacket>(&command.payload).unwrap(),
            ClientPacket::Transform {
                input_active: false,
                ..
            }
        ));
        assert_eq!(session.status.sent_transforms, 1);
        assert!(session.send(&socket, &ClientPacket::Ping));
        assert!(matches!(
            receive(&peer),
            PublicClientDatagram::SignedCommand { .. }
        ));
        assert_eq!(session.status.sent_gameplay_commands, 2);
        assert_eq!(session.status.sent_transforms, 1);
        assert_eq!(session.status.transform_gaps.count, 0);
        assert!(session.send(&socket, &pose));
        assert!(matches!(
            receive(&peer),
            PublicClientDatagram::SignedCommand { .. }
        ));
        assert_eq!(session.status.sent_gameplay_commands, 3);
        assert_eq!(session.status.sent_transforms, 2);
        assert_eq!(session.status.transform_gaps.count, 1);
        assert_eq!(session.status.command_work.prepare.count, 3);
        assert_eq!(session.status.command_work.sign.count, 3);
        assert_eq!(session.status.command_work.encode.count, 3);
        assert_eq!(session.status.command_work.send.count, 3);
        session.reset_transport(false);
        assert!(!session.send(&socket, &pose));
        assert_eq!(session.status.sent_gameplay_commands, 3);
        assert_eq!(session.status.sent_transforms, 2);
        assert_eq!(session.status.transform_gaps.count, 1);
    }

    #[test]
    fn late_career_authority_error_cannot_revoke_newer_authenticated_state() {
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Warden, "Fixture".into());
        session.epoch = 19;
        session.path = Some("path".into());
        session.auth = Some("auth".into());
        session.status.phase = "running";
        let lost = CareerView {
            storage_enabled: true,
            error: Some("Sign in before using your profile.".into()),
            ..Default::default()
        };
        session.career_packet(19, 10, CareerView::default());
        session.career_packet(18, 1000, lost.clone());
        session.career_packet(19, 9, lost.clone());
        session.career_packet(19, 10, lost.clone());
        assert_eq!(session.auth.as_deref(), Some("auth"));
        assert_eq!(session.path.as_deref(), Some("path"));
        assert_eq!(session.status.phase, "running");
        assert_eq!(session.status.stale_career_packets, 2);
        assert_eq!(session.status.authority_resets, 0);
        // A genuinely newer loss of authority must still force reauthentication.
        session.career_packet(19, 11, lost);
        assert!(session.auth.is_none() && session.path.is_none());
        assert_eq!(session.status.authority_resets, 1);
    }

    #[test]
    fn career_sequence_is_scoped_to_transport_and_foreign_epochs_cannot_poison_it() {
        let (socket, peer) = sockets();
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Warden, "Fixture".into());
        session.epoch = 19;
        session.path = Some("path".into());
        session.career_packet(19, 0, CareerView::default());
        session.career_packet(20, 1000, CareerView::default());
        assert_eq!(session.career_packet_sequence, 0);
        session.career_packet(19, 10, CareerView::default());
        assert_eq!(session.career_packet_sequence, 10);
        session.reset_transport(false);
        assert_eq!(session.career_packet_sequence, 0);
        session.career_packet(19, 1000, CareerView::default());
        assert_eq!(session.career_packet_sequence, 0);
        session.transport(
            &socket,
            PublicServerDatagram::TransportChallenge {
                client_nonce: session.client_nonce.clone(),
                server_epoch: 20,
                path_nonce: "new-path".into(),
                lobby: false,
            },
        );
        assert!(matches!(
            receive(&peer),
            PublicClientDatagram::TransportProof {
                server_epoch: 20,
                ..
            }
        ));
        session.career_packet(20, 1, CareerView::default());
        session.career_packet(19, 2000, CareerView::default());
        assert_eq!(session.career_packet_sequence, 1);
        // Repeated challenges within one scope preserve the high-water mark.
        session.transport(
            &socket,
            PublicServerDatagram::TransportChallenge {
                client_nonce: session.client_nonce.clone(),
                server_epoch: 20,
                path_nonce: "new-path".into(),
                lobby: false,
            },
        );
        assert!(matches!(
            receive(&peer),
            PublicClientDatagram::TransportProof { .. }
        ));
        assert_eq!(session.career_packet_sequence, 1);
    }

    #[test]
    fn draft_planning_honors_human_locks_and_assigns_five_distinct_classes() {
        let row = |id, team, locked, is_bot| {
            serde_json::from_value::<DraftPlayer>(serde_json::json!({
                "player_id":id,"nickname":"Fixture","team":team,"character":"ipfs",
                "hero_class":"warrior","avatar":null,"sprite_character":null,"role":"mid",
                "is_bot":is_bot,"locked":locked,"loaded":false
            }))
            .unwrap()
        };
        let mut players: Vec<_> = (1..=5).map(|id| row(id, "green", false, false)).collect();
        players.push(row(10, "green", true, true));
        players.push(row(11, "blue", true, false));
        assert_eq!(available_draft_class(&players, 1), Some(HeroClass::Warrior));
        players[4].locked = true;
        let mut classes = Vec::new();
        for id in 1..=5 {
            let class = available_draft_class(&players, id).unwrap();
            assert!(!classes.contains(&class));
            classes.push(class);
        }
        assert_eq!(available_draft_class(&players, 5), Some(HeroClass::Warrior));
        assert_ne!(available_draft_class(&players, 1), Some(HeroClass::Warrior));
        players.reverse();
        assert_eq!(available_draft_class(&players, 5), Some(HeroClass::Warrior));
        assert_ne!(available_draft_class(&players, 1), Some(HeroClass::Warrior));
    }

    fn sockets() -> (UdpSocket, UdpSocket) {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        peer.set_read_timeout(Some(RETRY)).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(peer.local_addr().unwrap()).unwrap();
        (socket, peer)
    }
    fn receive(peer: &UdpSocket) -> PublicClientDatagram {
        let mut bytes = [0; 16_384];
        let len = peer.recv(&mut bytes).unwrap();
        serde_json::from_slice(&bytes[..len]).unwrap()
    }

    #[test]
    fn rejected_challenge_requests_a_fresh_nonce_without_weakening_authorization() {
        let (socket, peer) = sockets();
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Mage, "Fill".into());
        session.epoch = 19;
        session.path = Some("b".repeat(64));
        session.next_probe = Instant::now() + Duration::from_secs(60);
        let old = AuthChallenge {
            public_key: session.public_key.clone(),
            nonce: "a".repeat(64),
            server_epoch: 19,
            session_id: session.session_id.clone(),
            nickname: session.nickname.clone(),
        };
        session.challenge = Some(old.clone());
        let rejected = CareerView {
            storage_enabled: true,
            challenge: Some(old),
            error: Some("Authentication failed. Request a fresh challenge.".into()),
            ..Default::default()
        };
        session.career(18, rejected.clone());
        assert!(
            session.challenge.is_some(),
            "foreign epoch cannot reset authentication"
        );
        session.career(19, rejected.clone());
        assert!(session.challenge.is_none());
        session.tick(&socket);
        let PublicClientDatagram::TransportBootstrap { payload, .. } = receive(&peer) else {
            panic!("missing fresh challenge request")
        };
        assert!(matches!(
            serde_json::from_str::<ClientPacket>(&payload).unwrap(),
            ClientPacket::Career {
                request: CareerRequest::Challenge { .. }
            }
        ));
        let retry = session.next_auth;
        session.career(19, rejected);
        assert_eq!(
            session.next_auth, retry,
            "periodic stale views cannot flood retries"
        );
        assert!(session.auth.is_none());
    }

    #[test]
    fn explicit_authority_loss_reprobes_but_empty_views_do_not_restart() {
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Mage, "Fill".into());
        session.epoch = 19;
        session.path = Some("b".repeat(64));
        session.auth = Some("a".repeat(64));
        session.account_sequence = 7;
        session.sequence = 8;
        session.career(
            19,
            CareerView {
                storage_enabled: true,
                ..Default::default()
            },
        );
        assert!(session.auth.is_some(), "empty bootstrap is not revocation");
        let key = session.public_key.clone();
        let nonce = session.client_nonce.clone();
        session.career(
            19,
            CareerView {
                storage_enabled: true,
                error: Some("Sign in before using your profile.".into()),
                ..Default::default()
            },
        );
        assert_eq!(session.public_key, key);
        assert_ne!(session.client_nonce, nonce);
        assert!(session.auth.is_none() && session.path.is_none());
        assert_eq!((session.account_sequence, session.sequence), (0, 0));
        assert_eq!(session.status.phase, "probing");
    }

    #[test]
    fn authenticated_home_keeps_the_session_alive_without_searching_until_released() {
        let (socket, peer) = sockets();
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Mage, "Fill".into());
        session.epoch = 19;
        session.path = Some("b".repeat(64));
        session.auth = Some("a".repeat(64));
        session.next_ping = Instant::now() + Duration::from_secs(60);
        session.set_search_enabled(false);
        session.tick(&socket);
        assert_eq!(session.status.phase, "authenticated");
        peer.set_nonblocking(true).unwrap();
        assert_eq!(
            peer.recv(&mut [0; 8192]).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        peer.set_nonblocking(false).unwrap();
        session.set_search_enabled(true);
        session.tick(&socket);
        let PublicClientDatagram::TransportBootstrap { payload, .. } = receive(&peer) else {
            panic!("missing ordinary search request")
        };
        assert!(matches!(
            serde_json::from_str::<ClientPacket>(&payload).unwrap(),
            ClientPacket::Career {
                request: CareerRequest::Authorized {
                    action: CareerAction::FindMatch {
                        preference: MatchPreference::Casual,
                        ..
                    },
                    ..
                }
            }
        ));
        assert_eq!(session.status.phase, "searching");
    }

    #[test]
    fn silent_home_path_is_reprobed_without_changing_account_identity() {
        let (socket, peer) = sockets();
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Mage, "Fill".into());
        session.search_enabled = false;
        session.path = Some("b".repeat(64));
        session.auth = Some("a".repeat(64));
        session.auth_confirmed_at = Some(Instant::now() - Duration::from_secs(26));
        let public_key = session.public_key.clone();
        let old_client_nonce = session.client_nonce.clone();
        session.tick(&socket);
        assert_eq!(session.public_key, public_key);
        assert_ne!(session.client_nonce, old_client_nonce);
        assert!(session.path.is_none() && session.auth.is_none());
        assert!(session.status_view().auth_age_ms.is_none());
        session.tick(&socket); // next event-loop turn opens the fresh probe
        let mut bytes = [0u8; 4096];
        let size = peer.recv(&mut bytes).unwrap();
        assert!(matches!(
            serde_json::from_slice::<PublicClientDatagram>(&bytes[..size]).unwrap(),
            PublicClientDatagram::TransportProbe { .. }
        ));
    }

    #[test]
    fn auth_freshness_requires_the_current_nonce_and_is_cleared_on_reprobe() {
        let mut session =
            PublicSession::new(MatchPreference::Casual, HeroClass::Mage, "Fill".into());
        session.epoch = 19;
        session.path = Some("b".repeat(64));
        let challenge = AuthChallenge {
            public_key: session.public_key.clone(),
            nonce: "a".repeat(64),
            server_epoch: 19,
            session_id: session.session_id.clone(),
            nickname: session.nickname.clone(),
        };
        session.challenge = Some(challenge.clone());
        let view = |nonce| CareerView {
            storage_enabled: true,
            auth_nonce: Some(nonce),
            profile: Some(ProfileSummary::new("f".repeat(64), "Fill".into())),
            ..Default::default()
        };
        session.career(19, view("c".repeat(64)));
        assert!(session.auth.is_none() && session.status_view().auth_age_ms.is_none());
        session.career(18, view(challenge.nonce.clone()));
        assert!(session.auth.is_none());
        session.career(19, view(challenge.nonce));
        assert!(session.auth.is_some() && session.status_view().auth_age_ms.is_some());
        session.auth_confirmed_at = Some(Instant::now() - Duration::from_secs(10));
        assert!(session.status_view().auth_age_ms.unwrap() >= 10_000);
        session.reset_transport(true);
        assert!(session.status_view().auth_age_ms.is_none());
    }

    #[test]
    fn foreign_challenges_are_never_signed_and_commands_bind_worker_and_sequence() {
        let (socket, peer) = sockets();
        let mut session =
            PublicSession::new(MatchPreference::Ranked, HeroClass::Mage, "Fill".into());
        let challenge = |session: &PublicSession| AuthChallenge {
            public_key: session.public_key.clone(),
            nonce: "a".repeat(64),
            server_epoch: 19,
            session_id: session.session_id.clone(),
            nickname: session.nickname.clone(),
        };
        // Wrong probe nonce cannot initialize admission.
        session.transport(
            &socket,
            PublicServerDatagram::TransportChallenge {
                server_epoch: 19,
                client_nonce: "foreign".into(),
                path_nonce: "b".repeat(64),
                lobby: true,
            },
        );
        assert!(session.path.is_none());
        session.transport(
            &socket,
            PublicServerDatagram::TransportChallenge {
                server_epoch: 19,
                client_nonce: session.client_nonce.clone(),
                path_nonce: "b".repeat(64),
                lobby: true,
            },
        );
        assert!(matches!(
            receive(&peer),
            PublicClientDatagram::TransportProof {
                server_epoch: 19,
                ..
            }
        ));
        let good = challenge(&session);
        for field in 0..4 {
            let mut bad = good.clone();
            match field {
                0 => bad.server_epoch += 1,
                1 => bad.public_key.push('0'),
                2 => bad.session_id.push('x'),
                _ => bad.nickname.push('x'),
            }
            session.career(
                19,
                CareerView {
                    challenge: Some(bad),
                    ..Default::default()
                },
            );
            assert!(session.challenge.is_none());
        }
        session.career(
            19,
            CareerView {
                challenge: Some(good.clone()),
                ..Default::default()
            },
        );
        assert_eq!(session.challenge, Some(good));
        session.auth = Some("c".repeat(64));
        session.match_id = 7;
        for sequence in 1..=2 {
            session.send(&socket, &ClientPacket::Ping);
            let PublicClientDatagram::SignedCommand { command } = receive(&peer) else {
                panic!("unsigned command")
            };
            assert_eq!(
                (command.server_epoch, command.match_id, command.sequence),
                (19, 7, sequence)
            );
            session
                .key
                .verifying_key()
                .verify(
                    &command.signing_bytes(),
                    &Signature::from_bytes(&decode_hex(&command.signature).unwrap()),
                )
                .unwrap();
            let mut replay = command.clone();
            replay.match_id += 1;
            assert!(
                session
                    .key
                    .verifying_key()
                    .verify(
                        &replay.signing_bytes(),
                        &Signature::from_bytes(&decode_hex(&command.signature).unwrap())
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn eligibility_surrender_requires_the_current_authoritative_window() {
        let (socket, peer) = sockets();
        let now = Instant::now();
        let mut session = PublicSession::new(
            MatchPreference::BotPractice,
            HeroClass::Warrior,
            "Probe".into(),
        );
        session.enable_eligibility_probe(peer.local_addr().unwrap());
        session.auth = Some("c".repeat(64));
        session.path = Some("b".repeat(64));
        session.epoch = 19;
        session.match_id = 7;
        session.lobby = false;
        session.joined = true;
        session.status.phase = "running";
        session.next_ping = now + Duration::from_secs(60);
        session.next_action = now;
        let base = shared::match_vote::View {
            server_epoch: 19,
            match_id: 7,
            next_generation: Some(5),
            cooldown_secs: 0,
            remake: shared::match_vote::Eligibility {
                available: false,
                starts_in_secs: 0,
                code: "window_closed".into(),
            },
            surrender: shared::match_vote::Eligibility {
                available: true,
                starts_in_secs: 0,
                code: "available".into(),
            },
            active: None,
        };
        peer.set_nonblocking(true).unwrap();
        for invalid in 0..3 {
            let mut view = base.clone();
            match invalid {
                0 => view.surrender.available = false,
                1 => view.server_epoch += 1,
                _ => view.cooldown_secs = 1,
            }
            session.match_vote = Some(view);
            session.tick(&socket);
            assert_eq!(
                peer.recv(&mut [0; 16384]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        peer.set_nonblocking(false).unwrap();
        session.match_vote = Some(base);
        session.tick(&socket);
        let PublicClientDatagram::SignedCommand { command } = receive(&peer) else {
            panic!("expected signed vote")
        };
        assert_eq!((command.server_epoch, command.match_id), (19, 7));
        assert!(matches!(
            serde_json::from_str::<ClientPacket>(&command.payload).unwrap(),
            ClientPacket::MatchVote {
                server_epoch: 19,
                match_id: 7,
                generation: 5,
                kind: shared::match_vote::Kind::Surrender,
                action: shared::match_vote::Action::Start,
            }
        ));
    }
}
