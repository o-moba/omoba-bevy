//! Bounded same-host worker pool. Children outlive a lobby restart; manifests do not.
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{fs, io};

use crate::match_allocation::{self, Manifest, Phase, Status};

pub(crate) struct Slot {
    pub manifest: Manifest,
    pub status: Option<Status>,
    directory: PathBuf,
    child: Option<Child>,
    /// This lobby reaped this exact child; a terminal receipt can release its
    /// socket immediately. Adopted processes have no such proof.
    child_exit_observed: bool,
    recovering: bool,
    /// The lobby saw this match running (LR-13). Its later failure ends a
    /// played match; it is not a cancelled or expired allocation.
    ran: bool,
    /// `cancel.json` exists: written by this lobby or by the worker when an
    /// allocated human left before the match started. Refreshed by `poll`.
    cancelled: bool,
    cancelled_sessions: Vec<String>,
    /// When this lobby spawned the child; `None` for an adopted worker.
    spawned_at: Option<Instant>,
}

/// A spawned worker that wrote no status at all within this long is wedged
/// (a blocked database connect, a bad environment): the lobby kills it so the
/// slot fails instead of holding ten players on "preparing the arena".
const FIRST_STATUS_DEADLINE: Duration = Duration::from_secs(20);
/// Grace after the manifest's join deadline before the lobby kills a worker
/// that never ran: the worker aborts itself at the deadline, this is the backstop.
const JOIN_DEADLINE_GRACE_MS: u64 = 30_000;
pub(crate) struct Pool {
    // Kernel-owned advisory lock is released on crash as well as normal drop.
    lock: fs::File,
    pub slots: HashMap<String, Slot>,
    root: PathBuf,
    executable: PathBuf,
    capacity: usize,
    bind_ip: String,
    public_host: String,
    first_port: u16,
    test_map: Option<PathBuf>,
}
fn setting<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    std::env::var(name).map_or(Ok(default), |v| {
        v.parse().map_err(|_| format!("Invalid {name}"))
    })
}
/// The host part of a worker endpoint, as an IP address when it is one.
fn host_ip(host: &str) -> Option<std::net::IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

fn loopback_host(host: &str) -> bool {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    host_ip(host).is_some_and(|ip| ip.is_loopback())
        || name == "localhost"
        || name.ends_with(".localhost")
}

/// The host clients dial for a worker (`OMOBA_MATCH_PUBLIC_HOST`, unset =
/// `127.0.0.1`). A lobby or its workers bound beyond loopback serve remote
/// players, and a loopback endpoint would send those players to their own
/// machine: such a lobby refuses to start instead of failing every match.
pub(crate) fn public_worker_host(
    configured: Option<&str>,
    lobby_bind: SocketAddr,
    worker_bind: std::net::IpAddr,
) -> Result<String, String> {
    let public_host = configured.unwrap_or("127.0.0.1");
    if public_host.is_empty()
        || public_host.len() > 200
        || public_host.contains(['/', '\n', '\r', ' '])
    {
        return Err("Invalid public worker host".into());
    }
    if host_ip(public_host).is_some_and(|ip| ip.is_unspecified()) {
        return Err(format!(
            "OMOBA_MATCH_PUBLIC_HOST={public_host} is not an address players can dial; \
             set it to this host's public IPv4 address or DNS name"
        ));
    }
    let exposed = !lobby_bind.ip().is_loopback() || !worker_bind.is_loopback();
    if exposed && loopback_host(public_host) {
        let shown =
            configured.map_or_else(|| "unset (default 127.0.0.1)".to_owned(), str::to_owned);
        return Err(format!(
            "OMOBA_MATCH_PUBLIC_HOST is {shown}, but the lobby binds {lobby_bind} and workers \
             bind {worker_bind}: remote players would be sent to their own loopback address. \
             Set OMOBA_MATCH_PUBLIC_HOST to this host's public IPv4 address or DNS name, \
             or bind SERVER_ADDR and OMOBA_MATCH_BIND_IP to loopback for local play"
        ));
    }
    Ok(public_host.to_owned())
}

/// Explicit rehearsal override, allowed only when both UDP listeners are local.
/// Ordinary OMOBA_MAP_CONFIG is always stripped from allocated workers.
fn loopback_test_map(
    configured: Option<PathBuf>,
    lobby_bind: SocketAddr,
    worker_bind: std::net::IpAddr,
) -> Option<PathBuf> {
    configured.filter(|_| lobby_bind.ip().is_loopback() && worker_bind.is_loopback())
}

