//! Every match ends (LR-11). A match worker whose humans all left keeps their
//! heroes under bot control; once no human has been connected for the reclaim
//! window plus a grace, the round is reaped. Any running match also ends at a
//! hard duration cap (`OMOBA_MATCH_MAX_DURATION_SECS`, default 45 minutes).
//!
//! Both settle as `Interrupted` through the existing career path: unrated, no
//! winner, and stored exactly once. A worker then retires (no further
//! simulation, `failed` status, process exit, port freed); a standalone server
//! starts a fresh round.
use std::time::{Duration, Instant};

use shared::career::MatchOutcome;
use shared::wire::GameState;

use crate::balance::SESSION_RECLAIM_WINDOW;
use crate::match_service::MatchService;
use crate::runtime::{ServerRuntime, network_endpoint};

/// How long past the reclaim window an unattended worker match keeps running.
pub(crate) const ABANDONED_MATCH_GRACE: Duration = Duration::from_secs(30);
/// The hard cap on a running match (launch default O-8: 45 minutes).
pub(crate) const DEFAULT_MATCH_MAX_DURATION: Duration = Duration::from_secs(45 * 60);
pub(crate) const MATCH_MAX_DURATION_ENV: &str = "OMOBA_MATCH_MAX_DURATION_SECS";
const MATCH_MAX_DURATION_LIMIT_SECS: u64 = 24 * 60 * 60;

/// Why the server ended a running match before a base fell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ForcedEnd {
    /// No human was connected for the reclaim window plus the grace.
    AbandonedReaped,
    /// The match reached the hard duration cap.
    DurationCapped,
}

impl ForcedEnd {
    pub(crate) fn metric(self) -> &'static str {
        match self {
            Self::AbandonedReaped => "abandoned_reaped",
            Self::DurationCapped => "duration_capped",
        }
    }
}

/// The `MATCH_METRIC` line for a forced end; `detail` is `key=value` pairs.
pub(crate) fn forced_end_metric(
    end: ForcedEnd,
    epoch: u64,
    match_id: u64,
    elapsed_ms: u128,
    detail: &str,
) -> String {
    format!(
        "MATCH_METRIC event={} outcome=Interrupted epoch={epoch} match={match_id} elapsed_ms={elapsed_ms} {detail}",
        end.metric()
    )
}

/// `OMOBA_MATCH_MAX_DURATION_SECS`: unset or empty is the default; anything
/// else must be whole seconds from 1 to 86400, or the server refuses to start.
pub(crate) fn max_match_duration(value: Option<&str>) -> Result<Duration, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(DEFAULT_MATCH_MAX_DURATION);
    };
    match value.parse::<u64>() {
        Ok(secs) if (1..=MATCH_MAX_DURATION_LIMIT_SECS).contains(&secs) => {
            Ok(Duration::from_secs(secs))
        }
        _ => Err(format!(
            "Invalid {MATCH_MAX_DURATION_ENV}: expected whole seconds from 1 to {MATCH_MAX_DURATION_LIMIT_SECS}"
        )),
    }
}

impl ServerRuntime {
    /// Ends a running match that nobody attends or that ran past the cap.
    /// The Combat Sandbox is a local tool and is never ended.
    pub(crate) fn enforce_match_lifecycle(&mut self, now: Instant) {
        if self.sandbox.is_some() || self.match_service.is_lobby() {
            return;
        }
        if let Some(end) = self.unattended_worker(now).or_else(|| self.capped(now)) {
            self.end_running_match(end, now);
        }
    }

    /// A running worker match with no human on a network endpoint. Seats a
    /// human left (Leave or timeout) stay joined and non-bot at an internal
    /// address, so they do not count as connected.
    fn unattended_worker(&mut self, now: Instant) -> Option<ForcedEnd> {
        let live = self.match_service.worker().is_some_and(|worker| {
            !worker.aborted
                && !worker.recovery
                && self.match_started_at.is_some()
                && matches!(self.world.game_state, GameState::Running)
        });
        let connected = self
            .world
            .players
            .iter()
            .any(|(addr, player)| player.joined && network_endpoint(*addr, player));
        if !live || connected {
            self.unattended_since = None;
            return None;
        }
        let since = *self.unattended_since.get_or_insert(now);
        (now.saturating_duration_since(since) >= SESSION_RECLAIM_WINDOW + ABANDONED_MATCH_GRACE)
            .then_some(ForcedEnd::AbandonedReaped)
    }

