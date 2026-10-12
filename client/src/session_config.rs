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
//! **Precedence for default server address** (see `persistence` + `net`):
//! 1. Non-empty `GAME_SERVER_ADDR` environment variable
//! 2. `game_server_addr` in the on-disk preferences file (if valid)
//! 3. [`DEFAULT_GAME_SERVER_ADDR`]
//!
//! **Contour builds.** A build with `OMOBA_DEFAULT_CONFIG_DIR_SUFFIX` (a dev-contour
//! tester build from `scripts/release.py build … --contour dev`) never offers or
//! accepts the production lobby: its online preset is its own compiled-in lobby, and
//! [`endpoint_allowed`] refuses the production VPS and every production `omoba.io` /
//! `ekza.io` name in every address it validates (typed, saved, `GAME_SERVER_ADDR`,
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

/// Public zones; inside them only `<contour>.omoba.io` / `<contour>.ekza.io` names
/// belong to a contour build.
const PUBLIC_ZONES: &[&str] = &["omoba.io", "ekza.io"];

/// The contour this build was made for: `Some("dev")` for a dev-contour tester build,
/// `None` for production. The same compile-time value names the settings folder
/// (`crate::platform`).
pub(crate) const BUILD_CONTOUR: Option<&str> = match option_env!("OMOBA_DEFAULT_CONFIG_DIR_SUFFIX")
{
    Some(suffix) if !suffix.is_empty() => Some(suffix),
    _ => None,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerPreset {
    /// The online lobby: the public beta, or a contour build's own lobby.
    Beta,
    Local,
}

impl ServerPreset {
    pub(crate) fn address(self) -> &'static str {
        preset_address(self, BUILD_CONTOUR, DEFAULT_GAME_SERVER_ADDR)
    }

    /// Localization key of the preset button.
    pub(crate) fn label_key(self) -> &'static str {
        preset_label_key(self, BUILD_CONTOUR)
    }
}

fn preset_address(
    preset: ServerPreset,
    contour: Option<&str>,
    default: &'static str,
) -> &'static str {
    match preset {
        ServerPreset::Local => FALLBACK_GAME_SERVER_ADDR,
        ServerPreset::Beta if contour.is_some() => default,
        ServerPreset::Beta => BETA_GAME_SERVER_ADDR,
    }
}

fn preset_label_key(preset: ServerPreset, contour: Option<&str>) -> &'static str {
    match preset {
        ServerPreset::Local => "phone.server.local",
        ServerPreset::Beta if contour.is_some() => "phone.server.dev",
        ServerPreset::Beta => "phone.server.beta",
    }
}

/// Whether this build may connect to `host` (an IP literal or a DNS name, without
/// the port). Production builds accept every host; a contour build refuses the
/// production lobby.
pub(crate) fn endpoint_allowed(host: &str) -> bool {
    endpoint_allowed_for(BUILD_CONTOUR, host)
}

fn endpoint_allowed_for(contour: Option<&str>, host: &str) -> bool {
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

/// Default UDP server address when env and saved preferences do not supply one. A
/// contour build without a compiled-in lobby falls back to the local server, never to
/// the production beta.
pub const DEFAULT_GAME_SERVER_ADDR: &str = match option_env!("OMOBA_DEFAULT_GAME_SERVER_ADDR") {
    Some(address) => address,
    None if BUILD_CONTOUR.is_some() => FALLBACK_GAME_SERVER_ADDR,
    None => BETA_GAME_SERVER_ADDR,
};

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
        assert_eq!(
            DEFAULT_GAME_SERVER_ADDR,
            option_env!("OMOBA_DEFAULT_GAME_SERVER_ADDR").unwrap_or(if BUILD_CONTOUR.is_some() {
                FALLBACK_GAME_SERVER_ADDR
            } else {
                BETA_GAME_SERVER_ADDR
            })
        );
    }

    fn host_of(address: &str) -> &str {
        address.rsplit_once(':').map_or(address, |(host, _)| host)
    }

    #[test]
    fn production_presets_are_the_public_beta_and_localhost() {
        assert_eq!(
            preset_address(ServerPreset::Beta, None, "ignored:1"),
            BETA_GAME_SERVER_ADDR
        );
        assert_eq!(
            preset_address(ServerPreset::Local, None, "ignored:1"),
            FALLBACK_GAME_SERVER_ADDR
        );
        assert_eq!(
            preset_label_key(ServerPreset::Beta, None),
            "phone.server.beta"
        );
        assert!(endpoint_allowed_for(None, "77.246.105.57"));
        assert!(endpoint_allowed_for(None, "play.dev.omoba.io"));
    }

    #[test]
    fn a_dev_build_exposes_no_production_preset() {
        let dev = Some("dev");
        for default in ["lobby.dev.omoba.io:4100", "203.0.113.10:4100"] {
            assert_eq!(preset_address(ServerPreset::Beta, dev, default), default);
            for preset in [ServerPreset::Beta, ServerPreset::Local] {
                let address = preset_address(preset, dev, default);
                assert_ne!(address, BETA_GAME_SERVER_ADDR);
                assert!(endpoint_allowed_for(dev, host_of(address)), "{address}");
            }
        }
        assert_eq!(
            preset_label_key(ServerPreset::Beta, dev),
            "phone.server.dev"
        );
        assert_eq!(
            preset_label_key(ServerPreset::Local, dev),
            "phone.server.local"
        );
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
    fn this_builds_presets_are_connectable() {
        for preset in [ServerPreset::Beta, ServerPreset::Local] {
            assert!(endpoint_allowed(host_of(preset.address())), "{preset:?}");
            assert!(
                crate::persistence::validate_game_server_addr(preset.address()).is_some(),
                "{preset:?}"
            );
        }
        assert_eq!(
            BUILD_CONTOUR.is_some(),
            ServerPreset::Beta.address() != BETA_GAME_SERVER_ADDR
        );
    }
}
