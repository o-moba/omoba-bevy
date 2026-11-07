//! Typed queue-dodge journal. Never passed through match-result recovery.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_RECORDS: usize = 4096;
const MAX_RECORD_BYTES: u64 = 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Event {
    version: u32,
    pub(crate) event_id: String,
    pub(crate) profile_id: String,
    pub(crate) observed_at_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Invalid,
    Conflict,
    Capacity,
    Busy,
    Io,
}
fn identity(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Event {
    pub(crate) fn new(profile_id: &str, observed_at_ms: u64) -> Result<Self, Error> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| Error::Io)?;
        let event = Self {
            version: 1,
            event_id: bytes.iter().map(|b| format!("{b:02x}")).collect(),
            profile_id: profile_id.to_owned(),
            observed_at_ms,
        };
        event.validate()?;
        Ok(event)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.version != 1
            || !identity(&self.event_id)
            || !shared::career::valid_profile_id(&self.profile_id)
            || self.observed_at_ms > i64::MAX as u64
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
fn sync_dir(path: &Path) -> Result<(), Error> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| Error::Io)
}
fn ensure_dir(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => return Ok(()),
        Ok(_) => return Err(Error::Invalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::Io),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(Error::Invalid)?;
    ensure_dir(parent)?;
    fs::create_dir(path).map_err(|_| Error::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Io)?;
    }
    sync_dir(parent)
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, Error> {
    let meta = fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| Error::Io)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
fn read_event(path: &Path) -> Result<Event, Error> {
    let bytes = read_bounded(path, MAX_RECORD_BYTES)?;
    let event: Event = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    event.validate()?;
    Ok(event)
}

/// Versionless legacy counters remain readable; pending typed events are
/// additive. Unknown/corrupt state is never interpreted as an empty history.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Penalty {
    pub(crate) strikes: u32,
    pub(crate) until_ms: u64,
    pub(crate) last_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pending_events: Vec<Event>,
}
pub(crate) fn load_penalties(path: &Path, now_ms: u64) -> Result<HashMap<String, Penalty>, Error> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(_) => return Err(Error::Io),
        Ok(_) => {}
    }
    let bytes = read_bounded(path, 16 * 1024 * 1024)?;
    let mut penalties: HashMap<String, Penalty> =
        serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    let mut ids = HashSet::new();
    for (profile, penalty) in &penalties {
        if !shared::career::valid_profile_id(profile)
            || penalty.strikes == 0
            || penalty.last_ms > i64::MAX as u64
            || penalty.until_ms > i64::MAX as u64
            || penalty.until_ms < penalty.last_ms
        {
            return Err(Error::Invalid);
        }
        for event in &penalty.pending_events {
            event.validate()?;
            if event.profile_id != *profile || !ids.insert(&event.event_id) {
                return Err(Error::Conflict);
            }
        }
    }
    drop(ids);
    penalties.retain(|_, p| {
        now_ms.saturating_sub(p.last_ms) < 86_400_000 || !p.pending_events.is_empty()
    });
    Ok(penalties)
}
fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

pub(crate) struct Journal {
    root: PathBuf,
    _lock: File,
    pending: BTreeMap<String, Event>,
}
impl Journal {
    pub(crate) fn open(root: PathBuf) -> Result<Self, Error> {
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir().map_err(|_| Error::Io)?.join(root)
        };
        ensure_dir(&root)?;
        let lock_path = root.join(".lock");
        if let Ok(meta) = fs::symlink_metadata(&lock_path)
            && (!meta.is_file() || meta.file_type().is_symlink())
        {
            return Err(Error::Invalid);
        }
        let lock = private_options()
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| Error::Io)?;
        lock.try_lock().map_err(|_| Error::Busy)?;
        let mut pending = BTreeMap::new();
        for (count, entry) in fs::read_dir(&root).map_err(|_| Error::Io)?.enumerate() {
            if count > MAX_RECORDS * 2 {
                return Err(Error::Capacity);
            }
            let entry = entry.map_err(|_| Error::Io)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(Error::Invalid)?;
            if name == ".lock" || (name.starts_with('.') && name.ends_with(".tmp")) {
                continue;
            }
            let id = name
                .strip_suffix(".json")
                .filter(|id| identity(id))
                .ok_or(Error::Invalid)?;
            let event = read_event(&entry.path())?;
            if event.event_id != id {
                return Err(Error::Conflict);
            }
            if pending.len() >= MAX_RECORDS {
                return Err(Error::Capacity);
            }
            pending.insert(id.to_owned(), event);
        }
        let journal = Self {
            root,
            _lock: lock,
            pending,
        };
        journal.flush()?;
        Ok(journal)
    }
    pub(crate) fn pending(&self) -> impl Iterator<Item = &Event> {
        self.pending.values()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn persist(&self, event: &Event) -> Result<(), Error> {
        event.validate()?;
        let path = self.root.join(format!("{}.json", event.event_id));
        if fs::symlink_metadata(&path).is_ok() {
            if read_event(&path)? != *event {
                return Err(Error::Conflict);
            }
            File::open(path)
                .and_then(|file| file.sync_all())
                .map_err(|_| Error::Io)?;
            return sync_dir(&self.root);
        }
        let temp = self.root.join(format!(
            ".{}.{}.{}.tmp",
            event.event_id,
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut own_temp = false;
        let result = (|| {
            let bytes = serde_json::to_vec(event).map_err(|_| Error::Invalid)?;
            if bytes.len() as u64 > MAX_RECORD_BYTES {
                return Err(Error::Invalid);
            }
            let mut file = private_options()
                .create_new(true)
                .open(&temp)
                .map_err(|_| Error::Io)?;
            own_temp = true;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| Error::Io)?;
            match fs::hard_link(&temp, &path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if read_event(&path)? != *event {
                        return Err(Error::Conflict);
                    }
                    File::open(&path)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| Error::Io)?;
                }
                Err(_) => return Err(Error::Io),
            }
            fs::remove_file(&temp).map_err(|_| Error::Io)?;
            sync_dir(&self.root)
        })();
        // Only this call's create_new path; never delete another receipt.
        if own_temp && temp.exists() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    pub(crate) fn record(&mut self, event: Event) -> Result<(), Error> {
        event.validate()?;
        if let Some(old) = self.pending.get(&event.event_id) {
            if old != &event {
                return Err(Error::Conflict);
            }
        } else if self.pending.len() >= MAX_RECORDS {
            return Err(Error::Capacity);
        }
        self.persist(&event)?;
        self.pending.insert(event.event_id.clone(), event);
        Ok(())
    }
    pub(crate) fn flush(&self) -> Result<(), Error> {
        for event in self.pending.values() {
            self.persist(event)?;
        }
        sync_dir(&self.root)
    }
    /// Call only after SQL commit. Missing files can be the result of an earlier
    /// unlink followed by failed directory sync; replay remains SQL-idempotent.
    pub(crate) fn complete(&mut self, id: &str) -> Result<(), Error> {
        let event = self.pending.get(id).ok_or(Error::Invalid)?;
        let path = self.root.join(format!("{id}.json"));
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                if read_event(&path)? != *event {
                    return Err(Error::Conflict);
                }
                fs::remove_file(path).map_err(|_| Error::Io)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Io),
        }
        sync_dir(&self.root)?;
        self.pending.remove(id);
        Ok(())
    }
}

#[cfg(test)]
#[path = "queue_dodge_outbox_tests.rs"]
mod tests;
