//! Lane-push brain for fill bots (TASK-23).
//!
//! Gives each bot a believable MOBA goal: walk its lane toward the enemy
//! base, fight enemy players/minions met on the way with the Q ability, and
//! siege enemy towers when it reaches them. Deliberately simple — nearest
//! target, no retreat, no skill combos. The server stays fully authoritative
//! (movement speed budget, cast ranges, cooldowns, damage); this module only
//! decides where a bot *wants* to go and what it *wants* to hit.
//!
//! Map geometry mirrors `server/src/world.rs::build_map_layout` and
//! `lane_control_points` (constants from `server/src/balance.rs`). Keep in
//! sync with the server, same convention as `protocol.rs`.

use shared::{HeroClass as SharedClass, SkillSlot, ability_for_class_slot};

use crate::protocol::{
    HeroClass, MinionState, PlayerState, ServerPacket, StructureState, TargetId, TargetKind, Team,
};

// --- Map geometry mirror (server/src/balance.rs + world.rs) ---------------

const PLAYER_SPEED: f32 = 5.0;
const TARGET_BASE_RUN_TIME_SECONDS: f32 = 45.0;
const TARGET_BASE_DISTANCE: f32 = PLAYER_SPEED * TARGET_BASE_RUN_TIME_SECONDS;
const BASE_PAD_SIZE: f32 = 46.0;
const BASE_EDGE_MARGIN: f32 = 6.0;
const LANE_WIDTH: f32 = 12.0;
const LANE_EDGE_PADDING: f32 = 6.0;

/// A bot walks at the legal server speed; exported for the driver loop.
pub const BOT_MOVE_SPEED: f32 = PLAYER_SPEED;
/// Extra pull distance beyond the Q cast range: enemies inside
/// `cast range + AGGRO_RANGE` make the bot leave its lane path and approach
/// until they are in cast range.
pub const AGGRO_RANGE: f32 = 9.0;
/// A waypoint counts as reached inside this radius.
pub const WAYPOINT_REACHED: f32 = 2.0;
/// Cast slightly inside the real range so server-side range checks pass
/// while both sides are moving.
const CAST_RANGE_SAFETY: f32 = 0.9;

/// Lanes, ordered like the server's minion lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Mid,
    Top,
    Bot,
}

impl Lane {
    pub const ALL: [Self; 3] = [Self::Mid, Self::Top, Self::Bot];
}

struct MapPoints {
    home: (f32, f32),
    away: (f32, f32),
    left_x: f32,
    right_x: f32,
    top_z: f32,
    bottom_z: f32,
}

fn map_points() -> MapPoints {
    let inner_side = TARGET_BASE_DISTANCE / 2.0_f32.sqrt();
    let half_inner_side = inner_side * 0.5;
    let base_padding = BASE_PAD_SIZE * 0.5 + BASE_EDGE_MARGIN;
    let half_map_size = half_inner_side + base_padding;
    let lane_edge_offset = LANE_EDGE_PADDING + LANE_WIDTH * 0.5;
    MapPoints {
        home: (-half_inner_side, -half_inner_side),
        away: (half_inner_side, half_inner_side),
        left_x: -half_map_size + lane_edge_offset,
        right_x: half_map_size - lane_edge_offset,
        top_z: half_map_size - lane_edge_offset,
        bottom_z: -half_map_size + lane_edge_offset,
    }
}

/// Lane waypoints oriented for `team`: index 0 is the own base, the last
/// entry is the enemy base. Mirrors `server::lane_control_points`.
pub fn lane_waypoints(lane: Lane, team: Team) -> Vec<(f32, f32)> {
    let m = map_points();
    let mut points = match lane {
        Lane::Mid => vec![m.home, m.away],
        Lane::Top => vec![
            m.home,
            (m.left_x, m.home.1),
            (m.left_x, m.top_z),
            (m.right_x, m.top_z),
            (m.away.0, m.top_z),
            m.away,
        ],
        Lane::Bot => vec![
            m.home,
            (m.home.0, m.bottom_z),
            (m.left_x, m.bottom_z),
            (m.right_x, m.bottom_z),
            (m.right_x, m.away.1),
            m.away,
        ],
    };
    if team == Team::Blue {
        points.reverse();
    }
    points
}

/// Server-authoritative Q cast range for a class (shared ability kit).
pub fn q_cast_range(class: HeroClass) -> f32 {
    let shared_class = match class {
        HeroClass::Warrior => SharedClass::Warrior,
        HeroClass::Mage => SharedClass::Mage,
        HeroClass::Ranger => SharedClass::Ranger,
        HeroClass::Cleric => SharedClass::Cleric,
    };
    ability_for_class_slot(shared_class, SkillSlot::Q).cast_range
}

