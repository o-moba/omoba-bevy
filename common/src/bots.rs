//! Practice-only server controllers. Actors use ordinary hero movement, attacks,
//! resources and receipts; internal addresses are never network endpoints.
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::map::{Lane, Team};
use shared::progression::skill_upgrade_order;
use shared::shop::plan_purchases;
use shared::wire::{GameState, TargetId, TargetKind, default_character_choice};
use shared::{
    HeroClass, SkillSlot, TargetingMode, ability_for_class_slot, unlocked_slots_for_level,
};

use crate::balance::PLAYER_GROUND_Y;
use crate::entities::{ConnectedPlayer, MapLayoutState, StructureRole};
use crate::host::CombatHost;
use crate::session::{handle_join_request_with_sprite, handle_transform_request_with_structures};
use crate::sim::cast::{apply_skill_upgrade, handle_cast_request};
use crate::sim::towers::structure_is_protected;
use crate::world::{build_minion_path, spawn_position_for_team, structure_collision_radius};
use crate::{basic_attack, hero_stats, match_stats, shop, vision};

const THINK_INTERVAL: Duration = Duration::from_millis(250);
const ROUTE_INTERVAL: Duration = Duration::from_millis(800);
use shared::hero_balance::BOT_ENGAGE_RANGE as VISION;
use shared::math::hero_yaw_towards;
/// A five-player team, one class per duty. Bots take the first entry whose
/// class is not on their team yet (humans pick first); the lane is the fallback
/// when the Warden has no living camp to clear.
const BOT_COMPOSITION: [(HeroClass, Lane, bool); 5] = [
    (HeroClass::Mage, Lane::Mid, false),
    (HeroClass::Warrior, Lane::Top, false),
    (HeroClass::Ranger, Lane::Bot, false),
    (HeroClass::Warden, Lane::Mid, true),
    (HeroClass::Cleric, Lane::Bot, false),
];
const HERO_SPACING: f32 = shared::PLAYER_TARGET_RADIUS * 2.0 + 0.08;

/// Exactly the living minions the tower authority can prefer over a hero.
/// A nearby wave outside its actual three-dimensional range is not cover.
fn tower_has_minion_cover(
    tower: &crate::entities::Structure,
    team: Team,
    minions: &HashMap<u64, crate::entities::Minion>,
) -> bool {
    let position = crate::entities::Vec3f::new(tower.state.x, tower.state.y, tower.state.z);
    minions.values().any(|minion| {
        minion.state.team == team
            && minion.state.hp > 0.0
            && position.distance_squared(crate::entities::Vec3f::new(
                minion.state.x,
                minion.state.y,
                minion.state.z,
            )) <= tower.attack_range.powi(2)
    })
}

/// Autonomous mobility needs room for its actual displacement, which can run
/// opposite the aim or follow a moving recast target. Withhold it before the
/// authority spends resources; ordinary ranged damage remains available.
fn bot_mobility_safe(
    definition: &shared::loadout::SkillDefinition,
    origin: [f32; 2],
    recast: bool,
    danger: &[shared::navigation::Disc],
) -> bool {
    use shared::loadout::{SkillEffect, Technique};
    let SkillEffect::Technique { action, .. } = definition.effect else {
        return true;
    };
    if !matches!(
        action,
        Technique::CollisionCharge
            | Technique::Lunge
            | Technique::EchoStrike
            | Technique::Hook
            | Technique::GuardLeap
            | Technique::AllyLeap
            | Technique::Lash
            | Technique::ExecuteRetreat
            | Technique::SpiritDash
            | Technique::BlinkShot
    ) {
        return true;
    }
    // These recasts follow the earlier victim, which can have moved beyond the
    // initial cast range; the AI has no safe fixed travel bound for that jump.
    if recast && matches!(action, Technique::EchoStrike | Technique::Hook) {
        return danger.is_empty();
    }
    // Seven covers the backwards execute; extra room covers target hitboxes
    // and the one-unit landing offset used by targeted leaps.
    let travel = definition.ability.cast_range.max(7.0) + 3.0;
    danger.iter().all(|disc| {
        (origin[0] - disc.center[0]).hypot(origin[1] - disc.center[1]) > disc.radius + travel
    })
}

/// Stop on the approach side of the first unsupported lane objective. Routing
/// around a danger disc must not let an idle pusher bypass its defended tower.
fn lane_staging_destination(
    world: &crate::game_world::GameWorld,
    team: Team,
    lane: Lane,
    destination: [f32; 2],
) -> [f32; 2] {
    let path = build_minion_path(&world.map_layout, lane, team);
    let progress = |point: [f32; 2]| {
        let mut traversed = 0.0;
        let mut best = (f32::INFINITY, 0.0);
        for pair in path.windows(2) {
            let delta = [pair[1].x - pair[0].x, pair[1].z - pair[0].z];
            let length = delta[0].hypot(delta[1]);
            if length <= 0.001 {
                continue;
            }
            let along =
                ((point[0] - pair[0].x) * delta[0] + (point[1] - pair[0].z) * delta[1]) / length;
            let along = along.clamp(0.0, length);
            let offset = (point[0] - pair[0].x - delta[0] / length * along)
                .hypot(point[1] - pair[0].z - delta[1] / length * along);
            if offset < best.0 {
                best = (offset, traversed + along);
            }
            traversed += length;
        }
        best.1
    };
    let stop = world
        .structures
        .values()
        .filter(|tower| {
            tower.state.team != team
                && tower.state.hp > 0.0
                && tower.attack_range > 0.0
                && tower.attack_damage > 0.0
                && match tower.role {
                    StructureRole::LaneTower { lane: tower_lane } => tower_lane == lane,
                    StructureRole::BaseTower => true,
                }
                && !tower_has_minion_cover(tower, team, &world.minions)
        })
        .map(|tower| {
            (progress([tower.state.x, tower.state.z])
                - tower.attack_range
                - shared::PLAYER_TARGET_RADIUS
                - 1.5)
                .max(0.0)
        })
        .min_by(f32::total_cmp);
    let Some(mut remaining) = stop.filter(|stop| *stop < progress(destination)) else {
        return destination;
    };
    for pair in path.windows(2) {
        let length = (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z);
        if remaining <= length && length > 0.001 {
            return [
                pair[0].x + (pair[1].x - pair[0].x) * remaining / length,
                pair[0].z + (pair[1].z - pair[0].z) * remaining / length,
            ];
        }
        remaining -= length;
    }
    destination
}

