//! Private immutable allocation manifest and atomic worker lifecycle receipts.
use std::collections::HashSet;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{fs, io};

use serde::{Deserialize, Serialize};
use shared::career::valid_profile_id;
use shared::map::Team;
use shared::match_service::MatchPreference;
use shared::wire::ClientPacket;

use crate::runtime::ServerRuntime;
use crate::session::normalize_session_id;

pub(crate) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AllocatedHuman {
    pub profile_id: String,
    pub session_id: String,
    pub team: shared::map::Team,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub version: u32,
    pub allocation_id: String,
    pub endpoint: String,
    pub bind: String,
    pub preference: MatchPreference,
    pub humans: Vec<AllocatedHuman>,
    pub join_deadline_ms: u64,
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        let ids: HashSet<_> = self.humans.iter().map(|h| &h.profile_id).collect();
        let sessions: HashSet<_> = self.humans.iter().map(|h| &h.session_id).collect();
        if self.version != 1
            || self.allocation_id.len() != 32
            || !self.allocation_id.bytes().all(|b| b.is_ascii_hexdigit())
            || self.humans.is_empty()
            || self.humans.len() > 10
            || ids.len() != self.humans.len()
            || sessions.len() != self.humans.len()
            || self.bind.parse::<SocketAddr>().is_err()
            || self.endpoint.len() > 253
            || self.endpoint.contains(['/', '\n', '\r'])
            || self.humans.iter().any(|h| {
                !valid_profile_id(&h.profile_id)
                    || normalize_session_id(Some(h.session_id.clone())).as_ref()
                        != Some(&h.session_id)
            })
            || [shared::map::Team::Green, shared::map::Team::Blue]
                .iter()
                .any(|t| self.humans.iter().filter(|h| h.team == *t).count() > 5)
            || (self.preference == MatchPreference::HumansOnly && self.humans.len() != 10)
            || (self.preference == MatchPreference::BotPractice && self.humans.len() != 1)
        {
            return Err("Invalid allocated match manifest".into());
        }
        Ok(())
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|_| "Cannot read allocation manifest")?;
        if bytes.len() > 16384 {
            return Err("Allocation manifest too large".into());
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| "Invalid allocation JSON")?;
        value.validate()?;
        Ok(value)
    }
    pub fn team(&self, profile: &str, session: &str) -> Option<Team> {
        self.humans
            .iter()
            .find(|h| h.profile_id == profile && h.session_id == session)
            .map(|h| h.team)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Ready,
    Forming,
    Running,
    Settling,
    Finished,
    Failed,
    Recovering,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Status {
    pub allocation_id: String,
    pub server_epoch: u64,
    pub heartbeat_ms: u64,
    pub phase: Phase,
    pub result_id: Option<String>,
}
pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
pub(crate) struct Worker {
    pub manifest: Manifest,
    pub directory: PathBuf,
    pub recovery: bool,
    pub origin_epoch: u64,
    pub aborted: bool,
    pub booted_at: Instant,
    pub last_status: Option<Instant>,
    pub terminal_at: Option<Instant>,
}
impl Worker {
    pub fn cancelled(&self) -> bool {
        self.directory.join("cancel.json").exists()
    }
}
impl ServerRuntime {
    pub(crate) fn allocated_team(&self, addr: SocketAddr, packet: &ClientPacket) -> Option<Team> {
        let worker = self.match_service.worker()?;
        let ClientPacket::Join {
            session_id: Some(session),
            ..
        } = packet
        else {
            return None;
        };
        let profile = self.career.backend.profile(addr)?;
        if self.career.backend.authenticated_session(addr).as_deref() != Some(session) {
            return None;
        }
        worker.manifest.team(&profile.profile_id, session)
    }
    pub(crate) fn authorize_allocated_join(&self, addr: SocketAddr, packet: &ClientPacket) -> bool {
        if self.match_service.is_lobby() {
            return false;
        }
        let Some(worker) = self.match_service.worker() else {
            return true;
        };
        if worker.recovery || self.allocated_team(addr, packet).is_none() {
            return false;
        }
        let ClientPacket::Join { session_id, .. } = packet else {
            return false;
        };
        if self.match_started_at.is_none() {
            return unix_ms() <= worker.manifest.join_deadline_ms && !worker.cancelled();
        }
        // No new gameplay identity can enter a frozen roster, even with a valid manifest.
        session_id.as_ref().is_some_and(|s| {
            self.world
                .players
                .values()
                .any(|p| p.joined && p.session_id.as_ref() == Some(s))
                || self
                    .world
                    .disconnected_sessions
                    .get(s)
                    .is_some_and(|p| p.player.joined)
        })
    }
    pub(crate) fn allocated_humans_ready(&self) -> bool {
        self.match_service.worker().is_none_or(|worker| {
            worker.manifest.humans.iter().all(|h| {
                self.world.players.values().any(|p| {
                    p.joined
                        && !p.hero.identity.is_bot
                        && p.session_id.as_deref() == Some(h.session_id.as_str())
                        && p.career_profile
                            .as_ref()
                            .is_some_and(|p| p.profile_id == h.profile_id)
                })
            })
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use shared::map::Team;
    fn fixture() -> Manifest {
        Manifest {
            version: 1,
            allocation_id: "a".repeat(32),
            endpoint: "127.0.0.1:41000".into(),
            bind: "127.0.0.1:41000".into(),
            preference: MatchPreference::BotPractice,
            humans: vec![AllocatedHuman {
                profile_id: "b".repeat(64),
                session_id: "session-1".into(),
                team: shared::map::Team::Green,
            }],
            join_deadline_ms: 1,
        }
    }
    #[test]
    fn roster_rejects_duplicate_and_mismatched_identity() {
        let mut m = fixture();
        assert!(m.validate().is_ok());
        assert_eq!(m.team(&"b".repeat(64), "session-1"), Some(Team::Green));
        assert!(m.team(&"b".repeat(64), "other").is_none());
        m.humans.push(m.humans[0].clone());
        assert!(m.validate().is_err());
    }
    #[test]
    fn humans_only_requires_full_roster() {
        let mut m = fixture();
        m.preference = MatchPreference::HumansOnly;
        assert!(m.validate().is_err());
    }
}

#[cfg(test)]
mod runtime_tests {
    use shared::wire::GameState;
    use std::net::{SocketAddr, UdpSocket};
    use std::time::{Duration, Instant};

    use shared::HeroClass;
    use shared::map::Team;
    use shared::wire::{ClientPacket, ServerPacket, default_character_choice};

    use super::*;
    use crate::entities::DisconnectedSession;
    use crate::match_rules::{MatchConfig, MatchMode};
    use crate::runtime::{PLAYER_TIMEOUT, ServerRuntime};
    use crate::snapshot::{SNAPSHOT_INTERVAL, build_players_snapshot};
    fn fixture() -> (ServerRuntime, SocketAddr, ClientPacket) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(
            socket,
            MatchConfig {
                mode: MatchMode::Practice,
                team_size: 5,
            },
        );
        rt.career.backend = Box::new(crate::career_backend::MemoryCareer::test_backend(
            rt.server_epoch,
        ));
        let addr = "127.0.0.1:60200".parse().unwrap();
        let profile = shared::career::ProfileSummary::new("a".repeat(64), "Alice".into());
        rt.career
            .backend
            .test_authenticated(addr, profile.clone(), "seat-a");
        rt.match_service = crate::match_service::MatchService::Worker(Worker {
            manifest: Manifest {
                version: 1,
                allocation_id: "b".repeat(32),
                endpoint: "127.0.0.1:41000".into(),
                bind: "127.0.0.1:41000".into(),
                preference: MatchPreference::BotPractice,
                humans: vec![AllocatedHuman {
                    profile_id: profile.profile_id,
                    session_id: "seat-a".into(),
                    team: shared::map::Team::Blue,
                }],
                join_deadline_ms: unix_ms() + 180_000,
            },
            directory: std::env::temp_dir().join(format!(
                "omoba-allocation-test-{}-{}",
                std::process::id(),
                rt.server_epoch
            )),
            recovery: false,
            origin_epoch: 0,
            aborted: false,
            booted_at: Instant::now(),
            last_status: None,
            terminal_at: None,
        });
        let join = ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: default_character_choice(),
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            session_id: Some("seat-a".into()),
            passport_ticket: None,
        };
        (rt, addr, join)
    }
    #[test]
    fn only_allocated_profile_and_session_can_cancel_an_unstarted_worker() {
        use ed25519_dalek::{Signer, SigningKey};
        use shared::career::{CareerAction, CareerRequest, authorized_signing_bytes};
        let (mut rt, member, _) = fixture();
        let now = Instant::now();
        let directory = rt.match_service.worker().unwrap().directory.clone();
        fs::create_dir_all(&directory).unwrap();
        let cancellation = directory.join("cancel.json");
        assert!(!cancellation.exists());
        let outsider = "127.0.0.1:60301".parse().unwrap();
        rt.career.backend.test_authenticated(
            outsider,
            shared::career::ProfileSummary::new("c".repeat(64), "Other".into()),
            "other-seat",
        );
        let wrong_session = "127.0.0.1:60302".parse().unwrap();
        rt.career.backend.test_authenticated(
            wrong_session,
            shared::career::ProfileSummary::new("a".repeat(64), "Alice".into()),
            "wrong-seat",
        );
        let key = SigningKey::from_bytes(&[7; 32]);
        let request = || {
            let nonce = "b".repeat(64);
            let action = CareerAction::CancelQueue;
            let bytes = authorized_signing_bytes(rt.server_epoch, &nonce, 1, &action);
            let signature: String = key
                .sign(&bytes)
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            CareerRequest::Authorized {
                session_nonce: nonce,
                sequence: 1,
                action,
                signature,
            }
        };
        let outsider_request = request();
        let wrong_request = request();
        let member_request = request();
        rt.handle_career_request(outsider, outsider_request, now);
        assert!(
            !cancellation.exists(),
            "foreign authenticated profile cancelled the worker"
        );
        rt.handle_career_request(wrong_session, wrong_request, now);
        assert!(
            !cancellation.exists(),
            "different session cancelled the frozen allocation"
        );
        rt.handle_career_request(member, member_request, now);
        assert!(
            cancellation.exists(),
            "allocated participant could not cancel formation"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn allocated_practice_forces_draft_team_and_durable_start() {
        let (mut rt, addr, join) = fixture();
        let now = Instant::now();
        rt.handle_packet(addr, join, now);
        assert!(rt.world.players[&addr].joined);
        assert!(rt.world.players[&addr].draft.capable);
        assert_eq!(rt.world.players[&addr].hero.identity.team, Team::Blue);
        assert_eq!(
            rt.world
                .players
                .values()
                .filter(|p| p.hero.identity.is_bot)
                .count(),
            9
        );
        assert!(matches!(rt.world.game_state, GameState::Forming { .. }));
        assert!(!rt.begin_career_round(now));
        let allocation = rt.career_allocation_for_test().unwrap();
        assert_eq!(allocation.ruleset, "public-casual-v1");
        assert_eq!(allocation.unrated_reason.as_deref(), Some("allocated_bots"));
        assert!(!allocation.rated);
        assert_eq!(allocation.participants.len(), 10);
        rt.career.backend.test_ack_start(&allocation.result_id);
        assert!(rt.begin_career_round(now));
    }
    #[test]
    fn strangers_cannot_enter_and_original_identity_reclaims_its_frozen_seat() {
        let (mut rt, addr, join) = fixture();
        let now = Instant::now();
        rt.handle_packet(addr, join.clone(), now);
        let id = rt.world.players[&addr].hero.identity.id;
        rt.begin_career_round(now);
        let allocation = rt.career_allocation_for_test().unwrap();
        rt.career.backend.test_ack_start(&allocation.result_id);
        assert!(rt.begin_career_round(now));
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(now);
        let outsider: SocketAddr = "127.0.0.1:60201".parse().unwrap();
        rt.career.backend.test_authenticated(
            outsider,
            shared::career::ProfileSummary::new("c".repeat(64), "Other".into()),
            "other-seat",
        );
        let mut foreign = join.clone();
        if let ClientPacket::Join { session_id, .. } = &mut foreign {
            *session_id = Some("other-seat".into());
        }
        rt.handle_packet(outsider, foreign, now);
        assert!(!rt.world.players[&outsider].joined);
        let original = rt.world.players.remove(&addr).unwrap();
        rt.world.disconnected_sessions.insert(
            "seat-a".into(),
            DisconnectedSession {
                player: original,
                disconnected_at: now,
            },
        );
        rt.career.backend.forget(addr);
        let reconnect: SocketAddr = "127.0.0.1:60202".parse().unwrap();
        rt.career.backend.test_authenticated(
            reconnect,
            shared::career::ProfileSummary::new("a".repeat(64), "Alice".into()),
            "seat-a",
        );
        rt.handle_packet(reconnect, join, now);
        assert!(rt.world.players[&reconnect].joined);
        assert_eq!(rt.world.players[&reconnect].hero.identity.id, id);
        assert_eq!(rt.world.players[&reconnect].hero.identity.team, Team::Blue);
        assert_eq!(
            rt.world
                .players
                .values()
                .filter(|p| p.hero.identity.is_bot)
                .count(),
            9
        );
    }
    #[test]
    fn empty_allocated_worker_preserves_the_full_reconnect_window() {
        let (mut rt, addr, join) = fixture();
        let now = Instant::now();
        rt.handle_packet(addr, join, now);
        rt.begin_career_round(now);
        let result = rt.career_allocation_for_test().unwrap();
        rt.career.backend.test_ack_start(&result.result_id);
        assert!(rt.begin_career_round(now));
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(now);
        rt.maintain_roster(now + PLAYER_TIMEOUT + Duration::from_secs(1));
        rt.maintain_roster(now + PLAYER_TIMEOUT + Duration::from_secs(20));
        assert!(rt.world.disconnected_sessions.contains_key("seat-a"));
        assert!(!rt.match_service.worker().unwrap().aborted);
        assert_eq!(rt.world.game_state, GameState::Running);
    }
    #[test]
    fn saved_worker_repeats_victory_udp_snapshots_after_first_frame_is_lost() {
        use shared::public_transport::{PublicClientDatagram, PublicServerDatagram};
        let (mut rt, _, join) = fixture();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let addr = client.local_addr().unwrap();
        let now = Instant::now();
        // Establish real public return-path state so the production snapshot
        // admission filter is exercised, rather than bypassed by test flags.
        let probe = serde_json::to_vec(&PublicClientDatagram::TransportProbe {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
            client_nonce: "a".repeat(32),
            padding: "0".repeat(384),
        })
        .unwrap();
        let reply = rt.public_transport.receive(
            addr,
            &probe,
            rt.server_epoch,
            rt.match_id,
            false,
            None,
            now,
        );
        let crate::public_transport::Decision::Reply(bytes) = reply else {
            panic!("missing path challenge");
        };
        let PublicServerDatagram::TransportChallenge { path_nonce, .. } =
            serde_json::from_slice(&bytes).unwrap();
        let proof = serde_json::to_vec(&PublicClientDatagram::TransportProof {
            server_epoch: rt.server_epoch,
            path_nonce,
        })
        .unwrap();
        let reply = rt.public_transport.receive(
            addr,
            &proof,
            rt.server_epoch,
            rt.match_id,
            false,
            None,
            now,
        );
        let crate::public_transport::Decision::Dispatch(hello) = reply else {
            panic!("path proof rejected");
        };
        rt.handle_packet(addr, hello, now);
        rt.career.backend.test_authenticated(
            addr,
            shared::career::ProfileSummary::new("a".repeat(64), "Alice".into()),
            "seat-a",
        );
        rt.handle_packet(addr, join, now);
        assert!(!rt.begin_career_round(now));
        let allocation = rt.career_allocation_for_test().unwrap();
        rt.career.backend.test_ack_start(&allocation.result_id);
        assert!(rt.begin_career_round(now));
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(now);
        let epoch = rt.server_epoch;
        let match_id = rt.match_id;
        rt.world.game_state = GameState::Victory { winner: Team::Blue };
        rt.last_snapshot_at = now - SNAPSHOT_INTERVAL;
        rt.tick(now, 0.0);

        let mut assembler = shared::transport::SnapshotAssembler::default();
        let read_snapshot = |assembler: &mut shared::transport::SnapshotAssembler| {
            let mut buffer = vec![0; 65_536];
            loop {
                let (len, _) = client
                    .recv_from(&mut buffer)
                    .expect("worker stopped repeating terminal snapshots");
                if let Some(bytes) = assembler.push(&buffer[..len], Instant::now()).unwrap() {
                    let packet: ServerPacket = serde_json::from_slice(&bytes).unwrap();
                    if let ServerPacket::Snapshot { .. } = packet {
                        break packet;
                    }
                }
            }
        };
        // Deliberately discard the first Victory frame, as if UDP lost it.
        assert!(matches!(
            read_snapshot(&mut assembler),
            ServerPacket::Snapshot {
                game_state: GameState::Victory { winner: Team::Blue },
                ..
            }
        ));
        let mut saved = rt.career_allocation_for_test().unwrap();
        saved.saved = true;
        rt.career.backend.test_ack_settle(saved);
        rt.poll_career(now);
        rt.tick_match_service(now);
        assert!(rt.match_service.worker().unwrap().terminal_at.is_some());
        let frozen_players =
            serde_json::to_value(build_players_snapshot(&rt.world, None, now)).unwrap();
        let frozen_minions = rt.world.minions.len();
        let frozen_projectiles = rt.world.projectiles.len();
        for millis in [100, 500, 1000] {
            let later = now + Duration::from_millis(millis);
            rt.last_snapshot_at = later - SNAPSHOT_INTERVAL;
            rt.tick(later, 5.0);
            let packet = read_snapshot(&mut assembler);
            let ServerPacket::Snapshot {
                meta,
                game_state,
                players,
                scoreboard,
                ..
            } = packet
            else {
                unreachable!()
            };
            assert_eq!(game_state, GameState::Victory { winner: Team::Blue });
            assert_eq!(meta.server_epoch, epoch);
            assert_eq!(meta.match_id, match_id);
            assert_eq!(players.len(), 5, "victory retains team fog");
            assert!(players.iter().all(|p| p.team == Team::Blue));
            assert_eq!(scoreboard.unwrap().players.len(), 10);
            assert_eq!(
                serde_json::to_value(build_players_snapshot(&rt.world, None, later)).unwrap(),
                frozen_players
            );
            assert_eq!(rt.world.minions.len(), frozen_minions);
            assert_eq!(rt.world.projectiles.len(), frozen_projectiles);
            assert_eq!(
                rt.career_allocation_for_test().unwrap().outcome,
                shared::career::MatchOutcome::Completed
            );
            assert!(rt.career_allocation_for_test().unwrap().saved);
            assert!(!rt.match_service.worker().unwrap().aborted);
        }
    }

    #[test]
    fn allocated_worker_never_restarts_into_a_second_world() {
        let (mut rt, addr, join) = fixture();
        let now = Instant::now();
        rt.handle_packet(addr, join, now);
        let epoch = rt.server_epoch;
        let match_id = rt.match_id;
        rt.restart_round(now);
        assert_eq!(rt.server_epoch, epoch);
        assert_eq!(rt.match_id, match_id);
        assert!(rt.match_service.worker().unwrap().aborted);
    }
}
