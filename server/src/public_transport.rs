//! Bounded return-path admission and authenticated public command dispatch.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, VerifyingKey};
use shared::public_transport::{
    GameplayPrincipal, MAX_COMMAND_BYTES, MAX_PUBLIC_DATAGRAM_BYTES, PublicClientDatagram,
    PublicServerDatagram, decode_hex, hex,
};
use shared::wire::ClientPacket;

use crate::admission::{Budget, Limits, SourceIp};
use crate::bots;

const MAX_ENDPOINTS: usize = 512;
const CHALLENGE_TTL: Duration = Duration::from_secs(5);
const VALIDATED_TTL: Duration = Duration::from_secs(30);
const MAX_ENDPOINT_PACKETS_PER_SECOND: u32 = 120;

pub(crate) enum Decision {
    Drop,
    Reply(Vec<u8>),
    Dispatch(ClientPacket),
}
struct Endpoint {
    client_nonce: String,
    path_nonce: String,
    created: Instant,
    touched: Instant,
    validated: bool,
    last_probe: Instant,
    rate_started: Instant,
    packets: u32,
    auth_nonce: Option<String>,
    sequence: u64,
}

/// Return-path admission. Probes and compatibility replies each have their
/// own per-second budget: per IP and per network first, then global with a
/// reserve for new sources (LR-16), so one source, even a spoofed one, cannot
/// lock everyone else out. One IP holds at most `endpoints_per_ip` endpoints.
#[derive(Default)]
pub(crate) struct PublicTransport {
    endpoints: HashMap<SocketAddr, Endpoint>,
    limits: Limits,
    probes: Budget,
    compatibility: Budget,
}
impl PublicTransport {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Independent of public admission: works for lobby, worker and standalone.
    /// No per-source allocation beyond the one-second budget; non-amplifying.
    pub(crate) fn compatibility_reply(
        &mut self,
        addr: SocketAddr,
        bytes: &[u8],
        now: Instant,
    ) -> Option<Vec<u8>> {
        if bots::is_bot_address(addr) || !self.compatibility.allows(addr, &self.limits.budget, now)
        {
            return None;
        }
        // LR-14: the report names the oldest admitted release and the update page.
        let reply = shared::compatibility::response_with_update(
            bytes,
            &shared::compatibility::ReleaseContract::current(),
            Some(&shared::compatibility::UpdateNotice::current()),
        )?;
        self.compatibility.admit(addr, &self.limits.budget, now);
        Some(reply)
    }

    pub(crate) fn validated(&self, addr: SocketAddr, now: Instant) -> bool {
        self.endpoints.get(&addr).is_some_and(|p| {
            p.validated && now.saturating_duration_since(p.touched) < VALIDATED_TTL
        })
    }