/// Sweep a bot's whole step against living heroes. Existing spawn/respawn
/// overlaps may recover outwards, but a step cannot enter a different hero.
fn clip_hero_step(from: [f32; 2], to: [f32; 2], others: &[(u64, [f32; 2])]) -> [f32; 2] {
    let delta = [to[0] - from[0], to[1] - from[1]];
    let length_sq = delta[0] * delta[0] + delta[1] * delta[1];
    if length_sq < 0.000_000_1 {
        return from;
    }
    let mut fraction = 1.0_f32;
    for (_, center) in others {
        let offset = [from[0] - center[0], from[1] - center[1]];
        let dot = offset[0] * delta[0] + offset[1] * delta[1];
        let c = offset[0] * offset[0] + offset[1] * offset[1] - HERO_SPACING.powi(2);
        if c < 0.0 {
            if dot < -0.000_01 {
                return from;
            }
            continue;
        }
        if dot >= 0.0 {
            continue;
        }
        let discriminant = dot * dot - length_sq * c;
        if discriminant >= 0.0 {
            let contact = (-dot - discriminant.sqrt()) / length_sq;
            if (0.0..=fraction).contains(&contact) {
                fraction = (contact - 0.001 / length_sq.sqrt()).max(0.0);
            }
        }
    }
    [from[0] + delta[0] * fraction, from[1] + delta[1] * fraction]
}

fn crowd_penetration(point: [f32; 2], others: &[(u64, [f32; 2])]) -> f32 {
    others
        .iter()
        .map(|(_, p)| (HERO_SPACING - (point[0] - p[0]).hypot(point[1] - p[1])).max(0.0))
        .sum()
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[test]
    fn hero_sweep_blocks_crossing_and_allows_only_outward_spawn_recovery() {
        let others = [(2, [0.0, 0.0])];
        let stopped = clip_hero_step([-5.0, 0.0], [5.0, 0.0], &others);
        assert!(stopped[0] < -HERO_SPACING);
        assert_eq!(clip_hero_step([0.5, 0.0], [0.4, 0.0], &others), [0.5, 0.0]);
        assert_eq!(clip_hero_step([0.5, 0.0], [0.7, 0.0], &others), [0.7, 0.0]);
        assert_eq!(clip_hero_step([0.0, 0.0], [0.2, 0.0], &others), [0.2, 0.0]);
    }
}

/// Local steering never moves humans or bypasses terrain/structure authority.
/// Fixed candidate order gives every bot a consistent passing side. Stationary
/// combatants only move when actually overlapping, so avoidance cannot make an
/// otherwise settled attack stance oscillate.
fn steer_bot_step(
    id: u64,
    origin: [f32; 2],
    desired: [f32; 2],
    max_step: f32,
    others: &[(u64, [f32; 2])],
    structures: &[shared::navigation::Disc],
) -> [f32; 2] {
    let delta = [desired[0] - origin[0], desired[1] - origin[1]];
    let distance = delta[0].hypot(delta[1]);
    let penetration = crowd_penetration(origin, others);
    if max_step <= 0.0 || (distance < 0.001 && penetration < 0.001) {
        return origin;
    }
    let heading = if distance > 0.001 {
        delta[1].atan2(delta[0])
    } else {
        // Different deterministic headings unstack heroes sharing a spawn.
        id as f32 * 2.399_963_1
    };
    let step = if penetration > 0.001 {
        max_step
    } else {
        max_step.min(distance)
    };
    let nav = shared::navigation::world_navigation();
    let straight = clip_hero_step(
        origin,
        [
            origin[0] + heading.cos() * step,
            origin[1] + heading.sin() * step,
        ],
        others,
    );
    let blocked_ahead = (straight[0] - origin[0]).hypot(straight[1] - origin[1]) < step * 0.75;
    let mut best = origin;
    let mut best_score = 0.0;
    for angle in [
        0.0_f32,
        0.4,
        -0.4,
        0.8,
        -0.8,
        1.2,
        -1.2,
        1.6,
        -1.6,
        2.4,
        -2.4,
        std::f32::consts::PI,
    ] {
        let direction = heading + angle;
        let candidate = clip_hero_step(
            origin,
            [
                origin[0] + direction.cos() * step,
                origin[1] + direction.sin() * step,
            ],
            others,
        );
        if !nav.segment_clear_with_discs(origin, candidate, structures) {
            continue;
        }
        let advance = if distance > 0.001 {
            ((candidate[0] - origin[0]) * delta[0] + (candidate[1] - origin[1]) * delta[1])
                / distance
        } else {
            0.0
        };
        let recovery = penetration - crowd_penetration(candidate, others);
        let detour = if blocked_ahead && distance > 0.001 {
            (candidate[0] - origin[0]).hypot(candidate[1] - origin[1]) * angle.sin().abs() * 0.35
        } else {
            0.0
        };
        let score = advance + recovery * 12.0 + detour - angle.abs() * 0.000_1;
        if score > best_score + 0.000_01 {
            best_score = score;
            best = candidate;
        }
    }
    best
}

pub fn bot_avatar(class: HeroClass, slot: u16) -> Option<&'static str> {
    // Fixed bundled models only; runtime-synced/paid cosmetics are never selected.
    // Each class alternates two free appearances without changing its sprite kit.
    let preferred = match class {
        HeroClass::Warrior => ["good-knight", "bao-samurai"],
        HeroClass::Ranger | HeroClass::Wildspark => ["megan-the-fox", "cyberpal"],
        HeroClass::Mage | HeroClass::Dawnweaver => ["agnes", "stitch-witch"],
        HeroClass::Cleric => ["anna", "mega-angel"],
        HeroClass::Warden => ["cool-tiger", "lady-koi"],
        _ => ["good-knight", "agnes"],
    };
    let variant = ((slot.saturating_sub(1) / 4) % 2) as usize;
    Some(preferred[variant])
}

#[derive(Default)]
pub struct BotControllers {
    controllers: HashMap<SocketAddr, Controller>,
    pub defer_fill: bool,
    /// A practice sandbox command replaced the standard roster: no automatic
    /// refill or trimming until `PracticeCommand::Roster` or a round reset
    /// (set by `debug::practice`).
    pub sandbox: bool,
}

impl BotControllers {
    /// Attach AI to an existing retained human without changing its identity,
    /// build, progression or the controller's current plan on repeated votes.
    pub fn attach_existing(&mut self, addr: SocketAddr, class: HeroClass, now: Instant) {
        let (_, lane, jungle) = BOT_COMPOSITION
            .iter()
            .copied()
            .find(|(candidate, ..)| *candidate == class)
            .unwrap_or((class, Lane::Mid, false));
        self.controllers.entry(addr).or_insert_with(|| Controller {
            kind: BotKind::Duelist,
            lane,
            jungle,
            waypoint: 1,
            route: VecDeque::new(),
            goal: None,
            next_route: now,
            next_think: now,
            next_retreat: now,
            retreat_until: None,
            target: None,
            holding_range: false,
        });
    }

    pub fn detach(&mut self, addr: SocketAddr) {
        self.controllers.remove(&addr);
    }

    pub fn clear(&mut self) {
        self.controllers.clear();
        self.sandbox = false;
    }

