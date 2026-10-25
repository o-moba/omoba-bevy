//! LR-16: a public lobby restarts and 300 clients reconnect at once. Every one
//! passes the return-path check, signs in and gets its profile without an
//! "unavailable" (or any other) error, before and after the restart.
//!
//! The lobby needs PostgreSQL: set `OMOBA_HARNESS_DATABASE_URL` to a
//! disposable database (the test migrates it with the prebuilt
//! `migrate-career`), build the server first (`cargo build -p server`) and run
//! `cargo test -p harness --test admission -- --ignored`.
//!
//! Every client runs from 127.0.0.1, so the per-IP limits are raised for this
//! test; the per-source limits have their own unit tests in the server.
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant, SystemTime};

use ed25519_dalek::{Signer, SigningKey};
use harness::ServerProcess;
use shared::career::{AuthChallenge, CareerRequest, CareerView};
use shared::public_transport::{
    PROBE_PADDING_BYTES, PublicClientDatagram, PublicServerDatagram, hex,
};
use shared::transport::SnapshotAssembler;
use shared::wire::{ClientPacket, ServerPacket};

const CLIENTS: usize = 300;
/// Every client must be signed in within this time of the (re)start.
const ADMISSION_BUDGET: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Probe,
    Challenge,
    Authenticate,
    Done,
}

struct Client {
    socket: UdpSocket,
    key: SigningKey,
    session: String,
    nonce: String,
    assembler: SnapshotAssembler,
    step: Step,
    epoch: u64,
    path: String,
    challenge: Option<AuthChallenge>,
    next_send: Instant,
    progress: Instant,
    profile: Option<String>,
    errors: Vec<String>,
}

impl Client {
    fn new(server: SocketAddr, seed: [u8; 32], index: usize) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(server).unwrap();
        socket.set_nonblocking(true).unwrap();
        let now = Instant::now();
        Self {
            socket,
            key: SigningKey::from_bytes(&seed),
            session: format!("harness-admission-{index}"),
            nonce: format!("{index:032x}"),
            assembler: SnapshotAssembler::default(),
            step: Step::Probe,
            epoch: 0,
            path: String::new(),
            challenge: None,
            next_send: now,
            progress: now,
            profile: None,
            errors: Vec::new(),
        }
    }

    fn send(&self, datagram: &PublicClientDatagram) {
        let _ = self.socket.send(&serde_json::to_vec(datagram).unwrap());
    }

    fn bootstrap(&self, request: CareerRequest) {
        self.send(&PublicClientDatagram::TransportBootstrap {
            path_nonce: self.path.clone(),
            payload: serde_json::to_string(&ClientPacket::Career { request }).unwrap(),
        });
    }

    /// Sends (or re-sends, like the real client) what the current step needs.
    fn drive(&mut self, now: Instant) {
        if self.step == Step::Done || now < self.next_send {
            return;
        }
        // No answer for a while: start over from the probe, as a client whose
        // path expired does.
        if self.step != Step::Probe && now.duration_since(self.progress) > Duration::from_secs(8) {
            self.step = Step::Probe;
            self.challenge = None;
            self.progress = now;
        }
        match self.step {
            Step::Probe => {
                self.send(&PublicClientDatagram::TransportProbe {
                    protocol_version: shared::protocol::PROTOCOL_VERSION,
                    client_nonce: self.nonce.clone(),
                    padding: "0".repeat(PROBE_PADDING_BYTES),
                });
                self.next_send = now + Duration::from_millis(700);
            }
            Step::Challenge => {
                self.send(&PublicClientDatagram::TransportProof {
                    server_epoch: self.epoch,
                    path_nonce: self.path.clone(),
                });
                self.bootstrap(CareerRequest::Challenge {
                    public_key: hex(self.key.verifying_key().as_bytes()),
                    nickname: "Player".into(),
                    session_id: self.session.clone(),
                });
                self.next_send = now + Duration::from_secs(1);
            }
            Step::Authenticate => {
                let challenge = self.challenge.clone().expect("a signed challenge");
                let signature = hex(&self.key.sign(&challenge.signing_bytes()).to_bytes());
                self.bootstrap(CareerRequest::Authenticate {
                    challenge,
                    signature,
                });
                self.next_send = now + Duration::from_secs(2);
            }
            Step::Done => {}
        }
    }

    fn receive(&mut self, now: Instant) {
        let mut buf = [0_u8; 65_536];
        while let Ok(len) = self.socket.recv(&mut buf) {
            let bytes = &buf[..len];
            if bytes.starts_with(b"OMB1") {
                if let Ok(Some(payload)) = self.assembler.push(bytes, now)
                    && let Ok(ServerPacket::Career { career, .. }) =
                        serde_json::from_slice(&payload)
                {
                    self.career(career, now);
                }
            } else if let Ok(PublicServerDatagram::TransportChallenge {
                server_epoch,
                client_nonce,
                path_nonce,
                ..
            }) = serde_json::from_slice(bytes)
                && self.step == Step::Probe
                && client_nonce == self.nonce
            {
                self.epoch = server_epoch;
                self.path = path_nonce;
                self.step = Step::Challenge;
                self.next_send = now;
                self.progress = now;
            }
        }
    }

    fn career(&mut self, view: CareerView, now: Instant) {
        if let Some(error) = view.error {
            if self.errors.last() != Some(&error) {
                self.errors.push(error);
            }
        }
        let public_key = hex(self.key.verifying_key().as_bytes());
        if self.step == Step::Challenge
            && let Some(challenge) = view.challenge.filter(|c| {
                c.public_key == public_key
                    && c.session_id == self.session
                    && c.server_epoch == self.epoch
            })
        {
            self.challenge = Some(challenge);
            self.step = Step::Authenticate;
            self.next_send = now;
            self.progress = now;
        } else if self.step == Step::Authenticate
            && let (Some(profile), Some(nonce)) = (view.profile, view.auth_nonce)
            && self
                .challenge
                .as_ref()
                .is_some_and(|challenge| challenge.nonce == nonce)
        {
            self.profile = Some(profile.profile_id);
            self.step = Step::Done;
        }
    }
}

