//! Opt-in authoritative load diagnostics. No wire fields, player identifiers,
//! signing material or behavior changes are added; reports stay on stderr.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const REPORT_INTERVAL: Duration = Duration::from_secs(1);
const BUCKET_UPPER_US: [u64; 12] = [
    1_000, 5_000, 10_000, 25_000, 45_000, 55_000, 75_000, 100_000, 150_000, 250_000, 500_000,
    1_000_000,
];

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct TimingHistogram {
    pub count: u64,
    pub total_us: u64,
    pub buckets: [u64; 13],
}

impl TimingHistogram {
    fn record(&mut self, elapsed: Duration) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let bucket = BUCKET_UPPER_US.partition_point(|&upper| micros >= upper);
        self.count = self.count.saturating_add(1);
        self.total_us = self.total_us.saturating_add(micros);
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
    }
}

#[derive(Default)]
struct Endpoint {
    player_id: Option<u64>,
    transport_commands: u64,
    gameplay_transforms: u64,
    last_sequence: u64,
    sequence_gaps: u64,
    sequence_restarts: u64,
    last_transform_at: Option<Instant>,
    snapshots: u64,
    latest_transform_age_at_snapshot: TimingHistogram,
}

#[derive(serde::Serialize)]
struct EndpointReport {
    player_id: Option<u64>,
    transport_commands: u64,
    gameplay_transforms: u64,
    last_sequence: u64,
    sequence_gaps: u64,
    sequence_restarts: u64,
    snapshots: u64,
    latest_transform_age_at_snapshot: TimingHistogram,
}

pub(crate) struct LoadMetrics {
    enabled: bool,
    started: Instant,
    last_report: Instant,
    last_broadcast: Option<Instant>,
    pending_sequence: Option<u64>,
    endpoints: HashMap<SocketAddr, Endpoint>,
    broadcasts: u64,
    broadcast_gaps: TimingHistogram,
}

impl LoadMetrics {
    pub(crate) fn from_env(now: Instant) -> Self {
        Self::new(
            std::env::var_os("OMOBA_LOAD_METRICS").as_deref() == Some(std::ffi::OsStr::new("1")),
            now,
        )
    }

    #[cfg(test)]
    fn enabled(now: Instant) -> Self {
        Self::new(true, now)
    }

    fn new(enabled: bool, now: Instant) -> Self {
        Self {
            enabled,
            started: now,
            last_report: now,
            last_broadcast: None,
            pending_sequence: None,
            endpoints: HashMap::new(),
            broadcasts: 0,
            broadcast_gaps: TimingHistogram::default(),
        }
    }

    /// A verified signed command reached application dispatch. Sequence gaps
    /// expose commands not accepted by this server, including transport drops.
    pub(crate) fn signed_dispatch(&mut self, addr: SocketAddr, sequence: u64) {
        if !self.enabled {
            return;
        }
        self.pending_sequence = Some(sequence);
        let endpoint = self.endpoints.entry(addr).or_default();
        endpoint.transport_commands = endpoint.transport_commands.saturating_add(1);
        if endpoint.last_sequence != 0 {
            if sequence > endpoint.last_sequence {
                endpoint.sequence_gaps = endpoint
                    .sequence_gaps
                    .saturating_add(sequence - endpoint.last_sequence - 1);
            } else {
                endpoint.sequence_restarts = endpoint.sequence_restarts.saturating_add(1);
            }
        }
        endpoint.last_sequence = sequence;
    }

    pub(crate) fn finish_dispatch(&mut self) {
        self.pending_sequence = None;
    }

    /// Called only after ordinary gameplay command application, never for a
    /// datagram that merely passed signature/replay validation.
    pub(crate) fn applied_transform(&mut self, addr: SocketAddr, player_id: u64, now: Instant) {
        if !self.enabled || self.pending_sequence.is_none() {
            return;
        }
        let endpoint = self.endpoints.entry(addr).or_default();
        endpoint.player_id = Some(player_id);
        endpoint.gameplay_transforms = endpoint.gameplay_transforms.saturating_add(1);
        endpoint.last_transform_at = Some(now);
    }

    /// Actual world-broadcast starts, including starts with no recipients.
    pub(crate) fn broadcast(&mut self, now: Instant) {
        if !self.enabled {
            return;
        }
        self.broadcasts = self.broadcasts.saturating_add(1);
        if let Some(previous) = self.last_broadcast.replace(now) {
            self.broadcast_gaps
                .record(now.saturating_duration_since(previous));
        }
    }

    /// A complete snapshot reached kernel send, not merely snapshot assembly.
    pub(crate) fn snapshot_sent(&mut self, addr: SocketAddr, now: Instant) {
        if !self.enabled {
            return;
        }
        let endpoint = self.endpoints.entry(addr).or_default();
        endpoint.snapshots = endpoint.snapshots.saturating_add(1);
        if let Some(sent) = endpoint.last_transform_at {
            endpoint
                .latest_transform_age_at_snapshot
                .record(now.saturating_duration_since(sent));
        }
    }