    pub fn kind(&self, addr: SocketAddr) -> Option<BotKind> {
        self.controllers.get(&addr).map(|c| c.kind)
    }

    /// Standard roster bots count toward the team size; sandbox spawns never do.
    fn is_lane_bot(&self, addr: SocketAddr) -> bool {
        self.kind(addr) == Some(BotKind::Lane)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotKind {
    /// Standard practice roster: walks its lane and fights what it meets.
    Lane,
    /// Sandbox 1v1 opponent: the lane AI with a configured level and budget.
    Duelist,
    /// Sandbox target: stands at `anchor`, never moves or attacks, and walks
    /// back to the anchor after every respawn.
    Dummy { anchor: [f32; 2] },
    MovingDummy {
        anchor: [f32; 2],
        started: Instant,
        respawning: bool,
    },
}

struct Controller {
    kind: BotKind,
    lane: Lane,
    /// Clears jungle camps before following `lane`.
    jungle: bool,
    waypoint: usize,
    route: VecDeque<[f32; 2]>,
    goal: Option<[f32; 2]>,
    next_route: Instant,
    next_think: Instant,
    next_retreat: Instant,
    retreat_until: Option<Instant>,
    target: Option<TargetId>,
    holding_range: bool,
}

// The unspecified IPv6 source cannot identify a remote UDP peer. Still reject
// this whole namespace explicitly before processing ANY received packet.
pub fn is_bot_address(addr: SocketAddr) -> bool {
    matches!(addr, SocketAddr::V6(addr) if addr.ip().is_unspecified())
}

pub fn human_count(players: &HashMap<SocketAddr, ConnectedPlayer>, team: Team) -> u32 {
    players
        .values()
        .filter(|p| p.joined && !p.hero.identity.is_bot && p.hero.identity.team == team)
        .count() as u32
}

pub fn assign_human_team(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    team_size: u32,
) -> Option<Team> {
    let green = human_count(players, Team::Green);
    let blue = human_count(players, Team::Blue);
    if green >= team_size && blue >= team_size {
        None
    } else if green <= blue && green < team_size {
        Some(Team::Green)
    } else {
        Some(Team::Blue)
    }
}

pub fn remove_replaced_bot(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    bots: &mut BotControllers,
    ledger: &mut match_stats::RoundLedger,
    team: Team,
) {
    let addr = players
        .iter()
        .filter(|(addr, p)| {
            p.hero.identity.is_bot && p.hero.identity.team == team && bots.is_lane_bot(**addr)
        })
        .min_by_key(|(_, p)| p.hero.identity.id)
        .map(|(addr, _)| *addr);
    if let Some(addr) = addr {
        remove_bot(players, bots, ledger, addr);
    }
}

/// Retire one bot: its ledger row stays (immutable round history) but it
/// leaves the simulation, the snapshots and the controller table.
pub fn remove_bot(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    bots: &mut BotControllers,
    ledger: &mut match_stats::RoundLedger,
    addr: SocketAddr,
) {
    if let Some(player) = players.remove(&addr) {
        ledger.update_earned_gold(player.hero.identity.id, player.economy.earned_gold);
        ledger.update_player(player.hero.identity.id, player.hero.progress.level, true);
    }
    bots.controllers.remove(&addr);
}

/// Team-size seat count: humans plus standard lane bots. Sandbox dummies and
/// duelists are extras and never displace or block the roster.
pub fn seated_count(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    bots: &BotControllers,
    team: Team,
) -> usize {
    players
        .iter()
        .filter(|(addr, p)| {
            p.joined
                && p.hero.identity.team == team
                && (!p.hero.identity.is_bot || bots.is_lane_bot(**addr))
        })
        .count()
}

/// Spend every skill point the way a player would
/// (`shared::progression::skill_upgrade_order`: the ultimate first once it
/// unlocks, then Q, W and E, never into a locked slot), through the ordinary
/// upgrade path.
pub fn auto_rank_skills(player: &mut ConnectedPlayer) {
    let progress = &player.hero.progress;
    let order = skill_upgrade_order(
        player.hero.identity.hero_class,
        progress.level,
        progress.ranks,
        progress.skill_points,
    );
    for slot in order {
        apply_skill_upgrade(player, slot);
    }
}

/// Buy what `shared::shop::plan_purchases` picks (the class's recommended
/// items in order while gold and inventory allow), one ordinary purchase per
/// item, so the bot must stand in its own base.
pub fn auto_shop(
    player: &mut ConnectedPlayer,
    map: &MapLayoutState,
    phase: &GameState,
    match_id: u64,
) {
    if !shop::shop_is_available(&player.hero, map, phase) {
        return;
    }
    let plan = plan_purchases(
        player.hero.identity.hero_class,
        player.economy.gold,
        &player.economy.inventory,
    );
    for item_id in plan {
        let request = player.economy.purchase_sequence + 1;
        shop::handle_purchase(player, map, phase, item_id.id(), request, match_id);
    }
}

/// Put a dummy on its anchor, facing `toward`, without a movement envelope
/// check: this is a placement, not a step. Used when a dummy spawns
/// (`debug::practice`) and when it walks back after a respawn.
pub fn place_dummy(player: &mut ConnectedPlayer, anchor: [f32; 2], toward: [f32; 2], now: Instant) {
    player.hero.x = anchor[0];
    player.hero.y = PLAYER_GROUND_Y;
    player.hero.z = anchor[1];
    player.hero.yaw = hero_yaw_towards(toward[0] - anchor[0], toward[1] - anchor[1]);
    player.timers.last_movement_at = now;
}

impl CombatHost<'_> {
    /// Create one joined bot on `team`. `class` defaults to the next roster
    /// composition class; sandbox bots take mid.
    pub fn spawn_bot(
        &mut self,
        team: Team,
        class: Option<HeroClass>,
        kind: BotKind,
        now: Instant,
    ) -> Option<SocketAddr> {
        let slot = (1..=32_u16).find(|slot| {
            !self
                .world
                .players
                .contains_key(&SocketAddr::from(([0_u16; 8], *slot)))
        })?;
        let addr = SocketAddr::from(([0_u16; 8], slot));
        self.world.ensure_connected(addr, now);
        // Roster bots complete the Mid/Solo/Carry/Jungle/Support composition;
        // sandbox bots take the requested class and hold mid without jungling.
        let teammates: Vec<_> = self
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.identity.team == team)
            .map(|p| p.hero.identity.hero_class)
            .collect();
        let (composition_class, composition_lane, composition_jungle) = BOT_COMPOSITION
            .into_iter()
            .find(|(class, ..)| !teammates.contains(class))
            .unwrap_or(BOT_COMPOSITION[teammates.len() % BOT_COMPOSITION.len()]);
        let class = class.unwrap_or(composition_class);
        let (lane, jungle) = if kind == BotKind::Lane {
            (composition_lane, composition_jungle)
        } else {
            (Lane::Mid, false)
        };
        let player = self.world.players.get_mut(&addr).unwrap();
        player.hero.identity.is_bot = true;
        handle_join_request_with_sprite(
            player,
            team,
            default_character_choice(),
            class,
            bot_avatar(class, slot),
            None,
            &self.world.map_layout,
            now,
        );
        self.bots.controllers.insert(
            addr,
            Controller {
                kind,
                lane,
                jungle,
                waypoint: 1,
                route: VecDeque::new(),
                goal: None,
                next_route: now,
                next_think: now,
                next_retreat: now,
                retreat_until: None,
                target: None,
                holding_range: false,
            },
        );
        Some(addr)
    }

