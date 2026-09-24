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
impl Pool {
    pub fn from_env() -> Result<Self, String> {
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
        let public_host =
            std::env::var("OMOBA_MATCH_PUBLIC_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let bind_ip = std::env::var("OMOBA_MATCH_BIND_IP").unwrap_or_else(|_| "127.0.0.1".into());
        format!("{bind_ip}:{first_port}")
            .parse::<SocketAddr>()
            .map_err(|_| "Invalid worker bind IP")?;
        if public_host.is_empty()
            || public_host.len() > 200
            || public_host.contains(['/', '\n', '\r', ' '])
        {
            return Err("Invalid public worker host".into());
        }
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
            pool.slots.insert(
                manifest.allocation_id.clone(),
                Slot {
                    manifest,
                    status,
                    directory,
                    child: None,
                    recovering: false,
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
            slot.status =
                read_status(&slot.directory, &slot.manifest.allocation_id).or(slot.status.take());
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
    pub fn ready(&self) -> bool {
        !self.recovering
            && !self.cancelled()
            && self.status.as_ref().is_some_and(|s| {
                matches!(
                    s.phase,
                    Phase::Ready | Phase::Forming | Phase::Running | Phase::Settling
                ) && match_allocation::unix_ms().saturating_sub(s.heartbeat_ms) < 10_000
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
    Command::new(executable)
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
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .spawn()
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
mod tests {
    use shared::match_service::MatchPreference;

    use super::*;
    use crate::match_allocation::AllocatedHuman;
    fn pool() -> Pool {
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
    fn only_one_coordinator_can_own_a_root_and_drop_releases_it() {
        let p = pool();
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
        let mut p = pool();
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
        let mut p = pool();
        let id = p.allocate(manifest('a')).unwrap();
        let slot = p.slots.get_mut(&id).unwrap();
        slot.status = Some(Status {
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Ready,
            result_id: None,
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
    fn running_assignment_cannot_be_cancelled_from_lobby() {
        let mut p = pool();
        let id = p.allocate(manifest('a')).unwrap();
        let slot = p.slots.get_mut(&id).unwrap();
        slot.status = Some(Status {
            allocation_id: id.clone(),
            server_epoch: 1,
            heartbeat_ms: match_allocation::unix_ms(),
            phase: Phase::Running,
            result_id: None,
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
