//! Live round data; deliberately independent of durable career result schemas.
use crate::{HeroClass, map::Team};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveScoreboard {
    pub players: Vec<LiveScorePlayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveScorePlayer {
    pub player_id: u64,
    pub nickname: String,
    pub team: Team,
    pub hero_class: HeroClass,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    /// Income accrued this round, excluding the starting wallet grant.
    pub earned_gold: u32,
    pub level: u32,
    pub connected: bool,
}
