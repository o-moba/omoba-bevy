//! Signed account boundary and bounded off-tick PostgreSQL worker.
//!
//! `CareerBackend<L>` is the game-thread state machine (challenges, signed
//! sessions, replay protection, pending records) behind the `CareerPort`
//! trait; it exchanges `Job`s and `Reply`s with a `JobLink`. `WorkerLink`
//! is the production link (a bounded channel pair to the `worker` thread,
//! which owns every SQL and outbox operation); `MemoryLink` is the test link
//! (a queue on the same thread, acknowledged by hand or immediately), so
//! `MemoryCareer` runs the identical account logic without a database.
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, VerifyingKey};
use omoba_career_store::career_store::CareerStore;
use serde::{Deserialize, Serialize};
use shared::career::*;
use shared::moderation::{Sanction, SanctionKind};

use crate::admission::{Limits, SourceIp};
use crate::career_port::CareerPort;

#[path = "ready_consent.rs"]
pub(crate) mod consent;
#[path = "queue_dodge_outbox.rs"]
pub(crate) mod queue_dodges;

/// Career clients one lobby keeps (its player ceiling together with
/// `public_transport::MAX_ENDPOINTS`); a newcomer beyond it gets `server_full`.
pub(crate) const MAX_CLIENTS: usize = 2048;
/// The English fallback of [`shared::career::error_code::SERVER_FULL`].
const SERVER_FULL_TEXT: &str = "The server is full right now. Signing in again shortly.";
/// A client of an IP at its limit gives way to a newcomer from that IP once
/// it was idle this long (a reconnect leaves its old endpoint behind).
const STALE_CLIENT: Duration = Duration::from_secs(30);
/// One Login job per endpoint and challenge at a time; a lost reply is
/// retried after this long.
const LOGIN_IN_FLIGHT: Duration = Duration::from_secs(10);
/// A Login or account action waits this long for the database to connect
/// before it is answered "unavailable".
const STORE_WAIT: Duration = Duration::from_secs(10);
/// How often the lobby runs the retention job, and its batch.
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
const RETENTION_BATCH: u32 = 200;
const MAX_JOBS: usize = 256;
const COSMETIC_CACHE_TTL: Duration = Duration::from_secs(30);
const ACCESS_REFRESH_AFTER: Duration = Duration::from_secs(10);
const ACCESS_REFRESH_STEP: Duration = Duration::from_millis(100);
const ACCESS_REFRESH_RETRY: Duration = Duration::from_secs(5);
const ACCESS_REFRESH_BATCH: usize = 16;
const PRESENCE_BATCH_SIZE: usize = 512;
const CHALLENGE_TTL: Duration = Duration::from_secs(30);
/// How often the worker reads the database-wide chat switch (LR-15).
const CHAT_SWITCH_POLL: Duration = Duration::from_secs(5);

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
    queue_sequence: Option<u64>,
    last_request: Option<Instant>,
    touched: Instant,
    playing: bool,
    profile_ready: bool,
    supporter_checked: Option<Instant>,
    access_refresh_sent: Option<Instant>,
    key_checked: Option<Instant>,
    /// When a Login job for the current challenge went to the worker.
    login_sent: Option<Instant>,
}
impl Default for Client {
    fn default() -> Self {
        Self {
            view: CareerView::default(),
            challenge: None,
            auth: None,
            sequence: 0,
            queue_sequence: None,
            last_request: None,
            touched: Instant::now(),
            playing: false,
            profile_ready: false,
            supporter_checked: None,
            access_refresh_sent: None,
            key_checked: None,
            login_sent: None,
        }
    }
}

pub(crate) enum Job {
    QueueAccess {
        request: crate::career_port::MatchRequest,
        nonce: String,
        sequence: u64,
        public_key: String,
        profile_id: String,
    },
    RefreshAccess {
        addr: SocketAddr,
        nonce: String,
        public_key: String,
        profile_id: String,
    },
    Login {
        addr: SocketAddr,
        challenge: AuthChallenge,
    },
    Action {
        addr: SocketAddr,
        nonce: String,
        profile_id: String,
        public_key: String,
        action: CareerAction,
    },
    Record(Box<PendingRecord>),
    QueueDodge(queue_dodges::Event),
    Ready(consent::Work),
    Flush(u64),
    Presence(Vec<(String, Option<String>)>),
}
pub(crate) enum Reply {
    QueueAccess {
        request: crate::career_port::MatchRequest,
        nonce: String,
        sequence: u64,
        access: Result<Option<omoba_career_store::career_store::moderation::Access>, String>,
    },
    RecoveryChecked,
    QueueDodgeCommitted(queue_dodges::Event),
    Ready(consent::Completion),
    Flushed(u64, bool),
    RefreshAccess {
        profile: Option<ProfileSummary>,
        completed_public_matches: Option<u32>,
        addr: SocketAddr,
        nonce: String,
        key_active: Option<bool>,
        supporter: Option<shared::supporter::SupporterStatus>,
        /// The profile's current name and active sanctions (LR-15); `None`
        /// when the database could not be read.
        nickname: Option<String>,
        sanctions: Option<Vec<Sanction>>,
        play_cooldown: Option<Option<shared::career::PlayCooldown>>,
    },
    Login {
        addr: SocketAddr,
        challenge: AuthChallenge,
        result: Result<CareerView, LoginRefusal>,
    },
    /// The database-wide chat switch (LR-15).
    ChatSwitch(bool),
    Action {
        addr: SocketAddr,
        nonce: String,
        result: Result<CareerView, String>,
        request_id: Option<u64>,
    },
    Started(String),
    GameplayStarted(String),
    RecordError(String, String),
    Rejected(String, String),
    Settled {
        result: MatchResult,
        profiles: Vec<ProfileSummary>,
    },
}

