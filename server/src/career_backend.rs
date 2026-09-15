//! Signed account boundary and bounded off-tick PostgreSQL worker.
use crate::career_store::CareerStore;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use shared::career::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

const MAX_CLIENTS: usize = 512;
const MAX_JOBS: usize = 256;
const CHALLENGE_TTL: Duration = Duration::from_secs(30);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0; N];
    for (i, item) in out.iter_mut().enumerate() {
        *item = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}
fn random_id<const N: usize>() -> String {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("OS randomness required for account authentication");
    hex(&bytes)
}
fn verify(public_key: &str, signature: &str, message: &[u8]) -> bool {
    let Some(key) = decode::<32>(public_key).and_then(|b| VerifyingKey::from_bytes(&b).ok()) else {
        return false;
    };
    let Some(signature) = decode::<64>(signature).map(|b| Signature::from_bytes(&b)) else {
        return false;
    };
    key.verify_strict(message, &signature).is_ok()
}

struct Client {
    view: CareerView,
    challenge: Option<(AuthChallenge, Instant)>,
    auth: Option<AuthChallenge>,
    sequence: u64,
    last_request: Option<Instant>,
    touched: Instant,
    playing: bool,
    profile_ready: bool,
}
impl Default for Client {
    fn default() -> Self {
        Self {
            view: CareerView::default(),
            challenge: None,
            auth: None,
            sequence: 0,
            last_request: None,
            touched: Instant::now(),
            playing: false,
            profile_ready: false,
        }
    }
}

enum Job {
    Login {
        addr: SocketAddr,
        challenge: AuthChallenge,
    },
    Action {
        addr: SocketAddr,
        nonce: String,
        profile_id: String,
        action: CareerAction,
    },
    Record(Box<PendingRecord>),
    Presence(Vec<(String, Option<String>)>),
}
enum Reply {
    Login {
        addr: SocketAddr,
        challenge: AuthChallenge,
        result: Result<CareerView, String>,
    },
    Action {
        addr: SocketAddr,
        nonce: String,
        result: Result<CareerView, String>,
        request_id: Option<u64>,
    },
    Started(String),
    RecordError(String, String),
    Rejected(String, String),
    Settled {
        result: MatchResult,
        profiles: Vec<ProfileSummary>,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum RecordKind {
    Start,
    Checkpoint,
    Settle,
    Rejected,
}
#[derive(Clone, Serialize, Deserialize)]
struct PendingRecord {
    kind: RecordKind,
    result: MatchResult,
    #[serde(default)]
    recovery_allocation: Option<MatchResult>,
    #[serde(default)]
    recovered_live: bool,
}

pub struct CareerBackend {
    epoch: u64,
    tx: Option<SyncSender<Job>>,
    rx: Mutex<Receiver<Reply>>,
    clients: HashMap<SocketAddr, Client>,
    starts: HashMap<String, Result<(), String>>,
    settled: Vec<MatchResult>,
    cancelled: Vec<SocketAddr>,
    social: Vec<(SocketAddr, shared::social::SocialRequest)>,
    last_presence: Instant,
    matches: HashMap<String, String>,
    pending_ids: HashSet<String>,
    rejected: HashMap<String, String>,
    #[cfg(test)]
    _test_jobs: Option<Mutex<Receiver<Job>>>,
}
impl CareerBackend {
    pub fn new(epoch: u64) -> Self {
        let url = std::env::var("OMOBA_DATABASE_URL")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let outbox = std::env::var_os("OMOBA_CAREER_OUTBOX")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".omoba/career-outbox"));
        Self::with_config(epoch, url, outbox)
    }
    fn with_config(epoch: u64, url: Option<String>, outbox: PathBuf) -> Self {
        let (reply_tx, rx) = mpsc::sync_channel(MAX_JOBS * 2);
        let tx = url.map(|url| {
            let (tx, jobs) = mpsc::sync_channel(MAX_JOBS);
            std::thread::Builder::new()
                .name("omoba-career".into())
                .spawn(move || worker(url, outbox, jobs, reply_tx))
                .expect("career worker starts");
            tx
        });
        Self {
            epoch,
            tx,
            rx: Mutex::new(rx),
            clients: HashMap::new(),
            starts: HashMap::new(),
            settled: Vec::new(),
            cancelled: Vec::new(),
            social: Vec::new(),
            last_presence: Instant::now(),
            matches: HashMap::new(),
            pending_ids: HashSet::new(),
            rejected: HashMap::new(),
            #[cfg(test)]
            _test_jobs: None,
        }
    }
    #[cfg(test)]
    pub fn test_with_database(epoch: u64, url: String, outbox: PathBuf) -> Self {
        Self::with_config(epoch, Some(url), outbox)
    }
    #[cfg(test)]
    pub fn test_ack_settle(&mut self, result: MatchResult) {
        self.settled.push(result);
    }
    #[cfg(test)]
    pub fn test_backend(epoch: u64) -> Self {
        let (tx, jobs) = mpsc::sync_channel(MAX_JOBS);
        let (_, rx) = mpsc::sync_channel(MAX_JOBS);
        Self {
            epoch,
            tx: Some(tx),
            rx: Mutex::new(rx),
            clients: HashMap::new(),
            starts: HashMap::new(),
            settled: Vec::new(),
            cancelled: Vec::new(),
            last_presence: Instant::now(),
            social: Vec::new(),
            matches: HashMap::new(),
            pending_ids: HashSet::new(),
            rejected: HashMap::new(),
            _test_jobs: Some(Mutex::new(jobs)),
        }
    }
    #[cfg(test)]
    pub fn test_authenticated(
        &mut self,
        addr: SocketAddr,
        profile: ProfileSummary,
        session_id: &str,
    ) {
        let challenge = AuthChallenge {
            public_key: hex(ed25519_dalek::SigningKey::from_bytes(&[7; 32])
                .verifying_key()
                .as_bytes()),
            nonce: "b".repeat(64),
            server_epoch: self.epoch,
            session_id: session_id.into(),
            nickname: profile.nickname.clone(),
        };
        self.clients.insert(
            addr,
            Client {
                view: CareerView {
                    auth_nonce: Some(challenge.nonce.clone()),
                    profile: Some(profile),
                    ..Default::default()
                },
                auth: Some(challenge),
                profile_ready: true,
                ..Default::default()
            },
        );
    }
    #[cfg(test)]
    pub fn test_reject_start(&mut self, id: &str, error: &str) {
        self.rejected.insert(id.into(), error.into());
    }
    #[cfg(test)]
    pub fn test_ack_start(&mut self, id: &str) {
        self.starts.insert(id.into(), Ok(()));
    }
    pub fn enabled(&self) -> bool {
        self.tx.is_some()
    }
    pub fn new_result_id(&self) -> String {
        random_id::<16>()
    }
    pub fn profile(&self, addr: SocketAddr) -> Option<ProfileSummary> {
        let client = self.clients.get(&addr)?;
        client
            .profile_ready
            .then(|| client.view.profile.clone())
            .flatten()
    }
    pub fn authenticated_session(&self, addr: SocketAddr) -> Option<String> {
        Some(self.clients.get(&addr)?.auth.as_ref()?.session_id.clone())
    }
    pub fn view(&self, addr: SocketAddr) -> CareerView {
        let mut view = self.clients.get(&addr).map(|c|c.view.clone()).unwrap_or_else(|| CareerView {error:(!self.enabled()).then(||"Career storage is not configured on this server. Guest matches are unranked.".into()),..Default::default()});
        view.storage_enabled = self.enabled();
        view
    }
    pub fn forget(&mut self, addr: SocketAddr) {
        self.clients.remove(&addr);
    }
    pub fn touch(&mut self, addr: SocketAddr) {
        if let Some(c) = self.clients.get_mut(&addr) {
            c.touched = Instant::now();
        }
    }
    pub fn set_playing(&mut self, addr: SocketAddr, playing: bool) {
        if let Some(c) = self.clients.get_mut(&addr) {
            c.playing = playing;
        }
    }
    #[cfg(test)]
    pub fn test_is_playing(&self, addr: SocketAddr) -> bool {
        self.clients.get(&addr).is_some_and(|client| client.playing)
    }
    pub fn take_cancelled(&mut self) -> Vec<SocketAddr> {
        std::mem::take(&mut self.cancelled)
    }

