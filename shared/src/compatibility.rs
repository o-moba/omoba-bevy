//! Stable pre-game JSON exchange. Keep this envelope readable across gameplay
//! protocol changes. It diagnoses compatibility; it does NOT authenticate a server.
//!
//! Since LR-14 a report may also carry an [`UpdateNotice`]: the oldest release
//! the server admits and the market's update page. It is additive (older
//! clients ignore it, older servers never send it) and, like the rest of the
//! report, unauthenticated: a client opens only links on its own trusted portal.
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub const HANDSHAKE_VERSION: u16 = 1;
/// Bump when simulation semantics change incompatibly without a wire/map/catalog bump.
pub const GAMEPLAY_REVISION: &str = "dawnweaver-2026-10-11-reference";
pub const MAX_PROBE_BYTES: usize = 2048;
pub const PROBE_PADDING: usize = 512;
/// Longest update link a report carries. With the contract it stays well
/// below the smallest probe (`PROBE_PADDING` alone), so a reply never amplifies.
pub const MAX_UPDATE_URL_BYTES: usize = 256;

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
            .all(|v| valid_label(v))
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

/// A contract field: 1 to 64 characters of `[A-Za-z0-9._+-]`.
fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// What a server tells a client it may not admit (LR-14): the oldest game
/// release it admits and where players get a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateNotice {
    /// The oldest release the server admits: its own release.
    pub min_release: String,
    /// The market's update page (`<portal>/download`), when the server build
    /// knows its portal (`OMOBA_PORTAL_ORIGIN`, `scripts/markets.py`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl UpdateNotice {
    /// The notice this build's servers send. Client and server ship together
    /// (`docs/RELEASING.md`), so the oldest admitted release is the server's own.
    pub fn current() -> Self {
        Self {
            min_release: env!("CARGO_PKG_VERSION").into(),
            url: option_env!("OMOBA_PORTAL_ORIGIN")
                .map(|origin| download_page(origin, None))
                .filter(|url| valid_update_url(url)),
        }
    }

    /// A readable release and, if present, an HTTPS link of bounded size.
    pub fn valid(&self) -> bool {
        valid_label(&self.min_release)
            && ReleaseVersion::parse(&self.min_release).is_some()
            && self.url.as_deref().is_none_or(valid_update_url)
    }
}

/// The market portal's download page, optionally for one catalog platform
/// (`<portal>/download?platform=android`). The one place the update link is
/// built: servers put it in their [`UpdateNotice`], clients open it.
pub fn download_page(portal_origin: &str, platform: Option<&str>) -> String {
    let page = format!("{}/download", portal_origin.trim_end_matches('/'));
    match platform {
        Some(platform) => format!("{page}?platform={platform}"),
        None => page,
    }
}

/// An absolute HTTPS link of at most [`MAX_UPDATE_URL_BYTES`] without
/// credentials, fragment or whitespace: what an update link may be.
pub fn valid_update_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?']).next().unwrap_or_default();
    url.len() <= MAX_UPDATE_URL_BYTES
        && !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b))
        && url.bytes().all(|b| b.is_ascii_graphic())
        && !url.contains(['#', '@', '\\'])
}

/// A release label read by SemVer precedence (`0.45.0` < `0.46.0-rc.1` <
/// `0.46.0`); build metadata (`+…`) is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseVersion {
    core: [u64; 3],
    pre: Vec<String>,
}

impl ReleaseVersion {
    pub fn parse(label: &str) -> Option<Self> {
        let label = label.split_once('+').map_or(label, |(version, _)| version);
        let (core, pre) = label
            .split_once('-')
            .map_or((label, None), |(c, p)| (c, Some(p)));
        let mut numbers = core.split('.').map(|part| {
            (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u64>().ok())
                .flatten()
        });
        let core = [numbers.next()??, numbers.next()??, numbers.next()??];
        if numbers.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => {
                let parts: Vec<String> = pre.split('.').map(str::to_owned).collect();
                if parts
                    .iter()
                    .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric()))
                {
                    return None;
                }
                parts
            }
        };
        Some(Self { core, pre })
    }
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(a), Ok(b)) => a.cmp(&b),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            })
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// How two release labels order; `None` when either is unreadable.
pub fn release_order(a: &str, b: &str) -> Option<Ordering> {
    Some(ReleaseVersion::parse(a)?.cmp(&ReleaseVersion::parse(b)?))
}

/// Which side of a confirmed mismatch has to move (LR-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchSide {
    /// The server needs another game release: the player updates.
    ClientOutdated,
    /// The game is newer than the server: clients ship before the server
    /// (`docs/RELEASING.md`), so the server is about to be updated.
    ServerOutdated,
}