impl Pool {
    /// `lobby_bind` is the lobby's own bound UDP address (`SERVER_ADDR`).
    pub fn from_env(lobby_bind: SocketAddr) -> Result<Self, String> {
        let root = std::env::var_os("OMOBA_MATCH_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".omoba/matches"));
        fs::create_dir_all(&root).map_err(|_| "Cannot create match root")?;
        let root = fs::canonicalize(root).map_err(|_| "Cannot resolve match root")?;
        let lock = lock_root(&root)?;
        let capacity = setting("OMOBA_MATCH_CAPACITY", 16_usize)?;
        let first_port = setting("OMOBA_MATCH_FIRST_PORT", 41000_u16)?;
        if !(1..=100).contains(&capacity)
            || first_port == 0
            || first_port as usize + capacity > 65535
        {
            return Err("Invalid match pool capacity or ports".into());
        }
        let bind_ip = std::env::var("OMOBA_MATCH_BIND_IP").unwrap_or_else(|_| "127.0.0.1".into());
        let worker_bind = format!("{bind_ip}:{first_port}")
            .parse::<SocketAddr>()
            .map_err(|_| "Invalid worker bind IP")?;
        let public_host = public_worker_host(
            std::env::var("OMOBA_MATCH_PUBLIC_HOST").ok().as_deref(),
            lobby_bind,
            worker_bind.ip(),
        )?;
        let executable = std::env::var_os("OMOBA_MATCH_EXECUTABLE")
            .map(PathBuf::from)
            .map_or_else(
                || std::env::current_exe().map_err(|_| "Cannot find worker executable".to_string()),
                Ok,
            )?;
        let configured_test_map =
            std::env::var_os("OMOBA_MATCH_TEST_MAP_CONFIG").map(PathBuf::from);
        let requested = configured_test_map.is_some();
        let test_map = loopback_test_map(configured_test_map, lobby_bind, worker_bind.ip())
            .map(fs::canonicalize)
            .transpose()
            .map_err(|_| "Cannot resolve loopback rehearsal map")?;
        if requested && test_map.is_none() {
            eprintln!(
                "Ignoring OMOBA_MATCH_TEST_MAP_CONFIG: lobby and workers must both bind loopback"
            );
        }
        let mut pool = Self {
            lock,
            slots: HashMap::new(),
            root,
            executable,
            capacity,
            bind_ip,
            public_host,
            first_port,
            test_map,
        };
        pool.adopt_existing()?;
        Ok(pool)
    }

    fn adopt_existing(&mut self) -> Result<(), String> {
        let entries = fs::read_dir(&self.root).map_err(|_| "Cannot inspect workers")?;
        for (index, entry) in entries.flatten().take(4097).enumerate() {
            if index == 4096 {
                return Err("Worker directory limit reached; archive completed allocations before starting the lobby".into());
            }
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let directory = entry.path();
            let Ok(manifest) = Manifest::read(&directory.join("manifest.json")) else {
                continue;
            };
            let status = read_status(&directory, &manifest.allocation_id);
            if status.as_ref().is_some_and(|s| {
                matches!(s.phase, Phase::Finished | Phase::Failed)
                    && match_allocation::unix_ms().saturating_sub(s.heartbeat_ms) > 35_000
            }) {
                archive(&self.root, &directory, &manifest.allocation_id);
                continue;
            }
            let ran = status.as_ref().is_some_and(Status::played);
            let cancelled = directory.join("cancel.json").exists();
            self.slots.insert(
                manifest.allocation_id.clone(),
                Slot {
                    manifest,
                    status,
                    directory,
                    child: None,
                    child_exit_observed: false,
                    recovering: false,
                    ran,
                    cancelled,
                    cancelled_sessions: Vec::new(),
                    spawned_at: None,
                },
            );
        }
        if self.occupied() > 100 {
            return Err("Too many occupied worker ports".into());
        }
        Ok(())
    }
    pub fn full(&self) -> bool {
        self.occupied() >= self.capacity
    }
    /// Live arenas and recently retired adopted workers whose exit is unknown.
    pub fn occupied(&self) -> usize {
        self.slots
            .values()
            .filter(|s| s.occupies_port())
            .map(|s| s.manifest.bind.as_str())
            .collect::<HashSet<_>>()
            .len()
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// The match root (`OMOBA_MATCH_ROOT`), where the lobby also reports its
    /// capacity (LR-09).
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
    pub fn allocate(&mut self, mut manifest: Manifest) -> Result<String, String> {
        if self.full() {
            return Err("capacity_busy".into());
        }
        let port = (self.first_port..self.first_port + self.capacity as u16)
            .find(|port| {
                !self.slots.values().any(|s| {
                    s.occupies_port()
                        && s.manifest
                            .bind
                            .parse::<SocketAddr>()
                            .is_ok_and(|a| a.port() == *port)
                })
            })
            .ok_or("capacity_busy")?;
        manifest.bind = format!("{}:{port}", self.bind_ip);
        manifest.endpoint = format!("{}:{port}", self.public_host);
        manifest.validate()?;
        let directory = self.root.join(&manifest.allocation_id);
        fs::create_dir(&directory).map_err(|_| "Cannot create worker directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect worker directory")?;
        }
        match_allocation::atomic_json(&directory.join("manifest.json"), &manifest)
            .map_err(|_| "Cannot persist allocation")?;
        let child = match spawn(
            &self.executable,
            &directory,
            &manifest,
            false,
            self.test_map.as_deref(),
        ) {
            Ok(child) => child,
            Err(_) => {
                let status = Status {
                    released_sessions: Vec::new(),
                    allocation_id: manifest.allocation_id.clone(),
                    server_epoch: 0,
                    heartbeat_ms: match_allocation::unix_ms(),
                    phase: Phase::Failed,
                    result_id: None,
                    ends_by_ms: None,
                    joined_sessions: Vec::new(),
                };
                let _ = match_allocation::atomic_json(&directory.join("status.json"), &status);
                // Nothing will ever run here: keep the failed receipt in the
                // archive instead of a dead directory in the live root.
                archive(&self.root, &directory, &manifest.allocation_id);
                return Err("worker_spawn_failed".into());
            }
        };
        let id = manifest.allocation_id.clone();
        self.slots.insert(
            id.clone(),
            Slot {
                manifest,
                status: None,
                directory,
                child: Some(child),
                child_exit_observed: false,
                recovering: false,
                ran: false,
                cancelled: false,
                cancelled_sessions: Vec::new(),
                spawned_at: Some(Instant::now()),
            },
        );
        Ok(id)
    }
    pub fn request_release(&mut self, id: &str, profile: &str) -> bool {
        let Some(slot) = self.slots.get_mut(id) else {
            return false;
        };
        slot.refresh_status();
        if slot.terminal()
            || slot.ran()
            || slot
                .status
                .as_ref()
                .is_some_and(|s| matches!(s.phase, Phase::Running | Phase::Settling))
        {
            return false;
        }
        match_allocation::releases::request(&slot.directory, &slot.manifest, profile).is_ok()
    }

    pub fn cancel(&mut self, id: &str) -> bool {
        let Some(slot) = self.slots.get_mut(id) else {
            return true;
        };
        slot.refresh_status();
        if slot.terminal()
            || slot
                .status
                .as_ref()
                .is_some_and(|s| matches!(s.phase, Phase::Running | Phase::Settling))
        {
            return false;
        }
        let written =
            match_allocation::atomic_json(&slot.directory.join("cancel.json"), &true).is_ok();
        slot.cancelled |= written;
        written
    }

    /// The sessions a worker named in its `cancel.json` (allocated humans who
    /// left before the match started), empty for a lobby-written or absent
    /// cancellation. Those humans are not re-queued; the others are.
    pub fn cancelled_sessions(&self, id: &str) -> Vec<String> {
        self.slots
            .get(id)
            .map_or_else(Vec::new, |slot| slot.cancelled_sessions.clone())
    }

    /// One filesystem snapshot per active worker, shared by a whole view batch.
    pub fn refresh_handoffs(&mut self) {
        for slot in self.slots.values_mut().filter(|s| !s.terminal()) {
            slot.refresh_status();
        }
    }

    pub fn poll(&mut self) {
        let now = match_allocation::unix_ms();
        let mut recovery_launches = 0;
        for slot in self.slots.values_mut() {
            slot.refresh_status();
            // Watchdog: a child this lobby spawned that never reported, or
            // that outlived its join deadline without ever running, is killed
            // so the slot fails instead of holding its players and its port.
            let wedged = slot.child.is_some()
                && !slot.terminal()
                && !slot.ran
                && (slot.status.is_none()
                    && slot
                        .spawned_at
                        .is_some_and(|at| at.elapsed() >= FIRST_STATUS_DEADLINE)
                    || slot.status.as_ref().is_some_and(|s| {
                        matches!(s.phase, Phase::Ready | Phase::Forming)
                            && now > slot.manifest.join_deadline_ms + JOIN_DEADLINE_GRACE_MS
                    }));
            if wedged && let Some(child) = slot.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
                slot.status = None;
            }
            let exited = wedged
                || slot
                    .child
                    .as_mut()
                    .is_some_and(|child| child.try_wait().is_ok_and(|r| r.is_some()));
            if exited {
                slot.child = None;
                slot.child_exit_observed = true;
                if slot.status.is_none() {
                    let status = Status {
                        released_sessions: Vec::new(),
                        allocation_id: slot.manifest.allocation_id.clone(),
                        server_epoch: 0,
                        heartbeat_ms: now,
                        phase: Phase::Failed,
                        result_id: None,
                        ends_by_ms: None,
                        joined_sessions: Vec::new(),
                    };
                    if match_allocation::atomic_json(&slot.directory.join("status.json"), &status)
                        .is_ok()
                    {
                        slot.status = Some(status);
                    }
                }
            }
            if slot.terminal() {
                continue;
            }
            let stale = slot
                .status
                .as_ref()
                .map_or(now > slot.manifest.join_deadline_ms + 100_000, |s| {
                    now > s.heartbeat_ms + 100_000
                });
            // A stale heartbeat alone is not authority to duplicate a known live child.
            if slot.child.is_none() && (exited || stale) && recovery_launches < 4 {
                // An adopted worker may still own its socket despite a delayed
                // disk heartbeat. Never launch a duplicate just for stale disk state.
                let Ok(probe) = std::net::UdpSocket::bind(&slot.manifest.bind) else {
                    continue;
                };
                drop(probe);
                recovery_launches += 1;
                if let Ok(child) = spawn(
                    &self.executable,
                    &slot.directory,
                    &slot.manifest,
                    true,
                    self.test_map.as_deref(),
                ) {
                    slot.child = Some(child);
                    slot.child_exit_observed = false;
                    slot.recovering = true;
                }
            }
        }
        // Bound retained in-memory terminal receipts. Durable manifests/results remain on disk.
        self.slots.retain(|id, s| {
            let keep = !s.terminal()
                || s.status
                    .as_ref()
                    .is_none_or(|t| now < t.heartbeat_ms + 120_000)
                || s.child.is_some();
            if !keep {
                archive(&self.root, &s.directory, id);
            }
            keep
        });
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        // `flock` belongs to the open file description, not to this fd. A child
        // forked by another thread holds a duplicate of that description until
        // its `exec` closes it (`O_CLOEXEC`), so closing our fd alone can leave
        // the root locked for that window. Unlock explicitly: `LOCK_UN` drops
        // the lock on the description whatever other fds still refer to it.
        let _ = self.lock.unlock();
    }
}
impl Slot {
    pub fn refresh_status(&mut self) {
        if let Some(next) = read_status(&self.directory, &self.manifest.allocation_id) {
            let advances = match self.status.as_ref().map(|s| &s.phase) {
                Some(Phase::Finished | Phase::Failed) => {
                    matches!(next.phase, Phase::Finished | Phase::Failed)
                }
                Some(Phase::Settling) => matches!(
                    next.phase,
                    Phase::Settling | Phase::Finished | Phase::Failed
                ),
                _ => true,
            };
            // A stale writer cannot step back through Recovering and then Ready.
            if advances {
                self.status = Some(next);
            }
        }
        self.ran |= self.status.as_ref().is_some_and(Status::played);
        if let Ok(bytes) = fs::read(self.directory.join("cancel.json")) {
            self.cancelled = true;
            if bytes.len() <= 4096
                && let Ok(sessions) = serde_json::from_slice::<Vec<String>>(&bytes)
            {
                self.cancelled_sessions = sessions;
            }
        }
    }

