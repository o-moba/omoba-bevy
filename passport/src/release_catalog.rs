//! The market's release catalog (LR-14): `GET /v1/releases` of the Account API
//! this build talks to (`OMOBA_ACCOUNT_API_URL`), the same list the portal's
//! download page shows (`account-api/README.md#downloads`). The game reads it
//! at boot and after a compatibility mismatch to offer an update.
//!
//! The catalog arrives over the build's trusted HTTPS endpoint. It is still
//! checked here: a malformed or oversized document is refused whole, and an
//! entry with an unreadable version or a link that is not plain HTTPS is
//! skipped, so one bad row never hides the others.
use crate::web_account::WebAccountApi;
use serde::Deserialize;
use shared::compatibility::{ReleaseVersion, valid_update_url};

/// The Account API refuses a larger catalog file (`account-api/src/read.rs`).
pub const MAX_CATALOG_BYTES: usize = 131_072;
/// The Account API lists at most this many entries.
pub const MAX_RELEASES: usize = 32;
/// The architecture of a package that installs on every architecture of its
/// platform (`scripts/release.py` `CATALOG_PLATFORMS`: the Android APK).
pub const UNIVERSAL: &str = "universal";

/// One downloadable package (the Account API's Release schema; the fields the
/// game needs).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogRelease {
    pub platform: String,
    pub architecture: String,
    pub version: String,
    pub url: String,
}

impl CatalogRelease {
    fn valid(&self) -> bool {
        let name = |value: &str| {
            !value.is_empty()
                && value.len() <= 32
                && value
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
        };
        name(&self.platform)
            && name(&self.architecture)
            && self.version.len() <= 64
            && ReleaseVersion::parse(&self.version).is_some()
            && valid_update_url(&self.url)
    }

    fn release_version(&self) -> Option<ReleaseVersion> {
        ReleaseVersion::parse(&self.version)
    }
}

/// The valid entries of a catalog document.
pub fn parse(bytes: &[u8]) -> Result<Vec<CatalogRelease>, String> {
    #[derive(Deserialize)]
    struct Document {
        releases: Vec<serde_json::Value>,
    }
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err("Release catalog is too large.".into());
    }
    let document: Document =
        serde_json::from_slice(bytes).map_err(|_| "Invalid release catalog.")?;
    if document.releases.len() > MAX_RELEASES {
        return Err("Release catalog lists too many releases.".into());
    }
    Ok(document
        .releases
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<CatalogRelease>(entry).ok())
        .filter(CatalogRelease::valid)
        .collect())
}

/// The newest release for `platform` and `architecture` (a [`UNIVERSAL`]
/// package matches any architecture).
pub fn newest<'a>(
    releases: &'a [CatalogRelease],
    platform: &str,
    architecture: &str,
) -> Option<&'a CatalogRelease> {
    releases
        .iter()
        .filter(|release| {
            release.platform == platform
                && (release.architecture == architecture || release.architecture == UNIVERSAL)
        })
        .filter(|release| release.release_version().is_some())
        .max_by(|a, b| a.release_version().cmp(&b.release_version()))
}