/// A refused sign-in: the English text, and the sanctions that refused it
/// (an account ban), so the client can say until when.
pub(crate) struct LoginRefusal {
    pub(crate) error: String,
    pub(crate) sanctions: Vec<Sanction>,
}
impl From<String> for LoginRefusal {
    fn from(error: String) -> Self {
        Self {
            error,
            sanctions: Vec::new(),
        }
    }
}
impl From<&str> for LoginRefusal {
    fn from(error: &str) -> Self {
        error.to_owned().into()
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum RecordKind {
    Start,
    Checkpoint,
    Settle,
    Rejected,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PendingRecord {
    pub(crate) kind: RecordKind,
    pub(crate) result: MatchResult,
    #[serde(default)]
    pub(crate) recovery_allocation: Option<MatchResult>,
    #[serde(default)]
    pub(crate) recovered_live: bool,
}

#[derive(Clone, Copy)]
enum RecoveryScope {
    Global,
    Disabled,
    Epoch(u64),
}

/// The exchange between the game-thread state machine and whatever performs
/// the store I/O. `try_send` never blocks and returns whether the job was
/// accepted; a saturated or disabled link refuses it (the caller reports
/// "busy" or retries later).
pub(crate) trait JobLink {
    fn enabled(&self) -> bool;
    fn durable_ready(&self) -> bool {
        self.enabled()
    }
    fn try_send(&mut self, job: Job) -> bool;
    fn try_recv(&mut self) -> Option<Reply>;
}

/// Bounded channels to the `worker` thread; `tx` is `None` when no database
/// URL is configured, which is the disabled (guest-only) store.
pub(crate) struct WorkerLink {
    tx: Option<SyncSender<Job>>,
    rx: Mutex<Receiver<Reply>>,
}
impl JobLink for WorkerLink {
    fn enabled(&self) -> bool {
        self.tx.is_some()
    }
    fn try_send(&mut self, job: Job) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.try_send(job).is_ok())
    }
    fn try_recv(&mut self) -> Option<Reply> {
        self.rx
            .lock()
            .expect("career reply receiver")
            .try_recv()
            .ok()
    }
}

/// Same-thread link for tests: jobs are captured (bounded like the worker
/// channel) and replies are whatever the test or the auto-acknowledger
/// queued. With `auto_ack` every job is answered at once the way a healthy
/// worker would (login creates a profile, a start is started, a settle is
/// saved, an access refresh keeps the key active); without it nothing is
/// acknowledged until a `test_ack_*` hook does it, which is what the old
/// `test_backend()` fixture did. A disabled link is the guest-only server
/// without a database URL.
#[cfg(test)]
pub(crate) struct MemoryLink {
    pub(crate) jobs: VecDeque<Job>,
    pub(crate) replies: VecDeque<Reply>,
    pub(crate) enabled: bool,
    pub(crate) auto_ack: bool,
    /// Profiles by public key, created on first login.
    pub(crate) profiles: HashMap<String, ProfileSummary>,
}
#[cfg(test)]
impl MemoryLink {
    fn new(enabled: bool, auto_ack: bool) -> Self {
        Self {
            jobs: VecDeque::new(),
            replies: VecDeque::new(),
            enabled,
            auto_ack,
            profiles: HashMap::new(),
        }
    }
    fn profile_view(&self, profile: ProfileSummary) -> CareerView {
        CareerView {
            supporter: Some(Default::default()),
            profile: Some(profile),
            ..Default::default()
        }
    }
    fn acknowledge(&mut self, job: Job) {
        match job {
            Job::QueueAccess {
                request,
                nonce,
                sequence,
                ..
            } => {
                self.replies.push_back(Reply::QueueAccess {
                    request,
                    nonce,
                    sequence,
                    access: Ok(None),
                });
            }
            Job::Login { addr, challenge } => {
                let profile = self
                    .profiles
                    .entry(challenge.public_key.clone())
                    .or_insert_with(|| {
                        ProfileSummary::new(
                            hex(&decode::<32>(&challenge.public_key).unwrap_or([0; 32])),
                            challenge.nickname.clone(),
                        )
                    })
                    .clone();
                let result = Ok(self.profile_view(profile));
                self.replies.push_back(Reply::Login {
                    addr,
                    challenge,
                    result,
                });
            }
            Job::RefreshAccess { addr, nonce, .. } => {
                self.replies.push_back(Reply::RefreshAccess {
                    profile: None,
                    completed_public_matches: None,
                    addr,
                    nonce,
                    key_active: Some(true),
                    supporter: Some(Default::default()),
                    nickname: None,
                    sanctions: None,
                    play_cooldown: None,
                });
            }
            Job::Action {
                addr,
                nonce,
                public_key,
                action,
                ..
            } => {
                let request_id = action_id(&action);
                let result = match (&action, self.profiles.get(&public_key)) {
                    (
                        CareerAction::Profile { .. } | CareerAction::SupporterStatus { .. },
                        Some(profile),
                    ) => Ok(self.profile_view(profile.clone())),
                    _ => Err("The in-memory career store does not serve this request.".into()),
                };
                self.replies.push_back(Reply::Action {
                    addr,
                    nonce,
                    result,
                    request_id,
                });
            }
            Job::Record(record) => match record.kind {
                RecordKind::Start => {
                    self.replies
                        .push_back(Reply::Started(record.result.result_id.clone()));
                }
                RecordKind::Settle => {
                    let mut result = record.result;
                    result.saved = true;
                    let profiles = result
                        .participants
                        .iter()
                        .filter_map(|p| {
                            let id = p.profile_id.as_ref()?;
                            self.profiles
                                .values()
                                .find(|profile| &profile.profile_id == id)
                                .cloned()
                        })
                        .collect();
                    self.replies.push_back(Reply::Settled { result, profiles });
                }
                RecordKind::Checkpoint => {
                    if record.result.gameplay_started {
                        self.replies
                            .push_back(Reply::GameplayStarted(record.result.result_id));
                    }
                }
                RecordKind::Rejected => {}
            },
            Job::QueueDodge(event) => self.replies.push_back(Reply::QueueDodgeCommitted(event)),
            Job::Ready(work) => self.replies.push_back(Reply::Ready(consent::Completion {
                work,
                result: Err(STORE_UNAVAILABLE.into()),
            })),
            Job::Flush(generation) => self.replies.push_back(Reply::Flushed(generation, true)),
            Job::Presence(_) => {}
        }
    }
}
#[cfg(test)]
impl JobLink for MemoryLink {
    fn durable_ready(&self) -> bool {
        false
    }
    fn enabled(&self) -> bool {
        self.enabled
    }
    fn try_send(&mut self, job: Job) -> bool {
        if !self.enabled {
            return false;
        }
        if self.auto_ack {
            self.acknowledge(job);
            return true;
        }
        if self.jobs.len() >= MAX_JOBS {
            return false;
        }
        self.jobs.push_back(job);
        true
    }
    fn try_recv(&mut self) -> Option<Reply> {
        self.replies.pop_front()
    }
}

/// The signed-account state machine over a job link. `CareerBackend` (the
/// default link) is the production store; `MemoryCareer` is the test store.
pub(crate) struct CareerBackend<L: JobLink = WorkerLink> {
    epoch: u64,
    recovery_checked: Option<Instant>,
    pub(crate) link: L,
    clients: HashMap<SocketAddr, Client>,
    starts: HashMap<String, Result<(), String>>,
    gameplay_starts: HashSet<String>,
    settled: Vec<MatchResult>,
    cancelled: Vec<SocketAddr>,
    social: Vec<(SocketAddr, shared::social::SocialRequest)>,
    match_requests: Vec<crate::career_port::MatchRequest>,
    ready_requests: Vec<crate::career_port::ReadyRequest>,
    last_presence: Instant,
    last_access_refresh: Option<Instant>,
    matches: HashMap<String, String>,
    pending_ids: HashSet<String>,
    queue_dodges: HashMap<String, queue_dodges::Event>,
    queue_dodge_acks: HashMap<String, queue_dodges::Event>,
    ready_jobs: HashMap<(String, u64), consent::Work>,
    ready_completions: Vec<consent::Completion>,
    match_refusals: Vec<(crate::career_port::MatchRequest, String)>,
    record_generation: u64,
    flush_requested: Option<u64>,
    flushed_generation: Option<u64>,
    rejected: HashMap<String, String>,
    limits: Limits,
    /// The database-wide chat switch as last read; `None` before the first
    /// read or without a database (LR-15).
    chat_switch: Option<bool>,
}
#[cfg(test)]
pub(crate) type MemoryCareer = CareerBackend<MemoryLink>;

impl<L: JobLink> CareerBackend<L> {
    /// Pace refresh across the bounded link. Oldest due clients get the next
    /// slot, including after congestion; submission alone never extends auth.
    fn refresh_access(&mut self, now: Instant) {
        if !self.link.enabled()
            || self
                .last_access_refresh
                .is_some_and(|at| now.saturating_duration_since(at) < ACCESS_REFRESH_STEP)
        {
            return;
        }
        self.last_access_refresh = Some(now);
        let mut due: Vec<_> = self
            .clients
            .iter()
            .filter_map(|(&addr, c)| {
                c.auth.as_ref()?;
                c.view.profile.as_ref()?;
                let checked = match (c.key_checked, c.supporter_checked) {
                    (Some(key), Some(supporter)) => key.min(supporter),
                    _ => now - ACCESS_REFRESH_AFTER,
                };
                if now.saturating_duration_since(checked) < ACCESS_REFRESH_AFTER
                    || c.access_refresh_sent
                        .is_some_and(|at| now.saturating_duration_since(at) < ACCESS_REFRESH_RETRY)
                {
                    return None;
                }
                Some((c.access_refresh_sent.unwrap_or(checked), checked, addr))
            })
            .collect();
        due.sort_unstable();
        for (_, _, addr) in due.into_iter().take(ACCESS_REFRESH_BATCH) {
            let c = self.clients.get_mut(&addr).unwrap();
            let auth = c.auth.as_ref().unwrap();
            let profile = c.view.profile.as_ref().unwrap();
            if !self.link.try_send(Job::RefreshAccess {
                addr,
                nonce: auth.nonce.clone(),
                public_key: auth.public_key.clone(),
                profile_id: profile.profile_id.clone(),
            }) {
                break;
            }
            c.access_refresh_sent = Some(now);
        }
    }
    fn over(epoch: u64, link: L) -> Self {
        Self {
            epoch,
            recovery_checked: None,
            link,
            clients: HashMap::new(),
            starts: HashMap::new(),
            gameplay_starts: HashSet::new(),
            settled: Vec::new(),
            cancelled: Vec::new(),
            social: Vec::new(),
            match_requests: Vec::new(),
            ready_requests: Vec::new(),
            last_presence: Instant::now(),
            last_access_refresh: None,
            matches: HashMap::new(),
            pending_ids: HashSet::new(),
            queue_dodges: HashMap::new(),
            queue_dodge_acks: HashMap::new(),
            ready_jobs: HashMap::new(),
            ready_completions: Vec::new(),
            match_refusals: Vec::new(),
            record_generation: 0,
            flush_requested: None,
            flushed_generation: None,
            rejected: HashMap::new(),
            limits: Limits::default(),
            chat_switch: None,
        }
    }
}
#[cfg(test)]
impl MemoryCareer {
    /// Enabled and acknowledging every job at once, without a thread.
    pub fn immediate(epoch: u64) -> Self {
        Self::over(epoch, MemoryLink::new(true, true))
    }
    /// Enabled but never acknowledging on its own: allocation stays pending
    /// until a `test_ack_*` hook answers, and every job is kept for
    /// inspection in `link.jobs`.
    pub fn test_backend(epoch: u64) -> Self {
        Self::over(epoch, MemoryLink::new(true, false))
    }
    /// No storage, as a server without `OMOBA_DATABASE_URL`: guests only,
    /// records refused, the view says so.
    pub fn disabled(epoch: u64) -> Self {
        Self::over(epoch, MemoryLink::new(false, false))
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}
impl CareerBackend {
    pub fn new(epoch: u64, limits: Limits) -> Self {
        let url = std::env::var("OMOBA_DATABASE_URL")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let outbox = std::env::var_os("OMOBA_CAREER_OUTBOX")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".omoba/career-outbox"));
        let scope = match std::env::var("OMOBA_SERVER_ROLE").ok().as_deref() {
            Some("lobby") => RecoveryScope::Disabled,
            Some("match") if std::env::var("OMOBA_MATCH_RECOVERY").as_deref() == Ok("1") => {
                std::env::var_os("OMOBA_MATCH_ALLOCATION")
                    .map(PathBuf::from)
                    .and_then(|p| p.parent().map(|p| p.join("status.json")))
                    .and_then(|p| fs::read(p).ok())
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .and_then(|v| v.get("server_epoch").and_then(|v| v.as_u64()))
                    .filter(|&v| v != 0)
                    .map_or(RecoveryScope::Disabled, RecoveryScope::Epoch)
            }
            Some("match") => RecoveryScope::Epoch(epoch),
            _ => RecoveryScope::Global,
        };
        Self::with_recovery(epoch, url, outbox, scope, limits)
    }
    #[cfg(test)]
    fn with_config(epoch: u64, url: Option<String>, outbox: PathBuf) -> Self {
        let limits = Limits {
            new_profiles: Admission::for_test().new_profiles,
            ..Limits::default()
        };
        Self::with_recovery(epoch, url, outbox, RecoveryScope::Global, limits)
    }
    fn with_recovery(
        epoch: u64,
        url: Option<String>,
        outbox: PathBuf,
        scope: RecoveryScope,
        limits: Limits,
    ) -> Self {
        let (reply_tx, rx) = mpsc::sync_channel(MAX_JOBS * 2);
        // Match workers are many short processes: only the lobby (and a
        // standalone server) runs the retention job.
        let admission = Admission {
            new_profiles: limits.new_profiles,
            retention_days: limits
                .retention_days
                .filter(|_| !matches!(scope, RecoveryScope::Epoch(_))),
        };
        let tx = url.map(|url| {
            let (tx, jobs) = mpsc::sync_channel(MAX_JOBS);
            std::thread::Builder::new()
                .name("omoba-career".into())
                .spawn(move || worker(url, outbox, jobs, reply_tx, scope, admission))
                .expect("career worker starts");
            tx
        });
        let mut backend = Self::over(
            epoch,
            WorkerLink {
                tx,
                rx: Mutex::new(rx),
            },
        );
        backend.limits = limits;
        backend
    }
    #[cfg(test)]
    pub fn test_with_database(epoch: u64, url: String, outbox: PathBuf) -> Self {
        Self::with_config(epoch, Some(url), outbox)
    }
}

impl<L: JobLink> CareerBackend<L> {
    /// Whether a new client at `addr` fits: at most [`MAX_CLIENTS`] in all and
    /// `endpoints_per_ip` per IP (LR-16). At its IP's limit, that IP's stalest
    /// client gives way once idle for [`STALE_CLIENT`] and not in a match: a
    /// reconnecting device leaves its old endpoint behind.
    fn make_room(&mut self, addr: SocketAddr) -> bool {
        let ip = SourceIp::of(addr);
        let mut same_ip = self
            .clients
            .iter()
            .filter(|(other, _)| SourceIp::of(**other) == ip)
            .map(|(other, client)| (*other, client.touched, client.playing))
            .collect::<Vec<_>>();
        if same_ip.len() >= self.limits.endpoints_per_ip {
            same_ip.retain(|(_, touched, playing)| !playing && touched.elapsed() >= STALE_CLIENT);
            let Some((stalest, _, _)) = same_ip.into_iter().min_by_key(|(_, touched, _)| *touched)
            else {
                return false;
            };
            self.clients.remove(&stalest);
        }
        self.clients.len() < MAX_CLIENTS
    }

    fn record(&mut self, kind: RecordKind, result: MatchResult) -> bool {
        if self.rejected.contains_key(&result.result_id) {
            return false;
        }
        if !self.pending_ids.contains(&result.result_id) && self.pending_ids.len() >= 32 {
            return false;
        }
        let accepted = self.link.try_send(Job::Record(Box::new(PendingRecord {
            kind,
            result: result.clone(),
            recovery_allocation: None,
            recovered_live: false,
        })));
        if accepted {
            self.record_generation = self.record_generation.saturating_add(1);
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
}

impl<L: JobLink> CareerPort for CareerBackend<L> {
    #[cfg(test)]
    fn test_authenticated(&mut self, addr: SocketAddr, profile: ProfileSummary, session_id: &str) {
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
                key_checked: Some(Instant::now()),
                ..Default::default()
            },
        );
    }
    #[cfg(test)]
    fn test_complete_ready(&mut self, completion: consent::Completion) {
        let key = (completion.work.check_id.clone(), completion.work.sequence);
        if self.ready_jobs.get(&key) == Some(&completion.work) {
            self.ready_jobs.remove(&key);
            self.ready_completions.push(completion);
        }
    }
    #[cfg(test)]
    fn test_ack_start(&mut self, id: &str) {
        self.starts.insert(id.into(), Ok(()));
    }
    #[cfg(test)]
    fn test_ack_gameplay(&mut self, id: &str) {
        self.gameplay_starts.insert(id.to_owned());
    }

