//! Optional visual-only seasonal data. Called only by explicit QA capture paths;
//! it never authenticates, qualifies or writes a real account.
use shared::{
    career::{MatchResult, ProfileSummary, RatingChange},
    ranked::{SEASON_DURATION_MS, SeasonBadge, SeasonProfile, Tier},
};

#[derive(Clone, Copy)]
enum Scene {
    Placement,
    Calibrated,
    Promoted,
    Demoted,
}
fn scene() -> Option<Scene> {
    match std::env::var("OMOBA_RANKED_SEASON_QA").ok()?.as_str() {
        "placement" => Some(Scene::Placement),
        "calibrated" => Some(Scene::Calibrated),
        "promoted" => Some(Scene::Promoted),
        "demoted" => Some(Scene::Demoted),
        _ => None,
    }
}
impl Scene {
    fn rating(self) -> i32 {
        match self {
            Self::Placement => 1650,
            Self::Demoted => 1478,
            _ => 1510,
        }
    }
}
pub(super) fn enabled() -> bool {
    scene().is_some()
}
pub(super) fn apply_profile(profile: &mut ProfileSummary) {
    let Some(scene) = scene() else {
        return;
    };
    apply_profile_scene(profile, scene);
}
fn apply_profile_scene(profile: &mut ProfileSummary, scene: Scene) {
    let started = 1793923200000;
    let count = if matches!(scene, Scene::Placement) {
        0
    } else {
        21
    };
    profile.rating = scene.rating();
    profile.rated_matches = 100;
    profile.completed_public_matches = 10;
    profile.matches_played = 120;
    profile.wins = 60;
    profile.losses = 40;
    profile.abandons = u32::from(matches!(scene, Scene::Demoted));
    profile.ranked = Some(SeasonProfile {
        season_id: 2,
        started_at_ms: started,
        ends_at_ms: started + SEASON_DURATION_MS,
        rated_matches: count,
        wins: if count == 0 { 0 } else { 12 },
        losses: if count == 0 { 0 } else { 9 },
        completed_matches: count - u32::from(matches!(scene, Scene::Demoted)),
        latest_badge: Some(SeasonBadge {
            season_id: 1,
            ended_at_ms: started,
            rating: 2300,
            rated_matches: 40,
            tier: Some(Tier::Platinum),
        }),
    });
}
pub(super) fn apply_result(result: &mut MatchResult) {
    let Some(scene) = scene() else {
        return;
    };
    if !matches!(scene, Scene::Promoted | Scene::Demoted)
        || result.outcome != shared::career::MatchOutcome::Completed
    {
        return;
    }
    result.ranked_season_id = Some(2);
    result.rated = true;
    result.unrated_reason = None;
    for participant in &mut result.participants {
        if participant.profile_id.as_deref() == Some("qa-profile") {
            let after = scene.rating();
            let before = if matches!(scene, Scene::Demoted) {
                1510
            } else {
                1490
            };
            participant.rating = Some(RatingChange {
                before,
                after,
                delta: after - before,
            });
            if matches!(scene, Scene::Demoted) {
                // A winning team still gives its proved leaver a personal loss.
                // Use the ordinary post-placement -32 penalty and zero XP.
                participant.participation = Some(shared::career::Participation {
                    connected_ms: 660_000,
                    disconnected_ms: 60_000,
                    afk_ms: 0,
                    abandoned: true,
                });
                participant.progression_xp_gained = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_capture_retains_lifetime_access_and_the_old_final_badge() {
        let mut profile = ProfileSummary::new("qa-profile".into(), "QA".into());
        apply_profile_scene(&mut profile, Scene::Placement);
        assert_eq!(profile.rated_matches, 100);
        assert!(profile.ranked_unlocked());
        assert_eq!(profile.placement_matches(), 0);
        let badge = profile
            .ranked
            .as_ref()
            .unwrap()
            .latest_badge
            .as_ref()
            .unwrap();
        assert_eq!(badge.rating, 2300);
        assert_eq!(badge.tier, Some(Tier::Platinum));
        assert_eq!(profile.rating, 1650);
        for scene in [Scene::Calibrated, Scene::Promoted, Scene::Demoted] {
            apply_profile_scene(&mut profile, scene);
            let season = profile.ranked.as_ref().unwrap();
            assert_eq!(season.wins + season.losses, season.rated_matches);
            assert_eq!(
                season.completed_matches,
                season.rated_matches - u32::from(matches!(scene, Scene::Demoted))
            );
            assert_eq!(profile.abandons, u32::from(matches!(scene, Scene::Demoted)));
            assert!(!profile.newcomer());
        }
    }
}
