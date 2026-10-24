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
        /// The in-match state (LR-13): this allocation's match is already
        /// running without this profile, which left it or lost its
        /// connection. The seat can only be resumed, and `FindMatch` queues
        /// nobody until the match ends. Additive, so protocol 11 clients
        /// without the field still read a plain handoff.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        in_match: Option<InMatch>,
    },
    Failed {
        code: String,
    },
}

/// A running match that still holds this profile's seat (LR-13).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InMatch {
    /// Whole seconds until the match ends at the latest, counted when the
    /// view was built: its hard duration cap, or the abandoned-match reaping
    /// while no human is connected. `None` when the worker did not report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_within_secs: Option<u64>,
}

impl MatchServiceView {
    /// The allocation and state of a running match that still holds this
    /// profile's seat; `None` for every other view, a fresh handoff included.
    pub fn in_match(&self) -> Option<(&MatchAllocation, InMatch)> {
        match self {
            Self::Assigned {
                allocation,
                in_match: Some(state),
            } => Some((allocation, *state)),
            _ => None,
        }
    }
}

impl InMatch {
    /// Whole minutes, rounded up and at least one, until the match ends at
    /// the latest.
    pub fn ends_within_minutes(&self) -> Option<u64> {
        self.ends_within_secs.map(|secs| secs.div_ceil(60).max(1))
    }
}

/// Team decision for an allocated human who is temporarily disconnected.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TakeoverPolicy {
    Idle,
    #[default]
    Bot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TakeoverSeatView {
    pub player_id: u64,
    pub generation: u64,
    pub nickname: String,
    pub policy: TakeoverPolicy,
    pub idle_votes: u32,
    pub bot_votes: u32,
    pub eligible_voters: u32,
    pub my_vote: Option<TakeoverPolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allocation() -> MatchAllocation {
        MatchAllocation {
            allocation_id: "a".repeat(32),
            endpoint: "127.0.0.1:41000".into(),
            preference: MatchPreference::Quick,
            team: Team::Blue,
            human_count: 3,
            bot_count: 7,
            rated: false,
            join_deadline_ms: 1,
        }
    }

    /// The `MatchServiceView` that protocol 11 clients before LR-13 decode.
    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(tag = "state", rename_all = "snake_case")]
    enum LegacyView {
        Assigned { allocation: MatchAllocation },
    }

    /// LR-13: the in-match state is additive. A plain handoff keeps its
    /// bytes, and an older client reads the in-match state as a handoff.
    #[test]
    fn the_in_match_state_is_additive_on_the_wire() {
        let handoff = MatchServiceView::Assigned {
            allocation: allocation(),
            in_match: None,
        };
        let bytes = serde_json::to_string(&handoff).unwrap();
        assert!(!bytes.contains("in_match"), "{bytes}");
        assert_eq!(
            serde_json::from_str::<MatchServiceView>(&bytes).unwrap(),
            handoff
        );
        assert_eq!(handoff.in_match(), None);

        let running = MatchServiceView::Assigned {
            allocation: allocation(),
            in_match: Some(InMatch {
                ends_within_secs: Some(61),
            }),
        };
        let bytes = serde_json::to_string(&running).unwrap();
        assert!(
            bytes.contains(r#""in_match":{"ends_within_secs":61}"#),
            "{bytes}"
        );
        assert_eq!(
            serde_json::from_str::<MatchServiceView>(&bytes).unwrap(),
            running
        );
        assert_eq!(
            serde_json::from_str::<LegacyView>(&bytes).unwrap(),
            LegacyView::Assigned {
                allocation: allocation()
            }
        );
        let (seat, state) = running.in_match().unwrap();
        assert_eq!(seat.allocation_id, "a".repeat(32));
        assert_eq!(state.ends_within_minutes(), Some(2));
        // An unknown end decodes from an empty object.
        let unknown: MatchServiceView =
            serde_json::from_str(&bytes.replace(r#"{"ends_within_secs":61}"#, "{}")).unwrap();
        assert_eq!(unknown.in_match().unwrap().1.ends_within_minutes(), None);
    }

    #[test]
    fn the_end_rounds_up_to_whole_minutes() {
        let minutes = |secs| {
            InMatch {
                ends_within_secs: Some(secs),
            }
            .ends_within_minutes()
        };
        assert_eq!(minutes(0), Some(1));
        assert_eq!(minutes(59), Some(1));
        assert_eq!(minutes(60), Some(1));
        assert_eq!(minutes(2700), Some(45));
        assert_eq!(InMatch::default().ends_within_minutes(), None);
    }
}
