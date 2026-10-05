//! Live round data; deliberately independent of durable career result schemas.
use crate::{HeroClass, map::Team};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveScoreboard {
    /// Authoritative running simulation time, rounded down to whole seconds.
    pub elapsed_secs: u64,
    /// Public kill notices carry identities only, never hidden positions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kills: Vec<KillNotice>,
    pub players: Vec<LiveScorePlayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveScorePlayer {
    pub player_id: u64,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub team: Team,
    pub hero_class: HeroClass,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    /// Income accrued this round, excluding the starting wallet grant.
    pub earned_gold: u32,
    pub level: u32,
    pub connected: bool,
    /// Public death timer only; never exposes a hidden position.
    #[serde(default)]
    pub respawn_remaining_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillNotice {
    pub event_id: u64,
    pub killer_id: u64,
    pub victim_id: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn old_live_player_defaults_death_timer_without_position_fields() {
        let row: super::LiveScorePlayer = serde_json::from_value(serde_json::json!({
            "player_id": 7, "nickname": "Opponent", "team": "blue", "hero_class": "mage",
            "kills": 0, "deaths": 0, "assists": 0, "earned_gold": 0, "level": 1, "connected": true
        }))
        .unwrap();
        assert_eq!(row.respawn_remaining_ms, 0);
        let value = serde_json::to_value(&row).unwrap();
        assert!(value.get("position").is_none() && value.get("hp").is_none());
    }

    #[test]
    fn legacy_scoreboard_has_zero_clock_without_fabricating_match_time() {
        let board: super::LiveScoreboard = serde_json::from_str(r#"{"players":[]}"#).unwrap();
        assert_eq!(board.elapsed_secs, 0);
    }
}
