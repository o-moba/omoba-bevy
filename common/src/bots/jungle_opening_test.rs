//! Sample the launch junglers on the same map camps using the production bot
//! controller, movement, combat, and progression.
//!
//! cargo test -p common --lib launch_jungle_opening -- --nocapture

use super::*;
use crate::neutrals::neutral_template;
use crate::offline::PracticeSession;
use crate::tick::{self, TickOptions};
use shared::hero_balance::{STARTING_LEVEL, xp_threshold_for_level};
use shared::wire::NeutralCampType;

const STEP: Duration = Duration::from_millis(50);
const OPENING_TICKS: u32 = 90 * 20;

struct CampClear {
    camp: NeutralCampType,
    at: Duration,
    health_fraction: f32,
    resource: f32,
    level: u32,
    total_xp: u32,
    camp_gold: u32,
}

fn opening(class: HeroClass, team: Team) -> Vec<CampClear> {
    let start = Instant::now();
    let mut practice = PracticeSession::new(start);
    practice.world.game_state = GameState::Running;
    let addr = practice
        .host()
        .spawn_bot(team, Some(class), BotKind::Duelist, start)
        .unwrap();
    // Assign the same jungle duty to both classes, independent of bot fill.
    practice.bots.controllers.get_mut(&addr).unwrap().jungle = true;
    practice.world.players.get_mut(&addr).unwrap().draft.role = Role::Jungle;
    let spawn = &practice.world.players[&addr].hero;
    assert_eq!(spawn.progress.level, STARTING_LEVEL);
    let mut route: Vec<_> = practice
        .world
        .neutrals
        .values()
        .filter(|neutral| !neutral.state.camp_type.is_boss())
        .map(|neutral| {
            (
                (spawn.x - neutral.anchor.x).hypot(spawn.z - neutral.anchor.z),
                neutral.state.id,
            )
        })
        .collect();
    route.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let camps: Vec<_> = route.into_iter().take(3).map(|(_, id)| id).collect();
    assert_eq!(camps.len(), 3);
    practice.world.neutrals.retain(|id, _| camps.contains(id));

    // Keep the route at three single clears. Preserve navigation, starter
    // purchases, camp attacks and healing, and ordinary skill/resource rules.
    // Lane waves, bosses, opponents and passive income do not enter this probe.
    let mut clears = Vec::new();
    let dt = STEP.as_secs_f32();
    for elapsed_ticks in 1..=OPENING_TICKS {
        let elapsed = STEP * elapsed_ticks;
        let now = start + elapsed;
        tick::prepare(&mut practice.world, &mut practice.combat_log, now, dt);
        tick::skills(&mut practice.world, &mut practice.combat_log, now, dt);
        practice.host().simulate_bots(now, dt);
        tick::finish(
            &mut practice.world,
            &mut practice.combat_log,
            now,
            dt,
            0.0,
            TickOptions {
                minions_running: false,
                ..Default::default()
            },
        );

        let player = &practice.world.players[&addr];
        assert!(
            player.hero.hp > 0.0,
            "{class:?}/{team:?} died at {:.2}s",
            elapsed.as_secs_f32()
        );
        for id in &camps {
            if !practice
                .world
                .neutrals
                .get(id)
                .is_some_and(|neutral| neutral.dead_until.is_some())
            {
                continue;
            }
            let neutral = practice.world.neutrals.remove(id).unwrap();
            let progress = player.hero.progress;
            let total_xp = (STARTING_LEVEL..progress.level)
                .map(xp_threshold_for_level)
                .sum::<u32>()
                + progress.xp;
            clears.push(CampClear {
                camp: neutral.state.camp_type,
                at: elapsed,
                health_fraction: hp_fraction(player),
                resource: player.hero.mana,
                level: progress.level,
                total_xp,
                camp_gold: player.economy.earned_gold,
            });
        }
        if clears.len() == camps.len() {
            break;
        }
    }
    assert_eq!(
        clears.len(),
        camps.len(),
        "{class:?}/{team:?} must finish all three camps in 90s"
    );
    clears
}

#[test]
fn launch_jungle_opening() {
    for team in [Team::Green, Team::Blue] {
        let warden = opening(HeroClass::Warden, team);
        let stormfist = opening(HeroClass::Stormfist, team);
        for (class, clears) in [
            (HeroClass::Warden, &warden),
            (HeroClass::Stormfist, &stormfist),
        ] {
            let mut expected_xp = 0;
            let mut expected_gold = 0;
            for clear in clears {
                let reward = neutral_template(clear.camp);
                expected_xp += reward.kill_xp;
                expected_gold += if class == HeroClass::Warden {
                    (reward.kill_gold as f32 * 1.4).round() as u32
                } else {
                    reward.kill_gold
                };
                assert_eq!(clear.total_xp, expected_xp);
                assert_eq!(clear.camp_gold, expected_gold);
                println!(
                    "{class:?} {team:?} {camp:?} {seconds:.2}s, HP {health:.0}%, resource {resource:.0}, level {level}, XP {xp}, camp gold {gold}",
                    camp = clear.camp,
                    seconds = clear.at.as_secs_f32(),
                    health = clear.health_fraction * 100.0,
                    resource = clear.resource,
                    level = clear.level,
                    xp = clear.total_xp,
                    gold = clear.camp_gold,
                );
            }
        }
        let warden_end = warden.last().unwrap();
        let stormfist_end = stormfist.last().unwrap();
        assert_eq!(warden_end.level, stormfist_end.level);
        assert_eq!(warden_end.total_xp, stormfist_end.total_xp);
        assert!(warden_end.camp_gold > stormfist_end.camp_gold);
    }
}
