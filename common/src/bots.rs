//! Practice-only server controllers. Actors use ordinary hero movement, attacks,
//! resources and receipts; internal addresses are never network endpoints.
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::bot_profile::BotDifficulty;
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

/// Every command the brain issues, in issue order, for the trace-equivalence
/// test (`bots_trace_tests.rs`). Outside `cfg(test)` it is a no-op: the line
/// is never formatted, and it only ever reads values the brain already chose.
mod command_trace {
    #[cfg(test)]
    thread_local! {
        static LOG: std::cell::RefCell<Option<Vec<String>>> =
            const { std::cell::RefCell::new(None) };
    }

    #[cfg(test)]
    pub(super) fn record(line: impl FnOnce() -> String) {
        LOG.with(|log| {
            if let Some(log) = log.borrow_mut().as_mut() {
                log.push(line());
            }
        });
    }

    #[cfg(not(test))]
    #[inline(always)]
    pub(super) fn record(_: impl FnOnce() -> String) {}

    /// Start recording on this thread, discarding anything unread.
    #[cfg(test)]
    pub(super) fn start() {
        LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
    }

    /// The commands recorded since `start` or the previous `drain`.
    #[cfg(test)]
    pub(super) fn drain() -> Vec<String> {
        LOG.with(|log| log.borrow_mut().as_mut().map(std::mem::take))
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn stop() {
        LOG.with(|log| *log.borrow_mut() = None);
    }
}

/// The decisions a level changes, as the difficulty simulator
/// (`common/tests/bot_difficulty_sim.rs`, docs/plans/bot-personas.md S3)
/// observes them. Recording is per thread and off until `start`; a record
/// only reads values the brain already chose. Without `test-support` (and
/// outside unit tests) `record` compiles to nothing.
pub mod sim_probe {
    use std::net::SocketAddr;

    /// Why a gate retreat began.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum GateCause {
        /// `outnumbered_retreat`.
        Outnumbered,
        /// Threat response refused the attacker.
        Threat,
    }

    /// How threat response answered an attacker.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ThreatAnswer {
        /// The attacker became the target.
        Retaliated,
        /// Retreat: the attacker is beyond `VISION`.
        OutOfRange,
        /// Retreat: the trade gate, the outnumbered hold, or both refused
        /// it.
        Refused { trade: bool, outnumbered: bool },
        /// Retreat: it cannot be reached without entering a danger disc.
        Unreachable,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Event {
        /// The health retreat triggered (`retreat_hp`, re-armed by
        /// `rearm_secs`) at this health fraction. `continued` when the bot
        /// was already backing off on a retreat timer or `resume_hp`, so the
        /// trigger only extended that retreat.
        HealthRetreat {
            bot: SocketAddr,
            hp: f32,
            continued: bool,
        },
        /// A gate retreat began in this think tick.
        GateRetreat { bot: SocketAddr, cause: GateCause },
        /// Threat response answered an attacker. A retreat is also
        /// reported as a `GateRetreat`.
        ThreatAnswer {
            bot: SocketAddr,
            answer: ThreatAnswer,
        },
        /// A route was planned (the simulator's cost and thrash indicator).
        Route { bot: SocketAddr },
    }

    #[cfg(any(test, feature = "test-support"))]
    thread_local! {
        static LOG: std::cell::RefCell<Option<Vec<Event>>> =
            const { std::cell::RefCell::new(None) };
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn record(event: impl FnOnce() -> Event) {
        LOG.with(|log| {
            if let Some(log) = log.borrow_mut().as_mut() {
                log.push(event());
            }
        });
    }

    #[cfg(not(any(test, feature = "test-support")))]
    #[inline(always)]
    pub(super) fn record(_: impl FnOnce() -> Event) {}

    /// Start recording on this thread, discarding anything unread.
    #[cfg(any(test, feature = "test-support"))]
    pub fn start() {
        LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
    }

    /// The events recorded since `start` or the previous `drain`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn drain() -> Vec<Event> {
        LOG.with(|log| log.borrow_mut().as_mut().map(std::mem::take))
            .unwrap_or_default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn stop() {
        LOG.with(|log| *log.borrow_mut() = None);
    }
}

#[cfg(test)]
#[path = "bots_trace_tests.rs"]
mod trace_tests;

#[cfg(test)]
#[path = "bots_level_tests.rs"]
mod level_tests;

/// Health fraction below which a bot presses its heal. The same at every
/// level (docs/bot-practice-and-social.md), unlike the retreat threshold.
const HEAL_HP: f32 = 0.28;

/// Which unit a bot picks as a *new* target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetPriority {
    /// The nearest hero or minion; heroes and minions count the same.
    Nearest,
    /// A hero only when no enemy minion is in sight, when it is already in
    /// basic-attack reach, or under threat response.
    MinionFirst,
}

/// When the jungler puts a visible hero ahead of its camp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JungleGank {
    Always,
    IfNotOutnumbered,
    /// Own health at least 0.6 and not outnumbered.
    IfAhead,
    Never,
}

/// What a difficulty level changes in the shared brain
/// (docs/plans/bot-personas.md, "Knobs and default values"). Every field is
/// a same-tick gate or a retreat timer: a level may remove an action or add
/// a retreat, never delay one. Reaction, aim, steering, cast order and the
/// think and route cadence read nothing from here. No level contests
/// objectives, since there is no such behaviour to scale.
///
/// `None` means the gate is off, which is the original behaviour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotProfile {
    /// Start a retreat below this health fraction.
    pub retreat_hp: f32,
    /// Stay home until health reaches this fraction (capped by
    /// `retreat_cap_secs`).
    pub resume_hp: Option<f32>,
    /// Minimum length of a health retreat.
    pub retreat_secs: f32,
    /// Hard stop for `resume_hp`.
    pub retreat_cap_secs: Option<f32>,
    /// Earliest next health-triggered retreat after one starts.
    pub rearm_secs: f32,
    /// Allied minions inside an enemy tower's range before it counts as
    /// covered, for danger discs, lane staging and sieges.
    pub cover_minions: usize,
    /// Added to an uncovered tower's danger-disc radius (m).
    pub tower_margin: f32,
    /// Stop this far short of an uncovered lane tower (m).
    pub staging_margin: f32,
    /// Hit a covered structure only at or above this health fraction.
    pub siege_min_hp: Option<f32>,
    /// Pick a new hero target within this distance (m). Minions keep the
    /// full engage range.
    pub hero_engage_range: f32,
    /// Keep the current target within the engage range plus this (m).
    pub target_leash: f32,
    pub target_priority: TargetPriority,
    /// A new hero target needs own health fraction at least
    /// `min(target's + margin, 0.95)`.
    pub trade_margin: Option<f32>,
    /// Refuse or drop hero targets at this outnumbered count.
    pub outnumbered_hold: Option<u8>,
    /// Start a retreat at this outnumbered count.
    pub outnumbered_retreat: Option<u8>,
    /// A visible hero that hit this bot this recently is exempt from the
    /// engage range and `MinionFirst`.
    pub threat_window_secs: Option<f32>,
    /// Use a mobility technique toward a hero only at or below this health
    /// fraction of the target.
    pub dash_hero_hp: Option<f32>,
    /// Extra travel room a mobility technique keeps from danger discs (m).
    pub mobility_margin: f32,
    pub jungle_gank: JungleGank,
}

impl BotProfile {
    /// The original bots, literal for literal.
    pub const ELITE: Self = Self {
        retreat_hp: 0.28,
        resume_hp: None,
        retreat_secs: 6.0,
        retreat_cap_secs: None,
        rearm_secs: 18.0,
        cover_minions: 1,
        tower_margin: 0.75,
        staging_margin: 1.5,
        siege_min_hp: None,
        hero_engage_range: VISION,
        target_leash: 2.0,
        target_priority: TargetPriority::Nearest,
        trade_margin: None,
        outnumbered_hold: None,
        outnumbered_retreat: None,
        threat_window_secs: None,
        dash_hero_hp: None,
        mobility_margin: 3.0,
        jungle_gank: JungleGank::Always,
    };

