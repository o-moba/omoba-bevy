//! What the client remembers of each replicated skill effect between two snapshots, for both
//! render backends: its stage, its transitions and how it ended. An effect that leaves the
//! snapshot ended silently unless what was last seen of it proves that it ran out, released
//! or detonated; a hit, an intercept, a cancel, fog and a lost snapshot all look the same
//! and none of them is drawn. A travelling body and an auxiliary object always end silently.
use super::SkillPresentation;
use super::cast::{CastKey, SkillCastObserved};
use super::category::{self, SkillKey};
use super::vocab::{MotionPhase, StageRule};
use crate::combat::CombatStats;
use crate::net::{GameState, GameStateSnapshot, NetworkHeroClass, NetworkPlayerId, PlayerLoadout};
use crate::player::Player;
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use shared::loadout::{EffectVisualKind, LoadoutState, SkillEffectState, SkillId};
use shared::{HeroClass, SkillSlot};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// A release is inferred only for a telegraph last seen this close to its firing moment:
/// one and a half snapshot intervals.
pub(crate) const RELEASE_SLACK_SECS: f32 = 0.075;
/// An effect ran out when it was last seen with at most this much time left: two snapshot
/// intervals and a margin.
pub(crate) const EXPIRY_SECS: f32 = 0.11;
/// Two sightings farther apart than this are not consecutive: what happened between them
/// was not observed and is not drawn late.
pub(crate) const MAX_GAP_SECS: f64 = 0.25;
/// A heading that changes by more than this between two snapshots is a turn.
pub(crate) const TURN_DEGREES: f32 = 45.0;
/// A remaining time that rises by more than this between two snapshots is a renewal.
pub(crate) const RENEW_SECS: f32 = 0.25;
/// A step longer than this between two snapshots is a relocation, not travel.
pub(crate) const JUMP_UNITS: f32 = 6.0;
/// Earlier positions kept for one instance.
pub(crate) const HISTORY: usize = 6;
/// A telegraph this far into its time at most was cast just now (rule E-16).
pub(crate) const FRESH_SECS: f32 = 0.15;
/// A step shorter than this is no travel; the position is not recorded again.
const REST_UNITS: f32 = 1e-3;
/// An instance that has not moved for this long is at rest: more than two snapshot
/// intervals, so that one repeated position is not a rest.
pub(crate) const REST_SECS: f64 = 0.12;

/// How one instance is followed from snapshot to snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectKey {
    /// The replicated id of the effect.
    Runtime(u64),
    /// An auxiliary object. Its replicated id is positional, so it is known only as the
    /// n-th object of its kind and owner (`common/src/skills/advanced.rs:2347`).
    Aux {
        owner: u64,
        kind: EffectVisualKind,
        ordinal: u8,
    },
}

impl Hash for EffectKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match *self {
            Self::Runtime(id) => (0u8, id).hash(state),
            Self::Aux {
                owner,
                kind,
                ordinal,
            } => (1u8, owner, kind as u8, ordinal).hash(state),
        }
    }
}

/// The well-formed effects of a snapshot with their keys, in snapshot order.
pub(crate) fn keyed(effects: &[SkillEffectState]) -> Vec<(EffectKey, &SkillEffectState)> {
    let mut counted: Vec<(u64, EffectVisualKind, u8)> = Vec::new();
    effects
        .iter()
        .take(shared::loadout::MAX_ACTIVE_EFFECTS)
        .filter(|effect| super::effects::valid_effect(effect))
        .map(|effect| {
            if !category::unstable_id(effect.kind) {
                return (EffectKey::Runtime(effect.id), effect);
            }
            let ordinal = match counted
                .iter_mut()
                .find(|(owner, kind, _)| (*owner, *kind) == (effect.owner_id, effect.kind))
            {
                Some((_, _, next)) => {
                    *next = next.saturating_add(1);
                    *next
                }
                None => {
                    counted.push((effect.owner_id, effect.kind, 0));
                    0
                }
            };
            (
                EffectKey::Aux {
                    owner: effect.owner_id,
                    kind: effect.kind,
                    ordinal,
                },
                effect,
            )
        })
        .collect()
}

