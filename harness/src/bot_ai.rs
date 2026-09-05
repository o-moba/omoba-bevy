//! Lane-push brain for fill bots (TASK-23).
//!
//! Gives each bot a believable MOBA goal: walk its lane toward the enemy
//! base, fight enemy players/minions met on the way with the Q ability, and
//! siege enemy towers when it reaches them. Deliberately simple — nearest
//! target, bounded tower retreat, and shared-kit self-sustain. The server remains
//! fully authoritative
//! (movement speed budget, cast ranges, cooldowns, damage); this module only
//! decides where a bot *wants* to go and what it *wants* to hit.
//!
//! Map geometry mirrors `server/src/world.rs::build_map_layout` and
//! `lane_control_points` (constants from `server/src/balance.rs`). Keep in
//! sync with the server, same convention as `protocol.rs`.

use shared::{
    HeroClass as SharedClass, SkillSlot, TargetingMode, ability_for_class_slot, scaled_cooldown,
    scaled_mana_cost, unlocked_slots_for_level,
};
use std::time::Instant;

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
/// Conservative envelope covering both lane tower (20) and base (24) range.
pub const TOWER_CAUTION_RADIUS: f32 = 26.0;
const TOWER_HOLD_RADIUS: f32 = TOWER_CAUTION_RADIUS + 2.0;
const MINION_SUPPORT_RADIUS: f32 = 24.0;
const MIN_SUPPORT_MINIONS: usize = 2;
const MIN_SIEGE_HEALTH: f32 = 0.5;

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
#[derive(Debug)]
pub struct WorldView {
    pub units: Vec<EnemyRef>,
    pub structures: Vec<EnemyRef>,
    pub friendly_minions: Vec<EnemyRef>,
    pub protected_structures: Vec<u64>,
    pub health_fraction: f32,
}

impl Default for WorldView {
    fn default() -> Self {
        Self {
            units: Vec::new(),
            structures: Vec::new(),
            friendly_minions: Vec::new(),
            protected_structures: Vec::new(),
            health_fraction: 1.0,
        }
    }
}

