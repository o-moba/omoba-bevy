//! Ordinary jungle camp anchors shared by simulation and map presentation.
//! Each pair has the same creature and rewards under a half-turn of the map.
use crate::{HeroClass, navigation::Point};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JungleCampKind {
    Skirmisher,
    Bruiser,
    Spitter,
}

/// Stable order is also the six minimap camp slots. Coordinates use full map
/// extent. Bruisers sit slightly toward the base-side clearing to leave more
/// than ten metres of clearance in both halves of the shipped Verdant forest.
pub fn camp_layout(map_size: f32) -> [(Point, JungleCampKind); 6] {
    use JungleCampKind::*;
    [
        ([-0.34 * map_size, 0.22 * map_size], Skirmisher),
        ([0.34 * map_size, -0.22 * map_size], Skirmisher),
        ([-0.30 * map_size, -0.227 * map_size], Bruiser),
        ([0.30 * map_size, 0.227 * map_size], Bruiser),
        ([-0.22 * map_size, -0.34 * map_size], Spitter),
        ([0.22 * map_size, 0.34 * map_size], Spitter),
    ]
}

/// Warden passive "Forest Tracker": damage multiplier against jungle camps.
pub const WARDEN_CAMP_DAMAGE_MULTIPLIER: f32 = 1.35;
/// Bosses are team objectives, so the jungler only gets a small edge there.
pub const WARDEN_BOSS_DAMAGE_MULTIPLIER: f32 = 1.15;
/// Extra gold and XP a Warden earns per ordinary camp kill.
pub const WARDEN_CAMP_GOLD_MULTIPLIER: f32 = 1.4;
pub const WARDEN_CAMP_XP_MULTIPLIER: f32 = 1.25;

/// Outgoing damage multiplier for a hero hitting a neutral monster.
pub fn neutral_damage_multiplier(class: HeroClass, boss: bool) -> f32 {
    match (class, boss) {
        (HeroClass::Warden, false) => WARDEN_CAMP_DAMAGE_MULTIPLIER,
        (HeroClass::Warden, true) => WARDEN_BOSS_DAMAGE_MULTIPLIER,
        _ => 1.0,
    }
}

/// Gold and XP a hero of `class` earns for the final blow on a neutral.
/// Boss rewards stay flat: their value is the team buff.
pub fn neutral_kill_rewards(class: HeroClass, boss: bool, gold: u32, xp: u32) -> (u32, u32) {
    if class != HeroClass::Warden || boss {
        return (gold, xp);
    }
    (
        (gold as f32 * WARDEN_CAMP_GOLD_MULTIPLIER).round() as u32,
        (xp as f32 * WARDEN_CAMP_XP_MULTIPLIER).round() as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::world_navigation;

    #[test]
    fn only_the_warden_farms_camps_faster_and_bosses_stay_team_objectives() {
        for class in HeroClass::ALL {
            let warden = class == HeroClass::Warden;
            assert_eq!(neutral_damage_multiplier(class, false) > 1.0, warden);
            assert!(
                neutral_damage_multiplier(class, true) <= neutral_damage_multiplier(class, false)
            );
            assert_eq!(neutral_kill_rewards(class, true, 150, 200), (150, 200));
            let (gold, xp) = neutral_kill_rewards(class, false, 52, 85);
            assert_eq!((gold > 52, xp > 85), (warden, warden));
        }
        assert_eq!(
            neutral_kill_rewards(HeroClass::Warden, false, 52, 85),
            (73, 106)
        );
    }

    #[test]
    fn mirrored_camps_have_equal_types_and_reachable_clear_fighting_space() {
        let nav = world_navigation();
        let bounds = nav.bounds();
        let camps = camp_layout(bounds.max[0] - bounds.min[0]);
        for pair in camps.chunks_exact(2) {
            assert_eq!(pair[0].1, pair[1].1);
            assert_eq!(pair[0].0, pair[1].0.map(|v| -v));
        }
        for (point, _) in camps {
            assert!(nav.point_clear(point));
            // A 3m fighting ring is unobstructed, with physical hero clearance.
            for step in 0..16 {
                let angle = step as f32 * std::f32::consts::TAU / 16.0;
                let rim = [point[0] + 3.0 * angle.cos(), point[1] + 3.0 * angle.sin()];
                assert!(nav.segment_clear(point, rim), "blocked camp ring {point:?}");
            }
            for base in [[-74.599_77, -74.599_77], [74.599_77, 74.599_77]] {
                let route = nav.plan_route(base, point, &[]).expect("camp route");
                let end = route.last().copied().unwrap_or(base);
                assert!((end[0] - point[0]).hypot(end[1] - point[1]) < 0.1);
            }
        }
    }
}
