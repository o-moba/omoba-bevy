//! Bounded same-host worker pool. Children outlive a lobby restart; manifests do not.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::{fs, io};

use crate::match_allocation::{self, Manifest, Phase, Status};

pub(crate) struct Slot {
    pub manifest: Manifest,
    pub status: Option<Status>,
    directory: PathBuf,
    child: Option<Child>,
    recovering: bool,
    /// The lobby saw this match running (LR-13). Its later failure ends a
    /// played match; it is not a cancelled or expired allocation.
    ran: bool,
}
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
        let mut pool = Self {
            lock,
            slots: HashMap::new(),
            root,
            executable,
            capacity,
            bind_ip,
            public_host,
            first_port,
        };
        let entries = fs::read_dir(&pool.root).map_err(|_| "Cannot inspect workers")?;
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
                archive(&pool.root, &directory, &manifest.allocation_id);
                continue;
            }
            let ran = status.as_ref().is_some_and(Status::played);
            pool.slots.insert(
                manifest.allocation_id.clone(),
                Slot {
                    manifest,
                    status,
                    directory,
                    child: None,
                    recovering: false,
                    ran,
                },
            );
        }
        if pool.slots.len() > 100 {
            return Err("Too many retained allocations".into());
        }
        Ok(pool)
    }
    pub fn full(&self) -> bool {
        self.slots.values().filter(|s| s.occupies_port()).count() >= self.capacity
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
        let child = match spawn(&self.executable, &directory, &manifest, false) {
            Ok(child) => child,
            Err(_) => {
                let status = Status {
                    allocation_id: manifest.allocation_id.clone(),
                    server_epoch: 0,
                    heartbeat_ms: match_allocation::unix_ms(),
                    phase: Phase::Failed,
                    result_id: None,
                    ends_by_ms: None,
                };
                let _ = match_allocation::atomic_json(&directory.join("status.json"), &status);
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
                recovering: false,
                ran: false,
            },
        );
        Ok(id)
    }
    pub fn cancel(&mut self, id: &str) -> bool {
        let Some(slot) = self.slots.get(id) else {
            return true;
        };
        if slot
            .status
            .as_ref()
            .is_some_and(|s| matches!(s.phase, Phase::Running | Phase::Settling))
        {
            return false;
        }
        match_allocation::atomic_json(&slot.directory.join("cancel.json"), &true).is_ok()
    }
    pub fn poll(&mut self) {
        let now = match_allocation::unix_ms();
        let mut recovery_launches = 0;
        for slot in self.slots.values_mut() {
            slot.refresh_status();
            let exited = slot
                .child
                .as_mut()
                .is_some_and(|child| child.try_wait().is_ok_and(|r| r.is_some()));
            if exited {
                slot.child = None;
                if slot.status.is_none() {
                    let status = Status {
                        allocation_id: slot.manifest.allocation_id.clone(),
                        server_epoch: 0,
                        heartbeat_ms: now,
                        phase: Phase::Failed,
                        result_id: None,
                        ends_by_ms: None,
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
                if let Ok(child) = spawn(&self.executable, &slot.directory, &slot.manifest, true) {
                    slot.child = Some(child);
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
    fn latest_status(&self) -> Option<Status> {
        read_status(&self.directory, &self.manifest.allocation_id).or_else(|| self.status.clone())
    }

    pub fn refresh_status(&mut self) {
        self.status = self.latest_status();
        self.ran |= self.status.as_ref().is_some_and(Status::played);
    }

    /// The lobby saw this match running (LR-13).
    pub fn ran(&self) -> bool {
        self.ran
    }

    pub fn occupies_port(&self) -> bool {
        self.child.is_some()
            || !self.terminal()
            || self.status.as_ref().is_some_and(|s| {
                match_allocation::unix_ms().saturating_sub(s.heartbeat_ms) <= 35_000
            })
    }
    pub fn cancelled(&self) -> bool {
        self.directory.join("cancel.json").exists()
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
    /// cancelled, `ready`, `forming` or `running`, with a fresh heartbeat.
    pub fn ready_status(&self) -> Option<Status> {
        if self.recovering || self.cancelled() {
            return None;
        }
        self.latest_status().filter(|s| {
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
) -> io::Result<Child> {
    let output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("worker.log"))?;
    worker_command(executable, directory, m, recovery)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .spawn()
}

/// A worker inherits the lobby's environment except its own settings and the
/// local-only overrides: a map file, targeting QA, and the practice bot level,
/// so an operator setting on the lobby host can never weaken online bots.
fn worker_command(
    executable: &std::path::Path,
    directory: &std::path::Path,
    m: &Manifest,
    recovery: bool,
) -> Command {
    let mut command = Command::new(executable);
    command
        .env("OMOBA_SERVER_ROLE", "match")
        .env("OMOBA_MATCH_ALLOCATION", directory.join("manifest.json"))
        .env("OMOBA_MATCH_RECOVERY", if recovery { "1" } else { "0" })
        .env("OMOBA_CAREER_OUTBOX", directory.join("outbox"))
        .env("SERVER_ADDR", &m.bind)
        .env("OMOBA_TEAM_SIZE", "5")
        .env(
            "OMOBA_MATCH_MODE",
            if m.humans.len() == 10 {
                "release"
            } else {
                "practice"
            },
        )
        .env_remove("OMOBA_MAP_CONFIG")
        .env_remove("OMOBA_TARGETING_QA")
        .env_remove(shared::bot_profile::BotDifficulty::ENV);
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
        }
    }
    #[test]
    fn workers_never_inherit_local_overrides_such_as_the_bot_level() {
        let command = worker_command(
            std::path::Path::new("omoba-server"),
            std::path::Path::new("/tmp/allocation"),
            &manifest('a'),
            false,
        );
        let envs: HashMap<_, _> = command.get_envs().collect();
        for removed in [
            "OMOBA_MAP_CONFIG",
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
        assert!(
            p.full(),
            "terminal receipt does not immediately recycle its port"
        );
    }
    #[test]
    fn cancellation_blocks_handoff_before_status_update() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        let slot = p.slots.get_mut(&id).unwrap();
        slot.status = Some(Status {
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Ready,
            result_id: None,
            ends_by_ms: None,
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
    fn retired_worker_frees_its_port_once_its_last_heartbeat_ages() {
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
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms() - age_ms,
            phase: Phase::Failed,
            result_id: Some("r".repeat(32)),
            ends_by_ms: None,
        };
        p.write_status_for_test(&id, &retired(1_000));
        p.poll();
        assert!(p.slots[&id].terminal());
        assert!(p.full(), "the port is held while the exit is recent");
        p.write_status_for_test(&id, &retired(36_000));
        p.poll();
        assert!(!p.full());
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
    fn running_assignment_cannot_be_cancelled_from_lobby() {
        let mut p = Pool::for_test();
        let id = p.allocate(manifest('a')).unwrap();
        let slot = p.slots.get_mut(&id).unwrap();
        slot.status = Some(Status {
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Running,
            result_id: None,
            ends_by_ms: None,
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