    /// Calibrated by the S3 simulator: no outnumbered hold and no threat
    /// response. Its engage range and target priority are Elite's, so it
    /// notices an attacker exactly as Elite does, and with threat response
    /// nearly every answer was a retreat from an attacker it could not reach
    /// (docs/plans/bot-personas.md, S3 notes).
    pub const VETERAN: Self = Self {
        retreat_hp: 0.33,
        resume_hp: Some(0.50),
        retreat_secs: 6.0,
        retreat_cap_secs: Some(20.0),
        rearm_secs: 15.0,
        cover_minions: 1,
        tower_margin: 1.25,
        staging_margin: 2.0,
        siege_min_hp: None,
        hero_engage_range: VISION,
        target_leash: 2.0,
        target_priority: TargetPriority::Nearest,
        trade_margin: None,
        outnumbered_hold: None,
        outnumbered_retreat: None,
        threat_window_secs: None,
        dash_hero_hp: None,
        mobility_margin: 4.0,
        jungle_gank: JungleGank::IfNotOutnumbered,
    };

    pub const REGULAR: Self = Self {
        retreat_hp: 0.40,
        resume_hp: Some(0.70),
        retreat_secs: 6.0,
        retreat_cap_secs: Some(25.0),
        rearm_secs: 12.0,
        cover_minions: 2,
        tower_margin: 2.0,
        staging_margin: 3.0,
        siege_min_hp: Some(0.45),
        hero_engage_range: 13.0,
        target_leash: 1.0,
        target_priority: TargetPriority::Nearest,
        trade_margin: Some(0.0),
        outnumbered_hold: Some(1),
        outnumbered_retreat: Some(2),
        threat_window_secs: Some(2.0),
        dash_hero_hp: Some(0.50),
        mobility_margin: 5.0,
        jungle_gank: JungleGank::IfAhead,
    };

    pub const RECRUIT: Self = Self {
        retreat_hp: 0.50,
        resume_hp: Some(0.85),
        retreat_secs: 6.0,
        retreat_cap_secs: Some(30.0),
        rearm_secs: 8.0,
        cover_minions: 3,
        tower_margin: 3.0,
        staging_margin: 4.0,
        siege_min_hp: Some(0.60),
        hero_engage_range: 11.0,
        target_leash: 0.0,
        target_priority: TargetPriority::MinionFirst,
        trade_margin: Some(0.15),
        outnumbered_hold: Some(1),
        outnumbered_retreat: Some(1),
        threat_window_secs: Some(2.0),
        dash_hero_hp: Some(0.35),
        mobility_margin: 6.0,
        jungle_gank: JungleGank::Never,
    };

    /// The profile a level plays; an unknown level plays as Elite.
    pub const fn for_level(level: BotDifficulty) -> Self {
        match level {
            BotDifficulty::Recruit => Self::RECRUIT,
            BotDifficulty::Regular => Self::REGULAR,
            BotDifficulty::Veteran => Self::VETERAN,
            BotDifficulty::Elite | BotDifficulty::Unknown => Self::ELITE,
        }
    }

    /// Whether a think tick needs `ThinkGates`: some gate reads the
    /// outnumbered count, the threat record, own health or the minions in
    /// sight. Never at Elite, which keeps the original think tick exactly.
    fn has_gates(&self) -> bool {
        self.siege_min_hp.is_some()
            || self.trade_margin.is_some()
            || self.outnumbered_hold.is_some()
            || self.outnumbered_retreat.is_some()
            || self.threat_window_secs.is_some()
            || self.target_priority != TargetPriority::Nearest
            || self.jungle_gank != JungleGank::Always
    }
}

/// Health as a fraction of the maximum; a hero without a maximum counts as
/// empty.
fn hp_fraction(player: &ConnectedPlayer) -> f32 {
    if player.hero.max_hp > 0.0 {
        player.hero.hp / player.hero.max_hp
    } else {
        0.0
    }
}

/// The defend exception: an enemy hero inside the range of one of `team`'s
/// living towers (the distance the tower itself shoots at) or inside
/// `team`'s own base, the fountain and shop zone. Such a hero may always be
/// targeted; the trade, outnumbered, minion-first and jungle-gank gates do
/// not apply to it.
fn defends(world: &crate::game_world::GameWorld, team: Team, hero: &ConnectedPlayer) -> bool {
    let base = match team {
        Team::Green => world.map_layout.home,
        Team::Blue => world.map_layout.away,
    };
    if (hero.hero.x - base.x).hypot(hero.hero.z - base.z) <= crate::balance::BASE_HEAL_RADIUS {
        return true;
    }
    let aim = crate::entities::Vec3f::new(
        hero.hero.x,
        hero.hero.y + crate::balance::AIM_HEIGHT,
        hero.hero.z,
    );
    world.structures.values().any(|tower| {
        tower.state.team == team
            && tower.state.hp > 0.0
            && tower.attack_range > 0.0
            && tower.attack_damage > 0.0
            && crate::entities::Vec3f::new(tower.state.x, tower.state.y, tower.state.z)
                .distance_squared(aim)
                <= tower.attack_range.powi(2)
    })
}

/// Counts `CombatHost::think_gates` scans on this thread, for the
/// once-per-think-tick test. A no-op outside `cfg(test)`.
mod gate_probe {
    #[cfg(test)]
    thread_local! {
        static SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[cfg(test)]
    pub(super) fn count() {
        SCANS.with(|scans| scans.set(scans.get() + 1));
    }

    #[cfg(not(test))]
    #[inline(always)]
    pub(super) fn count() {}

    #[cfg(test)]
    pub(super) fn scans() -> usize {
        SCANS.with(std::cell::Cell::get)
    }
}

/// What one think tick of a lower level knows for its gates
/// (docs/plans/bot-personas.md, "Definitions"). It is built once per think
/// tick, before any target choice, from what a human could see: enemy heroes
/// in the team's sight, its own towers, base and health, and the hits it
/// took. Building it allocates nothing. Elite never builds one.
#[derive(Clone, Copy, Debug)]
struct ThinkGates {
    profile: BotProfile,
    team: Team,
    /// Own health fraction.
    own_hp: f32,
    /// Own basic-attack reach (m), for `MinionFirst`.
    reach: f32,
    /// Visible living enemy heroes within `VISION`, minus living allied
    /// heroes within `VISION`, self included.
    outnumbered: i32,
    /// A visible enemy hero within `VISION` stands where the defend
    /// exception applies.
    defend_in_sight: bool,
    /// A visible living enemy minion is within `VISION`. Only read, and
    /// only computed, for `MinionFirst`.
    minion_in_sight: bool,
    /// The nearest visible living enemy hero that damaged this bot within
    /// `threat_window_secs`, with its distance.
    threat: Option<(u64, f32)>,
}

impl ThinkGates {
    fn outnumbered_at(&self, count: Option<u8>) -> bool {
        count.is_some_and(|count| self.outnumbered >= i32::from(count))
    }

    /// How far away a new hero target may be: the threat is exempt from the
    /// engage range, though not from `VISION`.
    fn hero_range(&self, hero: u64) -> f32 {
        if self.threat.is_some_and(|(id, _)| id == hero) {
            VISION
        } else {
            self.profile.hero_engage_range
        }
    }

    /// The trade gate, capped at 0.95 so a healthy bot can always open a
    /// trade with a healthy hero.
    fn trades_with(&self, hero: &ConnectedPlayer) -> bool {
        self.profile
            .trade_margin
            .is_none_or(|margin| self.own_hp >= (hp_fraction(hero) + margin).min(0.95))
    }

