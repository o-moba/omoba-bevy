//! Bounded, authoritative execution of resolved skill definitions.
//! Presets choose IDs; this module dispatches reusable effects, never hero classes.
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::combat::{CombatEntityKind, CombatEvent, ProjectileStyle};
use shared::loadout::{
    DamageType, EffectVisualKind, LoadoutState, PassiveId, ResolvedLoadout, SkillEffect,
    SkillEffectState, SkillId, SkillSlotState, WeaponMode, skill,
};
use shared::map::Team;
use shared::wire::{GameState, TargetId, TargetKind};
use shared::{SkillSlot, scaled_cast_range, scaled_mana_cost};

use crate::combat_feedback::{HitSource, apply_player_damage_kind};
use crate::entities::ConnectedPlayer;
use crate::game_world::{GameWorld, TickCtx};
use crate::hero_stats;

pub mod advanced;
mod crowd_control;
mod dagger;

const MAX_EFFECTS: usize = shared::loadout::MAX_ACTIVE_EFFECTS;
const MAX_OWNER_EFFECTS: usize = shared::loadout::MAX_EFFECTS_PER_OWNER;
const MAX_STATUS_SOURCES: usize = 32;
const MAX_MARKS: usize = 2048;
fn participation_window() -> Duration {
    match shared::loadout::passive(PassiveId::Momentum) {
        shared::loadout::PassiveEffect::Momentum {
            assist_window_secs, ..
        } => duration(assist_window_secs),
        _ => Duration::ZERO,
    }
}
type TargetKey = (u8, u64);
fn key(t: TargetId) -> TargetKey {
    (
        match t.kind {
            TargetKind::Player => 0,
            TargetKind::Minion => 1,
            TargetKind::Structure => 2,
            TargetKind::Neutral => 3,
        },
        t.id,
    )
}
fn duration(s: f32) -> Duration {
    Duration::from_secs_f32(s.max(0.0))
}
fn remaining(t: Option<Instant>, now: Instant) -> f32 {
    t.map_or(0.0, |t| t.saturating_duration_since(now).as_secs_f32())
}
fn add(a: [f32; 2], b: [f32; 2], factor: f32) -> [f32; 2] {
    [a[0] + b[0] * factor, a[1] + b[1] * factor]
}
fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shield {
    source: u64,
    amount: f32,
    expires: Instant,
}
#[derive(Debug, Clone, PartialEq)]
struct Slow {
    source: u64,
    multiplier: f32,
    expires: Instant,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ControlState {
    pub root_until: Option<Instant>,
    pub stun_until: Option<Instant>,
    slows: Vec<Slow>,
    pub reveal_until: Option<Instant>,
    revealed_to: [Option<Instant>; 2],
}
impl ControlState {
    pub fn movement(&self, now: Instant) -> f32 {
        if remaining(self.root_until, now).max(remaining(self.stun_until, now)) > 0.0 {
            return 0.0;
        }
        self.slows
            .iter()
            .filter(|s| now < s.expires)
            .fold(1.0_f32, |m, s| m.min(s.multiplier))
    }
    fn apply(&mut self, source: u64, root: f32, slow: f32, secs: f32, reveal: f32, now: Instant) {
        if root > 0.0 {
            self.root_until = Some(self.root_until.unwrap_or(now).max(now + duration(root)));
        }
        if reveal > 0.0 {
            self.reveal_until = Some(self.reveal_until.unwrap_or(now).max(now + duration(reveal)));
        }
        self.slows.retain(|s| now < s.expires);
        if slow < 1.0 && secs > 0.0 {
            if let Some(s) = self.slows.iter_mut().find(|s| s.source == source) {
                s.multiplier = slow;
                s.expires = now + duration(secs);
            } else if self.slows.len() < MAX_STATUS_SOURCES {
                self.slows.push(Slow {
                    source,
                    multiplier: slow,
                    expires: now + duration(secs),
                });
            }
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeroSkills {
    pub loadout: Option<ResolvedLoadout>,
    pub advanced: advanced::HeroState,
    pub request_id: u64,
    pub control: ControlState,
    pub shields: Vec<Shield>,
    pub mode: WeaponMode,
    pub stacks: u8,
    pub stacks_until: Option<Instant>,
    pub momentum_until: Option<Instant>,
    pub recovery_until: Option<Instant>,
    pub marked_until: Option<Instant>,
    // The effect ID identifies the exact zone; a stale recast cannot hit a replacement.
    zones: [Option<(u64, Instant, SkillId)>; 4],
}
impl HeroSkills {
    pub fn absorb(&mut self, damage: f32, now: Instant) -> f32 {
        self.shields.retain(|s| s.expires > now && s.amount > 0.0);
        self.shields.sort_by_key(|s| (s.expires, s.source));
        let mut left = damage;
        for shield in &mut self.shields {
            let taken = left.min(shield.amount);
            shield.amount -= taken;
            left -= taken;
        }
        left
    }
    pub fn transient_reset(&mut self) {
        self.advanced.reset();
        self.control = ControlState::default();
        self.shields.clear();
        self.stacks = 0;
        self.stacks_until = None;
        self.momentum_until = None;
        self.recovery_until = None;
        self.marked_until = None;
        self.zones = [None; 4];
        self.mode = WeaponMode::Repeater;
    }
    pub fn movement(&self, now: Instant) -> f32 {
        self.control.movement(now)
            * if remaining(self.advanced.speed_until, now) > 0.0 {
                1.3
            } else {
                1.0
            }
            * if remaining(self.momentum_until, now) > 0.0 {
                momentum_parameters(self).1
            } else {
                1.0
            }
    }
    pub fn attack_rate(&self, now: Instant) -> f32 {
        let stacks = if remaining(self.stacks_until, now) > 0.0 && self.mode == WeaponMode::Repeater
        {
            self.stacks
        } else {
            0
        };
        let step = self.loadout.as_ref().and_then(toggle).map_or(0.0, |t| t.5);
        self.advanced.attack_rate(now)
            * (1.0 + stacks as f32 * step)
            * if remaining(self.momentum_until, now) > 0.0 {
                momentum_parameters(self).2
            } else {
                1.0
            }
    }
}
fn momentum_parameters(s: &HeroSkills) -> (f32, f32, f32, f32) {
    match s
        .loadout
        .as_ref()
        .map(|l| shared::loadout::passive(l.passive()))
    {
        Some(shared::loadout::PassiveEffect::Momentum {
            duration_secs,
            move_multiplier,
            attack_speed_multiplier,
            assist_window_secs,
        }) => (
            duration_secs,
            move_multiplier,
            attack_speed_multiplier,
            assist_window_secs,
        ),
        _ => (0.0, 1.0, 1.0, 0.0),
    }
}
// range, damage, interval, mana, splash, stack step, stack duration, max stacks.
fn toggle(loadout: &ResolvedLoadout) -> Option<(f32, f32, f32, f32, f32, f32, f32, u8)> {
    loadout
        .skills()
        .iter()
        .find_map(|id| match skill(*id).effect {
            SkillEffect::WeaponToggle {
                rocket_range,
                rocket_damage_multiplier,
                rocket_cooldown_multiplier,
                rocket_mana_cost,
                splash_radius,
                minigun_stack_attack_speed,
                stack_duration_secs,
                max_stacks,
            } => Some((
                rocket_range,
                rocket_damage_multiplier,
                rocket_cooldown_multiplier,
                rocket_mana_cost,
                splash_radius,
                minigun_stack_attack_speed,
                stack_duration_secs,
                max_stacks,
            )),
            _ => None,
        })
}
pub fn attack_modifiers(p: &ConnectedPlayer) -> (f32, f32, f32, f32, f32) {
    let base = shared::basic_attack_for_class(
        p.hero
            .skills
            .loadout
            .as_ref()
            .map_or(p.hero.identity.hero_class, |l| l.core().class()),
    )
    .range;
    if p.hero.skills.mode == WeaponMode::Rockets {
        if let Some(t) = p.hero.skills.loadout.as_ref().and_then(toggle) {
            return (t.0, t.1, t.2, t.3, t.4);
        }
    }
    (base, 1.0, 1.0, 0.0, 0.0)
}
/// Shared accepted-Technique hooks: specialized executors must preserve the
/// recipe's passive and combat lifecycle after all admission gates have passed.
fn accepted_technique(p: &mut ConnectedPlayer, now: Instant) {
    p.hero.skills.advanced.last_combat = Some(now);
    p.hero.skills.advanced.forge_ready = false;
    p.hero.skills.advanced.forge_since = None;
    if p.hero
        .skills
        .loadout
        .is_some_and(|l| l.passive() == PassiveId::Flow)
    {
        p.hero.skills.advanced.flow_attacks = 2;
        p.hero.skills.advanced.flow_until = Some(now + duration(3.0));
    }
}

pub fn accepted_basic(p: &mut ConnectedPlayer, now: Instant) {
    p.hero.skills.advanced.last_combat = Some(now);
    p.hero.skills.advanced.forge_ready = false;
    p.hero.skills.advanced.forge_since = None;
    if p.hero.skills.mode == WeaponMode::Repeater {
        if let Some(t) = p.hero.skills.loadout.as_ref().and_then(toggle) {
            if remaining(p.hero.skills.stacks_until, now) == 0.0 {
                p.hero.skills.stacks = 0;
            }
            p.hero.skills.stacks = p.hero.skills.stacks.saturating_add(1).min(t.7);
            p.hero.skills.stacks_until = Some(now + duration(t.6));
        }
    }
}
pub fn state(p: &ConnectedPlayer, now: Instant) -> Option<LoadoutState> {
    let s = &p.hero.skills;
    let shields: f32 = s
        .shields
        .iter()
        .filter(|s| s.expires > now)
        .map(|s| s.amount)
        .sum();
    if s.loadout.is_none()
        && shields == 0.0
        && s.control.movement(now) == 1.0
        && remaining(s.marked_until, now) == 0.0
        && s.advanced.concussion_stacks == 0
        && remaining(s.advanced.brittle_until, now) == 0.0
    {
        return None;
    }
    let atk = attack_modifiers(p);
    Some(LoadoutState {
        concussion_stacks: s.advanced.concussion_stacks,
        brittle: remaining(s.advanced.brittle_until, now) > 0.0,
        vital_rotation: s.advanced.essence % 4,
        challenge_target: s.advanced.challenge.as_ref().map(|c| c.target.id),
        challenge_sides: s.advanced.challenge.as_ref().map_or(0, |c| c.sides),
        forge_ready: s.advanced.forge_ready,
        forge_remaining_secs: s.advanced.forge_since.map_or(0.0, |at| {
            (3.0 - now.saturating_duration_since(at).as_secs_f32()).max(0.0)
        }),
        energy: s
            .loadout
            .is_some_and(|l| l.core() == shared::loadout::CoreId::Stormfist),
        camouflaged: advanced::camouflaged(p, now),
        parrying: remaining(s.advanced.parry_until, now) > 0.0,
        souls: s.advanced.souls,
        orb_position: s.advanced.orb.as_ref().map(|o| o.pos),
        forged: s.advanced.forged,
        recipe: s.loadout.as_ref().map(|l| l.recipe().clone()),
        slots: std::array::from_fn(|i| {
            let zone = s.zones[i]
                .filter(|(_, _, skill)| s.loadout.is_some_and(|l| l.skills()[i] == *skill));
            let recast = s.advanced.recasts[i]
                .as_ref()
                .filter(|r| r.uses > 0 && s.loadout.is_some_and(|l| l.skills()[i] == r.skill));
            SkillSlotState {
                can_recast: zone.is_some_and(|(_, end, _)| now < end)
                    || recast.is_some_and(|r| r.until > now),
                recast_remaining_secs: zone.map_or_else(
                    || recast.map_or(0.0, |r| remaining(Some(r.until), now)),
                    |(_, end, _)| remaining(Some(end), now),
                ),
                active: zone.is_some_and(|(_, end, _)| now < end),
            }
        }),
        weapon_mode: s.mode,
        shield_hp: shields,
        stun_remaining_secs: remaining(s.control.stun_until, now),
        root_remaining_secs: remaining(s.control.root_until, now)
            .max(remaining(s.control.stun_until, now)),
        slow_multiplier: s
            .control
            .slows
            .iter()
            .filter(|s| s.expires > now)
            .fold(1.0_f32, |m, s| m.min(s.multiplier)),
        movement_multiplier: s.movement(now),
        basic_attack_range: atk.0,
        basic_attack_mana_cost: atk.3,
        passive_remaining_secs: remaining(s.momentum_until, now),
        mark_remaining_secs: remaining(s.marked_until, now),
        passive_stacks: if remaining(s.stacks_until, now) > 0.0 {
            s.stacks
        } else {
            0
        },
        cast_request_id: s.request_id,
    })
}

#[derive(Clone)]
struct ActiveEffect {
    id: u64,
    owner: u64,
    team: Team,
    skill: SkillId,
    slot: u8,
    pos: [f32; 2],
    origin: [f32; 2],
    end: [f32; 2],
    direction: [f32; 2],
    expires: Instant,
    armed_at: Instant,
    scale: f32,
    traveled: f32,
    returning: bool,
    hits: BTreeSet<TargetKey>,
    hit_count: u8,
    fired: bool,
    cast_request: u64,
}
#[derive(Clone)]
struct Mark {
    expires: Instant,
    amount: f32,
}
#[derive(Default)]
pub struct SkillWorld {
    dagger_chance: dagger::Chance,
    effects: BTreeMap<u64, ActiveEffect>,
    advanced: advanced::WorldState,
    next_id: u64,
    pub npc_controls: BTreeMap<TargetKey, ControlState>,
    marks: BTreeMap<(TargetKey, u64), Mark>,
    contributions: BTreeMap<TargetKey, BTreeMap<u64, Instant>>,
    pub attack_splash: BTreeMap<u64, f32>,
    pending: Vec<CombatEvent>,
    trap_hits: BTreeMap<(u64, u64, u64), Instant>,
    seen_deaths: BTreeSet<TargetKey>,
}
impl SkillWorld {
    pub(crate) fn rocket_sight(
        &self,
        team: Team,
    ) -> impl Iterator<Item = shared::vision::VisionSource> + '_ {
        self.effects
            .values()
            .filter(move |effect| {
                effect.team == team
                    && matches!(skill(effect.skill).effect, SkillEffect::ImpactRocket { .. })
            })
            .map(|effect| shared::vision::VisionSource {
                position: effect.pos,
                radius: shared::vision::ROCKET_SIGHT_RADIUS,
            })
    }

    fn id(&mut self) -> u64 {
        self.next_id = self.next_id.saturating_add(1);
        self.next_id
    }
    pub fn npc_stunned(&self, t: TargetId, now: Instant) -> bool {
        self.npc_controls
            .get(&key(t))
            .is_some_and(|s| remaining(s.stun_until, now) > 0.0)
    }
    pub fn npc_movement(&self, t: TargetId, now: Instant) -> f32 {
        self.npc_controls
            .get(&key(t))
            .map_or(1.0, |s| s.movement(now))
    }
    pub fn npc_revealed(&self, t: TargetId, team: Team, now: Instant) -> bool {
        self.npc_controls
            .get(&key(t))
            .is_some_and(|s| remaining(s.revealed_to[usize::from(team == Team::Blue)], now) > 0.0)
    }
}
#[derive(Clone, Copy)]
struct Candidate {
    target: TargetId,
    pos: [f32; 2],
    radius: f32,
    team: Option<Team>,
    hp: f32,
    max_hp: f32,
}
fn candidates(w: &GameWorld) -> Vec<Candidate> {
    let mut v = Vec::new();
    for p in w.players.values().filter(|p| p.joined && p.hero.hp > 0.0) {
        v.push(Candidate {
            target: TargetId {
                kind: TargetKind::Player,
                id: p.hero.identity.id,
            },
            pos: [p.hero.x, p.hero.z],
            radius: crate::balance::PLAYER_HIT_RADIUS,
            team: Some(p.hero.identity.team),
            hp: p.hero.hp,
            max_hp: p.hero.max_hp,
        });
    }
    for p in w.minions.values().filter(|p| p.state.hp > 0.0) {
        v.push(Candidate {
            target: TargetId {
                kind: TargetKind::Minion,
                id: p.state.id,
            },
            pos: [p.state.x, p.state.z],
            radius: crate::balance::MINION_RADIUS,
            team: Some(p.state.team),
            hp: p.state.hp,
            max_hp: p.state.max_hp,
        });
    }
    for p in w
        .neutrals
        .values()
        .filter(|p| p.state.hp > 0.0 && p.dead_until.is_none())
    {
        v.push(Candidate {
            target: TargetId {
                kind: TargetKind::Neutral,
                id: p.state.id,
            },
            pos: [p.state.x, p.state.z],
            radius: crate::balance::NEUTRAL_RADIUS,
            team: None,
            hp: p.state.hp,
            max_hp: p.state.max_hp,
        });
    }
    for p in w.structures.values().filter(|p| p.state.hp > 0.0) {
        v.push(Candidate {
            target: TargetId {
                kind: TargetKind::Structure,
                id: p.state.id,
            },
            pos: [p.state.x, p.state.z],
            radius: crate::world::structure_radius(p.state.kind),
            team: Some(p.state.team),
            hp: p.state.hp,
            max_hp: p.state.max_hp,
        });
    }
    v.sort_by_key(|c| key(c.target));
    v
}
fn hostile(c: &Candidate, team: Team) -> bool {
    c.team != Some(team) && c.target.kind != TargetKind::Structure
}
// First intersection parameter, not closest-point order; stable across HashMap layout.
fn intersection(a: [f32; 2], b: [f32; 2], p: [f32; 2], r: f32) -> Option<f32> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let m = [a[0] - p[0], a[1] - p[1]];
    let c = m[0] * m[0] + m[1] * m[1] - r * r;
    if c <= 0.0 {
        return Some(0.0);
    }
    let aa = d[0] * d[0] + d[1] * d[1];
    if aa <= 0.000001 {
        return None;
    }
    let bb = m[0] * d[0] + m[1] * d[1];
    let disc = bb * bb - aa * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-bb - disc.sqrt()) / aa;
    (0.0..=1.0).contains(&t).then_some(t)
}
fn hits(cs: &[Candidate], a: [f32; 2], b: [f32; 2], r: f32) -> Vec<Candidate> {
    let mut v: Vec<_> = cs
        .iter()
        .filter_map(|c| intersection(a, b, c.pos, r + c.radius).map(|t| (t, *c)))
        .collect();
    v.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| key(a.1.target).cmp(&key(b.1.target)))
    });
    v.into_iter().map(|(_, c)| c).collect()
}
fn source(owner: u64, slot: u8) -> HitSource {
    let mut s = HitSource::new(CombatEntityKind::Player, owner, ProjectileStyle::Arcane);
    s.action_slot = Some(slot);
    s
}
fn passive(w: &GameWorld, owner: u64) -> Option<PassiveId> {
    w.players
        .values()
        .find(|p| p.hero.identity.id == owner)
        .and_then(|p| p.hero.skills.loadout.as_ref())
        .map(|l| l.passive())
}
fn raw_damage(
    w: &mut GameWorld,
    target: TargetId,
    amount: f32,
    kind: DamageType,
    src: HitSource,
    team: Team,
    now: Instant,
) -> Option<CombatEvent> {
    let e = match target.kind {
        TargetKind::Player => {
            apply_player_damage_kind(&mut w.players, target.id, amount, now, kind)
        }
        TargetKind::Minion => crate::sim::minions::apply_minion_damage(
            &mut w.players,
            &mut w.minions,
            target.id,
            amount,
            team,
        ),
        TargetKind::Neutral => crate::sim::neutrals::apply_neutral_damage(
            &mut w.players,
            &mut w.neutrals,
            &mut w.team_buffs,
            target.id,
            amount,
            src.entity.id,
            now,
        ),
        TargetKind::Structure => crate::sim::towers::apply_structure_damage(
            &mut w.structures,
            target.id,
            amount,
            team,
            &mut w.game_state,
        ),
    };
    e.map(|e| src.annotate(e))
}
/// Accepted hit effects are separate from HP receipts: a fully absorbed hit still
/// applies a mark/control, while accounting never reports shield loss as HP damage.
pub fn apply_hit(
    w: &mut GameWorld,
    target: TargetId,
    amount: f32,
    kind: DamageType,
    src: HitSource,
    team: Team,
    basic: bool,
    mark: bool,
    consume: bool,
    now: Instant,
) -> Vec<CombatEvent> {
    let Some(c) = candidates(w)
        .into_iter()
        .find(|c| c.target == target && c.team != Some(team))
    else {
        return Vec::new();
    };
    if target.kind == TargetKind::Player
        && w.players
            .values()
            .any(|p| p.hero.identity.id == target.id && p.modifiers.god_mode)
    {
        return Vec::new();
    }
    let radiance = src.entity.kind == CombatEntityKind::Player
        && passive(w, src.entity.id) == Some(PassiveId::Radiance);
    let shield_before = if target.kind == TargetKind::Player {
        w.players
            .values()
            .find(|p| p.hero.identity.id == target.id)
            .map_or(0.0, |p| {
                p.hero
                    .skills
                    .shields
                    .iter()
                    .filter(|s| s.expires > now)
                    .map(|s| s.amount)
                    .sum()
            })
    } else {
        0.0
    };
    if target.kind == TargetKind::Player
        && w.players
            .values()
            .any(|p| p.hero.identity.id == target.id && p.hero.skills.advanced.immune(now))
    {
        return Vec::new();
    }
    let mut events = Vec::new();
    if (basic || consume) && radiance {
        if let Some(m) = w
            .skill_runtime
            .marks
            .remove(&(key(target), src.entity.id))
            .filter(|m| m.expires > now)
        {
            events.extend(raw_damage(
                w,
                target,
                m.amount,
                DamageType::Magic,
                src,
                team,
                now,
            ));
        }
    }
    let amount = if basic && src.entity.kind == CombatEntityKind::Player {
        amount * dagger::basic_multiplier(w, src.entity.id, target)
    } else {
        amount
    };
    let primary = raw_damage(w, target, amount, kind, src, team, now);
    if basic && target.kind != TargetKind::Structure && src.entity.kind == CombatEntityKind::Player
    {
        crate::shop::basic_lifesteal(
            w,
            src.entity.id,
            primary.iter().map(|event| event.amount).sum(),
        );
    }
    events.extend(primary);
    if src.entity.kind == CombatEntityKind::Player && amount > 0.0 {
        events.extend(advanced::on_hit(w, c, amount, src, team, basic, now));
    }
    if mark
        && radiance
        && c.target.kind != TargetKind::Structure
        && w.skill_runtime.marks.len() < MAX_MARKS
    {
        let shared::loadout::PassiveEffect::Radiance {
            mark_duration_secs,
            bonus_damage,
        } = shared::loadout::passive(PassiveId::Radiance)
        else {
            unreachable!()
        };
        let amount = w
            .players
            .values()
            .find(|p| p.hero.identity.id == src.entity.id)
            .map_or(bonus_damage, |p| {
                bonus_damage
                    * shared::hero_balance::ability_power_multiplier(
                        p.hero.identity.hero_class,
                        p.hero.progress.level,
                    )
                    * hero_stats::combat_bonuses(p).damage_multiplier
            });
        w.skill_runtime.marks.insert(
            (key(target), src.entity.id),
            Mark {
                expires: now + duration(mark_duration_secs),
                amount,
            },
        );
        if let Some(p) = w
            .players
            .values_mut()
            .find(|p| p.hero.identity.id == target.id && target.kind == TargetKind::Player)
        {
            p.hero.skills.marked_until = Some(now + duration(mark_duration_secs));
        }
    }
    if target.kind == TargetKind::Player {
        let marked_until = w
            .skill_runtime
            .marks
            .iter()
            .filter(|((t, _), m)| *t == key(target) && m.expires > now)
            .map(|(_, m)| m.expires)
            .max();
        if let Some(p) = w
            .players
            .values_mut()
            .find(|p| p.hero.identity.id == target.id)
        {
            p.hero.skills.marked_until = marked_until;
        }
    }
    if src.entity.kind == CombatEntityKind::Player
        && (!events.is_empty() || (shield_before > 0.0 && amount > 0.0))
    {
        let qualifies = target.kind == TargetKind::Player
            || target.kind == TargetKind::Structure
            || (target.kind == TargetKind::Neutral
                && w.neutrals
                    .get(&target.id)
                    .is_some_and(|n| n.state.camp_type.is_boss()));
        if qualifies {
            w.skill_runtime
                .contributions
                .entry(key(target))
                .or_default()
                .insert(src.entity.id, now);
        }
    }
    events
}
fn control(
    w: &mut GameWorld,
    c: Candidate,
    id: u64,
    team: Team,
    root: f32,
    slow: f32,
    secs: f32,
    reveal: f32,
    now: Instant,
) {
    crowd_control::apply(
        w,
        c,
        id,
        team,
        root,
        slow,
        secs,
        reveal,
        now,
        crowd_control::Kind::Root,
    );
}

