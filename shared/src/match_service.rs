//! Public lobby queue and immutable worker handoff contracts.
use crate::map::Team;
use serde::{Deserialize, Serialize};

/// The queue a player chooses (LR-08).
///
/// The wire names predate the Ranked/Casual split and stay as they are, so
/// protocol 11 clients keep working: an old Quick request (`quick`) now
/// searches Casual PvP, and `humans_only` is Ranked. Renaming them on the wire
/// needs a protocol bump.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MatchPreference {
    /// Casual PvP: every human in the queue, whatever their rating or
    /// newcomer cohort, then bots fill the empty seats. Never rated; awards
    /// public casual XP.
    #[default]
    #[serde(rename = "quick")]
    Casual,
    /// Ranked: ten humans of one rating cohort, never filled with bots.
    /// Rated when the match is eligible.
    #[serde(rename = "humans_only")]
    Ranked,
    /// vs Bots: one player or one party against server bots, at once.
    BotPractice,
}

impl MatchPreference {
    /// Only Ranked matches can change rating.
    pub fn rated(self) -> bool {
        self == Self::Ranked
    }
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
        /// Players searching this queue now, every rating cohort included
        /// (LR-08). Additive, like the two fields below: protocol 11 clients
        /// without them ignore them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        searching: Option<u32>,
        /// Ranked only: elapsed seconds after which the player may also search
        /// Casual PvP without leaving Ranked (owner default O-1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        casual_offer_after_secs: Option<u64>,
        /// This Ranked search also searches Casual PvP. For a party, only once
        /// every member asked for it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        also_casual: bool,
        /// Every arena is busy and this player's roster is ready (LR-09): its
        /// place in the line for the next free arena and the estimated wait.
        /// Set exactly when `capacity_wait` is. Additive: protocol 11 clients
        /// without it show the plain capacity wait.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capacity_line: Option<CapacityLine>,
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

/// A ready roster's place in the line for a free arena (LR-09).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapacityLine {
    /// 1 = this roster gets the next arena that frees.
    pub position: u32,
    /// Estimated whole seconds until then, from how fast arenas freed
    /// recently. `None` until the lobby has seen arenas free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_secs: Option<u64>,
}

impl CapacityLine {
    /// The estimate in whole minutes, rounded up and at least one.
    pub fn eta_minutes(&self) -> Option<u64> {
        self.eta_secs.map(|secs| secs.div_ceil(60).max(1))
    }
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
            preference: MatchPreference::Casual,
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

    /// LR-08: Ranked and Casual are distinct variants, but the wire names of
    /// protocol 11 stay: an old client's Quick request reads as Casual and its
    /// HumansOnly as Ranked.
    #[test]
    fn queue_names_keep_the_protocol_11_wire_names() {
        for (preference, wire) in [
            (MatchPreference::Casual, "\"quick\""),
            (MatchPreference::Ranked, "\"humans_only\""),
            (MatchPreference::BotPractice, "\"bot_practice\""),
        ] {
            assert_eq!(serde_json::to_string(&preference).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<MatchPreference>(wire).unwrap(),
                preference
            );
        }
        assert_eq!(MatchPreference::default(), MatchPreference::Casual);
        assert!(MatchPreference::Ranked.rated());
        assert!(!MatchPreference::Casual.rated());
        assert!(!MatchPreference::BotPractice.rated());
    }

    /// The `Waiting` view that protocol 11 clients before LR-08 decode.
    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(tag = "state", rename_all = "snake_case")]
    enum LegacyWaiting {
        Waiting {
            preference: MatchPreference,
            humans: u32,
            needed: u32,
            elapsed_secs: u64,
            bot_fill_after_secs: Option<u64>,
            capacity_wait: bool,
        },
    }

    /// LR-08: the queue count and the Casual offer are additive. A view
    /// without them keeps its bytes, and an older client reads a plain wait.
    #[test]
    fn the_queue_count_and_the_casual_offer_are_additive_on_the_wire() {
        let waiting = |searching, offer, also_casual| MatchServiceView::Waiting {
            preference: MatchPreference::Ranked,
            humans: 3,
            needed: 10,
            elapsed_secs: 301,
            bot_fill_after_secs: None,
            capacity_wait: false,
            searching,
            casual_offer_after_secs: offer,
            also_casual,
            capacity_line: None,
        };
        let plain = serde_json::to_string(&waiting(None, None, false)).unwrap();
        for field in ["searching", "casual_offer_after_secs", "also_casual"] {
            assert!(!plain.contains(field), "{plain}");
        }
        let full = waiting(Some(7), Some(300), true);
        let bytes = serde_json::to_string(&full).unwrap();
        assert!(
            bytes.contains(r#""searching":7,"casual_offer_after_secs":300,"also_casual":true"#),
            "{bytes}"
        );
        assert_eq!(
            serde_json::from_str::<MatchServiceView>(&bytes).unwrap(),
            full
        );
        assert_eq!(
            serde_json::from_str::<LegacyWaiting>(&bytes).unwrap(),
            LegacyWaiting::Waiting {
                preference: MatchPreference::Ranked,
                humans: 3,
                needed: 10,
                elapsed_secs: 301,
                bot_fill_after_secs: None,
                capacity_wait: false,
            }
        );
        assert_eq!(
            serde_json::from_str::<MatchServiceView>(&plain).unwrap(),
            waiting(None, None, false)
        );
    }

    /// LR-09: the place in the line for a free arena is additive. A plain
    /// capacity wait keeps its bytes, and an older client reads it as one.
    #[test]
    fn the_capacity_line_is_additive_on_the_wire() {
        let waiting = |capacity_line| MatchServiceView::Waiting {
            preference: MatchPreference::BotPractice,
            humans: 1,
            needed: 1,
            elapsed_secs: 40,
            bot_fill_after_secs: None,
            capacity_wait: true,
            searching: None,
            casual_offer_after_secs: None,
            also_casual: false,
            capacity_line,
        };
        let plain = serde_json::to_string(&waiting(None)).unwrap();
        assert!(!plain.contains("capacity_line"), "{plain}");
        let line = waiting(Some(CapacityLine {
            position: 3,
            eta_secs: Some(130),
        }));
        let bytes = serde_json::to_string(&line).unwrap();
        assert!(
            bytes.contains(r#""capacity_line":{"position":3,"eta_secs":130}"#),
            "{bytes}"
        );
        assert_eq!(
            serde_json::from_str::<MatchServiceView>(&bytes).unwrap(),
            line
        );
        assert_eq!(
            serde_json::from_str::<LegacyWaiting>(&bytes).unwrap(),
            LegacyWaiting::Waiting {
                preference: MatchPreference::BotPractice,
                humans: 1,
                needed: 1,
                elapsed_secs: 40,
                bot_fill_after_secs: None,
                capacity_wait: true,
            }
        );
        // No estimate yet: the position alone.
        let unknown = serde_json::to_string(&CapacityLine {
            position: 1,
            eta_secs: None,
        })
        .unwrap();
        assert_eq!(unknown, r#"{"position":1}"#);
        let minutes = |secs| {
            CapacityLine {
                position: 1,
                eta_secs: Some(secs),
            }
            .eta_minutes()
        };
        assert_eq!(minutes(0), Some(1));
        assert_eq!(minutes(61), Some(2));
        assert_eq!(CapacityLine::default().eta_minutes(), None);
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