    pub fn remove_all_bots(&mut self) {
        let addresses: Vec<_> = self
            .world
            .players
            .iter()
            .filter(|(_, p)| p.hero.identity.is_bot)
            .map(|(addr, _)| *addr)
            .collect();
        for addr in addresses {
            remove_bot(
                &mut self.world.players,
                self.bots,
                &mut self.combat_log.ledger,
                addr,
            );
        }
    }

    pub fn simulate_bots(&mut self, now: Instant, dt: f32) {
        if !matches!(self.world.game_state, GameState::Running) || dt <= 0.0 {
            return;
        }
        let mut addresses: Vec<_> = self.bots.controllers.keys().copied().collect();
        addresses.sort_unstable();
        let mut discs: Vec<_> = self
            .world
            .structures
            .values()
            .filter(|s| s.state.hp > 0.0)
            .map(|s| shared::navigation::Disc {
                center: [s.state.x, s.state.z],
                radius: structure_collision_radius(s.state.kind),
            })
            .collect();
        discs.extend(crate::skills::advanced::terrain(self.world, now));
        for addr in addresses {
            if !matches!(self.world.game_state, GameState::Running) {
                break;
            }
            let Some(mut controller) = self.bots.controllers.remove(&addr) else {
                continue;
            };
            let Some(player) = self.world.players.get(&addr) else {
                continue;
            };
            if player
                .hero
                .skills
                .control
                .stun_until
                .is_some_and(|until| until > now)
            {
                self.bots.controllers.insert(addr, controller);
                continue;
            }
            if let BotKind::MovingDummy {
                anchor,
                started,
                respawning,
            } = controller.kind
            {
                if player.hero.hp > 0.0 {
                    if respawning {
                        let dummy = self.world.players.get_mut(&addr).unwrap();
                        place_dummy(dummy, anchor, anchor, now);
                        controller.kind = BotKind::MovingDummy {
                            anchor,
                            started: now,
                            respawning: false,
                        };
                        self.bots.controllers.insert(addr, controller);
                        continue;
                    }
                    let phase = now.saturating_duration_since(started).as_secs_f32() * 0.8;
                    let goal = [
                        anchor[0] + 2.5 * (phase.cos() - 1.0),
                        anchor[1] + 2.5 * phase.sin(),
                    ];
                    let from = [player.hero.x, player.hero.z];
                    let distance = (goal[0] - from[0]).hypot(goal[1] - from[1]);
                    let control = player.hero.skills.movement(now);
                    let step = (3.0 * dt * control).min(distance);
                    if distance > 0.001 && step > 0.0 {
                        let desired = [
                            from[0] + (goal[0] - from[0]) / distance * step,
                            from[1] + (goal[1] - from[1]) / distance * step,
                        ];
                        let clipped =
                            shared::navigation::world_navigation().clip_movement(from, desired);
                        let clipped = shared::navigation::clip_discs(from, clipped, &discs);
                        let dummy = self.world.players.get_mut(&addr).unwrap();
                        dummy.hero.x = clipped[0];
                        dummy.hero.z = clipped[1];
                        dummy.hero.yaw = hero_yaw_towards(goal[0] - from[0], goal[1] - from[1]);
                        dummy.timers.last_movement_at = now;
                    }
                } else {
                    // Respawn continues from the practice anchor, without lane AI.
                    controller.kind = BotKind::MovingDummy {
                        anchor,
                        started: now,
                        respawning: true,
                    };
                }
                self.bots.controllers.insert(addr, controller);
                continue;
            }
            if let BotKind::Dummy { anchor } = controller.kind {
                // Never thinks, moves or attacks. After a respawn at base it
                // is put back on its anchor so target practice continues.
                if player.hero.hp > 0.0
                    && (player.hero.x - anchor[0]).hypot(player.hero.z - anchor[1]) > 0.5
                {
                    let toward = self
                        .world
                        .players
                        .values()
                        .find(|p| p.joined && !p.hero.identity.is_bot)
                        .map_or(anchor, |p| [p.hero.x, p.hero.z]);
                    let dummy = self.world.players.get_mut(&addr).unwrap();
                    place_dummy(dummy, anchor, toward, now);
                }
                self.bots.controllers.insert(addr, controller);
                continue;
            }
            if player.hero.hp <= 0.0 {
                controller.route.clear();
                controller.goal = None;
                controller.waypoint = 1;
                controller.target = None;
                controller.holding_range = false;
                self.bots.controllers.insert(addr, controller);
                continue;
            }
            let team = player.hero.identity.team;
            let origin = [player.hero.x, player.hero.z];
            let danger: Vec<_> = self
                .world
                .structures
                .values()
                .filter(|tower| !tower_has_minion_cover(tower, team, &self.world.minions))
                .filter(|tower| {
                    tower.state.team != team
                        && tower.state.hp > 0.0
                        && tower.attack_range > 0.0
                        && tower.attack_damage > 0.0
                })
                .map(|tower| shared::navigation::Disc {
                    center: [tower.state.x, tower.state.z],
                    radius: tower.attack_range + shared::PLAYER_TARGET_RADIUS + 0.75,
                })
                .collect();
            let turret_retreat = danger
                .iter()
                .filter(|disc| {
                    (origin[0] - disc.center[0]).hypot(origin[1] - disc.center[1]) < disc.radius
                })
                .min_by(|a, b| {
                    (origin[0] - a.center[0])
                        .hypot(origin[1] - a.center[1])
                        .total_cmp(&(origin[0] - b.center[0]).hypot(origin[1] - b.center[1]))
                });
            if turret_retreat.is_some() {
                // This check is per movement tick, not the slower target cadence.
                controller.target = None;
                controller.holding_range = false;
                controller.route.clear();
                controller.goal = None;
                controller.next_think = now;
            }
            let mut safe_discs = discs.clone();
            safe_discs.extend(danger.iter().copied());
            let low_health = player.hero.hp < player.hero.max_hp * 0.28;
            if low_health && now >= controller.next_retreat {
                controller.retreat_until = Some(now + Duration::from_secs(6));
                controller.next_retreat = now + Duration::from_secs(18);
                controller.route.clear();
                controller.next_route = now;
            }
            let retreating = turret_retreat.is_some()
                || controller.retreat_until.is_some_and(|until| now < until);
            if now >= controller.next_think {
                controller.next_think = now + THINK_INTERVAL;
                {
                    let match_id = self.match_id;
                    let bot = self.world.players.get_mut(&addr).unwrap();
                    auto_rank_skills(bot);
                    // Standing in the base shop (spawn, respawn or a retreat)
                    // spends earned gold the way a player would.
                    auto_shop(
                        bot,
                        &self.world.map_layout,
                        &self.world.game_state,
                        match_id,
                    );
                }
                let previous_target = controller.target;
                controller.target = if retreating {
                    None
                } else {
                    // Retain a living unit instead of swapping between nearly
                    // equidistant opponents every think tick. Nearby units can
                    // still interrupt a tower push.
                    controller
                        .target
                        .filter(|target| {
                            target.kind != TargetKind::Structure
                                && vision::target_visible(team, *target, self.world, now)
                                && basic_attack::resolve_hostile_target(
                                    team,
                                    *target,
                                    &self.world.players,
                                    &self.world.minions,
                                    &self.world.structures,
                                    &self.world.neutrals,
                                )
                                .is_some_and(|(p, _)| {
                                    (p.x - origin[0]).hypot(p.z - origin[1]) <= VISION + 2.0
                                })
                        })
                        .or_else(|| {
                            if controller.jungle {
                                self.jungle_target(team, origin, controller.lane, now)
                            } else {
                                self.bot_target(team, origin, controller.lane, now)
                            }
                        })
                };
                controller.target = controller.target.filter(|target| {
                    basic_attack::resolve_hostile_target(
                        team,
                        *target,
                        &self.world.players,
                        &self.world.minions,
                        &self.world.structures,
                        &self.world.neutrals,
                    )
                    .is_some_and(|(point, _)| {
                        let goal = [point.x, point.z];
                        shared::navigation::clip_discs(origin, goal, &danger) == goal
                    })
                });
                if controller.target != previous_target {
                    controller.holding_range = false;
                    controller.route.clear();
                    controller.next_route = now;
                }
                if let Some(target) = controller.target {
                    if self.world.players[&addr].hero.skills.loadout.is_some() {
                        if let Some((position, _)) = basic_attack::resolve_hostile_target(
                            team,
                            target,
                            &self.world.players,
                            &self.world.minions,
                            &self.world.structures,
                            &self.world.neutrals,
                        ) {
                            let mut skill_order = [0, 1, 2, 3];
                            skill_order.sort_by_key(|slot| {
                                use shared::loadout::{SkillEffect, Technique};
                                match self.world.players[&addr]
                                    .hero
                                    .skills
                                    .loadout
                                    .unwrap()
                                    .skill(SkillSlot::from_index(*slot).unwrap())
                                    .effect
                                {
                                    SkillEffect::Technique {
                                        action: Technique::DaggerBluff,
                                        ..
                                    } => 0,
                                    SkillEffect::Technique {
                                        action: Technique::DaggerBackstab,
                                        ..
                                    } => 1,
                                    _ => 2,
                                }
                            });
                            for slot in skill_order {
                                let p = &self.world.players[&addr];
                                let definition = p
                                    .hero
                                    .skills
                                    .loadout
                                    .as_ref()
                                    .unwrap()
                                    .skill(SkillSlot::from_index(slot).unwrap());
                                // Select a weapon once for this encounter; avoid repeatedly toggling.
                                if matches!(
                                    definition.effect,
                                    shared::loadout::SkillEffect::WeaponToggle { .. }
                                ) && p.hero.skills.mode == shared::loadout::WeaponMode::Rockets
                                {
                                    continue;
                                }
                                let mut aim = [position.x, position.z];
                                if matches!(
                                    definition.effect,
                                    shared::loadout::SkillEffect::Technique {
                                        action: shared::loadout::Technique::AllyLeap
                                            | shared::loadout::Technique::BallGuard
                                            | shared::loadout::Technique::GuardLeap,
                                        ..
                                    }
                                ) {
                                    aim = [p.hero.x, p.hero.z];
                                } else if definition.ability.targeting == TargetingMode::Point {
                                    let dx = aim[0] - p.hero.x;
                                    let dz = aim[1] - p.hero.z;
                                    let dist = dx.hypot(dz);
                                    if dist > definition.ability.cast_range {
                                        let f = definition.ability.cast_range / dist;
                                        aim = [p.hero.x + dx * f, p.hero.z + dz * f];
                                    }
                                }
                                let recast = p.hero.skills.advanced.recasts[slot as usize]
                                    .as_ref()
                                    .is_some_and(|r| r.until > now && r.uses > 0);
                                if !bot_mobility_safe(
                                    definition,
                                    [p.hero.x, p.hero.z],
                                    recast,
                                    &danger,
                                ) {
                                    continue;
                                }
                                let request = p.hero.skills.request_id.saturating_add(1);
                                crate::skills::cast(self.world, addr, slot, aim, request, now);
                            }
                        }
                    }
                    // Every unlocked hostile-target skill; the cast path
                    // enforces range, mana and cooldown for each slot.
                    for slot in 0..4 {
                        let p = &self.world.players[&addr];
                        if !unlocked_slots_for_level(p.hero.progress.level)[slot as usize]
                            || ability_for_class_slot(
                                p.hero.identity.hero_class,
                                SkillSlot::from_index(slot).unwrap(),
                            )
                            .targeting
                                != TargetingMode::UnitTarget
                        {
                            continue;
                        }
                        handle_cast_request(self.world, addr, target, slot, now);
                    }
                }
                if low_health {
                    for slot in 0..4 {
                        let p = &self.world.players[&addr];
                        if ability_for_class_slot(
                            p.hero.identity.hero_class,
                            SkillSlot::from_index(slot).unwrap(),
                        )
                        .targeting
                            == TargetingMode::SelfTarget
                        {
                            let target = TargetId {
                                kind: TargetKind::Player,
                                id: p.hero.identity.id,
                            };
                            handle_cast_request(self.world, addr, target, slot, now);
                        }
                    }
                }
            }
            // Mobility casts may displace the actor during this think tick.
            // Replan from the authoritative endpoint instead of steering along
            // a route and checking attack reach from the pre-cast position.
            let current = &self.world.players[&addr].hero;
            let current_origin = [current.x, current.z];
            if current_origin != origin {
                controller.route.clear();
                controller.goal = None;
                controller.holding_range = false;
                controller.next_route = now;
            }
            let origin = current_origin;
            let target = controller
                .target
                .filter(|target| vision::target_visible(team, *target, self.world, now))
                .and_then(|target| {
                    basic_attack::resolve_hostile_target(
                        team,
                        target,
                        &self.world.players,
                        &self.world.minions,
                        &self.world.structures,
                        &self.world.neutrals,
                    )
                    .map(|(position, radius)| (target, position, radius))
                });
            let reach = crate::skills::attack_modifiers(&self.world.players[&addr]).0;
            let mut in_range = false;
            let destination = if let Some(disc) = turret_retreat {
                let spawn = spawn_position_for_team(&self.world.map_layout, team);
                let mut away = [origin[0] - disc.center[0], origin[1] - disc.center[1]];
                let mut length = away[0].hypot(away[1]);
                if length < 0.001 {
                    away = [spawn.x - disc.center[0], spawn.z - disc.center[1]];
                    length = away[0].hypot(away[1]).max(0.001);
                }
                [
                    disc.center[0] + away[0] / length * (disc.radius + 1.0),
                    disc.center[1] + away[1] / length * (disc.radius + 1.0),
                ]
            } else if retreating {
                let spawn = spawn_position_for_team(&self.world.map_layout, team);
                [spawn.x, spawn.z]
            } else if let Some((target, position, radius)) = target {
                let dx = position.x - origin[0];
                let dz = position.z - origin[1];
                let distance = dx.hypot(dz);
                // Start following again only after leaving attack reach, and
                // approach slightly inside it before stopping a chase.
                let margin = if controller.holding_range { 0.0 } else { 0.25 };
                in_range = distance <= reach + radius - margin;
                controller.holding_range = in_range;
                if in_range {
                    self.world.players.get_mut(&addr).unwrap().hero.yaw = hero_yaw_towards(dx, dz);
                    let request = self.world.players[&addr]
                        .economy
                        .basic_attack_request_id
                        .saturating_add(1);
                    basic_attack::handle_basic_attack_request(
                        self.world, addr, target, request, now,
                    );
                }
                let approach = (reach + radius - 0.35).max(0.5).min(distance);
                [
                    position.x - dx / distance.max(0.001) * approach,
                    position.z - dz / distance.max(0.001) * approach,
                ]
            } else if let Some((_, camp)) = controller
                .jungle
                .then(|| self.jungle_camp(team, origin))
                .flatten()
            {
                camp
            } else {
                let path = build_minion_path(&self.world.map_layout, controller.lane, team);
                while controller.waypoint + 1 < path.len()
                    && (path[controller.waypoint].x - origin[0])
                        .hypot(path[controller.waypoint].z - origin[1])
                        < 1.5
                {
                    controller.waypoint += 1;
                }
                let point = path[controller.waypoint.min(path.len() - 1)];
                lane_staging_destination(self.world, team, controller.lane, [point.x, point.z])
            };
            let mut desired = origin;
            if turret_retreat.is_some() {
                desired = destination;
            } else if !in_range {
                let goal_changed = controller
                    .goal
                    .is_none_or(|p| (p[0] - destination[0]).hypot(p[1] - destination[1]) > 2.0);
                if controller.route.is_empty() || (now >= controller.next_route && goal_changed) {
                    controller.next_route = now + ROUTE_INTERVAL;
                    controller.goal = Some(destination);
                    controller.route = shared::navigation::world_navigation()
                        .plan_route(origin, destination, &safe_discs)
                        .unwrap_or_default()
                        .into();
                }
                while controller
                    .route
                    .front()
                    .is_some_and(|p| (p[0] - origin[0]).hypot(p[1] - origin[1]) < 0.2)
                {
                    controller.route.pop_front();
                }
                if let Some(point) = controller.route.front().copied() {
                    desired = point;
                }
            } else {
                controller.route.clear();
                controller.goal = None;
            }
            let id = self.world.players[&addr].hero.identity.id;
            let mut others: Vec<_> = self
                .world
                .players
                .values()
                .filter(|p| p.joined && p.hero.hp > 0.0 && p.hero.identity.id != id)
                .map(|p| (p.hero.identity.id, [p.hero.x, p.hero.z]))
                .collect();
            others.sort_unstable_by_key(|(id, _)| *id);
            let step = hero_stats::move_speed(&self.world.players[&addr])
                * self.world.players[&addr].hero.skills.movement(now)
                * dt;
            let accepted = steer_bot_step(id, origin, desired, step, &others, &discs);
            let accepted = shared::navigation::clip_discs(origin, accepted, &danger);
            let movement = [accepted[0] - origin[0], accepted[1] - origin[1]];
            if movement[0].hypot(movement[1]) > 0.000_1 {
                let player = self.world.players.get_mut(&addr).unwrap();
                let yaw = if in_range {
                    player.hero.yaw
                } else {
                    hero_yaw_towards(movement[0], movement[1])
                };
                handle_transform_request_with_structures(
                    player,
                    &self.world.map_layout,
                    &self.world.structures,
                    accepted[0],
                    PLAYER_GROUND_Y,
                    accepted[1],
                    yaw,
                    now,
                );
            }
            self.bots.controllers.insert(addr, controller);
        }
    }

