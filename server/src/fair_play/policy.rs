//! Bounded per-seat time accounting. Only the runtime decides whether a
//! packet is meaningful human input and applies a requested AI takeover.
use std::time::{Duration, Instant};

pub(super) const GRACE: Duration = Duration::from_secs(90);

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Participation {
    pub connected_ms: u64,
    pub disconnected_ms: u64,
    pub afk_ms: u64,
    pub abandoned: bool,
}

pub(super) struct Seat {
    started: Instant,
    last_tick: Instant,
    last_input: Instant,
    connected: bool,
    alive: bool,
    absent_since: Option<Instant>,
    connected_time: Duration,
    disconnected_time: Duration,
    afk_time: Duration,
    afk_takeover: bool,
    abandoned: bool,
    finalized: bool,
}

impl Seat {
    pub fn new(now: Instant, connected: bool, alive: bool) -> Self {
        Self {
            started: now,
            last_tick: now,
            last_input: now,
            connected,
            alive,
            absent_since: (!connected).then_some(now),
            connected_time: Duration::ZERO,
            disconnected_time: Duration::ZERO,
            afk_time: Duration::ZERO,
            afk_takeover: false,
            abandoned: false,
            finalized: false,
        }
    }

    /// Returns whether the connected actor now needs an AFK AI takeover.
    /// Observe a successful detach again at the same instant with connected=false.
    pub fn observe(&mut self, now: Instant, connected: bool, alive: bool) -> bool {
        if self.finalized || now < self.last_tick {
            return false;
        }
        self.accrue(now);
        if self
            .absent_since
            .is_some_and(|at| now.saturating_duration_since(at) >= GRACE)
        {
            self.abandoned = true;
        }
        if connected && !self.connected {
            self.absent_since = None;
            self.afk_takeover = false;
            self.last_input = now;
        } else if !connected && self.connected {
            self.absent_since = Some(now);
        }
        // Dead time is not inactivity, and respawn starts a new input grace.
        if !alive || !self.alive {
            self.last_input = now;
        }
        self.connected = connected;
        self.alive = alive;
        let afk = connected && alive && now.saturating_duration_since(self.last_input) >= GRACE;
        if afk {
            self.abandoned = true;
            self.afk_takeover = true;
        }
        afk
    }

