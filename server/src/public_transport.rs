//! Bounded return-path admission and authenticated public command dispatch.
use super::*;
use ed25519_dalek::{Signature, VerifyingKey};
use shared::public_transport::{
    GameplayPrincipal, MAX_COMMAND_BYTES, MAX_PUBLIC_DATAGRAM_BYTES, PublicClientDatagram,
    PublicServerDatagram, decode_hex, hex,
};

const MAX_ENDPOINTS: usize = 512;
const CHALLENGE_TTL: Duration = Duration::from_secs(5);
const VALIDATED_TTL: Duration = Duration::from_secs(30);
const MAX_ENDPOINT_PACKETS_PER_SECOND: u32 = 120;
const MAX_PROBES_PER_SECOND: u32 = 64;

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

#[derive(Default)]
pub(crate) struct PublicTransport {
    endpoints: HashMap<SocketAddr, Endpoint>,
    probe_window: Option<Instant>,
    probes: u32,
}
impl PublicTransport {
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
            if self
                .probe_window
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
            {
                self.probe_window = Some(now);
                self.probes = 0;
            }
            if self.probes >= MAX_PROBES_PER_SECOND {
                return Decision::Drop;
            }
            self.probes += 1;
            if !self.endpoints.contains_key(&addr) {
                if self.endpoints.len() >= MAX_ENDPOINTS {
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
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use shared::public_transport::{PROBE_PADDING_BYTES, SignedCommand};
    fn address(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
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
            serde_json::from_slice(&reply).unwrap();
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
        assert_eq!(gate.endpoints.len(), MAX_PROBES_PER_SECOND as usize);
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
}