    pub(crate) fn receive(
        &mut self,
        addr: SocketAddr,
        bytes: &[u8],
        server_epoch: u64,
        match_id: u64,
        lobby: bool,
        principal: Option<GameplayPrincipal>,
        now: Instant,
    ) -> Decision {
        if bytes.len() > MAX_PUBLIC_DATAGRAM_BYTES || bots::is_bot_address(addr) {
            return Decision::Drop;
        }
        self.endpoints.retain(|_, endpoint| {
            now.saturating_duration_since(endpoint.touched)
                < if endpoint.validated {
                    VALIDATED_TTL
                } else {
                    CHALLENGE_TTL
                }
        });
        let Ok(packet) = serde_json::from_slice::<PublicClientDatagram>(bytes) else {
            return Decision::Drop;
        };
        if let PublicClientDatagram::TransportProbe {
            protocol_version,
            client_nonce,
            padding,
        } = packet
        {
            if protocol_version != shared::protocol::PROTOCOL_VERSION
                || decode_hex::<16>(&client_nonce).is_none()
                || !(256..=512).contains(&padding.len())
            {
                return Decision::Drop;
            }
            if !self.probes.admit(addr, &self.limits.budget, now) {
                return Decision::Drop;
            }
            if !self.endpoints.contains_key(&addr) {
                if self.endpoints.len() >= MAX_ENDPOINTS {
                    // Say so instead of leaving the player to time out
                    // (LR-09); like the challenge, never more bytes than the
                    // probe, and within the probe budget it already took.
                    let reply =
                        serde_json::to_vec(&PublicServerDatagram::ServerFull { client_nonce })
                            .expect("server full serializes");
                    return if reply.len() <= bytes.len() {
                        Decision::Reply(reply)
                    } else {
                        Decision::Drop
                    };
                }
                let ip = SourceIp::of(addr);
                if self
                    .endpoints
                    .keys()
                    .filter(|other| SourceIp::of(**other) == ip)
                    .count()
                    >= self.limits.endpoints_per_ip
                {
                    return Decision::Drop;
                }
                let mut nonce = [0; 32];
                if getrandom::fill(&mut nonce).is_err() {
                    return Decision::Drop;
                }
                self.endpoints.insert(
                    addr,
                    Endpoint {
                        client_nonce: client_nonce.clone(),
                        path_nonce: hex(&nonce),
                        created: now,
                        touched: now,
                        validated: false,
                        last_probe: now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
                        rate_started: now,
                        packets: 0,
                        auth_nonce: None,
                        sequence: 0,
                    },
                );
            }
            let endpoint = self.endpoints.get_mut(&addr).unwrap();
            if endpoint.client_nonce != client_nonce
                || now.saturating_duration_since(endpoint.last_probe) < Duration::from_millis(500)
            {
                return Decision::Drop;
            }
            endpoint.last_probe = now;
            let reply = serde_json::to_vec(&PublicServerDatagram::TransportChallenge {
                server_epoch,
                client_nonce,
                path_nonce: endpoint.path_nonce.clone(),
                lobby,
            })
            .expect("challenge serializes");
            // A spoofed source never receives more bytes than it supplied.
            return if reply.len() <= bytes.len() {
                Decision::Reply(reply)
            } else {
                Decision::Drop
            };
        }
        let Some(endpoint) = self.endpoints.get_mut(&addr) else {
            return Decision::Drop;
        };
        if now.saturating_duration_since(endpoint.rate_started) >= Duration::from_secs(1) {
            endpoint.rate_started = now;
            endpoint.packets = 0;
        }
        if endpoint.packets >= MAX_ENDPOINT_PACKETS_PER_SECOND {
            return Decision::Drop;
        }
        endpoint.packets += 1;
        match packet {
            PublicClientDatagram::TransportProof {
                server_epoch: epoch,
                path_nonce,
            } => {
                if epoch != server_epoch
                    || endpoint.path_nonce != path_nonce
                    || (!endpoint.validated
                        && now.saturating_duration_since(endpoint.created) >= CHALLENGE_TTL)
                {
                    return Decision::Drop;
                }
                if !endpoint.validated {
                    endpoint.validated = true;
                    endpoint.touched = now;
                }
                Decision::Dispatch(ClientPacket::Hello {
                    protocol_version: shared::protocol::PROTOCOL_VERSION,
                })
            }
            PublicClientDatagram::TransportBootstrap {
                path_nonce,
                payload,
            } => {
                if !endpoint.validated
                    || endpoint.path_nonce != path_nonce
                    || payload.len() > MAX_COMMAND_BYTES
                {
                    return Decision::Drop;
                }
                match serde_json::from_str::<ClientPacket>(&payload) {
                    Ok(packet @ ClientPacket::Career { .. }) => Decision::Dispatch(packet),
                    _ => Decision::Drop,
                }
            }
            PublicClientDatagram::SignedCommand { command } => {
                let Some(principal) = principal else {
                    return Decision::Drop;
                };
                if !endpoint.validated
                    || command.server_epoch != server_epoch
                    || command.match_id != match_id
                    || command.path_nonce != endpoint.path_nonce
                    || command.session_id != principal.session_id
                    || command.session_nonce != principal.session_nonce
                    || command.sequence == 0
                    || command.payload.len() > MAX_COMMAND_BYTES
                {
                    return Decision::Drop;
                }
                if endpoint.auth_nonce.as_deref() == Some(&principal.session_nonce)
                    && command.sequence <= endpoint.sequence
                {
                    return Decision::Drop;
                }
                let Some(key) = decode_hex::<32>(&principal.public_key)
                    .and_then(|b| VerifyingKey::from_bytes(&b).ok())
                else {
                    return Decision::Drop;
                };
                let Some(signature) =
                    decode_hex::<64>(&command.signature).map(|b| Signature::from_bytes(&b))
                else {
                    return Decision::Drop;
                };
                if key
                    .verify_strict(&command.signing_bytes(), &signature)
                    .is_err()
                {
                    return Decision::Drop;
                }
                let Ok(packet) = serde_json::from_str::<ClientPacket>(&command.payload) else {
                    return Decision::Drop;
                };
                // Account operations retain their own independent replay counter.
                if matches!(packet, ClientPacket::Career { .. }) {
                    return Decision::Drop;
                }
                if let ClientPacket::Join { session_id, .. } = &packet
                    && session_id.as_deref() != Some(principal.session_id.as_str())
                {
                    return Decision::Drop;
                }
                endpoint.auth_nonce = Some(principal.session_nonce);
                endpoint.sequence = command.sequence;
                endpoint.touched = now;
                Decision::Dispatch(packet)
            }
            PublicClientDatagram::TransportProbe { .. } => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::wire::ClientPacket;
    use std::net::SocketAddr;
    use std::time::Instant;

    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use shared::public_transport::{PROBE_PADDING_BYTES, SignedCommand};
    /// A distinct source per `port`: its own IP in its own /24, so the
    /// per-source limits never mix two test endpoints.
    fn address(port: u16) -> SocketAddr {
        SocketAddr::from(([10, (port >> 8) as u8, port as u8, 1], port))
    }
    fn budget() -> crate::admission::BudgetLimits {
        Limits::default().budget
    }
    fn encode(packet: PublicClientDatagram) -> Vec<u8> {
        serde_json::to_vec(&packet).unwrap()
    }
    fn probe() -> Vec<u8> {
        encode(PublicClientDatagram::TransportProbe {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
            client_nonce: "a".repeat(32),
            padding: "0".repeat(PROBE_PADDING_BYTES),
        })
    }
    fn validate(gate: &mut PublicTransport, addr: SocketAddr, now: Instant) -> String {
        let bytes = probe();
        let Decision::Reply(reply) = gate.receive(addr, &bytes, 7, 2, false, None, now) else {
            panic!("challenge required")
        };
        assert!(reply.len() <= bytes.len());
        let PublicServerDatagram::TransportChallenge { path_nonce, .. } =
            serde_json::from_slice(&reply).unwrap()
        else {
            panic!("a transport challenge is expected")
        };
        let proof = encode(PublicClientDatagram::TransportProof {
            server_epoch: 7,
            path_nonce: path_nonce.clone(),
        });
        assert!(matches!(
            gate.receive(addr, &proof, 7, 2, false, None, now),
            Decision::Dispatch(ClientPacket::Hello { .. })
        ));
        path_nonce
    }
    #[test]
    fn compatibility_probe_is_rate_limited_and_never_admits_players() {
        use shared::compatibility::{CompatibilityProbe, ReleaseContract};
        let client = ReleaseContract::current();
        let probe = CompatibilityProbe::new(client.clone(), "ab".repeat(16));
        let request = probe.request();
        let now = Instant::now();
        let mut gate = PublicTransport::default();
        for index in 1..=budget().global as u16 {
            let reply = gate
                .compatibility_reply(address(index), &request, now)
                .unwrap();
            assert!(reply.len() <= request.len());
            assert_eq!(probe.accept(&reply), Some(client.clone()));
            // LR-14: the report names the oldest admitted release (this server's).
            assert_eq!(
                probe.accept_report(&reply).unwrap().update,
                Some(shared::compatibility::UpdateNotice::current())
            );
            assert!(gate.endpoints.is_empty());
        }
        assert!(
            gate.compatibility_reply(address(1000), &request, now)
                .is_none()
        );
        assert!(
            gate.compatibility_reply(address(1000), &request, now + Duration::from_secs(1))
                .is_some()
        );
        assert!(!gate.validated(address(1), now));
        assert!(
            gate.compatibility_reply(address(1), b"{}", now + Duration::from_secs(2))
                .is_none()
        );
    }

    #[test]
    fn unknown_unsigned_sources_never_create_endpoints_and_challenge_is_nonamplifying() {
        let mut gate = PublicTransport::default();
        let now = Instant::now();
        for index in 1..1000 {
            assert!(matches!(
                gate.receive(
                    address(index),
                    br#"{"type":"hello","protocol_version":2}"#,
                    7,
                    2,
                    false,
                    None,
                    now
                ),
                Decision::Drop
            ));
        }
        assert!(gate.endpoints.is_empty());
        let nonce = validate(&mut gate, address(1), now);
        let spoof = encode(PublicClientDatagram::TransportProof {
            server_epoch: 7,
            path_nonce: nonce,
        });
        assert!(matches!(
            gate.receive(address(2), &spoof, 7, 2, false, None, now),
            Decision::Drop
        ));
        assert!(!gate.validated(address(2), now));
    }
    #[test]
    fn signatures_reject_mutation_replay_old_round_and_foreign_endpoint() {
        let mut gate = PublicTransport::default();
        let now = Instant::now();
        let addr = address(1);
        let nonce = validate(&mut gate, addr, now);
        let key = SigningKey::from_bytes(&[7; 32]);
        let principal = GameplayPrincipal {
            public_key: hex(key.verifying_key().as_bytes()),
            session_id: "player".into(),
            session_nonce: "b".repeat(64),
        };
        let mut command = SignedCommand {
            server_epoch: 7,
            match_id: 2,
            session_id: principal.session_id.clone(),
            session_nonce: principal.session_nonce.clone(),
            path_nonce: nonce,
            sequence: 1,
            payload: "{\"type\":\"leave\"}".into(),
            signature: String::new(),
        };
        command.signature = hex(&key.sign(&command.signing_bytes()).to_bytes());
        let bytes = encode(PublicClientDatagram::SignedCommand {
            command: command.clone(),
        });
        assert!(matches!(
            gate.receive(addr, &bytes, 7, 3, false, Some(principal.clone()), now),
            Decision::Drop
        ));
        assert!(matches!(
            gate.receive(
                address(2),
                &bytes,
                7,
                2,
                false,
                Some(principal.clone()),
                now
            ),
            Decision::Drop
        ));
        let mut forged = command;
        forged.payload = "{\"type\":\"request_rematch\"}".into();
        assert!(matches!(
            gate.receive(
                addr,
                &encode(PublicClientDatagram::SignedCommand { command: forged }),
                7,
                2,
                false,
                Some(principal.clone()),
                now
            ),
            Decision::Drop
        ));
        assert!(matches!(
            gate.receive(addr, &bytes, 7, 2, false, Some(principal.clone()), now),
            Decision::Dispatch(ClientPacket::Leave)
        ));
        assert!(matches!(
            gate.receive(addr, &bytes, 7, 2, false, Some(principal), now),
            Decision::Drop
        ));
    }
    #[test]
    fn challenge_table_and_per_endpoint_work_are_bounded_and_expire() {
        let mut gate = PublicTransport::default();
        let now = Instant::now();
        let bytes = probe();
        for index in 1..1000 {
            let _ = gate.receive(address(index), &bytes, 7, 2, false, None, now);
        }
        assert_eq!(gate.endpoints.len(), budget().global as usize);
        let _ = gate.receive(
            address(2000),
            &bytes,
            7,
            2,
            false,
            None,
            now + CHALLENGE_TTL,
        );
        assert_eq!(gate.endpoints.len(), 1);
        let nonce = validate(&mut gate, address(2001), now + CHALLENGE_TTL);
        let invalid = encode(PublicClientDatagram::TransportBootstrap {
            path_nonce: nonce,
            payload: "{\"type\":\"leave\"}".into(),
        });
        for _ in 0..1000 {
            assert!(matches!(
                gate.receive(
                    address(2001),
                    &invalid,
                    7,
                    2,
                    false,
                    None,
                    now + CHALLENGE_TTL
                ),
                Decision::Drop
            ));
        }
        assert!(!gate.validated(address(2001), now + CHALLENGE_TTL + VALIDATED_TTL));
    }
    /// LR-09: at the endpoint ceiling a new source's probe is answered
    /// "server full" (its own nonce, never more bytes than it sent) instead
    /// of silence; no endpoint is created, and a freed place admits it.
    #[test]
    fn a_probe_at_the_endpoint_ceiling_gets_server_full() {
        let mut gate = PublicTransport::default();
        let now = Instant::now();
        for index in 0..MAX_ENDPOINTS as u16 {
            gate.endpoints.insert(
                address(index),
                Endpoint {
                    client_nonce: "b".repeat(32),
                    path_nonce: "c".repeat(64),
                    created: now,
                    touched: now,
                    validated: true,
                    last_probe: now,
                    rate_started: now,
                    packets: 0,
                    auth_nonce: None,
                    sequence: 0,
                },
            );
        }
        let bytes = probe();
        let newcomer = address(4000);
        let Decision::Reply(reply) = gate.receive(newcomer, &bytes, 7, 2, true, None, now) else {
            panic!("a full server answers the probe")
        };
        assert!(reply.len() <= bytes.len());
        assert_eq!(
            serde_json::from_slice::<PublicServerDatagram>(&reply).unwrap(),
            PublicServerDatagram::ServerFull {
                client_nonce: "a".repeat(32)
            }
        );
        assert_eq!(gate.endpoints.len(), MAX_ENDPOINTS);
        assert!(!gate.endpoints.contains_key(&newcomer));
        // A known endpoint still gets its challenge.
        let known = encode(PublicClientDatagram::TransportProbe {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
            client_nonce: "b".repeat(32),
            padding: "0".repeat(PROBE_PADDING_BYTES),
        });
        let later = now + Duration::from_secs(1);
        let Decision::Reply(reply) = gate.receive(address(1), &known, 7, 2, true, None, later)
        else {
            panic!("a known endpoint is challenged")
        };
        assert!(matches!(
            serde_json::from_slice(&reply).unwrap(),
            PublicServerDatagram::TransportChallenge { .. }
        ));
        // A place frees: the newcomer is challenged.
        gate.endpoints.remove(&address(2));
        validate(&mut gate, newcomer, later + Duration::from_secs(1));
    }
    #[test]
    fn expired_mobile_path_requires_new_proof_and_rejects_old_signed_packets() {
        let mut gate = PublicTransport::default();
        let now = Instant::now();
        let addr = address(1);
        let old_path = validate(&mut gate, addr, now);
        let key = SigningKey::from_bytes(&[17; 32]);
        let principal = GameplayPrincipal {
            public_key: hex(key.verifying_key().as_bytes()),
            session_id: "mobile-player".into(),
            session_nonce: "d".repeat(64),
        };
        let signed = |path_nonce: String| {
            let mut command = SignedCommand {
                server_epoch: 7,
                match_id: 2,
                session_id: principal.session_id.clone(),
                session_nonce: principal.session_nonce.clone(),
                path_nonce,
                sequence: 1,
                payload: serde_json::to_string(&ClientPacket::Ping).unwrap(),
                signature: String::new(),
            };
            command.signature = hex(&key.sign(&command.signing_bytes()).to_bytes());
            encode(PublicClientDatagram::SignedCommand { command })
        };
        let old = signed(old_path.clone());
        let resume = now + Duration::from_secs(120);
        assert!(matches!(
            gate.receive(addr, &old, 7, 2, false, Some(principal.clone()), resume),
            Decision::Drop
        ));
        assert!(!gate.validated(addr, resume));
        let fresh_path = validate(&mut gate, addr, resume);
        assert_ne!(fresh_path, old_path);
        assert!(matches!(
            gate.receive(addr, &old, 7, 2, false, Some(principal.clone()), resume),
            Decision::Drop
        ));
        assert!(matches!(
            gate.receive(
                addr,
                &signed(fresh_path),
                7,
                2,
                false,
                Some(principal.clone()),
                resume
            ),
            Decision::Dispatch(ClientPacket::Ping)
        ));
    }

    /// LR-16: 1000 probes per second from one address, compatibility probes
    /// included, never stop a second address from being admitted and answered.
    #[test]
    fn a_flooding_source_cannot_lock_out_a_second_source() {
        use shared::compatibility::{CompatibilityProbe, ReleaseContract};
        let compatibility = CompatibilityProbe::new(ReleaseContract::current(), "cd".repeat(16));
        let request = compatibility.request();
        let mut gate = PublicTransport::default();
        let start = Instant::now();
        let flood = SocketAddr::from(([198, 51, 100, 7], 40000));
        let second = SocketAddr::from(([203, 0, 113, 9], 50000));
        let mut answered = 0;
        for index in 0..1000_u64 {
            let at = start + Duration::from_millis(index);
            // Every probe from its own port, as a spoofer would send them.
            let from = SocketAddr::new(flood.ip(), 1024 + index as u16);
            let _ = gate.receive(from, &probe(), 7, 2, false, None, at);
            answered += usize::from(gate.compatibility_reply(from, &request, at).is_some());
            if index == 500 {
                assert!(
                    gate.compatibility_reply(second, &request, at)
                        .is_some_and(|reply| compatibility.accept(&reply).is_some()),
                    "the second address gets a compatibility reply"
                );
                validate(&mut gate, second, at);
                assert!(gate.validated(second, at));
            }
        }
        assert!(answered <= budget().per_ip as usize, "{answered}");
        let flooding = gate
            .endpoints
            .keys()
            .filter(|addr| addr.ip() == flood.ip())
            .count();
        assert!(flooding <= Limits::default().endpoints_per_ip, "{flooding}");
    }

    /// LR-16: one IP holds at most `endpoints_per_ip` endpoints; another IP,
    /// even in the same network, is not affected.
    #[test]
    fn one_ip_holds_at_most_n_endpoints() {
        let limit = Limits::default().endpoints_per_ip;
        let mut gate = PublicTransport::default();
        let mut now = Instant::now();
        let mut validated = 0;
        for port in 0..20_u16 {
            // Spread over seconds so only the endpoint limit applies.
            now += Duration::from_millis(400);
            let addr = SocketAddr::from(([203, 0, 113, 9], 50000 + port));
            if let Decision::Reply(reply) = gate.receive(addr, &probe(), 7, 2, false, None, now) {
                let PublicServerDatagram::TransportChallenge { path_nonce, .. } =
                    serde_json::from_slice(&reply).unwrap()
                else {
                    panic!("a transport challenge is expected")
                };
                let proof = encode(PublicClientDatagram::TransportProof {
                    server_epoch: 7,
                    path_nonce,
                });
                let _ = gate.receive(addr, &proof, 7, 2, false, None, now);
                validated += usize::from(gate.validated(addr, now));
            }
        }
        assert_eq!(validated, limit);
        let neighbour = SocketAddr::from(([203, 0, 113, 10], 50000));
        validate(&mut gate, neighbour, now);
        // An expired endpoint frees its place.
        let later = now + VALIDATED_TTL;
        let returning = SocketAddr::from(([203, 0, 113, 9], 60000));
        validate(&mut gate, returning, later);
    }
}