    /// The lobby saw this match running (LR-13).
    pub fn ran(&self) -> bool {
        self.ran
    }

    pub fn occupies_port(&self) -> bool {
        self.child.is_some()
            || !self.terminal()
            || (!self.child_exit_observed
                && self.status.as_ref().is_some_and(|s| {
                    match_allocation::unix_ms().saturating_sub(s.heartbeat_ms) <= 35_000
                }))
    }
    pub fn cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn terminal(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|s| matches!(s.phase, Phase::Finished | Phase::Failed))
    }
    #[cfg(test)]
    pub fn ready(&self) -> bool {
        self.ready_status().is_some()
    }

    /// The worker's latest receipt while the slot is ready: not recovering or
    /// cancelled, `ready`, `forming` or `running`, with a fresh heartbeat. Reads
    /// the shared snapshot: the send path refreshes it once for all players
    /// before a due batch; new enqueue/cancel also refresh at admission.
    pub fn ready_status(&self) -> Option<Status> {
        if self.recovering || self.cancelled() {
            return None;
        }
        self.status.clone().filter(|s| {
            matches!(s.phase, Phase::Ready | Phase::Forming | Phase::Running)
                && match_allocation::unix_ms().saturating_sub(s.heartbeat_ms) < 10_000
        })
    }
}
fn read_status(dir: &std::path::Path, id: &str) -> Option<Status> {
    let bytes = fs::read(dir.join("status.json")).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let s: Status = serde_json::from_slice(&bytes).ok()?;
    (s.allocation_id == id).then_some(s)
}
fn spawn(
    executable: &std::path::Path,
    directory: &std::path::Path,
    m: &Manifest,
    recovery: bool,
    test_map: Option<&std::path::Path>,
) -> io::Result<Child> {
    let output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("worker.log"))?;
    worker_command(executable, directory, m, recovery, test_map)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .spawn()
}

