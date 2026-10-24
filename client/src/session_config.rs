//! Named timing thresholds for multiplayer session lifecycle (TASK-14).
//! Values are tuned for local playtest; the lifecycle is summarized in `docs/features.md`
//! (client session lifecycle) and `docs/ARCHITECTURE.md` (protocol rules).
//!
//! **Failure detection** (client): stale qualifying snapshot ([`T_STALE_SNAPSHOT`]), transport
//! send/recv error streaks ([`TRANSPORT_CONSECUTIVE_SEND_ERRORS`],
//! [`TRANSPORT_CONSECUTIVE_RECV_ERRORS`]), UDP thread signals ([`crate::net::transport::NetThreadSignal`]),
//! snapshot channel disconnect ([`crate::net::NetIncomingDisconnected`]), and bounded wait in
//! **WaitingForServer** ([`T_WAIT_MAX`]).
//!
//! **Server address at startup** ([`startup_server_addr`], used by `net`):
//! 1. Desktop only: a non-empty `GAME_SERVER_ADDR` environment variable
//! 2. The running connection profile's lobby (`crate::connection_profile`):
//!    production's public lobby, or a contour build's chosen profile
//!
//! **Contour builds.** A build with `OMOBA_DEFAULT_CONFIG_DIR_SUFFIX` (a dev-contour
//! tester build from `scripts/release.py build … --contour dev`) never offers or
//! accepts the production lobby: its profiles are Dev, Local and Custom, and
//! [`endpoint_allowed_for`] refuses the production VPS and every production `omoba.io` /
//! `ekza.io` name in every address it validates (custom, `GAME_SERVER_ADDR`,
//! match allocations).
//!
//! **Failure detection** (TASK-14): `net::update_session_lifecycle` tears down on stale snapshot
//! (`is_stale` + [`T_STALE_SNAPSHOT`]), on `NetThreadSignal::TransportFailure` from the UDP thread
//! (send/recv thresholds below), and on implicit loss if the network thread stops servicing the
//! outgoing channel (`Disconnected`). See `RUNBOOK.md` for manual checks.

use std::time::Duration;

/// Safe local fallback even when a developer supplies an invalid build-time endpoint.
pub const FALLBACK_GAME_SERVER_ADDR: &str = "127.0.0.1:4000";

/// Public beta lobby. Match workers are allocated by this endpoint.
pub const BETA_GAME_SERVER_ADDR: &str = "77.246.105.57:4000";

/// Hosts of the production lobby. A contour build never connects to them, on any port.
const PRODUCTION_LOBBY_HOSTS: &[std::net::Ipv4Addr] = &[std::net::Ipv4Addr::new(77, 246, 105, 57)];

/// Public production zones (global `omoba.io` and `ekza.io`, the RU market's `omoba.ru`);
/// inside them only `<contour>.<zone>` names belong to a contour build.
const PUBLIC_ZONES: &[&str] = &["omoba.io", "ekza.io", "omoba.ru"];

/// The contour this build was made for: `Some("dev")` for a dev-contour tester build,
/// `None` for production. The same compile-time value names the settings folder
/// (`crate::platform`).
pub(crate) const BUILD_CONTOUR: Option<&str> = match option_env!("OMOBA_DEFAULT_CONFIG_DIR_SUFFIX")
{
    Some(suffix) if !suffix.is_empty() => Some(suffix),
    _ => None,
};

/// The Omoba market a player is pointed to for free items, without the scheme:
/// the running connection profile's (`app.omoba.io/market` in production,
/// `app.<contour>.omoba.io/market` for a contour build's own servers), so a
/// tester is never sent to the production market. The i18n hints show it as
/// `{market}`. Empty in a build without Ekza, which names no market.
pub(crate) fn market_address() -> String {
    use crate::connection_profile::{active, for_market};
    for_market(active().clone(), shared::market::FEATURES.ekza).market
}

/// The lobby this process connects to unless a desktop `GAME_SERVER_ADDR`
/// overrides it: the running connection profile's.
pub(crate) fn default_server_addr() -> &'static str {
    crate::connection_profile::active().lobby.as_str()
}

/// The lobby at startup: a desktop `GAME_SERVER_ADDR` (developers and
/// self-hosters; phones read none), else the connection profile's lobby. A
/// saved address is no longer an input: production builds have no server
/// selection, and a contour build's profile owns its lobby.
pub(crate) fn startup_server_addr() -> String {
    crate::connection_profile::desktop_override("GAME_SERVER_ADDR")
        .unwrap_or_else(|| default_server_addr().to_owned())
}

