//! Opt-in developer capture setup. Initial placement and the scenario's starting
//! progression are fixtures. Combat, vision and damage use the ordinary rules.

use std::collections::HashMap;
use std::net::SocketAddr;

use shared::{HeroClass, SkillSlot, map::Team};

use crate::balance::PLAYER_GROUND_Y;
use crate::entities::ConnectedPlayer;
use crate::match_rules::MatchMode;

fn enabled_for(debug: bool, mode: MatchMode, mode_env: Option<&str>, flag: Option<&str>) -> bool {
    debug && matches!(mode, MatchMode::Dev) && mode_env == Some("dev") && flag == Some("1")
}

pub(crate) fn enabled(mode: MatchMode) -> bool {
    enabled_for(
        cfg!(debug_assertions),
        mode,
        std::env::var("OMOBA_MATCH_MODE").ok().as_deref(),
        std::env::var("OMOBA_TARGETING_QA").ok().as_deref(),
    ) || vision_enabled(mode)
}

pub(crate) fn vision_enabled(mode: MatchMode) -> bool {
    enabled_for(
        cfg!(debug_assertions),
        mode,
        std::env::var("OMOBA_MATCH_MODE").ok().as_deref(),
        std::env::var("OMOBA_VISION_QA").ok().as_deref(),
    )
}

pub(crate) const GREEN: [f32; 2] = [-8.0, -8.0];
pub(crate) const BLUE: [[f32; 2]; 2] = [[-4.0, -8.0], [-8.0, -4.0]];
const WARNING_VIEWER: [f32; 2] = [3.0, -3.0];
const WARNING_CASTER: [f32; 2] = [31.0, 25.0];

fn wildspark_warning_enabled(
    vision_qa: bool,
    scenario: Option<&str>,
    bind: Option<&str>,
    sandbox: bool,
) -> bool {
    vision_qa
        && scenario == Some("wildspark-warning")
        && !sandbox
        && bind
            .and_then(|address| address.parse::<SocketAddr>().ok())
            .is_some_and(|address| address.ip().is_loopback())
}

fn prepare_warning_caster(player: &mut ConnectedPlayer) {
    let r_level = shared::SLOT_UNLOCK_LEVELS[SkillSlot::R.index()];
    let xp_needed = (player.hero.progress.level..r_level)
        .map(common::progression::xp_threshold_for_level)
        .sum::<u32>()
        .saturating_sub(player.hero.progress.xp);
    common::progression::grant_player_xp(&mut player.hero, xp_needed);
    player.hero.hp = player.hero.max_hp;
    player.hero.mana = player.hero.max_mana;
}

