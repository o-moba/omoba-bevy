use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let event = Event::new(&"a".repeat(64), 1).unwrap();
        let path = std::env::temp_dir().join(format!("omoba-dodge-journal-{}", event.event_id));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> PathBuf {
        self.0.join("outbox/queue-dodges")
    }
    fn journal(&self) -> Journal {
        Journal::open(self.root()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn event() -> Event {
    Event::new(&"a".repeat(64), 1234).unwrap()
}

#[test]
fn record_is_durable_and_recovery_keeps_exact_identity_and_original_observation() {
    let f = Fixture::new();
    let e = event();
    let mut journal = f.journal();
    journal.record(e.clone()).unwrap();
    journal.flush().unwrap();
    drop(journal);
    let reopened = f.journal();
    assert_eq!(reopened.pending().cloned().collect::<Vec<_>>(), vec![e]);
}
#[test]
fn duplicate_delivery_is_one_file_and_conflicting_content_never_overwrites_it() {
    let f = Fixture::new();
    let mut j = f.journal();
    let e = event();
    j.record(e.clone()).unwrap();
    j.record(e.clone()).unwrap();
    let path = f.root().join(format!("{}.json", e.event_id));
    let before = fs::read(&path).unwrap();
    let mut changed = e.clone();
    changed.observed_at_ms += 1;
    assert_eq!(j.record(changed), Err(Error::Conflict));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(j.pending().count(), 1);
    assert_eq!(fs::read_dir(f.root()).unwrap().count(), 2);
}
#[test]
fn unsafe_or_unknown_typed_records_are_refused_before_publication() {
    let f = Fixture::new();
    let mut j = f.journal();
    for id in ["../escape", "", &"A".repeat(32), &"a".repeat(33)] {
        let mut e = event();
        e.event_id = id.into();
        assert_eq!(j.record(e), Err(Error::Invalid));
    }
    let mut e = event();
    e.profile_id = "not-a-profile".into();
    assert_eq!(j.record(e), Err(Error::Invalid));
    let mut e = event();
    e.version = 2;
    assert_eq!(j.record(e), Err(Error::Invalid));
    let mut e = event();
    e.observed_at_ms = u64::MAX;
    assert_eq!(j.record(e), Err(Error::Invalid));
    assert!(j.is_empty());
    assert_eq!(fs::read_dir(f.root()).unwrap().count(), 1);
}
#[test]
fn a_second_live_writer_is_refused_and_a_restart_can_take_the_lock() {
    let f = Fixture::new();
    let j = f.journal();
    assert!(matches!(Journal::open(f.root()), Err(Error::Busy)));
    drop(j);
    assert!(Journal::open(f.root()).is_ok());
}
#[test]
fn a_duplicate_descriptor_does_not_keep_a_retired_journal_locked() {
    let f = Fixture::new();
    let j = f.journal();
    // Model the descriptor inherited by a concurrent process spawn before exec.
    let duplicate = j._lock.0.try_clone().unwrap();
    assert!(matches!(Journal::open(f.root()), Err(Error::Busy)));
    drop(j);
    let reopened = f.journal();
    assert!(matches!(Journal::open(f.root()), Err(Error::Busy)));
    drop(duplicate);
    // Closing an old descriptor must not unlock the new journal's ownership.
    assert!(matches!(Journal::open(f.root()), Err(Error::Busy)));
    drop(reopened);
    assert!(Journal::open(f.root()).is_ok());
}
#[test]
fn corrupt_oversized_and_wrong_format_receipts_fail_closed_without_deletion() {
    for data in [
        b"{broken".to_vec(),
        vec![b' '; MAX_RECORD_BYTES as usize + 1],
        br#"{"kind":"Settle","result":{}}"#.to_vec(),
    ] {
        let f = Fixture::new();
        drop(f.journal());
        let path = f.root().join(format!("{}.json", event().event_id));
        fs::write(&path, &data).unwrap();
        assert!(matches!(Journal::open(f.root()), Err(Error::Invalid)));
        assert_eq!(fs::read(path).unwrap(), data);
    }
}
#[test]
fn filename_identity_must_match_the_receipt() {
    let f = Fixture::new();
    drop(f.journal());
    let e = event();
    let path = f.root().join(format!(
        "{}.json",
        Event::new(&"b".repeat(64), 1).unwrap().event_id
    ));
    fs::write(path, serde_json::to_vec(&e).unwrap()).unwrap();
    assert!(matches!(Journal::open(f.root()), Err(Error::Conflict)));
}
#[test]
fn incomplete_temporary_file_is_preserved_without_fabricating_a_committed_event() {
    let f = Fixture::new();
    drop(f.journal());
    let temp = f.root().join(".interrupted.tmp");
    fs::write(&temp, b"partial").unwrap();
    let j = f.journal();
    assert!(j.is_empty());
    assert_eq!(fs::read(temp).unwrap(), b"partial");
}
#[test]
fn completion_removes_only_the_committed_receipt_and_does_not_claim_other_ids() {
    let f = Fixture::new();
    let mut j = f.journal();
    let a = event();
    let b = event();
    j.record(a.clone()).unwrap();
    j.record(b.clone()).unwrap();
    assert_eq!(j.complete(&event().event_id), Err(Error::Invalid));
    j.complete(&a.event_id).unwrap();
    drop(j);
    let j = f.journal();
    assert_eq!(j.pending().next(), Some(&b));
}
#[test]
fn a_changed_receipt_cannot_be_retired_or_certified_by_flush() {
    let f = Fixture::new();
    let mut j = f.journal();
    let e = event();
    j.record(e.clone()).unwrap();
    let path = f.root().join(format!("{}.json", e.event_id));
    let mut changed = e.clone();
    changed.profile_id = "b".repeat(64);
    fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(j.complete(&e.event_id), Err(Error::Conflict));
    assert_eq!(j.flush(), Err(Error::Conflict));
    assert_eq!(read_event(&path).unwrap(), changed);
    assert_eq!(j.pending().count(), 1);
}
#[test]
fn a_failed_publication_retains_the_foreign_entry_and_can_retry_after_repair() {
    let f = Fixture::new();
    let mut j = f.journal();
    let e = event();
    let path = f.root().join(format!("{}.json", e.event_id));
    fs::create_dir(&path).unwrap();
    assert!(j.record(e.clone()).is_err());
    assert!(path.is_dir());
    assert!(j.is_empty());
    fs::remove_dir(&path).unwrap();
    j.record(e.clone()).unwrap();
    fs::remove_file(&path).unwrap();
    j.flush().unwrap();
    assert_eq!(read_event(&path).unwrap(), e);
}
#[test]
fn capacity_refusal_keeps_all_older_receipts_in_memory() {
    let f = Fixture::new();
    let mut j = f.journal();
    for n in 0..MAX_RECORDS {
        let mut e = event();
        e.event_id = format!("{n:032x}");
        j.pending.insert(e.event_id.clone(), e);
    }
    assert_eq!(j.record(event()), Err(Error::Capacity));
    assert_eq!(j.pending().count(), MAX_RECORDS);
}
#[cfg(unix)]
#[test]
fn static_symlinks_and_special_files_are_rejected_without_following_or_blocking() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    drop(f.journal());
    let outside = f.0.join("outside");
    fs::write(&outside, b"do-not-touch").unwrap();
    let path = f.root().join(format!("{}.json", event().event_id));
    symlink(&outside, &path).unwrap();
    assert!(matches!(Journal::open(f.root()), Err(Error::Invalid)));
    assert_eq!(fs::read(&outside).unwrap(), b"do-not-touch");
    fs::remove_file(&path).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    assert!(matches!(Journal::open(f.root()), Err(Error::Invalid)));
}
#[cfg(unix)]
#[test]
fn a_symlinked_journal_root_is_not_opened() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = f.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let root = f.0.join("linked");
    symlink(&outside, &root).unwrap();
    assert!(matches!(Journal::open(root), Err(Error::Invalid)));
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn legacy_counter_load_is_compatible_and_keeps_pending_events_after_expiry() {
    let f = Fixture::new();
    let path = f.0.join("legacy.json");
    let profile = "a".repeat(64);
    fs::write(
        &path,
        format!(r#"{{"{profile}":{{"strikes":2,"last_ms":1000,"until_ms":301000}}}}"#),
    )
    .unwrap();
    let loaded = load_penalties(&path, 2000).unwrap();
    assert_eq!(loaded[&profile].strikes, 2);
    assert!(loaded[&profile].pending_events.is_empty());
    assert!(load_penalties(&path, 86_402_000).unwrap().is_empty());
    let mut row = loaded.into_values().next().unwrap();
    let e = event();
    row.pending_events.push(e.clone());
    fs::write(
        &path,
        serde_json::to_vec(&HashMap::from([(profile.clone(), row)])).unwrap(),
    )
    .unwrap();
    let loaded = load_penalties(&path, 86_402_000).unwrap();
    assert_eq!(loaded[&profile].pending_events, vec![e]);
}
#[test]
fn corrupt_or_foreign_pending_producer_state_is_not_an_empty_success() {
    let f = Fixture::new();
    let path = f.0.join("legacy.json");
    assert!(load_penalties(&path, 1).unwrap().is_empty());
    for bytes in [
        b"{broken".as_slice(),
        b"null",
        br#"{"a":{"strikes":0,"last_ms":1,"until_ms":0}}"#,
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(load_penalties(&path, 1).is_err());
    }
    let e = event();
    let row = Penalty {
        strikes: 1,
        last_ms: 1000,
        until_ms: 61000,
        pending_events: vec![e],
    };
    fs::write(
        &path,
        serde_json::to_vec(&HashMap::from([("b".repeat(64), row)])).unwrap(),
    )
    .unwrap();
    assert!(matches!(load_penalties(&path, 2000), Err(Error::Conflict)));
}
#[test]
fn producer_snapshot_reads_are_bounded_and_refuse_special_files() {
    let f = Fixture::new();
    let path = f.0.join("legacy.json");
    let file = File::create(&path).unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(load_penalties(&path, 1), Err(Error::Invalid)));
    drop(file);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(matches!(load_penalties(&path, 1), Err(Error::Invalid)));
}
