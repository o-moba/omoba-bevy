//! Public UDP admission and signed client-command wire contract.
//! This authenticates client commands; it does not encrypt UDP or authenticate
//! the server. A deployment still needs a trusted server address.
use serde::{Deserialize, Serialize};

pub const MAX_PUBLIC_DATAGRAM_BYTES: usize = 12 * 1024;
pub const MAX_COMMAND_BYTES: usize = 8 * 1024;
pub const MAX_PACKETS_PER_TICK: usize = 128;
pub const RECEIVE_BUDGET_MILLIS: u64 = 2;
pub const PROBE_PADDING_BYTES: usize = 384;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameplayPrincipal {
    pub public_key: String,
    pub session_id: String,
    pub session_nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedCommand {
    pub server_epoch: u64,
    pub match_id: u64,
    pub session_id: String,
    pub session_nonce: String,
    pub path_nonce: String,
    pub sequence: u64,
    /// Sign the exact inner JSON, not client/server DTO reserialization.
    pub payload: String,
    pub signature: String,
}
impl SignedCommand {
    pub fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            "omoba.gameplay.command.v1",
            self.server_epoch,
            self.match_id,
            &self.session_id,
            &self.session_nonce,
            &self.path_nonce,
            self.sequence,
            &self.payload,
        ))
        .expect("string tuple serializes")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PublicClientDatagram {
    TransportProbe {
        protocol_version: u16,
        client_nonce: String,
        padding: String,
    },
    TransportProof {
        server_epoch: u64,
        path_nonce: String,
    },
    TransportBootstrap {
        path_nonce: String,
        payload: String,
    },
    SignedCommand {
        command: SignedCommand,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PublicServerDatagram {
    TransportChallenge {
        server_epoch: u64,
        client_nonce: String,
        path_nonce: String,
        lobby: bool,
    },
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}
pub fn decode_hex<const N: usize>(raw: &str) -> Option<[u8; N]> {
    if raw.len() != N * 2 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_domain_binds_every_namespace_and_exact_body() {
        let base = SignedCommand {
            server_epoch: 7,
            match_id: 2,
            session_id: "device".into(),
            session_nonce: "a".repeat(64),
            path_nonce: "b".repeat(64),
            sequence: 1,
            payload: "{\"type\":\"leave\"}".into(),
            signature: String::new(),
        };
        for index in 0..7 {
            let mut changed = base.clone();
            match index {
                0 => changed.server_epoch += 1,
                1 => changed.match_id += 1,
                2 => changed.session_id.push('x'),
                3 => changed.session_nonce.push('x'),
                4 => changed.path_nonce.push('x'),
                5 => changed.sequence += 1,
                _ => changed.payload.push(' '),
            }
            assert_ne!(base.signing_bytes(), changed.signing_bytes());
        }
    }
}
