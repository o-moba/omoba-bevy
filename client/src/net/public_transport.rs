//! Public role detection, return-path proof and signing on the UDP I/O thread.
use ed25519_dalek::Signer;
use shared::protocol::PROTOCOL_VERSION;
use shared::wire::ClientPacket;
use std::{io, net::UdpSocket};

use super::transport::{SharedGameplaySigner, send_packet};
use shared::public_transport::{
    MAX_COMMAND_BYTES, MAX_PUBLIC_DATAGRAM_BYTES, PROBE_PADDING_BYTES, PublicClientDatagram,
    PublicServerDatagram, SignedCommand, decode_hex, hex,
};

pub(super) struct PublicClientTransport {
    client_nonce: String,
    challenge: Option<(u64, String)>,
    auth_nonce: Option<String>,
    sequence: u64,
    standalone_confirmed: bool,
}
impl PublicClientTransport {
    pub(super) fn new() -> Self {
        let mut nonce = [0; 16];
        let client_nonce = getrandom::fill(&mut nonce)
            .map(|_| hex(&nonce))
            .unwrap_or_default();
        Self {
            client_nonce,
            challenge: None,
            auth_nonce: None,
            sequence: 0,
            standalone_confirmed: false,
        }
    }

    pub(super) fn observed_server_packet(&mut self) {
        if self.challenge.is_none() {
            self.standalone_confirmed = true;
        }
    }

    pub(super) fn handle_challenge(&mut self, socket: &UdpSocket, bytes: &[u8]) -> bool {
        let Ok(PublicServerDatagram::TransportChallenge {
            server_epoch,
            client_nonce,
            path_nonce,
            ..
        }) = serde_json::from_slice(bytes)
        else {
            return false;
        };
        if self.client_nonce.is_empty()
            || client_nonce != self.client_nonce
            || server_epoch == 0
            || decode_hex::<32>(&path_nonce).is_none()
            || self
                .challenge
                .as_ref()
                .is_some_and(|(epoch, _)| server_epoch < *epoch)
        {
            return true;
        }
        self.challenge = Some((server_epoch, path_nonce.clone()));
        let _ = send_wire(
            socket,
            &PublicClientDatagram::TransportProof {
                server_epoch,
                path_nonce,
            },
        );
        true
    }

    /// A "server full" answer to a probe (LR-09): `None` for any other
    /// datagram; `Some(true)` when it answers this transport's own probe
    /// before any challenge, `Some(false)` for one to drop (another nonce,
    /// or the path is already proven).
    pub(super) fn server_full(&self, bytes: &[u8]) -> Option<bool> {
        let Ok(PublicServerDatagram::ServerFull { client_nonce }) = serde_json::from_slice(bytes)
        else {
            return None;
        };
        Some(
            !self.client_nonce.is_empty()
                && client_nonce == self.client_nonce
                && self.challenge.is_none(),
        )
    }

    pub(super) fn send(
        &mut self,
        socket: &UdpSocket,
        packet: &ClientPacket,
        signer: &SharedGameplaySigner,
    ) -> io::Result<()> {
        let standalone_hello =
            self.challenge.is_none() && matches!(packet, ClientPacket::Hello { .. });
        if standalone_hello {
            // Preserve the first datagram for existing standalone clients/tests.
            send_packet(socket, packet)?;
        }
        let signing_ready = self.challenge.as_ref().is_some_and(|(epoch, _)| {
            signer.lock().ok().is_some_and(|state| {
                state
                    .as_ref()
                    .is_some_and(|signer| signer.server_epoch == *epoch)
            })
        });
        if matches!(packet, ClientPacket::Hello { .. })
            && !self.client_nonce.is_empty()
            && !signing_ready
            && !self.standalone_confirmed
        {
            send_wire(
                socket,
                &PublicClientDatagram::TransportProbe {
                    protocol_version: PROTOCOL_VERSION,
                    client_nonce: self.client_nonce.clone(),
                    padding: "0".repeat(PROBE_PADDING_BYTES),
                },
            )?;
        }
        let Some((server_epoch, path_nonce)) = &self.challenge else {
            // Standalone hosts continue using the existing JSON packet format.
            return if standalone_hello {
                Ok(())
            } else {
                send_packet(socket, packet)
            };
        };
        let payload = serde_json::to_string(packet).map_err(io::Error::other)?;
        if payload.len() > MAX_COMMAND_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Command exceeds the public UDP budget",
            ));
        }
        if matches!(packet, ClientPacket::Career { .. }) {
            return send_wire(
                socket,
                &PublicClientDatagram::TransportBootstrap {
                    path_nonce: path_nonce.clone(),
                    payload,
                },
            );
        }
        let signer = signer
            .lock()
            .map_err(|_| io::Error::other("Gameplay signing state unavailable"))?;
        let Some(signer) = signer.as_ref().filter(|s| s.server_epoch == *server_epoch) else {
            // The UI retains/retries joins while the career challenge completes.
            // Movement and gameplay actions are not buffered across authentication.
            return Ok(());
        };
        if self.auth_nonce.as_deref() != Some(&signer.principal.session_nonce) {
            self.auth_nonce = Some(signer.principal.session_nonce.clone());
            self.sequence = 0;
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Gameplay sequence exhausted; reconnect"))?;
        let mut command = SignedCommand {
            server_epoch: *server_epoch,
            match_id: signer.match_id,
            session_id: signer.principal.session_id.clone(),
            session_nonce: signer.principal.session_nonce.clone(),
            path_nonce: path_nonce.clone(),
            sequence: self.sequence,
            payload,
            signature: String::new(),
        };
        command.signature = hex(&signer.key.sign(&command.signing_bytes()).to_bytes());
        send_wire(socket, &PublicClientDatagram::SignedCommand { command })
    }
}