/// What the client holds of one instance.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Memory {
    /// The effect as the newest snapshot that held it showed it.
    pub last: SkillEffectState,
    /// The presentation clock when that snapshot was taken in.
    pub last_seen_secs: f64,
    /// Positions observed before the newest one, newest first. A turn, a relocation and a
    /// gap in the sightings clear them, so nothing is ever laid on a path that was not seen.
    pub history: [Vec2; HISTORY],
    pub history_len: u8,
    /// A renewal of this very instance was observed.
    pub renewed: bool,
    /// Seconds of consecutive sightings since the instance last moved.
    pub still_secs: f64,
    /// The longest remaining time any sighting of this instance showed.
    pub peak_remaining_secs: f32,
}

impl Memory {
    fn first(effect: &SkillEffectState, now: f64) -> Self {
        Self {
            last: effect.clone(),
            last_seen_secs: now,
            history: [Vec2::ZERO; HISTORY],
            history_len: 0,
            renewed: false,
            still_secs: 0.0,
            peak_remaining_secs: effect.remaining_secs,
        }
    }

    /// The instance has stood still for longer than two snapshots are apart. A body at rest
    /// draws no trail, although the positions it came through are still remembered. A
    /// paused sandbox repeats one moment and makes nothing rest.
    pub(crate) fn resting(&self) -> bool {
        self.still_secs > REST_SECS
    }

    /// The earlier positions, newest first.
    pub(crate) fn trail(&self) -> &[Vec2] {
        &self.history[..usize::from(self.history_len)]
    }

    /// Takes in the next consecutive sighting, `elapsed` seconds after the one before it,
    /// and what changed with it.
    fn follow(&mut self, effect: &SkillEffectState, changes: &[Transition], elapsed: f64) {
        let from = Vec2::from_array(self.last.position);
        let step = from.distance(Vec2::from_array(effect.position));
        if changes.contains(&Transition::Turned) || step > JUMP_UNITS {
            self.history_len = 0;
        } else if step > REST_UNITS {
            self.history.copy_within(..HISTORY - 1, 1);
            self.history[0] = from;
            self.history_len = (self.history_len + 1).min(HISTORY as u8);
        }
        if step > REST_UNITS {
            self.still_secs = 0.0;
        } else {
            self.still_secs += elapsed;
        }
        self.renewed |= changes.contains(&Transition::Renewed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Announced; nothing is hit yet.
    Telegraph,
    Active,
}

/// The stage of one effect, from one snapshot alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StageView {
    pub stage: Stage,
    /// How far the telegraph has run, `0..=1`. It is 1 exactly when the server fires, also
    /// for an effect first seen in the middle of its telegraph.
    pub progress: f32,
    /// The replicated remaining time, never negative.
    pub remaining: f32,
}

pub(crate) fn stage_view(rule: StageRule, skill: SkillId, effect: &SkillEffectState) -> StageView {
    let remaining = if effect.remaining_secs.is_finite() {
        effect.remaining_secs.max(0.0)
    } else {
        0.0
    };
    // `remaining_secs` still reads the tail when the telegraph is over.
    let timed = || {
        category::telegraph_secs(skill)
            .zip(category::tail_secs(skill))
            .filter(|(telegraph, _)| *telegraph > 0.0)
            .map_or(0.0, |(telegraph, tail)| {
                (1.0 - (remaining - tail) / telegraph).clamp(0.0, 1.0)
            })
    };
    let (stage, progress) = match rule {
        StageRule::Active => (Stage::Active, 1.0),
        StageRule::ArmedGate if effect.armed => (Stage::Active, 1.0),
        StageRule::ArmedGate => (Stage::Telegraph, 0.0),
        StageRule::KindGate if effect.kind == EffectVisualKind::BeamWarning => {
            (Stage::Telegraph, timed())
        }
        StageRule::KindGate => (Stage::Active, 1.0),
        StageRule::Fuse => (Stage::Telegraph, timed()),
    };
    StageView {
        stage,
        progress,
        remaining,
    }
}

/// The stage of an effect under the rule its skill and kind derive.
pub(crate) fn view(effect: &SkillEffectState) -> StageView {
    stage_view(
        category::stage_rule(effect.skill, effect.kind),
        effect.skill,
        effect,
    )
}

/// A change of one instance between two consecutive snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transition {
    /// The replicated `armed` flag was set.
    Armed,
    /// A warning became the beam or the bolt it announced, on the same id.
    KindFlipped,
    /// The cage lost the side with this index.
    SegmentBroken(u8),
    /// The heading changed by more than 45 degrees.
    Turned,
    /// The remaining time rose by more than 0.25 s.
    Renewed,
}

