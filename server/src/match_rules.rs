//! Match mode and the policy derived from it.
//!
//! `MatchConfig` is the parsed input (`OMOBA_MATCH_MODE`, `OMOBA_TEAM_SIZE`,
//! or the worker manifest). `MatchRules` is the one place that turns the mode
//! into decisions: every field is a plain value fixed once in
//! [`MatchRules::for_mode`], and the rest of the server reads the field it
//! needs instead of comparing the mode. Runtime conditions that combine with
//! a rule (`match_service.worker()`, the Combat Sandbox being enabled, a
//! prematch-capable join) stay at the site that owns them.

/// How matches are allowed to start.
///
/// * `Release` — production-like: the match forms to a full
///   `2 x team_size` roster with server-assigned balanced teams before it
///   starts. Safe default.
/// * `Dev` — local development: the first join starts the match immediately
///   and the client-chosen team is honored (the pre-TASK-22 behavior).
/// * `Practice` — solo start against server bots; local results only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchMode {
    Release,
    Dev,
    Practice,
}

impl MatchMode {
    /// The wire and log name of the mode (`match_mode` in the snapshot).
    pub(crate) fn id(self) -> &'static str {
        match self {
            MatchMode::Release => "release",
            MatchMode::Dev => "dev",
            MatchMode::Practice => "practice",
        }
    }
}

/// The parsed match settings before any policy is derived from them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MatchConfig {
    pub(crate) mode: MatchMode,
    pub(crate) team_size: u32,
}

pub(crate) const DEFAULT_TEAM_SIZE: u32 = 5;

pub(crate) const MIN_TEAM_SIZE: u32 = 1;

pub(crate) const MAX_TEAM_SIZE: u32 = 16;

pub(crate) fn parse_match_mode(raw: Option<&str>) -> MatchMode {
    match raw
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") | Some("release") | Some("normal") => MatchMode::Release,
        Some("dev") | Some("debug") => MatchMode::Dev,
        Some("practice") => MatchMode::Practice,
        Some(other) => {
            eprintln!(
                "Unknown OMOBA_MATCH_MODE '{other}' - falling back to release (expected 'release', 'dev' or 'practice')"
            );
            MatchMode::Release
        }
    }
}

pub(crate) fn parse_team_size(raw: Option<&str>) -> u32 {
    let Some(raw) = raw else {
        return DEFAULT_TEAM_SIZE;
    };
    match raw.trim().parse::<u32>() {
        Ok(value) => value.clamp(MIN_TEAM_SIZE, MAX_TEAM_SIZE),
        Err(_) => {
            eprintln!("Invalid OMOBA_TEAM_SIZE '{raw}' - using default {DEFAULT_TEAM_SIZE}");
            DEFAULT_TEAM_SIZE
        }
    }
}

impl MatchConfig {
    pub(crate) fn from_env() -> Self {
        Self {
            mode: parse_match_mode(std::env::var("OMOBA_MATCH_MODE").ok().as_deref()),
            team_size: parse_team_size(std::env::var("OMOBA_TEAM_SIZE").ok().as_deref()),
        }
    }

    /// Instant-start config used by unit tests and as the documented dev
    /// baseline.
    #[cfg(test)]
    pub(crate) fn dev() -> Self {
        Self {
            mode: MatchMode::Dev,
            team_size: DEFAULT_TEAM_SIZE,
        }
    }

    #[cfg(test)]
    pub(crate) fn release(team_size: u32) -> Self {
        Self {
            mode: MatchMode::Release,
            team_size,
        }
    }
}

/// Who decides the team of a joining player when the match service has not
/// already allocated a seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamAssignment {
    /// The server balances teams; a rejoining player keeps its team.
    Balanced,
    /// The client's requested team is honored while the roster has room
    /// (the Combat Sandbox and a prematch-capable join override it at the
    /// site).
    ClientChoice,
    /// The human takes the seat of the bot it replaces.
    PracticeSeat,
}

/// When a lobby becomes a running match without a prematch draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartPolicy {
    /// The first join starts the match immediately.
    FirstJoin,
    /// Joins form the roster; a full roster starts the countdown.
    FullRoster,
}

/// How many joined players the prematch draft waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RosterPolicy {
    /// Whoever is joined (at least one) is the roster.
    Present,
    /// The full `2 x team_size` roster.
    Full,
}