    /// Ephemeral authenticated messages stay on the game thread, outside SQL.
    pub fn take_social(&mut self) -> Vec<(SocketAddr, shared::social::SocialRequest)> {
        std::mem::take(&mut self.social)
    }
    pub fn take_settled(&mut self) -> Vec<MatchResult> {
        std::mem::take(&mut self.settled)
    }
    pub fn started(&self, id: &str) -> bool {
        matches!(self.starts.get(id), Some(Ok(())))
    }
    pub fn start_rejected(&self, id: &str) -> Option<String> {
        self.rejected.get(id).cloned()
    }
    pub fn forget_start(&mut self, id: &str) {
        self.starts.remove(id);
        self.rejected.remove(id);
    }
    pub fn start_error(&self, id: &str) -> Option<String> {
        self.starts.get(id).and_then(|v| v.as_ref().err()).cloned()
    }
    pub fn start(&mut self, result: MatchResult) -> bool {
        self.record(RecordKind::Start, result)
    }
    pub fn checkpoint(&mut self, result: MatchResult) -> bool {
        self.record(RecordKind::Checkpoint, result)
    }
    pub fn settle(&mut self, result: MatchResult) -> bool {
        self.record(RecordKind::Settle, result)
    }
    fn record(&mut self, kind: RecordKind, result: MatchResult) -> bool {
        if self.rejected.contains_key(&result.result_id) {
            return false;
        }
        if !self.pending_ids.contains(&result.result_id) && self.pending_ids.len() >= 32 {
            return false;
        }
        let accepted = self.tx.as_ref().is_some_and(|tx| {
            tx.try_send(Job::Record(Box::new(PendingRecord {
                kind,
                result: result.clone(),
                recovery_allocation: None,
                recovered_live: false,
            })))
            .is_ok()
        });
        if accepted {
            self.pending_ids.insert(result.result_id.clone());
        }
        if accepted && matches!(kind, RecordKind::Start) {
            for p in &result.participants {
                if let Some(id) = &p.profile_id {
                    self.matches.insert(id.clone(), result.result_id.clone());
                }
            }
        }
        accepted
    }
    pub fn handle(&mut self, addr: SocketAddr, request: CareerRequest) {
        if !self.enabled() {
            return;
        }
        if !self.clients.contains_key(&addr) && self.clients.len() >= MAX_CLIENTS {
            return;
        }
        let c = self.clients.entry(addr).or_default();
        let now = Instant::now();
        if c.last_request
            .is_some_and(|at| now.duration_since(at) < Duration::from_millis(100))
        {
            return;
        }
        c.last_request = Some(now);
        c.touched = now;
        match request {
            CareerRequest::Challenge {
                public_key,
                nickname,
                session_id,
            } => {
                if c.playing && c.auth.is_some() {
                    return;
                }
                let Ok(nickname) = normalize_nickname(&nickname) else {
                    c.view.error = Some("Invalid nickname.".into());
                    return;
                };
                if decode::<32>(&public_key).is_none()
                    || session_id.is_empty()
                    || session_id.len() > 64
                    || !session_id
                        .bytes()
                        .all(|v| v.is_ascii_alphanumeric() || b"._-".contains(&v))
                {
                    return;
                }
                if let Some((old, at)) = &c.challenge {
                    if old.public_key == public_key
                        && old.session_id == session_id
                        && old.nickname == nickname
                        && at.elapsed() < CHALLENGE_TTL
                    {
                        c.view.challenge = Some(old.clone());
                        return;
                    }
                }
                let challenge = AuthChallenge {
                    public_key,
                    nonce: random_id::<32>(),
                    server_epoch: self.epoch,
                    session_id,
                    nickname,
                };
                // A new handshake invalidates old authorization on this endpoint.
                c.auth = None;
                c.sequence = 0;
                c.view = CareerView::default();
                c.challenge = Some((challenge.clone(), now));
                c.view.challenge = Some(challenge);
            }
            CareerRequest::Authenticate {
                challenge,
                signature,
            } => {
                if c.auth.as_ref() == Some(&challenge) {
                    return;
                }
                if !c.challenge.as_ref().is_some_and(|(expected, at)| {
                    expected == &challenge && at.elapsed() < CHALLENGE_TTL
                }) || !verify(
                    &challenge.public_key,
                    &signature,
                    &challenge.signing_bytes(),
                ) {
                    c.view.error = Some("Authentication failed. Request a fresh challenge.".into());
                    return;
                }
                c.view.loading = true;
                if self
                    .tx
                    .as_ref()
                    .unwrap()
                    .try_send(Job::Login { addr, challenge })
                    .is_err()
                {
                    c.view.error = Some("Account service is busy. Retry shortly.".into());
                    c.view.loading = false;
                }
            }
            CareerRequest::Authorized {
                session_nonce,
                sequence,
                action,
                signature,
            } => {
                let Some(auth) = c.auth.as_ref() else {
                    c.view.error = Some("Sign in before using your profile.".into());
                    return;
                };
                if session_nonce != auth.nonce
                    || sequence <= c.sequence
                    || !verify(
                        &auth.public_key,
                        &signature,
                        &authorized_signing_bytes(self.epoch, &session_nonce, sequence, &action),
                    )
                {
                    c.view.error = Some("Invalid or replayed account request.".into());
                    return;
                }
                c.sequence = sequence;
                c.view.error = None;
                if matches!(action, CareerAction::CancelQueue) {
                    self.cancelled.push(addr);
                    return;
                }
                if let CareerAction::Social { request } = action {
                    if request.server_epoch != self.epoch || request.session_id != auth.session_id {
                        c.view.error =
                            Some("Social request belongs to a different session.".into());
                    } else if self.social.len() >= MAX_JOBS {
                        c.view.error = Some("Chat is busy. Retry shortly.".into());
                    } else {
                        self.social.push((addr, request));
                    }
                    return;
                }
                let Some(profile) = c.view.profile.as_ref() else {
                    return;
                };
                let request_id = action_id(&action);
                if self
                    .tx
                    .as_ref()
                    .unwrap()
                    .try_send(Job::Action {
                        addr,
                        nonce: session_nonce,
                        profile_id: profile.profile_id.clone(),
                        action,
                    })
                    .is_err()
                {
                    c.view.error = Some("Account service is busy. Retry shortly.".into());
                    c.view.response_id = request_id;
                }
            }
            _ => {
                c.view.error = Some("Account requests require a valid signature.".into());
            }
        }
    }
    pub fn poll(&mut self) {
        loop {
            let reply = { self.rx.lock().expect("career reply receiver").try_recv() };
            let Ok(reply) = reply else {
                break;
            };
            match reply {
                Reply::Login {
                    addr,
                    challenge,
                    result,
                } => {
                    let Some(c) = self.clients.get_mut(&addr) else {
                        continue;
                    };
                    if !c
                        .challenge
                        .as_ref()
                        .is_some_and(|(expected, _)| *expected == challenge)
                        || c.auth.as_ref() == Some(&challenge)
                    {
                        continue;
                    }
                    c.view.loading = false;
                    match result {
                        Ok(mut view) => {
                            view.auth_nonce = Some(challenge.nonce.clone());
                            c.view = view;
                            c.auth = Some(challenge);
                            c.profile_ready = true;
                            c.sequence = 0;
                        }
                        Err(e) => c.view.error = Some(e),
                    }
                }
                Reply::Action {
                    addr,
                    nonce,
                    result,
                    request_id,
                } => {
                    let Some(c) = self.clients.get_mut(&addr) else {
                        continue;
                    };
                    if c.auth.as_ref().is_none_or(|a| a.nonce != nonce) {
                        continue;
                    }
                    c.view.response_id = request_id;
                    c.view.error = None;
                    c.view.history.clear();
                    c.view.history_loaded = false;
                    c.view.history_next = None;
                    c.view.detail = None;
                    c.view.friends = None;
                    c.view.visited_profile = None;
                    match result {
                        Ok(view) => {
                            if view.profile.is_some() {
                                c.view.profile = view.profile;
                                c.profile_ready = true;
                                c.view.loading = false;
                            }
                            if view.history_loaded {
                                c.view.history = view.history;
                                c.view.history_next = view.history_next;
                                c.view.history_loaded = true;
                            }
                            if view.detail.is_some() {
                                c.view.detail = view.detail;
                            }
                            if view.friends.is_some() {
                                c.view.friends = view.friends;
                            }
                            if view.found_player.is_some() {
                                c.view.found_player = view.found_player;
                            }
                            if view.visited_profile.is_some() {
                                c.view.visited_profile = view.visited_profile;
                            }
                        }
                        Err(e) => c.view.error = Some(e),
                    }
                }
                Reply::Started(id) => {
                    self.starts.insert(id, Ok(()));
                }
                Reply::Rejected(id, error) => {
                    for (addr, c) in &mut self.clients {
                        if let (Some(profile), Some(auth)) = (&c.view.profile, &c.auth) {
                            if self.matches.get(&profile.profile_id) == Some(&id) {
                                c.profile_ready = false;
                                let _ = self.tx.as_ref().unwrap().try_send(Job::Action {
                                    addr: *addr,
                                    nonce: auth.nonce.clone(),
                                    profile_id: profile.profile_id.clone(),
                                    action: CareerAction::Profile {
                                        request_id: 0,
                                        profile_id: profile.profile_id.clone(),
                                    },
                                });
                            }
                        }
                    }
                    self.pending_ids.remove(&id);
                    self.matches.retain(|_, result_id| *result_id != id);
                    self.rejected.insert(id, error);
                }
                Reply::RecordError(id, error) => {
                    if !self.started(&id) {
                        self.starts.insert(id, Err(error.clone()));
                    }
                    for c in self.clients.values_mut() {
                        if c.playing {
                            c.view.error = Some(error.clone());
                        }
                    }
                }
                Reply::Settled { result, profiles } => {
                    for c in self.clients.values_mut() {
                        if let Some(current) = &c.view.profile {
                            if let Some(fresh) =
                                profiles.iter().find(|p| p.profile_id == current.profile_id)
                            {
                                c.view.profile = Some(fresh.clone());
                                c.profile_ready = true;
                                c.view.last_result = Some(result.clone());
                                c.view.error = None;
                            }
                        }
                    }
                    self.pending_ids.remove(&result.result_id);
                    self.matches.retain(|_, id| *id != result.result_id);
                    self.starts.remove(&result.result_id);
                    self.settled.push(result);
                }
            }
        }
        self.clients
            .retain(|_, c| c.touched.elapsed() < Duration::from_secs(120));
        if self.last_presence.elapsed() >= Duration::from_secs(10) {
            self.last_presence = Instant::now();
            let presence = self
                .clients
                .values()
                .filter_map(|c| {
                    c.view.profile.as_ref().map(|p| {
                        (
                            p.profile_id.clone(),
                            c.playing
                                .then(|| self.matches.get(&p.profile_id).cloned())
                                .flatten(),
                        )
                    })
                })
                .collect();
            if let Some(tx) = &self.tx {
                let _ = tx.try_send(Job::Presence(presence));
                for (addr, c) in &self.clients {
                    if !c.profile_ready {
                        if let (Some(profile), Some(auth)) = (&c.view.profile, &c.auth) {
                            let _ = tx.try_send(Job::Action {
                                addr: *addr,
                                nonce: auth.nonce.clone(),
                                profile_id: profile.profile_id.clone(),
                                action: CareerAction::Profile {
                                    request_id: 0,
                                    profile_id: profile.profile_id.clone(),
                                },
                            });
                        }
                    }
                }
            }
        }
    }
}

