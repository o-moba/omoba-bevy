//! Practice-only server controllers. Actors use ordinary hero movement, attacks,
//! resources and receipts; internal addresses are never network endpoints.
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::map::{Lane, Team};
use shared::progression::equipped_skill_upgrade_order;
use shared::shop::plan_purchases;
use shared::wire::{GameState, TargetId, TargetKind, default_character_choice};
use shared::{
    HeroClass, SkillSlot, TargetingMode, ability_for_class_slot, unlocked_slots_for_level,
};

use crate::balance::PLAYER_GROUND_Y;
use crate::entities::{ConnectedPlayer, MapLayoutState, StructureRole};
use crate::host::CombatHost;
use crate::session::{handle_join_request_with_sprite, handle_transform_request_with_structures};
use crate::sim::cast::{apply_skill_upgrade, handle_cast_request, legacy_effect_scale};
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

/// A displacement between two thinks faster than any walk (level growth, items
/// and haste stay well below it) is a dash, blink or respawn, not a course to lead.
const MAX_LEAD_SPEED: f32 = shared::hero_balance::PLAYER_SPEED * 2.5;

/// A visible enemy hero as a bot sees it on one think.
#[derive(Clone, Copy, Debug)]
struct Sighting {
    id: u64,
    position: [f32; 2],
    /// From the previous think's sighting; `None` when unseen, stale or a jump.
    velocity: Option<[f32; 2]>,
    /// Time it cannot walk anyway: root, stun or its own cast lock.
    held_secs: f32,
    /// Held or slowed: the window a slow cast waits for.
    hampered: bool,
}

/// Effects dispatched by `skills::cast` hit through `skills::hostile`, which
/// never includes structures: aimed at a tower, such a damage skill is wasted.
/// Techniques have their own executors (a seal does land on structures).
fn damages_units_only(effect: shared::loadout::SkillEffect) -> bool {
    effect.damage().is_some() && !matches!(effect, shared::loadout::SkillEffect::Technique { .. })
}

/// Whether to recast a live field now. Any recast detonates it, so only an
/// enemy hero inside it that would walk out before the next think is worth it;
/// otherwise the field keeps slowing and detonates by itself when it expires.
fn field_detonation_due(centre: [f32; 2], radius: f32, heroes: &[Sighting]) -> bool {
    let reach = radius + crate::balance::PLAYER_HIT_RADIUS;
    // Thinks run on the movement tick after `THINK_INTERVAL`, so allow slack.
    let horizon = THINK_INTERVAL.as_secs_f32() * 1.5;
    let inside = |p: [f32; 2]| (p[0] - centre[0]).hypot(p[1] - centre[1]) <= reach;
    heroes.iter().any(|hero| {
        let v = hero.velocity.unwrap_or([0.0; 2]);
        inside(hero.position)
            && !inside([
                hero.position[0] + v[0] * horizon,
                hero.position[1] + v[1] * horizon,
            ])
    })
}

/// A warned beam's aim, and whether its victim is hampered.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BeamShot {
    aim: [f32; 2],
    hampered: bool,
}

/// Where to aim a warned beam from `origin`, or `None` to keep it. Each hero is
/// led along its observed velocity for the part of the windup it can walk, and
/// counts only while that point stays inside the fired capsule; a hero without
/// a velocity estimate counts only when held for the whole windup. A hampered
/// hero comes first, then the bot's own target, then the nearest. Minions alone
/// never justify the beam.
fn beam_shot(
    origin: [f32; 2],
    length: f32,
    width: f32,
    windup_secs: f32,
    preferred: Option<u64>,
    heroes: &[Sighting],
) -> Option<BeamShot> {
    let reach = length + width + crate::balance::PLAYER_HIT_RADIUS;
    heroes
        .iter()
        .filter_map(|hero| {
            let lead = (windup_secs - hero.held_secs).max(0.0);
            let v = match hero.velocity {
                Some(v) => v,
                None if lead == 0.0 => [0.0; 2],
                None => return None,
            };
            let aim = [
                hero.position[0] + v[0] * lead,
                hero.position[1] + v[1] * lead,
            ];
            let distance = (aim[0] - origin[0]).hypot(aim[1] - origin[1]);
            (distance <= reach).then_some((hero, aim, distance))
        })
        .min_by(|a, b| {
            (!a.0.hampered, preferred != Some(a.0.id))
                .cmp(&(!b.0.hampered, preferred != Some(b.0.id)))
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.0.id.cmp(&b.0.id))
        })
        .map(|(hero, aim, _)| BeamShot {
            aim,
            hampered: hero.hampered,
        })
}

