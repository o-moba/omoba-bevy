//! Server-authorized, short-lived map cosmetics. No gameplay or payment authority.
//! The Registry origin comes only from the selected connection profile.
pub mod model;
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

pub const SLOT: &str = shared::sponsorship::SLOT_ID;
pub const PROFILE: &str = "map-prop-glb-v1";
const MANIFEST_LIMIT: u64 = 32 * 1024;
const REFRESH: Duration = Duration::from_secs(30);
const MAX_STALE: Duration = Duration::from_secs(90);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Placement {
    pub id: String,
    pub slot_id: String,
    pub asset_id: String,
    pub revision_id: String,
    pub rendition_sha256: String,
    pub rendition_bytes: u64,
    pub rendition_url: String,
    pub title: String,
    pub credit: String,
    pub starts_at: String,
    pub expires_at: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub server_now: String,
    pub refresh_after_seconds: u64,
    pub max_stale_seconds: u64,
    pub placements: Vec<Placement>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Ready {
    pub placement: Placement,
    pub asset_path: String,
    pub fit: model::ModelFit,
}
struct Authorization {
    placement: Placement,
    expires: Instant,
    stale: Instant,
}
impl Authorization {
    fn live(&self, now: Instant) -> bool {
        now < self.expires && now < self.stale
    }
}
struct ReadResult {
    server_now: Option<f64>,
    result: Result<Option<Authorization>, String>,
}
struct DownloadResult {
    identity: Placement,
    result: Result<(String, model::ModelFit), String>,
}
struct Runtime {
    base: Url,
    client: Client,
    root: PathBuf,
    state: Mutex<State>,
    reads_tx: mpsc::Sender<ReadResult>,
    reads_rx: Mutex<mpsc::Receiver<ReadResult>>,
    downloads_tx: mpsc::Sender<DownloadResult>,
    downloads_rx: Mutex<mpsc::Receiver<DownloadResult>>,
}
#[derive(Default)]
struct State {
    authorization: Option<Authorization>,
    ready: Option<Ready>,
    reading: bool,
    downloading: Option<Placement>,
    last_attempt: Option<Instant>,
    // Survives expiry and empty feeds so cached grants cannot restart their lease.
    server_time_watermark: Option<f64>,
    error: Option<String>,
}
static RUNTIME: OnceLock<Option<Runtime>> = OnceLock::new();
fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }
        })
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn origin(base: &str) -> Result<Url, String> {
    let url = Url::parse(base).map_err(|_| "Invalid registry origin")?;
    let local = matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    );
    if !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("Invalid registry origin".into());
    }
    Ok(url)
}
fn rendition_url(base: &Url, p: &Placement) -> Result<Url, String> {
    let expected = base
        .join(&format!(
            "/v1/studio/assets/{}/rendition",
            p.rendition_sha256
        ))
        .map_err(|e| e.to_string())?;
    let actual = Url::parse(&p.rendition_url).map_err(|e| e.to_string())?;
    if actual != expected {
        return Err("Rendition URL is not the trusted immutable Registry route".into());
    }
    Ok(actual)
}
/// Strict UTC RFC3339 (Z or +00:00, optional fractional seconds). No local clock.
fn timestamp(value: &str) -> Result<f64, String> {
    let s = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
        .ok_or("Expected UTC timestamp")?;
    if !s.is_ascii()
        || s.len() < 19
        || &s[4..5] != "-"
        || &s[7..8] != "-"
        || &s[10..11] != "T"
        || &s[13..14] != ":"
        || &s[16..17] != ":"
    {
        return Err("Invalid timestamp".into());
    }
    let part = |a, b| {
        s[a..b]
            .parse::<i64>()
            .map_err(|_| "Invalid timestamp".to_owned())
    };
    let (mut y, m, d, h, min, sec) = (
        part(0, 4)?,
        part(5, 7)?,
        part(8, 10)?,
        part(11, 13)?,
        part(14, 16)?,
        part(17, 19)?,
    );
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || d < 1
        || d > days[(m - 1) as usize]
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&min)
        || !(0..=59).contains(&sec)
    {
        return Err("Invalid calendar timestamp".into());
    }
    let fraction = if s.len() == 19 {
        0.
    } else {
        let f = &s[19..];
        if !f.starts_with('.')
            || f.len() < 2
            || f.len() > 10
            || !f[1..].bytes().all(|b| b.is_ascii_digit())
        {
            return Err("Invalid timestamp fraction".into());
        }
        f.parse::<f64>().map_err(|_| "Invalid timestamp fraction")?
    };
    y -= i64::from(m <= 2);
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    Ok((days * 86400 + h * 3600 + min * 60 + sec) as f64 + fraction)
}
fn authorize(base: &Url, m: Manifest, started: Instant) -> Result<Option<Authorization>, String> {
    if m.refresh_after_seconds != 30 || m.max_stale_seconds != 90 || m.placements.len() > 1 {
        return Err("Unsupported placement policy".into());
    }
    let now = timestamp(&m.server_now)?;
    let Some(p) = m.placements.into_iter().next() else {
        return Ok(None);
    };
    if !uuid(&p.id)
        || !uuid(&p.revision_id)
        || !p.asset_id.strip_prefix("ekza:map_prop:").is_some_and(uuid)
        || p.slot_id != SLOT
        || !digest(&p.rendition_sha256)
        || !(28..=model::MAX_BYTES as u64).contains(&p.rendition_bytes)
        || p.title.is_empty()
        || p.title.chars().count() > 80
        || p.credit.chars().count() > 120
        || p.title
            .chars()
            .chain(p.credit.chars())
            .any(char::is_control)
    {
        return Err("Invalid placement identity or metadata".into());
    }
    rendition_url(base, &p)?;
    let start = timestamp(&p.starts_at)?;
    let end = timestamp(&p.expires_at)?;
    if start > now || end <= now || end - start > 86400.001 || end <= start {
        return Err("Placement is outside its authorization interval".into());
    }
    Ok(Some(Authorization {
        placement: p,
        expires: started + Duration::from_secs_f64(end - now),
        stale: started + MAX_STALE,
    }))
}
fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())
}
fn fetch(client: &Client, url: Url, limit: u64) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .header("Cache-Control", "no-cache")
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > limit) {
        return Err("Response exceeds policy".into());
    }
    let mut bytes = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Response exceeds policy".into());
    }
    Ok(bytes)
}
fn evaluate_manifest(base: &Url, m: Manifest, started: Instant) -> ReadResult {
    let server_now = timestamp(&m.server_now).ok();
    ReadResult {
        server_now,
        result: Ok(authorize(base, m, started).ok().flatten()),
    }
}
fn read_manifest(client: &Client, base: &Url, started: Instant) -> ReadResult {
    let bytes = fetch(
        client,
        base.join("/v1/projects/omoba/placements/active")
            .expect("validated Registry origin"),
        MANIFEST_LIMIT,
    );
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            return ReadResult {
                server_now: None,
                result: Err(error),
            };
        }
    };
    // A malformed trusted feed is a denial, not an outage grace period.
    let Ok(m) = serde_json::from_slice(&bytes) else {
        return ReadResult {
            server_now: None,
            result: Ok(None),
        };
    };
    evaluate_manifest(base, m, started)
}
fn verified(bytes: &[u8], p: &Placement) -> Result<model::ModelFit, String> {
    if bytes.len() as u64 != p.rendition_bytes
        || format!("{:x}", Sha256::digest(bytes)) != p.rendition_sha256
    {
        return Err("Map prop hash or size mismatch".into());
    }
    model::validate(bytes)
}
fn install(
    client: &Client,
    base: &Url,
    root: &Path,
    p: &Placement,
) -> Result<(String, model::ModelFit), String> {
    let url = rendition_url(base, p)?;
    let folder = root.join("map-props");
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!("{}.glb", p.rendition_sha256));
    if let Ok(meta) = fs::metadata(&path)
        && meta.len() == p.rendition_bytes
        && let Ok(bytes) = fs::read(&path)
        && let Ok(fit) = verified(&bytes, p)
    {
        return Ok((format!("ekza://map-props/{}.glb", p.rendition_sha256), fit));
    }
    let bytes = fetch(client, url, p.rendition_bytes)?;
    let fit = verified(&bytes, p)?;
    let temporary = folder.join(format!("{}.part", p.rendition_sha256));
    fs::write(&temporary, &bytes)
        .and_then(|_| fs::rename(&temporary, &path))
        .map_err(|e| e.to_string())?;
    Ok((format!("ekza://map-props/{}.glb", p.rendition_sha256), fit))
}
impl State {
    fn apply_read(&mut self, read: ReadResult, now: Instant) {
        self.reading = false;
        match read.result {
            Ok(mut auth) => {
                self.error = None;
                // Every healthy snapshot must advance Registry time, including empty
                // revocation feeds. Keep this watermark after authorization expires.
                // A backwards server clock fails closed until it catches up.
                if let Some(server_now) = read.server_now {
                    if self
                        .server_time_watermark
                        .is_some_and(|previous| server_now <= previous)
                    {
                        auth = None;
                    } else {
                        self.server_time_watermark = Some(server_now);
                    }
                } else {
                    auth = None;
                }
                // Replayed/cached serverNow must never renew the same grant.
                if let (Some(old), Some(next)) = (&self.authorization, &mut auth)
                    && old.placement.id == next.placement.id
                {
                    next.expires = next.expires.min(old.expires);
                }
                self.authorization = auth.filter(|a| a.live(now));
                if self.ready.as_ref().is_some_and(|r| {
                    self.authorization
                        .as_ref()
                        .is_none_or(|a| a.placement != r.placement)
                }) {
                    self.ready = None;
                }
            }
            Err(e) => self.error = Some(e),
        }
        self.expire(now);
    }
    fn apply_download(&mut self, download: DownloadResult, now: Instant) {
        self.downloading = None;
        if self
            .authorization
            .as_ref()
            .is_none_or(|a| !a.live(now) || a.placement != download.identity)
        {
            return;
        }
        match download.result {
            Ok((asset_path, fit)) => {
                self.ready = Some(Ready {
                    placement: download.identity,
                    asset_path,
                    fit,
                })
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn expire(&mut self, now: Instant) {
        if self.authorization.as_ref().is_some_and(|a| !a.live(now)) {
            self.authorization = None;
            self.ready = None;
        }
    }
}
/// Initializes only profile-isolated runtime state; no cached authorization is restored.
pub fn initialize(root: PathBuf) {
    RUNTIME.get_or_init(|| {
        let base = origin(&crate::community::registry_url()).ok()?;
        let client = client().ok()?;
        let (reads_tx, reads_rx) = mpsc::channel();
        let (downloads_tx, downloads_rx) = mpsc::channel();
        Some(Runtime {
            base,
            client,
            root,
            state: Mutex::default(),
            reads_tx,
            reads_rx: Mutex::new(reads_rx),
            downloads_tx,
            downloads_rx: Mutex::new(downloads_rx),
        })
    });
}
/// Called once per frame. Only channel reads/state transitions happen on this thread.
pub fn poll() -> Option<Ready> {
    let runtime = RUNTIME.get()?.as_ref()?;
    let now = Instant::now();
    let mut state = runtime.state.lock().unwrap();
    while let Ok(result) = runtime.reads_rx.lock().unwrap().try_recv() {
        state.apply_read(result, now);
    }
    while let Ok(result) = runtime.downloads_rx.lock().unwrap().try_recv() {
        state.apply_download(result, now);
    }
    state.expire(now);
    if !state.reading
        && state
            .last_attempt
            .is_none_or(|at| now.duration_since(at) >= REFRESH)
    {
        state.reading = true;
        state.last_attempt = Some(now);
        let (client, base, tx) = (
            runtime.client.clone(),
            runtime.base.clone(),
            runtime.reads_tx.clone(),
        );
        std::thread::spawn(move || {
            let _ = tx.send(read_manifest(&client, &base, now));
        });
    }
    if state.downloading.is_none()
        && state.ready.is_none()
        && state.error.is_none()
        && let Some(auth) = &state.authorization
    {
        let p = auth.placement.clone();
        state.downloading = Some(p.clone());
        let (client, base, root, tx) = (
            runtime.client.clone(),
            runtime.base.clone(),
            runtime.root.clone(),
            runtime.downloads_tx.clone(),
        );
        std::thread::spawn(move || {
            let result = install(&client, &base, &root, &p);
            let _ = tx.send(DownloadResult {
                identity: p,
                result,
            });
        });
    }
    state.ready.clone()
}
#[cfg(test)]
mod tests;