/// The replicated heading of a kind that carries one.
fn heading(effect: &SkillEffectState) -> Option<Vec2> {
    category::heading_only(effect.kind)
        .then(|| (Vec2::from_array(effect.end) - Vec2::from_array(effect.position)).try_normalize())
        .flatten()
}

/// What changed between two consecutive sightings of one instance. An auxiliary object has
/// no transitions: its key does not prove that both sightings are the same object.
pub(crate) fn transitions(
    rule: StageRule,
    before: &SkillEffectState,
    now: &SkillEffectState,
) -> Vec<Transition> {
    use EffectVisualKind as K;
    let mut changes = Vec::new();
    if before.skill != now.skill
        || category::unstable_id(before.kind)
        || category::unstable_id(now.kind)
    {
        return changes;
    }
    if rule == StageRule::ArmedGate && !before.armed && now.armed {
        changes.push(Transition::Armed);
    }
    if rule == StageRule::KindGate
        && before.kind == K::BeamWarning
        && matches!(now.kind, K::Beam | K::Bolt)
    {
        changes.push(Transition::KindFlipped);
    }
    if before.kind == K::Cage && now.kind == K::Cage {
        let broken = now.consumed_segments & !before.consumed_segments;
        changes.extend(
            (0..5)
                .filter(|side| broken & (1 << side) != 0)
                .map(Transition::SegmentBroken),
        );
    }
    if heading(before)
        .zip(heading(now))
        .is_some_and(|(was, is)| was.dot(is) < TURN_DEGREES.to_radians().cos())
    {
        changes.push(Transition::Turned);
    }
    if now.remaining_secs - before.remaining_secs > RENEW_SECS {
        changes.push(Transition::Renewed);
    }
    changes
}

/// How an effect that left the snapshot ended, as far as the client can prove it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndKind {
    /// Not known: nothing is drawn.
    Silent,
    /// Its time ran out.
    TrueExpiry,
    /// The telegraph fired.
    Released,
    /// The zone burst, on a recast or when its time ran out.
    Detonated,
}

/// The owner of an effect as the client holds it in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OwnerSeen {
    pub visible: bool,
    pub alive: bool,
    /// The replicated parry flag; `None` for a hero without a loadout state.
    pub parrying: Option<bool>,
    pub position: Vec3,
    /// The slot that holds the skill of the effect in the owner's accepted kit.
    pub slot: Option<u8>,
    pub local: bool,
}

/// What the snapshot that dropped an effect shows around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EndContext {
    /// Seconds from the last snapshot that held the effect to the one that dropped it.
    pub gap_secs: f64,
    /// The owner in the dropping snapshot; `None` when the client holds no such hero.
    pub owner: Option<OwnerSeen>,
    /// The owner recast the skill of the effect in the dropping snapshot.
    pub recast_edge: bool,
}