pub(crate) fn place_initial_join(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    addr: SocketAddr,
) {
    let Some(player) = players.get(&addr) else {
        return;
    };
    if !player.joined {
        return;
    }
    let team = player.hero.identity.team;
    let index = players
        .values()
        .filter(|p| {
            p.joined && p.hero.identity.team == team && p.hero.identity.id < player.hero.identity.id
        })
        .count();
    let vision_qa = vision_enabled(MatchMode::Dev);
    let warning_fixture = wildspark_warning_enabled(
        vision_qa,
        std::env::var("OMOBA_VISION_QA_SCENARIO").ok().as_deref(),
        std::env::var("SERVER_ADDR").ok().as_deref(),
        std::env::var("OMOBA_COMBAT_SANDBOX").as_deref() == Ok("1"),
    );
    let warning_caster = warning_fixture
        && team == Team::Blue
        && player.hero.identity.hero_class == HeroClass::Wildspark;
    let position = if warning_fixture {
        let navigation = shared::navigation::world_navigation();
        if !navigation.segment_clear(WARNING_VIEWER, WARNING_CASTER) {
            return;
        }
        if warning_caster {
            Some(WARNING_CASTER)
        } else if team == Team::Green && index == 0 {
            Some(WARNING_VIEWER)
        } else {
            None
        }
    } else if vision_qa {
        let zone = shared::vision::brush_layout()[0];
        let offset = if team == Team::Green {
            zone.radius + 5.0
        } else {
            zone.radius + 1.0
        };
        Some([zone.center[0] + offset, zone.center[1]])
    } else {
        match team {
            Team::Green if index == 0 => Some(GREEN),
            Team::Blue => BLUE.get(index).copied(),
            _ => None,
        }
    };
    let Some([x, z]) = position else { return };
    // Refuse setup if a later map revision makes these points obstructed.
    if !shared::navigation::world_navigation().point_clear([x, z]) {
        return;
    }
    let player = players.get_mut(&addr).unwrap();
    player.hero.x = x;
    player.hero.y = PLAYER_GROUND_Y;
    player.hero.z = z;
    if warning_caster {
        prepare_warning_caster(player);
        println!(
            "VISION_QA wildspark-warning player={} level={} hp={} mana={}",
            player.hero.identity.id, player.hero.progress.level, player.hero.hp, player.hero.mana,
        );
    }
    println!(
        "TARGETING_QA initial_join player={} team={team:?} x={x} z={z}; ambient AI disabled; player damage authoritative",
        player.hero.identity.id
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_rules::MatchMode;
    use shared::vision::{BASE_SIGHT_RADIUS, HERO_SIGHT_RADIUS, TOWER_SIGHT_RADIUS};

    #[test]
    fn requires_all_explicit_development_gates_and_walkable_anchors() {
        assert!(enabled_for(true, MatchMode::Dev, Some("dev"), Some("1")));
        assert!(!enabled_for(false, MatchMode::Dev, Some("dev"), Some("1")));
        assert!(!enabled_for(
            true,
            MatchMode::Release,
            Some("dev"),
            Some("1")
        ));
        assert!(!enabled_for(true, MatchMode::Dev, None, Some("1")));
        assert!(!enabled_for(true, MatchMode::Dev, Some("dev"), None));
        assert!(!enabled_for(
            true,
            MatchMode::Dev,
            Some("dev"),
            Some("true")
        ));
        for position in [GREEN, BLUE[0], BLUE[1]] {
            assert!(shared::navigation::world_navigation().point_clear(position));
        }
    }

    #[test]
    fn warning_fixture_requires_vision_qa_on_an_explicit_loopback_without_sandbox() {
        let scenario = Some("wildspark-warning");
        let loopback = Some("127.0.0.1:42000");
        assert!(wildspark_warning_enabled(true, scenario, loopback, false));
        assert!(wildspark_warning_enabled(
            true,
            scenario,
            Some("[::1]:42000"),
            false
        ));
        assert!(!wildspark_warning_enabled(false, scenario, loopback, false));
        assert!(!wildspark_warning_enabled(true, None, loopback, false));
        assert!(!wildspark_warning_enabled(true, scenario, None, false));
        assert!(!wildspark_warning_enabled(
            true,
            scenario,
            Some("0.0.0.0:42000"),
            false
        ));
        assert!(!wildspark_warning_enabled(true, scenario, loopback, true));
    }

    #[test]
    fn warning_fixture_has_a_clear_lane_from_outside_all_green_sight() {
        let navigation = shared::navigation::world_navigation();
        let map = shared::map::ResolvedMap::default();
        let distance_from_caster =
            |point: [f32; 2]| (point[0] - WARNING_CASTER[0]).hypot(point[1] - WARNING_CASTER[1]);
        let distance = distance_from_caster(WARNING_VIEWER);
        assert!((38.0..=45.0).contains(&distance) && distance > HERO_SIGHT_RADIUS);
        assert!(navigation.point_clear(WARNING_VIEWER));
        assert!(navigation.point_clear(WARNING_CASTER));
        assert!(
            map.structures
                .iter()
                .filter(|s| s.team == Team::Green)
                .all(|s| {
                    let sight = if s.lane.is_some() {
                        TOWER_SIGHT_RADIUS
                    } else {
                        BASE_SIGHT_RADIUS
                    };
                    distance_from_caster(s.position) > sight
                })
        );
        let structures: Vec<_> = map
            .structures
            .iter()
            .map(|s| shared::navigation::Disc {
                center: s.position,
                radius: if s.lane.is_some() {
                    shared::TOWER_TARGET_RADIUS
                } else {
                    3.2
                },
            })
            .collect();
        assert!(navigation.segment_clear_with_discs(WARNING_VIEWER, WARNING_CASTER, &structures));
    }
}
