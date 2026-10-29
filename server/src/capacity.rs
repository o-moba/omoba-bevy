//! Arena capacity on the lobby (LR-09): how fast arenas free, the estimated
//! wait in the line for a free arena, and the capacity-busy metric.
//!
//! The metric is a `CAPACITY_METRIC` journal line when the line forms or
//! clears (and every minute while it lasts), and `<match root>/capacity.json`,
//! rewritten every few seconds, which the RU host monitor alerts on
//! (`ops/vps/ru/ru_monitor.py`).
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The lobby's report in the match root.
pub(crate) const REPORT_FILE: &str = "capacity.json";
/// Arena frees older than this no longer count for the wait estimate.
const TURNOVER_WINDOW: Duration = Duration::from_secs(15 * 60);
/// A lobby younger than this has no estimate yet.
const MIN_TURNOVER_WINDOW: Duration = Duration::from_secs(60);
const MAX_FREES: usize = 1024;
const REPORT_INTERVAL: Duration = Duration::from_secs(5);
const LOG_INTERVAL: Duration = Duration::from_secs(60);

/// When arenas freed, to estimate the wait for the next ones.
#[derive(Default)]
pub(crate) struct Turnover {
    since: Option<Instant>,
    /// Arenas holding a port after the previous pass's allocations.
    occupied: Option<usize>,
    frees: VecDeque<Instant>,
}

impl Turnover {
    /// The pool right after its poll: the arenas that stopped holding a port
    /// since [`Turnover::settle`] freed now.
    pub fn observe(&mut self, occupied: usize, now: Instant) {
        self.since.get_or_insert(now);
        if let Some(before) = self.occupied {
            for _ in 0..before.saturating_sub(occupied) {
                self.frees.push_back(now);
            }
        }
        while self.frees.len() > MAX_FREES
            || self
                .frees
                .front()
                .is_some_and(|at| now.saturating_duration_since(*at) >= TURNOVER_WINDOW)
        {
            self.frees.pop_front();
        }
    }

    /// The pool after this pass's allocations.
    pub fn settle(&mut self, occupied: usize) {
        self.occupied = Some(occupied);
    }

    /// Estimated whole seconds until `position` more arenas free, at the
    /// rate they freed over the last 15 minutes (or since the lobby started,
    /// once that is a minute ago). `None` while none freed in that time.
    pub fn eta_secs(&self, position: u32, now: Instant) -> Option<u64> {
        let window = now
            .saturating_duration_since(self.since?)
            .min(TURNOVER_WINDOW);
        let frees = self
            .frees
            .iter()
            .filter(|at| now.saturating_duration_since(**at) < window)
            .count();
        if window < MIN_TURNOVER_WINDOW || frees == 0 {
            return None;
        }
        let millis = u128::from(position) * window.as_millis() / frees as u128;
        u64::try_from(millis.div_ceil(1000)).ok()
    }
}

/// What the lobby reports about its arenas ([`REPORT_FILE`], version 1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CapacityReport {
    pub version: u32,
    pub updated_ms: u64,
    /// `OMOBA_MATCH_CAPACITY`.
    pub capacity: usize,
    /// Arenas holding their port (live, or retired less than 35 s ago).
    pub occupied: usize,
    /// Players in the lobby's queue.
    pub searching: usize,
    /// Players whose roster is ready but waits for a free arena.
    pub waiting_for_arena: usize,
    /// Since when (Unix ms) some ready roster has waited for a free arena;
    /// `None` while none does.
    pub busy_since_ms: Option<u64>,
    /// The longest current wait for a free arena, in whole seconds.
    pub longest_wait_secs: u64,
    /// Busy periods since the lobby started.
    pub busy_periods: u64,
}

/// The lobby at the end of one scheduling pass.
pub(crate) struct Sample<'a> {
    pub capacity: usize,
    pub occupied: usize,
    pub searching: usize,
    /// Profiles in the line for a free arena.
    pub line: &'a HashMap<String, u32>,
}

/// What one pass reports.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Update {
    pub report: CapacityReport,
    /// Rewrite [`REPORT_FILE`] now.
    pub write: bool,
    /// A `CAPACITY_METRIC` line for the journal.
    pub log: Option<String>,
}

/// The capacity-busy metric: a busy period lasts while some ready roster
/// waits for a free arena.
#[derive(Default)]
pub(crate) struct CapacityMetric {
    busy_since: Option<(Instant, u64)>,
    periods: u64,
    /// When each profile in the line joined it.
    waits: HashMap<String, Instant>,
    last_report: Option<Instant>,
    last_log: Option<Instant>,
}