    pub(crate) fn emit_if_due(&mut self, now: Instant) {
        if let Some(report) = self.report_if_due(now) {
            eprintln!("LOAD_METRICS {report}");
        }
    }

    fn report_if_due(&mut self, now: Instant) -> Option<String> {
        if !self.enabled || now.saturating_duration_since(self.last_report) < REPORT_INTERVAL {
            return None;
        }
        self.last_report = now;
        let mut endpoints: Vec<_> = self
            .endpoints
            .iter()
            .map(|(addr, endpoint)| (*addr, endpoint))
            .collect();
        endpoints.sort_unstable_by_key(|(addr, _)| *addr);
        let endpoints: Vec<_> = endpoints
            .into_iter()
            .map(|(_, endpoint)| EndpointReport {
                player_id: endpoint.player_id,
                transport_commands: endpoint.transport_commands,
                gameplay_transforms: endpoint.gameplay_transforms,
                last_sequence: endpoint.last_sequence,
                sequence_gaps: endpoint.sequence_gaps,
                sequence_restarts: endpoint.sequence_restarts,
                snapshots: endpoint.snapshots,
                latest_transform_age_at_snapshot: endpoint.latest_transform_age_at_snapshot.clone(),
            })
            .collect();
        serde_json::to_string(&serde_json::json!({
            "elapsed_secs": now.saturating_duration_since(self.started).as_secs_f64(),
            "bucket_upper_us": BUCKET_UPPER_US,
            "broadcasts": self.broadcasts,
            "broadcast_gaps": &self.broadcast_gaps,
            "endpoints": endpoints,
            "scope": "verified signed application dispatch; ordinary applied transforms; complete snapshot kernel sends; cumulative since process start"
        }))
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_metrics_keep_no_endpoint_state() {
        let now = Instant::now();
        let addr: SocketAddr = "127.0.0.1:4001".parse().unwrap();
        let mut metrics = LoadMetrics::new(false, now);
        metrics.signed_dispatch(addr, 1);
        metrics.applied_transform(addr, 7, now);
        metrics.broadcast(now);
        metrics.snapshot_sent(addr, now);
        assert!(metrics.endpoints.is_empty());
        assert_eq!(metrics.broadcasts, 0);
        assert!(metrics.report_if_due(now + REPORT_INTERVAL).is_none());
    }

    #[test]
    fn report_preserves_gaps_restarts_and_applied_transform_latency() {
        let now = Instant::now();
        let addr: SocketAddr = "127.0.0.1:4001".parse().unwrap();
        let mut metrics = LoadMetrics::enabled(now);
        metrics.signed_dispatch(addr, 1);
        metrics.applied_transform(addr, 7, now);
        metrics.finish_dispatch();
        metrics.signed_dispatch(addr, 4);
        metrics.finish_dispatch();
        metrics.signed_dispatch(addr, 1);
        metrics.applied_transform(addr, 7, now + Duration::from_millis(20));
        metrics.finish_dispatch();
        metrics.broadcast(now + Duration::from_millis(30));
        metrics.broadcast(now + Duration::from_millis(85));
        metrics.snapshot_sent(addr, now + Duration::from_millis(70));
        let report = metrics.report_if_due(now + REPORT_INTERVAL).unwrap();
        let value: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(value["broadcasts"], 2);
        assert_eq!(value["broadcast_gaps"]["count"], 1);
        assert_eq!(value["broadcast_gaps"]["total_us"], 55_000);
        let endpoint = &value["endpoints"][0];
        assert_eq!(endpoint["player_id"], 7);
        assert_eq!(endpoint["transport_commands"], 3);
        assert_eq!(endpoint["gameplay_transforms"], 2);
        assert_eq!(endpoint["sequence_gaps"], 2);
        assert_eq!(endpoint["sequence_restarts"], 1);
        assert_eq!(endpoint["snapshots"], 1);
        assert_eq!(endpoint["latest_transform_age_at_snapshot"]["count"], 1);
        assert_eq!(
            endpoint["latest_transform_age_at_snapshot"]["total_us"],
            50_000
        );
    }

    #[test]
    fn histogram_keeps_boundaries_and_long_stalls() {
        let mut histogram = TimingHistogram::default();
        for micros in [0, 999, 1_000, 50_000, 1_000_000, 2_000_000] {
            histogram.record(Duration::from_micros(micros));
        }
        assert_eq!(histogram.count, 6);
        assert_eq!(histogram.total_us, 3_051_999);
        assert_eq!(histogram.buckets.iter().sum::<u64>(), 6);
        assert_eq!(histogram.buckets[0], 2);
        assert_eq!(histogram.buckets[1], 1);
        assert_eq!(histogram.buckets[5], 1);
        assert_eq!(histogram.buckets[12], 2);
    }
}