/// Classifies the end of an effect from its last sighting. Everything that is not proven
/// is `Silent`.
pub(crate) fn classify_end(
    rule: StageRule,
    skill: SkillId,
    last: &Memory,
    ctx: &EndContext,
) -> EndKind {
    let effect = &last.last;
    if !(0.0..=MAX_GAP_SECS).contains(&ctx.gap_secs)
        || category::unstable_id(effect.kind)
        || category::travels(skill, effect.kind)
    {
        return EndKind::Silent;
    }
    let owner = ctx.owner.filter(|_| effect.owner_id != 0);
    if rule == StageRule::Fuse {
        // The firing tick removes the telegraph while `remaining_secs` still reads the
        // tail, so only an end at the tail is the release.
        let at_the_tail = category::tail_secs(skill)
            .is_some_and(|tail| effect.remaining_secs <= tail + RELEASE_SLACK_SECS);
        let parry = category::derived_phase(skill) == MotionPhase::Parry;
        let fired = owner.is_some_and(|owner| {
            owner.visible && owner.alive && (!parry || owner.parrying == Some(false))
        });
        return if at_the_tail && fired {
            EndKind::Released
        } else {
            EndKind::Silent
        };
    }
    // An effect of an owner the client does not see may still run out; one whose owner is
    // known must have a living owner, because death removes what a hero left behind.
    let expired = effect.remaining_secs <= EXPIRY_SECS
        && (effect.owner_id == 0 || owner.is_some_and(|owner| owner.alive))
        // A warning that vanishes never fired; only the beam it became fades.
        && (rule != StageRule::KindGate || effect.kind == EffectVisualKind::Beam);
    if category::detonates(skill) && effect.kind == EffectVisualKind::Field {
        // The server bursts the zone on its owner's recast and when its time runs out.
        let recast = ctx.recast_edge && owner.is_some_and(|owner| owner.visible && owner.alive);
        return if expired || recast {
            EndKind::Detonated
        } else {
            EndKind::Silent
        };
    }
    if expired {
        EndKind::TrueExpiry
    } else {
        EndKind::Silent
    }
}

/// What happened to one instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageChange {
    Transition(Transition),
    /// The effect left the snapshot in a way the client can name; never `Silent`.
    Ended(EndKind),
}

/// A transition or a classified end of an effect that was in the previous snapshot. Only an
/// effect with a replicated id of its own has events.
#[derive(Message, Clone, Debug, PartialEq)]
pub(crate) struct StageEvent {
    /// The effect as the newest snapshot holds it; for an end, as it was last seen.
    pub effect: SkillEffectState,
    pub change: StageChange,
    /// The owner as this frame shows it; `None` when the client holds no such hero.
    pub owner: Option<OwnerSeen>,
}

/// A hero as the client holds it in the frame that takes a snapshot in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeroSeen<'a> {
    pub id: u64,
    /// The entity is drawn: it exists and nothing hides it.
    pub visible: bool,
    pub alive: bool,
    pub position: Vec3,
    pub forward: Vec3,
    pub class: HeroClass,
    pub loadout: Option<&'a LoadoutState>,
    pub local: bool,
}

impl HeroSeen<'_> {
    fn slot_of(&self, skill: SkillId) -> Option<u8> {
        let equipped = crate::equipped_skills::resolve_state(self.class, self.loadout)?;
        SkillSlot::ALL
            .into_iter()
            .position(|slot| equipped.skill(slot).is_some_and(|def| def.id == skill))
            .map(|slot| slot as u8)
    }

    fn owner_of(&self, skill: SkillId) -> OwnerSeen {
        OwnerSeen {
            visible: self.visible,
            alive: self.alive,
            parrying: self.loadout.map(|loadout| loadout.parrying),
            position: self.position,
            slot: self.slot_of(skill),
            local: self.local,
        }
    }
}

/// One snapshot and what the frame that applied it knows around it.
pub(crate) struct Frame<'a> {
    pub round: Option<(u64, u64)>,
    /// The presentation clock.
    pub now: f64,
    pub running: bool,
    pub effects: &'a [SkillEffectState],
    pub heroes: &'a [HeroSeen<'a>],
    /// The accepted actions the cast observer reported in this frame.
    pub casts: &'a [SkillCastObserved],
    pub registry: Option<&'a SkillPresentation>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Taken {
    pub events: Vec<StageEvent>,
    /// Casts known only from their own fresh telegraph (rule E-16).
    pub casts: Vec<SkillCastObserved>,
}

/// The memory of every effect of the newest snapshot. A new round starts it empty.
#[derive(Resource, Default)]
pub(crate) struct EffectMemory {
    round: Option<(u64, u64)>,
    /// When the newest snapshot was taken in.
    taken_secs: Option<f64>,
    seen: HashMap<EffectKey, Memory>,
    /// Heroes that were visible when the newest snapshot was taken in.
    visible: HashSet<u64>,
    /// First casts the observer reported a moment ago: hero, skill and when.
    edges: Vec<(u64, SkillId, f64)>,
}