/// Connects every key at once and drives them until all are signed in.
/// Returns each key's profile id and every error any client saw.
fn admit_all(server: SocketAddr, seeds: &[[u8; 32]]) -> (Vec<String>, HashMap<usize, Vec<String>>) {
    let mut clients: Vec<Client> = seeds
        .iter()
        .enumerate()
        .map(|(index, seed)| Client::new(server, *seed, index))
        .collect();
    let deadline = Instant::now() + ADMISSION_BUDGET;
    while clients.iter().any(|c| c.step != Step::Done) {
        let now = Instant::now();
        assert!(
            now < deadline,
            "{} of {CLIENTS} clients not signed in after {ADMISSION_BUDGET:?}; steps: {:?}",
            clients.iter().filter(|c| c.step != Step::Done).count(),
            clients
                .iter()
                .filter(|c| c.step != Step::Done)
                .map(|c| c.step)
                .take(10)
                .collect::<Vec<_>>()
        );
        for client in &mut clients {
            client.drive(now);
            client.receive(now);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let errors = clients
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.errors.is_empty())
        .map(|(index, c)| (index, c.errors.clone()))
        .collect();
    (
        clients.into_iter().map(|c| c.profile.unwrap()).collect(),
        errors,
    )
}

#[test]
#[ignore = "requires OMOBA_HARNESS_DATABASE_URL (a disposable PostgreSQL database) and `cargo build -p server`"]
fn three_hundred_clients_reconnecting_after_a_lobby_restart_all_get_in() {
    let url = std::env::var("OMOBA_HARNESS_DATABASE_URL")
        .expect("set OMOBA_HARNESS_DATABASE_URL to a disposable PostgreSQL database");
    let migrate = harness::server::prebuilt("migrate-career")
        .expect("build the server first: cargo build -p server");
    let migrated = std::process::Command::new(migrate)
        .env("OMOBA_DATABASE_URL", &url)
        .output()
        .unwrap();
    assert!(
        migrated.status.success(),
        "migrate-career failed: {}",
        String::from_utf8_lossy(&migrated.stderr)
    );
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("omoba-harness-admission-{stamp}"));
    std::fs::create_dir_all(&root).unwrap();
    let matches = root.join("matches").display().to_string();
    let outbox = root.join("outbox").display().to_string();
    let mut lobby = ServerProcess::spawn_with_env(&[
        ("OMOBA_SERVER_ROLE", "lobby"),
        ("OMOBA_DATABASE_URL", &url),
        ("OMOBA_MATCH_MODE", "release"),
        ("OMOBA_TEAM_SIZE", "5"),
        ("OMOBA_MATCH_ROOT", &matches),
        ("OMOBA_CAREER_OUTBOX", &outbox),
        // One IP for all 300 clients: only the global budgets apply.
        ("OMOBA_LIMIT_ENDPOINTS_PER_IP", "512"),
        ("OMOBA_LIMIT_PROBES_PER_IP", "10000"),
        ("OMOBA_LIMIT_PROBES_PER_NETWORK", "10000"),
        ("OMOBA_LIMIT_NEW_PROFILES_PER_IP", "100000"),
        ("OMOBA_LIMIT_NEW_PROFILES_TOTAL", "100000"),
    ]);
    // Fresh keys for this run, so a reused database never mixes runs.
    let seeds: Vec<[u8; 32]> = (0..CLIENTS)
        .map(|index| {
            let mut seed = [0_u8; 32];
            seed[..16].copy_from_slice(&stamp.to_le_bytes());
            seed[16..24].copy_from_slice(&(index as u64).to_le_bytes());
            seed
        })
        .collect();
    let started = Instant::now();
    let (first, errors) = admit_all(lobby.addr(), &seeds);
    assert!(errors.is_empty(), "errors on first sign-in: {errors:?}");
    println!(
        "{CLIENTS} new profiles signed in after {:?}",
        started.elapsed()
    );
    lobby.restart();
    let restarted = Instant::now();
    let (second, errors) = admit_all(lobby.addr(), &seeds);
    assert!(
        errors.is_empty(),
        "errors while reconnecting after the restart: {errors:?}"
    );
    assert_eq!(second, first, "every key keeps its profile");
    println!(
        "{CLIENTS} clients reconnected after the restart in {:?}",
        restarted.elapsed()
    );
    let _ = std::fs::remove_dir_all(&root); // Only this test's own folder.
}