fn action_id(action: &CareerAction) -> Option<u64> {
    match action {
        CareerAction::History { request_id, .. }
        | CareerAction::Detail { request_id, .. }
        | CareerAction::Friends { request_id }
        | CareerAction::Friend { request_id, .. }
        | CareerAction::Profile { request_id, .. }
        | CareerAction::LookupPlayer { request_id, .. }
        | CareerAction::Rename { request_id, .. } => Some(*request_id),
        CareerAction::CancelQueue => None,
        CareerAction::Social { request } => Some(request.request_id),
    }
}

async fn account_action(
    store: &CareerStore,
    id: &str,
    action: CareerAction,
) -> Result<CareerView, String> {
    let mut view = CareerView::default();
    match action {
        CareerAction::History { before, .. } => {
            let (history, next) = store.history(id, before).await?;
            view.history = history;
            view.history_next = next;
            view.history_loaded = true;
        }
        CareerAction::Detail { result_id, .. } => {
            if result_id.len() > 128 {
                return Err("Invalid match ID.".into());
            }
            view.detail = Some(store.detail(id, &result_id).await?);
        }
        CareerAction::Friends { .. } => view.friends = Some(store.friends(id).await?),
        CareerAction::Friend {
            profile_id, action, ..
        } => view.friends = Some(store.friend_action(id, &profile_id, action).await?),
        CareerAction::Profile { profile_id, .. } => {
            if profile_id == id {
                view.profile = Some(store.profile(id).await?);
            } else {
                view.visited_profile = Some(store.public_profile(id, &profile_id).await?);
            }
        }
        CareerAction::LookupPlayer { handle, .. } => {
            view.found_player = Some(store.lookup_player(&handle).await?);
        }
        CareerAction::Rename { nickname, .. } => {
            view.profile = Some(store.rename(id, &nickname).await?)
        }
        CareerAction::CancelQueue => {}
        CareerAction::Social { .. } => {
            return Err("Social requests are handled by the game server.".into());
        }
    }
    Ok(view)
}

