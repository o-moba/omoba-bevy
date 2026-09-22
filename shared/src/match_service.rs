//! Public lobby queue and immutable worker handoff contracts.
use crate::map::Team;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MatchPreference {
    #[default]
    Quick,
    HumansOnly,
    BotPractice,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchAllocation {
    pub allocation_id: String,
    pub endpoint: String,
    pub preference: MatchPreference,
    pub team: Team,
    pub human_count: u32,
    pub bot_count: u32,
    pub rated: bool,
    pub join_deadline_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MatchServiceView {
    #[default]
    Idle,
    Waiting {
        preference: MatchPreference,
        humans: u32,
        needed: u32,
        elapsed_secs: u64,
        bot_fill_after_secs: Option<u64>,
        capacity_wait: bool,
    },
    Allocating,
    Assigned {
        allocation: MatchAllocation,
    },
    Failed {
        code: String,
    },
}