impl WebAccountApi {
    /// The market's release catalog (`GET /v1/releases`, public, no session).
    pub fn releases(&self) -> Result<Vec<CatalogRelease>, String> {
        parse(&self.get_bytes("v1/releases", MAX_CATALOG_BYTES)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn entry(platform: &str, architecture: &str, version: &str, url: &str) -> serde_json::Value {
        json!({
            "platform": platform, "architecture": architecture, "version": version, "url": url,
            "sha256": "0".repeat(64), "size_bytes": "1", "published_at": "2026-10-24T00:00:00Z",
            "instructions": "Unpack and run.", "test_status": "playtest",
        })
    }

    fn document(releases: Vec<serde_json::Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({ "releases": releases })).unwrap()
    }

    #[test]
    fn newest_release_per_platform_with_universal_packages() {
        let releases = parse(&document(vec![
            entry(
                "linux",
                "x86_64",
                "0.45.0",
                "https://downloads.example/a.tar.gz",
            ),
            entry(
                "linux",
                "x86_64",
                "0.46.0",
                "https://downloads.example/b.tar.gz",
            ),
            entry(
                "linux",
                "x86_64",
                "0.46.0-rc.2",
                "https://downloads.example/c.tar.gz",
            ),
            entry(
                "windows",
                "x86_64",
                "0.47.0",
                "https://downloads.example/w.zip",
            ),
            entry(
                "android",
                UNIVERSAL,
                "0.46.0",
                "https://downloads.example/o.apk",
            ),
        ]))
        .unwrap();
        assert_eq!(releases.len(), 5);
        let linux = newest(&releases, "linux", "x86_64").unwrap();
        assert_eq!(linux.version, "0.46.0");
        assert_eq!(linux.url, "https://downloads.example/b.tar.gz");
        assert_eq!(
            newest(&releases, "android", "arm64").unwrap().version,
            "0.46.0"
        );
        assert!(newest(&releases, "linux", "arm64").is_none());
        assert!(newest(&releases, "ios", "arm64").is_none());
        assert!(newest(&[], "linux", "x86_64").is_none());
    }

    #[test]
    fn bad_documents_are_refused_and_bad_entries_skipped() {
        for bad in [
            b"".to_vec(),
            b"not json".to_vec(),
            b"{}".to_vec(),
            br#"{"releases":{}}"#.to_vec(),
            document(vec![
                entry(
                    "linux",
                    "x86_64",
                    "0.46.0",
                    "https://d.example/x"
                );
                33
            ]),
            vec![b' '; MAX_CATALOG_BYTES + 1],
        ] {
            assert!(parse(&bad).is_err(), "{:?}", String::from_utf8_lossy(&bad));
        }
        let good = entry("linux", "x86_64", "0.46.0", "https://d.example/ok");
        let releases = parse(&document(vec![
            good,
            entry("linux", "x86_64", "0.99.0", "http://d.example/plain-http"),
            entry(
                "linux",
                "x86_64",
                "0.99.0",
                "https://user@d.example/credentials",
            ),
            entry("linux", "x86_64", "0.99.0", "javascript:alert(1)"),
            entry("linux", "x86_64", "latest", "https://d.example/no-version"),
            entry(
                "Linux",
                "x86_64",
                "0.99.0",
                "https://d.example/odd-platform",
            ),
            json!({"platform": "linux", "architecture": "x86_64", "version": "0.99.0"}),
            json!("not an object"),
        ]))
        .unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].url, "https://d.example/ok");
    }

    /// An honest empty catalog (`{"releases":[]}`) offers nothing; an old or
    /// stale catalog is just older entries, which never read as newer.
    #[test]
    fn empty_and_stale_catalogs_offer_nothing_newer() {
        assert_eq!(parse(&document(vec![])).unwrap(), vec![]);
        let stale = parse(&document(vec![entry(
            "linux",
            "x86_64",
            "0.1.0",
            "https://d.example/old",
        )]))
        .unwrap();
        let newest = newest(&stale, "linux", "x86_64").unwrap();
        assert_eq!(
            shared::compatibility::release_order(&newest.version, env!("CARGO_PKG_VERSION")),
            Some(std::cmp::Ordering::Less)
        );
    }

    /// One HTTP exchange on loopback: the status line and body to answer with.
    fn serve_once(status: &'static str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]).into_owned();
            assert!(request.starts_with("GET /v1/releases "), "{request}");
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            // The client may stop reading an oversized body early.
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        });
        origin
    }

    #[test]
    fn the_catalog_is_read_from_the_account_api() {
        let body = document(vec![entry(
            "linux",
            "x86_64",
            "0.46.0",
            "https://d.example/l",
        )]);
        let api =
            WebAccountApi::new(&serve_once("200 OK", body), "http://127.0.0.1:3010", true).unwrap();
        let releases = api.releases().unwrap();
        assert_eq!(
            newest(&releases, "linux", "x86_64").unwrap().version,
            "0.46.0"
        );
        for (status, body) in [
            (
                "503 Service Unavailable",
                br#"{"error":"catalog_unavailable"}"#.to_vec(),
            ),
            ("200 OK", vec![b' '; MAX_CATALOG_BYTES + 1]),
            ("200 OK", b"<html>".to_vec()),
        ] {
            let api = WebAccountApi::new(&serve_once(status, body), "http://127.0.0.1:3010", true)
                .unwrap();
            assert!(api.releases().is_err(), "{status}");
        }
    }
}
