//! Online host compatibility imports for the shared combat engine.
#[allow(unused_imports)]
pub(crate) use common::match_rules::*;

pub(crate) fn from_env() -> MatchConfig {
    MatchConfig {
        mode: parse_match_mode(std::env::var("OMOBA_MATCH_MODE").ok().as_deref()),
        team_size: parse_team_size(std::env::var("OMOBA_TEAM_SIZE").ok().as_deref()),
    }
}