    /// Nearest living ordinary camp, preferring the bot's own half of the map.
    /// Camp spots are public map knowledge, like a human jungler's timers.
    fn jungle_camp(&self, team: Team, origin: [f32; 2]) -> Option<(u64, [f32; 2])> {
        let (own, enemy) = match team {
            Team::Green => (self.world.map_layout.home, self.world.map_layout.away),
            Team::Blue => (self.world.map_layout.away, self.world.map_layout.home),
        };
        self.world
            .neutrals
            .values()
            .filter(|n| !n.state.camp_type.is_boss() && n.dead_until.is_none() && n.state.hp > 0.0)
            .map(|n| {
                let at = [n.anchor.x, n.anchor.z];
                let enemy_half =
                    (at[0] - own.x).hypot(at[1] - own.z) > (at[0] - enemy.x).hypot(at[1] - enemy.z);
                let distance = (at[0] - origin[0]).hypot(at[1] - origin[1]);
                (enemy_half, distance, n.state.id, at)
            })
            .min_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.total_cmp(&b.1))
                    .then_with(|| a.2.cmp(&b.2))
            })
            .map(|(_, _, id, at)| (id, at))
    }

    /// Jungler priorities: a visible enemy hero, then the chosen camp once in
    /// sight. With every camp down it farms and pushes its fallback lane.
    fn jungle_target(
        &self,
        team: Team,
        origin: [f32; 2],
        lane: Lane,
        now: Instant,
    ) -> Option<TargetId> {
        let visible = |target| vision::target_visible(team, target, self.world, now);
        let hero = self
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.hp > 0.0 && p.hero.identity.team != team)
            .map(|p| {
                (
                    (p.hero.x - origin[0]).hypot(p.hero.z - origin[1]),
                    TargetId {
                        kind: TargetKind::Player,
                        id: p.hero.identity.id,
                    },
                )
            })
            .filter(|(d, target)| *d <= VISION && visible(*target))
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
        if let Some((_, target)) = hero {
            return Some(target);
        }
        let Some((id, at)) = self.jungle_camp(team, origin) else {
            return self.bot_target(team, origin, lane, now);
        };
        let camp = TargetId {
            kind: TargetKind::Neutral,
            id,
        };
        ((at[0] - origin[0]).hypot(at[1] - origin[1]) <= VISION && visible(camp)).then_some(camp)
    }

    pub fn bot_target(
        &self,
        team: Team,
        origin: [f32; 2],
        lane: Lane,
        now: Instant,
    ) -> Option<TargetId> {
        let distance = |x: f32, z: f32| (x - origin[0]).hypot(z - origin[1]);
        // Fight nearby lane units before diving a structure. Stable ID tie breaks
        // keep behavior independent of HashMap iteration order.
        let mut units: Vec<_> = self
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.hp > 0.0 && p.hero.identity.team != team)
            .map(|p| {
                (
                    distance(p.hero.x, p.hero.z),
                    TargetId {
                        kind: TargetKind::Player,
                        id: p.hero.identity.id,
                    },
                )
            })
            .collect();
        units.extend(
            self.world
                .minions
                .values()
                .filter(|p| p.state.hp > 0.0 && p.state.team != team)
                .map(|p| {
                    (
                        distance(p.state.x, p.state.z),
                        TargetId {
                            kind: TargetKind::Minion,
                            id: p.state.id,
                        },
                    )
                }),
        );
        if let Some((_, target)) = units
            .into_iter()
            .filter(|(d, target)| {
                *d <= VISION && vision::target_visible(team, *target, self.world, now)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)))
        {
            return Some(target);
        }
        self.world
            .structures
            .values()
            .filter(|s| {
                s.state.hp > 0.0
                    && s.state.team != team
                    && tower_has_minion_cover(s, team, &self.world.minions)
                    && !structure_is_protected(&self.world.structures, s.state.id)
                    && match s.role {
                        StructureRole::BaseTower => true,
                        StructureRole::LaneTower { lane: tower_lane } => tower_lane == lane,
                    }
            })
            .filter(|s| {
                distance(s.state.x, s.state.z) <= VISION + s.attack_range
                    && vision::target_visible(
                        team,
                        TargetId {
                            kind: TargetKind::Structure,
                            id: s.state.id,
                        },
                        self.world,
                        now,
                    )
            })
            .min_by(|a, b| {
                distance(a.state.x, a.state.z)
                    .total_cmp(&distance(b.state.x, b.state.z))
                    .then_with(|| a.state.id.cmp(&b.state.id))
            })
            .map(|s| TargetId {
                kind: TargetKind::Structure,
                id: s.state.id,
            })
    }
}