// --- World view ------------------------------------------------------------

/// One attackable enemy as the brain sees it.
#[derive(Debug, Clone, Copy)]
pub struct EnemyRef {
    pub kind: TargetKind,
    pub id: u64,
    pub x: f32,
    pub z: f32,
}

/// Enemies extracted from a snapshot, split into units (players + minions)
/// and structures (towers).
#[derive(Debug, Default)]
pub struct WorldView {
    pub units: Vec<EnemyRef>,
    pub structures: Vec<EnemyRef>,
}

impl WorldView {
    /// Builds the enemy view for a bot on `my_team` with player id `my_id`.
    pub fn from_snapshot(snapshot: &ServerPacket, my_id: u64, my_team: Team) -> Self {
        let is_enemy_team = |team: &Option<Team>| team.is_some_and(|t| t != my_team);
        let mut view = WorldView::default();
        for player in snapshot.players() {
            let PlayerState {
                id, x, z, team, hp, ..
            } = player;
            if *id != my_id && is_enemy_team(team) && *hp > 0.0 {
                view.units.push(EnemyRef {
                    kind: TargetKind::Player,
                    id: *id,
                    x: *x,
                    z: *z,
                });
            }
        }
        for minion in snapshot.minions() {
            let MinionState { id, team, x, z, hp } = minion;
            if is_enemy_team(team) && *hp > 0.0 {
                view.units.push(EnemyRef {
                    kind: TargetKind::Minion,
                    id: *id,
                    x: *x,
                    z: *z,
                });
            }
        }
        for structure in snapshot.structures() {
            let StructureState { id, team, x, z, hp } = structure;
            if is_enemy_team(team) && *hp > 0.0 {
                view.structures.push(EnemyRef {
                    kind: TargetKind::Structure,
                    id: *id,
                    x: *x,
                    z: *z,
                });
            }
        }
        view
    }
}

// --- Brain -------------------------------------------------------------------

/// What the bot wants to do this tick. The driver steps toward
/// `move_target` at the legal speed and casts Q at `cast` (throttled).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotDecision {
    pub move_target: Option<(f32, f32)>,
    pub cast: Option<TargetId>,
}

pub struct BotBrain {
    waypoints: Vec<(f32, f32)>,
    next_waypoint: usize,
    q_range: f32,
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = a.0 - b.0;
    let dz = a.1 - b.1;
    (dx * dx + dz * dz).sqrt()
}

