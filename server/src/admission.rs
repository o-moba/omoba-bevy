//! Per-source admission and abuse limits (LR-16).
//!
//! One source is one IP: an IPv4 address, or an IPv6 /64 (one IPv6
//! subscriber usually holds a whole /64). One network is an IPv4 /24 or an
//! IPv6 /48. Every limit here is a per-process setting read once at start from
//! `OMOBA_LIMIT_*` (`ops/vps/ru/README.md`); the defaults are the production
//! values. Nothing here is market-specific.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// One IP: an IPv4 address or an IPv6 /64.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SourceIp {
    V4(u32),
    V6(u64),
}

/// One network: an IPv4 /24 or an IPv6 /48.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Network {
    V4(u32),
    V6(u64),
}

impl SourceIp {
    pub(crate) fn of(addr: SocketAddr) -> Self {
        match canonical(addr.ip()) {
            IpAddr::V4(ip) => Self::V4(u32::from(ip)),
            IpAddr::V6(ip) => Self::V6((u128::from(ip) >> 64) as u64),
        }
    }

    pub(crate) fn network(self) -> Network {
        match self {
            Self::V4(ip) => Network::V4(ip >> 8),
            Self::V6(prefix) => Network::V6(prefix >> 16),
        }
    }

    /// The text the career store hashes for its new-profile counter: the
    /// address, or the /64 prefix.
    pub(crate) fn key(self) -> String {
        match self {
            Self::V4(ip) => std::net::Ipv4Addr::from(ip).to_string(),
            Self::V6(prefix) => {
                format!("{}/64", std::net::Ipv6Addr::from(u128::from(prefix) << 64))
            }
        }
    }
}

/// An IPv4-mapped IPv6 address is its IPv4 address.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// Every admission limit of one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub budget: BudgetLimits,
    /// Transport endpoints, and career clients, one IP may hold at once.
    pub endpoints_per_ip: usize,
    /// Profiles new device keys may create in 24 hours.
    pub new_profiles: omoba_career_store::career_store::NewProfileLimits,
    /// Running or forming arenas one IP may hold alone (every human of the
    /// match came from it).
    pub arenas_per_ip: usize,
    /// vs Bots allocations from one IP left unplayed within
    /// [`PRACTICE_ABANDON_WINDOW`] before a cooldown.
    pub practice_abandons: usize,
    pub practice_cooldown: Duration,
    /// Retention of profiles that never played; `None` keeps them all.
    pub retention_days: Option<u32>,
}

/// Abandoned vs Bots allocations count within this window.
pub(crate) const PRACTICE_ABANDON_WINDOW: Duration = Duration::from_secs(600);

/// Per-second datagram budgets for transport probes and for compatibility
/// replies, each counted on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BudgetLimits {
    pub per_ip: u32,
    pub per_network: u32,
    pub global: u32,
    /// The part of `global` only a source's first datagram of the second may
    /// use, so busy sources cannot take the whole budget.
    pub new_source_reserve: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            budget: BudgetLimits {
                per_ip: 8,
                per_network: 32,
                global: 64,
                new_source_reserve: 16,
            },
            endpoints_per_ip: 8,
            new_profiles: omoba_career_store::career_store::NewProfileLimits {
                per_source: 10,
                total: 5000,
            },
            arenas_per_ip: 1,
            practice_abandons: 3,
            practice_cooldown: Duration::from_secs(120),
            retention_days: Some(30),
        }
    }
}

impl Limits {
    /// One startup line for the journal.
    pub(crate) fn summary(&self) -> String {
        let b = &self.budget;
        format!(
            "Admission limits: probes {}/s per IP, {}/s per network, {}/s in all ({} kept for new sources); \
             {} endpoints per IP; new profiles per 24 h {} per IP, {} in all; {} arena(s) per IP; \
             vs Bots cooldown {} s after {} unplayed in {} min; profile retention {}",
            b.per_ip,
            b.per_network,
            b.global,
            b.new_source_reserve,
            self.endpoints_per_ip,
            self.new_profiles.per_source,
            self.new_profiles.total,
            self.arenas_per_ip,
            self.practice_cooldown.as_secs(),
            self.practice_abandons,
            PRACTICE_ABANDON_WINDOW.as_secs() / 60,
            self.retention_days
                .map_or_else(|| "off".to_owned(), |days| format!("{days} days")),
        )
    }