/// Decides a mismatch from the release labels. The server is behind only when
/// this game is strictly newer than the server's release and not older than
/// the minimum the server states; anything unreadable asks for an update.
pub fn mismatch_side(
    client: &ReleaseContract,
    server: &ReleaseContract,
    notice: Option<&UpdateNotice>,
) -> MismatchSide {
    let newer_than_server =
        release_order(&client.release, &server.release) == Some(Ordering::Greater);
    let below_minimum = notice.is_some_and(|notice| {
        !matches!(
            release_order(&client.release, &notice.min_release),
            Some(Ordering::Greater | Ordering::Equal)
        )
    });
    if newer_than_server && !below_minimum {
        MismatchSide::ServerOutdated
    } else {
        MismatchSide::ClientOutdated
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
        /// LR-14. Absent from older servers; older clients ignore it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        update: Option<UpdateNotice>,
    },
}

/// No socket, account, seat or match state is needed to answer this request.
pub fn response(bytes: &[u8], server: &ReleaseContract) -> Option<Vec<u8>> {
    response_with_update(bytes, server, None)
}

/// [`response`] that also carries `update` when it is valid and the reply
/// still fits the request; otherwise the plain report.
pub fn response_with_update(
    bytes: &[u8],
    server: &ReleaseContract,
    update: Option<&UpdateNotice>,
) -> Option<Vec<u8>> {
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
    let reply = |update: Option<UpdateNotice>| {
        serde_json::to_vec(&CompatibilityDatagram::CompatibilityReport {
            nonce: nonce.clone(),
            server: server.clone(),
            update,
        })
        .ok()
        .filter(|reply| reply.len() <= bytes.len())
    };
    reply(update.filter(|notice| notice.valid()).cloned()).or_else(|| reply(None))
}