/// Whether a build of `contour` may connect to `host` (an IP literal or a DNS
/// name, without the port). Production builds accept every host; a contour build
/// refuses the production lobby and every production `omoba.io` / `ekza.io` / `omoba.ru` name.
/// This build's rule is `endpoint_allowed_for(BUILD_CONTOUR, host)`.
pub(crate) fn endpoint_allowed_for(contour: Option<&str>, host: &str) -> bool {
    let Some(contour) = contour else {
        return true;
    };
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        let v4 = match address {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped(),
        };
        return !v4.is_some_and(|v4| PRODUCTION_LOBBY_HOSTS.contains(&v4));
    }
    let in_zone = |zone: &str| host == zone || host.ends_with(&format!(".{zone}"));
    !PUBLIC_ZONES
        .iter()
        .any(|zone| in_zone(zone) && !in_zone(&format!("{contour}.{zone}")))
}

/// The lobby compiled into this build (`OMOBA_DEFAULT_GAME_SERVER_ADDR`, set by
/// `scripts/release.py --server` or a contour's default), if any. The running
/// lobby is the connection profile's ([`default_server_addr`]): this one, else
/// the public beta in production or the contour's own lobby, never the
/// production beta in a contour build.
pub(crate) const COMPILED_GAME_SERVER_ADDR: Option<&str> =
    option_env!("OMOBA_DEFAULT_GAME_SERVER_ADDR");

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
    }

    #[test]
    fn a_dev_build_refuses_every_production_lobby_address() {
        let dev = Some("dev");
        for host in [
            "77.246.105.57",
            " 77.246.105.57 ",
            "::ffff:77.246.105.57",
            "[::ffff:77.246.105.57]",
            "omoba.io",
            "app.omoba.io",
            "play.omoba.io",
            "LOBBY.OMOBA.IO.",
            "ekza.io",
            "registry.ekza.io",
            "avatar.ekza.io",
            "omoba.ru",
            "lobby.omoba.ru",
            "Account.Omoba.RU.",
        ] {
            assert!(!endpoint_allowed_for(dev, host), "{host}");
        }
        for host in [
            "play.dev.omoba.io",
            "Lobby.Dev.Omoba.IO.",
            "dev.omoba.io",
            "x.dev.ekza.io",
            "203.0.113.10",
            "127.0.0.1",
            "::1",
            "localhost",
            "192.168.1.20",
            "notomoba.io",
            "notomoba.ru",
            "play.dev.omoba.io.example",
        ] {
            assert!(endpoint_allowed_for(dev, host), "{host}");
        }
    }

    #[test]
    fn this_build_accepts_the_production_lobby_only_outside_a_contour() {
        for address in [
            BETA_GAME_SERVER_ADDR,
            "[::ffff:77.246.105.57]:4000",
            "app.omoba.io:4000",
        ] {
            assert_eq!(
                crate::persistence::validate_game_server_addr(address).is_some(),
                BUILD_CONTOUR.is_none(),
                "{address}"
            );
        }
        assert!(crate::persistence::validate_game_server_addr("127.0.0.1:4000").is_some());
    }

    #[test]
    fn this_builds_market_and_lobby_match_its_contour() {
        let address = market_address();
        assert_eq!(address.is_empty(), !shared::market::FEATURES.ekza);
        if shared::market::FEATURES.ekza {
            assert_eq!(address == "app.omoba.io/market", BUILD_CONTOUR.is_none());
            assert!(endpoint_allowed_for(
                BUILD_CONTOUR,
                address.split('/').next().unwrap()
            ));
        }
        let lobby = default_server_addr();
        assert!(crate::persistence::validate_game_server_addr(lobby).is_some());
        if BUILD_CONTOUR.is_none() {
            assert_eq!(
                lobby,
                COMPILED_GAME_SERVER_ADDR.unwrap_or(BETA_GAME_SERVER_ADDR)
            );
        } else {
            assert_ne!(lobby, BETA_GAME_SERVER_ADDR);
        }
    }

    #[test]
    fn the_startup_lobby_is_the_profiles_unless_a_desktop_override_is_set() {
        let expected = std::env::var("GAME_SERVER_ADDR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .filter(|_| !cfg!(any(target_os = "android", target_os = "ios")))
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|| default_server_addr().to_owned());
        assert_eq!(startup_server_addr(), expected);
    }
}
