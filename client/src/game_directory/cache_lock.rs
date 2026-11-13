use std::fs::{File, TryLockError};

/// Release the OS lock explicitly before closing this descriptor. A parallel
/// fork may have inherited the same open-file description until it execs;
/// closing only our descriptor would leave that shared lock held in the child.
pub(super) struct CacheLock(File);

impl CacheLock {
    pub(super) fn acquire(file: File) -> Result<Self, TryLockError> {
        file.try_lock()?;
        Ok(Self(file))
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        path::Path,
        process::{Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn open(path: &Path) -> File {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .unwrap()
    }

    #[test]
    fn inherited_descriptor_does_not_keep_a_released_cache_busy() {
        const ENV: &str = "OMOBA_DIRECTORY_INHERITED_LOCK_PROBE";
        if let Some(dir) = std::env::var_os(ENV) {
            // stdin intentionally retains a duplicate of the parent's locked
            // description, modelling the pre-exec interval of a parallel fork.
            let dir = std::path::PathBuf::from(dir);
            fs::write(dir.join("ready"), []).unwrap();
            let started = Instant::now();
            while !dir.join("release").exists() {
                assert!(started.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(10));
            }
            return;
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "omoba-directory-inherited-lock-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("lock");
        let held = CacheLock::acquire(open(&path)).unwrap();
        let name = format!(
            "{}::inherited_descriptor_does_not_keep_a_released_cache_busy",
            module_path!().split_once("::").unwrap().1
        );
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name])
            .env(ENV, &dir)
            .stdin(Stdio::from(held.0.try_clone().unwrap()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !dir.join("ready").exists() && child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(5) {
                child.kill().unwrap();
                let _ = child.wait();
                panic!("inherited-lock child did not become ready");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(dir.join("ready").exists());
        let blocked_while_owned = CacheLock::acquire(open(&path));
        drop(held);
        let acquired_after_release = CacheLock::acquire(open(&path));
        fs::write(dir.join("release"), []).unwrap();
        // Child has its own deadline; wait only after unblocking it.
        let child_status = child.wait().unwrap();
        assert!(matches!(blocked_while_owned, Err(TryLockError::WouldBlock)));
        assert!(
            acquired_after_release.is_ok(),
            "child retained a released lock"
        );
        assert!(child_status.success());
        drop(acquired_after_release);
        fs::remove_dir_all(dir).unwrap();
    }
}