async fn settled_profiles(
    store: &CareerStore,
    result: &MatchResult,
) -> Result<Vec<ProfileSummary>, String> {
    let mut profiles = Vec::new();
    for participant in &result.participants {
        if let Some(id) = &participant.profile_id {
            profiles.push(store.profile(id).await?);
        }
    }
    Ok(profiles)
}

fn spool_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("Invalid result identity.".into());
    }
    Ok(root.join(format!("{id}.json")))
}
fn write_record(root: &Path, record: &PendingRecord) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|_| "Cannot create the career outbox.".to_string())?;
    let path = spool_path(root, &record.result.result_id)?;
    let temp = path.with_extension("tmp");
    let data =
        serde_json::to_vec(record).map_err(|_| "Cannot encode match receipt.".to_string())?;
    let mut file =
        fs::File::create(&temp).map_err(|_| "Cannot write career outbox.".to_string())?;
    file.write_all(&data)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Cannot sync career outbox.".to_string())?;
    fs::rename(temp, path).map_err(|_| "Cannot commit career outbox.".to_string())?;
    fs::File::open(root)
        .and_then(|f| f.sync_all())
        .map_err(|_| "Cannot sync career outbox directory.".to_string())?;
    Ok(())
}

fn worker(url: String, outbox: PathBuf, jobs: Receiver<Job>, replies: SyncSender<Reply>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("career async runtime");
    let mut store: Option<CareerStore> = None;
    let mut pending: BTreeMap<String, PendingRecord> = BTreeMap::new();
    let mut rejected_ids = VecDeque::new();
    if let Ok(entries) = fs::read_dir(&outbox) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "json") {
                match fs::read(entry.path())
                    .ok()
                    .and_then(|v| serde_json::from_slice::<PendingRecord>(&v).ok())
                {
                    Some(mut record) => {
                        if matches!(record.kind, RecordKind::Rejected) {
                            rejected_ids.push_back(record.result.result_id.clone());
                            if rejected_ids.len() > MAX_JOBS * 2 {
                                rejected_ids.pop_front();
                            }
                            continue;
                        }
                        if !matches!(record.kind, RecordKind::Settle) {
                            if record.recovery_allocation.is_none() {
                                record.recovery_allocation = Some(record.result.clone());
                            }
                            record.kind = RecordKind::Settle;
                            record.recovered_live = true;
                            record.result.outcome = MatchOutcome::Interrupted;
                            record.result.winner = None;
                            record.result.rated = false;
                            record.result.unrated_reason = Some("server_interrupted".into());
                            record.result.ended_at_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis()
                                .min(u64::MAX as u128)
                                as u64;
                            record.result.duration_ms = record
                                .result
                                .ended_at_ms
                                .saturating_sub(record.result.started_at_ms);
                        }
                        pending.insert(record.result.result_id.clone(), record);
                    }
                    None => eprintln!(
                        "Career outbox contains an unreadable receipt; preserved for recovery."
                    ),
                }
            }
        }
    }
    let mut last_retry = Instant::now() - Duration::from_secs(10);
    let mut heartbeat = Instant::now();
    let mut connected = true;
    let mut _heartbeat_task = None;
    while connected || !pending.is_empty() {
        if store.is_none() && last_retry.elapsed() >= Duration::from_secs(2) {
            last_retry = Instant::now();
            store = rt.block_on(CareerStore::connect_runtime(&url)).ok();
            if let Some(s) = &store {
                let s = s.clone();
                _heartbeat_task = Some(rt.spawn(async move {
                    let mut timer = tokio::time::interval(Duration::from_secs(10));
                    loop {
                        timer.tick().await;
                        let _ = s.heartbeat().await;
                    }
                }));
            }
        }
        match jobs.recv_timeout(Duration::from_millis(100)) {
            Ok(Job::Record(record)) => {
                let mut record = *record;
                let id = record.result.result_id.clone();
                if rejected_ids.contains(&id) {
                    continue;
                }
                if let Some(old) = pending.get(&id) {
                    if matches!(old.kind, RecordKind::Settle) {
                        continue;
                    }
                    if matches!(old.kind, RecordKind::Checkpoint)
                        && matches!(record.kind, RecordKind::Checkpoint)
                        && record.result.duration_ms < old.result.duration_ms
                    {
                        continue;
                    }
                    record.recovery_allocation = old.recovery_allocation.clone().or_else(|| {
                        matches!(old.kind, RecordKind::Start).then(|| old.result.clone())
                    });
                }
                pending.insert(id, record);
                last_retry = Instant::now() - Duration::from_secs(3);
            }
            Ok(Job::Login { addr, challenge }) => {
                let result = match &store {
                    Some(store) => rt.block_on(async {
                        let profile = store
                            .authenticate(&challenge.public_key, &challenge.nickname)
                            .await?;
                        let (history, _) = store.history(&profile.profile_id, None).await?;
                        let last_result = match history.first() {
                            Some(last) => store
                                .detail(&profile.profile_id, &last.result_id)
                                .await
                                .ok(),
                            None => None,
                        };
                        Ok(CareerView {
                            profile: Some(profile),
                            last_result,
                            ..Default::default()
                        })
                    }),
                    None => Err("Profile storage is unavailable. Retry shortly.".into()),
                };
                let _ = replies.try_send(Reply::Login {
                    addr,
                    challenge,
                    result,
                });
            }
            Ok(Job::Action {
                addr,
                nonce,
                profile_id,
                action,
            }) => {
                let request_id = action_id(&action);
                let result = match &store {
                    Some(s) => rt.block_on(account_action(s, &profile_id, action)),
                    None => Err("Profile storage is unavailable. Retry shortly.".into()),
                };
                let _ = replies.try_send(Reply::Action {
                    addr,
                    nonce,
                    result,
                    request_id,
                });
            }
            Ok(Job::Presence(presence)) => {
                if let Some(s) = &store {
                    let _ = rt.block_on(s.touch_presences(&presence));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => connected = false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if last_retry.elapsed() >= Duration::from_secs(2) || !connected {
            last_retry = Instant::now();
            let ids: Vec<_> = pending.keys().cloned().collect();
            for id in ids {
                let record = pending.get(&id).unwrap();
                if matches!(record.kind, RecordKind::Rejected) {
                    let error = record
                        .result
                        .unrated_reason
                        .clone()
                        .unwrap_or_else(|| "Clear the queue and retry this roster.".into());
                    if replies.try_send(Reply::Rejected(id.clone(), error)).is_ok() {
                        if let Ok(path) = spool_path(&outbox, &id) {
                            let _ = fs::remove_file(path);
                        }
                        pending.remove(&id);
                    }
                    continue;
                }
                let result = write_record(&outbox, record).and_then(|_| match &store {
                    None => Err("Match result is pending: PostgreSQL is unavailable.".into()),
                    Some(s) => rt.block_on(async {
                        if record.recovered_live {
                            if let Some(result) = s.settled_result(&record.result.result_id).await?
                            {
                                let profiles = settled_profiles(s, &result).await?;
                                return Ok(Some((result, profiles)));
                            }
                        }
                        if let Some(allocation) = &record.recovery_allocation {
                            s.ensure_allocation(allocation.clone()).await?;
                        }
                        match record.kind {
                            RecordKind::Start => {
                                s.start(record.result.clone()).await?;
                                Ok(None)
                            }
                            RecordKind::Checkpoint => {
                                s.checkpoint(record.result.clone()).await?;
                                Ok(None)
                            }
                            RecordKind::Rejected => Ok(None),
                            RecordKind::Settle => {
                                let result = s.settle(record.result.clone()).await?;
                                let profiles = settled_profiles(s, &result).await?;
                                Ok(Some((result, profiles)))
                            }
                        }
                    }),
                });
                match result {
                    Ok(result) => {
                        if matches!(record.kind, RecordKind::Start) {
                            if replies.try_send(Reply::Started(id.clone())).is_err() {
                                continue;
                            }
                        }
                        if let Some((result, profiles)) = result {
                            if replies
                                .try_send(Reply::Settled { result, profiles })
                                .is_err()
                            {
                                continue;
                            }
                        }
                        if let Ok(path) = spool_path(&outbox, &id) {
                            let _ = fs::remove_file(path);
                        }
                        pending.remove(&id);
                    }
                    Err(error) => {
                        if matches!(record.kind, RecordKind::Start)
                            && (error
                                == "A conflicting career identity or active assignment already exists."
                                || error
                                    == "Queued ratings changed. Clear the queue and try again.")
                        {
                            let message = if error.starts_with("Queued ratings") {
                                error.clone()
                            } else {
                                "An account in this roster already has an active match. Clear the queue and retry after that match is saved.".into()
                            };
                            let mut rejected = PendingRecord {
                                kind: RecordKind::Rejected,
                                result: record.result.clone(),
                                recovery_allocation: None,
                                recovered_live: false,
                            };
                            rejected.result.unrated_reason = Some(message.clone());
                            // A durable tombstone must precede rejection: an undeletable
                            // old Start must never allocate a canceled roster on restart.
                            if write_record(&outbox, &rejected).is_ok() {
                                rejected_ids.push_back(id.clone());
                                if rejected_ids.len() > MAX_JOBS * 2 {
                                    rejected_ids.pop_front();
                                }
                                pending.insert(id.clone(), rejected);
                                if replies
                                    .try_send(Reply::Rejected(id.clone(), message))
                                    .is_ok()
                                {
                                    if let Ok(path) = spool_path(&outbox, &id) {
                                        let _ = fs::remove_file(path);
                                    }
                                    pending.remove(&id);
                                }
                                continue;
                            }
                        }
                        let _ = replies.try_send(Reply::RecordError(id, error));
                    }
                }
            }
        }
        if heartbeat.elapsed() >= Duration::from_secs(10) {
            heartbeat = Instant::now();
            if let Some(s) = &store {
                let _ = rt.block_on(s.heartbeat());
                // Local terminal outbox must be adopted before an expiry sweep.
                for id in pending.keys() {
                    let _ = rt.block_on(s.adopt_expired(id));
                }
                if pending.is_empty() {
                    if let Ok(results) = rt.block_on(s.recover_expired()) {
                        for result in results {
                            if let Ok(profiles) = rt.block_on(settled_profiles(s, &result)) {
                                if replies
                                    .try_send(Reply::Settled { result, profiles })
                                    .is_err()
                                {
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }
        rt.block_on(async {
            tokio::task::yield_now().await;
        });
        if !connected {
            break;
        } // Durable files remain for the next process.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn signatures_bind_domain_nonce_epoch_sequence_and_action() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let public = hex(key.verifying_key().as_bytes());
        let action = CareerAction::Friends { request_id: 1 };
        let bytes = authorized_signing_bytes(1, "nonce", 1, &action);
        let signature = hex(&key.sign(&bytes).to_bytes());
        assert!(verify(&public, &signature, &bytes));
        for bad in [
            authorized_signing_bytes(2, "nonce", 1, &action),
            authorized_signing_bytes(1, "other", 1, &action),
            authorized_signing_bytes(1, "nonce", 2, &action),
            authorized_signing_bytes(1, "nonce", 1, &CareerAction::CancelQueue),
        ] {
            assert!(!verify(&public, &signature, &bad));
        }
        assert!(!verify(&public, &"00".repeat(64), &bytes));
    }
    #[test]
    fn receipt_paths_cannot_escape_outbox() {
        for id in ["", "../secret", "/tmp/key", "a/b", "a.b"] {
            assert!(spool_path(Path::new("outbox"), id).is_err());
        }
        assert!(spool_path(Path::new("outbox"), "match-abc123").is_ok());
    }
    #[test]
    fn forged_and_replayed_account_actions_cannot_cancel_queue() {
        let addr = "127.0.0.1:30001".parse().unwrap();
        let mut backend = CareerBackend::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Player".into()),
            "session",
        );
        let action = CareerAction::CancelQueue;
        let nonce = "b".repeat(64);
        let key = SigningKey::from_bytes(&[7; 32]);
        let signature = hex(&key
            .sign(&authorized_signing_bytes(9, &nonce, 1, &action))
            .to_bytes());
        backend.handle(addr, CareerRequest::CancelQueue);
        assert!(backend.take_cancelled().is_empty());
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(
            addr,
            CareerRequest::Authorized {
                session_nonce: nonce.clone(),
                sequence: 99,
                action: action.clone(),
                signature: "00".repeat(64),
            },
        );
        assert!(backend.take_cancelled().is_empty());
        assert_eq!(backend.clients[&addr].sequence, 0);
        let valid = CareerRequest::Authorized {
            session_nonce: nonce,
            sequence: 1,
            action,
            signature,
        };
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, valid.clone());
        assert_eq!(backend.take_cancelled(), vec![addr]);
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, valid);
        assert!(backend.take_cancelled().is_empty());
    }

    #[test]
    fn social_signatures_bind_message_and_session_without_sending_sql_jobs() {
        let addr = "127.0.0.1:30003".parse().unwrap();
        let mut backend = CareerBackend::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Player".into()),
            "session",
        );
        let key = SigningKey::from_bytes(&[7; 32]);
        let nonce = "b".repeat(64);
        let request = shared::social::SocialRequest {
            request_id: 1,
            server_epoch: 9,
            match_id: 1,
            session_id: "session".into(),
            command: shared::social::SocialCommand::Chat {
                channel: shared::social::SocialChannel::Team,
                text: "Hello".into(),
            },
        };
        let action = CareerAction::Social {
            request: request.clone(),
        };
        let sign = |action: CareerAction, sequence| {
            let signature = hex(&key
                .sign(&authorized_signing_bytes(9, &nonce, sequence, &action))
                .to_bytes());
            CareerRequest::Authorized {
                session_nonce: nonce.clone(),
                sequence,
                action,
                signature,
            }
        };
        backend.handle(
            addr,
            CareerRequest::Social {
                request: request.clone(),
            },
        );
        assert!(backend.take_social().is_empty());
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        let valid = sign(action.clone(), 1);
        let mut forged = valid.clone();
        if let CareerRequest::Authorized {
            action: CareerAction::Social { request },
            ..
        } = &mut forged
        {
            request.command = shared::social::SocialCommand::Reaction {
                reaction_id: "heart".into(),
            };
        }
        backend.handle(addr, forged);
        assert!(backend.take_social().is_empty());
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, valid.clone());
        assert_eq!(backend.take_social(), vec![(addr, request.clone())]);
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, valid);
        assert!(backend.take_social().is_empty());
        let mut wrong = request;
        wrong.session_id = "someone-else".into();
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, sign(CareerAction::Social { request: wrong }, 2));
        assert!(backend.take_social().is_empty());
        assert!(
            backend
                ._test_jobs
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .try_recv()
                .is_err()
        );
    }

    #[test]
    fn late_duplicate_login_ack_cannot_reset_sequence() {
        let addr = "127.0.0.1:30002".parse().unwrap();
        let mut backend = CareerBackend::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Player".into()),
            "session",
        );
        let (tx, rx) = mpsc::sync_channel(2);
        backend.rx = Mutex::new(rx);
        let c = backend.clients.get_mut(&addr).unwrap();
        let challenge = c.auth.clone().unwrap();
        c.challenge = Some((challenge.clone(), Instant::now()));
        c.sequence = 42;
        tx.send(Reply::Login {
            addr,
            challenge,
            result: Ok(c.view.clone()),
        })
        .unwrap();
        backend.poll();
        assert_eq!(backend.clients[&addr].sequence, 42);
    }

    #[test]
    #[ignore = "requires isolated OMOBA_TEST_DATABASE_URL; exercises a real PostgreSQL worker restart"]
    fn postgres_outbox_restart_restores_terminal_and_interrupts_live_checkpoint() {
        let url =
            std::env::var("OMOBA_TEST_DATABASE_URL").expect("OMOBA_TEST_DATABASE_URL is required");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let store = rt.block_on(CareerStore::connect(&url)).unwrap();
        let pool = rt.block_on(sqlx::PgPool::connect(&url)).unwrap();
        let directory =
            std::env::temp_dir().join(format!("omoba-outbox-test-{}", random_id::<16>()));
        let mut expected = Vec::new();
        for terminal in [true, false] {
            let profile = rt
                .block_on(store.authenticate(&random_id::<32>(), "Restart tester"))
                .unwrap();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let mut result = MatchResult {
                result_id: random_id::<16>(),
                server_epoch: now,
                match_id: if terminal { 1 } else { 2 },
                started_at_ms: now - 1000,
                ended_at_ms: 0,
                duration_ms: 0,
                map_profile: "test".into(),
                ruleset: "test-unranked".into(),
                outcome: MatchOutcome::Interrupted,
                winner: None,
                rated: false,
                unrated_reason: Some("test".into()),
                saved: false,
                participants: vec![ParticipantResult {
                    is_bot: false,
                    player_id: 1,
                    profile_id: Some(profile.profile_id.clone()),
                    nickname: profile.nickname,
                    team: shared::map::Team::Green,
                    hero_class: shared::HeroClass::Warrior,
                    character: "test".into(),
                    avatar: None,
                    sprite_character: None,
                    stats: MatchStats::default(),
                    disconnected: false,
                    rating: None,
                    progression_xp_gained: 0,
                }],
            };
            rt.block_on(store.start(result.clone())).unwrap();
            result.participants[0].stats.damage_to_heroes = 125.0;
            result.ended_at_ms = now;
            result.duration_ms = 1000;
            if terminal {
                result.outcome = MatchOutcome::Completed;
                result.winner = Some(shared::map::Team::Green);
            } else {
                rt.block_on(store.checkpoint(result.clone())).unwrap();
            }
            write_record(
                &directory,
                &PendingRecord {
                    kind: if terminal {
                        RecordKind::Settle
                    } else {
                        RecordKind::Checkpoint
                    },
                    result: result.clone(),
                    recovery_allocation: None,
                    recovered_live: false,
                },
            )
            .unwrap();
            rt.block_on(sqlx::query("UPDATE career_matches SET lease_until=clock_timestamp()-interval '1 second' WHERE result_id=$1").bind(&result.result_id).execute(&pool)).unwrap();
            expected.push((result.result_id, profile.profile_id, terminal));
        }
        let mut backend = CareerBackend::test_with_database(100, url.clone(), directory.clone());
        let deadline = Instant::now() + Duration::from_secs(35);
        let mut settled = Vec::new();
        while Instant::now() < deadline && settled.len() < 2 {
            backend.poll();
            settled.extend(backend.take_settled());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(settled.len(), 2, "worker did not recover both receipts");
        for (id, profile, terminal) in expected {
            let saved = settled.iter().find(|r| r.result_id == id).unwrap();
            assert!(saved.saved);
            assert_eq!(saved.participants[0].stats.damage_to_heroes, 125.0);
            assert_eq!(
                saved.outcome,
                if terminal {
                    MatchOutcome::Completed
                } else {
                    MatchOutcome::Interrupted
                }
            );
            assert_eq!(
                rt.block_on(store.profile(&profile)).unwrap().matches_played,
                1
            );
            assert!(!spool_path(&directory, &id).unwrap().exists());
        }
        drop(backend);
        rt.block_on(pool.close());
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
#[path = "career_backend_resilience_tests.rs"]
mod resilience_tests;