/// A worker inherits the lobby's environment except its own settings and the
/// local-only overrides: a map file, targeting QA, and the practice bot level,
/// so an operator setting on the lobby host can never weaken online bots.
/// The explicit test map has already passed the pool's two-loopback guard;
/// altered map stats remain ineligible for Ranked credit.
fn worker_command(
    executable: &std::path::Path,
    directory: &std::path::Path,
    m: &Manifest,
    recovery: bool,
    test_map: Option<&std::path::Path>,
) -> Command {
    let mut command = Command::new(executable);
    command
        .env("OMOBA_SERVER_ROLE", "match")
        .env("OMOBA_MATCH_ALLOCATION", directory.join("manifest.json"))
        .env("OMOBA_MATCH_RECOVERY", if recovery { "1" } else { "0" })
        .env("OMOBA_CAREER_OUTBOX", directory.join("outbox"))
        .env("SERVER_ADDR", &m.bind)
        .env("OMOBA_TEAM_SIZE", "5")
        .env("OMOBA_MATCH_MODE", m.match_mode().id())
        .env_remove("OMOBA_MAP_CONFIG")
        .env_remove("OMOBA_MATCH_TEST_MAP_CONFIG")
        .env_remove("OMOBA_TARGETING_QA")
        .env_remove(shared::bot_profile::BotDifficulty::ENV);
    if let Some(path) = test_map
        && m.bind
            .parse::<SocketAddr>()
            .is_ok_and(|addr| addr.ip().is_loopback())
    {
        command.env("OMOBA_MAP_CONFIG", path);
    }
    command
}