    /// The defaults with every `OMOBA_LIMIT_*` and `OMOBA_PROFILE_RETENTION_DAYS`
    /// override from the environment; a malformed value stops the process.
    pub(crate) fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub(crate) fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut limits = Self::default();
        let read = |name: &str, current: u32, max: u32| -> Result<u32, String> {
            match lookup(name).map(|value| value.trim().to_owned()) {
                None => Ok(current),
                Some(value) if value.is_empty() => Ok(current),
                Some(value) => value
                    .parse::<u32>()
                    .ok()
                    .filter(|n| (1..=max).contains(n))
                    .ok_or_else(|| format!("{name} must be a whole number from 1 to {max}")),
            }
        };
        let b = &mut limits.budget;
        b.per_ip = read("OMOBA_LIMIT_PROBES_PER_IP", b.per_ip, 10_000)?;
        b.per_network = read("OMOBA_LIMIT_PROBES_PER_NETWORK", b.per_network, 10_000)?;
        b.global = read("OMOBA_LIMIT_PROBES_GLOBAL", b.global, 10_000)?;
        b.new_source_reserve = match lookup("OMOBA_LIMIT_PROBES_NEW_RESERVE").as_deref() {
            Some("0") => 0,
            _ => read(
                "OMOBA_LIMIT_PROBES_NEW_RESERVE",
                b.new_source_reserve,
                10_000,
            )?,
        };
        if b.new_source_reserve >= b.global {
            return Err(
                "OMOBA_LIMIT_PROBES_NEW_RESERVE must be below OMOBA_LIMIT_PROBES_GLOBAL".into(),
            );
        }
        limits.endpoints_per_ip = read(
            "OMOBA_LIMIT_ENDPOINTS_PER_IP",
            limits.endpoints_per_ip as u32,
            512,
        )? as usize;
        let n = &mut limits.new_profiles;
        n.per_source = read("OMOBA_LIMIT_NEW_PROFILES_PER_IP", n.per_source, 1_000_000)?;
        n.total = read("OMOBA_LIMIT_NEW_PROFILES_TOTAL", n.total, 10_000_000)?;
        limits.arenas_per_ip = read(
            "OMOBA_LIMIT_ARENAS_PER_IP",
            limits.arenas_per_ip as u32,
            100,
        )? as usize;
        limits.practice_abandons = read(
            "OMOBA_LIMIT_PRACTICE_ABANDONS",
            limits.practice_abandons as u32,
            1000,
        )? as usize;
        limits.practice_cooldown = Duration::from_secs(u64::from(read(
            "OMOBA_LIMIT_PRACTICE_COOLDOWN_SECS",
            limits.practice_cooldown.as_secs() as u32,
            86_400,
        )?));
        limits.retention_days = match lookup("OMOBA_PROFILE_RETENTION_DAYS")
            .map(|value| value.trim().to_owned())
            .as_deref()
        {
            None | Some("") => limits.retention_days,
            Some("0") => None,
            Some(value) => Some(
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|days| (7..=3650).contains(days))
                    .ok_or("OMOBA_PROFILE_RETENTION_DAYS must be 0 (off) or 7 to 3650")?,
            ),
        };
        Ok(limits)
    }
}

/// A fixed one-second window of datagram budgets: per IP and per network
/// first, then the global budget, whose last [`BudgetLimits::new_source_reserve`]
/// places only a source new in this second may take. Only admitted datagrams
/// are recorded, so the tables never hold more than `global` entries.
#[derive(Default)]
pub(crate) struct Budget {
    window: Option<Instant>,
    total: u32,
    by_ip: HashMap<SourceIp, u32>,
    by_network: HashMap<Network, u32>,
}

impl Budget {
    fn roll(&mut self, now: Instant) {
        if self
            .window
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
        {
            self.window = Some(now);
            self.total = 0;
            self.by_ip.clear();
            self.by_network.clear();
        }
    }

    /// Whether a datagram from `addr` fits, without spending anything.
    pub(crate) fn allows(&mut self, addr: SocketAddr, limits: &BudgetLimits, now: Instant) -> bool {
        self.roll(now);
        let ip = SourceIp::of(addr);
        let used = self.by_ip.get(&ip).copied().unwrap_or(0);
        let network = self.by_network.get(&ip.network()).copied().unwrap_or(0);
        let global = if used == 0 {
            limits.global
        } else {
            limits.global.saturating_sub(limits.new_source_reserve)
        };
        used < limits.per_ip && network < limits.per_network && self.total < global
    }