fn nearest(from: (f32, f32), enemies: &[EnemyRef]) -> Option<(&EnemyRef, f32)> {
    enemies
        .iter()
        .map(|enemy| (enemy, distance(from, (enemy.x, enemy.z))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

impl BotBrain {
    pub fn new(lane: Lane, team: Team, class: HeroClass) -> Self {
        Self {
            waypoints: lane_waypoints(lane, team),
            next_waypoint: 0,
            q_range: q_cast_range(class),
        }
    }

    /// Re-aims at the nearest waypoint; call after a respawn teleports the
    /// bot back to base so it resumes pushing instead of chasing a stale
    /// mid-lane waypoint from spawn.
    pub fn resync(&mut self, x: f32, z: f32) {
        self.next_waypoint = self
            .waypoints
            .iter()
            .enumerate()
            .min_by(|a, b| distance((x, z), *a.1).total_cmp(&distance((x, z), *b.1)))
            .map(|(index, _)| index)
            .unwrap_or(0);
    }

    /// Pure per-tick decision: fight what is close, otherwise push the lane.
    pub fn decide(&mut self, x: f32, z: f32, view: &WorldView) -> BotDecision {
        let me = (x, z);
        let engage_range = self.q_range * CAST_RANGE_SAFETY;
        let pull_range = engage_range + AGGRO_RANGE;

        // 1. Enemy units (players/minions) pull us off the path.
        if let Some((unit, dist)) = nearest(me, &view.units)
            && dist <= pull_range
        {
            if dist <= engage_range {
                return BotDecision {
                    move_target: None,
                    cast: Some(TargetId {
                        kind: unit.kind,
                        id: unit.id,
                    }),
                };
            }
            return BotDecision {
                move_target: Some((unit.x, unit.z)),
                cast: None,
            };
        }

        // 2. No units around: siege the nearest structure in reach.
        if let Some((structure, dist)) = nearest(me, &view.structures)
            && dist <= pull_range
        {
            if dist <= engage_range {
                return BotDecision {
                    move_target: None,
                    cast: Some(TargetId {
                        kind: structure.kind,
                        id: structure.id,
                    }),
                };
            }
            return BotDecision {
                move_target: Some((structure.x, structure.z)),
                cast: None,
            };
        }

        // 3. Push the lane.
        while self.next_waypoint + 1 < self.waypoints.len()
            && distance(me, self.waypoints[self.next_waypoint]) <= WAYPOINT_REACHED
        {
            self.next_waypoint += 1;
        }
        let target = self.waypoints[self.next_waypoint];
        if distance(me, target) <= WAYPOINT_REACHED {
            // Final waypoint (enemy base) reached: hold.
            return BotDecision {
                move_target: None,
                cast: None,
            };
        }
        BotDecision {
            move_target: Some(target),
            cast: None,
        }
    }
}

/// One legal movement step toward `target`: at most `BOT_MOVE_SPEED * dt`
/// world units, plus the facing yaw (models face -Z, TASK-21 convention).
pub fn step_toward(from: (f32, f32), target: (f32, f32), dt: f32) -> (f32, f32, f32) {
    let dx = target.0 - from.0;
    let dz = target.1 - from.1;
    let dist = (dx * dx + dz * dz).sqrt();
    let yaw = (-dx).atan2(-dz);
    if dist <= f32::EPSILON {
        return (from.0, from.1, yaw);
    }
    let step = (BOT_MOVE_SPEED * dt).min(dist);
    (from.0 + dx / dist * step, from.1 + dz / dist * step, yaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: u64, x: f32, z: f32) -> EnemyRef {
        EnemyRef {
            kind: TargetKind::Minion,
            id,
            x,
            z,
        }
    }

    fn tower(id: u64, x: f32, z: f32) -> EnemyRef {
        EnemyRef {
            kind: TargetKind::Structure,
            id,
            x,
            z,
        }
    }

    #[test]
    fn lanes_start_at_own_base_and_end_at_enemy_base() {
        let m = map_points();
        for lane in Lane::ALL {
            let green = lane_waypoints(lane, Team::Green);
            assert_eq!(
                *green.first().unwrap(),
                m.home,
                "{lane:?} green starts home"
            );
            assert_eq!(*green.last().unwrap(), m.away, "{lane:?} green ends away");
            let blue = lane_waypoints(lane, Team::Blue);
            assert_eq!(*blue.first().unwrap(), m.away, "{lane:?} blue starts away");
            assert_eq!(*blue.last().unwrap(), m.home, "{lane:?} blue ends home");
            // Same geometry, opposite direction.
            let mut reversed = blue.clone();
            reversed.reverse();
            assert_eq!(green, reversed, "{lane:?} blue is the exact reverse");
        }
    }

    #[test]
    fn follows_waypoints_and_advances_on_reach() {
        let mut brain = BotBrain::new(Lane::Top, Team::Green, HeroClass::Warrior);
        let start = brain.waypoints[0];
        let empty = WorldView::default();

        // Far from the first waypoint: steer toward it.
        let decision = brain.decide(start.0 + 20.0, start.1, &empty);
        assert_eq!(decision.move_target, Some(brain.waypoints[0]));
        assert_eq!(decision.cast, None);

        // Standing on waypoint 0: advance to waypoint 1.
        let decision = brain.decide(start.0, start.1, &empty);
        assert_eq!(decision.move_target, Some(brain.waypoints[1]));
    }

    #[test]
    fn holds_at_enemy_base() {
        let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
        let end = *brain.waypoints.last().unwrap();
        brain.resync(end.0, end.1);
        let empty = WorldView::default();
        let decision = brain.decide(end.0, end.1, &empty);
        assert_eq!(decision.move_target, None);
        assert_eq!(decision.cast, None);
    }

    #[test]
    fn approaches_unit_in_aggro_and_casts_in_range() {
        let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
        let range = q_cast_range(HeroClass::Warrior);

        // Inside the pull radius but outside cast range: approach the unit.
        // (Warrior Q range 12 -> engage at 10.8, pull at 19.8; 15 approaches.)
        let engage = range * 0.9;
        assert!(engage < 15.0 && 15.0 < engage + AGGRO_RANGE);
        let near_edge = WorldView {
            units: vec![unit(7, 15.0, 0.0)],
            structures: vec![],
        };
        let decision = brain.decide(0.0, 0.0, &near_edge);
        assert_eq!(decision.move_target, Some((15.0, 0.0)));
        assert_eq!(decision.cast, None);

        // Inside cast range: hold and cast Q at it.
        let close = WorldView {
            units: vec![unit(7, 3.0, 0.0)],
            structures: vec![],
        };
        let decision = brain.decide(0.0, 0.0, &close);
        assert_eq!(decision.move_target, None);
        assert_eq!(
            decision.cast,
            Some(TargetId {
                kind: TargetKind::Minion,
                id: 7
            })
        );
    }

    #[test]
    fn prefers_units_over_structures() {
        let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
        let view = WorldView {
            units: vec![unit(1, 4.0, 0.0)],
            structures: vec![tower(2, 2.0, 0.0)],
        };
        let decision = brain.decide(0.0, 0.0, &view);
        assert_eq!(
            decision.cast,
            Some(TargetId {
                kind: TargetKind::Minion,
                id: 1
            })
        );
    }

    #[test]
    fn sieges_tower_when_no_units_around() {
        let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
        let view = WorldView {
            units: vec![],
            structures: vec![tower(42, 5.0, 0.0)],
        };
        let decision = brain.decide(0.0, 0.0, &view);
        assert_eq!(decision.move_target, None);
        assert_eq!(
            decision.cast,
            Some(TargetId {
                kind: TargetKind::Structure,
                id: 42
            })
        );
    }

    #[test]
    fn resync_after_respawn_picks_nearest_waypoint() {
        let mut brain = BotBrain::new(Lane::Top, Team::Green, HeroClass::Warrior);
        // Simulate mid-lane progress...
        brain.next_waypoint = 3;
        // ...then death + respawn at base: resync back to the start.
        let home = brain.waypoints[0];
        brain.resync(home.0, home.1);
        assert_eq!(brain.next_waypoint, 0);
        // Respawn resync from near the 3rd waypoint keeps pushing there.
        let wp3 = brain.waypoints[3];
        brain.resync(wp3.0 + 1.0, wp3.1);
        assert_eq!(brain.next_waypoint, 3);
    }

    #[test]
    fn step_toward_respects_speed_budget_and_faces_movement() {
        let (x, z, yaw) = step_toward((0.0, 0.0), (100.0, 0.0), 0.1);
        // One 100ms step at PLAYER_SPEED = 0.5 units.
        assert!((x - 0.5).abs() < 1e-5);
        assert!(z.abs() < 1e-5);
        // Facing +X movement: yaw = atan2(-1, 0).
        assert!((yaw - (-1.0_f32).atan2(0.0)).abs() < 1e-5);
        // Short hop clamps to the target, no overshoot.
        let (x, z, _) = step_toward((0.0, 0.0), (0.2, 0.0), 0.1);
        assert!((x - 0.2).abs() < 1e-5 && z.abs() < 1e-5);
    }

    #[test]
    fn world_view_filters_enemies_and_dead_entities() {
        // Constructed via the struct directly (no live snapshot needed):
        // covered indirectly by decide() tests; here we assert the enemy
        // filter logic through from_snapshot with a hand-built packet.
        let raw = serde_json::json!({
            "type": "snapshot",
            "your_id": 1,
            "players": [
                { "id": 1, "x": 0.0, "z": 0.0, "team": "green", "hp": 100.0 },
                { "id": 2, "x": 1.0, "z": 0.0, "team": "blue", "hp": 100.0 },
                { "id": 3, "x": 2.0, "z": 0.0, "team": "blue", "hp": 0.0 },
                { "id": 4, "x": 3.0, "z": 0.0, "team": "green", "hp": 100.0 }
            ],
            "minions": [
                { "id": 10, "x": 5.0, "z": 0.0, "team": "blue", "hp": 30.0 },
                { "id": 11, "x": 6.0, "z": 0.0, "team": "green", "hp": 30.0 },
                { "id": 12, "x": 7.0, "z": 0.0, "team": "blue", "hp": 0.0 }
            ],
            "structures": [
                { "id": 20, "x": 8.0, "z": 0.0, "team": "blue", "hp": 500.0 },
                { "id": 21, "x": 9.0, "z": 0.0, "team": "green", "hp": 500.0 }
            ],
            "game_state": { "type": "running" }
        });
        let snapshot: ServerPacket = serde_json::from_value(raw).unwrap();
        let view = WorldView::from_snapshot(&snapshot, 1, Team::Green);
        let unit_ids: Vec<u64> = view.units.iter().map(|u| u.id).collect();
        assert_eq!(
            unit_ids,
            vec![2, 10],
            "alive blue player + alive blue minion"
        );
        let structure_ids: Vec<u64> = view.structures.iter().map(|s| s.id).collect();
        assert_eq!(structure_ids, vec![20], "only the enemy tower");
    }
}