#[cfg(test)]
mod turret_tests {
    use super::*;
    use crate::{
        combat_feedback::CombatLog,
        game_world::GameWorld,
        match_rules::{MatchMode, MatchRules},
    };
    use shared::wire::StructureKind;

    #[test]
    fn dagger_bot_bluffs_before_backstab_and_stunned_bot_preserves_rear_facing() {
        let now = Instant::now();
        let mut world = GameWorld::empty();
        let address: SocketAddr = "[::]:31101".parse().unwrap();
        let enemy: SocketAddr = "[::]:31102".parse().unwrap();
        for (addr, team, class, x) in [
            (address, Team::Green, HeroClass::Adventurer, 0.0),
            (enemy, Team::Blue, HeroClass::Warrior, 2.0),
        ] {
            world.ensure_connected(addr, now);
            crate::session::handle_join_request(
                world.players.get_mut(&addr).unwrap(),
                team,
                shared::wire::CharacterChoice::Ipfs,
                class,
                None,
                &world.map_layout,
                now,
            );
            let p = world.players.get_mut(&addr).unwrap();
            p.hero.x = x;
            p.hero.z = 0.0;
            p.hero.hp = 1000.0;
            p.hero.max_hp = 1000.0;
            p.hero.mana = 500.0;
            p.hero.max_mana = 500.0;
            p.modifiers.unlock_all = true;
            p.hero.progress.skill_points = 0;
        }
        let mut bots = BotControllers::default();
        bots.attach_existing(address, HeroClass::Adventurer, now);
        let mut log = CombatLog::default();
        let mut host = CombatHost {
            world: &mut world,
            bots: &mut bots,
            combat_log: &mut log,
            rules: MatchRules::for_mode(MatchMode::Release, 1),
            match_id: 1,
        };
        host.simulate_bots(now + Duration::from_millis(10), 0.01);
        let p = &host.world.players[&address];
        assert!(p.timers.last_cast_at[1].is_some(), "Bluff must be first");
        assert!(p.timers.last_cast_at[0].is_none());
        assert!(p.timers.last_cast_at[2].is_none());
        let away = host.world.players[&enemy].hero.yaw;
        host.bots.attach_existing(enemy, HeroClass::Warrior, now);
        host.simulate_bots(now + Duration::from_millis(310), 0.01);
        assert!(
            host.world.players[&address].timers.last_cast_at[2].is_some(),
            "follow Bluff with Backstab"
        );
        assert!(host.world.players[&enemy].hero.hp < 1000.0);
        assert_eq!(host.world.players[&enemy].hero.yaw, away);
        assert!(
            host.world.players[&enemy]
                .timers
                .last_basic_attack_at
                .is_none()
        );
    }

