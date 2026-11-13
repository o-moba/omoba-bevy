//! Disposable operational report. No account IDs, DSNs or error strings.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default)]
pub(crate) struct Probe {
    pub checked_ms: Option<u64>,
    pub ok: bool,
}

pub(crate) struct Sample {
    pub epoch: Option<u64>,
    pub probe: Probe,
    pub journal_ready: bool,
    pub staged_dodges: usize,
    pub legacy_floors: usize,
    pub unpersisted_results: usize,
    pub critical_replies: usize,
    pub reply_receiver_closed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    pub version: u32,
    pub updated_ms: u64,
    pub server_epoch: Option<u64>,
    pub probe_checked_ms: Option<u64>,
    pub sql_ready: bool,
    pub journal_ready: bool,
    pub staged_dodges: usize,
    pub legacy_floors: usize,
    pub unpersisted_results: usize,
    pub critical_replies: usize,
    pub reply_receiver_closed: bool,
    pub blocked_since_ms: Option<u64>,
}

#[derive(Default)]
pub(crate) struct Health {
    last_write: Option<Instant>,
    blocked_since: Option<u64>,
}
impl Health {
    fn sample(&mut self, now: u64, sample: Sample) -> Report {
        let sql_ready = sample.probe.ok
            && sample.probe.checked_ms.is_some_and(|at| {
                at <= now.saturating_add(5_000) && now.saturating_sub(at) <= 30_000
            });
        let blocked = !sql_ready
            || !sample.journal_ready
            || sample.staged_dodges != 0
            || sample.legacy_floors != 0
            || sample.unpersisted_results != 0;
        let blocked = blocked || sample.critical_replies != 0 || sample.reply_receiver_closed;
        if blocked {
            self.blocked_since.get_or_insert(now);
        } else {
            self.blocked_since = None;
        }
        Report {
            version: 2,
            updated_ms: now,
            server_epoch: sample.epoch,
            probe_checked_ms: sample.probe.checked_ms,
            sql_ready,
            journal_ready: sample.journal_ready,
            staged_dodges: sample.staged_dodges,
            legacy_floors: sample.legacy_floors,
            unpersisted_results: sample.unpersisted_results,
            critical_replies: sample.critical_replies,
            reply_receiver_closed: sample.reply_receiver_closed,
            blocked_since_ms: self.blocked_since,
        }
    }

    pub fn publish(&mut self, root: &Path, now: u64, sample: Sample) {
        let report = self.sample(now, sample);
        if self
            .last_write
            .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.last_write = Some(Instant::now());
        if write(root, &report).is_err() {
            // Missing/stale reports are themselves actionable in the monitor.
            eprintln!("Cannot publish career worker health; storage report remains unavailable.");
        }
    }
}

fn write(root: &Path, report: &Report) -> std::io::Result<()> {
    let dir = root.join("health");
    fs::create_dir_all(&dir)?;
    let data = serde_json::to_vec(report)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let temp = dir.join("status.tmp");
    let mut file = options.open(&temp)?;
    file.write_all(&data)?;
    drop(file);
    // Disposable status is atomic but deliberately not a durability/fsync job.
    fs::rename(temp, dir.join("status.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn healthy(now: u64) -> Sample {
        Sample {
            epoch: None,
            probe: Probe {
                checked_ms: Some(now),
                ok: true,
            },
            journal_ready: true,
            staged_dodges: 0,
            legacy_floors: 0,
            unpersisted_results: 0,
            critical_replies: 0,
            reply_receiver_closed: false,
        }
    }
    #[test]
    fn consecutive_failures_preserve_the_first_block_time_and_recovery_clears_it() {
        let mut h = Health::default();
        let mut sample = healthy(1000);
        sample.probe = Probe::default();
        assert_eq!(h.sample(1000, sample).blocked_since_ms, Some(1000));
        let mut sample = healthy(5000);
        sample.legacy_floors = 1;
        assert_eq!(h.sample(5000, sample).blocked_since_ms, Some(1000));
        assert_eq!(h.sample(6000, healthy(6000)).blocked_since_ms, None);
        let mut sample = healthy(7000);
        sample.staged_dodges = 1;
        assert_eq!(h.sample(7000, sample).blocked_since_ms, Some(7000));
    }
    #[test]
    fn stale_and_future_probes_never_certify_sql_readiness() {
        let mut h = Health::default();
        assert!(!h.sample(40_000, healthy(1)).sql_ready);
        assert!(!h.sample(1000, healthy(7000)).sql_ready);
        assert!(h.sample(40_000, healthy(40_000)).sql_ready);
    }
    #[test]
    fn unpersisted_results_and_unavailable_journals_are_visible_without_a_receipt() {
        let mut h = Health::default();
        let mut sample = healthy(1000);
        sample.unpersisted_results = 1;
        let report = h.sample(1000, sample);
        assert_eq!(report.unpersisted_results, 1);
        assert_eq!(report.blocked_since_ms, Some(1000));
        let mut sample = healthy(2000);
        sample.journal_ready = false;
        assert!(!h.sample(2000, sample).journal_ready);
    }
    #[test]
    fn atomic_report_contains_only_the_typed_anonymous_contract() {
        let root = std::env::temp_dir().join(format!(
            "omoba-outbox-unit-health-{}",
            crate::career_backend::hex(&getrandom_bytes())
        ));
        let mut h = Health::default();
        let outbox = root.join("var/lib/omoba-ru/lobby-outbox");
        write(&outbox, &h.sample(1000, healthy(1000))).unwrap();
        let path = outbox.join("health/status.json");
        let report: Report = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(report.version, 2);
        assert!(report.sql_ready);
        assert!(!outbox.join("health/status.tmp").exists());
        #[cfg(unix)]
        {
            let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("scripts/check_monitor_fixture.py");
            let output = std::process::Command::new("python3")
                .arg(helper)
                .arg("--storage-only")
                .arg("--root")
                .arg(&root)
                .arg("--now-ms")
                .arg("1000")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let proof: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(proof["status"], "PASS");
        }
        fs::remove_dir_all(root).unwrap();
    }
    fn getrandom_bytes() -> [u8; 16] {
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        b
    }
}
