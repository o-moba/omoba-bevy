//! Stateful route following for ordinary fill bots. Tactical decisions remain
//! in bot_ai; geometry and path search are shared with the native client/server.
use shared::navigation::{Disc, Point, world_navigation};
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
};

#[derive(Default)]
pub struct BotNavigator {
    goal: Option<Point>,
    position: Option<Point>,
    revision: u64,
    remaining: VecDeque<Point>,
}

impl BotNavigator {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn next(&mut self, position: Point, goal: Point, structures: &[Disc]) -> Option<Point> {
        let mut keys: Vec<_> = structures
            .iter()
            .map(|d| {
                (
                    d.center[0].to_bits(),
                    d.center[1].to_bits(),
                    d.radius.to_bits(),
                )
            })
            .collect();
        keys.sort_unstable();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        keys.hash(&mut hash);
        let revision = hash.finish();
        let map = world_navigation();
        let distance = |a: Point, b: Point| (a[0] - b[0]).hypot(a[1] - b[1]);
        let changed = self.goal.is_none_or(|old| distance(old, goal) > 0.75)
            || revision != self.revision
            || self
                .position
                .is_some_and(|old| distance(old, position) > 3.0)
            || self
                .remaining
                .front()
                .is_some_and(|next| !map.segment_clear(position, *next));
        if changed {
            self.remaining = map
                .plan_route(position, goal, structures)
                .unwrap_or_default()
                .into();
            self.goal = Some(goal);
            self.revision = revision;
        }
        self.position = Some(position);
        while self
            .remaining
            .front()
            .is_some_and(|next| distance(position, *next) <= 0.01)
        {
            self.remaining.pop_front();
        }
        self.remaining.front().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_ai::step_toward;

    #[test]
    fn spawn_and_respawn_leave_the_solid_friendly_base_on_every_lane() {
        use crate::{
            HeroClass, Team,
            bot_ai::{BotBrain, Lane, WorldView, lane_waypoints},
        };
        let map = world_navigation();
        for team in [Team::Green, Team::Blue] {
            for lane in Lane::ALL {
                let base = lane_waypoints(lane, team)[0];
                let length = base.0.hypot(base.1);
                let spawn = [
                    base.0 - base.0 / length * 7.0,
                    base.1 - base.1 / length * 7.0,
                ];
                let structures = [Disc {
                    center: [base.0, base.1],
                    radius: 3.2,
                }];
                let mut brain = BotBrain::new(lane, team, HeroClass::Warrior);
                let mut follower = BotNavigator::default();
                for _life in 0..2 {
                    let mut current = spawn;
                    brain.resync(current[0], current[1]);
                    follower.clear();
                    for _ in 0..200 {
                        let decision = brain.decide(current[0], current[1], &WorldView::default());
                        if let Some(goal) = decision.move_target
                            && let Some(next) =
                                follower.next(current, [goal.0, goal.1], &structures)
                        {
                            let (x, z, _) =
                                step_toward((current[0], current[1]), (next[0], next[1]), 0.1);
                            assert!(map.segment_clear_with_discs(current, [x, z], &structures));
                            current = [x, z];
                        }
                    }
                    assert!(
                        (current[0] - spawn[0]).hypot(current[1] - spawn[1]) > 20.0,
                        "{team:?}/{lane:?} stalled at its own solid base"
                    );
                }
            }
        }
    }

    #[test]
    fn bot_detours_through_real_forest_at_normal_speed_and_recovers_after_reset() {
        let map = world_navigation();
        let (start, goal) = map
            .obstacles()
            .iter()
            .filter(|o| o.kind == "tree_trunk")
            .find_map(|o| {
                let center = o
                    .vertices
                    .iter()
                    .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]])
                    .map(|x| x / o.vertices.len() as f32);
                let a = [center[0] - 4.0, center[1]];
                let b = [center[0] + 4.0, center[1]];
                (map.point_clear(a)
                    && map.point_clear(b)
                    && !map.segment_clear(a, b)
                    && map.plan_route(a, b, &[]).is_some())
                .then_some((a, b))
            })
            .expect("current forest has a traversable tree crossing");
        let mut follower = BotNavigator::default();
        let mut current = start;
        let mut steps = 0;
        while let Some(target) = follower.next(current, goal, &[]) {
            let (x, z, _) = step_toward((current[0], current[1]), (target[0], target[1]), 0.1);
            let next = [x, z];
            assert!((x - current[0]).hypot(z - current[1]) <= 0.501);
            assert!(
                map.segment_clear(current, next),
                "bot tried to cross a trunk"
            );
            current = next;
            steps += 1;
            assert!(steps < 150, "bot failed to finish its cached detour");
        }
        assert!((current[0] - goal[0]).hypot(current[1] - goal[1]) < 0.02);
        assert!(steps > 16, "8m crossing should require a detour at 5m/s");
        follower.clear();
        assert!(follower.next(start, goal, &[]).is_some());
        assert!(follower.next(goal, start, &[]).is_some());
    }
}