    #[test]
    fn retained_advanced_bot_does_not_retreat_dash_into_an_uncovered_turret() {
        let mut now = Instant::now();
        let mut world = GameWorld::empty();
        // Both attack approach and backwards retreat need a real terrain-clear
        // corridor; a blocked dash would not exercise the turret hazard.
        let nav = shared::navigation::world_navigation();
        let origin = (-40..=40)
            .step_by(5)
            .flat_map(|x| (-40..=40).step_by(5).map(move |z| [x as f32, z as f32]))
            .find(|p| nav.segment_clear([p[0] - 4.0, p[1]], [p[0] + 12.0, p[1]]))
            .expect("map must provide a clear corridor for the retreat fixture");
        let tower = [origin[0] + 12.0, origin[1]];
        assert_eq!(
            nav.clip_movement(origin, [origin[0] + 7.0, origin[1]]),
            [origin[0] + 7.0, origin[1]]
        );
        let address: SocketAddr = "[::]:31001".parse().unwrap();
        let enemy: SocketAddr = "127.0.0.1:31002".parse().unwrap();
        for (addr, team, class, x) in [
            (address, Team::Green, HeroClass::Veilstalker, origin[0]),
            (enemy, Team::Blue, HeroClass::Warrior, origin[0] - 4.0),
        ] {
            world.ensure_connected(addr, now);
            crate::session::handle_join_request(
                world.players.get_mut(&addr).unwrap(),
                team,
                shared::wire::CharacterChoice::Ipfs,
                class,
                None,
                &world.map_layout,
                now,
            );
            let p = world.players.get_mut(&addr).unwrap();
            p.hero.x = x;
            p.hero.z = origin[1];
            p.hero.hp = 1000.0;
            p.hero.max_hp = 1000.0;
            p.hero.mana = 500.0;
            p.hero.max_mana = 500.0;
            p.hero.progress.level = 10;
            p.hero.progress.ranks = [3; 4];
            p.hero.progress.skill_points = 0;
        }
        // Isolate the backwards retreat; other skills remain on cooldown.
        world.players.get_mut(&address).unwrap().timers.last_cast_at =
            [Some(now), Some(now), Some(now), None];
        let mut next = 1;
        crate::world::add_structure(
            &mut world.structures,
            &mut next,
            StructureKind::Tower,
            StructureRole::LaneTower { lane: Lane::Mid },
            Team::Blue,
            crate::entities::Vec3f::new(tower[0], 3.0, tower[1]),
        );
        world.structures.get_mut(&1).unwrap().attack_range = 8.0;
        let mut bots = BotControllers::default();
        bots.attach_existing(address, HeroClass::Veilstalker, now);
        let mut log = CombatLog::default();
        let mut host = CombatHost {
            world: &mut world,
            bots: &mut bots,
            combat_log: &mut log,
            rules: MatchRules::for_mode(MatchMode::Release, 1),
            match_id: 1,
        };
        now += Duration::from_millis(50);
        host.simulate_bots(now, 0.05);
        let p = &host.world.players[&address];
        assert!(
            p.timers.last_cast_at[3].is_none(),
            "unsafe displacement must be rejected before cast side effects"
        );
        assert!((p.hero.x - tower[0]).hypot(p.hero.z - tower[1]) > 8.0);
        assert!(!p.hero.identity.is_bot);
        crate::world::spawn_minion_wave_for_team_lane(
            &host.world.map_layout,
            &mut host.world.minions,
            &mut host.world.next_minion_id,
            Team::Green,
            Lane::Mid,
        );
        for minion in host.world.minions.values_mut() {
            minion.state.x = tower[0];
            minion.state.z = tower[1] + 2.0;
        }
        host.world.players.get_mut(&address).unwrap().hero.x = origin[0];
        host.world.players.get_mut(&address).unwrap().hero.z = origin[1];
        now += Duration::from_millis(400);
        host.simulate_bots(now, 0.05);
        let p = &host.world.players[&address];
        assert!(
            p.timers.last_cast_at[3].is_some(),
            "cover must permit the same mobility skill"
        );
        assert!(
            (p.hero.x - tower[0]).hypot(p.hero.z - tower[1]) < 8.0,
            "fixture must exercise an actual backwards dash toward the turret: origin={origin:?}, tower={tower:?}, actual={:?}",
            [p.hero.x, p.hero.z]
        );
    }