fn lock_root(root: &std::path::Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(root.join(".coordinator.lock"))
        .map_err(|_| "Cannot open coordinator lock")?;
    file.try_lock().map_err(
        |_| "Another lobby already owns this match root (or its filesystem cannot lock)",
    )?;
    Ok(file)
}

fn archive(root: &std::path::Path, directory: &std::path::Path, id: &str) {
    let target = root.join("archive");
    if fs::create_dir_all(&target).is_ok() {
        let _ = fs::rename(directory, target.join(id));
    }
}

#[cfg(test)]
impl Pool {
    pub(crate) fn write_status_for_test(&self, id: &str, status: &Status) {
        match_allocation::atomic_json(&self.slots[id].directory.join("status.json"), status)
            .unwrap();
    }

    /// The slot as a restarted lobby adopts it: the worker lives on, but
    /// this lobby is not its parent. Reaps the stand-in test child.
    pub(crate) fn adopt_for_test(&mut self, id: &str) {
        if let Some(mut child) = self.slots.get_mut(id).unwrap().child.take() {
            child.wait().unwrap();
        }
    }

    pub(crate) fn test_directory(&self) -> &std::path::Path {
        &self.root
    }

    pub(crate) fn set_capacity_for_test(&mut self, capacity: usize) {
        self.capacity = capacity;
    }

    pub(crate) fn for_test() -> Pool {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "omoba-pool-test-{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ));
        fs::create_dir(&root).unwrap();
        Pool {
            lock: lock_root(&root).unwrap(),
            slots: HashMap::new(),
            root,
            executable: PathBuf::from("/usr/bin/true"),
            capacity: 1,
            bind_ip: "127.0.0.1".into(),
            public_host: "127.0.0.1".into(),
            first_port: 45000,
            test_map: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::match_service::MatchPreference;

    use super::*;
    use crate::match_allocation::AllocatedHuman;
    fn manifest(id: char) -> Manifest {
        Manifest {
            ranked_season_id: None,
            version: 1,
            allocation_id: id.to_string().repeat(32),
            endpoint: String::new(),
            bind: String::new(),
            preference: MatchPreference::BotPractice,
            humans: vec![AllocatedHuman {
                profile_id: "a".repeat(64),
                session_id: "s1".into(),
                team: shared::map::Team::Green,
            }],
            join_deadline_ms: match_allocation::unix_ms() + 180_000,
            cohort: None,
        }
    }
    #[test]
    fn workers_never_inherit_local_overrides_such_as_the_bot_level() {
        let command = worker_command(
            std::path::Path::new("omoba-server"),
            std::path::Path::new("/tmp/allocation"),
            &manifest('a'),
            false,
            None,
        );
        let envs: HashMap<_, _> = command.get_envs().collect();
        for removed in [
            "OMOBA_MAP_CONFIG",
            "OMOBA_MATCH_TEST_MAP_CONFIG",
            "OMOBA_TARGETING_QA",
            shared::bot_profile::BotDifficulty::ENV,
        ] {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(removed)),
                Some(&None),
                "{removed} must be removed"
            );
        }
        assert_eq!(
            envs.get(std::ffi::OsStr::new("OMOBA_SERVER_ROLE")),
            Some(&Some(std::ffi::OsStr::new("match")))
        );
        assert_eq!(
            envs.get(std::ffi::OsStr::new("OMOBA_MATCH_MODE")),
            Some(&Some(std::ffi::OsStr::new("practice")))
        );
    }