fn shield(w: &mut GameWorld, target: u64, owner: u64, amount: f32, secs: f32, now: Instant) {
    if let Some(p) = w
        .players
        .values_mut()
        .find(|p| p.hero.identity.id == target && p.hero.hp > 0.0)
    {
        let shields = &mut p.hero.skills.shields;
        shields.retain(|s| s.expires > now);
        if let Some(s) = shields.iter_mut().find(|s| s.source == owner) {
            s.amount = (s.amount + amount).min(amount * 2.0);
            s.expires = now + duration(secs);
        } else if shields.len() < MAX_STATUS_SOURCES {
            shields.push(Shield {
                source: owner,
                amount,
                expires: now + duration(secs),
            });
        }
    }
}

pub fn cast(
    w: &mut GameWorld,
    addr: SocketAddr,
    slot: u8,
    aim: [f32; 2],
    request: u64,
    now: Instant,
) {
    let live_effect_count = advanced::active_count(w);
    let Some(p) = w.players.get_mut(&addr) else {
        return;
    };
    if !p.joined || request == 0 || request <= p.hero.skills.request_id {
        return;
    }
    p.hero.skills.request_id = request;
    p.last_seen = now;
    crate::recall::cancel(p);
    if !matches!(w.game_state, GameState::Running)
        || p.hero.hp <= 0.0
        || slot >= 4
        || !shared::loadout::valid_aim(aim)
    {
        return;
    }
    if remaining(p.hero.skills.control.stun_until, now) > 0.0 || p.hero.skills.advanced.immune(now)
    {
        return;
    }
    let Some(loadout) = &p.hero.skills.loadout else {
        return;
    };
    let id = loadout.skills()[slot as usize];
    let d = skill(id);
    let rank = p.hero.progress.ranks[slot as usize].clamp(1, d.ability.max_rank);
    if !p.modifiers.unlock_all && !loadout.unlocked(p.hero.progress.level)[slot as usize] {
        return;
    }
    if let SkillEffect::Technique { action, .. } = d.effect {
        if dagger::handles(action) {
            dagger::cast(w, addr, slot, aim, now);
            return;
        }
        advanced::cast(w, addr, slot, aim, now);
        return;
    }
    if p.hero
        .skills
        .advanced
        .charm
        .is_some_and(|(_, until)| until > now)
    {
        return;
    }
    if let Some((zone, expires, _)) = p.hero.skills.zones[slot as usize] {
        if now < expires
            && w.skill_runtime
                .effects
                .get(&zone)
                .is_some_and(|e| e.owner == p.hero.identity.id && e.skill == id)
        {
            p.hero.skills.zones[slot as usize] = None;
            crate::sim::cast::record_player_action(p, SkillSlot::from_index(slot).unwrap());
            if let Some(e) = w.skill_runtime.effects.remove(&zone) {
                let events = detonate(w, &e, now);
                w.skill_runtime.pending.extend(events);
            }
            return;
        }
    }
    if !p.modifiers.no_cooldowns && remaining(p.hero.skills.recovery_until, now) > 0.0 {
        return;
    }
    let cooldown = hero_stats::ability_cooldown(p, SkillSlot::from_index(slot).unwrap());
    if !p.modifiers.no_cooldowns
        && p.timers.last_cast_at[slot as usize]
            .is_some_and(|at| now.saturating_duration_since(at) < cooldown)
    {
        return;
    }
    let cost = scaled_mana_cost(&d.ability, rank);
    if !p.modifiers.infinite_resource && p.hero.mana < cost {
        return;
    }
    let origin = [p.hero.x, p.hero.z];
    let len = distance(origin, aim);
    let range = scaled_cast_range(&d.ability, rank);
    let is_toggle = matches!(d.effect, SkillEffect::WeaponToggle { .. });
    if !is_toggle
        && (!len.is_finite()
            || (len < 0.0001
                && !matches!(
                    d.effect,
                    SkillEffect::RecastZone { .. } | SkillEffect::TrapLine { .. }
                )))
    {
        return;
    }
    let dir = if len > 0.0001 {
        [(aim[0] - origin[0]) / len, (aim[1] - origin[1]) / len]
    } else {
        [0.0, 1.0]
    };
    let is_point = matches!(
        d.effect,
        SkillEffect::RecastZone { .. } | SkillEffect::TrapLine { .. }
    );
    // Client point previews clamp in f32; their rounded endpoint can land a
    // few ulps beyond our hypot result. Accept only that tiny margin, then
    // place the effect at the authoritative maximum range.
    let point_aim = if is_point {
        if len > range + 0.001 {
            return;
        }
        if len > range {
            add(origin, dir, range)
        } else {
            aim
        }
    } else {
        aim
    };
    let count = match d.effect {
        SkillEffect::TrapLine { count, .. } => count as usize,
        _ => usize::from(!is_toggle),
    };
    if live_effect_count + count > MAX_EFFECTS
        || w.skill_runtime
            .effects
            .values()
            .filter(|e| e.owner == p.hero.identity.id)
            .count()
            + count
            > MAX_OWNER_EFFECTS
    {
        return;
    }
    if !p.modifiers.infinite_resource {
        p.hero.mana -= cost;
    }
    p.timers.last_cast_at[slot as usize] = Some(now);
    p.hero.skills.recovery_until =
        Some(now + duration(d.windup_secs.max(if is_toggle { 0.0 } else { 0.15 })));
    crate::sim::cast::record_player_action(p, SkillSlot::from_index(slot).unwrap());
    if !is_toggle {
        crate::sim::cast::face_player_action(p, dir[0], dir[1]);
    }
    let owner = p.hero.identity.id;
    let team = p.hero.identity.team;
    let scale = shared::rank_effect_scale(rank)
        * shared::hero_balance::ability_power_multiplier(
            p.hero.identity.hero_class,
            p.hero.progress.level,
        )
        * hero_stats::combat_bonuses(p).damage_multiplier
        * w.team_buffs.damage_multiplier(team, now);
    if is_toggle {
        p.hero.skills.mode = if p.hero.skills.mode == WeaponMode::Repeater {
            WeaponMode::Rockets
        } else {
            WeaponMode::Repeater
        };
        return;
    }
    let end = if is_point {
        point_aim
    } else {
        add(origin, dir, range)
    };
    let mut effects = Vec::new();
    for n in 0..count {
        let mut pos = origin;
        let mut end = end;
        let mut lifetime = 20.0;
        let mut armed = d.windup_secs;
        match d.effect {
            SkillEffect::LinearProjectile { speed, .. }
            | SkillEffect::ImpactRocket { speed, .. } => lifetime = range / speed + 0.2,
            SkillEffect::ReturningShield { speed, .. } => lifetime = range / speed * 2.0 + 3.0,
            SkillEffect::RecastZone { duration_secs, .. } => {
                pos = end;
                lifetime = duration_secs;
            }
            SkillEffect::Beam { .. } => lifetime = d.windup_secs + 0.15,
            SkillEffect::TrapLine {
                spacing,
                arm_secs,
                duration_secs,
                ..
            } => {
                end = add(
                    end,
                    [-dir[1], dir[0]],
                    (n as f32 - (count as f32 - 1.0) * 0.5) * spacing,
                );
                pos = end;
                armed = arm_secs;
                lifetime = duration_secs;
            }
            _ => {}
        }
        let eid = w.skill_runtime.id();
        if matches!(d.effect, SkillEffect::RecastZone { .. }) {
            w.players.get_mut(&addr).unwrap().hero.skills.zones[slot as usize] =
                Some((eid, now + duration(lifetime), id));
        }
        effects.push(ActiveEffect {
            id: eid,
            owner,
            team,
            skill: id,
            slot,
            pos,
            origin,
            end,
            direction: dir,
            expires: now + duration(lifetime),
            armed_at: now + duration(armed),
            scale,
            traveled: 0.0,
            returning: false,
            hits: BTreeSet::new(),
            hit_count: 0,
            fired: false,
            cast_request: request,
        });
    }
    for e in effects {
        if let SkillEffect::ReturningShield {
            amount,
            duration_secs,
            ..
        } = d.effect
        {
            shield(w, owner, owner, amount * scale, duration_secs, now);
        }
        w.skill_runtime.effects.insert(e.id, e);
    }
}
fn detonate(w: &mut GameWorld, e: &ActiveEffect, now: Instant) -> Vec<CombatEvent> {
    let SkillEffect::RecastZone { radius, damage, .. } = skill(e.skill).effect else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for c in candidates(w)
        .into_iter()
        .filter(|c| hostile(c, e.team) && distance(c.pos, e.pos) <= radius + c.radius)
    {
        out.extend(apply_hit(
            w,
            c.target,
            damage * e.scale,
            skill(e.skill).damage_type,
            source(e.owner, e.slot),
            e.team,
            false,
            true,
            false,
            now,
        ));
    }
    if let Some(p) = w
        .players
        .values_mut()
        .find(|p| p.hero.identity.id == e.owner)
    {
        if p.hero.skills.zones[e.slot as usize].is_some_and(|(id, _, _)| id == e.id) {
            p.hero.skills.zones[e.slot as usize] = None;
        }
    }
    out
}