/// The shot of the beam bound to `slot`, if that slot holds a beam with a victim.
fn slot_beam_shot(
    p: &ConnectedPlayer,
    slot: u8,
    preferred: Option<u64>,
    heroes: &[Sighting],
) -> Option<BeamShot> {
    let definition = p.hero.skills.loadout?.skill(SkillSlot::from_index(slot)?);
    let shared::loadout::SkillEffect::Beam { width, .. } = definition.effect else {
        return None;
    };
    let rank = p.hero.progress.ranks[usize::from(slot)].clamp(1, definition.ability.max_rank);
    beam_shot(
        [p.hero.x, p.hero.z],
        shared::scaled_cast_range(&definition.ability, rank),
        width,
        definition.windup_secs,
        preferred,
        heroes,
    )
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
            seen: Vec::new(),
            seen_at: None,
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
    /// Disposable sandbox target: stands still and never attacks.
    Dummy { anchor: [f32; 2] },
    /// Disposable passive target following a bounded circular route.
    MovingDummy { anchor: [f32; 2], started: Instant },
    /// Disposable opponent that immediately pursues the requesting human.
    AggressiveDummy { target: u64 },
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
    /// Enemy hero positions at `seen_at`, the last think that looked.
    seen: Vec<(u64, [f32; 2])>,
    seen_at: Option<Instant>,
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

/// Legacy self-target slots worth pressing now, in press order. Every accepted
/// cast starts the shared recovery, so the order decides which one lands first.
/// A heal is the emergency button: only at low health, the largest first. A
/// mana restore follows whenever the bar can hold all of it, whatever the
/// health, and never at a full bar.
fn self_sustain_order(player: &ConnectedPlayer, low_health: bool) -> Vec<u8> {
    let hero = &player.hero;
    let mut heals = Vec::new();
    let mut restores = Vec::new();
    for slot in SkillSlot::ALL {
        let def = ability_for_class_slot(hero.identity.hero_class, slot);
        if def.targeting != TargetingMode::SelfTarget {
            continue;
        }
        let rank = hero.progress.ranks[slot.index()].clamp(1, def.max_rank);
        let scale = legacy_effect_scale(player, rank);
        if let Some(heal) = def.self_heal.filter(|_| low_health) {
            heals.push((slot.index() as u8, heal * scale));
        } else if def
            .self_mana_restore
            .is_some_and(|restore| hero.max_mana - hero.mana >= restore * scale)
        {
            restores.push(slot.index() as u8);
        }
    }
    heals.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    heals
        .into_iter()
        .map(|(slot, _)| slot)
        .chain(restores)
        .collect()
}

/// Spend every skill point the way a player would
/// (`shared::progression::skill_upgrade_order`: the ultimate first once it
/// unlocks, then Q, W and E, never into a locked slot), through the ordinary
/// upgrade path.
pub fn auto_rank_skills(player: &mut ConnectedPlayer) {
    let progress = &player.hero.progress;
    let order = equipped_skill_upgrade_order(
        hero_stats::equipped_skills(player),
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
/// (`debug::practice`). Disposable practice targets do not respawn.
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
        self.world.separate_spawn(addr);
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
                seen: Vec::new(),
                seen_at: None,
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
            // Remove disposable targets before stun/respawn handling. Their
            // absence must never make the ordinary roster refill a replacement.
            if player.hero.hp <= 0.0
                && matches!(
                    controller.kind,
                    BotKind::Dummy { .. }
                        | BotKind::MovingDummy { .. }
                        | BotKind::AggressiveDummy { .. }
                )
            {
                remove_bot(
                    &mut self.world.players,
                    self.bots,
                    &mut self.combat_log.ledger,
                    addr,
                );
                continue;
            }
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
            if let BotKind::MovingDummy { anchor, started } = controller.kind {
                let phase = now.saturating_duration_since(started).as_secs_f32() * 0.8;
                let goal = [
                    anchor[0] + 2.5 * (phase.cos() - 1.0),
                    anchor[1] + 2.5 * phase.sin(),
                ];
                let from = [player.hero.x, player.hero.z];
                let distance = (goal[0] - from[0]).hypot(goal[1] - from[1]);
                let step = (3.0 * dt * player.hero.skills.movement(now)).min(distance);
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
                self.bots.controllers.insert(addr, controller);
                continue;
            }
            if matches!(controller.kind, BotKind::Dummy { .. }) {
                self.bots.controllers.insert(addr, controller);
                continue;
            }
            if let BotKind::AggressiveDummy { target } = controller.kind {
                let origin = [player.hero.x, player.hero.z];
                let target = TargetId {
                    kind: TargetKind::Player,
                    id: target,
                };
                if let Some((position, radius)) = basic_attack::resolve_hostile_target(
                    player.hero.identity.team,
                    target,
                    &self.world.players,
                    &self.world.minions,
                    &self.world.structures,
                    &self.world.neutrals,
                ) {
                    let dx = position.x - origin[0];
                    let dz = position.z - origin[1];
                    let distance = dx.hypot(dz);
                    let reach = crate::skills::attack_modifiers(player).0 + radius;
                    if distance <= reach {
                        let request = player.economy.basic_attack_request_id.saturating_add(1);
                        self.world.players.get_mut(&addr).unwrap().hero.yaw =
                            hero_yaw_towards(dx, dz);
                        basic_attack::handle_basic_attack_request(
                            self.world, addr, target, request, now,
                        );
                    } else {
                        let step = (hero_stats::move_speed(player)
                            * player.hero.skills.movement(now)
                            * dt)
                            .min(distance - reach + 0.1);
                        let desired = [
                            origin[0] + dx / distance * step,
                            origin[1] + dz / distance * step,
                        ];
                        let next =
                            shared::navigation::world_navigation().clip_movement(origin, desired);
                        let next = shared::navigation::clip_discs(origin, next, &discs);
                        handle_transform_request_with_structures(
                            self.world.players.get_mut(&addr).unwrap(),
                            &self.world.map_layout,
                            &self.world.structures,
                            next[0],
                            PLAYER_GROUND_Y,
                            next[1],
                            hero_yaw_towards(dx, dz),
                            now,
                        );
                    }
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
                // Every think looks, so a slow cast can lead the hero it picks.
                let sightings = if self.world.players[&addr].hero.skills.loadout.is_some() {
                    self.sight_enemy_heroes(team, &mut controller, now)
                } else {
                    Vec::new()
                };
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
                            use shared::loadout::SkillEffect;
                            let preferred =
                                (target.kind == TargetKind::Player).then_some(target.id);
                            let mut skill_order = [0, 1, 2, 3];
                            skill_order.sort_by_key(|slot| {
                                use shared::loadout::Technique;
                                let p = &self.world.players[&addr];
                                // A hampered victim, e.g. right after this bot's own
                                // root landed, is the beam's window: take it before the
                                // shared recovery goes to another slot.
                                if slot_beam_shot(p, *slot, preferred, &sightings)
                                    .is_some_and(|shot| shot.hampered)
                                {
                                    return 0;
                                }
                                match p
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
                                    } => 1,
                                    SkillEffect::Technique {
                                        action: Technique::DaggerBackstab,
                                        ..
                                    } => 2,
                                    _ => 3,
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
                                if matches!(definition.effect, SkillEffect::WeaponToggle { .. })
                                    && p.hero.skills.mode == shared::loadout::WeaponMode::Rockets
                                {
                                    continue;
                                }
                                // Any recast detonates the live field instead of paying
                                // for a new one.
                                if let SkillEffect::RecastZone { radius, .. } = definition.effect
                                    && let Some(centre) =
                                        crate::skills::live_zone(self.world, p, slot, now)
                                {
                                    if field_detonation_due(centre, radius, &sightings) {
                                        let request = p.hero.skills.request_id.saturating_add(1);
                                        crate::skills::cast(
                                            self.world, addr, slot, centre, request, now,
                                        );
                                    }
                                    continue;
                                }
                                let mut aim = [position.x, position.z];
                                if matches!(definition.effect, SkillEffect::Beam { .. }) {
                                    // The beam picks its own victim, from where the
                                    // bot stands now; without one it waits.
                                    let Some(shot) = slot_beam_shot(p, slot, preferred, &sightings)
                                    else {
                                        continue;
                                    };
                                    aim = shot.aim;
                                } else if target.kind == TargetKind::Structure
                                    && damages_units_only(definition.effect)
                                {
                                    continue;
                                } else if matches!(
                                    definition.effect,
                                    SkillEffect::Technique {
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
                // The cast path still enforces unlock, mana, cooldown and the
                // shared recovery, so a refused attempt falls through to the
                // next slot of the order.
                let bot = &self.world.players[&addr];
                let target = TargetId {
                    kind: TargetKind::Player,
                    id: bot.hero.identity.id,
                };
                for slot in self_sustain_order(bot, low_health) {
                    handle_cast_request(self.world, addr, target, slot, now);
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
                // Route to the target and stop at attack reach on the way.
                // A radial approach point can fall inside an intervening tree,
                // whose projected boundary may still be outside attack reach.
                [position.x, position.z]
            } else if let Some((_, camp)) = controller
                .jungle
                .then(|| self.jungle_camp(team, origin))
                .flatten()
            {
                camp
            } else {
                let path = build_minion_path(&self.world.map_layout, controller.lane, team);
                while controller.waypoint + 1 < path.len() {
                    let waypoint = [path[controller.waypoint].x, path[controller.waypoint].z];
                    let near = (waypoint[0] - origin[0]).hypot(waypoint[1] - origin[1]) < 1.5;
                    // A live tower can occupy an authored lane corner. Reaching
                    // its planned boundary is arrival, even when the original
                    // point lies farther away than the ordinary corner tolerance.
                    // A turret staging destination is a different goal and must
                    // never advance the lane behind an unsupported defense.
                    let projected_arrival = controller.goal == Some(waypoint)
                        && controller.route.back().is_some_and(|end| {
                            (end[0] - origin[0]).hypot(end[1] - origin[1]) < 0.2
                        });
                    if !near && !projected_arrival {
                        break;
                    }
                    controller.waypoint += 1;
                    controller.route.clear();
                    controller.goal = None;
                    controller.next_route = now;
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

    /// Visible living enemy heroes. Velocities come from the controller's previous
    /// sightings (only if that think was recent), which this one then replaces.
    fn sight_enemy_heroes(
        &self,
        team: Team,
        controller: &mut Controller,
        now: Instant,
    ) -> Vec<Sighting> {
        let elapsed = controller
            .seen_at
            .map(|at| now.saturating_duration_since(at).as_secs_f32())
            .filter(|secs| *secs > 0.0 && *secs <= THINK_INTERVAL.as_secs_f32() * 2.0);
        let remaining = |until: Option<Instant>| {
            until.map_or(0.0, |until| {
                until.saturating_duration_since(now).as_secs_f32()
            })
        };
        let mut heroes: Vec<_> = self
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.hp > 0.0 && p.hero.identity.team != team)
            .filter(|p| {
                let target = TargetId {
                    kind: TargetKind::Player,
                    id: p.hero.identity.id,
                };
                vision::target_visible(team, target, self.world, now)
            })
            .map(|p| {
                let id = p.hero.identity.id;
                let position = [p.hero.x, p.hero.z];
                let velocity = elapsed
                    .zip(controller.seen.iter().find(|(seen, _)| *seen == id))
                    .map(|(secs, (_, at))| {
                        [(position[0] - at[0]) / secs, (position[1] - at[1]) / secs]
                    })
                    .filter(|v| v[0].hypot(v[1]) <= MAX_LEAD_SPEED);
                let skills = &p.hero.skills;
                let held_secs = remaining(skills.control.root_until)
                    .max(remaining(skills.control.stun_until))
                    .max(remaining(skills.cast_lock_until));
                Sighting {
                    id,
                    position,
                    velocity,
                    held_secs,
                    hampered: held_secs > 0.0 || skills.control.movement(now) < 1.0,
                }
            })
            .collect();
        heroes.sort_unstable_by_key(|hero| hero.id);
        controller.seen = heroes.iter().map(|hero| (hero.id, hero.position)).collect();
        controller.seen_at = Some(now);
        heroes
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

#[cfg(test)]
mod sustain_tests {
    use super::*;
    use crate::{
        combat_feedback::CombatLog,
        game_world::GameWorld,
        match_rules::{MatchMode, MatchRules},
    };

    const MAX_HP: f32 = 1000.0;
    const MAX_MANA: f32 = 500.0;

    /// One level-10 legacy bot alone on an empty map: with nothing to fight,
    /// every accepted cast is a self-sustain decision.
    pub(super) fn lone_bot(
        class: HeroClass,
        now: Instant,
    ) -> (GameWorld, BotControllers, SocketAddr) {
        let mut world = GameWorld::empty();
        let address: SocketAddr = "[::]:32001".parse().unwrap();
        world.ensure_connected(address, now);
        crate::session::handle_join_request(
            world.players.get_mut(&address).unwrap(),
            Team::Green,
            shared::wire::CharacterChoice::Ipfs,
            class,
            None,
            &world.map_layout,
            now,
        );
        let p = world.players.get_mut(&address).unwrap();
        // Away from the base shop, so the build and the bars stay as authored here.
        p.hero.x = 0.0;
        p.hero.z = 0.0;
        p.hero.hp = MAX_HP;
        p.hero.max_hp = MAX_HP;
        p.hero.mana = MAX_MANA;
        p.hero.max_mana = MAX_MANA;
        p.hero.progress.level = 10;
        p.hero.progress.ranks = [3; 4];
        p.hero.progress.skill_points = 0;
        let mut bots = BotControllers::default();
        bots.attach_existing(address, class, now);
        (world, bots, address)
    }

    pub(super) fn think(world: &mut GameWorld, bots: &mut BotControllers, now: Instant) {
        let mut log = CombatLog::default();
        CombatHost {
            world,
            bots,
            combat_log: &mut log,
            rules: MatchRules::for_mode(MatchMode::Release, 1),
            match_id: 1,
        }
        .simulate_bots(now, 0.05);
    }

    fn restore_amount(player: &ConnectedPlayer, slot: SkillSlot) -> f32 {
        let def = ability_for_class_slot(player.hero.identity.hero_class, slot);
        def.self_mana_restore.unwrap() * legacy_effect_scale(player, 3)
    }

    #[test]
    fn legacy_bot_restores_mana_on_need_and_heals_only_at_low_health() {
        let now = Instant::now();
        // Mage: W is the mana restore, there is no heal.
        let (mut world, _, address) = lone_bot(HeroClass::Mage, now);
        let p = world.players.get_mut(&address).unwrap();
        assert!(self_sustain_order(p, false).is_empty());
        assert!(self_sustain_order(p, true).is_empty());
        p.hero.mana = MAX_MANA - restore_amount(p, SkillSlot::W);
        assert_eq!(self_sustain_order(p, false), [1]);
        assert_eq!(self_sustain_order(p, true), [1]);

        // Warrior: W is a heal, there is no mana restore.
        let (mut world, _, address) = lone_bot(HeroClass::Warrior, now);
        let p = world.players.get_mut(&address).unwrap();
        p.hero.mana = 0.0;
        assert!(self_sustain_order(p, false).is_empty());
        assert_eq!(self_sustain_order(p, true), [1]);

        // Cleric: W and R heal (R is larger), E restores mana.
        let (mut world, _, address) = lone_bot(HeroClass::Cleric, now);
        let p = world.players.get_mut(&address).unwrap();
        assert!(self_sustain_order(p, false).is_empty());
        assert_eq!(self_sustain_order(p, true), [3, 1]);
        p.hero.mana = MAX_MANA - restore_amount(p, SkillSlot::E);
        assert_eq!(self_sustain_order(p, false), [2]);
        assert_eq!(self_sustain_order(p, true), [3, 1, 2]);
    }

    #[test]
    fn healthy_bot_restores_missing_mana_and_ignores_a_full_bar() {
        let mut now = Instant::now();
        let (mut world, mut bots, address) = lone_bot(HeroClass::Mage, now);
        let restore = restore_amount(&world.players[&address], SkillSlot::W);

        // A full bar, then a gap the restore would overfill: nothing to gain.
        for mana in [MAX_MANA, MAX_MANA - restore + 1.0] {
            world.players.get_mut(&address).unwrap().hero.mana = mana;
            now += Duration::from_millis(300);
            think(&mut world, &mut bots, now);
            let p = &world.players[&address];
            assert_eq!(p.timers.last_cast_at, [None; 4]);
            assert_eq!(p.hero.mana, mana);
        }

        let mana = MAX_MANA - restore - 10.0;
        world.players.get_mut(&address).unwrap().hero.mana = mana;
        now += Duration::from_millis(300);
        think(&mut world, &mut bots, now);
        let p = &world.players[&address];
        assert_eq!(p.timers.last_cast_at, [None, Some(now), None, None]);
        assert!((p.hero.mana - (mana + restore)).abs() < 0.001);
        assert_eq!(p.hero.hp, MAX_HP, "a healthy bot spends no heal");
    }

    #[test]
    fn low_health_bot_does_not_burn_a_mana_restore_at_full_mana() {
        let now = Instant::now();
        let (mut world, mut bots, address) = lone_bot(HeroClass::Mage, now);
        world.players.get_mut(&address).unwrap().hero.hp = MAX_HP * 0.2;
        think(&mut world, &mut bots, now + Duration::from_millis(10));
        let p = &world.players[&address];
        assert_eq!(
            p.timers.last_cast_at, [None; 4],
            "a full mana bar keeps the restore and its cooldown"
        );
        assert_eq!(p.hero.mana, MAX_MANA);
    }

    #[test]
    fn low_health_cleric_bot_casts_the_largest_heal_first() {
        let mut now = Instant::now();
        let (mut world, mut bots, address) = lone_bot(HeroClass::Cleric, now);
        world.players.get_mut(&address).unwrap().hero.hp = MAX_HP * 0.1;
        now += Duration::from_millis(10);
        think(&mut world, &mut bots, now);
        let first = now;
        let p = &world.players[&address];
        assert_eq!(
            p.timers.last_cast_at,
            [None, None, None, Some(first)],
            "the ultimate heal is the emergency button; shared recovery holds the rest"
        );
        let after_ultimate = p.hero.hp;
        assert!(after_ultimate > MAX_HP * 0.1 && after_ultimate < MAX_HP * 0.28);

        // Past the shared recovery the smaller heal follows; the mana restore
        // still waits because the bar cannot hold all of it yet.
        now += Duration::from_millis(500);
        think(&mut world, &mut bots, now);
        let p = &world.players[&address];
        assert_eq!(p.timers.last_cast_at, [None, Some(now), None, Some(first)]);
        assert!(p.hero.hp > after_ultimate);
    }
}

#[cfg(test)]
mod kit_tests {
    use super::sustain_tests::{lone_bot, think};
    use super::*;
    use crate::game_world::{GameWorld, TickCtx};
    use crate::skills::live_zone;
    use shared::combat::CombatEvent;
    use shared::loadout::SkillEffect;
    use shared::wire::StructureKind;

    const FIELD: u8 = 2;
    const RAY: u8 = 3;
    const FRAME: Duration = Duration::from_millis(50);

    fn add_enemy(world: &mut GameWorld, port: u16, at: [f32; 2], now: Instant) -> SocketAddr {
        let address: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        world.ensure_connected(address, now);
        crate::session::handle_join_request(
            world.players.get_mut(&address).unwrap(),
            Team::Blue,
            shared::wire::CharacterChoice::Ipfs,
            HeroClass::Warrior,
            None,
            &world.map_layout,
            now,
        );
        let p = world.players.get_mut(&address).unwrap();
        p.hero.x = at[0];
        p.hero.z = at[1];
        p.hero.hp = 1000.0;
        p.hero.max_hp = 1000.0;
        address
    }

    fn add_minion(world: &mut GameWorld, team: Team, at: [f32; 2]) -> u64 {
        crate::world::spawn_minion_wave_for_team_lane(
            &world.map_layout,
            &mut world.minions,
            &mut world.next_minion_id,
            team,
            Lane::Mid,
        );
        let id = *world.minions.keys().max().unwrap();
        world
            .minions
            .retain(|key, minion| *key == id || minion.state.team != team);
        let minion = world.minions.get_mut(&id).unwrap();
        minion.state.x = at[0];
        minion.state.z = at[1];
        id
    }

    /// Spend `slots` now, so only the remaining ones can be cast.
    fn rest(world: &mut GameWorld, address: SocketAddr, slots: &[u8], now: Instant) {
        for slot in slots {
            world.players.get_mut(&address).unwrap().timers.last_cast_at[usize::from(*slot)] =
                Some(now);
        }
    }

    /// One 50 ms frame: the bot (thinking on its own cadence), then the skill runtime.
    fn frame(world: &mut GameWorld, bots: &mut BotControllers, now: Instant) -> Vec<CombatEvent> {
        think(world, bots, now);
        let events = crate::skills::tick(world, TickCtx { now, dt: 0.05 });
        crate::skills::observe(world, &events, now);
        crate::skills::normalize(world, now);
        events
    }

    /// Walk `address` along `direction` for one frame at its own hero speed.
    fn walk(world: &mut GameWorld, address: SocketAddr, direction: [f32; 2], now: Instant) {
        let p = world.players.get_mut(&address).unwrap();
        let step = hero_stats::move_speed(p) * p.hero.skills.movement(now) * FRAME.as_secs_f32();
        p.hero.x += direction[0] * step;
        p.hero.z += direction[1] * step;
    }

    fn position(world: &GameWorld, address: SocketAddr) -> [f32; 2] {
        [
            world.players[&address].hero.x,
            world.players[&address].hero.z,
        ]
    }

    fn ray_hit(events: &[CombatEvent], caster: u64, victim: u64) -> Option<&CombatEvent> {
        events.iter().find(|e| {
            e.source.id == caster
                && e.target.id == victim
                && e.action_slot == Some(RAY)
                && !e.radiance_burst
        })
    }

    fn field_reach() -> f32 {
        let SkillEffect::RecastZone { radius, .. } =
            shared::loadout::skill(shared::loadout::SkillId::DawnField).effect
        else {
            unreachable!()
        };
        radius + crate::balance::PLAYER_HIT_RADIUS
    }

    #[test]
    fn beam_shot_leads_by_the_walkable_windup_and_ranks_hampered_heroes_first() {
        let walker = Sighting {
            id: 7,
            position: [10.0, 0.0],
            velocity: Some([0.0, 5.0]),
            held_secs: 0.0,
            hampered: false,
        };
        let shot = beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[walker]).unwrap();
        assert_eq!(shot.aim, [10.0, 4.0], "led by the whole windup");
        let rooted = Sighting {
            held_secs: 0.5,
            hampered: true,
            ..walker
        };
        let shot = beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[rooted]).unwrap();
        assert!(
            (shot.aim[1] - 1.5).abs() < 1e-4,
            "led only once the root ends"
        );
        assert!(shot.hampered);

        let unknown = Sighting {
            velocity: None,
            ..walker
        };
        assert_eq!(beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[unknown]), None);
        let held = Sighting {
            held_secs: 1.0,
            ..unknown
        };
        assert_eq!(
            beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[held]).map(|s| s.aim),
            Some([10.0, 0.0]),
            "held for the whole windup, no estimate is needed"
        );
        let far = Sighting {
            position: [46.5, 0.0],
            velocity: Some([0.0; 2]),
            ..walker
        };
        assert_eq!(beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[far]), None);

        let target = Sighting {
            id: 3,
            position: [5.0, 0.0],
            velocity: Some([0.0; 2]),
            ..walker
        };
        let slowed = Sighting {
            id: 9,
            position: [12.0, 0.0],
            velocity: Some([0.0; 2]),
            held_secs: 0.0,
            hampered: true,
        };
        let pick = |heroes: &[Sighting], preferred| {
            beam_shot([0.0; 2], 45.0, 0.8, 0.8, preferred, heroes).map(|s| s.aim)
        };
        assert_eq!(pick(&[target, slowed], Some(3)), Some([12.0, 0.0]));
        assert_eq!(pick(&[target, far, walker], Some(7)), Some([10.0, 4.0]));
        assert_eq!(pick(&[walker, target], None), Some([5.0, 0.0]));
    }

    #[test]
    fn field_is_held_until_an_enemy_hero_inside_would_walk_out() {
        let mut now = Instant::now();
        let (mut world, mut bots, bot) = lone_bot(HeroClass::Dawnweaver, now);
        let enemy = add_enemy(&mut world, 32101, [8.0, 0.0], now);
        rest(&mut world, bot, &[0, 1, RAY], now);
        now += Duration::from_millis(10);
        frame(&mut world, &mut bots, now);
        let placed = now;
        let centre = live_zone(&world, &world.players[&bot], FIELD, now)
            .expect("the field is placed on the target");
        assert!((centre[0] - 8.0).abs() < 1e-3 && centre[1].abs() < 1e-3);
        assert_eq!(live_zone(&world, &world.players[&bot], 0, now), None);
        assert_eq!(
            live_zone(
                &world,
                &world.players[&bot],
                FIELD,
                now + Duration::from_secs(5)
            ),
            None,
            "an expired field cannot be recast"
        );

        // Nobody inside: every think keeps the field.
        world.players.get_mut(&enemy).unwrap().hero.z = field_reach() + 2.0;
        for _ in 0..20 {
            now += FRAME;
            frame(&mut world, &mut bots, now);
        }
        assert_eq!(
            live_zone(&world, &world.players[&bot], FIELD, now),
            Some(centre)
        );
        // Standing inside: still kept, it slows the hero.
        world.players.get_mut(&enemy).unwrap().hero.z = 0.5;
        for _ in 0..20 {
            now += FRAME;
            frame(&mut world, &mut bots, now);
        }
        assert_eq!(
            live_zone(&world, &world.players[&bot], FIELD, now),
            Some(centre)
        );
        assert!(world.players[&enemy].hero.skills.control.movement(now) < 1.0);
        assert_eq!(world.players[&enemy].hero.hp, 1000.0);

        // Walking out: detonated while still inside, before it would expire.
        while live_zone(&world, &world.players[&bot], FIELD, now).is_some() {
            walk(&mut world, enemy, [0.0, 1.0], now);
            now += FRAME;
            assert!(now < placed + Duration::from_secs(5), "never recast");
            frame(&mut world, &mut bots, now);
        }
        let at = position(&world, enemy);
        assert!((at[0] - centre[0]).hypot(at[1] - centre[1]) <= field_reach());
        assert!(
            world.players[&enemy].hero.hp < 1000.0,
            "the recast caught it"
        );
    }

    #[test]
    fn damage_skills_are_not_spent_on_a_structure() {
        for class in [HeroClass::Dawnweaver, HeroClass::Wildspark] {
            let mut now = Instant::now();
            let (mut world, mut bots, bot) = lone_bot(class, now);
            let mut next = 1;
            crate::world::add_structure(
                &mut world.structures,
                &mut next,
                StructureKind::Tower,
                StructureRole::LaneTower { lane: Lane::Mid },
                Team::Blue,
                crate::entities::Vec3f::new(12.0, 3.0, 0.0),
            );
            // Allied cover lets the bot push the tower.
            add_minion(&mut world, Team::Green, [8.0, 0.0]);
            for _ in 0..60 {
                now += FRAME;
                frame(&mut world, &mut bots, now);
            }
            assert_eq!(
                bots.controllers[&bot].target.map(|t| t.kind),
                Some(TargetKind::Structure),
                "{class:?}"
            );
            let p = &world.players[&bot];
            let loadout = p.hero.skills.loadout.unwrap();
            let mut withheld = 0;
            for slot in SkillSlot::ALL {
                if damages_units_only(loadout.skill(slot).effect) {
                    withheld += 1;
                    assert_eq!(
                        p.timers.last_cast_at[slot.index()],
                        None,
                        "{class:?} {slot:?}"
                    );
                }
            }
            assert_eq!(withheld, 3, "{class:?}: Q/E/R or W/E/R deal damage");
        }
    }

    #[test]
    fn ray_leads_a_hero_walking_at_hero_speed_onto_its_core() {
        let mut now = Instant::now();
        let (mut world, mut bots, bot) = lone_bot(HeroClass::Dawnweaver, now);
        let enemy = add_enemy(&mut world, 32102, [8.0, -2.0], now);
        rest(&mut world, bot, &[0, 1, FIELD], now);
        let caster = world.players[&bot].hero.identity.id;
        let victim = world.players[&enemy].hero.identity.id;
        let mut cast = None;
        let mut hit = None;
        for _ in 0..40 {
            walk(&mut world, enemy, [0.0, 1.0], now);
            now += FRAME;
            let events = frame(&mut world, &mut bots, now);
            if cast.is_none() && world.players[&bot].timers.last_cast_at[3].is_some() {
                cast = Some((position(&world, bot), position(&world, enemy)));
            }
            if let Some(event) = ray_hit(&events, caster, victim) {
                hit = Some(event.clone());
                break;
            }
        }
        let (origin, seen) = cast.expect("the ray is cast at a walking hero");
        let hit = hit.expect("the led ray hits");
        assert!(hit.core_hit, "on the axis, not the edge: {hit:?}");
        // Aimed where the hero stood at the cast, the ray would have missed.
        let at = position(&world, enemy);
        let axis = [seen[0] - origin[0], seen[1] - origin[1]];
        let length = axis[0].hypot(axis[1]);
        let lateral =
            ((at[0] - origin[0]) * axis[1] - (at[1] - origin[1]) * axis[0]).abs() / length;
        assert!(
            lateral > 0.8 + crate::balance::PLAYER_HIT_RADIUS,
            "{lateral}"
        );
    }

    #[test]
    fn ray_takes_the_window_right_after_the_bots_own_root_lands() {
        let mut now = Instant::now();
        let (mut world, mut bots, bot) = lone_bot(HeroClass::Dawnweaver, now);
        let enemy = add_enemy(&mut world, 32104, [8.0, 0.0], now);
        let caster = world.players[&bot].hero.identity.id;
        let victim = world.players[&enemy].hero.identity.id;
        let mut cast = false;
        let mut hit = None;
        for _ in 0..40 {
            now += FRAME;
            let events = frame(&mut world, &mut bots, now);
            let timers = &world.players[&bot].timers;
            if timers.last_cast_at[3] == Some(now) {
                cast = true;
                assert!(timers.last_cast_at[0].is_some(), "Q first");
                assert_eq!(timers.last_cast_at[2], None, "R jumps ahead of E");
                let root = world.players[&enemy].hero.skills.control.root_until;
                assert!(root.is_some_and(|until| until > now), "cast into the root");
            }
            if let Some(event) = ray_hit(&events, caster, victim) {
                hit = Some(event.clone());
                break;
            }
        }
        assert!(cast);
        assert!(hit.expect("the ray follows the root").core_hit);
    }

    #[test]
    fn ray_ignores_a_lone_minion_and_prefers_a_hero_behind_it() {
        let mut now = Instant::now();
        let (mut world, mut bots, bot) = lone_bot(HeroClass::Dawnweaver, now);
        let minion = add_minion(&mut world, Team::Blue, [5.0, 0.0]);
        // Sturdy enough to stay the target through every skill the bot spends.
        let creep = world.minions.get_mut(&minion).unwrap();
        creep.state.hp = 5000.0;
        creep.state.max_hp = 5000.0;
        for _ in 0..40 {
            now += FRAME;
            frame(&mut world, &mut bots, now);
        }
        let p = &world.players[&bot];
        assert_eq!(
            bots.controllers[&bot].target,
            Some(TargetId {
                kind: TargetKind::Minion,
                id: minion,
            })
        );
        assert!(p.timers.last_cast_at[0].is_some(), "Q is still used on it");
        assert_eq!(p.timers.last_cast_at[3], None, "no ultimate on a minion");

        let enemy = add_enemy(&mut world, 32103, [6.0, 5.0], now);
        let caster = world.players[&bot].hero.identity.id;
        let victim = world.players[&enemy].hero.identity.id;
        let mut hit = None;
        for _ in 0..30 {
            now += FRAME;
            if let Some(event) = ray_hit(&frame(&mut world, &mut bots, now), caster, victim) {
                hit = Some(event.clone());
                break;
            }
        }
        assert_eq!(
            bots.controllers[&bot].target.map(|t| t.kind),
            Some(TargetKind::Minion),
            "the nearer minion stays the attack target"
        );
        assert!(hit.expect("the ray picks the hero").core_hit);
    }
}
