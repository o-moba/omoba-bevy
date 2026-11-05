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
    prematch::{PrematchAction, PrematchPhase, PrematchRequest},
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
    pub profile_id: Option<String>,
    pub initial_profile: Option<ProfileSummary>,
    pub profile: Option<ProfileSummary>,
    pub result: Option<MatchResult>,
    pub history: Option<Vec<MatchSummary>>,
    pub allocation: Option<MatchAllocation>,
    pub error: Option<String>,
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
    challenge: Option<AuthChallenge>,
    account_sequence: u64,
    sequence: u64,
    prematch_sequence: u64,
    lobby: bool,
    preference: MatchPreference,
    class: HeroClass,
    view: Option<MatchServiceView>,
    joined: bool,
    next_probe: Instant,
    next_auth: Instant,
    next_action: Instant,
    next_ping: Instant,
    next_join: Instant,
    next_result_query: Instant,
    result_query: u64,
    pub status: PublicBotStatus,
}

fn random_hex() -> String {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).expect("OS entropy for disposable bot session");
    hex(&bytes)
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
            challenge: None,
            account_sequence: 0,
            sequence: 0,
            prematch_sequence: 0,
            lobby: true,
            preference,
            class,
            view: None,
            joined: false,
            next_probe: now,
            next_auth: now,
            next_action: now,
            next_ping: now,
            next_join: now,
            next_result_query: now,
            result_query: 100,
            status: PublicBotStatus {
                phase: "probing",
                profile_id: None,
                initial_profile: None,
                profile: None,
                result: None,
                history: None,
                allocation: None,
                error: None,
            },
        }
    }

    fn datagram(socket: &UdpSocket, value: &PublicClientDatagram) {
        let bytes = serde_json::to_vec(value).expect("public bot datagram serializes");
        socket.send(&bytes).expect("send public bot datagram");
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

    pub fn send(&mut self, socket: &UdpSocket, packet: &ClientPacket) {
        let (Some(auth), Some(path)) = (&self.auth, &self.path) else {
            return;
        };
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
        command.signature = hex(&self.key.sign(&command.signing_bytes()).to_bytes());
        Self::datagram(socket, &PublicClientDatagram::SignedCommand { command });
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
                    self.challenge = None;
                    self.sequence = 0;
                    self.account_sequence = 0;
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

    /// Returns a worker endpoint only for this search's authenticated handoff.
    pub fn career(&mut self, epoch: u64, view: CareerView) -> Option<SocketAddr> {
        if self.path.is_none() || epoch != self.epoch {
            return None;
        }
        if self.auth.is_none()
            && let Some(challenge) = view.challenge
            && self.valid_challenge(&challenge)
            && self.challenge.as_ref() != Some(&challenge)
        {
            self.challenge = Some(challenge);
            self.next_auth = Instant::now();
        }
        if let (Some(nonce), Some(profile)) = (view.auth_nonce, view.profile) {
            self.auth = Some(nonce);
            self.status.profile_id = Some(profile.profile_id.clone());
            self.status
                .initial_profile
                .get_or_insert_with(|| profile.clone());
            self.status.profile = Some(profile);
            if matches!(self.status.phase, "authenticating" | "probing") {
                self.status.phase = if self.lobby { "searching" } else { "joining" };
            }
        }
        if view.history_loaded {
            self.status.history = Some(view.history);
        }
        if let Some(result) = view.last_result.or(view.detail) {
            self.status.result = Some(result);
        }
        self.status.error = view.error_code.or(view.error);
        if self.auth.is_none() || !self.lobby || view.match_service_request_id != Some(SEARCH_ID) {
            return None;
        }
        self.view = view.match_service;
        if let Some(MatchServiceView::Assigned { allocation, .. }) = &self.view {
            let endpoint = allocation.endpoint.to_socket_addrs().ok()?.find(|addr| {
                addr.port() != 0 && !addr.ip().is_unspecified() && !addr.ip().is_multicast()
            })?;
            self.status.allocation = Some(allocation.clone());
            self.status.phase = "probing";
            self.client_nonce = random_hex();
            self.epoch = 0;
            self.match_id = 0;
            self.path = None;
            self.auth = None;
            self.challenge = None;
            self.account_sequence = 0;
            self.sequence = 0;
            self.prematch_sequence = 0;
            self.view = None;
            self.joined = false;
            self.lobby = false;
            self.next_probe = Instant::now();
            self.next_auth = Instant::now();
            self.next_join = Instant::now();
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
                    Some(PrematchAction::Lock { locked: true })
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
}
