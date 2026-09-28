//! Local lane sandbox. Structures use the authored shared map and protection order.
//! Minion tuning mirrors server/balance.rs; the client deliberately has no server dependency.
use super::*;
use shared::combat::{CombatEntity, MinionKind};
use shared::map::{Lane, ResolvedMap, StructureStats};
use shared::wire::{
    MinionBrainState, MinionState, MinionTargetKind, StructureKind, StructureState,
};

const WAVE_INTERVAL: f32 = 60.0;
const MAX_MINIONS: usize = 108;
pub(super) const MAX_SHOTS: usize = 256;
const MINION_SPEED: f32 = 3.1;
const MINION_VISION: f32 = 10.0;

pub(super) struct Tower {
    pub(super) state: StructureState,
    stats: StructureStats,
    cooldown: f32,
}
pub(super) struct Minion {
    pub(super) state: MinionState,
    route: Vec<[f32; 2]>,
    waypoint: usize,
    cooldown: f32,
    age: f32,
}
#[derive(Default)]
pub(super) struct LaneWorld {
    pub(super) structures: Vec<Tower>,
    pub(super) minions: Vec<Minion>,
    wave_in: f32,
    next_id: u64,
}
#[derive(Clone, Copy)]
pub(super) struct TargetInfo {
    id: TargetId,
    pub(super) team: Team,
    pub(super) position: Vec3,
    pub(super) radius: f32,
    pub(super) vulnerable: bool,
}
fn minion_stats(kind: MinionKind) -> (f32, f32, f32, f32) {
    match kind {
        MinionKind::Melee => (65.0, 2.4, 8.0, 0.95),
        MinionKind::Caster => (45.0, 8.0, 7.0, 1.2),
    }
}
impl LaneWorld {
    pub(super) fn new() -> Self {
        let mut lanes = Self {
            structures: ResolvedMap::default()
                .structures
                .into_iter()
                .map(|s| Tower {
                    state: StructureState {
                        protected: false,
                        map_key: s.key,
                        visual_profile: s.visual_profile,
                        lane: s.lane,
                        tier: s.tier,
                        id: s.id,
                        kind: if s.lane.is_some() {
                            StructureKind::Tower
                        } else {
                            StructureKind::BaseTower
                        },
                        team: s.team,
                        x: s.position[0],
                        z: s.position[1],
                        y: if s.lane.is_some() { 3.0 } else { 4.0 },
                        hp: s.stats.max_hp,
                        max_hp: s.stats.max_hp,
                    },
                    stats: s.stats,
                    cooldown: 0.0,
                })
                .collect(),
            next_id: 1,
            wave_in: WAVE_INTERVAL,
            ..default()
        };
        lanes.refresh_protection();
        // Immediate first wave gives a newly opened playground visible lane activity.
        lanes.spawn_wave();
        lanes
    }
    fn refresh_protection(&mut self) {
        let flags: Vec<bool> = self
            .structures
            .iter()
            .map(|target| {
                let t = &target.state;
                if let Some(lane) = t.lane {
                    self.structures.iter().any(|s| {
                        s.state.team == t.team
                            && s.state.lane == Some(lane)
                            && s.state.tier < t.tier
                            && s.state.hp > 0.0
                    })
                } else {
                    let lanes: Vec<_> = [Lane::Top, Lane::Mid, Lane::Bot]
                        .into_iter()
                        .filter(|lane| {
                            self.structures
                                .iter()
                                .any(|s| s.state.team == t.team && s.state.lane == Some(*lane))
                        })
                        .collect();
                    !lanes.is_empty()
                        && lanes.iter().all(|lane| {
                            self.structures.iter().any(|s| {
                                s.state.team == t.team
                                    && s.state.lane == Some(*lane)
                                    && s.state.hp > 0.0
                            })
                        })
                }
            })
            .collect();
        for (tower, protected) in self.structures.iter_mut().zip(flags) {
            tower.state.protected = protected;
        }
    }
    fn spawn_wave(&mut self) {
        // All-or-nothing waves keep both teams symmetric, even in long sandbox sessions.
        if self.minions.len() + 18 > MAX_MINIONS {
            return;
        }
        for team in [Team::Green, Team::Blue] {
            // A destroyed base no longer manufactures minions; practice stays open.
            if !self
                .structures
                .iter()
                .any(|s| s.state.team == team && s.state.lane.is_none() && s.state.hp > 0.0)
            {
                continue;
            }
            for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
                let mut route = shared::map::minion_lane_points(lane);
                if team == Team::Blue {
                    route.reverse();
                }
                if route.len() < 2 {
                    continue;
                }
                let direction =
                    (Vec2::from_array(route[1]) - Vec2::from_array(route[0])).normalize_or_zero();
                for slot in 0..3 {
                    let kind = if slot == 2 {
                        MinionKind::Caster
                    } else {
                        MinionKind::Melee
                    };
                    let hp = minion_stats(kind).0;
                    let pos = Vec2::from_array(route[0]) - direction * slot as f32 * 1.5;
                    self.minions.push(Minion {
                        state: MinionState {
                            kind,
                            attack_sequence: 0,
                            id: self.next_id,
                            team,
                            lane,
                            x: pos.x,
                            y: 0.5,
                            z: pos.y,
                            yaw: shared::math::unit_yaw_towards(direction.x, direction.y),
                            hp,
                            max_hp: hp,
                            state: MinionBrainState::Marching,
                            target_kind: None,
                            target_id: None,
                        },
                        route: route.clone(),
                        waypoint: 1,
                        cooldown: 0.0,
                        age: 0.0,
                    });
                    self.next_id += 1;
                }
            }
        }
    }
}
impl Simulation {
    pub(super) fn target_info(&self, id: TargetId) -> Option<TargetInfo> {
        let (team, position, radius, vulnerable) = match id.kind {
            TargetKind::Player => {
                let p = self.players.iter().find(|p| p.id == id.id && p.hp > 0.0)?;
                (
                    p.team,
                    Vec3::new(p.x, p.y + 0.8, p.z),
                    shared::PLAYER_TARGET_RADIUS,
                    true,
                )
            }
            TargetKind::Minion => {
                let p = &self
                    .lanes
                    .minions
                    .iter()
                    .find(|p| p.state.id == id.id && p.state.hp > 0.0)?
                    .state;
                (
                    p.team,
                    Vec3::new(p.x, p.y + 0.4, p.z),
                    shared::MINION_TARGET_RADIUS,
                    true,
                )
            }
            TargetKind::Structure => {
                let p = &self
                    .lanes
                    .structures
                    .iter()
                    .find(|p| p.state.id == id.id && p.state.hp > 0.0)?
                    .state;
                (
                    p.team,
                    Vec3::new(p.x, p.y, p.z),
                    if p.kind == StructureKind::Tower {
                        shared::TOWER_TARGET_RADIUS
                    } else {
                        shared::BASE_TOWER_TARGET_RADIUS
                    },
                    !p.protected,
                )
            }
            _ => return None,
        };
        Some(TargetInfo {
            id,
            team,
            position,
            radius,
            vulnerable,
        })
    }
    fn lane_targets(&self) -> Vec<TargetInfo> {
        self.players
            .iter()
            // Stationary/ring test targets remain isolated from autonomous lane fire.
            .filter(|p| {
                p.id == LOCAL_ID
                    || self
                        .bots
                        .get(&p.id)
                        .is_some_and(|b| b.kind == BotKind::Duelist)
            })
            .map(|p| TargetId {
                kind: TargetKind::Player,
                id: p.id,
            })
            .chain(self.lanes.minions.iter().map(|p| TargetId {
                kind: TargetKind::Minion,
                id: p.state.id,
            }))
            .chain(self.lanes.structures.iter().map(|p| TargetId {
                kind: TargetKind::Structure,
                id: p.state.id,
            }))
            .filter_map(|id| self.target_info(id))
            .collect()
    }
    pub(super) fn advance_lanes(&mut self, dt: f32) {
        if self.players.is_empty() {
            return;
        }
        self.lanes
            .minions
            .retain(|m| m.state.hp > 0.0 && m.age < 300.0);
        self.lanes.refresh_protection();
        self.lanes.wave_in -= dt;
        if self.lanes.wave_in <= 0.0 {
            self.lanes.wave_in = WAVE_INTERVAL;
            self.lanes.spawn_wave();
        }
        let targets = self.lane_targets();
        let mut strikes = Vec::new();
        for minion in &mut self.lanes.minions {
            minion.age += dt;
            minion.cooldown = (minion.cooldown - dt).max(0.0);
            let m = &mut minion.state;
            let origin = Vec2::new(m.x, m.z);
            let (_, reach, damage, cooldown) = minion_stats(m.kind);
            let target = targets
                .iter()
                .filter(|t| t.team != m.team && t.vulnerable)
                .filter_map(|t| {
                    let distance = origin.distance(Vec2::new(t.position.x, t.position.z));
                    (distance <= MINION_VISION + t.radius).then_some((t, distance))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let goal = if let Some((target, distance)) = target {
                m.target_id = Some(target.id.id);
                m.target_kind = Some(match target.id.kind {
                    TargetKind::Player => MinionTargetKind::Player,
                    TargetKind::Minion => MinionTargetKind::Minion,
                    _ => MinionTargetKind::Structure,
                });
                if distance <= reach + target.radius {
                    m.state = MinionBrainState::Attacking;
                    m.yaw = shared::math::unit_yaw_towards(
                        target.position.x - m.x,
                        target.position.z - m.z,
                    );
                    if minion.cooldown <= 0.0 {
                        minion.cooldown = cooldown;
                        m.attack_sequence += 1;
                        strikes.push((
                            CombatEntityKind::Minion,
                            m.id,
                            m.team,
                            Vec3::new(m.x, m.y + 0.4, m.z),
                            target.id,
                            damage,
                            if m.kind == MinionKind::Caster {
                                ProjectileStyle::CasterBolt
                            } else {
                                ProjectileStyle::Standard
                            },
                        ));
                    }
                    None
                } else {
                    m.state = MinionBrainState::Chasing;
                    Some(Vec2::new(target.position.x, target.position.z))
                }
            } else {
                m.state = MinionBrainState::Marching;
                m.target_id = None;
                m.target_kind = None;
                while minion.waypoint + 1 < minion.route.len()
                    && origin.distance(Vec2::from_array(minion.route[minion.waypoint])) < 0.3
                {
                    minion.waypoint += 1;
                }
                minion
                    .route
                    .get(minion.waypoint)
                    .map(|p| Vec2::from_array(*p))
            };
            if let Some(goal) = goal {
                let delta = goal - origin;
                let step = delta.normalize_or_zero() * (MINION_SPEED * dt).min(delta.length());
                let next = shared::navigation::world_navigation()
                    .clip_movement([m.x, m.z], [m.x + step.x, m.z + step.y]);
                m.x = next[0];
                m.z = next[1];
                if step.length_squared() > 0.000001 {
                    m.yaw = shared::math::unit_yaw_towards(step.x, step.y);
                }
            }
        }
        for tower in &mut self.lanes.structures {
            tower.cooldown = (tower.cooldown - dt).max(0.0);
            let s = &tower.state;
            if s.hp <= 0.0 || tower.cooldown > 0.0 {
                continue;
            }
            let origin = Vec2::new(s.x, s.z);
            // Prefer the wave tanking a tower over heroes, as ordinary lane play does.
            let target = targets
                .iter()
                .filter(|t| t.team != s.team && t.id.kind != TargetKind::Structure)
                .filter(|t| {
                    origin.distance(Vec2::new(t.position.x, t.position.z))
                        <= tower.stats.attack_range
                })
                .min_by(|a, b| {
                    (a.id.kind != TargetKind::Minion)
                        .cmp(&(b.id.kind != TargetKind::Minion))
                        .then_with(|| {
                            origin
                                .distance_squared(Vec2::new(a.position.x, a.position.z))
                                .total_cmp(
                                    &origin.distance_squared(Vec2::new(b.position.x, b.position.z)),
                                )
                        })
                });
            if let Some(t) = target {
                tower.cooldown = tower.stats.attack_cooldown_ms as f32 / 1000.0;
                let damage = tower.stats.attack_damage
                    * if t.id.kind == TargetKind::Player {
                        tower.stats.hero_damage_multiplier
                    } else {
                        1.0
                    };
                strikes.push((
                    CombatEntityKind::Structure,
                    s.id,
                    s.team,
                    Vec3::new(s.x, s.y, s.z),
                    t.id,
                    damage,
                    ProjectileStyle::TowerBolt,
                ));
            }
        }
        for (kind, id, team, position, target, damage, style) in strikes {
            if self.shots.len() >= MAX_SHOTS {
                break;
            }
            self.sequence += 1;
            self.shots.push(Shot {
                state: ProjectileState {
                    id: self.sequence,
                    owner_id: id,
                    owner_team: team,
                    source_kind: kind,
                    style,
                    action_slot: None,
                    direction: [0.0; 3],
                    x: position.x,
                    y: position.y,
                    z: position.z,
                },
                target,
                damage,
                remaining: 3.0,
            });
        }
    }
    pub(super) fn advance_shots(&mut self, dt: f32) {
        for mut shot in std::mem::take(&mut self.shots) {
            shot.remaining -= dt;
            let Some(target) = self.target_info(shot.target) else {
                continue;
            };
            if shot.remaining <= 0.0 || target.team == shot.state.owner_team || !target.vulnerable {
                continue;
            }
            let position = Vec3::new(shot.state.x, shot.state.y, shot.state.z);
            let delta = target.position - position;
            if delta.length() > PROJECTILE_SPEED * dt + 0.3 {
                let direction = delta.normalize();
                let next = position + direction * PROJECTILE_SPEED * dt;
                shot.state.x = next.x;
                shot.state.y = next.y;
                shot.state.z = next.z;
                shot.state.direction = direction.to_array();
                if self.shots.len() < MAX_SHOTS {
                    self.shots.push(shot);
                }
                continue;
            }
            let hp = match shot.target.kind {
                TargetKind::Player => {
                    if shot.target.id == LOCAL_ID && self.god_mode {
                        continue;
                    }
                    &mut self
                        .players
                        .iter_mut()
                        .find(|p| p.id == shot.target.id)
                        .unwrap()
                        .hp
                }
                TargetKind::Minion => {
                    &mut self
                        .lanes
                        .minions
                        .iter_mut()
                        .find(|p| p.state.id == shot.target.id)
                        .unwrap()
                        .state
                        .hp
                }
                TargetKind::Structure => {
                    &mut self
                        .lanes
                        .structures
                        .iter_mut()
                        .find(|p| p.state.id == shot.target.id)
                        .unwrap()
                        .state
                        .hp
                }
                _ => continue,
            };
            let amount = shot.damage.max(0.0).min(*hp);
            *hp -= amount;
            let killed = *hp <= 0.0;
            if killed && shot.target.kind == TargetKind::Player {
                self.stats.entry(shot.target.id).or_default().1 += 1;
                if shot.state.source_kind == CombatEntityKind::Player
                    && shot.state.owner_id != shot.target.id
                {
                    self.stats.entry(shot.state.owner_id).or_default().0 += 1;
                }
            }
            if killed && shot.target.kind == TargetKind::Minion {
                let minion = &mut self
                    .lanes
                    .minions
                    .iter_mut()
                    .find(|p| p.state.id == shot.target.id)
                    .unwrap()
                    .state;
                minion.state = MinionBrainState::Dead;
                minion.target_id = None;
                minion.target_kind = None;
            }
            self.sequence += 1;
            self.events.push_back(CombatEvent {
                id: self.sequence,
                source: CombatEntity {
                    kind: shot.state.source_kind,
                    id: shot.state.owner_id,
                },
                target: CombatEntity {
                    kind: match shot.target.kind {
                        TargetKind::Player => CombatEntityKind::Player,
                        TargetKind::Minion => CombatEntityKind::Minion,
                        _ => CombatEntityKind::Structure,
                    },
                    id: shot.target.id,
                },
                amount,
                x: target.position.x,
                y: target.position.y,
                z: target.position.z,
                style: shot.state.style,
                action_slot: shot.state.action_slot,
                killed,
            });
            if self.events.len() > 32 {
                self.events.pop_front();
            }
            if killed && shot.target.kind == TargetKind::Structure {
                self.lanes.refresh_protection();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn joined() -> Simulation {
        let mut sim = Simulation::default();
        sim.command(ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: default(),
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        sim
    }
    fn run(sim: &mut Simulation, seconds: f32) {
        for _ in 0..(seconds / 0.05).ceil() as usize {
            sim.advance(0.05);
        }
    }
    fn near(sim: &mut Simulation, target: TargetId) {
        let position = sim.target_info(target).unwrap().position;
        sim.players[0].x = position.x + 1.0;
        sim.players[0].y = position.y;
        sim.players[0].z = position.z;
    }
    #[test]
    fn shared_structures_and_both_team_lane_waves_march_and_reset() {
        let mut sim = joined();
        assert_eq!(
            sim.lanes.structures.len(),
            ResolvedMap::default().structures.len()
        );
        assert_eq!(sim.lanes.minions.len(), 18);
        for team in [Team::Green, Team::Blue] {
            for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
                assert_eq!(
                    sim.lanes
                        .minions
                        .iter()
                        .filter(|m| m.state.team == team && m.state.lane == lane)
                        .count(),
                    3
                );
            }
        }
        let start = (sim.lanes.minions[0].state.x, sim.lanes.minions[0].state.z);
        run(&mut sim, 1.0);
        let m = &sim.lanes.minions[0].state;
        assert!((m.x - start.0).hypot(m.z - start.1) > 2.5);
        let ServerPacket::Snapshot {
            structures,
            minions,
            ..
        } = sim.snapshot()
        else {
            panic!()
        };
        assert!(!structures.is_empty() && minions.len() == 18);
        sim.command(ClientPacket::Leave);
        assert!(
            sim.lanes.structures.is_empty() && sim.lanes.minions.is_empty() && sim.shots.is_empty()
        );
        assert_eq!(sim.elapsed, 0.0);
        sim.command(ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: default(),
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        assert_eq!(sim.lanes.minions.len(), 18);
        assert!(
            sim.lanes
                .structures
                .iter()
                .all(|s| s.state.hp == s.state.max_hp)
        );
    }
    #[test]
    fn hero_hits_minions_and_towers_without_id_aliasing_or_hero_kill_credit() {
        let mut sim = joined();
        let id = sim
            .lanes
            .minions
            .iter()
            .find(|m| m.state.team == Team::Blue)
            .unwrap()
            .state
            .id;
        // Deliberately collide numeric ids across kinds, as the actual wire protocol allows.
        sim.players[1].id = id;
        let hero_hp = sim.players[1].hp;
        let target = TargetId {
            kind: TargetKind::Minion,
            id,
        };
        near(&mut sim, target);
        let hp = sim
            .lanes
            .minions
            .iter()
            .find(|m| m.state.id == id)
            .unwrap()
            .state
            .hp;
        sim.command(ClientPacket::Cast { target, slot: 0 });
        assert_eq!(sim.shots.len(), 1);
        for _ in 0..20 {
            sim.advance_shots(0.05);
        }
        assert!(
            sim.lanes
                .minions
                .iter()
                .find(|m| m.state.id == id)
                .unwrap()
                .state
                .hp
                < hp
        );
        assert_eq!(sim.players[1].hp, hero_hp);
        sim.attack(target, 1000.0, None);
        for _ in 0..20 {
            sim.advance_shots(0.05);
        }
        assert_eq!(
            sim.stats.get(&LOCAL_ID).copied().unwrap_or_default(),
            (0, 0)
        );
        sim.advance_lanes(0.05);
        assert!(!sim.lanes.minions.iter().any(|m| m.state.id == id));
        let tower = sim
            .lanes
            .structures
            .iter()
            .find(|s| s.state.team == Team::Blue && !s.state.protected)
            .unwrap()
            .state
            .id;
        let target = TargetId {
            kind: TargetKind::Structure,
            id: tower,
        };
        near(&mut sim, target);
        assert!(sim.valid_target(target, 5.0));
        sim.command(ClientPacket::BasicAttack {
            target,
            server_epoch: u64::MAX,
            match_id: 1,
            request_id: 1,
        });
        for _ in 0..20 {
            sim.advance_shots(0.05);
        }
        let tower = &sim
            .lanes
            .structures
            .iter()
            .find(|s| s.state.id == tower)
            .unwrap()
            .state;
        assert!(tower.hp < tower.max_hp);
        assert!(
            sim.events
                .iter()
                .any(|e| e.target.kind == CombatEntityKind::Structure
                    && e.source.kind == CombatEntityKind::Player)
        );
    }
    #[test]
    fn target_validation_protection_death_and_front_to_back_unlock() {
        let mut sim = joined();
        let ally = TargetId {
            kind: TargetKind::Minion,
            id: sim.lanes.minions[0].state.id,
        };
        near(&mut sim, ally);
        assert!(!sim.valid_target(ally, 1000.0));
        assert!(!sim.valid_target(
            TargetId {
                kind: TargetKind::Neutral,
                id: 1
            },
            1000.0
        ));
        let tower_index = sim
            .lanes
            .structures
            .iter()
            .position(|s| {
                s.state.team == Team::Blue && s.state.lane == Some(Lane::Mid) && s.state.tier == 0
            })
            .unwrap();
        let tower = TargetId {
            kind: TargetKind::Structure,
            id: sim.lanes.structures[tower_index].state.id,
        };
        assert!(!sim.valid_target(tower, 1.0), "distant targets rejected");
        let base_id = sim
            .lanes
            .structures
            .iter()
            .find(|s| s.state.team == Team::Blue && s.state.lane.is_none())
            .unwrap()
            .state
            .id;
        let base = TargetId {
            kind: TargetKind::Structure,
            id: base_id,
        };
        near(&mut sim, base);
        assert!(
            !sim.valid_target(base, 1000.0),
            "base protected before lane cleared"
        );
        for s in &mut sim.lanes.structures {
            if s.state.team == Team::Blue && s.state.lane == Some(Lane::Mid) {
                s.state.hp = 0.0;
            }
        }
        sim.lanes.refresh_protection();
        assert!(sim.valid_target(base, 1000.0));
        assert!(!sim.valid_target(tower, 1000.0), "dead tower rejected");
        sim.attack(base, 100000.0, None);
        for _ in 0..20 {
            sim.advance_shots(0.05);
        }
        assert!(sim.target_info(base).is_none());
        let ServerPacket::Snapshot { game_state, .. } = sim.snapshot() else {
            panic!()
        };
        assert_eq!(
            game_state,
            GameState::Running,
            "practice never produces an online match settlement"
        );
        sim.players[0].hp = 0.0;
        sim.command(ClientPacket::BasicAttack {
            target: tower,
            server_epoch: u64::MAX,
            match_id: 1,
            request_id: 1,
        });
        assert!(sim.shots.is_empty(), "dead hero cannot attack");
    }
    #[test]
    fn autonomous_minions_fight_and_towers_fire_without_attacking_test_dummies() {
        let mut sim = joined();
        sim.lanes
            .minions
            .retain(|m| m.state.lane == Lane::Mid && m.state.kind == MinionKind::Melee);
        sim.lanes.minions.truncate(1);
        // Add a Blue melee on the same road and close enough to fight immediately.
        let mut enemy_world = LaneWorld::new();
        let mut enemy = enemy_world.minions.remove(9);
        enemy.state.lane = Lane::Mid;
        let a = &mut sim.lanes.minions[0].state;
        a.x = 0.0;
        a.z = 0.0;
        enemy.state.x = 1.0;
        enemy.state.z = 0.0;
        let ids = [a.id, enemy.state.id];
        sim.lanes.minions.push(enemy);
        run(&mut sim, 1.0);
        assert!(
            sim.lanes
                .minions
                .iter()
                .filter(|m| ids.contains(&m.state.id))
                .all(|m| m.state.hp < m.state.max_hp)
        );
        assert!(
            sim.events
                .iter()
                .any(|e| e.source.kind == CombatEntityKind::Minion
                    && e.target.kind == CombatEntityKind::Minion)
        );
        let tower = sim
            .lanes
            .structures
            .iter()
            .find(|s| s.state.team == Team::Green && s.state.lane == Some(Lane::Mid))
            .unwrap();
        let (x, z, id) = (tower.state.x, tower.state.z, tower.state.id);
        let enemy = sim
            .lanes
            .minions
            .iter_mut()
            .find(|m| m.state.team == Team::Blue)
            .unwrap();
        enemy.state.x = x + 3.0;
        enemy.state.z = z;
        sim.advance_lanes(0.05);
        assert!(
            sim.shots
                .iter()
                .any(|s| s.state.source_kind == CombatEntityKind::Structure
                    && s.state.owner_id == id)
        );
        let dummy_id = sim.players[1].id;
        assert!(
            !sim.shots
                .iter()
                .any(|s| s.state.source_kind != CombatEntityKind::Player
                    && s.target == TargetId::player(dummy_id))
        );
    }
    #[test]
    fn wave_and_projectile_population_is_bounded_and_clear_bots_preserves_lanes() {
        let mut sim = joined();
        for _ in 0..100 {
            sim.lanes.spawn_wave();
        }
        assert_eq!(sim.lanes.minions.len(), MAX_MINIONS);
        sim.command(ClientPacket::Practice {
            command: shared::practice::PracticeCommand::ClearBots,
        });
        assert_eq!(sim.players.len(), 1);
        assert_eq!(sim.lanes.minions.len(), MAX_MINIONS);
        let target = TargetId {
            kind: TargetKind::Minion,
            id: sim
                .lanes
                .minions
                .iter()
                .find(|m| m.state.team == Team::Blue)
                .unwrap()
                .state
                .id,
        };
        for _ in 0..1000 {
            sim.attack(target, 1.0, None);
        }
        assert_eq!(sim.shots.len(), MAX_SHOTS);
        sim.advance_shots(3.1);
        assert!(
            sim.shots.is_empty(),
            "projectiles expire even when target keeps moving"
        );
        for m in &mut sim.lanes.minions {
            m.age = 301.0;
        }
        sim.advance_lanes(0.05);
        assert!(
            sim.lanes.minions.is_empty(),
            "stuck units cannot accumulate forever"
        );
    }
}