    #[test]
    fn rehearsal_map_requires_loopback_lobby_and_workers_including_ipv6() {
        let path = PathBuf::from("/tmp/rehearsal-map.json");
        for lobby in [
            "127.0.0.1:4000",
            "[::1]:4000",
            "0.0.0.0:4000",
            "[::]:4000",
            "192.0.2.1:4000",
        ] {
            for worker in ["127.0.0.1", "::1", "0.0.0.0", "::", "192.0.2.1"] {
                let lobby: SocketAddr = lobby.parse().unwrap();
                let worker: std::net::IpAddr = worker.parse().unwrap();
                assert_eq!(
                    loopback_test_map(Some(path.clone()), lobby, worker).is_some(),
                    lobby.ip().is_loopback() && worker.is_loopback()
                );
            }
        }
        let mut m = manifest('a');
        m.bind = "127.0.0.1:41000".into();
        let command = worker_command(
            std::path::Path::new("server"),
            std::path::Path::new("/tmp/allocation"),
            &m,
            false,
            Some(&path),
        );
        let envs: HashMap<_, _> = command.get_envs().collect();
        assert_eq!(
            envs[std::ffi::OsStr::new("OMOBA_MAP_CONFIG")],
            Some(path.as_os_str())
        );
        assert_eq!(
            envs[std::ffi::OsStr::new("OMOBA_MATCH_TEST_MAP_CONFIG")],
            None
        );
        // Even a caller mistake cannot apply a test map to an exposed worker.
        m.bind = "0.0.0.0:41000".into();
        let command = worker_command(
            std::path::Path::new("server"),
            std::path::Path::new("/tmp/allocation"),
            &m,
            false,
            Some(&path),
        );
        let envs: HashMap<_, _> = command.get_envs().collect();
        assert_eq!(envs[std::ffi::OsStr::new("OMOBA_MAP_CONFIG")], None);
    }

