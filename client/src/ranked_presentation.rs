//! Seasonal Ranked copy. Lifetime experience never substitutes for placements.
// i18n-strict
use crate::i18n::{tr, trf};
use shared::{
    career::{NEWCOMER_MATCHES, ProfileSummary, RatingChange},
    ranked::{Tier, calibrated_tier},
};

pub(crate) fn tier_name(tier: Tier) -> &'static str {
    tr(match tier {
        Tier::Bronze => "ranked.tier.bronze",
        Tier::Silver => "ranked.tier.silver",
        Tier::Gold => "ranked.tier.gold",
        Tier::Platinum => "ranked.tier.platinum",
        Tier::Diamond => "ranked.tier.diamond",
        Tier::Master => "ranked.tier.master",
        Tier::Unknown => "ranked.tier.unknown",
    })
}

pub(crate) fn progress_text(profile: &ProfileSummary) -> String {
    match calibrated_tier(profile.rating, profile.placement_matches()) {
        Some(tier) => trf("ranked.current_tier", &[("tier", &tier_name(tier))]),
        None => trf(
            "ranked.placements",
            &[
                ("played", &profile.placement_matches()),
                ("total", &NEWCOMER_MATCHES),
            ],
        ),
    }
}

pub(crate) fn profile_lines(profile: &ProfileSummary) -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    if let Some(season) = &profile.ranked {
        lines.push((
            "CareerRankedSeason",
            trf(
                "ranked.season.until",
                &[
                    ("season", &season.season_id),
                    ("until", &super::timestamp(season.ends_at_ms)),
                ],
            ),
        ));
        if let Some(badge) = &season.latest_badge {
            let tier = badge
                .tier
                .map(tier_name)
                .unwrap_or_else(|| tr("ranked.badge.participant"));
            lines.push((
                "CareerRankedBadge",
                trf(
                    "ranked.badge.earned",
                    &[
                        ("season", &badge.season_id),
                        ("tier", &tier),
                        ("rating", &badge.rating),
                    ],
                ),
            ));
        }
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RankChange {
    Calibrated(Tier),
    Raised(Tier),
    Lowered(Tier),
}

/// Caller supplies the scoped, rated, local participant's immutable award.
/// Refuse a stale award, a different season or provisional progress. The first
/// calibrated tier is announced even when the final placement was a loss.
fn rank_change(
    profile: &ProfileSummary,
    season_id: Option<u64>,
    change: &RatingChange,
) -> Option<RankChange> {
    let season = profile.ranked.as_ref()?;
    if Some(season.season_id) != season_id || profile.rating != change.after {
        return None;
    }
    let tier = calibrated_tier(change.after, season.rated_matches)?;
    if season.rated_matches == NEWCOMER_MATCHES {
        Some(RankChange::Calibrated(tier))
    } else if change.after > change.before && Tier::for_rating(change.before) != tier {
        Some(RankChange::Raised(tier))
    } else if change.after < change.before && Tier::for_rating(change.before) != tier {
        Some(RankChange::Lowered(tier))
    } else {
        None
    }
}

pub(crate) struct RankNotice {
    pub text: String,
    pub demotion: bool,
}

pub(crate) fn rank_change_notice(
    profile: &ProfileSummary,
    season_id: Option<u64>,
    change: &RatingChange,
) -> Option<RankNotice> {
    rank_change(profile, season_id, change).map(|event| {
        let (key, tier) = match event {
            RankChange::Calibrated(tier) => ("ranked.placement_complete", tier),
            RankChange::Raised(tier) => ("ranked.promoted", tier),
            RankChange::Lowered(tier) => ("ranked.demoted", tier),
        };
        RankNotice {
            text: trf(key, &[("tier", &tier_name(tier))]),
            demotion: matches!(event, RankChange::Lowered(_)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::ranked::{SEASON_DURATION_MS, SeasonProfile};
    fn profile(count: u32, rating: i32) -> ProfileSummary {
        let mut p = ProfileSummary::new("a".repeat(64), "Ranked".into());
        p.rating = rating;
        p.rated_matches = 100;
        p.ranked = Some(SeasonProfile {
            season_id: 2,
            started_at_ms: 1000,
            ends_at_ms: 1000 + SEASON_DURATION_MS,
            rated_matches: count,
            wins: 0,
            losses: 0,
            completed_matches: count,
            latest_badge: None,
        });
        p
    }
    #[test]
    fn reset_and_stale_season_awards_never_announce_a_promotion() {
        let change = RatingChange {
            before: 1490,
            after: 1510,
            delta: 20,
        };
        for count in [0, 19] {
            assert_eq!(rank_change(&profile(count, 1510), Some(2), &change), None);
        }
        assert_eq!(rank_change(&profile(21, 1510), Some(1), &change), None);
        assert_eq!(rank_change(&profile(21, 1510), None, &change), None);
        assert_eq!(rank_change(&profile(21, 1530), Some(2), &change), None);
        let mut legacy = profile(21, 1510);
        legacy.ranked = None;
        assert_eq!(rank_change(&legacy, Some(2), &change), None);
    }
    #[test]
    fn final_placement_loss_announces_calibration_then_both_directions_of_tier_crossings() {
        let loss = RatingChange {
            before: 1510,
            after: 1490,
            delta: -20,
        };
        assert_eq!(
            rank_change(&profile(20, 1490), Some(2), &loss),
            Some(RankChange::Calibrated(Tier::Silver))
        );
        assert_eq!(
            rank_change(&profile(21, 1490), Some(2), &loss),
            Some(RankChange::Lowered(Tier::Silver))
        );
        let gain = RatingChange {
            before: 1490,
            after: 1510,
            delta: 20,
        };
        assert_eq!(
            rank_change(&profile(21, 1510), Some(2), &gain),
            Some(RankChange::Raised(Tier::Gold))
        );
        let same_tier = RatingChange {
            before: 1510,
            after: 1530,
            delta: 20,
        };
        assert_eq!(rank_change(&profile(22, 1530), Some(2), &same_tier), None);
    }
}
