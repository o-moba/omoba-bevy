//! Operator-owned maintenance marker. Only the bounded lobby tick polls it;
//! player views read cached state and workers never consult this directory.
use std::path::Path;

pub(super) const MARKER: &str = "drain";
pub(super) const CODE: &str = "server_draining";

#[derive(Debug, Default, PartialEq, Eq)]
struct Observed {
    active: bool,
    operation_id: Option<String>,
    problem: Option<&'static str>,
}

fn observe(root: &Path) -> Observed {
    let invalid = || Observed {
        active: true,
        operation_id: None,
        problem: Some("invalid_marker"),
    };
    let path = root.join(MARKER);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return if root.is_dir() {
                Observed::default()
            } else {
                invalid()
            };
        }
        Err(_) => return invalid(),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return invalid();
    }
    // At most two directory entries, no marker contents. A special file can
    // neither force an unbounded read nor block this process on a FIFO read.
    let entries = match std::fs::read_dir(path)
        .and_then(|entries| entries.take(2).collect::<Result<Vec<_>, _>>())
    {
        Ok(entries) if entries.len() == 1 => entries,
        _ => return invalid(),
    };
    let entry = &entries[0];
    let name = entry.file_name();
    let Some(id) = name.to_str().filter(|id| {
        id.len() == 32
            && id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) else {
        return invalid();
    };
    let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
        return invalid();
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 {
        return invalid();
    }
    Observed {
        active: true,
        operation_id: Some(id.to_owned()),
        problem: None,
    }
}

pub(super) struct State {
    observed: Observed,
    pub report_pending: bool,
}

impl State {
    pub fn new(root: &Path) -> Self {
        let mut state = Self {
            observed: Observed::default(),
            report_pending: true,
        };
        state.poll(root);
        state
    }

    pub fn active(&self) -> bool {
        self.observed.active
    }

    pub fn operation_id(&self) -> Option<&str> {
        self.observed.operation_id.as_deref()
    }

    pub fn poll(&mut self, root: &Path) {
        let next = observe(root);
        if next != self.observed {
            println!(
                "DRAIN_METRIC active={} operation_id={} error={}",
                next.active,
                next.operation_id.as_deref().unwrap_or("-"),
                next.problem.unwrap_or("-")
            );
            self.observed = next;
            self.report_pending = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_valid_replaced_invalid_and_removed_markers_are_cached_and_acknowledged() {
        let pool = crate::match_pool::Pool::for_test();
        let root = pool.test_directory();
        let marker = root.join(MARKER);
        let mut state = State::new(root);
        assert!(!state.active());
        std::fs::create_dir(&marker).unwrap();
        state.poll(root);
        assert!(state.active());
        assert_eq!(state.operation_id(), None, "empty marker fails closed");
        let id = "a".repeat(32);
        std::fs::write(marker.join(&id), []).unwrap();
        state.poll(root);
        assert_eq!(state.operation_id(), Some(id.as_str()));
        state.report_pending = false;
        state.poll(root);
        assert!(
            !state.report_pending,
            "unchanged marker does not force more reports"
        );
        // Cached views do no I/O and keep the last acknowledged state.
        std::fs::write(marker.join(&id), b"unexpected content").unwrap();
        assert_eq!(state.operation_id(), Some(id.as_str()));
        state.poll(root);
        assert!(state.active());
        assert_eq!(state.operation_id(), None);
        assert!(state.report_pending);
        std::fs::remove_file(marker.join(&id)).unwrap();
        std::fs::write(marker.join("b".repeat(32)), []).unwrap();
        state.poll(root);
        assert_eq!(state.operation_id(), Some("b".repeat(32).as_str()));
        std::fs::write(marker.join("c".repeat(32)), []).unwrap();
        state.poll(root);
        assert_eq!(state.operation_id(), None, "multiple owners fail closed");
        std::fs::remove_dir_all(&marker).unwrap();
        state.poll(root);
        assert!(!state.active());
        assert_eq!(state.operation_id(), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ordinary_file_and_malformed_operation_names_fail_closed() {
        let pool = crate::match_pool::Pool::for_test();
        let root = pool.test_directory();
        let marker = root.join(MARKER);
        std::fs::write(&marker, []).unwrap();
        assert!(State::new(root).active());
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        for id in ["short".into(), "G".repeat(32), "a".repeat(33)] {
            std::fs::write(marker.join(&id), []).unwrap();
            let state = State::new(root);
            assert!(state.active());
            assert_eq!(state.operation_id(), None);
            std::fs::remove_file(marker.join(id)).unwrap();
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_marker_or_operation_entry_is_never_followed() {
        use std::os::unix::fs::symlink;
        let pool = crate::match_pool::Pool::for_test();
        let root = pool.test_directory();
        let marker = root.join(MARKER);
        let target = root.join("other");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("a".repeat(32)), []).unwrap();
        symlink(&target, &marker).unwrap();
        assert_eq!(State::new(root).operation_id(), None);
        assert!(State::new(root).active());
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        symlink(target.join("a".repeat(32)), marker.join("a".repeat(32))).unwrap();
        assert_eq!(State::new(root).operation_id(), None);
        assert!(State::new(root).active());
        let _ = std::fs::remove_dir_all(root);
    }
}