    #[test]
    fn bots_wait_for_actual_turret_cover_and_retreat_on_the_first_unprotected_tick() {
        let mut now = Instant::now();
        let mut world = GameWorld::empty();
        let address: SocketAddr = "[::]:30001".parse().unwrap();
        world.ensure_connected(address, now);
        let actor = world.players.get_mut(&address).unwrap();
        actor.joined = true;
        actor.hero.identity.team = Team::Green;
        actor.hero.identity.hero_class = HeroClass::Warrior;
        actor.hero.x = -16.0;
        actor.hero.z = -8.0;
        actor.hero.hp = actor.hero.max_hp;
        let mut next = 1;
        crate::world::add_structure(
            &mut world.structures,
            &mut next,
            StructureKind::Tower,
            StructureRole::LaneTower { lane: Lane::Top },
            Team::Blue,
            crate::entities::Vec3f::new(-4.0, 3.0, -8.0),
        );
        world.structures.get_mut(&1).unwrap().attack_range = 8.0;
        let mut controllers = BotControllers::default();
        controllers.attach_existing(address, HeroClass::Warrior, now);
        controllers.attach_existing(address, HeroClass::Warrior, now);
        assert_eq!(controllers.controllers.len(), 1);
        let mut log = CombatLog::default();
        let mut host = CombatHost {
            world: &mut world,
            bots: &mut controllers,
            combat_log: &mut log,
            rules: MatchRules::for_mode(MatchMode::Release, 1),
            match_id: 1,
        };
        for _ in 0..100 {
            now += Duration::from_millis(50);
            host.simulate_bots(now, 0.05);
            let hero = &host.world.players[&address].hero;
            assert!(
                (hero.x + 4.0).hypot(hero.z + 8.0) > 8.0,
                "uncovered turret cannot be entered"
            );
        }
        crate::world::spawn_minion_wave_for_team_lane(
            &host.world.map_layout,
            &mut host.world.minions,
            &mut host.world.next_minion_id,
            Team::Green,
            Lane::Top,
        );
        for minion in host.world.minions.values_mut() {
            minion.state.x = -4.0;
            minion.state.z = -6.0;
        }
        assert!(tower_has_minion_cover(
            &host.world.structures[&1],
            Team::Green,
            &host.world.minions
        ));
        // Reset only placement, keeping the same real controller and movement authority.
        let hero = host.world.players.get_mut(&address).unwrap();
        hero.hero.x = -16.0;
        hero.hero.z = -8.0;
        for _ in 0..160 {
            now += Duration::from_millis(50);
            host.simulate_bots(now, 0.05);
        }
        let hero = &host.world.players[&address].hero;
        let before = (hero.x + 4.0).hypot(hero.z + 8.0);
        assert!(
            before < 8.0,
            "living allied minions permit a push: {before}"
        );
        for minion in host.world.minions.values_mut() {
            minion.state.hp = 0.0;
        }
        assert!(!tower_has_minion_cover(
            &host.world.structures[&1],
            Team::Green,
            &host.world.minions
        ));
        let shots = host.world.next_projectile_id;
        now += Duration::from_millis(50);
        host.simulate_bots(now, 0.05);
        let hero = &host.world.players[&address].hero;
        assert!(
            (hero.x + 4.0).hypot(hero.z + 8.0) > before,
            "retreat begins immediately"
        );
        assert_eq!(
            host.world.next_projectile_id, shots,
            "retreat interrupts attack intent"
        );
        for _ in 0..100 {
            now += Duration::from_millis(50);
            host.simulate_bots(now, 0.05);
        }
        let hero = &host.world.players[&address].hero;
        assert!((hero.x + 4.0).hypot(hero.z + 8.0) > 8.0);
        assert!(!hero.identity.is_bot, "takeover retains human identity");
        host.bots.detach(address);
        assert!(host.bots.controllers.is_empty());
    }
}