pub fn tick(w: &mut GameWorld, t: TickCtx) -> Vec<CombatEvent> {
    if !matches!(w.game_state, GameState::Running) || t.dt <= 0.0 {
        return Vec::new();
    }
    let now = t.now;
    let mut out = std::mem::take(&mut w.skill_runtime.pending);
    advanced::tick(w, t, &mut out);
    w.skill_runtime.marks.retain(|_, m| m.expires > now);
    let ids: Vec<_> = w.skill_runtime.effects.keys().copied().collect();
    for id in ids {
        let Some(mut e) = w.skill_runtime.effects.remove(&id) else {
            continue;
        };
        let def = skill(e.skill);
        let cs = candidates(w);
        let mut keep = now < e.expires;
        if !keep {
            if matches!(def.effect, SkillEffect::RecastZone { .. }) {
                out.extend(detonate(w, &e, now));
            }
            continue;
        }
        match def.effect {
            SkillEffect::Technique { .. } => keep = advanced::effect_tick(w, &mut e, t, &mut out),
            SkillEffect::LinearProjectile {
                speed,
                radius,
                damage,
                max_hits,
                root_secs,
                slow_multiplier,
                slow_secs,
                reveal_secs,
            } => {
                let travel = (speed * t.dt).min(distance(e.pos, e.end));
                let to = add(e.pos, e.direction, travel);
                if let Some((holder, factor)) =
                    advanced::intercept(w, e.team, e.pos, to, radius, now)
                {
                    out.extend(apply_hit(
                        w,
                        TargetId {
                            kind: TargetKind::Player,
                            id: holder,
                        },
                        damage * e.scale * factor,
                        def.damage_type,
                        source(e.owner, e.slot),
                        e.team,
                        false,
                        true,
                        false,
                        now,
                    ));
                    continue;
                }
                for c in hits(&cs, e.pos, to, radius)
                    .into_iter()
                    .filter(|c| hostile(c, e.team))
                {
                    if !e.hits.insert(key(c.target)) {
                        continue;
                    }
                    e.hit_count += 1;
                    out.extend(apply_hit(
                        w,
                        c.target,
                        damage * e.scale,
                        def.damage_type,
                        source(e.owner, e.slot),
                        e.team,
                        false,
                        true,
                        false,
                        now,
                    ));
                    control(
                        w,
                        c,
                        e.id,
                        e.team,
                        root_secs,
                        slow_multiplier,
                        slow_secs,
                        reveal_secs,
                        now,
                    );
                    if e.hit_count >= max_hits {
                        keep = false;
                        break;
                    }
                }
                e.pos = to;
                keep &= distance(e.pos, e.end) > 0.001;
            }
            SkillEffect::ReturningShield {
                speed,
                radius,
                amount,
                duration_secs,
            } => {
                if e.returning {
                    if let Some(p) = w
                        .players
                        .values()
                        .find(|p| p.joined && p.hero.identity.id == e.owner && p.hero.hp > 0.0)
                    {
                        e.end = [p.hero.x, p.hero.z];
                    } else {
                        continue;
                    }
                }
                let len = distance(e.pos, e.end);
                let dir = if len > 0.001 {
                    [(e.end[0] - e.pos[0]) / len, (e.end[1] - e.pos[1]) / len]
                } else {
                    [0.0, 0.0]
                };
                let to = add(e.pos, dir, (speed * t.dt).min(len));
                for c in hits(&cs, e.pos, to, radius)
                    .into_iter()
                    .filter(|c| c.target.kind == TargetKind::Player && c.team == Some(e.team))
                {
                    if c.target.id == e.owner && !e.returning {
                        continue;
                    }
                    if e.hits.insert(key(c.target)) {
                        shield(
                            w,
                            c.target.id,
                            e.owner,
                            amount * e.scale,
                            duration_secs,
                            now,
                        );
                    }
                }
                e.pos = to;
                e.direction = dir;
                if len <= speed * t.dt + 0.001 {
                    if e.returning {
                        keep = false;
                    } else {
                        e.returning = true;
                        e.hits.clear();
                    }
                }
            }
            SkillEffect::RecastZone {
                radius,
                slow_multiplier,
                ..
            } => {
                for c in cs
                    .into_iter()
                    .filter(|c| hostile(c, e.team) && distance(c.pos, e.pos) <= radius + c.radius)
                {
                    control(w, c, e.id, e.team, 0.0, slow_multiplier, 0.12, 0.0, now);
                }
            }
            SkillEffect::Beam { width, damage } => {
                if !e.fired
                    && !w
                        .players
                        .values()
                        .any(|p| p.joined && p.hero.identity.id == e.owner && p.hero.hp > 0.0)
                {
                    continue;
                }
                if now >= e.armed_at && !e.fired {
                    for c in hits(&cs, e.origin, e.end, width)
                        .into_iter()
                        .filter(|c| hostile(c, e.team))
                    {
                        out.extend(apply_hit(
                            w,
                            c.target,
                            damage * e.scale,
                            def.damage_type,
                            source(e.owner, e.slot),
                            e.team,
                            false,
                            true,
                            true,
                            now,
                        ));
                    }
                    e.fired = true;
                }
            }
            SkillEffect::TrapLine {
                radius,
                damage,
                root_secs,
                ..
            } => {
                if now >= e.armed_at {
                    if let Some(c) = cs
                        .into_iter()
                        .filter(|c| {
                            c.target.kind == TargetKind::Player
                                && hostile(c, e.team)
                                && distance(c.pos, e.pos) <= radius + c.radius
                                && !w.skill_runtime.trap_hits.contains_key(&(
                                    e.owner,
                                    e.cast_request,
                                    c.target.id,
                                ))
                        })
                        .min_by(|a, b| {
                            distance(a.pos, e.pos)
                                .total_cmp(&distance(b.pos, e.pos))
                                .then_with(|| a.target.id.cmp(&b.target.id))
                        })
                    {
                        // A target cannot trigger sibling traps of the same cast in this tick.
                        if !w.skill_runtime.trap_hits.contains_key(&(
                            e.owner,
                            e.cast_request,
                            c.target.id,
                        )) {
                            let mut receipts = apply_hit(
                                w,
                                c.target,
                                damage * e.scale,
                                def.damage_type,
                                source(e.owner, e.slot),
                                e.team,
                                false,
                                true,
                                false,
                                now,
                            );
                            // Activation is independent of HP loss: shields may
                            // absorb the damage while the trap still roots and
                            // disappears. Mark exactly one receipt for this victim.
                            if let Some(receipt) = receipts.iter_mut().find(|receipt| {
                                receipt.target.kind == CombatEntityKind::Player
                                    && receipt.target.id == c.target.id
                            }) {
                                receipt.trap_triggered = true;
                            } else {
                                let target = w
                                    .players
                                    .values()
                                    .find(|p| p.hero.identity.id == c.target.id)
                                    .expect("trap candidate must still exist");
                                receipts.push(source(e.owner, e.slot).annotate(CombatEvent {
                                    target: shared::combat::CombatEntity {
                                        kind: CombatEntityKind::Player,
                                        id: c.target.id,
                                    },
                                    x: target.hero.x,
                                    y: target.hero.y + crate::balance::AIM_HEIGHT,
                                    z: target.hero.z,
                                    trap_triggered: true,
                                    ..Default::default()
                                }));
                            }
                            out.extend(receipts);
                            control(w, c, e.id, e.team, root_secs, 1.0, 0.0, 0.0, now);
                            w.skill_runtime
                                .trap_hits
                                .insert((e.owner, e.cast_request, c.target.id), e.expires);
                            keep = false;
                        }
                    }
                }
            }
            SkillEffect::ImpactRocket {
                speed,
                radius,
                blast_radius,
                damage,
                min_damage_multiplier,
                max_distance,
                missing_health_ratio,
            } => {
                let travel = (speed * t.dt).min(distance(e.pos, e.end));
                let to = add(e.pos, e.direction, travel);
                if let Some((holder, factor)) =
                    advanced::intercept(w, e.team, e.pos, to, radius, now)
                {
                    out.extend(apply_hit(
                        w,
                        TargetId {
                            kind: TargetKind::Player,
                            id: holder,
                        },
                        damage * e.scale * factor,
                        def.damage_type,
                        source(e.owner, e.slot),
                        e.team,
                        false,
                        true,
                        false,
                        now,
                    ));
                    continue;
                }
                if let Some(c) = hits(&cs, e.pos, to, radius)
                    .into_iter()
                    .find(|c| c.target.kind == TargetKind::Player && hostile(c, e.team))
                {
                    let impact_t = intersection(e.pos, to, c.pos, radius + c.radius).unwrap_or(0.0);
                    e.pos = add(e.pos, e.direction, travel * impact_t);
                    e.traveled += travel * impact_t;
                    let factor = min_damage_multiplier
                        + (1.0 - min_damage_multiplier)
                            * (e.traveled / max_distance).clamp(0.0, 1.0);
                    for victim in cs.into_iter().filter(|c| {
                        hostile(c, e.team) && distance(c.pos, e.pos) <= blast_radius + c.radius
                    }) {
                        let amount = damage * e.scale * factor
                            + (victim.max_hp - victim.hp).max(0.0) * missing_health_ratio;
                        out.extend(apply_hit(
                            w,
                            victim.target,
                            amount,
                            def.damage_type,
                            source(e.owner, e.slot),
                            e.team,
                            false,
                            true,
                            false,
                            now,
                        ));
                    }
                    keep = false;
                } else {
                    e.pos = to;
                    e.traveled += travel;
                    keep &= distance(e.pos, e.end) > 0.001;
                }
            }
            SkillEffect::WeaponToggle { .. } => keep = false,
        }
        if keep {
            w.skill_runtime.effects.insert(e.id, e);
        }
    }
    out
}

