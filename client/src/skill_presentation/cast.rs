//! The one detector of accepted actions and of the relocations that come with them, for both
//! render backends. An action counts once, when a hero the client saw in the previous frame
//! and sees now reports a newer action sequence; a hero that appears is taken as it is and
//! nothing it did unseen is replayed. A relocation is reported with the positions the client
//! observed and never with one it did not.
use super::category::{self, SkillKey};
use crate::combat::CombatStats;
use crate::net::{
    GameState, GameStateSnapshot, NetworkHeroClass, NetworkPlayerId, PlayerActionFacing,
    PlayerCosmeticAction, PlayerLoadout, PlayerUtility, SessionEvent,
};
use crate::player::Player;
use bevy::prelude::*;
use shared::loadout::LoadoutState;
use shared::utility::UtilityState;
use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass, SkillSlot};
use std::collections::{HashMap, HashSet};

/// The remaining dash cooldown is aged locally between snapshots, so the restart of the
/// cooldown has to exceed any drift between the two clocks.
const DASH_RESTART_SECS: f32 = 1.0;

/// The row an accepted action and its receipts belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CastKey {
    Skill(SkillKey),
    /// The basic attack of the kit whose core is this class.
    Basic(HeroClass),
}

impl CastKey {
    /// Resolved from the accepted recipe, as the motion of the action is: the skill in the
    /// slot, or for a basic attack the core of the kit (rule E-13). A malformed recipe
    /// resolves to nothing.
    pub(crate) fn of(class: HeroClass, loadout: Option<&LoadoutState>, slot: u8) -> Option<Self> {
        let equipped = crate::equipped_skills::resolve_state(class, loadout)?;
        if slot == BASIC_ATTACK_ACTION_SLOT {
            return Some(Self::Basic(
                equipped.resolved().map_or(class, |kit| kit.core().class()),
            ));
        }
        SkillKey::from_id(equipped.ability(SkillSlot::from_index(slot)?).id).map(Self::Skill)
    }

    pub(crate) fn skill(self) -> Option<SkillKey> {
        match self {
            Self::Skill(key) => Some(key),
            Self::Basic(_) => None,
        }
    }
}

/// The yaw of an accepted action, when the snapshot carries one for that very action.
pub(crate) fn action_yaw(
    action: &PlayerCosmeticAction,
    facing: &PlayerActionFacing,
) -> Option<f32> {
    (facing.sequence == action.sequence)
        .then_some(facing.yaw)
        .flatten()
}