/// A correlated, valid answer to a probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerReport {
    pub contract: ReleaseContract,
    /// `None` from older servers, or when the notice is not valid: the
    /// contract still counts, the client falls back to its own portal.
    pub update: Option<UpdateNotice>,
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
        self.accept_report(bytes).map(|report| report.contract)
    }

    /// [`Self::accept`] with the server's [`UpdateNotice`]. A report for another
    /// attempt (a stale nonce) is refused whole; a bad notice is dropped alone.
    pub fn accept_report(&self, bytes: &[u8]) -> Option<ServerReport> {
        if bytes.len() > MAX_PROBE_BYTES {
            return None;
        }
        let CompatibilityDatagram::CompatibilityReport {
            nonce,
            server,
            update,
        } = serde_json::from_slice(bytes).ok()?
        else {
            return None;
        };
        (nonce == self.nonce && server.valid()).then(|| ServerReport {
            contract: server,
            update: update.filter(UpdateNotice::valid),
        })
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

    fn notice(min_release: &str, url: Option<&str>) -> UpdateNotice {
        UpdateNotice {
            min_release: min_release.into(),
            url: url.map(str::to_owned),
        }
    }

    #[test]
    fn release_versions_follow_semver_precedence() {
        let ordered = [
            "0.9.9",
            "0.45.0-dev.1",
            "0.45.0-rc.1",
            "0.45.0-rc.2",
            "0.45.0-rc.10",
            "0.45.0-rc.10.1",
            "0.45.0",
            "0.45.1",
            "0.46.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert_eq!(
                release_order(pair[0], pair[1]),
                Some(Ordering::Less),
                "{pair:?}"
            );
            assert_eq!(release_order(pair[1], pair[0]), Some(Ordering::Greater));
        }
        assert_eq!(
            release_order("0.45.0+build.7", "0.45.0"),
            Some(Ordering::Equal)
        );
        for bad in [
            "",
            "0.45",
            "0.45.0.1",
            "v0.45.0",
            "0.45.x",
            "0..1",
            "0.45.0-",
            "0.45.0-a..b",
        ] {
            assert_eq!(ReleaseVersion::parse(bad), None, "{bad:?}");
            assert_eq!(release_order(bad, "0.45.0"), None);
        }
        assert!(ReleaseVersion::parse(env!("CARGO_PKG_VERSION")).is_some());
    }

    #[test]
    fn a_mismatch_asks_the_older_side_to_move() {
        let client = |release: &str| ReleaseContract {
            release: release.into(),
            ..ReleaseContract::current()
        };
        let server = client("0.46.0");
        // An older game updates; a newer game waits for the servers.
        assert_eq!(
            mismatch_side(&client("0.45.0"), &server, None),
            MismatchSide::ClientOutdated
        );
        assert_eq!(
            mismatch_side(&client("0.47.0"), &server, None),
            MismatchSide::ServerOutdated
        );
        // The same label with another contract (a dev build) or an unreadable
        // label never claims the servers are behind.
        for release in ["0.46.0", "next"] {
            assert_eq!(
                mismatch_side(&client(release), &server, None),
                MismatchSide::ClientOutdated
            );
        }
        // The server's stated minimum wins over its own label.
        let raised = notice("0.48.0", None);
        assert_eq!(
            mismatch_side(&client("0.47.0"), &server, Some(&raised)),
            MismatchSide::ClientOutdated
        );
        let met = notice("0.46.0", None);
        assert_eq!(
            mismatch_side(&client("0.47.0"), &server, Some(&met)),
            MismatchSide::ServerOutdated
        );
    }

    #[test]
    fn update_notices_validate_release_and_link() {
        assert!(notice("0.46.0", None).valid());
        assert!(notice("0.46.0", Some("https://portal.example/download")).valid());
        assert!(notice("0.46.0-rc.1", Some("https://127.0.0.1:8443/download")).valid());
        for bad in [
            notice("", None),
            notice("latest", None),
            notice(&"9".repeat(65), None),
            notice("0.46.0", Some("http://portal.example/download")),
            notice("0.46.0", Some("https://")),
            notice("0.46.0", Some("https://user@evil.example/download")),
            notice("0.46.0", Some("https://portal.example/download#x")),
            notice("0.46.0", Some("https://portal.example/down load")),
            notice("0.46.0", Some("https://portal.example\\@evil.example/")),
            notice("0.46.0", Some("javascript:alert(1)")),
            notice(
                "0.46.0",
                Some(&format!(
                    "https://portal.example/{}",
                    "a".repeat(MAX_UPDATE_URL_BYTES)
                )),
            ),
        ] {
            assert!(!bad.valid(), "{bad:?}");
        }
        assert_eq!(
            download_page("https://portal.example", Some("android")),
            "https://portal.example/download?platform=android"
        );
        assert_eq!(
            download_page("https://portal.example/", None),
            "https://portal.example/download"
        );
        let current = UpdateNotice::current();
        assert!(current.valid());
        assert_eq!(current.min_release, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            current.url,
            option_env!("OMOBA_PORTAL_ORIGIN").map(|origin| download_page(origin, None))
        );
    }

    #[test]
    fn reports_carry_a_valid_update_notice_and_drop_a_bad_one() {
        let current = ReleaseContract::current();
        let probe = CompatibilityProbe::new(current.clone(), "ab".repeat(16));
        let request = probe.request();
        let good = notice("0.46.0", Some("https://portal.example/download"));
        let reply = response_with_update(&request, &current, Some(&good)).unwrap();
        assert!(reply.len() <= request.len());
        assert_eq!(
            probe.accept_report(&reply),
            Some(ServerReport {
                contract: current.clone(),
                update: Some(good.clone()),
            })
        );
        // A server never sends an invalid notice; it answers without one.
        let bad = notice("0.46.0", Some("http://portal.example/download"));
        let reply = response_with_update(&request, &current, Some(&bad)).unwrap();
        assert_eq!(probe.accept_report(&reply).unwrap().update, None);
        // A forged report with a bad notice keeps its contract, not the link.
        for forged in [
            serde_json::json!({"min_release":"0.46.0","url":"https://evil.example@portal.example/"}),
            serde_json::json!({"min_release":"not-a-version"}),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "type":"compatibility_report","nonce":"ab".repeat(16),
                "server":current,"update":forged,
            }))
            .unwrap();
            let report = probe.accept_report(&bytes).unwrap();
            assert_eq!((report.contract, report.update), (current.clone(), None));
        }
        // A report for an earlier attempt (expired nonce) is refused whole.
        let stale = CompatibilityProbe::new(current.clone(), "cd".repeat(16));
        let reply = response_with_update(&request, &current, Some(&good)).unwrap();
        assert!(stale.accept_report(&reply).is_none());
    }

    /// A pre-LR-14 client reads a report with a notice, and a pre-LR-14 report
    /// still reads without one.
    #[test]
    fn the_update_notice_is_additive_on_the_wire() {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum LegacyDatagram {
            #[allow(dead_code)]
            CompatibilityProbe {},
            CompatibilityReport {
                nonce: String,
                server: ReleaseContract,
            },
        }
        let current = ReleaseContract::current();
        let probe = CompatibilityProbe::new(current.clone(), "ab".repeat(16));
        let request = probe.request();
        let reply = response_with_update(
            &request,
            &current,
            Some(&notice("0.46.0", Some("https://portal.example/download"))),
        )
        .unwrap();
        let LegacyDatagram::CompatibilityReport { nonce, server } =
            serde_json::from_slice(&reply).unwrap()
        else {
            panic!("a report decodes as a report")
        };
        assert_eq!((nonce, server), ("ab".repeat(16), current.clone()));
        let legacy = response(&request, &current).unwrap();
        assert!(!String::from_utf8_lossy(&legacy).contains("update"));
        assert_eq!(probe.accept_report(&legacy).unwrap().update, None);
    }
}