pub fn effects(w: &GameWorld, now: Instant) -> Vec<SkillEffectState> {
    let mut result: Vec<_> = w
        .skill_runtime
        .effects
        .values()
        .map(|e| {
            let (kind, radius) = match skill(e.skill).effect {
                SkillEffect::Technique { action, radius, .. } => (
                    match action {
                        shared::loadout::Technique::TerrainLine => EffectVisualKind::Trap,
                        shared::loadout::Technique::Lantern => EffectVisualKind::Lantern,
                        shared::loadout::Technique::SegmentCage => EffectVisualKind::Cage,
                        shared::loadout::Technique::BallField => EffectVisualKind::Field,
                        shared::loadout::Technique::InterceptShield => EffectVisualKind::ShieldWall,
                        shared::loadout::Technique::Parry => EffectVisualKind::Barrier,
                        shared::loadout::Technique::PiercingWave if now < e.armed_at => {
                            EffectVisualKind::BeamWarning
                        }
                        shared::loadout::Technique::GlacialFissure
                        | shared::loadout::Technique::ConeBrittle
                        | shared::loadout::Technique::BallPull => EffectVisualKind::BeamWarning,
                        _ => EffectVisualKind::Bolt,
                    },
                    radius,
                ),
                SkillEffect::LinearProjectile { radius, .. } => (EffectVisualKind::Bolt, radius),
                SkillEffect::ReturningShield { radius, .. } => (EffectVisualKind::Barrier, radius),
                SkillEffect::RecastZone { radius, .. } => (EffectVisualKind::Field, radius),
                SkillEffect::Beam { width, .. } => (
                    if e.fired {
                        EffectVisualKind::Beam
                    } else {
                        EffectVisualKind::BeamWarning
                    },
                    width,
                ),
                SkillEffect::TrapLine { radius, .. } => (EffectVisualKind::Trap, radius),
                SkillEffect::ImpactRocket { radius, .. } => (EffectVisualKind::Rocket, radius),
                _ => (EffectVisualKind::Bolt, 0.0),
            };
            SkillEffectState {
                id: e.id,
                owner_id: e.owner,
                owner_team: e.team,
                skill: e.skill,
                kind,
                position: e.pos,
                end: if matches!(
                    kind,
                    EffectVisualKind::Bolt | EffectVisualKind::Barrier | EffectVisualKind::Rocket
                ) {
                    add(e.pos, e.direction, 1.0)
                } else {
                    e.end
                },
                radius,
                remaining_secs: remaining(Some(e.expires), now),
                armed: now >= e.armed_at,
                consumed_segments: if kind == EffectVisualKind::Cage {
                    e.hit_count
                } else {
                    0
                },
            }
        })
        .collect();
    result.extend(advanced::visuals(w, now));
    result.truncate(shared::loadout::MAX_ACTIVE_EFFECTS);
    result
}
/// Called exactly once for each accepted receipt stream, before cosmetic retention.
pub fn observe(w: &mut GameWorld, events: &[CombatEvent], now: Instant) {
    advanced::observe(w, events, now);
    for event in events {
        let kind = match event.target.kind {
            CombatEntityKind::Player => TargetKind::Player,
            CombatEntityKind::Minion => TargetKind::Minion,
            CombatEntityKind::Neutral => TargetKind::Neutral,
            CombatEntityKind::Structure => TargetKind::Structure,
            _ => continue,
        };
        let target = TargetId {
            kind,
            id: event.target.id,
        };
        let qualifies = match kind {
            TargetKind::Player | TargetKind::Structure => true,
            TargetKind::Neutral => w
                .neutrals
                .get(&target.id)
                .is_some_and(|n| n.state.camp_type.is_boss()),
            _ => false,
        };
        if qualifies {
            if event.killed && !w.skill_runtime.seen_deaths.insert(key(target)) {
                continue;
            }
            let entries = w
                .skill_runtime
                .contributions
                .entry(key(target))
                .or_default();
            entries.retain(|_, at| now.saturating_duration_since(*at) <= participation_window());
            if event.source.kind == CombatEntityKind::Player && event.amount > 0.0 {
                entries.insert(event.source.id, now);
            }
            if event.killed {
                let participants = w
                    .skill_runtime
                    .contributions
                    .remove(&key(target))
                    .unwrap_or_default();
                crate::shop::award_hero_kill(
                    w,
                    event,
                    &participants.keys().copied().collect::<Vec<_>>(),
                );
                for p in w
                    .players
                    .values_mut()
                    .filter(|p| p.joined && p.hero.hp > 0.0)
                {
                    if participants.contains_key(&p.hero.identity.id)
                        && p.hero
                            .skills
                            .loadout
                            .as_ref()
                            .is_some_and(|l| l.passive() == PassiveId::Momentum)
                    {
                        p.hero.skills.momentum_until =
                            Some(now + duration(momentum_parameters(&p.hero.skills).0));
                    }
                }
            }
        }
        if event.killed {
            w.skill_runtime.marks.retain(|(t, _), _| *t != key(target));
            w.skill_runtime.npc_controls.remove(&key(target));
        }
    }
    w.skill_runtime.contributions.retain(|_, entries| {
        entries.retain(|_, at| now.saturating_duration_since(*at) <= participation_window());
        !entries.is_empty()
    });
}
pub fn normalize(w: &mut GameWorld, now: Instant) {
    for p in w.players.values_mut() {
        if p.hero.hp <= 0.0 {
            p.hero.skills.transient_reset();
        }
        p.hero
            .skills
            .shields
            .retain(|s| s.expires > now && s.amount > 0.0);
        p.hero.skills.control.slows.retain(|s| s.expires > now);
    }
    let alive: BTreeSet<_> = candidates(w).into_iter().map(|c| key(c.target)).collect();
    w.skill_runtime
        .seen_deaths
        .retain(|key| !alive.contains(key));
    w.skill_runtime.trap_hits.retain(|_, until| *until > now);
    w.skill_runtime.npc_controls.retain(|key, s| {
        s.slows.retain(|s| s.expires > now);
        alive.contains(key) && (s.movement(now) != 1.0 || remaining(s.reveal_until, now) > 0.0)
    });
    w.skill_runtime
        .marks
        .retain(|(key, _), m| alive.contains(key) && m.expires > now);
    w.skill_runtime
        .attack_splash
        .retain(|id, _| w.projectiles.contains_key(id));
}
pub fn basic_impact(
    w: &mut GameWorld,
    target: TargetId,
    damage: f32,
    src: HitSource,
    team: Team,
    projectile: u64,
    now: Instant,
) -> Vec<CombatEvent> {
    let splash = w
        .skill_runtime
        .attack_splash
        .remove(&projectile)
        .unwrap_or(0.0);
    let center = candidates(w)
        .into_iter()
        .find(|c| c.target == target)
        .map(|c| c.pos);
    let mut events = apply_hit(
        w,
        target,
        damage,
        DamageType::Physical,
        src,
        team,
        true,
        false,
        false,
        now,
    );
    if let Some(center) = center.filter(|_| splash > 0.0) {
        for c in candidates(w).into_iter().filter(|c| {
            c.target != target && hostile(c, team) && distance(c.pos, center) <= splash + c.radius
        }) {
            events.extend(apply_hit(
                w,
                c.target,
                damage,
                DamageType::Physical,
                src,
                team,
                false,
                false,
                false,
                now,
            ));
        }
    }
    events
}

#[cfg(test)]
mod tests;

/// A sandbox actor edit starts a fresh local combat incarnation, not a new
/// network request sequence. Ordinary death intentionally preserves in-flight effects.
pub fn clear_actor(w: &mut GameWorld, id: u64) {
    advanced::clear_actor(w, id);
    w.skill_runtime.effects.retain(|_, e| e.owner != id);
    w.skill_runtime
        .marks
        .retain(|(target, owner), _| *owner != id && *target != (0, id));
    w.skill_runtime.contributions.remove(&(0, id));
    for entries in w.skill_runtime.contributions.values_mut() {
        entries.remove(&id);
    }
    w.skill_runtime.seen_deaths.remove(&(0, id));
}