impl CapacityMetric {
    pub fn update(&mut self, sample: Sample<'_>, now: Instant, unix_ms: u64) -> Update {
        self.waits
            .retain(|profile, _| sample.line.contains_key(profile));
        for profile in sample.line.keys() {
            self.waits.entry(profile.clone()).or_insert(now);
        }
        let longest = self
            .waits
            .values()
            .map(|since| now.saturating_duration_since(*since).as_secs())
            .max()
            .unwrap_or(0);
        let busy = !sample.line.is_empty();
        let mut event = None;
        match (busy, self.busy_since) {
            (true, None) => {
                self.busy_since = Some((now, unix_ms));
                self.periods = self.periods.saturating_add(1);
                event = Some("busy");
            }
            (false, Some(_)) => event = Some("clear"),
            (true, Some(_))
                if self
                    .last_log
                    .is_none_or(|at| now.saturating_duration_since(at) >= LOG_INTERVAL) =>
            {
                event = Some("still_busy");
            }
            _ => {}
        }
        let busy_secs = self.busy_since.map_or(0, |(since, _)| {
            now.saturating_duration_since(since).as_secs()
        });
        if !busy {
            self.busy_since = None;
        }
        let report = CapacityReport {
            version: 1,
            updated_ms: unix_ms,
            capacity: sample.capacity,
            occupied: sample.occupied,
            searching: sample.searching,
            waiting_for_arena: sample.line.len(),
            busy_since_ms: self.busy_since.map(|(_, ms)| ms),
            longest_wait_secs: longest,
            busy_periods: self.periods,
        };
        let log = event.map(|event| {
            self.last_log = Some(now);
            format!(
                "CAPACITY_METRIC event={event} capacity={} occupied={} searching={} \
                 waiting_for_arena={} longest_wait_secs={longest} busy_secs={busy_secs}",
                report.capacity, report.occupied, report.searching, report.waiting_for_arena
            )
        });
        let write = event.is_some_and(|event| event != "still_busy")
            || self
                .last_report
                .is_none_or(|at| now.saturating_duration_since(at) >= REPORT_INTERVAL);
        if write {
            self.last_report = Some(now);
        }
        Update { report, write, log }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(profiles: &[&str]) -> HashMap<String, u32> {
        profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| ((*profile).to_owned(), index as u32 + 1))
            .collect()
    }

    #[test]
    fn the_wait_estimate_follows_the_rate_arenas_freed() {
        let start = Instant::now();
        let mut turnover = Turnover::default();
        turnover.observe(2, start);
        turnover.settle(2);
        assert_eq!(turnover.eta_secs(1, start), None, "nothing freed yet");
        // One arena freed after 30 s, another at 60 s; both were reused.
        turnover.observe(1, start + Duration::from_secs(30));
        turnover.settle(2);
        assert_eq!(
            turnover.eta_secs(1, start + Duration::from_secs(30)),
            None,
            "too young for an estimate"
        );
        turnover.observe(1, start + Duration::from_secs(60));
        turnover.settle(2);
        // Two frees in 60 s: one every 30 s.
        let now = start + Duration::from_secs(60);
        assert_eq!(turnover.eta_secs(1, now), Some(30));
        assert_eq!(turnover.eta_secs(3, now), Some(90));
        // Allocations alone free nothing.
        turnover.observe(2, now + Duration::from_secs(1));
        turnover.settle(2);
        // Frees older than 15 minutes stop counting.
        let late = start + TURNOVER_WINDOW + Duration::from_secs(61);
        turnover.observe(2, late);
        assert_eq!(turnover.eta_secs(1, late), None);
    }

    #[test]
    fn a_busy_period_is_logged_reported_and_cleared() {
        let start = Instant::now();
        let mut metric = CapacityMetric::default();
        let sample = |line| Sample {
            capacity: 2,
            occupied: 2,
            searching: 3,
            line,
        };
        let idle = line(&[]);
        let first = metric.update(sample(&idle), start, 1_000);
        assert!(first.write && first.log.is_none());
        assert_eq!(first.report.busy_since_ms, None);
        // Within the report interval nothing is rewritten.
        let quiet = metric.update(sample(&idle), start + Duration::from_secs(1), 2_000);
        assert!(!quiet.write);

        let waiting = line(&["a", "b"]);
        let busy = metric.update(sample(&waiting), start + Duration::from_secs(2), 3_000);
        assert!(busy.write);
        assert_eq!(
            busy.log.as_deref(),
            Some(
                "CAPACITY_METRIC event=busy capacity=2 occupied=2 searching=3 \
                 waiting_for_arena=2 longest_wait_secs=0 busy_secs=0"
            )
        );
        assert_eq!(busy.report.busy_since_ms, Some(3_000));
        assert_eq!(busy.report.busy_periods, 1);

        let later = start + Duration::from_secs(70);
        let only_b = line(&["b"]);
        let still = metric.update(sample(&only_b), later, 71_000);
        assert_eq!(still.report.longest_wait_secs, 68);
        assert_eq!(still.report.busy_since_ms, Some(3_000));
        assert!(
            still
                .log
                .as_deref()
                .is_some_and(|log| log.starts_with("CAPACITY_METRIC event=still_busy")
                    && log.ends_with("busy_secs=68")),
            "{:?}",
            still.log
        );

        let clear = metric.update(sample(&idle), later + Duration::from_secs(1), 72_000);
        assert!(clear.write);
        assert_eq!(clear.report.busy_since_ms, None);
        assert_eq!(clear.report.longest_wait_secs, 0);
        assert!(
            clear
                .log
                .as_deref()
                .is_some_and(|log| log.starts_with("CAPACITY_METRIC event=clear")
                    && log.ends_with("busy_secs=69"))
        );
        let again = metric.update(sample(&waiting), later + Duration::from_secs(2), 73_000);
        assert_eq!(again.report.busy_periods, 2);
        assert_eq!(again.report.longest_wait_secs, 0, "a new wait starts at 0");
    }
}