impl WorldView {
    /// Builds the enemy view for a bot on `my_team` with player id `my_id`.
    pub fn from_snapshot(snapshot: &ServerPacket, my_id: u64, my_team: Team) -> Self {
        let is_enemy_team = |team: &Option<Team>| team.is_some_and(|t| t != my_team);
        let mut view = WorldView::default();
        if let Some(me) = snapshot.player(my_id) {
            view.health_fraction = if me.max_hp > 0.0 {
                me.hp / me.max_hp
            } else {
                0.0
            };
        }
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
            if *team == Some(my_team) && *hp > 0.0 {
                view.friendly_minions.push(EnemyRef {
                    kind: TargetKind::Minion,
                    id: *id,
                    x: *x,
                    z: *z,
                });
            }
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
            let StructureState {
                id,
                team,
                x,
                z,
                hp,
                protected,
            } = structure;
            if is_enemy_team(team) && *hp > 0.0 {
                if *protected {
                    view.protected_structures.push(*id);
                }
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

fn tower_supported(me: (f32, f32), tower: &EnemyRef, view: &WorldView) -> bool {
    view.health_fraction >= MIN_SIEGE_HEALTH
        && view
            .friendly_minions
            .iter()
            .filter(|minion| {
                let d = distance((minion.x, minion.z), (tower.x, tower.z));
                d < MINION_SUPPORT_RADIUS && d < distance(me, (tower.x, tower.z))
            })
            .take(MIN_SUPPORT_MINIONS)
            .count()
            >= MIN_SUPPORT_MINIONS
}

fn segment_circle_entry(
    from: (f32, f32),
    to: (f32, f32),
    center: (f32, f32),
    radius: f32,
) -> Option<f32> {
    let delta = (to.0 - from.0, to.1 - from.1);
    let offset = (from.0 - center.0, from.1 - center.1);
    let a = delta.0 * delta.0 + delta.1 * delta.1;
    if a < 0.0001 {
        return None;
    }
    let b = 2.0 * (offset.0 * delta.0 + offset.1 * delta.1);
    let c = offset.0 * offset.0 + offset.1 * offset.1 - radius * radius;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let entry = (-b - discriminant.sqrt()) / (2.0 * a);
    (0.0..=1.0).contains(&entry).then_some(entry)
}

fn authoritative_class(me: &PlayerState) -> SharedClass {
    me.hero_class
        .as_deref()
        .and_then(SharedClass::from_id)
        .unwrap_or_default()
}

/// Spend one point, prioritizing Q then W/E/R, only on an unlocked legal rank.
pub fn choose_skill_upgrade(me: &PlayerState) -> Option<u8> {
    if me.skill_points == 0 {
        return None;
    }
    let unlocked = unlocked_slots_for_level(me.level);
    (0..4u8).find(|&index| {
        let slot = SkillSlot::from_index(index).unwrap();
        unlocked[index as usize]
            && me.ranks[index as usize]
                < ability_for_class_slot(authoritative_class(me), slot).max_rank
    })
}

/// Shared-kit affordability/unlock/cooldown checks before any outbound cast.
pub fn can_cast_slot(
    me: &PlayerState,
    slot: u8,
    last_casts: &[Option<Instant>; 4],
    now: Instant,
) -> bool {
    let Some(skill_slot) = SkillSlot::from_index(slot) else {
        return false;
    };
    let def = ability_for_class_slot(authoritative_class(me), skill_slot);
    let index = slot as usize;
    let rank = me.ranks[index].clamp(1, def.max_rank);
    me.hp > 0.0
        && unlocked_slots_for_level(me.level)[index]
        && me.mana >= scaled_mana_cost(def, rank)
        && last_casts[index]
            .is_none_or(|last| now.saturating_duration_since(last) >= scaled_cooldown(def, rank))
}

/// A self heal or mana restore is useful after at least 20% of that resource is missing.
pub fn choose_self_sustain(
    me: &PlayerState,
    last_casts: &[Option<Instant>; 4],
    now: Instant,
) -> Option<u8> {
    (0..4u8).find(|&index| {
        let def = ability_for_class_slot(
            authoritative_class(me),
            SkillSlot::from_index(index).unwrap(),
        );
        let needs_hp = def.self_heal.is_some_and(|amount| amount > 0.0)
            && me.max_hp > 0.0
            && me.hp <= me.max_hp * 0.8;
        let needs_mana = def.self_mana_restore.is_some_and(|amount| amount > 0.0)
            && me.max_mana > 0.0
            && me.mana <= me.max_mana * 0.8;
        def.targeting == TargetingMode::SelfTarget
            && (needs_hp || needs_mana)
            && can_cast_slot(me, index, last_casts, now)
    })
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
        let unsupported = view
            .structures
            .iter()
            .filter(|tower| !tower_supported(me, tower, view))
            .collect::<Vec<_>>();
        if let Some(tower) = unsupported
            .iter()
            .filter(|tower| distance(me, (tower.x, tower.z)) < TOWER_HOLD_RADIUS)
            .min_by(|a, b| distance(me, (a.x, a.z)).total_cmp(&distance(me, (b.x, b.z))))
        {
            let mut away = (x - tower.x, z - tower.z);
            let mut length = distance((0.0, 0.0), away);
            if length < 0.001 {
                away = (self.waypoints[0].0 - tower.x, self.waypoints[0].1 - tower.z);
                length = distance((0.0, 0.0), away);
            }
            if length < 0.001 {
                away = (-1.0, 0.0);
                length = 1.0;
            }
            return BotDecision {
                move_target: Some((
                    tower.x + away.0 / length * TOWER_HOLD_RADIUS,
                    tower.z + away.1 / length * TOWER_HOLD_RADIUS,
                )),
                cast: None,
            };
        }
        let mut decision = self.decide_intent(x, z, view);
        if let Some(target) = decision.move_target {
            let mut fraction = 1.0_f32;
            for tower in unsupported {
                if let Some(entry) =
                    segment_circle_entry(me, target, (tower.x, tower.z), TOWER_HOLD_RADIUS)
                {
                    fraction = fraction.min(entry);
                }
            }
            if fraction < 1.0 {
                let clipped = (x + (target.0 - x) * fraction, z + (target.1 - z) * fraction);
                decision.move_target = (distance(me, clipped) > 0.1).then_some(clipped);
            }
        }
        decision
    }

    fn decide_intent(&mut self, x: f32, z: f32, view: &WorldView) -> BotDecision {
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
        let attackable = view
            .structures
            .iter()
            .filter(|tower| !view.protected_structures.contains(&tower.id))
            .copied()
            .collect::<Vec<_>>();
        if let Some((structure, dist)) = nearest(me, &attackable)
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
            ..Default::default()
        };
        let decision = brain.decide(0.0, 0.0, &near_edge);
        assert_eq!(decision.move_target, Some((15.0, 0.0)));
        assert_eq!(decision.cast, None);

        // Inside cast range: hold and cast Q at it.
        let close = WorldView {
            units: vec![unit(7, 3.0, 0.0)],
            structures: vec![],
            ..Default::default()
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
            friendly_minions: vec![unit(50, 1.5, 0.0), unit(51, 2.0, 0.0)],
            ..Default::default()
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
            friendly_minions: vec![unit(50, 4.0, 0.0), unit(51, 5.0, 0.0)],
            ..Default::default()
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
        assert_eq!(
            view.friendly_minions
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            vec![11]
        );
    }
    fn hero(class: &str, level: u32) -> PlayerState {
        serde_json::from_value(serde_json::json!({"id":1,"hp":50.0,"max_hp":100.0,
            "mana":100.0,"max_mana":100.0,"level":level,"hero_class":class,"skill_points":3,"ranks":[1,1,1,1]})).unwrap()
    }

    #[test]
    fn upgrades_respect_authoritative_points_unlocks_and_caps() {
        for class in SharedClass::ALL {
            let mut me = hero(class.id(), 1);
            assert_eq!(choose_skill_upgrade(&me), Some(0));
            me.ranks[0] = class.abilities()[0].max_rank;
            assert_eq!(
                choose_skill_upgrade(&me),
                None,
                "locked slots cannot receive upgrades"
            );
            me.level = 2;
            assert_eq!(choose_skill_upgrade(&me), Some(1));
            me.skill_points = 0;
            assert_eq!(choose_skill_upgrade(&me), None);
            me.skill_points = 5;
            me.level = 10;
            me.ranks = class.abilities().map(|def| def.max_rank);
            assert_eq!(choose_skill_upgrade(&me), None);
        }
    }

    #[test]
    fn self_sustain_requires_need_unlock_mana_and_shared_cooldown() {
        let now = Instant::now();
        let mut me = hero("warrior", 1);
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), None);
        me.level = 2;
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), Some(1));
        me.hp = 100.0;
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), None);
        me.hp = 50.0;
        me.mana = 0.0;
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), None);
        me.mana = 100.0;
        let last = [None, Some(now), None, None];
        assert_eq!(choose_self_sustain(&me, &last, now), None);
        let cooldown = scaled_cooldown(
            ability_for_class_slot(SharedClass::Warrior, SkillSlot::W),
            1,
        );
        assert_eq!(choose_self_sustain(&me, &last, now + cooldown), Some(1));
        me.hp = 0.0;
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), None);
    }

    #[test]
    fn cleric_uses_unlocked_mana_restore_at_zero_mana() {
        let now = Instant::now();
        let mut me = hero("cleric", 4);
        me.hp = 100.0;
        me.mana = 0.0;
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), Some(2));
        me.level = 2;
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), None);
        me.level = 4;
        me.mana = 100.0;
        assert_eq!(choose_self_sustain(&me, &[None; 4], now), None);
    }

    #[test]
    fn bot_retreats_without_two_frontline_minions_and_when_low_health() {
        let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
        let mut view = WorldView {
            units: vec![unit(2, 3.0, 0.0)],
            structures: vec![tower(42, 5.0, 0.0)],
            ..Default::default()
        };
        for allies in [
            vec![],
            vec![unit(50, 4.0, 0.0)],
            vec![unit(50, -5.0, 0.0), unit(51, -6.0, 0.0)],
        ] {
            view.friendly_minions = allies;
            let decision = brain.decide(0.0, 0.0, &view);
            assert_eq!(decision.cast, None);
            let retreat = decision.move_target.unwrap();
            assert!(distance(retreat, (5.0, 0.0)) >= TOWER_CAUTION_RADIUS);
            assert!(retreat.0 < 0.0);
        }
        view.friendly_minions = vec![unit(50, 4.0, 0.0), unit(51, 5.0, 0.0)];
        assert!(brain.decide(0.0, 0.0, &view).cast.is_some());
        view.health_fraction = 0.49;
        assert_eq!(brain.decide(0.0, 0.0, &view).cast, None);
    }

    #[test]
    fn pursuing_units_cannot_cross_an_unsupported_tower_circle() {
        let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
        let view = WorldView {
            units: vec![unit(2, 15.0, 0.0)],
            structures: vec![tower(42, 40.0, 0.0)],
            ..Default::default()
        };
        let decision = brain.decide(0.0, 0.0, &view);
        let target = decision.move_target.unwrap();
        assert!(target.0 <= 12.001 && target.0 >= 11.999);
        assert_eq!(decision.cast, None);
        let held = brain.decide(12.0, 0.0, &view);
        assert_eq!(held.move_target, None);
    }

    #[test]
    fn protected_base_is_not_selected_even_with_minion_support() {
        let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
        let view = WorldView {
            structures: vec![tower(42, 5.0, 0.0)],
            friendly_minions: vec![unit(50, 4.0, 0.0), unit(51, 5.0, 0.0)],
            protected_structures: vec![42],
            ..Default::default()
        };
        assert_eq!(brain.decide(0.0, 0.0, &view).cast, None);
    }
}