fn send_wire(socket: &UdpSocket, packet: &PublicClientDatagram) -> io::Result<()> {
    let bytes = serde_json::to_vec(packet).map_err(io::Error::other)?;
    if bytes.len() > MAX_PUBLIC_DATAGRAM_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Public UDP envelope exceeds its budget",
        ));
    }
    let sent = socket.send(&bytes)?;
    if sent != bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "Incomplete public UDP datagram",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, SigningKey};
    use shared::public_transport::GameplayPrincipal;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    #[test]
    fn real_udp_challenge_echo_and_signed_command_use_saved_identity_namespace() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender.connect(receiver.local_addr().unwrap()).unwrap();
        let mut transport = PublicClientTransport::new();
        let mut buf = [0; 2048];
        transport
            .send(
                &sender,
                &ClientPacket::Hello {
                    protocol_version: PROTOCOL_VERSION,
                },
                &Arc::default(),
            )
            .unwrap();
        receiver.recv_from(&mut buf).unwrap(); // Standalone compatibility Hello.
        let (len, _) = receiver.recv_from(&mut buf).unwrap();
        assert!(matches!(
            serde_json::from_slice::<PublicClientDatagram>(&buf[..len]).unwrap(),
            PublicClientDatagram::TransportProbe { .. }
        ));
        let challenge = serde_json::to_vec(&PublicServerDatagram::TransportChallenge {
            server_epoch: 7,
            client_nonce: transport.client_nonce.clone(),
            path_nonce: "b".repeat(64),
            lobby: false,
        })
        .unwrap();
        // LR-09: a full server's answer to this probe; another nonce is dropped.
        let full = |nonce: &str| {
            serde_json::to_vec(&PublicServerDatagram::ServerFull {
                client_nonce: nonce.to_owned(),
            })
            .unwrap()
        };
        assert_eq!(
            transport.server_full(&full(&transport.client_nonce.clone())),
            Some(true)
        );
        assert_eq!(transport.server_full(&full(&"f".repeat(32))), Some(false));
        assert_eq!(transport.server_full(&challenge), None);
        assert!(transport.handle_challenge(&sender, &challenge));
        assert_eq!(
            transport.server_full(&full(&transport.client_nonce.clone())),
            Some(false),
            "a proven path is not full"
        );
        let (len, _) = receiver.recv_from(&mut buf).unwrap();
        assert!(matches!(
            serde_json::from_slice::<PublicClientDatagram>(&buf[..len]).unwrap(),
            PublicClientDatagram::TransportProof { .. }
        ));
        let key = SigningKey::from_bytes(&[4; 32]);
        let verifying = key.verifying_key();
        let signer = Arc::new(Mutex::new(Some(crate::career_identity::GameplaySigner {
            key,
            server_epoch: 7,
            match_id: 2,
            principal: GameplayPrincipal {
                public_key: hex(verifying.as_bytes()),
                session_id: "saved-device".into(),
                session_nonce: "c".repeat(64),
            },
        })));
        transport
            .send(&sender, &ClientPacket::Leave, &signer)
            .unwrap();
        let (len, _) = receiver.recv_from(&mut buf).unwrap();
        let PublicClientDatagram::SignedCommand { command } =
            serde_json::from_slice(&buf[..len]).unwrap()
        else {
            panic!("signed command required")
        };
        assert_eq!(command.sequence, 1);
        assert_eq!(command.match_id, 2);
        assert_eq!(command.session_id, "saved-device");
        verifying
            .verify_strict(
                &command.signing_bytes(),
                &Signature::from_bytes(&decode_hex::<64>(&command.signature).unwrap()),
            )
            .unwrap();
        for (sequence, action) in [
            (2, shared::utility::UtilityAction::Dash),
            (3, shared::utility::UtilityAction::Haste),
        ] {
            let packet = ClientPacket::Utility {
                action,
                direction: [1.0, 0.0],
                server_epoch: 7,
                match_id: 2,
                request_id: sequence,
            };
            transport.send(&sender, &packet, &signer).unwrap();
            let (len, _) = receiver.recv_from(&mut buf).unwrap();
            let PublicClientDatagram::SignedCommand { command } =
                serde_json::from_slice(&buf[..len]).unwrap()
            else {
                panic!("utility must use the public signed path");
            };
            assert_eq!(command.sequence, sequence);
            assert_eq!(command.server_epoch, 7);
            assert_eq!(command.match_id, 2);
            assert!(
                matches!(serde_json::from_str::<ClientPacket>(&command.payload).unwrap(), ClientPacket::Utility { action: sent, .. } if sent == action)
            );
            verifying
                .verify_strict(
                    &command.signing_bytes(),
                    &Signature::from_bytes(&decode_hex::<64>(&command.signature).unwrap()),
                )
                .unwrap();
        }
    }
}