    #[cfg(test)]
    fn test_ack_settle(&mut self, result: MatchResult) {
        self.settled.push(result);
    }
    #[cfg(test)]
    fn test_reject_start(&mut self, id: &str, error: &str) {
        self.rejected.insert(id.into(), error.into());
    }
    #[cfg(test)]
    fn test_is_playing(&self, addr: SocketAddr) -> bool {
        self.clients.get(&addr).is_some_and(|client| client.playing)
    }
    #[cfg(test)]
    fn test_set_play_cooldown(
        &mut self,
        addr: SocketAddr,
        cooldown: Option<shared::career::PlayCooldown>,
    ) {
        if let Some(client) = self.clients.get_mut(&addr) {
            client.view.play_cooldown = cooldown;
        }
    }
    #[cfg(test)]
    fn test_set_sanctions(&mut self, addr: SocketAddr, sanctions: Vec<Sanction>) {
        if let Some(client) = self.clients.get_mut(&addr) {
            client.view.sanctions = sanctions;
        }
    }
    #[cfg(test)]
    fn test_set_chat_switch(&mut self, enabled: Option<bool>) {
        self.chat_switch = enabled;
    }
    #[cfg(test)]
    fn test_refresh_now(&mut self) {
        self.last_presence = Instant::now() - Duration::from_secs(10);
        self.last_access_refresh = None;
        for c in self.clients.values_mut() {
            c.supporter_checked = Some(Instant::now() - ACCESS_REFRESH_AFTER);
        }
    }
    fn enabled(&self) -> bool {
        self.link.enabled()
    }
    fn new_result_id(&self) -> String {
        random_id::<16>()
    }
    fn profile(&self, addr: SocketAddr) -> Option<ProfileSummary> {
        let client = self.clients.get(&addr)?;
        (client.profile_ready
            && client.auth.is_some()
            && client
                .key_checked
                .is_some_and(|at| at.elapsed() < COSMETIC_CACHE_TTL))
        .then(|| client.view.profile.clone())
        .flatten()
    }
    /// Cosmetic authorization expires locally even if the DB worker stops responding.
    fn supporter_aura(&self, addr: SocketAddr) -> Option<shared::supporter::AuraStyle> {
        let client = self.clients.get(&addr)?;
        client.auth.as_ref()?;
        if client.key_checked?.elapsed() >= COSMETIC_CACHE_TTL {
            return None;
        }
        if client.supporter_checked?.elapsed() >= COSMETIC_CACHE_TTL {
            return None;
        }
        let status = client.view.supporter.as_ref()?;
        authorized_supporter_aura(status, unix_seconds())
    }
    fn authenticated_session(&self, addr: SocketAddr) -> Option<String> {
        let client = self.clients.get(&addr)?;
        if client.key_checked?.elapsed() >= COSMETIC_CACHE_TTL {
            return None;
        }
        Some(client.auth.as_ref()?.session_id.clone())
    }
    fn view(&self, addr: SocketAddr) -> CareerView {
        let mut view = self.clients.get(&addr).map(|c|c.view.clone()).unwrap_or_else(|| CareerView {error:(!self.enabled()).then(||"Career storage is not configured on this server. Guest matches are unranked.".into()),..Default::default()});
        // At the client ceiling a new endpoint's requests are not taken: say
        // so, and its retries get in once a place frees (LR-09).
        if self.enabled() && !self.clients.contains_key(&addr) && self.clients.len() >= MAX_CLIENTS
        {
            view.error = Some(SERVER_FULL_TEXT.into());
            view.error_code = Some(shared::career::error_code::SERVER_FULL.into());
        }
        view.storage_enabled = self.enabled();
        if let Some(status) = &mut view.supporter {
            if status
                .active_until
                .is_none_or(|until| until <= unix_seconds())
            {
                status.active = false;
                status.equipped_aura = None;
                status.active_until = None;
            }
        }
        let now = unix_seconds();
        view.sanctions.retain(|sanction| sanction.active_at(now));
        view.play_cooldown = view.play_cooldown.filter(|p| p.until > now);
        view
    }
    fn sanctions(&self, addr: SocketAddr) -> Vec<Sanction> {
        let now = unix_seconds();
        self.clients.get(&addr).map_or_else(Vec::new, |c| {
            c.view
                .sanctions
                .iter()
                .filter(|sanction| sanction.active_at(now))
                .copied()
                .collect()
        })
    }
    fn chat_switch(&self) -> Option<bool> {
        self.chat_switch
    }
    fn forget(&mut self, addr: SocketAddr) {
        self.clients.remove(&addr);
    }
    fn touch(&mut self, addr: SocketAddr) {
        if let Some(c) = self.clients.get_mut(&addr) {
            c.touched = Instant::now();
        }
    }
    fn set_playing(&mut self, addr: SocketAddr, playing: bool) {
        if let Some(c) = self.clients.get_mut(&addr) {
            c.playing = playing;
        }
    }
    fn recovery_confirmed_since(&self, since: Instant) -> bool {
        self.recovery_checked.is_some_and(|at| at >= since)
    }
    fn take_ready_requests(&mut self) -> Vec<crate::career_port::ReadyRequest> {
        std::mem::take(&mut self.ready_requests)
    }
    fn take_match_requests(&mut self) -> Vec<crate::career_port::MatchRequest> {
        std::mem::take(&mut self.match_requests)
    }
    fn gameplay_principal(
        &self,
        addr: SocketAddr,
    ) -> Option<shared::public_transport::GameplayPrincipal> {
        let c = self.clients.get(&addr)?;
        let auth = c.auth.as_ref()?;
        c.view.profile.as_ref()?;
        if c.key_checked
            .is_none_or(|at| at.elapsed() >= COSMETIC_CACHE_TTL)
        {
            return None;
        }
        Some(shared::public_transport::GameplayPrincipal {
            public_key: auth.public_key.clone(),
            session_id: auth.session_id.clone(),
            session_nonce: auth.nonce.clone(),
        })
    }
    fn take_cancelled(&mut self) -> Vec<SocketAddr> {
        std::mem::take(&mut self.cancelled)
    }

