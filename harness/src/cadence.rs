//! Bounded wall-time diagnostics for the native load producer.
use std::time::Duration;

/// Exclusive upper bounds; the final bucket includes all larger durations.
pub const TIMING_BUCKET_UPPER_US: [u64; 12] = [
    1_000, 5_000, 10_000, 25_000, 45_000, 55_000, 75_000, 100_000, 150_000, 250_000, 500_000,
    1_000_000,
];

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TimingHistogram {
    pub count: u64,
    pub total_us: u64,
    pub buckets: [u64; 13],
}

impl TimingHistogram {
    pub fn record(&mut self, elapsed: Duration) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let bucket = TIMING_BUCKET_UPPER_US.partition_point(|&upper| micros >= upper);
        self.count = self.count.saturating_add(1);
        self.total_us = self.total_us.saturating_add(micros);
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_boundaries_and_long_stalls_do_not_disappear() {
        let mut timing = TimingHistogram::default();
        for micros in [0, 999, 1_000, 50_000, 1_000_000, 2_000_000] {
            timing.record(Duration::from_micros(micros));
        }
        assert_eq!(timing.count, 6);
        assert_eq!(timing.total_us, 3_051_999);
        assert_eq!(timing.buckets.iter().sum::<u64>(), timing.count);
        assert_eq!(timing.buckets[0], 2);
        assert_eq!(timing.buckets[1], 1);
        assert_eq!(timing.buckets[5], 1);
        assert_eq!(timing.buckets[12], 2);
    }

    #[test]
    fn oversized_durations_saturate_without_wrapping() {
        let mut timing = TimingHistogram::default();
        timing.record(Duration::MAX);
        timing.record(Duration::from_micros(1));
        assert_eq!(timing.count, 2);
        assert_eq!(timing.total_us, u64::MAX);
        assert_eq!(timing.buckets[12], 1);
    }
}