/// The policy derived from the match mode, decided once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatchRules {
    /// Kept for the startup banner, logs and the `match_mode` wire string.
    pub(crate) mode: MatchMode,
    pub(crate) team_size: u32,
    /// Team of a joining player without an allocated seat.
    pub(crate) team_assignment: TeamAssignment,
    /// How a lobby without a prematch draft becomes a running match.
    pub(crate) start: StartPolicy,
    /// Roster the prematch draft waits for.
    pub(crate) prematch_roster: RosterPolicy,
    /// The server seats bots on the empty seats, replaces a bot with a joining
    /// human, drops the bots on a round reset and runs `simulate_bots`. A
    /// worker-allocated practice match fills once from the manifest instead;
    /// that check stays at the site.
    pub(crate) fills_with_bots: bool,
    /// `SetGodMode` / `SetSpeedBoost` are accepted.
    pub(crate) debug_commands: bool,
    /// The Combat Sandbox may be enabled (startup also requires a loopback
    /// bind and no public role).
    pub(crate) combat_sandbox_allowed: bool,
    /// The ranked career queue applies (with a career backend and no public
    /// role, checked at the site).
    pub(crate) career_credit: bool,
    /// Results are local: never rated, no career checkpoint, and no
    /// "did not finish" reason; a worker-allocated match still settles a
    /// public-casual result (checked at the site).
    pub(crate) local_results: bool,
    /// The career hero-select flow applies: guest rounds cap their
    /// participants, a round reset un-joins everyone in the career flow, and
    /// play-again waits for the saved result.
    pub(crate) career_flow: bool,
}

impl MatchRules {
    /// The one place that turns a mode into decisions.
    pub(crate) fn for_mode(mode: MatchMode, team_size: u32) -> Self {
        let (team_assignment, start, prematch_roster) = match mode {
            MatchMode::Release => (
                TeamAssignment::Balanced,
                StartPolicy::FullRoster,
                RosterPolicy::Full,
            ),
            MatchMode::Dev => (
                TeamAssignment::ClientChoice,
                StartPolicy::FirstJoin,
                RosterPolicy::Present,
            ),
            MatchMode::Practice => (
                TeamAssignment::PracticeSeat,
                StartPolicy::FirstJoin,
                RosterPolicy::Full,
            ),
        };
        let practice = mode == MatchMode::Practice;
        Self {
            mode,
            team_size,
            team_assignment,
            start,
            prematch_roster,
            fills_with_bots: practice,
            debug_commands: matches!(mode, MatchMode::Dev | MatchMode::Practice),
            combat_sandbox_allowed: mode == MatchMode::Dev,
            career_credit: mode == MatchMode::Release,
            local_results: practice,
            career_flow: !practice,
        }
    }

    pub(crate) fn mode_id(&self) -> &'static str {
        self.mode.id()
    }

    pub(crate) fn roster_size(&self) -> u32 {
        self.team_size * 2
    }

    #[cfg(test)]
    pub(crate) fn dev() -> Self {
        MatchConfig::dev().into()
    }

    #[cfg(test)]
    pub(crate) fn release(team_size: u32) -> Self {
        MatchConfig::release(team_size).into()
    }

    #[cfg(test)]
    pub(crate) fn practice(team_size: u32) -> Self {
        Self::for_mode(MatchMode::Practice, team_size)
    }
}

impl From<MatchConfig> for MatchRules {
    fn from(config: MatchConfig) -> Self {
        Self::for_mode(config.mode, config.team_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy table: one row per mode, every field spelled out. A change
    /// here is a behaviour change and needs a test in the module that reads
    /// the field.
    #[test]
    fn rules_table_per_mode() {
        assert_eq!(
            MatchRules::release(5),
            MatchRules {
                mode: MatchMode::Release,
                team_size: 5,
                team_assignment: TeamAssignment::Balanced,
                start: StartPolicy::FullRoster,
                prematch_roster: RosterPolicy::Full,
                fills_with_bots: false,
                debug_commands: false,
                combat_sandbox_allowed: false,
                career_credit: true,
                local_results: false,
                career_flow: true,
            }
        );
        assert_eq!(
            MatchRules::dev(),
            MatchRules {
                mode: MatchMode::Dev,
                team_size: DEFAULT_TEAM_SIZE,
                team_assignment: TeamAssignment::ClientChoice,
                start: StartPolicy::FirstJoin,
                prematch_roster: RosterPolicy::Present,
                fills_with_bots: false,
                debug_commands: true,
                combat_sandbox_allowed: true,
                career_credit: false,
                local_results: false,
                career_flow: true,
            }
        );
        assert_eq!(
            MatchRules::practice(2),
            MatchRules {
                mode: MatchMode::Practice,
                team_size: 2,
                team_assignment: TeamAssignment::PracticeSeat,
                start: StartPolicy::FirstJoin,
                prematch_roster: RosterPolicy::Full,
                fills_with_bots: true,
                debug_commands: true,
                combat_sandbox_allowed: false,
                career_credit: false,
                local_results: true,
                career_flow: false,
            }
        );
    }

    #[test]
    fn rules_derive_from_config_and_keep_the_mode_id() {
        let rules: MatchRules = MatchConfig {
            mode: MatchMode::Practice,
            team_size: 3,
        }
        .into();
        assert_eq!(rules, MatchRules::practice(3));
        assert_eq!(rules.roster_size(), 6);
        assert_eq!(rules.mode_id(), "practice");
        assert_eq!(MatchRules::dev().mode_id(), "dev");
        assert_eq!(MatchRules::release(1).mode_id(), "release");
    }
}
