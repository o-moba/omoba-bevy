//! D1 directory contract. Parsing is not authentication: callers verify the
//! envelope with a pinned Ed25519 key, then validate scope/time/sequence before
//! selecting a lobby. A valid maintenance/update policy must not use fallback.
use crate::compatibility::{ReleaseContract, ReleaseVersion, valid_update_url};
use crate::region::RegionId;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
};

pub const MAX_WIRE_BYTES: usize = 65_536;
pub const MAX_PAYLOAD_BYTES: usize = 49_152;
pub const MAX_LOBBIES: usize = 32;
pub const MAX_TTL_MS: u64 = 7 * 86_400_000;
const CLOCK_SKEW_MS: u64 = 30_000;
const DOMAIN: &[u8] = b"omoba.game-directory.v1\0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedDirectory {
    pub key_id: String,
    /// Exact UTF-8 JSON bytes signed, without cross-language reserialization.
    pub payload: String,
    pub signature: String,
}
impl SignedDirectory {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err("directory envelope too large");
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| "invalid directory envelope")?;
        value.signing_bytes()?;
        if !crate::web_account::lowercase_hex(&value.signature, 64) {
            return Err("invalid directory signature encoding");
        }
        Ok(value)
    }
    /// Bind both key identity and exact payload to the directory-only domain.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if !label(&self.key_id, 32) || self.payload.len() > MAX_PAYLOAD_BYTES {
            return Err("invalid directory signing input");
        }
        let mut bytes = DOMAIN.to_vec();
        bytes.push(self.key_id.len() as u8);
        bytes.extend_from_slice(self.key_id.as_bytes());
        bytes.extend_from_slice(self.payload.as_bytes());
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Directory {
    pub version: u32,
    pub region: RegionId,
    /// `production` or the exact build contour, such as `dev`.
    pub contour: String,
    pub sequence: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub policy: Policy,
    pub lobbies: Vec<Lobby>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub min_release: String,
    pub maintenance: bool,
    pub update_url: Option<String>,
    pub message: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LobbyStatus {
    Open,
    Full,
    Maintenance,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lobby {
    pub id: String,
    /// Operator locality label, separate from account-region isolation.
    pub location: String,
    /// Operator label/contour checks; never resolve this name for UDP routing.
    pub host: String,
    /// Signed concrete destinations avoid a DNS change replacing the target.
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub contract: ReleaseContract,
    pub status: LobbyStatus,
    pub weight: u16,
}
impl Lobby {
    /// Callers use these exact authenticated IPs, not a fresh DNS lookup.
    pub fn network_endpoints(&self) -> Vec<SocketAddr> {
        self.addresses
            .iter()
            .map(|ip| SocketAddr::new(canonical_ip(*ip), self.port))
            .collect()
    }
    pub fn display_endpoint(&self) -> String {
        let host = match self.host.parse::<IpAddr>() {
            Ok(IpAddr::V6(ip)) => ip
                .to_ipv4_mapped()
                .map_or_else(|| ip.to_string(), |v4| v4.to_string()),
            Ok(ip) => ip.to_string(),
            Err(_) => self.host.clone(),
        };
        if host.contains(':') {
            format!("[{host}]:{}", self.port)
        } else {
            format!("{host}:{}", self.port)
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Selection<'a> {
    Lobby(&'a Lobby),
    UpdateRequired,
    Maintenance,
    Full,
    Incompatible,
}
impl Directory {
    /// Call only after authenticating the envelope's exact payload.
    pub fn parse(payload: &str) -> Result<Self, &'static str> {
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err("directory payload too large");
        }
        serde_json::from_str(payload).map_err(|_| "invalid directory payload")
    }
    pub fn validate_at(
        &self,
        region: RegionId,
        contour: &str,
        now_ms: u64,
        minimum_sequence: u64,
    ) -> Result<(), &'static str> {
        if self.version != 1
            || self.region != region
            || self.contour != contour
            || !label(contour, 32)
        {
            return Err("directory authority mismatch");
        }
        if self.sequence == 0 || self.sequence < minimum_sequence {
            return Err("directory sequence rollback");
        }
        if self.issued_at_ms > now_ms.saturating_add(CLOCK_SKEW_MS)
            || self.expires_at_ms <= now_ms
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms - self.issued_at_ms > MAX_TTL_MS
        {
            return Err("directory window invalid or expired");
        }
        if ReleaseVersion::parse(&self.policy.min_release).is_none()
            || self.policy.min_release.len() > 64
            || self
                .policy
                .update_url
                .as_deref()
                .is_some_and(|u| !valid_update_url(u))
            || self
                .policy
                .message
                .as_ref()
                .is_some_and(|m| m.len() > 1024 || m.chars().any(char::is_control))
        {
            return Err("invalid directory client policy");
        }
        if self.lobbies.len() > MAX_LOBBIES {
            return Err("too many directory lobbies");
        }
        let mut ids = HashSet::new();
        let mut endpoints = HashSet::new();
        let mut targets = HashSet::new();
        for lobby in &self.lobbies {
            if !label(&lobby.id, 64)
                || !label(&lobby.location, 32)
                || !valid_host(&lobby.host, contour != "production")
                || lobby.port == 0
                || !lobby.contract.valid()
                || lobby.contract.protocol == 0
                || lobby.contract.handshake == 0
                || ReleaseVersion::parse(&lobby.contract.release).is_none()
                || !(1..=10_000).contains(&lobby.weight)
                || !ids.insert(&lobby.id)
                || !endpoints.insert(lobby.display_endpoint())
                || lobby.addresses.is_empty()
                || lobby.addresses.len() > 8
                || lobby
                    .addresses
                    .iter()
                    .any(|ip| !valid_host(&ip.to_string(), contour != "production"))
                || lobby
                    .network_endpoints()
                    .iter()
                    .any(|address| !targets.insert(*address))
                || lobby.host.parse::<IpAddr>().is_ok_and(|host| {
                    !lobby
                        .addresses
                        .iter()
                        .any(|ip| canonical_ip(*ip) == canonical_ip(host))
                })
            {
                return Err("invalid or duplicate directory lobby");
            }
        }
        if self.lobbies.is_empty() && !self.policy.maintenance {
            return Err("empty open directory");
        }
        Ok(())
    }
    /// Selection is for new admission. Resume must retain the saved live worker
    /// and its owning lobby; client integration must not re-route that session.
    pub fn select(&self, client: &ReleaseContract, entropy: u64) -> Selection<'_> {
        let (Some(minimum), Some(current)) = (
            ReleaseVersion::parse(&self.policy.min_release),
            ReleaseVersion::parse(&client.release),
        ) else {
            return Selection::Incompatible;
        };
        if current < minimum {
            return Selection::UpdateRequired;
        }
        if self.policy.maintenance {
            return Selection::Maintenance;
        }
        let compatible: Vec<_> = self
            .lobbies
            .iter()
            .filter(|l| client.compare(&l.contract).is_ok())
            .collect();
        let total: u64 = compatible
            .iter()
            .filter(|l| l.status == LobbyStatus::Open)
            .map(|l| u64::from(l.weight))
            .sum();
        if total == 0 {
            return if compatible.iter().any(|l| l.status == LobbyStatus::Full) {
                Selection::Full
            } else if compatible
                .iter()
                .any(|l| l.status == LobbyStatus::Maintenance)
            {
                Selection::Maintenance
            } else {
                Selection::Incompatible
            };
        }
        let mut ticket = entropy % total;
        for lobby in compatible
            .into_iter()
            .filter(|l| l.status == LobbyStatus::Open)
        {
            if ticket < u64::from(lobby.weight) {
                return Selection::Lobby(lobby);
            }
            ticket -= u64::from(lobby.weight);
        }
        Selection::Incompatible
    }
}

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        ip => ip,
    }
}
fn label(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn valid_host(host: &str, local: bool) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ip) => {
                !ip.is_unspecified()
                    && !ip.is_multicast()
                    && !ip.is_broadcast()
                    && (local
                        || (!ip.is_private()
                            && !ip.is_loopback()
                            && !ip.is_link_local()
                            && !ip.is_documentation()
                            && ip.octets()[0] != 0
                            && ip.octets()[0] < 224
                            && !(ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                            && !(ip.octets()[0] == 198 && (18..=19).contains(&ip.octets()[1]))
                            && !(ip.octets()[0] == 192
                                && ip.octets()[1] == 0
                                && ip.octets()[2] == 0)
                            && !(ip.octets()[0] == 192
                                && ip.octets()[1] == 88
                                && ip.octets()[2] == 99)))
            }
            IpAddr::V6(ip) => {
                if let Some(ip) = ip.to_ipv4_mapped() {
                    valid_host(&ip.to_string(), local)
                } else {
                    !ip.is_unspecified()
                        && !ip.is_multicast()
                        && (local
                            || ((ip.segments()[0] & 0xe000) == 0x2000
                                && !ip.is_loopback()
                                && !ip.is_unique_local()
                                && !ip.is_unicast_link_local()
                                && ip.segments()[..2] != [0x2001, 0xdb8]))
                }
            }
        };
    }
    host.len() <= 253
        && host.is_ascii()
        && host == host.to_ascii_lowercase()
        && (local
            || (host.rsplit('.').next().is_some_and(|label| {
                label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphabetic)
            }) && host.contains('.')
                && !["localhost", "local", "internal", "invalid"]
                    .iter()
                    .any(|s| host.ends_with(&format!(".{s}")))))
        && host.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
#[path = "game_directory_tests.rs"]
mod tests;