/// One accepted action of a living hero that was visible before it and is visible now.
#[derive(Message, Clone, Debug, PartialEq)]
pub(crate) struct SkillCastObserved {
    pub actor_id: u64,
    pub key: CastKey,
    pub slot: u8,
    pub sequence: u64,
    /// Whether the slot offered a recast in the previous observation; without one the edge
    /// is a first cast.
    pub recast: bool,
    /// Where the hero was observed before any move of the same snapshot.
    pub origin: Vec3,
    /// Where the hero is observed now, after such a move.
    pub position: Vec3,
    pub yaw: Option<f32>,
    /// The way the hero faces; the direction of an action that carries no yaw.
    pub forward: Vec3,
    pub local: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MoveCause {
    /// A completed return to base. Nothing is drawn.
    Recall,
    /// The utility dash, which keeps its own look.
    UtilityDash,
    /// The hero's own cast moved it.
    SkillCast,
    /// Something else displaced the hero; the client does not know what.
    Forced,
}

/// A relocation of a living hero that the server reported as an instant one.
#[derive(Message, Clone, Debug, PartialEq)]
pub(crate) struct MoveObserved {
    pub actor_id: u64,
    /// Where the hero was observed before the move; `None` when it was not visible then.
    pub from: Option<Vec3>,
    /// Where the hero is observed after it; `None` when it is no longer visible.
    pub to: Option<Vec3>,
    pub cause: MoveCause,
    /// The row of the cast that moved the hero, for `MoveCause::SkillCast`.
    pub skill: Option<SkillKey>,
    /// The action sequence of that cast, else a number of this relocation alone.
    pub seed: u64,
    pub local: bool,
}

/// Why the movement barrier of a hero advanced, in this order: a recall, the utility dash
/// (its cooldown restarted), an own cast of a skill that can move its caster, the utility
/// dash again (the hero made a utility request in the same snapshot), else a displacement
/// by something else. The request mark alone shows the dash when cooldowns are switched
/// off and the server reports none (`common/src/hero_timers.rs:139-144`,
/// `common/src/utility.rs:64`).
pub(crate) fn classify_move(
    prev: &UtilityState,
    next: &UtilityState,
    own_action_edge: bool,
    movement_capable: bool,
) -> MoveCause {
    if next.recall_sequence > prev.recall_sequence {
        MoveCause::Recall
    } else if next.dash_remaining_secs > prev.dash_remaining_secs + DASH_RESTART_SECS {
        MoveCause::UtilityDash
    } else if own_action_edge && movement_capable {
        MoveCause::SkillCast
    } else if next.last_request_id > prev.last_request_id {
        MoveCause::UtilityDash
    } else {
        MoveCause::Forced
    }
}

/// Heroes whose relocation of this frame was painted by the choreography, so the generic
/// dash is left out for them. Refilled every frame.
#[derive(Resource, Default)]
pub(crate) struct ThemedDashes(pub HashSet<u64>);

/// One hero as the client holds it this frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sighting<'a> {
    pub actor: Entity,
    pub actor_id: u64,
    pub local: bool,
    /// The entity is drawn: it exists and nothing hides it.
    pub visible: bool,
    pub alive: bool,
    pub position: Vec3,
    pub forward: Vec3,
    pub class: HeroClass,
    pub loadout: Option<&'a LoadoutState>,
    pub action: PlayerCosmeticAction,
    pub facing: PlayerActionFacing,
    pub utility: UtilityState,
}

/// What the previous frame showed of a hero.
#[derive(Clone, Copy)]
struct Seen {
    visible: bool,
    /// The highest action sequence seen, so an older or repeated action is never an edge.
    sequence: u64,
    position: Vec3,
    utility: UtilityState,
    can_recast: [bool; 4],
    /// The frame of this sighting; a hero the client no longer holds is forgotten.
    frame: u64,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Observed {
    pub casts: Vec<SkillCastObserved>,
    pub moves: Vec<MoveObserved>,
}

/// Frame-to-frame memory of the heroes the client holds.
#[derive(Default)]
pub(crate) struct CastObserver {
    round: Option<(u64, u64)>,
    frame: u64,
    seen: HashMap<Entity, Seen>,
}

impl CastObserver {
    /// Drops every memory: the next sighting of each hero is a baseline. A new connection
    /// starts this way.
    pub(crate) fn forget(&mut self) {
        self.seen.clear();
    }

