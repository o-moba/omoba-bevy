//! Named timing thresholds for multiplayer session lifecycle (TASK-14).
//! Values are tuned for local playtest; see `docs/network-client-session.md` and task spec.
//!
//! **Failure detection** (client): stale qualifying snapshot ([`T_STALE_SNAPSHOT`]), transport
//! send/recv error streaks ([`TRANSPORT_CONSECUTIVE_SEND_ERRORS`],
//! [`TRANSPORT_CONSECUTIVE_RECV_ERRORS`]), UDP thread signals ([`crate::net::NetThreadSignal`]),
//! snapshot channel disconnect ([`crate::net::NetIncomingDisconnected`]), and bounded wait in
//! **WaitingForServer** ([`T_WAIT_MAX`]).
//!
//! **Precedence for default server address** (see `persistence` + `net`):
//! 1. Non-empty `GAME_SERVER_ADDR` environment variable
//! 2. `game_server_addr` in the on-disk preferences file (if valid)
//! 3. [`DEFAULT_GAME_SERVER_ADDR`]
//!
//! **Failure detection** (TASK-14): `net::update_session_lifecycle` tears down on stale snapshot
//! (`is_stale` + [`T_STALE_SNAPSHOT`]), on `NetThreadSignal::TransportFailure` from the UDP thread
//! (send/recv thresholds below), and on implicit loss if the network thread stops servicing the
//! outgoing channel (`Disconnected`). See `RUNBOOK.md` for manual checks.

use std::time::Duration;

/// Default UDP server address when env and saved preferences do not supply one.
pub const DEFAULT_GAME_SERVER_ADDR: &str = "127.0.0.1:4000";

/// Outbound keepalive / retry interval while waiting for the first qualifying snapshot (P1).
pub const T_RETRY: Duration = Duration::from_secs(2);

/// Max wall time in **WaitingForServer** before moving to **Disconnected** with Retry (P1).
pub const T_WAIT_MAX: Duration = Duration::from_secs(45);

/// **Connected**: no qualifying snapshot for this continuous wall time ⇒ session loss (P3).
pub const T_STALE_SNAPSHOT: Duration = Duration::from_secs(3);

/// Consecutive UDP recv errors (excluding `WouldBlock`) before transport is treated as failed (P3).
pub const TRANSPORT_CONSECUTIVE_RECV_ERRORS: u32 = 8;

/// Consecutive UDP send failures before transport is treated as failed (P3).
pub const TRANSPORT_CONSECUTIVE_SEND_ERRORS: u32 = 6;

/// Returns true when `last` is missing or older than `limit` relative to `now`.
pub fn is_stale(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
    limit: Duration,
) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stale_none_is_stale() {
        let now = std::time::Instant::now();
        assert!(is_stale(None, now, Duration::from_secs(1)));
    }

    #[test]
    fn is_stale_recent_not_stale() {
        let now = std::time::Instant::now();
        assert!(!is_stale(Some(now), now, Duration::from_secs(10)));
    }

    #[test]
    fn is_stale_old_is_stale() {
        let now = std::time::Instant::now();
        let old = now.checked_sub(Duration::from_secs(10)).unwrap_or(now);
        assert!(is_stale(Some(old), now, Duration::from_secs(3)));
    }

    /// Frozen TASK-14 policy: keep literals in this module only; tests lock expected playtest values.
    #[test]
    fn task_14_timing_constants_match_frozen_spec() {
        assert_eq!(T_RETRY, Duration::from_secs(2));
        assert_eq!(T_WAIT_MAX, Duration::from_secs(45));
        assert_eq!(T_STALE_SNAPSHOT, Duration::from_secs(3));
        assert_eq!(TRANSPORT_CONSECUTIVE_RECV_ERRORS, 8);
        assert_eq!(TRANSPORT_CONSECUTIVE_SEND_ERRORS, 6);
        assert_eq!(DEFAULT_GAME_SERVER_ADDR, "127.0.0.1:4000");
    }
}