    #[test]
    fn only_one_coordinator_can_own_a_root_and_drop_releases_it() {
        let p = Pool::for_test();
        let root = p.root.clone();
        assert!(lock_root(&root).is_err());
        // Stand-in for a child another test thread forked while `p` was alive:
        // it shares the lock's open file description until its `exec`. The
        // lock must still be released by the drop, not by the last close.
        let forked_duplicate = p.lock.try_clone().unwrap();
        drop(p);
        assert!(lock_root(&root).is_ok());
        drop(forked_duplicate);
    }
    #[test]
    fn capacity_and_readiness_require_a_live_worker_receipt() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        assert!(p.full());
        assert!(!p.slots[&id].ready());
        assert_eq!(p.allocate(manifest('b')).unwrap_err(), "capacity_busy");
        p.slots
            .get_mut(&id)
            .unwrap()
            .child
            .as_mut()
            .unwrap()
            .wait()
            .unwrap();
        p.poll();
        assert!(p.slots[&id].terminal());
        assert!(!p.slots[&id].ready());
        assert!(p.slots[&id].child_exit_observed);
        assert!(!p.full(), "reaped terminal child no longer owns its port");
    }
    #[test]
    fn cancellation_blocks_handoff_before_status_update() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        let slot = p.slots.get_mut(&id).unwrap();
        slot.status = Some(Status {
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Ready,
            result_id: None,
            ends_by_ms: None,
            joined_sessions: Vec::new(),
        });
        assert!(p.slots[&id].ready());
        assert!(p.cancel(&id));
        assert!(!p.slots[&id].ready());
        p.slots
            .get_mut(&id)
            .unwrap()
            .child
            .as_mut()
            .unwrap()
            .wait()
            .unwrap();
    }
    #[test]
    fn exposed_lobby_refuses_a_loopback_or_unset_public_worker_host() {
        let public: SocketAddr = "0.0.0.0:4000".parse().unwrap();
        let local: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        let any: std::net::IpAddr = "0.0.0.0".parse().unwrap();
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        // The RU/production shape: public binds need a reachable host.
        for host in [
            None,
            Some("127.0.0.1"),
            Some("127.0.1.1"),
            Some("::1"),
            Some("[::1]"),
            Some("localhost"),
            Some("LOCALHOST."),
            Some("game.localhost"),
        ] {
            for (lobby, workers) in [(public, any), (public, loopback), (local, any)] {
                let error = public_worker_host(host, lobby, workers).unwrap_err();
                assert!(
                    error.contains("OMOBA_MATCH_PUBLIC_HOST") && error.contains("loopback"),
                    "{host:?} {lobby} {workers}: {error}"
                );
            }
        }
        assert!(
            public_worker_host(None, public, any)
                .unwrap_err()
                .contains("unset (default 127.0.0.1)")
        );
        // Never dialable, whatever the binds.
        for host in ["0.0.0.0", "::", "[::]"] {
            assert!(
                public_worker_host(Some(host), local, loopback)
                    .unwrap_err()
                    .contains("not an address players can dial")
            );
        }
        let long = "a".repeat(201);
        for host in ["", "a b", "a/b", long.as_str()] {
            assert_eq!(
                public_worker_host(Some(host), local, loopback).unwrap_err(),
                "Invalid public worker host"
            );
        }
        // Local play keeps the loopback default; public hosts are accepted.
        assert_eq!(
            public_worker_host(None, local, loopback).unwrap(),
            "127.0.0.1"
        );
        for host in ["lobby.omoba.ru", "203.0.113.7", "lobby.dev.omoba.io"] {
            assert_eq!(public_worker_host(Some(host), public, any).unwrap(), host);
        }
    }

    #[test]
    fn reaped_terminal_worker_frees_its_port_immediately() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        let port = p.slots[&id].manifest.bind.clone();
        p.slots
            .get_mut(&id)
            .unwrap()
            .child
            .as_mut()
            .unwrap()
            .wait()
            .unwrap();
        // What an aborted worker (reaped or capped) writes before it exits.
        let retired = |age_ms: u64| Status {
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms() - age_ms,
            phase: Phase::Failed,
            result_id: Some("r".repeat(32)),
            ends_by_ms: None,
            joined_sessions: Vec::new(),
        };
        p.write_status_for_test(&id, &retired(1_000));
        p.poll();
        assert!(p.slots[&id].terminal());
        assert!(p.slots[&id].child_exit_observed);
        assert!(
            !p.full(),
            "the exact child was reaped; its port is free now"
        );
        let next = p.allocate(manifest('b')).unwrap();
        assert_eq!(p.slots[&next].manifest.bind, port);
        p.slots
            .get_mut(&next)
            .unwrap()
            .child
            .as_mut()
            .unwrap()
            .wait()
            .unwrap();
    }

    #[test]
    fn adopted_terminal_worker_keeps_the_heartbeat_hold_without_exit_proof() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        // Simulate a restart: the new lobby has no child handle/exit observation.
        p.slots
            .get_mut(&id)
            .unwrap()
            .child
            .take()
            .unwrap()
            .wait()
            .unwrap();
        let mut status = Status {
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Finished,
            result_id: Some("r".repeat(32)),
            ends_by_ms: None,
            joined_sessions: Vec::new(),
        };
        p.write_status_for_test(&id, &status);
        p.poll();
        assert!(!p.slots[&id].child_exit_observed);
        assert!(
            p.full(),
            "terminal status does not prove an adopted process exited"
        );
        status.heartbeat_ms -= 36_000;
        p.write_status_for_test(&id, &status);
        p.poll();
        assert!(!p.full());
    }

    #[test]
    fn terminal_receipt_never_releases_a_still_live_child() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        p.slots
            .get_mut(&id)
            .unwrap()
            .child
            .take()
            .unwrap()
            .wait()
            .unwrap();
        p.slots.get_mut(&id).unwrap().child =
            Some(Command::new("sleep").arg("30").spawn().unwrap());
        let status = Status {
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms() - 36_000,
            phase: Phase::Finished,
            result_id: Some("r".repeat(32)),
            ends_by_ms: None,
            joined_sessions: Vec::new(),
        };
        p.write_status_for_test(&id, &status);
        p.poll();
        let occupied = p.full();
        let child = p.slots.get_mut(&id).unwrap().child.as_mut().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(
            occupied,
            "an old terminal receipt cannot free a live child's port"
        );
        p.poll();
        assert!(p.slots[&id].child_exit_observed);
        assert!(!p.full());
    }

    #[test]
    fn handoff_views_share_one_receipt_and_observed_settlement_cannot_regress() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        let mut status = Status {
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Ready,
            result_id: None,
            ends_by_ms: None,
            joined_sessions: Vec::new(),
        };
        p.write_status_for_test(&id, &status);
        p.refresh_handoffs();
        assert!(p.slots[&id].ready());
        // A whole send batch uses one snapshot even if the worker writes during it.
        status.phase = Phase::Settling;
        p.write_status_for_test(&id, &status);
        for _ in 0..10 {
            assert!(p.slots[&id].ready());
        }
        p.refresh_handoffs();
        for _ in 0..10 {
            assert!(!p.slots[&id].ready());
        }
        // A stale writer cannot revive a handoff after Settling was observed.
        status.phase = Phase::Ready;
        status.heartbeat_ms += 1;
        p.write_status_for_test(&id, &status);
        p.refresh_handoffs();
        assert_eq!(p.slots[&id].status.as_ref().unwrap().phase, Phase::Settling);
        assert!(!p.slots[&id].ready());
        for phase in [Phase::Recovering, Phase::Ready] {
            status.phase = phase;
            p.write_status_for_test(&id, &status);
            p.refresh_handoffs();
            assert_eq!(p.slots[&id].status.as_ref().unwrap().phase, Phase::Settling);
        }
        // Cancellation sessions are part of the same snapshot, not per-user I/O.
        let cancel_path = p.slots[&id].directory.join("cancel.json");
        match_allocation::atomic_json(&cancel_path, &vec!["s1", "s2"]).unwrap();
        p.refresh_handoffs();
        fs::remove_file(cancel_path).unwrap();
        for _ in 0..10 {
            assert!(p.slots[&id].cancelled());
            assert_eq!(p.cancelled_sessions(&id), ["s1", "s2"]);
            assert!(!p.slots[&id].ready());
        }
        status.phase = Phase::Failed;
        p.write_status_for_test(&id, &status);
        p.refresh_handoffs();
        status.phase = Phase::Recovering;
        p.write_status_for_test(&id, &status);
        p.slots.get_mut(&id).unwrap().refresh_status();
        assert_eq!(p.slots[&id].status.as_ref().unwrap().phase, Phase::Failed);
        p.slots
            .get_mut(&id)
            .unwrap()
            .child
            .as_mut()
            .unwrap()
            .wait()
            .unwrap();
    }

    #[test]
    fn reused_port_receipts_do_not_become_a_hundred_live_arenas_on_restart() {
        let mut p = Pool::for_test();
        for index in 1..=101 {
            let mut m = manifest('a');
            m.allocation_id = format!("{index:032x}");
            let id = p.allocate(m).unwrap();
            p.slots
                .get_mut(&id)
                .unwrap()
                .child
                .as_mut()
                .unwrap()
                .wait()
                .unwrap();
            p.write_status_for_test(
                &id,
                &Status {
                    released_sessions: Vec::new(),
                    allocation_id: id.clone(),
                    server_epoch: 1,
                    heartbeat_ms: match_allocation::unix_ms(),
                    phase: Phase::Finished,
                    result_id: Some(format!("{index:032x}")),
                    ends_by_ms: None,
                    joined_sessions: Vec::new(),
                },
            );
            p.poll();
            assert_eq!(p.occupied(), 0);
        }
        assert_eq!(p.slots.len(), 101);
        // Re-read as a coordinator that has no exit observation for adopted slots.
        p.slots.clear();
        p.adopt_existing().unwrap();
        assert_eq!(p.slots.len(), 101);
        assert_eq!(
            p.occupied(),
            1,
            "retained receipts share one physical bind endpoint"
        );
        assert!(
            p.full(),
            "one adopted port still keeps the conservative heartbeat hold"
        );
        assert!(p.slots.values().all(|s| !s.child_exit_observed));
    }

    #[test]
    fn running_assignment_cannot_be_cancelled_from_lobby() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        let slot = p.slots.get_mut(&id).unwrap();
        slot.status = Some(Status {
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Running,
            result_id: None,
            ends_by_ms: None,
            joined_sessions: Vec::new(),
        });
        assert!(!p.cancel(&id));
        assert!(!p.slots[&id].cancelled());
        p.slots
            .get_mut(&id)
            .unwrap()
            .child
            .as_mut()
            .unwrap()
            .wait()
            .unwrap();
    }
}