impl EffectMemory {
    pub(crate) fn get(&self, key: EffectKey) -> Option<&Memory> {
        self.seen.get(&key)
    }

    /// Takes one snapshot in: remembers every effect it holds, reports the transitions of
    /// those the previous snapshot held as well, and classifies the end of those it no
    /// longer holds. Effects of another round are forgotten without a word.
    pub(crate) fn take(&mut self, frame: &Frame) -> Taken {
        let now = frame.now;
        if self.round != frame.round {
            *self = Self {
                round: frame.round,
                ..default()
            };
        }
        // An appearance is an event of this snapshot only when the one before it is known.
        let consecutive = self
            .taken_secs
            .is_some_and(|taken| (0.0..=MAX_GAP_SECS).contains(&(now - taken)));
        let owner_of = |effect: &SkillEffectState| {
            frame
                .heroes
                .iter()
                .find(|hero| effect.owner_id != 0 && hero.id == effect.owner_id)
                .map(|hero| hero.owner_of(effect.skill))
        };
        let recast_of = |effect: &SkillEffectState| {
            frame.casts.iter().any(|cast| {
                effect.owner_id != 0
                    && cast.recast
                    && cast.actor_id == effect.owner_id
                    && cast.key == CastKey::Skill(SkillKey::Modular(effect.skill))
            })
        };
        // An edge accounts for a warning that reaches the client with it or just after it.
        self.edges
            .retain(|(.., seen)| (0.0..=MAX_GAP_SECS).contains(&(now - seen)));
        self.edges.extend(
            frame
                .casts
                .iter()
                .filter(|cast| !cast.recast)
                .filter_map(|cast| match cast.key {
                    CastKey::Skill(SkillKey::Modular(skill)) => Some((cast.actor_id, skill, now)),
                    CastKey::Skill(SkillKey::Legacy(..)) | CastKey::Basic(_) => None,
                }),
        );
        let current = keyed(frame.effects);
        let mut taken = Taken::default();

        let held: HashSet<EffectKey> = current.iter().map(|(key, _)| *key).collect();
        let mut gone = Vec::new();
        self.seen.retain(|key, memory| {
            let stays = held.contains(key);
            if !stays {
                gone.push(memory.clone());
            }
            stays
        });
        gone.sort_by_key(|memory| memory.last.id);
        for memory in gone {
            let effect = &memory.last;
            let owner = owner_of(effect);
            let kind = classify_end(
                category::stage_rule(effect.skill, effect.kind),
                effect.skill,
                &memory,
                &EndContext {
                    gap_secs: now - memory.last_seen_secs,
                    owner,
                    recast_edge: recast_of(effect),
                },
            );
            if kind != EndKind::Silent {
                taken.events.push(StageEvent {
                    effect: memory.last,
                    change: StageChange::Ended(kind),
                    owner,
                });
            }
        }

        for (key, effect) in current {
            let Some(memory) = self.seen.get_mut(&key) else {
                if consecutive
                    && frame.running
                    && let Some(cast) = self.warned_cast(key, effect, frame)
                {
                    taken.casts.push(cast);
                }
                self.seen.insert(key, Memory::first(effect, now));
                continue;
            };
            let elapsed = now - memory.last_seen_secs;
            if (0.0..=MAX_GAP_SECS).contains(&elapsed) {
                let rule = category::stage_rule(effect.skill, effect.kind);
                let changes = transitions(rule, &memory.last, effect);
                memory.follow(effect, &changes, elapsed);
                let owner = owner_of(effect);
                taken
                    .events
                    .extend(changes.into_iter().map(|change| StageEvent {
                        effect: effect.clone(),
                        change: StageChange::Transition(change),
                        owner,
                    }));
            } else {
                // What the effect did while it was not seen is not known.
                memory.history_len = 0;
                memory.still_secs = 0.0;
            }
            memory.peak_remaining_secs = memory.peak_remaining_secs.max(effect.remaining_secs);
            memory.last = effect.clone();
            memory.last_seen_secs = now;
        }

        self.taken_secs = Some(now);
        self.visible = frame
            .heroes
            .iter()
            .filter(|hero| hero.visible)
            .map(|hero| hero.id)
            .collect();
        taken
    }

