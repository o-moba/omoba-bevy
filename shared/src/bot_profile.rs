//! Behavioural bot difficulty (docs/plans/bot-personas.md).
//!
//! A level changes only when a bot commits, when it backs off and what it
//! prefers to hit. Reaction time, aim, steering, the think and route cadence,
//! cast order, shopping, stats and gold are the same at every level. The
//! gameplay knobs per level live in `common::bots::BotProfile`.
use serde::{Deserialize, Serialize};

/// How boldly a team's bots play. `Elite` is the original bots, and the only
/// level online: public lobbies, Quick-match fill, Server bot matches and
/// takeover of a disconnected human always play at `Elite`.
///
/// Wire format: snake_case string. The type is not on the UDP protocol yet
/// (the practice command and the snapshot echo come with the picker), but it
/// is tolerant already: an unknown value decodes as `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotDifficulty {
    Recruit,
    Regular,
    Veteran,
    /// The original bots, exactly.
    #[default]
    Elite,
    /// A level this build does not know, such as a newer host's. It plays as
    /// `Elite` and is shown as a plain "AI" with no level name, so it is
    /// never mislabelled. Never sent.
    #[serde(other)]
    Unknown,
}

impl BotDifficulty {
    /// The selectable levels, weakest first.
    pub const ALL: [Self; 4] = [Self::Recruit, Self::Regular, Self::Veteran, Self::Elite];

    /// Operator override for practice hosts only: a standalone practice
    /// server and offline practice read it. The lobby removes it from every
    /// match worker's environment, and workers never read it.
    pub const ENV: &'static str = "OMOBA_BOT_DIFFICULTY";

    /// The level this build plays: an unknown level plays as `Elite`.
    pub fn effective(self) -> Self {
        match self {
            Self::Unknown => Self::Elite,
            level => level,
        }
    }

    /// The wire and operator name: `recruit`, `regular`, `veteran`, `elite`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Recruit => "recruit",
            Self::Regular => "regular",
            Self::Veteran => "veteran",
            Self::Elite => "elite",
            Self::Unknown => "unknown",
        }
    }

    /// Parse an operator value, ignoring case and surrounding space. Only the
    /// selectable levels parse.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|level| level.name().eq_ignore_ascii_case(value))
    }

    /// The [`Self::ENV`] override: `Ok(None)` when unset or empty, `Err` with
    /// the raw value when it names no level.
    pub fn from_env_value(value: Option<&str>) -> Result<Option<Self>, String> {
        match value.map(str::trim) {
            None | Some("") => Ok(None),
            Some(value) => Self::parse(value).map(Some).ok_or_else(|| value.to_owned()),
        }
    }

    /// Read [`Self::ENV`] (see [`Self::from_env_value`]).
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_env_value(std::env::var(Self::ENV).ok().as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_round_trip_and_unknown_values_play_as_elite() {
        for level in BotDifficulty::ALL {
            let json = serde_json::to_string(&level).unwrap();
            assert_eq!(json, format!("\"{}\"", level.name()));
            assert_eq!(serde_json::from_str::<BotDifficulty>(&json).unwrap(), level);
            assert_eq!(level.effective(), level);
        }
        let future: BotDifficulty = serde_json::from_str("\"grandmaster\"").unwrap();
        assert_eq!(future, BotDifficulty::Unknown);
        assert_eq!(future.effective(), BotDifficulty::Elite);
        assert_eq!(BotDifficulty::default(), BotDifficulty::Elite);
    }

    #[test]
    fn operator_values_parse_only_selectable_levels() {
        assert_eq!(
            BotDifficulty::parse(" Recruit "),
            Some(BotDifficulty::Recruit)
        );
        assert_eq!(BotDifficulty::parse("ELITE"), Some(BotDifficulty::Elite));
        assert_eq!(BotDifficulty::parse("unknown"), None);
        assert_eq!(BotDifficulty::parse("top"), None);
        assert_eq!(BotDifficulty::from_env_value(None), Ok(None));
        assert_eq!(BotDifficulty::from_env_value(Some("  ")), Ok(None));
        assert_eq!(
            BotDifficulty::from_env_value(Some("veteran")),
            Ok(Some(BotDifficulty::Veteran))
        );
        assert_eq!(
            BotDifficulty::from_env_value(Some("hard")),
            Err("hard".to_owned())
        );
    }
}