    fn accrue(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_tick);
        if self.connected {
            self.connected_time = self.connected_time.saturating_add(elapsed);
        } else {
            self.disconnected_time = self.disconnected_time.saturating_add(elapsed);
        }
        // An authenticated AFK endpoint can still receive snapshots. AI time
        // may overlap connected time, but must end exactly on the reclaim input.
        if self.afk_takeover {
            self.afk_time = self.afk_time.saturating_add(elapsed);
        }
        self.last_tick = now;
    }

    pub fn input(&mut self, now: Instant) {
        if !self.finalized && now >= self.last_tick && now >= self.last_input {
            self.accrue(now);
            self.last_input = now;
            self.afk_takeover = false;
        }
    }

    pub fn finish(&mut self, now: Instant) -> Participation {
        self.finalize(now, self.connected, self.alive)
    }

    pub fn snapshot(&self) -> Participation {
        let millis = |time: Duration| time.as_millis().min(u64::MAX as u128) as u64;
        Participation {
            connected_ms: millis(self.connected_time),
            disconnected_ms: millis(self.disconnected_time),
            afk_ms: millis(self.afk_time),
            abandoned: self.abandoned,
        }
    }

    pub fn finalize(&mut self, now: Instant, connected: bool, alive: bool) -> Participation {
        if !self.finalized && now >= self.last_tick {
            self.observe(now, connected, alive);
            // A brief grace reconnect alone is never an abandon. Repeated
            // shorter absences cannot evade the 80% rule after 90 s total away.
            let duration = now.saturating_duration_since(self.started).as_nanos();
            if self.disconnected_time >= GRACE
                && self.disconnected_time.as_nanos().saturating_mul(5) >= duration.saturating_mul(4)
            {
                self.abandoned = true;
            }
            self.finalized = true;
        }
        self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_disconnect_has_exact_grace_and_reclaim_cannot_erase_abandon() {
        for seconds in [89, 90] {
            let start = Instant::now();
            let mut seat = Seat::new(start, true, true);
            seat.observe(start, false, true);
            let now = start + Duration::from_secs(seconds);
            seat.observe(now, true, true);
            seat.input(now);
            assert_eq!(seat.snapshot().abandoned, seconds == 90);
            assert_eq!(seat.snapshot().disconnected_ms, seconds * 1000);
            // Reconnect also grants a fresh AFK window.
            assert!(!seat.observe(now + Duration::from_secs(89), true, true));
        }
    }

    #[test]
    fn idle_heartbeats_do_not_replace_input_and_afk_time_belongs_to_ai_takeover() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        for seconds in 1..90 {
            assert!(!seat.observe(start + Duration::from_secs(seconds), true, true));
        }
        let afk = start + GRACE;
        assert!(seat.observe(afk, true, true));
        assert!(seat.snapshot().abandoned);
        seat.observe(afk, false, true);
        let reclaim = afk + Duration::from_secs(5);
        seat.observe(reclaim, true, true);
        seat.input(reclaim);
        let stats = seat.finalize(reclaim + Duration::from_secs(1), true, true);
        assert_eq!(stats.connected_ms, 91_000);
        assert_eq!(stats.disconnected_ms, 5_000);
        assert_eq!(stats.afk_ms, 5_000);
        assert!(stats.abandoned);
    }

    #[test]
    fn meaningful_input_resets_the_afk_window_but_not_history() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        seat.observe(start + Duration::from_secs(89), true, true);
        seat.input(start + Duration::from_secs(89));
        assert!(!seat.observe(start + Duration::from_secs(178), true, true));
        assert!(seat.observe(start + Duration::from_secs(179), true, true));
    }

    #[test]
    fn an_afk_endpoint_can_stay_connected_without_bot_actions_reclaiming_it() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        assert!(seat.observe(start + GRACE, true, true));
        assert!(seat.observe(start + GRACE + Duration::from_secs(6), true, true));
        seat.input(start + GRACE + Duration::from_secs(6));
        let stats = seat.finish(start + GRACE + Duration::from_secs(7));
        assert_eq!(stats.connected_ms, 97_000);
        assert_eq!(stats.disconnected_ms, 0);
        assert_eq!(stats.afk_ms, 6_000);
        assert!(stats.abandoned);
    }

    #[test]
    fn reclaim_input_accrues_the_last_ai_interval_before_releasing_control() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        assert!(seat.observe(start + GRACE, true, true));
        seat.input(start + GRACE + Duration::from_secs(5));
        let stats = seat.finish(start + GRACE + Duration::from_secs(6));
        assert_eq!(stats.afk_ms, 5000);
        assert_eq!(stats.connected_ms, 96_000);
        assert!(stats.abandoned);
    }

    #[test]
    fn death_and_respawn_do_not_create_an_afk_abandon() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        assert!(!seat.observe(start + Duration::from_secs(89), true, false));
        assert!(!seat.observe(start + Duration::from_secs(300), true, false));
        assert!(!seat.observe(start + Duration::from_secs(301), true, true));
        assert!(!seat.observe(start + Duration::from_secs(390), true, true));
        assert!(!seat.snapshot().abandoned);
    }

    #[test]
    fn ordinary_short_reconnects_retain_accurate_accounting_without_afk_time() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        seat.observe(start + Duration::from_secs(10), false, true);
        seat.observe(start + Duration::from_secs(25), true, true);
        assert_eq!(
            seat.finalize(start + Duration::from_secs(30), true, true),
            Participation {
                connected_ms: 15_000,
                disconnected_ms: 15_000,
                afk_ms: 0,
                abandoned: false,
            }
        );
    }

    #[test]
    fn repeated_sub_grace_absences_at_eighty_percent_are_an_abandon() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        seat.observe(start + Duration::from_secs(10), false, true);
        seat.observe(start + Duration::from_secs(55), true, true);
        seat.observe(start + Duration::from_secs(65), false, true);
        seat.observe(start + Duration::from_secs(110), true, true);
        assert!(!seat.snapshot().abandoned);
        let stats = seat.finalize(start + Duration::from_millis(112_500), true, true);
        assert_eq!(stats.disconnected_ms, 90_000);
        assert_eq!(stats.connected_ms, 22_500);
        assert!(stats.abandoned);
    }

    #[test]
    fn one_sub_grace_reconnect_alone_cannot_trigger_the_percentage_rule() {
        let start = Instant::now();
        let mut seat = Seat::new(start, false, true);
        seat.observe(start + Duration::from_secs(89), true, true);
        assert!(
            !seat
                .finalize(start + Duration::from_secs(100), true, true)
                .abandoned
        );
    }

    #[test]
    fn rollback_and_repeated_finalization_cannot_rewrite_or_double_count_time() {
        let start = Instant::now();
        let mut seat = Seat::new(start, true, true);
        seat.observe(start + Duration::from_secs(50), true, true);
        seat.observe(start + Duration::from_secs(49), false, false);
        seat.input(start + Duration::from_secs(49));
        let stats = seat.finalize(start + Duration::from_secs(60), true, true);
        assert_eq!(stats.connected_ms, 60_000);
        assert_eq!(
            seat.finalize(start + Duration::from_secs(600), false, true),
            stats
        );
        assert!(!seat.observe(start + Duration::from_secs(600), true, true));
    }
}