    /// The gates on a new hero target already in range and in sight.
    fn admits_new_hero(
        &self,
        world: &crate::game_world::GameWorld,
        hero: &ConnectedPlayer,
        distance: f32,
    ) -> bool {
        if defends(world, self.team, hero) {
            return true;
        }
        let threat = self
            .threat
            .is_some_and(|(id, _)| id == hero.hero.identity.id);
        let minion_first = self.profile.target_priority == TargetPriority::MinionFirst
            && !threat
            && self.minion_in_sight
            && distance > self.reach + shared::PLAYER_TARGET_RADIUS;
        !minion_first
            && self.trades_with(hero)
            && !self.outnumbered_at(self.profile.outnumbered_hold)
    }

    /// Whether to keep the current hero target: only the outnumbered hold
    /// drops one. The other gates apply to new targets only.
    fn keeps_hero(&self, world: &crate::game_world::GameWorld, hero: &ConnectedPlayer) -> bool {
        defends(world, self.team, hero) || !self.outnumbered_at(self.profile.outnumbered_hold)
    }

    /// The jungler's hero priority (`jungle_gank`).
    fn ganks(&self, world: &crate::game_world::GameWorld, hero: &ConnectedPlayer) -> bool {
        defends(world, self.team, hero)
            || match self.profile.jungle_gank {
                JungleGank::Always => true,
                JungleGank::IfNotOutnumbered => self.outnumbered <= 0,
                JungleGank::IfAhead => self.own_hp >= 0.6 && self.outnumbered <= 0,
                JungleGank::Never => false,
            }
    }