    /// Rule E-16. The cast edge of a skill that warns before it fires is often hidden by a
    /// later action of the same snapshot. Its own fresh warning proves the cast, so the
    /// first sight of that warning stands in for the edge: once per effect, for a living
    /// hero that was visible before and is visible now, and not beside the edge itself.
    fn warned_cast(
        &self,
        key: EffectKey,
        effect: &SkillEffectState,
        frame: &Frame,
    ) -> Option<SkillCastObserved> {
        let skill = effect.skill;
        let row = SkillKey::Modular(skill);
        let charged = frame.registry?.profile(skill).is_some_and(|profile| {
            profile.windup.is_some() && profile.phase(row) == MotionPhase::WarnFire
        });
        let stage = view(effect);
        let fresh = stage.stage == Stage::Telegraph
            && category::telegraph_secs(skill)
                .is_some_and(|telegraph| stage.progress * telegraph <= FRESH_SECS);
        let edge_seen = self
            .edges
            .iter()
            .any(|(actor, cast, _)| (*actor, *cast) == (effect.owner_id, skill));
        if !(charged && fresh && matches!(key, EffectKey::Runtime(_)))
            || effect.kind != EffectVisualKind::BeamWarning
            || edge_seen
        {
            return None;
        }
        let hero = frame.heroes.iter().find(|hero| {
            effect.owner_id != 0
                && hero.id == effect.owner_id
                && hero.visible
                && hero.alive
                && self.visible.contains(&hero.id)
        })?;
        let aim = (Vec2::from_array(effect.end) - Vec2::from_array(effect.position))
            .try_normalize()
            .map(|aim| shared::math::hero_yaw_towards(aim.x, aim.y));
        Some(SkillCastObserved {
            actor_id: hero.id,
            key: CastKey::Skill(row),
            slot: hero.slot_of(skill)?,
            sequence: effect.id,
            recast: false,
            origin: hero.position,
            position: hero.position,
            yaw: aim,
            forward: hero.forward,
            local: hero.local,
        })
    }
}

/// Takes in every snapshot the session applied, after the cast observer has read the frame.
pub(crate) fn track_effects(
    game: Option<Res<GameStateSnapshot>>,
    clock: Res<crate::vfx_clock::VfxClock>,
    registry: Option<Res<SkillPresentation>>,
    heroes: Query<(
        &NetworkPlayerId,
        &Transform,
        &InheritedVisibility,
        &CombatStats,
        &NetworkHeroClass,
        Option<&PlayerLoadout>,
        Has<Player>,
    )>,
    mut memory: ResMut<EffectMemory>,
    mut cursor: Local<MessageCursor<SkillCastObserved>>,
    mut casts: ResMut<Messages<SkillCastObserved>>,
    mut events: MessageWriter<StageEvent>,
) {
    let observed: Vec<SkillCastObserved> = cursor.read(&casts).cloned().collect();
    // The resource changes when a snapshot is applied and at no other time.
    let Some(game) = game.filter(|game| game.is_changed()) else {
        return;
    };
    let heroes: Vec<HeroSeen> = heroes
        .iter()
        .map(
            |(id, pose, visibility, stats, class, loadout, local)| HeroSeen {
                id: id.0,
                visible: visibility.get(),
                alive: stats.is_alive(),
                position: pose.translation,
                forward: pose.forward().as_vec3(),
                class: class.0,
                loadout: loadout.and_then(|loadout| loadout.0.as_ref()),
                local,
            },
        )
        .collect();
    let taken = memory.take(&Frame {
        round: Some((game.meta.server_epoch, game.meta.match_id)),
        now: clock.now,
        running: matches!(game.state, GameState::Running),
        effects: &game.skill_effects,
        heroes: &heroes,
        casts: &observed,
        registry: registry.as_deref(),
    });
    events.write_batch(taken.events);
    casts.write_batch(taken.casts);
    // Its own casts are no edges to compare the next warning with.
    cursor.clear(&casts);
}

#[cfg(test)]
mod tests;