    /// Compares this frame with the previous one. A hero seen for the first time, and every
    /// hero after a round change, is only remembered. Outside a running match nothing is
    /// reported.
    pub(crate) fn observe<'a>(
        &mut self,
        round: Option<(u64, u64)>,
        running: bool,
        heroes: impl IntoIterator<Item = Sighting<'a>>,
    ) -> Observed {
        if self.round != round {
            self.round = round;
            self.seen.clear();
        }
        self.frame += 1;
        let frame = self.frame;
        let mut observed = Observed::default();
        for hero in heroes {
            let before = self.seen.get(&hero.actor).copied();
            self.seen.insert(
                hero.actor,
                Seen {
                    visible: hero.visible,
                    sequence: before
                        .map_or(0, |seen| seen.sequence)
                        .max(hero.action.sequence),
                    position: hero.position,
                    utility: hero.utility,
                    can_recast: hero.loadout.map_or([false; 4], |loadout| {
                        loadout.slots.map(|slot| slot.can_recast)
                    }),
                    frame,
                },
            );
            let Some(before) = before else {
                continue;
            };
            if !(running && hero.alive) {
                continue;
            }
            let edge = hero.action.sequence > before.sequence;
            let key = edge
                .then(|| CastKey::of(hero.class, hero.loadout, hero.action.slot))
                .flatten();
            let recast = before
                .can_recast
                .get(usize::from(hero.action.slot))
                .copied()
                .unwrap_or(false);
            let moved = hero.utility.dash_sequence > before.utility.dash_sequence;
            if moved {
                let skill = key.and_then(CastKey::skill);
                let capable = skill
                    .and_then(SkillKey::modular)
                    .is_some_and(|id| category::movement_capable(id, recast));
                let cause = classify_move(&before.utility, &hero.utility, edge, capable);
                let from = before.visible.then_some(before.position);
                let to = hero.visible.then_some(hero.position);
                if from.is_some() || to.is_some() {
                    let own_cast = cause == MoveCause::SkillCast;
                    observed.moves.push(MoveObserved {
                        actor_id: hero.actor_id,
                        from,
                        to,
                        cause,
                        skill: skill.filter(|_| own_cast),
                        seed: if own_cast {
                            hero.action.sequence
                        } else {
                            hero.actor_id << 16 | hero.utility.dash_sequence
                        },
                        local: hero.local,
                    });
                }
            }
            // A hero that was hidden a frame ago is taken as it is now; one that is hidden
            // now shows nothing of what it just did.
            if let Some(key) = key.filter(|_| before.visible && hero.visible) {
                observed.casts.push(SkillCastObserved {
                    actor_id: hero.actor_id,
                    key,
                    slot: hero.action.slot,
                    sequence: hero.action.sequence,
                    recast,
                    origin: if moved {
                        before.position
                    } else {
                        hero.position
                    },
                    position: hero.position,
                    yaw: action_yaw(&hero.action, &hero.facing),
                    forward: hero.forward,
                    local: hero.local,
                });
            }
        }
        self.seen.retain(|_, seen| seen.frame == frame);
        observed
    }
}

/// Reads every hero once per frame, after the snapshot was applied and remote heroes were
/// placed, and reports its accepted actions and relocations.
pub(crate) fn observe_skill_casts(
    game: Option<Res<GameStateSnapshot>>,
    mut session: MessageReader<SessionEvent>,
    heroes: Query<(
        Entity,
        &NetworkPlayerId,
        &Transform,
        &InheritedVisibility,
        &CombatStats,
        &PlayerCosmeticAction,
        &PlayerActionFacing,
        &NetworkHeroClass,
        Option<&PlayerLoadout>,
        Option<&PlayerUtility>,
        Has<Player>,
    )>,
    mut observer: Local<CastObserver>,
    mut casts: MessageWriter<SkillCastObserved>,
    mut moves: MessageWriter<MoveObserved>,
) {
    let reconnected = session
        .read()
        .filter(|event| {
            matches!(
                event,
                SessionEvent::TransportStarted { .. }
                    | SessionEvent::Connected
                    | SessionEvent::Disconnected { .. }
            )
        })
        .count()
        > 0;
    if reconnected {
        observer.forget();
    }
    let Some(game) = game else {
        return;
    };
    let observed = observer.observe(
        Some((game.meta.server_epoch, game.meta.match_id)),
        matches!(game.state, GameState::Running),
        heroes.iter().map(
            |(
                actor,
                id,
                pose,
                visibility,
                stats,
                action,
                facing,
                class,
                loadout,
                utility,
                local,
            )| {
                Sighting {
                    actor,
                    actor_id: id.0,
                    local,
                    visible: visibility.get(),
                    alive: stats.is_alive(),
                    position: pose.translation,
                    forward: pose.forward().as_vec3(),
                    class: class.0,
                    loadout: loadout.and_then(|loadout| loadout.0.as_ref()),
                    action: *action,
                    facing: *facing,
                    utility: utility.map(|utility| utility.state).unwrap_or_default(),
                }
            },
        ),
    );
    casts.write_batch(observed.casts);
    moves.write_batch(observed.moves);
}

#[cfg(test)]
mod tests;
