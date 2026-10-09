//! Stable pre-game JSON exchange. Keep this envelope readable across gameplay
//! protocol changes. It diagnoses compatibility; it does NOT authenticate a server.
use serde::{Deserialize, Serialize};

pub const HANDSHAKE_VERSION: u16 = 1;
/// Bump when simulation semantics change incompatibly without a wire/map/catalog bump.
pub const GAMEPLAY_REVISION: &str = "combat-2026-10-08-mechanics";
pub const MAX_PROBE_BYTES: usize = 2048;
pub const PROBE_PADDING: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseContract {
    pub handshake: u16,
    pub release: String,
    pub protocol: u16,
    pub catalog: String,
    pub geometry: String,
    pub gameplay: String,
}
impl ReleaseContract {
    pub fn current() -> Self {
        Self {
            handshake: HANDSHAKE_VERSION,
            release: env!("CARGO_PKG_VERSION").into(),
            protocol: crate::protocol::PROTOCOL_VERSION,
            catalog: crate::loadout::CATALOG_REVISION.into(),
            geometry: crate::map::GEOMETRY_ID.into(),
            gameplay: GAMEPLAY_REVISION.into(),
        }
    }

    pub fn valid(&self) -> bool {
        [&self.release, &self.catalog, &self.geometry, &self.gameplay]
            .into_iter()
            .all(|v| {
                !v.is_empty()
                    && v.len() <= 64
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
            })
    }

    /// Release labels are deliberately excluded: compatible patches can differ.
    pub fn compare(&self, server: &Self) -> Result<(), CompatibilityIssue> {
        for (matches, reason) in [
            (
                self.handshake == HANDSHAKE_VERSION && server.handshake == HANDSHAKE_VERSION,
                CompatibilityIssue::Handshake,
            ),
            (
                self.protocol == server.protocol,
                CompatibilityIssue::Protocol,
            ),
            (self.catalog == server.catalog, CompatibilityIssue::Catalog),
            (
                self.geometry == server.geometry,
                CompatibilityIssue::Geometry,
            ),
            (
                self.gameplay == server.gameplay,
                CompatibilityIssue::Gameplay,
            ),
        ] {
            if !matches {
                return Err(reason);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityIssue {
    Handshake,
    Protocol,
    Catalog,
    Geometry,
    Gameplay,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompatibilityDatagram {
    CompatibilityProbe {
        nonce: String,
        client: ReleaseContract,
        padding: String,
    },
    CompatibilityReport {
        nonce: String,
        server: ReleaseContract,
    },
}

/// No socket, account, seat or match state is needed to answer this request.
pub fn response(bytes: &[u8], server: &ReleaseContract) -> Option<Vec<u8>> {
    if bytes.len() > MAX_PROBE_BYTES {
        return None;
    }
    let CompatibilityDatagram::CompatibilityProbe {
        nonce,
        client,
        padding,
    } = serde_json::from_slice(bytes).ok()?
    else {
        return None;
    };
    if crate::public_transport::decode_hex::<16>(&nonce).is_none()
        || !client.valid()
        || !(PROBE_PADDING..=1024).contains(&padding.len())
    {
        return None;
    }
    let reply = serde_json::to_vec(&CompatibilityDatagram::CompatibilityReport {
        nonce,
        server: server.clone(),
    })
    .ok()?;
    (reply.len() <= bytes.len()).then_some(reply)
}

pub struct CompatibilityProbe {
    pub client: ReleaseContract,
    nonce: String,
}
impl CompatibilityProbe {
    pub fn new(client: ReleaseContract, nonce: String) -> Self {
        Self { client, nonce }
    }
    pub fn request(&self) -> Vec<u8> {
        serde_json::to_vec(&CompatibilityDatagram::CompatibilityProbe {
            nonce: self.nonce.clone(),
            client: self.client.clone(),
            padding: "0".repeat(PROBE_PADDING),
        })
        .expect("compatibility request serializes")
    }
    /// A connected UDP socket must also filter the source endpoint. The nonce
    /// correlates this attempt; it is not an identity signature.
    pub fn accept(&self, bytes: &[u8]) -> Option<ReleaseContract> {
        if bytes.len() > MAX_PROBE_BYTES {
            return None;
        }
        let CompatibilityDatagram::CompatibilityReport { nonce, server } =
            serde_json::from_slice(bytes).ok()?
        else {
            return None;
        };
        (nonce == self.nonce && server.valid()).then_some(server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_matrix() {
        let client = ReleaseContract::current();
        let mut server = client.clone();
        server.release = "999.1.2".into();
        assert_eq!(client.compare(&server), Ok(()));
        let mut future = client.clone();
        future.handshake += 1;
        assert_eq!(future.compare(&future), Err(CompatibilityIssue::Handshake));
        for reason in [
            CompatibilityIssue::Handshake,
            CompatibilityIssue::Protocol,
            CompatibilityIssue::Catalog,
            CompatibilityIssue::Geometry,
            CompatibilityIssue::Gameplay,
        ] {
            let mut server = client.clone();
            match reason {
                CompatibilityIssue::Handshake => server.handshake += 1,
                CompatibilityIssue::Protocol => server.protocol += 1,
                CompatibilityIssue::Catalog => server.catalog.push('x'),
                CompatibilityIssue::Geometry => server.geometry.push('x'),
                CompatibilityIssue::Gameplay => server.gameplay.push('x'),
                _ => unreachable!(),
            }
            assert_eq!(client.compare(&server), Err(reason));
            assert_eq!(server.compare(&client), Err(reason));
        }
    }
    #[test]
    fn compatibility_envelope_bounds_and_nonce() {
        let current = ReleaseContract::current();
        let probe = CompatibilityProbe::new(current.clone(), "ab".repeat(16));
        let request = probe.request();
        let reply = response(&request, &current).unwrap();
        assert!(reply.len() <= request.len());
        assert_eq!(probe.accept(&reply), Some(current.clone()));
        let stale = CompatibilityProbe::new(current.clone(), "cd".repeat(16));
        assert!(stale.accept(&reply).is_none());
        assert!(probe.accept(&request).is_none());
        assert!(probe.accept(b"{}").is_none());
        assert!(response(&vec![0; MAX_PROBE_BYTES + 1], &current).is_none());
        let invalid = CompatibilityProbe::new(current.clone(), "invalid".into());
        assert!(response(&invalid.request(), &current).is_none());
        let mut invalid_contract = current.clone();
        invalid_contract.release = "x".repeat(65);
        let bad_reply = response(&request, &invalid_contract).unwrap();
        assert!(probe.accept(&bad_reply).is_none());
    }
}