    /// Spends one datagram for `addr` when it fits.
    pub(crate) fn admit(&mut self, addr: SocketAddr, limits: &BudgetLimits, now: Instant) -> bool {
        if !self.allows(addr, limits, now) {
            return false;
        }
        let ip = SourceIp::of(addr);
        *self.by_ip.entry(ip).or_default() += 1;
        *self.by_network.entry(ip.network()).or_default() += 1;
        self.total += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 5000)
    }

    #[test]
    fn sources_are_ipv4_addresses_and_ipv6_slash_64s() {
        assert_eq!(SourceIp::of(at("203.0.113.7")).key(), "203.0.113.7");
        assert_eq!(
            SourceIp::of(at("::ffff:203.0.113.7")),
            SourceIp::of(at("203.0.113.7"))
        );
        assert_eq!(
            SourceIp::of(at("2001:db8:1:2:aaaa::1")),
            SourceIp::of(at("2001:db8:1:2:bbbb::9"))
        );
        assert_eq!(
            SourceIp::of(at("2001:db8:1:2::1")).key(),
            "2001:db8:1:2::/64"
        );
        assert_ne!(
            SourceIp::of(at("2001:db8:1:3::1")),
            SourceIp::of(at("2001:db8:1:2::1"))
        );
        assert_eq!(
            SourceIp::of(at("203.0.113.7")).network(),
            SourceIp::of(at("203.0.113.200")).network()
        );
        assert_ne!(
            SourceIp::of(at("203.0.114.7")).network(),
            SourceIp::of(at("203.0.113.7")).network()
        );
        assert_eq!(
            SourceIp::of(at("2001:db8:1:2::1")).network(),
            SourceIp::of(at("2001:db8:1:ff::1")).network()
        );
    }

    #[test]
    fn a_busy_source_never_takes_the_reserve_of_new_sources() {
        let limits = BudgetLimits {
            per_ip: 4,
            per_network: 1000,
            global: 10,
            new_source_reserve: 3,
        };
        let now = Instant::now();
        let mut budget = Budget::default();
        // Each source is held to its own budget.
        assert_eq!(
            (0..100)
                .filter(|_| budget.admit(at("198.51.100.1"), &limits, now))
                .count(),
            4
        );
        assert_eq!(
            (0..100)
                .filter(|_| budget.admit(at("198.51.101.1"), &limits, now))
                .count(),
            3,
            "repeat datagrams stop where the reserve starts"
        );
        // Three new sources still get one place each, then nobody.
        for host in ["192.0.2.1", "192.0.2.2", "192.0.2.3"] {
            assert!(budget.admit(at(host), &limits, now), "{host}");
        }
        assert!(!budget.admit(at("192.0.2.4"), &limits, now));
        // The next second starts afresh.
        assert!(budget.admit(at("192.0.2.4"), &limits, now + Duration::from_secs(1)));
    }

    #[test]
    fn a_network_shares_one_budget() {
        let limits = BudgetLimits {
            per_ip: 4,
            per_network: 6,
            global: 100,
            new_source_reserve: 10,
        };
        let now = Instant::now();
        let mut budget = Budget::default();
        let admitted = (1..=50)
            .filter(|host| budget.admit(at(&format!("203.0.113.{host}")), &limits, now))
            .count();
        assert_eq!(admitted, 6);
        assert!(budget.admit(at("203.0.114.1"), &limits, now));
    }

    #[test]
    fn limits_read_overrides_and_refuse_bad_values() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(Limits::from_lookup(env(&[])).unwrap(), Limits::default());
        let limits = Limits::from_lookup(env(&[
            ("OMOBA_LIMIT_ENDPOINTS_PER_IP", "16"),
            ("OMOBA_LIMIT_NEW_PROFILES_PER_IP", "3"),
            ("OMOBA_LIMIT_ARENAS_PER_IP", "2"),
            ("OMOBA_LIMIT_PRACTICE_COOLDOWN_SECS", "60"),
            ("OMOBA_LIMIT_PROBES_NEW_RESERVE", "0"),
            ("OMOBA_PROFILE_RETENTION_DAYS", "0"),
        ]))
        .unwrap();
        assert_eq!(limits.endpoints_per_ip, 16);
        assert_eq!(limits.new_profiles.per_source, 3);
        assert_eq!(limits.arenas_per_ip, 2);
        assert_eq!(limits.practice_cooldown, Duration::from_secs(60));
        assert_eq!(limits.budget.new_source_reserve, 0);
        assert_eq!(limits.retention_days, None);
        for bad in [
            &[("OMOBA_LIMIT_ENDPOINTS_PER_IP", "0")][..],
            &[("OMOBA_LIMIT_ENDPOINTS_PER_IP", "many")],
            &[("OMOBA_LIMIT_PROBES_NEW_RESERVE", "64")],
            &[("OMOBA_PROFILE_RETENTION_DAYS", "3")],
        ] {
            assert!(Limits::from_lookup(env(bad)).is_err(), "{bad:?}");
        }
    }
}