    /// Ephemeral authenticated messages stay on the game thread, outside SQL.
    fn take_social(&mut self) -> Vec<(SocketAddr, shared::social::SocialRequest)> {
        std::mem::take(&mut self.social)
    }
    fn take_settled(&mut self) -> Vec<MatchResult> {
        std::mem::take(&mut self.settled)
    }
    fn gameplay_committed(&self, id: &str) -> bool {
        self.gameplay_starts.contains(id)
    }
    fn started(&self, id: &str) -> bool {
        matches!(self.starts.get(id), Some(Ok(())))
    }
    fn start_rejected(&self, id: &str) -> Option<String> {
        self.rejected.get(id).cloned()
    }
    fn forget_start(&mut self, id: &str) {
        self.starts.remove(id);
        self.gameplay_starts.remove(id);
        self.rejected.remove(id);
    }
    fn start_error(&self, id: &str) -> Option<String> {
        self.starts.get(id).and_then(|v| v.as_ref().err()).cloned()
    }
    fn start(&mut self, result: MatchResult) -> bool {
        self.record(RecordKind::Start, result)
    }
    fn checkpoint(&mut self, result: MatchResult) -> bool {
        self.record(RecordKind::Checkpoint, result)
    }
    fn settle(&mut self, result: MatchResult) -> bool {
        self.record(RecordKind::Settle, result)
    }
    fn take_match_refusals(&mut self) -> Vec<(crate::career_port::MatchRequest, String)> {
        std::mem::take(&mut self.match_refusals)
    }
    fn shared_ready_enabled(&self) -> bool {
        self.link.durable_ready()
    }
    fn submit_ready(&mut self, work: consent::Work) -> bool {
        if !self.enabled() || !work.valid() {
            return false;
        }
        let key = (work.check_id.clone(), work.sequence);
        if let Some(old) = self.ready_jobs.get(&key) {
            return old == &work;
        }
        if let Some(old) = self
            .ready_completions
            .iter()
            .find(|c| c.work.check_id == work.check_id && c.work.sequence == work.sequence)
        {
            return old.work == work;
        }
        if self.ready_jobs.len() + self.ready_completions.len() >= MAX_JOBS
            || !self.link.try_send(Job::Ready(work.clone()))
        {
            return false;
        }
        self.ready_jobs.insert(key, work);
        self.record_generation = self.record_generation.saturating_add(1);
        true
    }
    fn take_ready_completions(&mut self) -> Vec<consent::Completion> {
        std::mem::take(&mut self.ready_completions)
    }
    fn record_queue_dodge(&mut self, event: queue_dodges::Event) -> bool {
        if event.validate().is_err() || !self.enabled() {
            return false;
        }
        if let Some(old) = self
            .queue_dodges
            .get(&event.event_id)
            .or_else(|| self.queue_dodge_acks.get(&event.event_id))
        {
            return old == &event;
        }
        if self.queue_dodges.len() + self.queue_dodge_acks.len() >= MAX_JOBS {
            return false;
        }
        if !self.link.try_send(Job::QueueDodge(event.clone())) {
            return false;
        }
        self.queue_dodges.insert(event.event_id.clone(), event);
        self.record_generation = self.record_generation.saturating_add(1);
        true
    }
    fn take_queue_dodge_committed(&mut self) -> Vec<String> {
        self.queue_dodge_acks.drain().map(|(id, _)| id).collect()
    }
    fn flush_outbox(&mut self) -> bool {
        if !self.enabled()
            || self
                .flushed_generation
                .is_some_and(|g| g >= self.record_generation)
        {
            return true;
        }
        if self.flush_requested != Some(self.record_generation)
            && self.link.try_send(Job::Flush(self.record_generation))
        {
            self.flush_requested = Some(self.record_generation);
        }
        false
    }
    fn handle(&mut self, addr: SocketAddr, request: CareerRequest) {
        if !self.enabled() {
            return;
        }
        if !self.clients.contains_key(&addr) && !self.make_room(addr) {
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
                c.login_sent = None;
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
                c.view.error = None;
                c.view.error_code = None;
                // One Login per challenge at a time: retries of a sign-in that is
                // still being answered add no database work (LR-16).
                if c.login_sent
                    .is_some_and(|at| now.duration_since(at) < LOGIN_IN_FLIGHT)
                {
                    return;
                }
                // A full queue is not an error: the client retries the same
                // signed challenge shortly, as it does for a lost reply.
                if self.link.try_send(Job::Login { addr, challenge }) {
                    c.login_sent = Some(now);
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
                if c.key_checked
                    .is_none_or(|at| at.elapsed() >= COSMETIC_CACHE_TTL)
                    || session_nonce != auth.nonce
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
                c.view.error_code = None;
                if let CareerAction::FindMatch {
                    request_id,
                    preference,
                    also_casual,
                } = action
                {
                    c.view.match_service = None;
                    c.view.match_service_request_id = None;
                    if let Some(profile) = &c.view.profile {
                        c.queue_sequence = Some(sequence);
                        let request = crate::career_port::MatchRequest {
                            addr,
                            request_id,
                            preference,
                            also_casual,
                        };
                        if !self.link.try_send(Job::QueueAccess {
                            request,
                            nonce: session_nonce,
                            sequence,
                            public_key: auth.public_key.clone(),
                            profile_id: profile.profile_id.clone(),
                        }) {
                            c.view.error = Some("Account service is busy. Retry shortly.".into());
                        }
                    }
                    return;
                }
                if let CareerAction::ReadyMatch {
                    request_id,
                    check_id,
                    accepted,
                } = action
                {
                    if c.view.profile.is_some()
                        && check_id.len() == 32
                        && self.ready_requests.len() < MAX_JOBS
                    {
                        self.ready_requests.push(crate::career_port::ReadyRequest {
                            addr,
                            request_id,
                            check_id,
                            accepted,
                        });
                    }
                    return;
                }
                if matches!(action, CareerAction::CancelQueue) {
                    c.queue_sequence = None;
                    c.view.match_service = None;
                    c.view.match_service_request_id = None;
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
                let job = Job::Action {
                    addr,
                    nonce: session_nonce,
                    profile_id: profile.profile_id.clone(),
                    public_key: auth.public_key.clone(),
                    action,
                };
                if !self.link.try_send(job) {
                    c.view.error = Some("Account service is busy. Retry shortly.".into());
                    c.view.response_id = request_id;
                }
            }
            _ => {
                c.view.error = Some("Account requests require a valid signature.".into());
            }
        }
    }
    fn poll(&mut self) {
        while let Some(reply) = self.link.try_recv() {
            match reply {
                Reply::QueueAccess {
                    request,
                    nonce,
                    sequence,
                    access,
                } => {
                    let Some(c) = self.clients.get_mut(&request.addr) else {
                        continue;
                    };
                    if c.auth.as_ref().is_none_or(|a| a.nonce != nonce)
                        || c.queue_sequence != Some(sequence)
                    {
                        continue;
                    }
                    c.queue_sequence = None;
                    match access {
                        Ok(access) => {
                            c.view.match_service = None;
                            c.view.match_service_request_id = None;
                            if let Some(access) = access {
                                if !access.key_active {
                                    c.auth = None;
                                    c.challenge = None;
                                    c.profile_ready = false;
                                    c.key_checked = None;
                                    c.view.auth_nonce = None;
                                    c.view.challenge = None;
                                    c.view.error = Some("This device has been revoked.".into());
                                    continue;
                                }
                                c.key_checked = Some(Instant::now());
                                if let Some(profile) = access.profile {
                                    accept_profile_snapshot(&mut c.view.profile, profile, true);
                                }
                                c.view.sanctions = access.sanctions;
                                c.view.play_cooldown = access.play_cooldown;
                                if let Some(profile) = c.view.profile.as_mut() {
                                    profile.completed_public_matches = profile
                                        .completed_public_matches
                                        .max(access.completed_public_matches);
                                }
                                if let (Some(nickname), Some(profile)) =
                                    (access.nickname, c.view.profile.as_mut())
                                {
                                    profile.nickname = nickname;
                                }
                            }
                            if self.match_requests.len() < MAX_JOBS {
                                self.match_requests.push(request);
                            } else {
                                c.view.error =
                                    Some("Account service is busy. Retry shortly.".into());
                            }
                        }
                        Err(error) => {
                            c.view.error_code = queue_refusal_code(&error).map(str::to_owned);
                            if let Some(code) = queue_refusal_code(&error) {
                                if self.match_refusals.len() < MAX_JOBS {
                                    self.match_refusals.push((request, code.into()));
                                }
                                c.view.match_service =
                                    Some(shared::match_service::MatchServiceView::Failed {
                                        code: code.into(),
                                    });
                                c.view.match_service_request_id = Some(request.request_id);
                            }
                            c.view.error = Some(error);
                        }
                    }
                }
                Reply::Flushed(generation, success) => {
                    if success {
                        self.flushed_generation =
                            Some(self.flushed_generation.unwrap_or(0).max(generation));
                    } else if self.flush_requested == Some(generation) {
                        self.flush_requested = None;
                    }
                }
                Reply::Ready(completion) => {
                    let key = (completion.work.check_id.clone(), completion.work.sequence);
                    if self.ready_jobs.get(&key) == Some(&completion.work) {
                        self.ready_jobs.remove(&key);
                        self.ready_completions.push(completion);
                    }
                }
                Reply::RecoveryChecked => {
                    self.recovery_checked = Some(Instant::now());
                }
                Reply::QueueDodgeCommitted(event) => {
                    // A recovered receipt can share an ID with conflicting
                    // producer state. Only the full committed payload is an ACK.
                    if self
                        .queue_dodges
                        .get(&event.event_id)
                        .is_some_and(|expected| expected == &event)
                    {
                        self.queue_dodges.remove(&event.event_id);
                        self.queue_dodge_acks.insert(event.event_id.clone(), event);
                    }
                }
                Reply::ChatSwitch(enabled) => self.chat_switch = Some(enabled),
                Reply::RefreshAccess {
                    profile,
                    completed_public_matches,
                    addr,
                    nonce,
                    key_active,
                    supporter,
                    nickname,
                    sanctions,
                    play_cooldown,
                } => {
                    let Some(c) = self.clients.get_mut(&addr) else {
                        continue;
                    };
                    if c.auth.as_ref().is_none_or(|auth| auth.nonce != nonce) {
                        continue;
                    }
                    c.access_refresh_sent = None;
                    if key_active == Some(true)
                        && let Some(profile) = profile
                    {
                        accept_profile_snapshot(&mut c.view.profile, profile, true);
                    }
                    if let (Some(count), Some(profile)) =
                        (completed_public_matches, c.view.profile.as_mut())
                    {
                        profile.completed_public_matches =
                            profile.completed_public_matches.max(count);
                    }
                    if let Some(cooldown) = play_cooldown {
                        c.view.play_cooldown = cooldown;
                    }
                    if let Some(sanctions) = sanctions {
                        c.view.sanctions = sanctions;
                    }
                    // A moderator's name reset shows in the next presence (LR-15).
                    if let (Some(name), Some(profile)) = (nickname, c.view.profile.as_mut()) {
                        profile.nickname = name;
                    }
                    let banned = shared::moderation::active(
                        &c.view.sanctions,
                        SanctionKind::AccountBan,
                        unix_seconds(),
                    )
                    .is_some();
                    if key_active == Some(false) {
                        c.auth = None;
                        c.profile_ready = false;
                        c.view.auth_nonce = None;
                        c.view.supporter = Some(Default::default());
                        c.view.error = Some("This device was revoked. Authorize this device again from your account.".into());
                    } else if banned {
                        // An account ban ends the signed session at once, as a
                        // revoked key does; signing in again is refused while
                        // it lasts (LR-15).
                        c.auth = None;
                        c.profile_ready = false;
                        c.view.auth_nonce = None;
                        c.view.supporter = Some(Default::default());
                        c.view.error = Some(shared::moderation::ACCOUNT_BANNED_TEXT.into());
                        c.view.error_code = Some(error_code::ACCOUNT_BANNED.into());
                    } else {
                        if key_active == Some(true) {
                            c.key_checked = Some(Instant::now());
                        }
                        c.view.supporter = supporter;
                    }
                    c.supporter_checked = Some(Instant::now());
                }
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
                    c.login_sent = None;
                    match result {
                        Ok(mut view) => {
                            view.auth_nonce = Some(challenge.nonce.clone());
                            c.supporter_checked = Some(Instant::now());
                            c.key_checked = Some(Instant::now());
                            c.view = view;
                            c.auth = Some(challenge);
                            c.profile_ready = true;
                            c.sequence = 0;
                        }
                        Err(refusal) => {
                            c.view.error_code =
                                sign_in_error_code(&refusal.error).map(str::to_owned);
                            c.view.error = Some(refusal.error);
                            c.view.sanctions = refusal.sanctions;
                        }
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
                    c.view.error_code = None;
                    c.view.history.clear();
                    c.view.history_loaded = false;
                    c.view.history_next = None;
                    c.view.detail = None;
                    c.view.friends = None;
                    c.view.visited_profile = None;
                    match result {
                        Ok(view) => {
                            if let Some(supporter) = view.supporter {
                                c.view.supporter = Some(supporter);
                                c.supporter_checked = Some(Instant::now());
                            }
                            if let Some(profile) = view.profile {
                                accept_profile_snapshot(&mut c.view.profile, profile, false);
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
                        Err(e) => {
                            c.view.error_code = action_error_code(&e).map(str::to_owned);
                            c.view.error = Some(e);
                        }
                    }
                }
                Reply::GameplayStarted(id) => {
                    self.gameplay_starts.insert(id);
                }
                Reply::Started(id) => {
                    self.starts.insert(id, Ok(()));
                }
                Reply::Rejected(id, error) => {
                    for (addr, c) in &mut self.clients {
                        if let (Some(profile), Some(auth)) = (&c.view.profile, &c.auth) {
                            if self.matches.get(&profile.profile_id) == Some(&id) {
                                c.profile_ready = false;
                                self.link.try_send(Job::Action {
                                    addr: *addr,
                                    nonce: auth.nonce.clone(),
                                    profile_id: profile.profile_id.clone(),
                                    public_key: auth.public_key.clone(),
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
                                accept_profile_snapshot(&mut c.view.profile, fresh.clone(), false);
                                c.profile_ready = true;
                                c.view.last_result = Some(result.clone());
                                c.view.error = None;
                            }
                        }
                    }
                    self.pending_ids.remove(&result.result_id);
                    self.matches.retain(|_, id| *id != result.result_id);
                    self.starts.remove(&result.result_id);
                    self.gameplay_starts.remove(&result.result_id);
                    self.settled.push(result);
                }
            }
        }
        for c in self.clients.values_mut() {
            if c.auth.is_some()
                && c.key_checked
                    .is_none_or(|at| at.elapsed() >= COSMETIC_CACHE_TTL)
            {
                c.auth = None;
                c.challenge = None;
                c.profile_ready = false;
                c.view.auth_nonce = None;
                c.view.challenge = None;
                c.view.supporter = None;
                c.view.error = Some("Account authorization could not be refreshed. Reconnect when the account service is available.".into());
            }
        }
        self.clients
            .retain(|_, c| c.touched.elapsed() < Duration::from_secs(120));
        self.refresh_access(Instant::now());
        if self.last_presence.elapsed() >= Duration::from_secs(10) {
            self.last_presence = Instant::now();
            // Multiple device endpoints may represent one profile. Its live
            // allocation takes precedence over an idle duplicate session.
            let mut presence = BTreeMap::<String, Option<String>>::new();
            for c in self.clients.values() {
                if let Some(profile) = &c.view.profile {
                    let game = c
                        .playing
                        .then(|| self.matches.get(&profile.profile_id).cloned())
                        .flatten();
                    let entry = presence.entry(profile.profile_id.clone()).or_default();
                    if game.is_some() {
                        *entry = game;
                    }
                }
            }
            let presence = presence.into_iter().collect();
            if self.link.enabled() {
                self.link.try_send(Job::Presence(presence));
                for (addr, c) in &self.clients {
                    if !c.profile_ready {
                        if let (Some(profile), Some(auth)) = (&c.view.profile, &c.auth) {
                            self.link.try_send(Job::Action {
                                addr: *addr,
                                nonce: auth.nonce.clone(),
                                profile_id: profile.profile_id.clone(),
                                public_key: auth.public_key.clone(),
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

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
}
/// The aura a player may show at `now`: re-checked against the grant window
/// and, for a tier style, the Hero tier points every time (LR-67).
fn authorized_supporter_aura(
    status: &shared::supporter::SupporterStatus,
    now: i64,
) -> Option<shared::supporter::AuraStyle> {
    status.authorized_aura(now)
}

fn action_id(action: &CareerAction) -> Option<u64> {
    match action {
        CareerAction::ReadyMatch { request_id, .. }
        | CareerAction::FindMatch { request_id, .. }
        | CareerAction::SupporterStatus { request_id }
        | CareerAction::EquipSupporterAura { request_id, .. }
        | CareerAction::History { request_id, .. }
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
        CareerAction::SupporterStatus { .. } => {
            view.supporter = Some(store.supporter_status(id).await?)
        }
        CareerAction::EquipSupporterAura { aura, .. } => {
            view.supporter = Some(store.equip_supporter_aura(id, aura).await?)
        }
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
        CareerAction::CancelQueue
        | CareerAction::FindMatch { .. }
        | CareerAction::ReadyMatch { .. } => {}
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

/// What a restarted career worker takes over from its outbox directory.
#[derive(Default)]
struct RecoveredOutbox {
    /// Receipts still to deliver, by result id. A live (Start or Checkpoint)
    /// receipt comes back as an unrated `Interrupted` settle.
    pending: BTreeMap<String, PendingRecord>,
    /// Results the store already refused; later records for them are dropped.
    rejected_ids: VecDeque<String>,
}

/// Reads the outbox left by a previous worker. Only `*.json` receipts are
/// considered; an unreadable one is left on disk for manual recovery and a
/// missing directory is an empty outbox.
fn recover_outbox(outbox: &Path) -> RecoveredOutbox {
    let mut pending: BTreeMap<String, PendingRecord> = BTreeMap::new();
    let mut rejected_ids = VecDeque::new();
    if let Ok(entries) = fs::read_dir(outbox) {
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
    RecoveredOutbox {
        pending,
        rejected_ids,
    }
}

/// What the worker needs of the admission limits (LR-16).
#[derive(Clone, Copy)]
pub(crate) struct Admission {
    new_profiles: omoba_career_store::career_store::NewProfileLimits,
    /// Retention of profiles that never played; `None` in match workers.
    retention_days: Option<u32>,
}

#[cfg(test)]
impl Admission {
    pub(crate) fn for_test() -> Self {
        Self {
            // Live fixtures share a reusable loopback database with load
            // probes. Their synthetic identities must not exhaust the real
            // network's daily signup quota; quota behavior has store tests.
            new_profiles: omoba_career_store::career_store::NewProfileLimits {
                per_source: 1_000_000,
                total: 10_000_000,
            },
            retention_days: None,
        }
    }
}

const STORE_UNAVAILABLE: &str = "Profile storage is unavailable. Retry shortly.";

/// A sign-in or account action that waited [`STORE_WAIT`] for the database.
fn unavailable(job: Job) -> Option<Reply> {
    match job {
        Job::QueueAccess {
            request,
            nonce,
            sequence,
            ..
        } => Some(Reply::QueueAccess {
            request,
            nonce,
            sequence,
            access: Err(STORE_UNAVAILABLE.into()),
        }),
        Job::Login { addr, challenge } => Some(Reply::Login {
            addr,
            challenge,
            result: Err(STORE_UNAVAILABLE.into()),
        }),
        Job::Action {
            addr,
            nonce,
            action,
            ..
        } => Some(Reply::Action {
            addr,
            nonce,
            request_id: action_id(&action),
            result: Err(STORE_UNAVAILABLE.into()),
        }),
        _ => None,
    }
}

/// Answers a sign-in or an account action with a connected store. A sign-in
/// of a new key counts against the new-profile limits of its source IP.
fn answer(
    rt: &tokio::runtime::Runtime,
    store: &CareerStore,
    job: Job,
    admission: &Admission,
) -> Option<Reply> {
    match job {
        Job::QueueAccess {
            request,
            nonce,
            sequence,
            public_key,
            profile_id,
        } => Some(Reply::QueueAccess {
            request,
            nonce,
            sequence,
            access: rt
                .block_on(store.queue_access(&public_key, &profile_id))
                .map(Some),
        }),
        Job::Login { addr, challenge } => {
            let source = SourceIp::of(addr).key();
            let result = rt.block_on(async {
                let profile = store
                    .authenticate_from(
                        &challenge.public_key,
                        &challenge.nickname,
                        &source,
                        admission.new_profiles,
                    )
                    .await?;
                // An account ban refuses the sign-in itself (LR-15).
                let access = store
                    .access(&challenge.public_key, &profile.profile_id)
                    .await?;
                let sanctions = access.sanctions;
                if shared::moderation::active(&sanctions, SanctionKind::AccountBan, unix_seconds())
                    .is_some()
                {
                    return Err(LoginRefusal {
                        error: shared::moderation::ACCOUNT_BANNED_TEXT.into(),
                        sanctions,
                    });
                }
                let (history, _) = store.history(&profile.profile_id, None).await?;
                let last_result = match history.first() {
                    Some(last) => store
                        .detail(&profile.profile_id, &last.result_id)
                        .await
                        .ok(),
                    None => None,
                };
                Ok(CareerView {
                    supporter: store.supporter_status(&profile.profile_id).await.ok(),
                    profile: Some(profile),
                    last_result,
                    play_cooldown: access.play_cooldown,
                    sanctions,
                    ..Default::default()
                })
            });
            Some(Reply::Login {
                addr,
                challenge,
                result,
            })
        }
        Job::Action {
            addr,
            nonce,
            profile_id,
            public_key,
            action,
        } => {
            let request_id = action_id(&action);
            let result = rt.block_on(async {
                if !store.key_is_active(&public_key, &profile_id).await? {
                    return Err("This device has been revoked.".into());
                }
                account_action(store, &profile_id, action).await
            });
            Some(Reply::Action {
                addr,
                nonce,
                result,
                request_id,
            })
        }
        _ => None,
    }
}

/// The client's code for a refused sign-in (`CareerView::error_code`).
fn sign_in_error_code(error: &str) -> Option<&'static str> {
    use omoba_career_store::career_store::{NEW_PROFILE_LIMIT_ERROR, NEW_PROFILES_PAUSED_ERROR};
    use shared::career::error_code;
    match error {
        NEW_PROFILE_LIMIT_ERROR => Some(error_code::NEW_PROFILE_LIMIT),
        NEW_PROFILES_PAUSED_ERROR => Some(error_code::NEW_PROFILES_PAUSED),
        shared::moderation::ACCOUNT_BANNED_TEXT => Some(error_code::ACCOUNT_BANNED),
        _ => None,
    }
}

/// The client's code for a refused account action (LR-15: a locked rename).
fn action_error_code(error: &str) -> Option<&'static str> {
    (error == omoba_career_store::career_store::moderation::NAME_LOCKED_ERROR)
        .then_some(shared::career::error_code::NAME_LOCKED)
}

const ACCESS_READ_BATCH: usize = 4;

fn collect_access_reads(first: Job, jobs: &Receiver<Job>, deferred: &mut Option<Job>) -> Vec<Job> {
    let mut batch = vec![first];
    while batch.len() < ACCESS_READ_BATCH {
        match jobs.try_recv() {
            Ok(job @ (Job::QueueAccess { .. } | Job::RefreshAccess { .. })) => batch.push(job),
            Ok(barrier) => {
                *deferred = Some(barrier);
                break;
            }
            Err(_) => break,
        }
    }
    batch
}

async fn answer_access_read(store: CareerStore, job: Job) -> Reply {
    match job {
        Job::QueueAccess {
            request,
            nonce,
            sequence,
            public_key,
            profile_id,
        } => Reply::QueueAccess {
            request,
            nonce,
            sequence,
            access: store.queue_access(&public_key, &profile_id).await.map(Some),
        },
        Job::RefreshAccess {
            addr,
            nonce,
            public_key,
            profile_id,
        } => {
            let access = store.access(&public_key, &profile_id).await.ok();
            let key_active = access.as_ref().map(|a| a.key_active);
            let supporter = if key_active == Some(true) {
                store.supporter_status(&profile_id).await.ok()
            } else {
                None
            };
            let (profile, nickname, sanctions, play_cooldown, completed_public_matches) = access
                .map(|a| {
                    (
                        a.profile,
                        a.nickname,
                        Some(a.sanctions),
                        Some(a.play_cooldown),
                        Some(a.completed_public_matches),
                    )
                })
                .unwrap_or_default();
            Reply::RefreshAccess {
                profile,
                completed_public_matches,
                addr,
                nonce,
                key_active,
                supporter,
                nickname,
                sanctions,
                play_cooldown,
            }
        }
        _ => unreachable!("only access reads enter a concurrent batch"),
    }
}

async fn answer_access_reads(store: &CareerStore, batch: Vec<Job>) -> Vec<Reply> {
    let mut tasks = tokio::task::JoinSet::new();
    for job in batch {
        tasks.spawn(answer_access_read(store.clone(), job));
    }
    let mut replies = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(reply) => replies.push(reply),
            Err(_) => eprintln!("Account read task failed; authorization remains fail-closed."),
        }
    }
    replies
}

fn queue_refusal_code(error: &str) -> Option<&str> {
    use shared::career::error_code::{
        ALLOCATION_CANCELLED_OR_EXPIRED, DODGE_COOLDOWN, PROFILE_HAS_ANOTHER_SESSION,
    };
    matches!(
        error,
        DODGE_COOLDOWN | PROFILE_HAS_ANOTHER_SESSION | ALLOCATION_CANCELLED_OR_EXPIRED
    )
    .then_some(error)
}

fn persist_queue_dodges(
    journal: &mut Option<queue_dodges::Journal>,
    staged: &mut BTreeMap<String, queue_dodges::Event>,
) -> bool {
    let Some(journal) = journal else {
        return false;
    };
    let events: Vec<_> = staged.values().cloned().collect();
    for event in events {
        match journal.record(event.clone()) {
            Ok(()) => {
                staged.remove(&event.event_id);
            }
            Err(error) => eprintln!("Cannot persist queue-dodge journal: {error:?}"),
        }
    }
    staged.is_empty()
}

fn worker(
    url: String,
    outbox: PathBuf,
    jobs: Receiver<Job>,
    replies: SyncSender<Reply>,
    scope: RecoveryScope,
    admission: Admission,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("career async runtime");
    let mut store: Option<CareerStore> = None;
    let RecoveredOutbox {
        mut pending,
        mut rejected_ids,
    } = recover_outbox(&outbox);
    let queue_root = outbox.join("queue-dodges");
    let mut queue_journal = queue_dodges::Journal::open(queue_root.clone()).ok();
    let mut queue_staged: BTreeMap<String, queue_dodges::Event> = BTreeMap::new();
    let mut last_queue_open = Instant::now();
    let mut queue_retry_at = Instant::now();
    let mut last_retry = Instant::now() - Duration::from_secs(10);
    let mut heartbeat = Instant::now();
    let mut connected = true;
    let mut _heartbeat_task = None;
    // Sign-ins and account actions that arrived while the database was not
    // connected (after a host reboot): answered once it is, or "unavailable"
    // after STORE_WAIT, instead of failing at once.
    let mut parked: VecDeque<(Instant, Job)> = VecDeque::new();
    // The first retention run waits out a restart's reconnect storm.
    let mut next_retention = Instant::now() + Duration::from_secs(300);
    let mut last_chat_poll: Option<Instant> = None;
    let mut deferred_job: Option<Job> = None;
    while connected
        || !pending.is_empty()
        || !queue_staged.is_empty()
        || queue_journal.as_ref().is_some_and(|j| !j.is_empty())
    {
        if queue_journal.is_none() && last_queue_open.elapsed() >= Duration::from_secs(2) {
            last_queue_open = Instant::now();
            queue_journal = queue_dodges::Journal::open(queue_root.clone()).ok();
            if queue_journal.is_none() {
                eprintln!("Queue-dodge journal unavailable; new queue access remains fail-closed.");
            }
        }
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
        // A reconnect drains older parked operations before any fresh read.
        let next = if store.is_some() && !parked.is_empty() {
            Err(mpsc::RecvTimeoutError::Timeout)
        } else {
            deferred_job
                .take()
                .map(Ok)
                .unwrap_or_else(|| jobs.recv_timeout(Duration::from_millis(100)))
        };
        match next {
            Ok(job @ (Job::QueueAccess { .. } | Job::RefreshAccess { .. })) if store.is_some() => {
                let batch = collect_access_reads(job, &jobs, &mut deferred_job);
                let ready =
                    queue_staged.is_empty() && queue_journal.as_ref().is_some_and(|j| j.is_empty());
                let mut allowed = Vec::new();
                for job in batch {
                    if matches!(job, Job::QueueAccess { .. }) && !ready {
                        if let Some(reply) = unavailable(job) {
                            let _ = replies.try_send(reply);
                        }
                    } else {
                        allowed.push(job);
                    }
                }
                for reply in rt.block_on(answer_access_reads(store.as_ref().unwrap(), allowed)) {
                    let _ = replies.try_send(reply);
                }
            }
            Ok(Job::Ready(work)) => {
                let mut batch = vec![work];
                while batch.len() < ACCESS_READ_BATCH {
                    match jobs.try_recv() {
                        Ok(Job::Ready(work)) => batch.push(work),
                        Ok(other) => {
                            deferred_job = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                let completed = match &store {
                    Some(s) => rt.block_on(async {
                        let mut tasks = tokio::task::JoinSet::new();
                        for work in batch {
                            tasks.spawn(consent::execute(s.clone(), work));
                        }
                        let mut completed = Vec::new();
                        while let Some(result) = tasks.join_next().await {
                            if let Ok(reply) = result {
                                completed.push(reply);
                            }
                        }
                        completed
                    }),
                    None => batch
                        .into_iter()
                        .map(|work| consent::Completion {
                            work,
                            result: Err(STORE_UNAVAILABLE.into()),
                        })
                        .collect(),
                };
                for completion in completed {
                    let _ = replies.send(Reply::Ready(completion));
                }
            }
            Ok(Job::Flush(generation)) => {
                let matches_flushed = pending
                    .values()
                    .all(|record| write_record(&outbox, record).is_ok());
                let queue_flushed = persist_queue_dodges(&mut queue_journal, &mut queue_staged)
                    && queue_journal.as_ref().is_some_and(|j| j.flush().is_ok());
                let flushed = matches_flushed && queue_flushed;
                // Shutdown keeps polling the receiver; unlike an optional view,
                // this durability acknowledgement must not be dropped on a full channel.
                let _ = replies.send(Reply::Flushed(generation, flushed));
            }
            Ok(Job::QueueDodge(event)) => {
                if queue_staged.len() >= MAX_JOBS && !queue_staged.contains_key(&event.event_id) {
                    deferred_job = Some(Job::QueueDodge(event));
                } else {
                    match queue_staged.get(&event.event_id) {
                        Some(old) if old != &event => {
                            eprintln!("Conflicting queue-dodge event identity; original retained.")
                        }
                        _ => {
                            queue_staged.insert(event.event_id.clone(), event);
                        }
                    }
                    persist_queue_dodges(&mut queue_journal, &mut queue_staged);
                }
            }
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
            Ok(job @ (Job::Login { .. } | Job::Action { .. } | Job::QueueAccess { .. })) => {
                if parked.len() >= MAX_JOBS
                    && let Some(reply) = parked.pop_front().and_then(|(_, job)| unavailable(job))
                {
                    let _ = replies.try_send(reply);
                }
                parked.push_back((Instant::now(), job));
            }
            Ok(Job::RefreshAccess {
                addr,
                nonce,
                public_key,
                profile_id,
            }) => {
                // One query: the key, the current name and the sanctions.
                let access = match &store {
                    Some(s) => rt.block_on(s.access(&public_key, &profile_id)).ok(),
                    None => None,
                };
                let key_active = access.as_ref().map(|a| a.key_active);
                let supporter = match &store {
                    Some(s) if key_active == Some(true) => {
                        rt.block_on(s.supporter_status(&profile_id)).ok()
                    }
                    _ => None,
                };
                let (profile, nickname, sanctions, play_cooldown, completed_public_matches) =
                    access
                        .map(|a| {
                            (
                                a.profile,
                                a.nickname,
                                Some(a.sanctions),
                                Some(a.play_cooldown),
                                Some(a.completed_public_matches),
                            )
                        })
                        .unwrap_or_default();
                let _ = replies.try_send(Reply::RefreshAccess {
                    profile,
                    completed_public_matches,
                    addr,
                    nonce,
                    key_active,
                    supporter,
                    nickname,
                    sanctions,
                    play_cooldown,
                });
            }
            Ok(Job::Presence(presence)) => {
                if let Some(s) = &store {
                    for batch in presence.chunks(PRESENCE_BATCH_SIZE) {
                        let _ = rt.block_on(s.touch_presences(batch));
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => connected = false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while let Some((at, _)) = parked.front() {
            let waited = at.elapsed() >= STORE_WAIT;
            let reply = match &store {
                Some(s) => parked.pop_front().and_then(|(_, job)| {
                    let ready = queue_staged.is_empty()
                        && queue_journal.as_ref().is_some_and(|j| j.is_empty());
                    if matches!(job, Job::QueueAccess { .. }) && !ready {
                        unavailable(job)
                    } else {
                        answer(&rt, s, job, &admission)
                    }
                }),
                None if waited => parked.pop_front().and_then(|(_, job)| unavailable(job)),
                None => break,
            };
            if let Some(reply) = reply {
                let _ = replies.try_send(reply);
            }
        }
        if let Some(s) = &store
            && last_chat_poll.is_none_or(|at| at.elapsed() >= CHAT_SWITCH_POLL)
        {
            last_chat_poll = Some(Instant::now());
            if let Ok(enabled) = rt.block_on(s.chat_enabled()) {
                let _ = replies.try_send(Reply::ChatSwitch(enabled));
            }
        }
        if let (Some(days), Some(s)) = (admission.retention_days, &store)
            && Instant::now() >= next_retention
        {
            next_retention = Instant::now() + RETENTION_INTERVAL;
            match rt.block_on(s.prune_unplayed_profiles(days, RETENTION_BATCH)) {
                Ok(0) => {}
                Ok(pruned) => println!(
                    "PROFILE_RETENTION pruned={pruned} min_age_days={days} (never played, unlinked)"
                ),
                Err(error) => eprintln!("Profile retention failed: {error}"),
            }
        }
        if Instant::now() >= queue_retry_at || !connected {
            persist_queue_dodges(&mut queue_journal, &mut queue_staged);
            let mut failed = false;
            if let (Some(journal), Some(s)) = (&mut queue_journal, &store) {
                let events: Vec<_> = journal.pending().take(16).cloned().collect();
                for event in events {
                    match rt.block_on(s.record_queue_dodge(
                        &event.event_id,
                        &event.profile_id,
                        event.observed_at_ms,
                    )) {
                        Ok(_) => {
                            if replies
                                .try_send(Reply::QueueDodgeCommitted(event.clone()))
                                .is_ok()
                                && let Err(error) = journal.complete(&event.event_id)
                            {
                                failed = true;
                                eprintln!("Cannot retire committed queue-dodge receipt: {error:?}");
                            }
                        }
                        Err(error) => {
                            failed = true;
                            eprintln!("Queue-dodge SQL delivery pending: {error}");
                        }
                    }
                }
            }
            queue_retry_at = Instant::now()
                + if failed || store.is_none() {
                    Duration::from_secs(2)
                } else {
                    Duration::from_millis(100)
                };
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
                        if matches!(record.kind, RecordKind::Checkpoint)
                            && record.result.gameplay_started
                            && replies
                                .try_send(Reply::GameplayStarted(id.clone()))
                                .is_err()
                        {
                            continue; // This ACK gates Running; retain and retry the durable record.
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
                                    == "Queued ratings changed. Clear the queue and try again."
                                || queue_refusal_code(&error).is_some())
                        {
                            let message = if error.starts_with("Queued ratings")
                                || queue_refusal_code(&error).is_some()
                            {
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
                    let recovered = match scope {
                        RecoveryScope::Global => rt.block_on(s.recover_expired()),
                        RecoveryScope::Disabled => Ok(Vec::new()),
                        RecoveryScope::Epoch(epoch) => {
                            rt.block_on(s.recover_expired_for_epoch(epoch))
                        }
                    };
                    if let Ok(results) = recovered {
                        if matches!(scope, RecoveryScope::Epoch(_)) {
                            let _ = replies.try_send(Reply::RecoveryChecked);
                        }
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

/// Periodic reads adopt a reset, not an older award from the same season.
/// Every own-profile reply also refuses a cross-season downgrade: concurrent
/// access reads may overtake a pre-reset action/settlement reply.
fn accept_profile_snapshot(
    current: &mut Option<ProfileSummary>,
    fresh: ProfileSummary,
    only_newer_season: bool,
) {
    if let Some(old) = current.as_ref() {
        let season = |p: &ProfileSummary| p.ranked.as_ref().map_or(0, |s| s.season_id);
        if fresh.profile_id != old.profile_id
            || season(&fresh) < season(old)
            || (only_newer_season && season(&fresh) == season(old))
        {
            return;
        }
    } else if only_newer_season {
        return;
    }
    *current = Some(fresh);
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    fn challenge_request(n: u8) -> CareerRequest {
        CareerRequest::Challenge {
            public_key: hex(SigningKey::from_bytes(&[n; 32]).verifying_key().as_bytes()),
            nickname: "Player".into(),
            session_id: format!("session-{n}"),
        }
    }
    fn at(ip: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::from((ip, port))
    }

    /// LR-16: one IP holds at most `endpoints_per_ip` career clients; another
    /// IP is not affected, and an idle client of that IP gives way.
    #[test]
    fn queue_access_is_fresh_correlated_and_fails_closed() {
        let mut backend = MemoryCareer::test_backend(7);
        let addr = at([127, 0, 0, 1], 50101);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("c".repeat(64), "Player".into()),
            "session",
        );
        let request = crate::career_port::MatchRequest {
            addr,
            request_id: 1,
            preference: shared::match_service::MatchPreference::Ranked,
            also_casual: false,
        };
        let nonce = backend.clients[&addr].auth.as_ref().unwrap().nonce.clone();
        let until = unix_seconds() + 300;
        let fresh = || omoba_career_store::career_store::moderation::Access {
            profile: None,
            completed_public_matches: 10,
            key_active: true,
            nickname: None,
            sanctions: Vec::new(),
            play_cooldown: Some(shared::career::PlayCooldown { until, strikes: 1 }),
        };
        backend.clients.get_mut(&addr).unwrap().queue_sequence = Some(1);
        backend.link.replies.push_back(Reply::QueueAccess {
            request,
            nonce: nonce.clone(),
            sequence: 1,
            access: Ok(Some(fresh())),
        });
        backend.poll();
        assert_eq!(backend.take_match_requests(), vec![request]);
        assert_eq!(backend.view(addr).play_cooldown.unwrap().until, until);
        assert_eq!(
            backend.view(addr).profile.unwrap().completed_public_matches,
            10,
            "fresh QueueAccess must replace the stale login's zero progress"
        );
        for (sequence, current, valid_nonce, access) in [
            (1, Some(2), true, Ok(Some(fresh()))),  // replaced search
            (2, None, true, Ok(Some(fresh()))),     // cancellation
            (2, Some(2), false, Ok(Some(fresh()))), // old sign-in
            (2, Some(2), true, Err(STORE_UNAVAILABLE.to_owned())),
            (
                3,
                Some(3),
                true,
                Ok(Some(omoba_career_store::career_store::moderation::Access {
                    key_active: false,
                    ..fresh()
                })),
            ),
        ] {
            backend.clients.get_mut(&addr).unwrap().queue_sequence = current;
            backend.link.replies.push_back(Reply::QueueAccess {
                request,
                nonce: if valid_nonce {
                    nonce.clone()
                } else {
                    "old".into()
                },
                sequence,
                access,
            });
            backend.poll();
            assert!(backend.take_match_requests().is_empty());
        }
        assert!(backend.authenticated_session(addr).is_none());
        assert!(backend.view(addr).auth_nonce.is_none());
    }

    #[test]
    fn one_ip_holds_at_most_n_career_clients() {
        let limit = Limits::default().endpoints_per_ip;
        let mut backend = MemoryCareer::test_backend(7);
        for port in 0..20 {
            backend.handle(at([203, 0, 113, 9], 50000 + port), challenge_request(1));
        }
        let admitted = (0..20)
            .filter(|port| {
                backend
                    .view(at([203, 0, 113, 9], 50000 + port))
                    .challenge
                    .is_some()
            })
            .count();
        assert_eq!(admitted, limit);
        backend.handle(at([203, 0, 113, 10], 50000), challenge_request(2));
        assert!(
            backend
                .view(at([203, 0, 113, 10], 50000))
                .challenge
                .is_some()
        );
        // A reconnect after the old endpoint went quiet takes its place.
        let idle = at([203, 0, 113, 9], 50000);
        backend.clients.get_mut(&idle).unwrap().touched =
            Instant::now().checked_sub(STALE_CLIENT).unwrap();
        let returning = at([203, 0, 113, 9], 60000);
        backend.handle(returning, challenge_request(1));
        assert!(backend.view(returning).challenge.is_some());
        assert!(!backend.clients.contains_key(&idle));
    }

    /// LR-09: at the career-client ceiling a new endpoint's sign-in is
    /// answered "server full" instead of silence; clients already in keep
    /// their view, and the retry gets in once a place frees.
    #[test]
    fn a_sign_in_at_the_client_ceiling_gets_server_full() {
        use shared::career::error_code;
        let mut backend = MemoryCareer::test_backend(7);
        for index in 0..MAX_CLIENTS {
            let index = u16::try_from(index).unwrap();
            backend.handle(
                at([10, (index >> 8) as u8, index as u8, 1], 40000),
                challenge_request(1),
            );
        }
        assert_eq!(backend.clients.len(), MAX_CLIENTS);
        let inside = at([10, 0, 0, 1], 40000);
        assert!(backend.view(inside).challenge.is_some());
        assert_eq!(backend.view(inside).error_code, None);
        let newcomer = at([198, 51, 100, 7], 50000);
        backend.handle(newcomer, challenge_request(2));
        let view = backend.view(newcomer);
        assert!(view.challenge.is_none());
        assert_eq!(view.error_code.as_deref(), Some(error_code::SERVER_FULL));
        assert_eq!(view.error.as_deref(), Some(SERVER_FULL_TEXT));
        assert!(!backend.clients.contains_key(&newcomer));
        // A place frees (the endpoint went away): the retry signs in.
        backend.forget(inside);
        backend.handle(newcomer, challenge_request(2));
        let view = backend.view(newcomer);
        assert!(view.challenge.is_some());
        assert_eq!(view.error_code, None);
        // Without career storage nothing is full.
        let disabled = MemoryCareer::disabled(7);
        assert_eq!(disabled.view(newcomer).error_code, None);
    }

    /// LR-16: retries of one sign-in queue one Login, a full queue is no
    /// error (the client retries), and a refused new profile carries its code.
    #[test]
    fn a_sign_in_queues_one_login_and_a_refusal_carries_its_code() {
        use omoba_career_store::career_store::NEW_PROFILE_LIMIT_ERROR;
        let key = SigningKey::from_bytes(&[3; 32]);
        let addr = at([203, 0, 113, 9], 50000);
        let mut backend = MemoryCareer::test_backend(7);
        backend.handle(addr, challenge_request(3));
        let challenge = backend.view(addr).challenge.unwrap();
        let authenticate = CareerRequest::Authenticate {
            challenge: challenge.clone(),
            signature: hex(&key.sign(&challenge.signing_bytes()).to_bytes()),
        };
        // Fill the worker queue: the sign-in waits without an error.
        for _ in 0..MAX_JOBS {
            backend.link.jobs.push_back(Job::Presence(Vec::new()));
        }
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, authenticate.clone());
        assert_eq!(backend.view(addr).error, None);
        backend.link.jobs.clear();
        for _ in 0..3 {
            backend.clients.get_mut(&addr).unwrap().last_request = None;
            backend.handle(addr, authenticate.clone());
        }
        let logins = |backend: &MemoryCareer| {
            backend
                .link
                .jobs
                .iter()
                .filter(|job| matches!(job, Job::Login { .. }))
                .count()
        };
        assert_eq!(logins(&backend), 1);
        backend.link.replies.push_back(Reply::Login {
            addr,
            challenge,
            result: Err(NEW_PROFILE_LIMIT_ERROR.into()),
        });
        backend.poll();
        let view = backend.view(addr);
        assert_eq!(
            view.error_code.as_deref(),
            Some(shared::career::error_code::NEW_PROFILE_LIMIT)
        );
        assert_eq!(view.error.as_deref(), Some(NEW_PROFILE_LIMIT_ERROR));
        assert!(view.profile.is_none());
        // Answered: a later retry asks the store again.
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, authenticate);
        assert_eq!(logins(&backend), 2);
    }
    #[test]
    fn an_account_ban_refuses_sign_in_with_its_end_and_lifting_it_needs_no_restart() {
        use shared::moderation::{ACCOUNT_BANNED_TEXT, Sanction, SanctionKind};
        let key = SigningKey::from_bytes(&[4; 32]);
        let addr = at([203, 0, 113, 10], 50000);
        let mut backend = MemoryCareer::test_backend(7);
        backend.handle(addr, challenge_request(4));
        let challenge = backend.view(addr).challenge.unwrap();
        let authenticate = CareerRequest::Authenticate {
            challenge: challenge.clone(),
            signature: hex(&key.sign(&challenge.signing_bytes()).to_bytes()),
        };
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, authenticate.clone());
        let until = unix_seconds() + 3600;
        let ban = Sanction {
            kind: SanctionKind::AccountBan,
            until: Some(until),
        };
        backend.link.replies.push_back(Reply::Login {
            addr,
            challenge: challenge.clone(),
            result: Err(LoginRefusal {
                error: ACCOUNT_BANNED_TEXT.into(),
                sanctions: vec![ban],
            }),
        });
        backend.poll();
        let view = backend.view(addr);
        assert_eq!(
            view.error_code.as_deref(),
            Some(shared::career::error_code::ACCOUNT_BANNED)
        );
        assert_eq!(view.sanctions, vec![ban], "the client can say until when");
        assert!(view.profile.is_none() && backend.authenticated_session(addr).is_none());
        // Party presence still sees the refused endpoint's ban.
        assert_eq!(backend.sanctions(addr), vec![ban]);
        // Lifted: the next sign-in (the client retries) goes to the store again.
        backend.link.jobs.clear();
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, authenticate);
        assert!(
            backend
                .link
                .jobs
                .iter()
                .any(|job| matches!(job, Job::Login { .. }))
        );
    }

    #[test]
    fn access_refreshes_apply_and_lift_sanctions_and_a_name_reset() {
        use shared::moderation::{Sanction, SanctionKind};
        let addr = "127.0.0.1:30031".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Rude#0001".into()),
            "session",
        );
        let mute = Sanction {
            kind: SanctionKind::ChatMute,
            until: Some(unix_seconds() + 600),
        };
        let refresh = |sanctions: Vec<Sanction>, nickname: Option<&str>| Reply::RefreshAccess {
            profile: None,
            completed_public_matches: None,
            addr,
            nonce: "b".repeat(64),
            key_active: Some(true),
            supporter: None,
            nickname: nickname.map(str::to_owned),
            sanctions: Some(sanctions),
            play_cooldown: Some(None),
        };
        backend
            .link
            .replies
            .push_back(refresh(vec![mute], Some("Player#0042")));
        backend.poll();
        assert_eq!(backend.sanctions(addr), vec![mute]);
        assert_eq!(backend.profile(addr).unwrap().nickname, "Player#0042");
        assert!(backend.authenticated_session(addr).is_some());
        // An expired sanction is gone from the view at once, a lifted one at
        // the next refresh.
        backend.clients.get_mut(&addr).unwrap().view.sanctions = vec![Sanction {
            kind: SanctionKind::RankedBan,
            until: Some(unix_seconds() - 1),
        }];
        assert!(backend.view(addr).sanctions.is_empty());
        assert!(backend.sanctions(addr).is_empty());
        backend.link.replies.push_back(refresh(vec![mute], None));
        backend.poll();
        backend.link.replies.push_back(refresh(Vec::new(), None));
        backend.poll();
        assert!(backend.sanctions(addr).is_empty());
        // A database outage keeps what was last read.
        backend.link.replies.push_back(refresh(vec![mute], None));
        backend.poll();
        backend.link.replies.push_back(Reply::RefreshAccess {
            profile: None,
            completed_public_matches: None,
            addr,
            nonce: "b".repeat(64),
            key_active: None,
            supporter: None,
            nickname: None,
            sanctions: None,
            play_cooldown: None,
        });
        backend.poll();
        assert_eq!(backend.sanctions(addr), vec![mute]);
        // An account ban applied while signed in ends the session at once.
        backend.link.replies.push_back(refresh(
            vec![Sanction {
                kind: SanctionKind::AccountBan,
                until: None,
            }],
            None,
        ));
        backend.poll();
        assert!(backend.authenticated_session(addr).is_none());
        assert!(backend.profile(addr).is_none());
        assert_eq!(
            backend.view(addr).error_code.as_deref(),
            Some(shared::career::error_code::ACCOUNT_BANNED)
        );
        // The chat switch as the worker last read it.
        assert_eq!(backend.chat_switch(), None);
        backend.link.replies.push_back(Reply::ChatSwitch(false));
        backend.poll();
        assert_eq!(backend.chat_switch(), Some(false));
    }

    #[test]
    fn supporter_is_denied_for_free_expired_disabled_stale_and_unauthenticated_profiles() {
        use shared::supporter::{AuraStyle, SupporterStatus};
        let mut status = SupporterStatus {
            active: true,
            active_until: Some(101),
            equipped_aura: Some(AuraStyle::Solar),
            ..Default::default()
        };
        assert_eq!(
            authorized_supporter_aura(&status, 100),
            Some(AuraStyle::Solar)
        );
        assert_eq!(authorized_supporter_aura(&status, 101), None);
        status.active = false;
        assert_eq!(authorized_supporter_aura(&status, 100), None);
        status.active = true;
        status.equipped_aura = None;
        assert_eq!(authorized_supporter_aura(&status, 100), None);
        let addr = "127.0.0.1:30020".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Tester".into()),
            "session",
        );
        let c = backend.clients.get_mut(&addr).unwrap();
        c.view.supporter = Some(SupporterStatus {
            active: true,
            active_until: Some(unix_seconds() + 60),
            equipped_aura: Some(AuraStyle::Verdant),
            ..Default::default()
        });
        c.supporter_checked = Some(Instant::now());
        assert_eq!(backend.supporter_aura(addr), Some(AuraStyle::Verdant));
        backend.clients.get_mut(&addr).unwrap().supporter_checked =
            Some(Instant::now() - COSMETIC_CACHE_TTL);
        assert_eq!(backend.supporter_aura(addr), None);
        let c = backend.clients.get_mut(&addr).unwrap();
        c.supporter_checked = Some(Instant::now());
        c.auth = None;
        assert_eq!(backend.supporter_aura(addr), None);
    }

    #[test]
    fn tier_auras_need_points_and_end_with_the_tier() {
        use shared::supporter::{AuraStyle, SupporterGrantSummary, SupporterStatus};
        let grant = |until: i64, points: u32| SupporterGrantSummary {
            provider: "yookassa".into(),
            valid_from: 0,
            valid_until: until,
            revoked: false,
            renewal_enabled: Some(false),
            tier_points: points,
        };
        // 1000 ₽ this month: Crimson; Prism I (2500) is refused even if a stale
        // or forged status names it.
        let crimson = SupporterStatus::from_grants(
            vec![grant(200, 1000)],
            100,
            1000,
            Some(Some(AuraStyle::Crimson)),
        );
        assert_eq!(
            authorized_supporter_aura(&crimson, 100),
            Some(AuraStyle::Crimson)
        );
        let forged = SupporterStatus {
            equipped_aura: Some(AuraStyle::PrismI),
            ..crimson.clone()
        };
        assert_eq!(authorized_supporter_aura(&forged, 100), None);
        // Expiry removes the aura without waiting for a refresh.
        assert_eq!(authorized_supporter_aura(&crimson, 200), None);
        // A Solana grant (0 points) keeps base styles only.
        let solana =
            SupporterStatus::from_grants(vec![grant(200, 0)], 100, 0, Some(Some(AuraStyle::Rose)));
        assert_eq!(solana.equipped_aura, Some(AuraStyle::Solar));
        assert_eq!(
            authorized_supporter_aura(&solana, 100),
            Some(AuraStyle::Solar)
        );
        let addr = "127.0.0.1:30024".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Tester".into()),
            "session",
        );
        let now = unix_seconds();
        let c = backend.clients.get_mut(&addr).unwrap();
        c.view.supporter = Some(SupporterStatus::from_grants(
            vec![grant(now + 60, 1000)],
            now,
            1000,
            Some(Some(AuraStyle::Crimson)),
        ));
        c.supporter_checked = Some(Instant::now());
        assert_eq!(backend.supporter_aura(addr), Some(AuraStyle::Crimson));
        // The next refresh reports the grant gone: the aura goes with it.
        let nonce = backend.clients[&addr].auth.as_ref().unwrap().nonce.clone();
        backend.link.replies.push_back(Reply::RefreshAccess {
            profile: None,
            completed_public_matches: None,
            addr,
            nonce,
            key_active: Some(true),
            supporter: Some(SupporterStatus::default()),
            nickname: None,
            sanctions: None,
            play_cooldown: None,
        });
        backend.poll();
        assert_eq!(backend.supporter_aura(addr), None);
    }

    #[test]
    fn revoked_key_refresh_clears_cached_authority_and_cannot_resurrect_via_profile_ready() {
        let addr = "127.0.0.1:30021".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Tester".into()),
            "session",
        );
        backend.link.replies.push_back(Reply::RefreshAccess {
            profile: None,
            completed_public_matches: None,
            addr,
            nonce: "b".repeat(64),
            key_active: Some(false),
            supporter: None,
            nickname: None,
            sanctions: None,
            play_cooldown: None,
        });
        backend.poll();
        assert!(backend.authenticated_session(addr).is_none());
        assert!(backend.profile(addr).is_none());
        assert!(backend.supporter_aura(addr).is_none());
        backend.clients.get_mut(&addr).unwrap().profile_ready = true;
        assert!(backend.profile(addr).is_none());
    }

    #[test]
    fn missing_key_refresh_never_extends_authority_during_a_database_outage() {
        let addr = "127.0.0.1:30022".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Tester".into()),
            "session",
        );
        let expired = Instant::now() - COSMETIC_CACHE_TTL;
        backend.clients.get_mut(&addr).unwrap().key_checked = Some(expired);
        backend.link.replies.push_back(Reply::RefreshAccess {
            profile: None,
            completed_public_matches: None,
            addr,
            nonce: "b".repeat(64),
            key_active: None,
            supporter: None,
            nickname: None,
            sanctions: None,
            play_cooldown: None,
        });
        backend.poll();
        assert_eq!(backend.clients[&addr].key_checked, Some(expired));
        assert!(backend.clients[&addr].auth.is_none());
        assert!(backend.authenticated_session(addr).is_none());
        assert!(backend.profile(addr).is_none());
    }

    #[test]
    fn unsigned_cosmetic_equip_never_reaches_persisted_preferences() {
        let addr = "127.0.0.1:30023".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Tester".into()),
            "session",
        );
        backend.handle(
            addr,
            CareerRequest::EquipSupporterAura {
                request_id: 1,
                aura: Some(shared::supporter::AuraStyle::Solar),
            },
        );
        assert!(backend.link.jobs.is_empty());
        assert_eq!(backend.supporter_aura(addr), None);
        assert!(
            backend
                .view(addr)
                .error
                .unwrap()
                .contains("valid signature")
        );
    }

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
    fn outbox_result(id: &str, outcome: MatchOutcome) -> MatchResult {
        MatchResult {
            ranked_season_id: None,
            gameplay_started: true,
            end_reason: None,
            result_id: id.into(),
            server_epoch: 1,
            match_id: 1,
            started_at_ms: 1_000,
            ended_at_ms: 2_000,
            duration_ms: 1_000,
            map_profile: "test".into(),
            ruleset: "test-ranked".into(),
            outcome,
            winner: Some(shared::map::Team::Green),
            rated: true,
            unrated_reason: None,
            participants: Vec::new(),
            saved: false,
        }
    }

    fn outbox_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omoba-outbox-unit-{}", random_id::<16>()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// O29: a missing outbox is empty; a terminal receipt is kept as is; a
    /// live one becomes an unrated Interrupted settle carrying its allocation;
    /// a rejected id is remembered; unreadable and non-json files are left
    /// on disk and skipped.
    #[test]
    fn recover_outbox_restores_terminal_interrupts_live_and_skips_junk() {
        let missing = std::env::temp_dir().join(format!("omoba-no-outbox-{}", random_id::<16>()));
        let empty = recover_outbox(&missing);
        assert!(empty.pending.is_empty() && empty.rejected_ids.is_empty());

        let dir = outbox_dir();
        let record = |kind, id: &str, outcome| PendingRecord {
            kind,
            result: outbox_result(id, outcome),
            recovery_allocation: None,
            recovered_live: false,
        };
        write_record(
            &dir,
            &record(RecordKind::Settle, "settled", MatchOutcome::Completed),
        )
        .unwrap();
        write_record(
            &dir,
            &record(RecordKind::Checkpoint, "live", MatchOutcome::Completed),
        )
        .unwrap();
        write_record(
            &dir,
            &record(RecordKind::Start, "started", MatchOutcome::Completed),
        )
        .unwrap();
        write_record(
            &dir,
            &record(RecordKind::Rejected, "refused", MatchOutcome::Completed),
        )
        .unwrap();
        fs::write(dir.join("broken.json"), b"{not json").unwrap();
        fs::write(dir.join("half.tmp"), b"{}").unwrap();

        let recovered = recover_outbox(&dir);
        assert_eq!(
            recovered.pending.keys().collect::<Vec<_>>(),
            vec!["live", "settled", "started"]
        );
        assert_eq!(
            recovered.rejected_ids,
            VecDeque::from(["refused".to_string()])
        );

        let settled = &recovered.pending["settled"];
        assert!(matches!(settled.kind, RecordKind::Settle));
        assert!(!settled.recovered_live);
        assert_eq!(
            settled.result,
            outbox_result("settled", MatchOutcome::Completed)
        );

        for id in ["live", "started"] {
            let live = &recovered.pending[id];
            assert!(matches!(live.kind, RecordKind::Settle), "{id}");
            assert!(live.recovered_live);
            assert_eq!(live.result.outcome, MatchOutcome::Interrupted);
            assert_eq!(live.result.winner, None);
            assert!(!live.result.rated);
            assert_eq!(
                live.result.unrated_reason.as_deref(),
                Some("server_interrupted")
            );
            assert!(live.result.ended_at_ms > 2_000);
            assert_eq!(
                live.result.duration_ms,
                live.result.ended_at_ms - live.result.started_at_ms
            );
            assert_eq!(
                live.recovery_allocation,
                Some(outbox_result(id, MatchOutcome::Completed)),
                "the allocation is the receipt as written"
            );
        }
        assert!(
            dir.join("broken.json").exists(),
            "unreadable receipts stay on disk"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// O29: a live receipt that already carries its allocation keeps it.
    #[test]
    fn recover_outbox_keeps_an_existing_recovery_allocation() {
        let dir = outbox_dir();
        let allocation = outbox_result("carried", MatchOutcome::Interrupted);
        write_record(
            &dir,
            &PendingRecord {
                kind: RecordKind::Checkpoint,
                result: outbox_result("carried", MatchOutcome::Completed),
                recovery_allocation: Some(allocation.clone()),
                recovered_live: false,
            },
        )
        .unwrap();
        let recovered = recover_outbox(&dir);
        assert_eq!(
            recovered.pending["carried"].recovery_allocation,
            Some(allocation)
        );
        fs::remove_dir_all(&dir).unwrap();
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
        let mut backend = MemoryCareer::test_backend(9);
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
        let mut backend = MemoryCareer::test_backend(9);
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
        assert!(backend.link.jobs.is_empty());
    }

    #[test]
    fn late_duplicate_login_ack_cannot_reset_sequence() {
        let addr = "127.0.0.1:30002".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Player".into()),
            "session",
        );
        let c = backend.clients.get_mut(&addr).unwrap();
        let challenge = c.auth.clone().unwrap();
        c.challenge = Some((challenge.clone(), Instant::now()));
        c.sequence = 42;
        let login = Reply::Login {
            addr,
            challenge,
            result: Ok(c.view.clone()),
        };
        backend.link.replies.push_back(login);
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
                ranked_season_id: None,
                gameplay_started: true,
                end_reason: None,
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
                    input_kind: Default::default(),
                    participation: None,
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
    #[test]
    fn ready_answers_require_a_signature_and_bind_check_identity_and_acceptance() {
        let addr = "127.0.0.1:30009".parse().unwrap();
        let mut backend = MemoryCareer::test_backend(9);
        backend.test_authenticated(
            addr,
            ProfileSummary::new("a".repeat(64), "Player".into()),
            "session",
        );
        let key = SigningKey::from_bytes(&[7; 32]);
        let nonce = "b".repeat(64);
        let action = CareerAction::ReadyMatch {
            request_id: 7,
            check_id: "c".repeat(32),
            accepted: true,
        };
        let signature = hex(&key
            .sign(&authorized_signing_bytes(9, &nonce, 1, &action))
            .to_bytes());
        backend.handle(
            addr,
            CareerRequest::ReadyMatch {
                request_id: 7,
                check_id: "c".repeat(32),
                accepted: true,
            },
        );
        assert!(backend.take_ready_requests().is_empty());
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(
            addr,
            CareerRequest::Authorized {
                session_nonce: nonce.clone(),
                sequence: 1,
                action: CareerAction::ReadyMatch {
                    request_id: 7,
                    check_id: "d".repeat(32),
                    accepted: true,
                },
                signature: signature.clone(),
            },
        );
        assert!(backend.take_ready_requests().is_empty());
        let valid = CareerRequest::Authorized {
            session_nonce: nonce,
            sequence: 1,
            action,
            signature,
        };
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, valid.clone());
        let answers = backend.take_ready_requests();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].request_id, 7);
        assert!(answers[0].accepted);
        backend.clients.get_mut(&addr).unwrap().last_request = None;
        backend.handle(addr, valid);
        assert!(backend.take_ready_requests().is_empty());
    }
    #[test]
    fn flush_barrier_requires_an_ack_and_is_invalidated_by_a_later_record() {
        let mut backend = MemoryCareer::immediate(9);
        assert!(!backend.flush_outbox());
        backend.poll();
        assert!(backend.flush_outbox());
        let result = outbox_result("flush-order", MatchOutcome::Interrupted);
        assert!(backend.start(result));
        assert!(
            !backend.flush_outbox(),
            "a previous barrier cannot cover a later record"
        );
        backend.poll();
        assert!(backend.flush_outbox());
    }
}

#[cfg(test)]
#[path = "career_backend_resilience_tests.rs"]
mod resilience_tests;

#[cfg(test)]
#[path = "career_refresh_tests.rs"]
mod refresh_tests;

#[cfg(test)]
#[path = "career_dodge_tests.rs"]
mod dodge_tests;
