//! Owner-approved Ranked tiers, season arithmetic and declared input telemetry.
//! Persistence and UI use these same rules; historical unknown input stays unknown.
use serde::{Deserialize, Serialize};

use crate::career::{INITIAL_RATING, NEWCOMER_MATCHES};

pub const MAX_RATING: i32 = 5000;
pub const SEASON_DURATION_MS: u64 = 90 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
    Platinum,
    Diamond,
    Master,
    #[default]
    #[serde(other)]
    Unknown,
}

impl Tier {
    pub fn for_rating(rating: i32) -> Self {
        match rating {
            ..1000 => Self::Bronze,
            1000..1500 => Self::Silver,
            1500..2000 => Self::Gold,
            2000..2500 => Self::Platinum,
            2500..3000 => Self::Diamond,
            3000.. => Self::Master,
        }
    }
}

/// Placement games do not expose a calibrated tier, even after a soft reset.
pub fn calibrated_tier(rating: i32, season_rated_matches: u32) -> Option<Tier> {
    (season_rated_matches >= NEWCOMER_MATCHES).then(|| Tier::for_rating(rating))
}

/// Half the signed distance from1000, truncating fractional points toward1000.
/// Clamp invalid legacy input before arithmetic so even i32 extremes are safe.
pub fn reset_rating(rating: i32) -> i32 {
    INITIAL_RATING + (rating.clamp(0, MAX_RATING) - INITIAL_RATING) / 2
}

/// Declared control scheme, not hardware attestation or an operating system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Touch,
    Mouse,
    Mixed,
    #[default]
    #[serde(other)]
    Unknown,
}

impl InputKind {
    pub fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }

    /// Historical/missing reports cannot erase observed input or a switch.
    pub fn observe(self, incoming: Self) -> Self {
        match (self, incoming) {
            (known, Self::Unknown) => known,
            (Self::Unknown, observed) => observed,
            (before, after) if before == after => before,
            _ => Self::Mixed,
        }
    }
}

/// Current-season progress is distinct from lifetime career/eligibility totals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonProfile {
    pub season_id: u64,
    pub started_at_ms: u64,
    pub ends_at_ms: u64,
    pub rated_matches: u32,
    pub wins: u32,
    pub losses: u32,
    pub completed_matches: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_badge: Option<SeasonBadge>,
}

impl SeasonProfile {
    pub fn active_at(&self, now_ms: u64) -> bool {
        self.started_at_ms <= now_ms && now_ms < self.ends_at_ms
    }

    pub fn calibrated_tier(&self, rating: i32) -> Option<Tier> {
        calibrated_tier(rating, self.rated_matches)
    }
}

/// Cosmetic participation award; provisional seasons have no calibrated tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonBadge {
    pub season_id: u64,
    pub ended_at_ms: u64,
    pub rating: i32,
    pub rated_matches: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
}

/// Counts, rather than rounded percentages, keep small/unknown samples visible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputStatistics {
    pub receipt_limit: u32,
    pub scanned_receipts: u64,
    pub completed_ranked_matches: u64,
    pub rows: Vec<InputStatisticsRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputStatisticsRow {
    pub kind: InputKind,
    pub matches: u64,
    pub wins: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_tier_boundaries_and_twentieth_placement() {
        for (rating, tier) in [
            (0, Tier::Bronze),
            (999, Tier::Bronze),
            (1000, Tier::Silver),
            (1499, Tier::Silver),
            (1500, Tier::Gold),
            (1999, Tier::Gold),
            (2000, Tier::Platinum),
            (2499, Tier::Platinum),
            (2500, Tier::Diamond),
            (2999, Tier::Diamond),
            (3000, Tier::Master),
            (5000, Tier::Master),
        ] {
            assert_eq!(Tier::for_rating(rating), tier);
            assert_eq!(calibrated_tier(rating, 19), None);
            assert_eq!(calibrated_tier(rating, 20), Some(tier));
            assert_eq!(calibrated_tier(rating, u32::MAX), Some(tier));
        }
        assert_eq!(SEASON_DURATION_MS, 7_776_000_000);
    }

    #[test]
    fn reset_rounding_bounds_and_monotonic_distance() {
        for (before, after) in [
            (i32::MIN, 500),
            (0, 500),
            (1, 501),
            (998, 999),
            (999, 1000),
            (1000, 1000),
            (1001, 1000),
            (1002, 1001),
            (4999, 2999),
            (5000, 3000),
            (i32::MAX, 3000),
        ] {
            assert_eq!(reset_rating(before), after);
        }
        for rating in 0..=MAX_RATING {
            let reset = reset_rating(rating);
            assert!((500..=3000).contains(&reset));
            assert_eq!((reset - 1000).abs(), (rating - 1000).abs() / 2);
            if rating < MAX_RATING {
                assert!(reset <= reset_rating(rating + 1));
            }
        }
    }

    #[test]
    fn declared_input_switch_is_monotonic_and_unknown_does_not_erase_it() {
        use InputKind::*;
        assert_eq!(Unknown.observe(Unknown), Unknown);
        assert_eq!(Unknown.observe(Touch).observe(Unknown), Touch);
        assert_eq!(Unknown.observe(Mouse).observe(Mouse), Mouse);
        assert_eq!(Touch.observe(Mouse), Mixed);
        assert_eq!(Mouse.observe(Touch), Mixed);
        for before in [Unknown, Touch, Mouse, Mixed] {
            for after in [Unknown, Touch, Mouse, Mixed] {
                assert_eq!(Mixed.observe(after), Mixed);
                assert_eq!(before.observe(after), after.observe(before));
            }
        }
        assert!(Unknown.is_unknown());
    }

    #[test]
    fn unknown_future_tier_and_input_values_remain_decodable() {
        assert_eq!(
            serde_json::from_str::<Tier>("\"future_tier\"").unwrap(),
            Tier::Unknown
        );
        assert_eq!(
            serde_json::from_str::<InputKind>("\"controller\"").unwrap(),
            InputKind::Unknown
        );
        assert_eq!(
            serde_json::to_string(&Tier::Platinum).unwrap(),
            "\"platinum\""
        );
        assert_eq!(
            serde_json::to_string(&InputKind::Mixed).unwrap(),
            "\"mixed\""
        );
    }
}

#[cfg(test)]
mod season_profile_tests {
    use super::*;
    #[test]
    fn season_placements_do_not_reset_lifetime_totals_or_public_access() {
        let mut profile = crate::career::ProfileSummary::new("a".repeat(64), "Hero".into());
        profile.rated_matches = 100;
        profile.matches_played = 150;
        profile.completed_public_matches = 150;
        assert!(!profile.newcomer());
        assert!(!serde_json::to_string(&profile).unwrap().contains("ranked"));
        profile.ranked = Some(SeasonProfile {
            season_id: 2,
            started_at_ms: 1000,
            ends_at_ms: 1000 + SEASON_DURATION_MS,
            rated_matches: 0,
            wins: 0,
            losses: 0,
            completed_matches: 0,
            latest_badge: None,
        });
        assert!(profile.newcomer() && profile.ranked_unlocked());
        assert_eq!((profile.rated_matches, profile.matches_played), (100, 150));
        let season = profile.ranked.as_mut().unwrap();
        assert!(!season.active_at(999));
        assert!(season.active_at(1000));
        assert!(season.active_at(season.ends_at_ms - 1));
        assert!(!season.active_at(season.ends_at_ms));
        assert_eq!(season.calibrated_tier(1500), None);
        season.rated_matches = 20;
        assert_eq!(season.calibrated_tier(1500), Some(Tier::Gold));
        assert!(!profile.newcomer());
    }
}