    /// The latest moment the running match ends unless a base falls first:
    /// the duration cap or, while no human is connected to a worker, the
    /// abandoned-match reaping (`None` when no match is running). A reclaim
    /// resets the reaping, so the value can move later again.
    pub(crate) fn latest_match_end(&self, now: Instant) -> Option<Instant> {
        let started = self.match_started_at?;
        if !matches!(self.world.game_state, GameState::Running) {
            return None;
        }
        let capped = started + self.max_match_duration;
        let reaped = self
            .unattended_since
            .filter(|_| self.match_service.worker().is_some())
            .map(|since| since + SESSION_RECLAIM_WINDOW + ABANDONED_MATCH_GRACE);
        Some(reaped.map_or(capped, |reaped| reaped.min(capped)).max(now))
    }

    fn capped(&self, now: Instant) -> Option<ForcedEnd> {
        let started = self.match_started_at?;
        let live = matches!(self.world.game_state, GameState::Running)
            && self
                .match_service
                .worker()
                .is_none_or(|worker| !worker.aborted && !worker.recovery);
        (live && now.saturating_duration_since(started) >= self.max_match_duration)
            .then_some(ForcedEnd::DurationCapped)
    }

    fn end_running_match(&mut self, end: ForcedEnd, now: Instant) {
        let connected = self
            .world
            .players
            .iter()
            .filter(|(addr, player)| player.joined && network_endpoint(**addr, player))
            .count();
        let detail = match end {
            ForcedEnd::AbandonedReaped => format!(
                "unattended_ms={}",
                self.unattended_since
                    .map_or(0, |since| now.saturating_duration_since(since).as_millis())
            ),
            ForcedEnd::DurationCapped => format!(
                "cap_secs={} connected={connected}",
                self.max_match_duration.as_secs()
            ),
        };
        println!(
            "{}",
            forced_end_metric(
                end,
                self.server_epoch,
                self.match_id,
                self.elapsed_match_ms(now),
                &detail
            )
        );
        self.finish_career_round(MatchOutcome::Interrupted, None, now);
        self.unattended_since = None;
        if let MatchService::Worker(worker) = &mut self.match_service {
            // `tick_match_service` reports `settling` until the result is
            // stored, then `failed`, and exits 30 s later; the lobby frees the
            // port once the last heartbeat is 35 s old.
            worker.aborted = true;
        } else {
            self.restart_round(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    use shared::career::MatchOutcome;
    use shared::wire::{ClientPacket, GameState};

    use super::*;
    use crate::match_allocation::runtime_tests::fixture;
    use crate::match_allocation::{Phase, Status};
    use crate::runtime::ServerRuntime;

    /// The allocated BotPractice fixture with its durable start acknowledged
    /// and the round running, plus its private allocation directory.
    fn running_worker() -> (ServerRuntime, SocketAddr, ClientPacket, Instant) {
        let (mut rt, addr, join) = fixture();
        fs::create_dir_all(&rt.match_service.worker().unwrap().directory).unwrap();
        let now = Instant::now();
        rt.handle_packet(addr, join.clone(), now);
        assert!(!rt.begin_career_round(now));
        let allocation = rt.career_allocation_for_test().unwrap();
        rt.career.backend.test_ack_start(&allocation.result_id);
        assert!(rt.begin_career_round(now));
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(now);
        (rt, addr, join, now)
    }

    fn status(rt: &ServerRuntime) -> Status {
        let directory = &rt.match_service.worker().unwrap().directory;
        serde_json::from_slice(&fs::read(directory.join("status.json")).unwrap()).unwrap()
    }

    /// Stores the settled result as the career store would acknowledge it.
    fn acknowledge_settlement(rt: &mut ServerRuntime, now: Instant) {
        let mut saved = rt.career_allocation_for_test().unwrap();
        saved.saved = true;
        rt.career.backend.test_ack_settle(saved);
        rt.poll_career(now);
    }

    fn cleanup(rt: &ServerRuntime) {
        let _ = fs::remove_dir_all(&rt.match_service.worker().unwrap().directory);
    }

    #[test]
    fn duration_setting_defaults_to_forty_five_minutes_and_rejects_nonsense() {
        assert_eq!(max_match_duration(None), Ok(Duration::from_secs(2700)));
        assert_eq!(max_match_duration(Some(" ")), Ok(Duration::from_secs(2700)));
        assert_eq!(max_match_duration(Some("90")), Ok(Duration::from_secs(90)));
        assert_eq!(
            max_match_duration(Some("86400")),
            Ok(Duration::from_secs(86_400))
        );
        for bad in ["0", "-5", "1.5", "45m", "86401", "18446744073709551616"] {
            let error = max_match_duration(Some(bad)).unwrap_err();
            assert!(error.contains(MATCH_MAX_DURATION_ENV), "{bad}: {error}");
        }
    }

    #[test]
    fn forced_ends_have_distinct_match_metric_events() {
        let line = forced_end_metric(ForcedEnd::AbandonedReaped, 7, 3, 1234, "unattended_ms=9");
        assert_eq!(
            line,
            "MATCH_METRIC event=abandoned_reaped outcome=Interrupted epoch=7 match=3 elapsed_ms=1234 unattended_ms=9"
        );
        assert!(
            forced_end_metric(ForcedEnd::DurationCapped, 7, 3, 1, "cap_secs=2700")
                .starts_with("MATCH_METRIC event=duration_capped ")
        );
    }

    #[test]
    fn abandoned_bot_practice_settles_interrupted_and_retires_the_worker() {
        let (mut rt, addr, join, now) = running_worker();
        // The only human leaves; a bot keeps their hero alive.
        rt.handle_packet(addr, ClientPacket::Leave, now);
        assert!(
            rt.world
                .players
                .iter()
                .any(|(seat, p)| crate::bots::is_bot_address(*seat)
                    && !p.hero.identity.is_bot
                    && p.joined)
        );
        rt.tick(now, 0.0);
        let window = SESSION_RECLAIM_WINDOW + ABANDONED_MATCH_GRACE;
        let before = now + window - Duration::from_secs(1);
        rt.tick(before, 0.0);
        assert!(!rt.match_service.worker().unwrap().aborted);
        assert_eq!(rt.world.game_state, GameState::Running);

        let after = now + window;
        rt.tick(after, 0.0);
        assert!(rt.match_service.worker().unwrap().aborted);
        let result = rt.career_allocation_for_test().unwrap();
        assert_eq!(result.outcome, MatchOutcome::Interrupted);
        assert!(!result.rated && result.winner.is_none());
        assert_eq!(result.ruleset, "public-casual-v1");
        assert_eq!(
            result.unrated_reason.as_deref(),
            Some("allocated_bots"),
            "the store's public-casual policy accepts only its own reason"
        );
        // An aborted worker admits nobody and no longer simulates.
        assert!(!rt.authorize_allocated_join(addr, &join));

        rt.tick_match_service(after);
        assert_eq!(status(&rt).phase, Phase::Settling);
        acknowledge_settlement(&mut rt, after);
        let later = after + Duration::from_secs(1);
        rt.tick_match_service(later);
        let status = status(&rt);
        assert_eq!(status.phase, Phase::Failed);
        assert_eq!(status.result_id, Some(result.result_id.clone()));
        // The worker process exits 30 s after its terminal status.
        assert_eq!(rt.match_service.worker().unwrap().terminal_at, Some(later));
        // Settlement is final: a later tick cannot rewrite it.
        rt.finish_career_round(MatchOutcome::Completed, None, later);
        assert_eq!(
            rt.career_allocation_for_test().unwrap().outcome,
            MatchOutcome::Interrupted
        );
        cleanup(&rt);
    }

    /// LR-13: a running worker reports when its match ends at the latest:
    /// the cap while a human plays, the reaping once every human left, and
    /// the cap again after a reclaim.
    #[test]
    fn the_running_receipt_reports_the_latest_end_for_players_who_left() {
        let (mut rt, addr, join, now) = running_worker();
        let left_ms = |rt: &ServerRuntime| {
            let status = status(rt);
            assert_eq!(status.phase, Phase::Running);
            status
                .ends_by_ms
                .unwrap()
                .saturating_sub(crate::match_allocation::unix_ms())
        };
        let near = |actual: u64, expected: Duration| {
            let expected = expected.as_millis() as u64;
            assert!(
                actual <= expected && actual + 5_000 > expected,
                "{actual} ms, expected about {expected} ms"
            );
        };
        let cap = rt.max_match_duration;
        rt.tick(now, 0.0);
        rt.tick_match_service(now);
        near(left_ms(&rt), cap);

        rt.handle_packet(addr, ClientPacket::Leave, now);
        let left = now + Duration::from_secs(1);
        rt.tick(left, 0.0);
        rt.tick_match_service(left);
        near(left_ms(&rt), SESSION_RECLAIM_WINDOW + ABANDONED_MATCH_GRACE);

        let back = left + Duration::from_secs(1);
        rt.career.backend.test_authenticated(
            addr,
            shared::career::ProfileSummary::new("a".repeat(64), "Alice".into()),
            "seat-a",
        );
        rt.handle_packet(addr, join, back);
        rt.tick(back, 0.0);
        rt.tick_match_service(back);
        near(left_ms(&rt), cap - Duration::from_secs(2));
        // Nothing to report once the match no longer runs.
        rt.world.game_state = GameState::Victory {
            winner: shared::map::Team::Green,
        };
        assert_eq!(rt.latest_match_end(back), None);
        cleanup(&rt);
    }

    #[test]
    fn reclaim_inside_the_window_resets_the_abandonment_timer() {
        let (mut rt, addr, join, now) = running_worker();
        rt.handle_packet(addr, ClientPacket::Leave, now);
        rt.tick(now, 0.0);
        let window = SESSION_RECLAIM_WINDOW + ABANDONED_MATCH_GRACE;
        let back = now + window - Duration::from_secs(5);
        rt.career.backend.test_authenticated(
            addr,
            shared::career::ProfileSummary::new("a".repeat(64), "Alice".into()),
            "seat-a",
        );
        rt.handle_packet(addr, join, back);
        assert!(rt.world.players[&addr].joined);
        rt.tick(back, 0.0);
        assert!(rt.unattended_since.is_none());
        // A connected human keeps the match alive long after the window.
        rt.world.players.get_mut(&addr).unwrap().last_seen = now + window * 2;
        rt.tick(now + window * 2, 0.0);
        assert!(!rt.match_service.worker().unwrap().aborted);
        assert_eq!(rt.world.game_state, GameState::Running);
        cleanup(&rt);
    }

    #[test]
    fn worker_match_past_the_cap_settles_interrupted_with_humans_connected() {
        let (mut rt, addr, _, now) = running_worker();
        rt.max_match_duration = Duration::from_secs(600);
        let almost = now + Duration::from_secs(599);
        rt.world.players.get_mut(&addr).unwrap().last_seen = almost;
        rt.tick(almost, 0.0);
        assert!(!rt.match_service.worker().unwrap().aborted);

        let capped = now + Duration::from_secs(600);
        rt.world.players.get_mut(&addr).unwrap().last_seen = capped;
        rt.tick(capped, 0.0);
        assert!(rt.match_service.worker().unwrap().aborted);
        let result = rt.career_allocation_for_test().unwrap();
        assert_eq!(result.outcome, MatchOutcome::Interrupted);
        assert!(!result.rated && result.winner.is_none());
        // The connected player's career view carries the terminal receipt,
        // which moves the client to its result screen.
        let receipt = rt.career_view(addr, capped).last_result.unwrap();
        assert_eq!(receipt.result_id, result.result_id);
        assert_eq!(receipt.outcome, MatchOutcome::Interrupted);
        acknowledge_settlement(&mut rt, capped);
        rt.tick_match_service(capped);
        assert_eq!(status(&rt).phase, Phase::Failed);
        cleanup(&rt);
    }

    #[test]
    fn standalone_match_past_the_cap_starts_a_fresh_round() {
        use crate::match_rules::{MatchConfig, MatchMode};
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(
            socket,
            MatchConfig {
                mode: MatchMode::Dev,
                team_size: 1,
            },
        );
        rt.max_match_duration = Duration::from_secs(60);
        let now = Instant::now();
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(now);
        let first = rt.match_id;
        rt.enforce_match_lifecycle(now + Duration::from_secs(59));
        assert_eq!(rt.match_id, first);
        rt.enforce_match_lifecycle(now + Duration::from_secs(60));
        assert_eq!(rt.match_id, first + 1);
        assert!(rt.match_started_at.is_none());
        // The Combat Sandbox is never capped.
        rt.sandbox = Some(crate::sandbox::SandboxRuntime::new(now));
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(now);
        rt.enforce_match_lifecycle(now + Duration::from_secs(3600));
        assert_eq!(rt.match_id, first + 1);
    }
}