    /// Whether a covered structure may be hit at this health.
    fn sieges(&self) -> bool {
        self.profile
            .siege_min_hp
            .is_none_or(|min| self.own_hp >= min)
    }
}

/// The hero `id` among the joined players.
fn hero_by_id(players: &HashMap<SocketAddr, ConnectedPlayer>, id: u64) -> Option<&ConnectedPlayer> {
    players
        .values()
        .find(|p| p.joined && p.hero.identity.id == id)
}

fn team_index(team: Team) -> usize {
    match team {
        Team::Green => 0,
        Team::Blue => 1,
    }
}

/// At least `cover` of exactly the living minions the tower authority can
/// prefer over a hero. A nearby wave outside its actual three-dimensional
/// range is not cover.
fn tower_has_minion_cover_of(
    tower: &crate::entities::Structure,
    team: Team,
    minions: &HashMap<u64, crate::entities::Minion>,
    cover: usize,
) -> bool {
    let position = crate::entities::Vec3f::new(tower.state.x, tower.state.y, tower.state.z);
    minions
        .values()
        .filter(|minion| {
            minion.state.team == team
                && minion.state.hp > 0.0
                && position.distance_squared(crate::entities::Vec3f::new(
                    minion.state.x,
                    minion.state.y,
                    minion.state.z,
                )) <= tower.attack_range.powi(2)
        })
        .take(cover)
        .count()
        >= cover
}

/// The single-minion cover of the original bots.
#[cfg(test)]
fn tower_has_minion_cover(
    tower: &crate::entities::Structure,
    team: Team,
    minions: &HashMap<u64, crate::entities::Minion>,
) -> bool {
    tower_has_minion_cover_of(tower, team, minions, BotProfile::ELITE.cover_minions)
}

/// The techniques that displace their caster: `bot_mobility_safe` keeps them
/// out of danger, and the dash gate withholds them toward a healthy hero.
fn mobility_technique(
    definition: &shared::loadout::SkillDefinition,
) -> Option<shared::loadout::Technique> {
    use shared::loadout::{SkillEffect, Technique};
    let SkillEffect::Technique { action, .. } = definition.effect else {
        return None;
    };
    matches!(
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
    )
    .then_some(action)
}

/// Autonomous mobility needs room for its actual displacement, which can run
/// opposite the aim or follow a moving recast target. Withhold it before the
/// authority spends resources; ordinary ranged damage remains available.
fn bot_mobility_safe(
    definition: &shared::loadout::SkillDefinition,
    origin: [f32; 2],
    recast: bool,
    danger: &[shared::navigation::Disc],
    margin: f32,
) -> bool {
    use shared::loadout::Technique;
    let Some(action) = mobility_technique(definition) else {
        return true;
    };
    // These recasts follow the earlier victim, which can have moved beyond the
    // initial cast range; the AI has no safe fixed travel bound for that jump.
    if recast && matches!(action, Technique::EchoStrike | Technique::Hook) {
        return danger.is_empty();
    }
    // Seven covers the backwards execute; the margin covers target hitboxes
    // and the one-unit landing offset used by targeted leaps.
    let travel = definition.ability.cast_range.max(7.0) + margin;
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
    /// From the previous think's sighting; `None` when unseen, stale, a jump or
    /// held at either sighting (a hold says nothing of the walk after it).
    velocity: Option<[f32; 2]>,
    /// Steady walking speed, the bound on a course that is not known.
    speed: f32,
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
/// a velocity estimate is aimed where it stands and counts only if that part of
/// the windup is too short to walk out of the ray in any direction. A hampered
/// hero comes first, then the bot's own target, then the nearest. Minions alone
/// never justify the beam.
fn beam_shot<'a>(
    origin: [f32; 2],
    length: f32,
    width: f32,
    windup_secs: f32,
    preferred: Option<u64>,
    heroes: impl IntoIterator<Item = &'a Sighting>,
) -> Option<BeamShot> {
    let side = width + crate::balance::PLAYER_HIT_RADIUS;
    let reach = length + side;
    heroes
        .into_iter()
        .filter_map(|hero| {
            let lead = (windup_secs - hero.held_secs).max(0.0);
            let (aim, walk) = match hero.velocity {
                Some(v) => (
                    [
                        hero.position[0] + v[0] * lead,
                        hero.position[1] + v[1] * lead,
                    ],
                    0.0,
                ),
                None => (hero.position, hero.speed * lead),
            };
            let distance = (aim[0] - origin[0]).hypot(aim[1] - origin[1]);
            (walk <= side && distance + walk <= reach).then_some((hero, aim, distance))
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

/// A beam may select a different hero than the attack target, but must not
/// bypass a lower level's target gates. Reuse the existing once-per-think scan;
/// this filter neither allocates nor changes the shared aiming/cast cadence.
fn admits_beam_target(
    world: &crate::game_world::GameWorld,
    controller: &Controller,
    gates: Option<&ThinkGates>,
    origin: [f32; 2],
    held: bool,
    sighting: &Sighting,
) -> bool {
    let Some(gates) = gates else {
        return true;
    };
    if controller
        .target
        .is_some_and(|target| target.kind == TargetKind::Player && target.id == sighting.id)
    {
        return true; // The normal target path has already admitted this hero.
    }
    let Some(hero) = hero_by_id(&world.players, sighting.id) else {
        return false;
    };
    let distance = (sighting.position[0] - origin[0]).hypot(sighting.position[1] - origin[1]);
    distance <= gates.hero_range(sighting.id)
        && (!held || defends(world, gates.team, hero))
        && gates.admits_new_hero(world, hero, distance)
        && (!controller.jungle || gates.ganks(world, hero))
}

/// The shot of the beam bound to `slot`, if that slot holds a beam with a victim.
fn slot_beam_shot<'a>(
    p: &ConnectedPlayer,
    slot: u8,
    preferred: Option<u64>,
    heroes: impl IntoIterator<Item = &'a Sighting>,
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
    profile: &BotProfile,
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
                && !tower_has_minion_cover_of(tower, team, &world.minions, profile.cover_minions)
        })
        .map(|tower| {
            (progress([tower.state.x, tower.state.z])
                - tower.attack_range
                - shared::PLAYER_TARGET_RADIUS
                - profile.staging_margin)
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
    /// Behaviour level of each team's bots, Green then Blue; Elite unless a
    /// practice host sets it. It survives `clear`, so a round reset keeps it.
    difficulty: [BotDifficulty; 2],
    /// Scratch for the sight of the team a lower-level bot thinks for. It is
    /// reused, so the level gates allocate nothing per think tick.
    sight: Vec<shared::vision::VisionSource>,
    pub defer_fill: bool,
    /// A practice sandbox command replaced the standard roster: no automatic
    /// refill or trimming until `PracticeCommand::Roster` or a round reset
    /// (set by `debug::practice`).
    pub sandbox: bool,
}

impl BotControllers {
    /// Attach AI to an existing retained human without changing its identity,
    /// build, progression or the controller's current plan on repeated votes.
    /// Takeover always plays Elite, whatever the team's level.
    pub fn attach_existing(&mut self, addr: SocketAddr, class: HeroClass, now: Instant) {
        self.attach(addr, class, BotProfile::ELITE, now);
    }

    /// `attach_existing` with an explicit profile, for per-level tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn attach_with_profile(
        &mut self,
        addr: SocketAddr,
        class: HeroClass,
        profile: BotProfile,
        now: Instant,
    ) {
        self.attach(addr, class, profile, now);
    }

    fn attach(&mut self, addr: SocketAddr, class: HeroClass, profile: BotProfile, now: Instant) {
        let (_, lane, jungle) = BOT_COMPOSITION
            .iter()
            .copied()
            .find(|(candidate, ..)| *candidate == class)
            .unwrap_or((class, Lane::Mid, false));
        self.controllers
            .entry(addr)
            .or_insert_with(|| Controller::new(BotKind::Duelist, lane, jungle, profile, now));
    }

    /// The profile the controller at `addr` plays, if any.
    pub fn profile(&self, addr: SocketAddr) -> Option<BotProfile> {
        self.controllers.get(&addr).map(|c| c.profile)
    }

    /// The current target of the controller at `addr`, for the difficulty
    /// simulator.
    #[cfg(any(test, feature = "test-support"))]
    pub fn target(&self, addr: SocketAddr) -> Option<TargetId> {
        self.controllers.get(&addr).and_then(|c| c.target)
    }

    /// The behaviour level of `team`'s bots.
    pub fn difficulty(&self, team: Team) -> BotDifficulty {
        self.difficulty[team_index(team)]
    }

    /// Set `team`'s level and retune its live bots in place; no bot
    /// respawns. Takeover controllers on retained humans stay Elite.
    /// Online hosts never call this: workers always play Elite.
    pub fn set_difficulty(
        &mut self,
        team: Team,
        level: BotDifficulty,
        players: &HashMap<SocketAddr, ConnectedPlayer>,
    ) {
        self.difficulty[team_index(team)] = level;
        let profile = BotProfile::for_level(level);
        for (addr, controller) in &mut self.controllers {
            if players
                .get(addr)
                .is_some_and(|p| p.hero.identity.is_bot && p.hero.identity.team == team)
            {
                controller.profile = profile;
            }
        }
    }

    /// Give `team`'s live bots `profile` instead of their level's, for the
    /// difficulty simulator's calibration sweeps. The team's level is
    /// unchanged, so a later spawn plays the level again.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_team_profile(
        &mut self,
        team: Team,
        profile: BotProfile,
        players: &HashMap<SocketAddr, ConnectedPlayer>,
    ) {
        for (addr, controller) in &mut self.controllers {
            if players
                .get(addr)
                .is_some_and(|p| p.hero.identity.is_bot && p.hero.identity.team == team)
            {
                controller.profile = profile;
            }
        }
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
    /// The behaviour level this bot plays (Elite for takeover).
    profile: BotProfile,
    waypoint: usize,
    route: VecDeque<[f32; 2]>,
    goal: Option<[f32; 2]>,
    next_route: Instant,
    next_think: Instant,
    next_retreat: Instant,
    retreat_until: Option<Instant>,
    /// When the current health retreat began, while `resume_hp` keeps it
    /// going after `retreat_until`. Unused at Elite, which has no
    /// `resume_hp`.
    retreat_started: Option<Instant>,
    target: Option<TargetId>,
    holding_range: bool,
    /// Enemy hero positions at `seen_at`, the last think that looked, and
    /// whether each was held then.
    seen: Vec<(u64, [f32; 2], bool)>,
    seen_at: Option<Instant>,
}

impl Controller {
    fn new(kind: BotKind, lane: Lane, jungle: bool, profile: BotProfile, now: Instant) -> Self {
        Self {
            kind,
            lane,
            jungle,
            profile,
            waypoint: 1,
            route: VecDeque::new(),
            goal: None,
            next_route: now,
            next_think: now,
            next_retreat: now,
            retreat_until: None,
            retreat_started: None,
            target: None,
            holding_range: false,
            seen: Vec::new(),
            seen_at: None,
        }
    }

    /// A gate retreat (outnumbered, or an attacker the gates refuse), from
    /// this think tick: `retreat_secs` long, and the health re-arm is
    /// neither checked nor reset.
    fn gate_retreat(&mut self, profile: &BotProfile, now: Instant) {
        self.retreat_until = Some(now + Duration::from_secs_f32(profile.retreat_secs));
        self.target = None;
        self.route.clear();
        self.next_route = now;
    }
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
        let profile = BotProfile::for_level(self.bots.difficulty(team));
        self.bots
            .controllers
            .insert(addr, Controller::new(kind, lane, jungle, profile, now));
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
            let profile = controller.profile;
            let danger: Vec<_> = self
                .world
                .structures
                .values()
                .filter(|tower| {
                    !tower_has_minion_cover_of(
                        tower,
                        team,
                        &self.world.minions,
                        profile.cover_minions,
                    )
                })
                .filter(|tower| {
                    tower.state.team != team
                        && tower.state.hp > 0.0
                        && tower.attack_range > 0.0
                        && tower.attack_damage > 0.0
                })
                .map(|tower| shared::navigation::Disc {
                    center: [tower.state.x, tower.state.z],
                    radius: tower.attack_range
                        + shared::PLAYER_TARGET_RADIUS
                        + profile.tower_margin,
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
            // One health reading, two thresholds: the retreat depends on the
            // level, the heal never does.
            let retreat_low = player.hero.hp < player.hero.max_hp * profile.retreat_hp;
            let heal_low = player.hero.hp < player.hero.max_hp * HEAL_HP;
            if retreat_low && now >= controller.next_retreat {
                sim_probe::record(|| sim_probe::Event::HealthRetreat {
                    bot: addr,
                    hp: hp_fraction(player),
                    continued: controller.retreat_until.is_some_and(|until| now < until)
                        || controller.retreat_started.is_some(),
                });
                controller.retreat_until =
                    Some(now + Duration::from_secs_f32(profile.retreat_secs));
                controller.next_retreat = now + Duration::from_secs_f32(profile.rearm_secs);
                if profile.resume_hp.is_some() {
                    controller.retreat_started.get_or_insert(now);
                }
                controller.route.clear();
                controller.next_route = now;
            }
            // `resume_hp`: a health retreat goes on until health is back, but
            // never past `retreat_cap_secs` from its start.
            let hold = controller.retreat_started.is_some_and(|started| {
                profile
                    .resume_hp
                    .is_some_and(|resume| player.hero.hp < player.hero.max_hp * resume)
                    && profile
                        .retreat_cap_secs
                        .is_none_or(|cap| now < started + Duration::from_secs_f32(cap))
            });
            let timed = controller.retreat_until.is_some_and(|until| now < until);
            if !timed && !hold {
                controller.retreat_started = None;
            }
            // The hold yields to the defend exception: a hero the think tick
            // took on during the hold (below) is fought, not walked away
            // from. During the hold the target is that hero or nothing.
            let recovering = hold
                && !controller
                    .target
                    .is_some_and(|target| target.kind == TargetKind::Player);
            let mut retreating = turret_retreat.is_some() || timed || recovering;
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
                // The level gates read one scan per think tick, made before any
                // target choice. Elite has no gates and makes none.
                let mut sight = std::mem::take(&mut self.bots.sight);
                let gates = profile.has_gates().then(|| {
                    vision::sources_into(team, self.world, &mut sight);
                    self.think_gates(&profile, addr, &sight, now)
                });
                self.bots.sight = sight;
                // Outnumbered: back off in this think tick, unless there is a
                // hero to defend against.
                if let Some(gates) = gates
                    && !retreating
                    && gates.outnumbered_at(profile.outnumbered_retreat)
                    && !gates.defend_in_sight
                {
                    sim_probe::record(|| sim_probe::Event::GateRetreat {
                        bot: addr,
                        cause: sim_probe::GateCause::Outnumbered,
                    });
                    controller.gate_retreat(&profile, now);
                    retreating = true;
                }
                // Only the `resume_hp` hold keeps the bot home: no timed or
                // turret retreat runs, including one the outnumbered gate just
                // started.
                let held = hold
                    && turret_retreat.is_none()
                    && controller.retreat_until.is_none_or(|until| now >= until);
                controller.target = if let Some(gates) = gates.filter(|_| held) {
                    self.defend_target(&profile, &gates, team, origin, controller.target, now)
                } else if retreating {
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
                                    (p.x - origin[0]).hypot(p.z - origin[1])
                                        <= VISION + profile.target_leash
                                })
                                && (target.kind != TargetKind::Player
                                    || gates.is_none_or(|gates| {
                                        hero_by_id(&self.world.players, target.id)
                                            .is_none_or(|hero| gates.keeps_hero(self.world, hero))
                                    }))
                        })
                        .or_else(|| {
                            if controller.jungle {
                                self.jungle_target(
                                    &profile,
                                    gates.as_ref(),
                                    team,
                                    origin,
                                    controller.lane,
                                    now,
                                )
                            } else {
                                self.profiled_target(
                                    &profile,
                                    gates.as_ref(),
                                    team,
                                    origin,
                                    controller.lane,
                                    now,
                                )
                            }
                        })
                };
                controller.target = controller
                    .target
                    .filter(|target| self.reachable(team, origin, *target, &danger));
                if held {
                    retreating = controller.target.is_none();
                }
                // Threat response: a hero that hit this bot gets an answer in
                // this think tick. Retaliate if the gates allow it, otherwise
                // back off; never carry on as if nothing happened.
                if let Some(gates) = gates
                    && !retreating
                    && let Some((threat, distance)) = gates.threat
                    && controller
                        .target
                        .is_none_or(|target| target.kind != TargetKind::Player)
                {
                    let target = TargetId {
                        kind: TargetKind::Player,
                        id: threat,
                    };
                    let in_range = distance <= VISION;
                    let admitted = in_range
                        && hero_by_id(&self.world.players, threat).is_some_and(|hero| {
                            defends(self.world, team, hero)
                                || (gates.trades_with(hero)
                                    && !gates.outnumbered_at(profile.outnumbered_hold))
                        });
                    let retaliate = admitted && self.reachable(team, origin, target, &danger);
                    sim_probe::record(|| sim_probe::Event::ThreatAnswer {
                        bot: addr,
                        answer: if retaliate {
                            sim_probe::ThreatAnswer::Retaliated
                        } else if !in_range {
                            sim_probe::ThreatAnswer::OutOfRange
                        } else if !admitted {
                            let hero = hero_by_id(&self.world.players, threat);
                            sim_probe::ThreatAnswer::Refused {
                                trade: hero.is_some_and(|hero| !gates.trades_with(hero)),
                                outnumbered: gates.outnumbered_at(profile.outnumbered_hold),
                            }
                        } else {
                            sim_probe::ThreatAnswer::Unreachable
                        },
                    });
                    if retaliate {
                        controller.target = Some(target);
                    } else {
                        sim_probe::record(|| sim_probe::Event::GateRetreat {
                            bot: addr,
                            cause: sim_probe::GateCause::Threat,
                        });
                        controller.gate_retreat(&profile, now);
                        retreating = true;
                    }
                }
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
                            // The dash gate: no mobility technique toward a
                            // hero above the level's health line.
                            let dash_withheld = profile.dash_hero_hp.is_some_and(|limit| {
                                target.kind == TargetKind::Player
                                    && hero_by_id(&self.world.players, target.id)
                                        .is_some_and(|hero| hp_fraction(hero) > limit)
                            });
                            let mut skill_order = [0, 1, 2, 3];
                            skill_order.sort_by_key(|slot| {
                                use shared::loadout::Technique;
                                let p = &self.world.players[&addr];
                                // A hampered victim, e.g. right after this bot's own
                                // root landed, is the beam's window: take it before the
                                // shared recovery goes to another slot.
                                if slot_beam_shot(
                                    p,
                                    *slot,
                                    preferred,
                                    sightings.iter().filter(|hero| {
                                        admits_beam_target(
                                            self.world,
                                            &controller,
                                            gates.as_ref(),
                                            origin,
                                            held,
                                            hero,
                                        )
                                    }),
                                )
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
                                    let Some(shot) = slot_beam_shot(
                                        p,
                                        slot,
                                        preferred,
                                        sightings.iter().filter(|hero| {
                                            admits_beam_target(
                                                self.world,
                                                &controller,
                                                gates.as_ref(),
                                                origin,
                                                held,
                                                hero,
                                            )
                                        }),
                                    ) else {
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
                                if dash_withheld && mobility_technique(definition).is_some() {
                                    continue;
                                }
                                if !bot_mobility_safe(
                                    definition,
                                    [p.hero.x, p.hero.z],
                                    recast,
                                    &danger,
                                    profile.mobility_margin,
                                ) {
                                    continue;
                                }
                                let request = p.hero.skills.request_id.saturating_add(1);
                                command_trace::record(|| {
                                    format!("{} cast {slot} {aim:?}", addr.port())
                                });
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
                        command_trace::record(|| format!("{} unit {slot} {target:?}", addr.port()));
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
                for slot in self_sustain_order(bot, heal_low) {
                    command_trace::record(|| format!("{} sustain {slot}", addr.port()));
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
                    command_trace::record(|| {
                        format!("{} attack {target:?} {request}", addr.port())
                    });
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
                lane_staging_destination(
                    self.world,
                    team,
                    controller.lane,
                    [point.x, point.z],
                    &profile,
                )
            };
            let mut desired = origin;
            if turret_retreat.is_some() {
                desired = destination;
            } else if !in_range {
                let goal_changed = controller
                    .goal
                    .is_none_or(|p| (p[0] - destination[0]).hypot(p[1] - destination[1]) > 2.0);
                if controller.route.is_empty() || (now >= controller.next_route && goal_changed) {
                    sim_probe::record(|| sim_probe::Event::Route { bot: addr });
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
    /// sightings (only if that think was recent and neither think saw the hero
    /// held), which this one then replaces.
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
                let skills = &p.hero.skills;
                let held_secs = remaining(skills.control.root_until)
                    .max(remaining(skills.control.stun_until))
                    .max(remaining(skills.cast_lock_until));
                // A displacement measured across a hold is not the walk after it.
                let velocity = elapsed
                    .zip(controller.seen.iter().find(|(seen, ..)| *seen == id))
                    .filter(|(_, (.., was_held))| !was_held && held_secs == 0.0)
                    .map(|(secs, (_, at, _))| {
                        [(position[0] - at[0]) / secs, (position[1] - at[1]) / secs]
                    })
                    .filter(|v| v[0].hypot(v[1]) <= MAX_LEAD_SPEED);
                Sighting {
                    id,
                    position,
                    velocity,
                    speed: hero_stats::move_speed(p),
                    held_secs,
                    hampered: held_secs > 0.0 || skills.control.movement(now) < 1.0,
                }
            })
            .collect();
        heroes.sort_unstable_by_key(|hero| hero.id);
        controller.seen = heroes
            .iter()
            .map(|hero| (hero.id, hero.position, hero.held_secs > 0.0))
            .collect();
        controller.seen_at = Some(now);
        heroes
    }

    /// Whether `target` can be reached without entering a danger disc.
    fn reachable(
        &self,
        team: Team,
        origin: [f32; 2],
        target: TargetId,
        danger: &[shared::navigation::Disc],
    ) -> bool {
        basic_attack::resolve_hostile_target(
            team,
            target,
            &self.world.players,
            &self.world.minions,
            &self.world.structures,
            &self.world.neutrals,
        )
        .is_some_and(|(point, _)| {
            let goal = [point.x, point.z];
            shared::navigation::clip_discs(origin, goal, danger) == goal
        })
    }

    /// The one scan a lower-level think tick makes for its gates. `sight`
    /// is the current sight of the bot's team. It visits each player at most
    /// twice and allocates nothing.
    fn think_gates(
        &self,
        profile: &BotProfile,
        addr: SocketAddr,
        sight: &[shared::vision::VisionSource],
        now: Instant,
    ) -> ThinkGates {
        gate_probe::count();
        let me = &self.world.players[&addr];
        let team = me.hero.identity.team;
        let origin = [me.hero.x, me.hero.z];
        let distance = |x: f32, z: f32| (x - origin[0]).hypot(z - origin[1]);
        let mut outnumbered = 0_i32;
        let mut defend_in_sight = false;
        for p in self
            .world
            .players
            .values()
            .filter(|p| p.joined && p.hero.hp > 0.0 && distance(p.hero.x, p.hero.z) <= VISION)
        {
            if p.hero.identity.team == team {
                outnumbered -= 1;
            } else if vision::player_visible(sight, p, now) {
                outnumbered += 1;
                defend_in_sight = defend_in_sight || defends(self.world, team, p);
            }
        }
        let minion_in_sight = profile.target_priority == TargetPriority::MinionFirst
            && self.world.minions.values().any(|m| {
                m.state.hp > 0.0
                    && m.state.team != team
                    && distance(m.state.x, m.state.z) <= VISION
                    && vision::target_visible_with(
                        sight,
                        team,
                        TargetId {
                            kind: TargetKind::Minion,
                            id: m.state.id,
                        },
                        self.world,
                        now,
                    )
            });
        let threat = profile.threat_window_secs.and_then(|window| {
            self.combat_log
                .ledger
                .recent_attackers(me.hero.identity.id, now, Duration::from_secs_f32(window))
                .filter_map(|(attacker, _)| hero_by_id(&self.world.players, attacker))
                .filter(|p| {
                    p.hero.hp > 0.0
                        && p.hero.identity.team != team
                        && vision::player_visible(sight, p, now)
                })
                .map(|p| (p.hero.identity.id, distance(p.hero.x, p.hero.z)))
                .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
        });
        ThinkGates {
            profile: *profile,
            team,
            own_hp: hp_fraction(me),
            reach: crate::skills::attack_modifiers(me).0,
            outnumbered,
            defend_in_sight,
            minion_in_sight,
            threat,
        }
    }

    /// The target of a bot that only the `resume_hp` hold keeps home: an
    /// enemy hero the defend exception covers. The current one is kept while
    /// it stays covered, visible and within `VISION` plus the leash;
    /// otherwise the nearest covered hero in range and in sight is taken.
    /// Anything else waits for the hold to end. It reads the sight the gate
    /// scan just filled, so it allocates nothing.
    fn defend_target(
        &self,
        profile: &BotProfile,
        gates: &ThinkGates,
        team: Team,
        origin: [f32; 2],
        current: Option<TargetId>,
        now: Instant,
    ) -> Option<TargetId> {
        let sight = &self.bots.sight;
        let distance = |p: &ConnectedPlayer| (p.hero.x - origin[0]).hypot(p.hero.z - origin[1]);
        let covered = |p: &ConnectedPlayer| {
            p.joined
                && p.hero.hp > 0.0
                && p.hero.identity.team != team
                && vision::player_visible(sight, p, now)
                && defends(self.world, team, p)
        };
        current
            .filter(|target| target.kind == TargetKind::Player)
            .and_then(|target| hero_by_id(&self.world.players, target.id))
            .filter(|p| covered(p) && distance(p) <= VISION + profile.target_leash)
            .or_else(|| {
                self.world
                    .players
                    .values()
                    .filter(|p| covered(p) && distance(p) <= gates.hero_range(p.hero.identity.id))
                    .min_by(|a, b| {
                        distance(a)
                            .total_cmp(&distance(b))
                            .then_with(|| a.hero.identity.id.cmp(&b.hero.identity.id))
                    })
            })
            .map(|p| TargetId {
                kind: TargetKind::Player,
                id: p.hero.identity.id,
            })
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
        profile: &BotProfile,
        gates: Option<&ThinkGates>,
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
                    p,
                )
            })
            .filter(|(d, target, p)| {
                let range = gates.map_or(profile.hero_engage_range, |g| g.hero_range(target.id));
                *d <= range
                    && visible(*target)
                    && gates.is_none_or(|g| {
                        g.ganks(self.world, p) && g.admits_new_hero(self.world, p, *d)
                    })
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
        if let Some((_, target, _)) = hero {
            return Some(target);
        }
        let Some((id, at)) = self.jungle_camp(team, origin) else {
            return self.profiled_target(profile, gates, team, origin, lane, now);
        };
        let camp = TargetId {
            kind: TargetKind::Neutral,
            id,
        };
        ((at[0] - origin[0]).hypot(at[1] - origin[1]) <= VISION && visible(camp)).then_some(camp)
    }

    /// The new target an Elite lane bot at `origin` picks.
    pub fn bot_target(
        &self,
        team: Team,
        origin: [f32; 2],
        lane: Lane,
        now: Instant,
    ) -> Option<TargetId> {
        self.profiled_target(&BotProfile::ELITE, None, team, origin, lane, now)
    }

    fn profiled_target(
        &self,
        profile: &BotProfile,
        gates: Option<&ThinkGates>,
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
                let range = if target.kind == TargetKind::Player {
                    gates.map_or(profile.hero_engage_range, |g| g.hero_range(target.id))
                } else {
                    VISION
                };
                *d <= range
                    && vision::target_visible(team, *target, self.world, now)
                    && (target.kind != TargetKind::Player
                        || gates.is_none_or(|g| {
                            hero_by_id(&self.world.players, target.id)
                                .is_some_and(|hero| g.admits_new_hero(self.world, hero, *d))
                        }))
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
                    && tower_has_minion_cover_of(
                        s,
                        team,
                        &self.world.minions,
                        profile.cover_minions,
                    )
                    && !structure_is_protected(&self.world.structures, s.state.id)
                    && match s.role {
                        StructureRole::BaseTower => true,
                        StructureRole::LaneTower { lane: tower_lane } => tower_lane == lane,
                    }
                    && gates.is_none_or(ThinkGates::sieges)
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
    use crate::combat_feedback::CombatLog;
    use crate::game_world::{GameWorld, TickCtx};
    use crate::match_rules::{MatchMode, MatchRules};
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

    /// Off-axis distance at which the ray still hits a hero.
    fn ray_side_reach() -> f32 {
        let SkillEffect::Beam { width, .. } =
            shared::loadout::skill(shared::loadout::SkillId::DawnRay).effect
        else {
            unreachable!()
        };
        width + crate::balance::PLAYER_HIT_RADIUS
    }

    #[test]
    fn beam_shot_leads_by_the_walkable_windup_and_ranks_hampered_heroes_first() {
        let walker = Sighting {
            id: 7,
            position: [10.0, 0.0],
            velocity: Some([0.0, 5.0]),
            speed: 5.0,
            held_secs: 0.0,
            hampered: false,
        };
        let shot = beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[walker]).unwrap();
        assert_eq!(shot.aim, [10.0, 4.0], "led by the whole windup");

        // No estimate (unseen, or held): aimed where it stands, and only if the
        // walkable part of the windup cannot carry it out of the ray.
        let unknown = Sighting {
            velocity: None,
            ..walker
        };
        assert_eq!(beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[unknown]), None);
        let freed_soon = Sighting {
            held_secs: 0.4,
            hampered: true,
            ..unknown
        };
        assert_eq!(
            beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[freed_soon]),
            None,
            "0.4 s free is 2 units off the axis"
        );
        let held = Sighting {
            held_secs: 0.7,
            ..freed_soon
        };
        let shot = beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[held]).unwrap();
        assert_eq!(shot.aim, [10.0, 0.0], "0.1 s free stays inside the ray");
        assert!(shot.hampered);
        let held_through = Sighting {
            held_secs: 1.0,
            ..freed_soon
        };
        assert_eq!(
            beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[held_through]).map(|s| s.aim),
            Some([10.0, 0.0]),
            "held for the whole windup, no estimate is needed"
        );
        // Nor may the free part carry it past the end of the ray.
        let at_the_end = Sighting {
            position: [45.0 + 0.8 + crate::balance::PLAYER_HIT_RADIUS - 0.2, 0.0],
            ..held
        };
        assert_eq!(
            beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[at_the_end]),
            None
        );
        assert!(
            beam_shot(
                [0.0; 2],
                45.0,
                0.8,
                0.8,
                None,
                &[Sighting {
                    held_secs: 1.0,
                    ..at_the_end
                }]
            )
            .is_some()
        );
        // A hold that ends inside the windup does not outrank a led walker.
        let nearer = Sighting {
            id: 11,
            position: [6.0, 0.0],
            ..freed_soon
        };
        assert_eq!(
            beam_shot([0.0; 2], 45.0, 0.8, 0.8, None, &[nearer, walker]).map(|s| s.aim),
            Some([10.0, 4.0])
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
            speed: 5.0,
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
    fn sightings_measure_no_velocity_across_a_hold() {
        let start = Instant::now();
        let (mut world, mut bots, bot) = lone_bot(HeroClass::Dawnweaver, start);
        let enemy = add_enemy(&mut world, 32106, [8.0, 0.0], start);
        let mut controller = bots.controllers.remove(&bot).unwrap();
        let mut log = CombatLog::default();
        let mut look = |world: &mut GameWorld, think: u32| {
            CombatHost {
                world,
                bots: &mut bots,
                combat_log: &mut log,
                rules: MatchRules::for_mode(MatchMode::Release, 1),
                match_id: 1,
            }
            .sight_enemy_heroes(
                Team::Green,
                &mut controller,
                start + THINK_INTERVAL * think,
            )[0]
            .velocity
        };
        let step = |world: &mut GameWorld| world.players.get_mut(&enemy).unwrap().hero.z += 1.0;
        assert_eq!(look(&mut world, 0), None, "nothing seen before");
        step(&mut world);
        assert_eq!(look(&mut world, 1), Some([0.0, 4.0]));
        // Rooted since the last look: what it walked before is no course.
        step(&mut world);
        world
            .players
            .get_mut(&enemy)
            .unwrap()
            .hero
            .skills
            .control
            .root_until = Some(start + THINK_INTERVAL * 2 + Duration::from_millis(400));
        assert_eq!(look(&mut world, 2), None);
        assert_eq!(look(&mut world, 3), None, "still rooted");
        // Free again, but the last look saw it held.
        step(&mut world);
        assert_eq!(look(&mut world, 4), None);
        step(&mut world);
        assert_eq!(look(&mut world, 5), Some([0.0, 4.0]));
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
    fn independent_ray_targets_respect_lower_level_minion_and_trade_gates() {
        for (profile, own_hp, should_cast) in [
            (BotProfile::ELITE, 1.0, true),
            (BotProfile::RECRUIT, 1.0, false),
            (BotProfile::REGULAR, 0.8, false),
            (BotProfile::REGULAR, 1.0, true),
        ] {
            let now = Instant::now();
            let (mut world, mut bots, bot) = lone_bot(HeroClass::Dawnweaver, now);
            bots.controllers.get_mut(&bot).unwrap().profile = profile;
            world.players.get_mut(&bot).unwrap().hero.hp = own_hp * 1000.0;
            let minion = add_minion(&mut world, Team::Blue, [5.0, 0.0]);
            // Outside Dawnweaver's 10 m basic reach, but inside Recruit's 11 m
            // engage range: MinionFirst must actually reject this candidate.
            let enemy_distance = 10.8;
            assert!(
                enemy_distance
                    > crate::skills::attack_modifiers(&world.players[&bot]).0
                        + shared::PLAYER_TARGET_RADIUS
            );
            assert!(enemy_distance < BotProfile::RECRUIT.hero_engage_range);
            let enemy = add_enemy(&mut world, 32107, [enemy_distance, 0.0], now);
            world
                .players
                .get_mut(&enemy)
                .unwrap()
                .hero
                .skills
                .control
                .root_until = Some(now + Duration::from_secs(2));
            rest(&mut world, bot, &[0, 1, FIELD], now);
            frame(&mut world, &mut bots, now + FRAME);
            assert_eq!(
                bots.controllers[&bot].target,
                Some(TargetId {
                    kind: TargetKind::Minion,
                    id: minion
                }),
                "fixture keeps the nearer minion as its normal target"
            );
            assert_eq!(
                world.players[&bot].timers.last_cast_at[usize::from(RAY)].is_some(),
                should_cast,
                "independent beam target must obey {profile:?}, hp={own_hp}"
            );
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
        assert!(lateral > ray_side_reach(), "{lateral}");
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
    fn ray_is_not_aimed_at_a_root_that_ends_inside_its_windup() {
        let mut now = Instant::now();
        let (mut world, mut bots, bot) = lone_bot(HeroClass::Dawnweaver, now);
        let enemy = add_enemy(&mut world, 32105, [8.0, -2.0], now);
        rest(&mut world, bot, &[0, 1, FIELD, RAY], now);
        let caster = world.players[&bot].hero.identity.id;
        let victim = world.players[&enemy].hero.identity.id;
        // Rooted through several thinks; the ray comes off cooldown with 0.3 s left.
        let freed = now + Duration::from_millis(1300);
        world
            .players
            .get_mut(&enemy)
            .unwrap()
            .hero
            .skills
            .control
            .root_until = Some(freed);
        let ready = freed - Duration::from_millis(300);
        let mut readied = false;
        let mut cast = None;
        let mut hit = None;
        for _ in 0..60 {
            if !readied && now >= ready {
                world.players.get_mut(&bot).unwrap().timers.last_cast_at[usize::from(RAY)] = None;
                readied = true;
            }
            walk(&mut world, enemy, [0.0, 1.0], now);
            now += FRAME;
            let events = frame(&mut world, &mut bots, now);
            if cast.is_none()
                && world.players[&bot].timers.last_cast_at[usize::from(RAY)] == Some(now)
            {
                cast = Some(now);
            }
            if let Some(event) = ray_hit(&events, caster, victim) {
                hit = Some(event.clone());
                break;
            }
        }
        let cast = cast.expect("the ray is cast once the hero walks again");
        assert!(cast > freed, "not aimed at the rooted spot it walks out of");
        assert!(hit.is_some(), "led onto the walking hero instead");
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

#[cfg(test)]
mod profile_tests {
    use super::*;
    use crate::{
        combat_feedback::CombatLog,
        game_world::GameWorld,
        match_rules::{MatchMode, MatchRules},
    };

    fn host<'a>(
        world: &'a mut GameWorld,
        bots: &'a mut BotControllers,
        log: &'a mut CombatLog,
    ) -> CombatHost<'a> {
        CombatHost {
            world,
            bots,
            combat_log: log,
            rules: MatchRules::for_mode(MatchMode::Practice, 5),
            match_id: 1,
        }
    }

    fn profile_of(bots: &BotControllers, addr: SocketAddr) -> BotProfile {
        bots.controllers[&addr].profile
    }

    fn join(world: &mut GameWorld, addr: SocketAddr, team: Team, class: HeroClass, x: f32) {
        let now = Instant::now();
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
        p.hero.progress.level = 10;
        p.hero.progress.ranks = [3; 4];
        p.hero.progress.skill_points = 0;
    }

    #[test]
    fn elite_profile_pins_the_original_literals_with_every_new_gate_off() {
        let p = BotProfile::ELITE;
        assert_eq!(
            (
                p.retreat_hp,
                p.retreat_secs,
                p.rearm_secs,
                p.tower_margin,
                p.staging_margin,
                p.cover_minions,
                p.hero_engage_range,
                p.target_leash,
                p.mobility_margin,
            ),
            (0.28, 6.0, 18.0, 0.75, 1.5, 1, 15.0, 2.0, 3.0)
        );
        assert_eq!(VISION, 15.0);
        assert_eq!(
            (
                p.resume_hp,
                p.retreat_cap_secs,
                p.siege_min_hp,
                p.trade_margin,
                p.outnumbered_hold,
                p.outnumbered_retreat,
                p.threat_window_secs,
                p.dash_hero_hp,
            ),
            (None, None, None, None, None, None, None, None)
        );
        assert_eq!(p.target_priority, TargetPriority::Nearest);
        assert_eq!(p.jungle_gank, JungleGank::Always);
        assert_eq!(HEAL_HP, 0.28, "the heal threshold is not a level knob");
        assert_eq!(BotProfile::for_level(BotDifficulty::Elite), p);
        assert_eq!(BotProfile::for_level(BotDifficulty::Unknown), p);
    }

    #[test]
    fn weaker_levels_are_never_bolder_on_any_knob() {
        let levels = BotDifficulty::ALL.map(BotProfile::for_level);
        for pair in levels.windows(2) {
            let (weaker, stronger) = (&pair[0], &pair[1]);
            assert!(weaker.retreat_hp >= stronger.retreat_hp);
            assert!(weaker.resume_hp.unwrap_or(0.0) >= stronger.resume_hp.unwrap_or(0.0));
            assert_eq!(weaker.retreat_secs, stronger.retreat_secs);
            assert!(weaker.rearm_secs <= stronger.rearm_secs);
            assert!(weaker.cover_minions >= stronger.cover_minions);
            assert!(weaker.tower_margin >= stronger.tower_margin);
            assert!(weaker.staging_margin >= stronger.staging_margin);
            assert!(weaker.siege_min_hp.unwrap_or(0.0) >= stronger.siege_min_hp.unwrap_or(0.0));
            assert!(weaker.hero_engage_range <= stronger.hero_engage_range);
            assert!(weaker.hero_engage_range <= VISION);
            assert!(weaker.target_leash <= stronger.target_leash);
            assert!(weaker.mobility_margin >= stronger.mobility_margin);
            assert!(weaker.dash_hero_hp.unwrap_or(1.0) <= stronger.dash_hero_hp.unwrap_or(1.0));
        }
    }

    #[test]
    fn levels_are_per_team_survive_a_round_reset_and_never_retune_takeover() {
        let now = Instant::now();
        let mut world = GameWorld::empty();
        let mut bots = BotControllers::default();
        let mut log = CombatLog::default();
        assert_eq!(bots.difficulty(Team::Green), BotDifficulty::Elite);
        assert_eq!(bots.difficulty(Team::Blue), BotDifficulty::Elite);
        bots.set_difficulty(Team::Blue, BotDifficulty::Recruit, &world.players);
        let human: SocketAddr = "127.0.0.1:33001".parse().unwrap();
        join(&mut world, human, Team::Blue, HeroClass::Warrior, 0.0);
        let mut host = host(&mut world, &mut bots, &mut log);
        let blue = host
            .spawn_bot(Team::Blue, None, BotKind::Lane, now)
            .unwrap();
        let green = host
            .spawn_bot(Team::Green, None, BotKind::Lane, now)
            .unwrap();
        host.bots.attach_existing(human, HeroClass::Warrior, now);
        assert_eq!(profile_of(host.bots, blue), BotProfile::RECRUIT);
        assert_eq!(profile_of(host.bots, green), BotProfile::ELITE);
        assert_eq!(
            profile_of(host.bots, human),
            BotProfile::ELITE,
            "takeover is Elite"
        );

        // A level change retunes the live bots of that team only, in place.
        let position = [
            host.world.players[&blue].hero.x,
            host.world.players[&blue].hero.z,
        ];
        host.bots
            .set_difficulty(Team::Blue, BotDifficulty::Regular, &host.world.players);
        assert_eq!(profile_of(host.bots, blue), BotProfile::REGULAR);
        assert_eq!(profile_of(host.bots, green), BotProfile::ELITE);
        assert_eq!(profile_of(host.bots, human), BotProfile::ELITE);
        assert_eq!(
            [
                host.world.players[&blue].hero.x,
                host.world.players[&blue].hero.z
            ],
            position,
            "no respawn"
        );

        host.bots.clear();
        assert!(host.bots.controllers.is_empty());
        assert_eq!(host.bots.difficulty(Team::Blue), BotDifficulty::Regular);
        assert_eq!(host.bots.difficulty(Team::Green), BotDifficulty::Elite);
    }

    #[test]
    fn the_retreat_threshold_comes_from_the_profile_and_the_heal_stays_at_28_percent() {
        // Warrior: W is a heal. Alone on an empty map, every decision is a
        // retreat or a self-sustain cast.
        for (hp, profile, retreats, heals) in [
            (0.45, BotProfile::ELITE, false, false),
            (0.45, BotProfile::RECRUIT, true, false),
            (0.20, BotProfile::ELITE, true, true),
            (0.20, BotProfile::RECRUIT, true, true),
        ] {
            let now = Instant::now();
            let mut world = GameWorld::empty();
            let addr: SocketAddr = "[::]:33101".parse().unwrap();
            join(&mut world, addr, Team::Green, HeroClass::Warrior, 0.0);
            world.players.get_mut(&addr).unwrap().hero.hp = 1000.0 * hp;
            let mut bots = BotControllers::default();
            bots.attach_with_profile(addr, HeroClass::Warrior, profile, now);
            let mut log = CombatLog::default();
            host(&mut world, &mut bots, &mut log)
                .simulate_bots(now + Duration::from_millis(10), 0.01);
            let case = format!("hp {hp} retreat_hp {}", profile.retreat_hp);
            assert_eq!(
                bots.controllers[&addr].retreat_until.is_some(),
                retreats,
                "{case}"
            );
            assert_eq!(
                world.players[&addr].timers.last_cast_at[1].is_some(),
                heals,
                "{case}"
            );
        }
    }

    #[test]
    fn a_new_hero_target_respects_the_engage_range_and_minions_keep_vision() {
        let now = Instant::now();
        let mut world = GameWorld::empty();
        // The bot's own hero is the only Green sight source.
        let me: SocketAddr = "[::]:33201".parse().unwrap();
        let enemy: SocketAddr = "127.0.0.1:33202".parse().unwrap();
        join(&mut world, me, Team::Green, HeroClass::Warrior, 0.0);
        join(&mut world, enemy, Team::Blue, HeroClass::Warrior, 13.0);
        let id = world.players[&enemy].hero.identity.id;
        let mut bots = BotControllers::default();
        let mut log = CombatLog::default();
        let host = host(&mut world, &mut bots, &mut log);
        let pick = |profile: &BotProfile| {
            host.profiled_target(profile, None, Team::Green, [0.0, 0.0], Lane::Mid, now)
        };
        let hero = Some(TargetId {
            kind: TargetKind::Player,
            id,
        });
        assert_eq!(pick(&BotProfile::ELITE), hero);
        assert_eq!(
            host.bot_target(Team::Green, [0.0, 0.0], Lane::Mid, now),
            hero
        );
        assert_eq!(pick(&BotProfile::REGULAR), hero, "13 m is inside 13 m");
        assert_eq!(pick(&BotProfile::RECRUIT), None, "13 m is beyond 11 m");
    }
}
