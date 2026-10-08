use super::super::accents::{self, LinkBook, OneShot};
use super::super::tests::target::target;
use super::super::vocab::ExpireKind;
use super::*;
use crate::combat_feedback::ConfirmedHit;
use crate::game_vfx::{ConfirmedBurst, ParticleSource, ParticleSpec, SkillBurst};
use crate::vfx_clock::VfxClock;
use common::offline::{EPOCH, LOCAL_ADDR, PracticeSession};
use shared::loadout::skill;
use shared::practice::PracticeCommand;
use shared::wire::{CharacterChoice, ClientPacket, ServerPacket};
use std::time::Instant;

use EffectVisualKind as K;

const ROUND: Option<(u64, u64)> = Some((3, 1));
/// One snapshot interval.
const STEP: f64 = 0.05;
const HOME: Vec3 = Vec3::new(2.0, 0.5, -4.0);
const OWNER: u64 = 7;
/// A frame that drew nothing.
const NOTHING: Vec<Vec<ParticleSpec>> = Vec::new();

fn effect(id: u64, skill: SkillId, kind: EffectVisualKind) -> SkillEffectState {
    SkillEffectState {
        id,
        owner_id: OWNER,
        owner_team: shared::map::Team::Green,
        skill,
        kind,
        position: [2.0, 3.0],
        end: [2.0, 9.0],
        radius: 1.5,
        remaining_secs: 5.0,
        armed: false,
        consumed_segments: 0,
    }
}

/// The effect as a later snapshot shows it.
fn with(effect: &SkillEffectState, change: impl FnOnce(&mut SkillEffectState)) -> SkillEffectState {
    let mut next = effect.clone();
    change(&mut next);
    next
}

fn hero(class: HeroClass) -> HeroSeen<'static> {
    HeroSeen {
        id: OWNER,
        visible: true,
        alive: true,
        position: HOME,
        forward: Vec3::NEG_Z,
        class,
        loadout: None,
        local: false,
    }
}

fn owner() -> OwnerSeen {
    OwnerSeen {
        visible: true,
        alive: true,
        parrying: Some(false),
        position: HOME,
        slot: Some(3),
        local: false,
    }
}

/// The slot the class preset binds `skill` to.
fn slot_of(class: HeroClass, skill: SkillId) -> u8 {
    shared::loadout::preset_for_class(class)
        .unwrap()
        .skills()
        .iter()
        .position(|id| *id == skill)
        .unwrap() as u8
}

fn rule(effect: &SkillEffectState) -> StageRule {
    category::stage_rule(effect.skill, effect.kind)
}

/// A memory fed with one snapshot per interval.
struct Feed {
    memory: EffectMemory,
    registry: SkillPresentation,
    now: f64,
}

impl Feed {
    fn new(registry: SkillPresentation) -> Self {
        Self {
            memory: EffectMemory::default(),
            registry,
            now: 100.0,
        }
    }

    fn snap_with(
        &mut self,
        effects: &[SkillEffectState],
        heroes: &[HeroSeen],
        casts: &[SkillCastObserved],
    ) -> Taken {
        self.now += STEP;
        self.memory.take(&Frame {
            round: ROUND,
            now: self.now,
            running: true,
            effects,
            heroes,
            casts,
            registry: Some(&self.registry),
        })
    }

    /// A snapshot that holds one effect.
    fn one(&mut self, effect: &SkillEffectState) -> Vec<(u64, StageChange)> {
        self.snap(std::slice::from_ref(effect))
    }

    fn snap(&mut self, effects: &[SkillEffectState]) -> Vec<(u64, StageChange)> {
        let taken = self.snap_with(effects, &[], &[]);
        assert!(taken.casts.is_empty());
        taken
            .events
            .iter()
            .map(|event| (event.effect.id, event.change))
            .collect()
    }
}

fn step(change: Transition) -> StageChange {
    StageChange::Transition(change)
}

/// The classification of an effect last seen as `last`, `gap` seconds before the snapshot
/// that dropped it.
fn ended(last: &SkillEffectState, ctx: EndContext) -> EndKind {
    classify_end(rule(last), last.skill, &Memory::first(last, 100.0), &ctx)
}

/// The dropping snapshot came one interval later and shows a living, visible owner.
fn seen_out() -> EndContext {
    EndContext {
        gap_secs: STEP,
        owner: Some(owner()),
        recast_edge: false,
    }
}

#[test]
fn stage_view_reads_armed_kind_and_remaining() {
    // A live body is active for all it is replicated, whatever its flags say.
    for (id, kind) in [
        (SkillId::OrbitalField, K::Field),
        (SkillId::WinterShard, K::Bolt),
        (SkillId::WinterDivide, K::BeamWarning),
        (SkillId::OrbitalCommand, K::Orb),
    ] {
        for armed in [false, true] {
            let seen = with(&effect(1, id, kind), |e| e.armed = armed);
            let view = view(&seen);
            assert_eq!(
                (view.stage, view.progress),
                (Stage::Active, 1.0),
                "{kind:?}"
            );
            assert_eq!(view.remaining, 5.0);
        }
    }
    // A trap and the pillar are telegraphs until the replicated flag is set.
    for id in [SkillId::WildTraps, SkillId::FaultLine] {
        let trap = effect(1, id, K::Trap);
        assert_eq!(view(&trap).stage, Stage::Telegraph);
        assert_eq!(view(&trap).progress, 0.0);
        let armed = with(&trap, |e| e.armed = true);
        assert_eq!(
            (view(&armed).stage, view(&armed).progress),
            (Stage::Active, 1.0)
        );
    }
    // A warning is a telegraph for as long as its kind says so.
    for (id, live) in [(SkillId::DawnRay, K::Beam), (SkillId::HorizonWave, K::Bolt)] {
        let (telegraph, tail) = category::telegraph_secs(id)
            .zip(category::tail_secs(id))
            .unwrap();
        let warning = effect(1, id, K::BeamWarning);
        let at = |remaining: f32| view(&with(&warning, |e| e.remaining_secs = remaining));
        assert_eq!(at(tail + telegraph).stage, Stage::Telegraph);
        assert!(at(tail + telegraph).progress < 1e-5);
        assert!((at(tail + 0.5 * telegraph).progress - 0.5).abs() < 1e-5);
        assert_eq!(at(tail).progress, 1.0);
        let fired = with(&warning, |e| e.kind = live);
        assert_eq!(
            (view(&fired).stage, view(&fired).progress),
            (Stage::Active, 1.0)
        );
    }
    // A fuse is a telegraph for all it is observed. Its fill is complete exactly when the
    // server fires, whatever the first value the client saw of it.
    for id in [
        SkillId::FurnaceBreath,
        SkillId::MirrorGuard,
        SkillId::OrbitalCollapse,
    ] {
        let kind = category::own_kinds(id)[0];
        let (telegraph, tail) = category::telegraph_secs(id)
            .zip(category::tail_secs(id))
            .unwrap();
        let fuse = effect(1, id, kind);
        let at = |remaining: f32| view(&with(&fuse, |e| e.remaining_secs = remaining));
        for first_seen in [tail + telegraph, tail + 0.6 * telegraph, tail + 0.01] {
            let seen = at(first_seen);
            assert_eq!(seen.stage, Stage::Telegraph, "{}", id.id());
            let expected = 1.0 - (first_seen - tail) / telegraph;
            assert!((seen.progress - expected).abs() < 1e-5, "{}", id.id());
        }
        assert_eq!(at(tail).progress, 1.0, "{}", id.id());
        assert!(at(tail + telegraph).progress < 1e-5, "{}", id.id());
        // Outside the telegraph the fill is clamped and stays a number.
        assert_eq!(at(tail + telegraph + 3.0).progress, 0.0);
        assert_eq!(at(0.0).progress, 1.0);
        assert_eq!(at(f32::NAN).remaining, 0.0);
        assert_eq!(at(-1.0).remaining, 0.0);
        // The armed flag of a fuse says nothing.
        assert_eq!(
            view(&with(&fuse, |e| e.armed = true)).stage,
            Stage::Telegraph
        );
    }
}

#[test]
fn each_transition_fires_once() {
    let mut feed = Feed::new(target());

    // Armed: the flag of a trap goes up. First sight reports nothing, armed or not.
    let trap = effect(1, SkillId::WildTraps, K::Trap);
    let born_armed = with(&effect(2, SkillId::WildTraps, K::Trap), |e| e.armed = true);
    assert_eq!(feed.snap(&[trap.clone(), born_armed.clone()]), []);
    let armed = with(&trap, |e| e.armed = true);
    assert_eq!(
        feed.snap(&[armed.clone(), born_armed.clone()]),
        [(1, step(Transition::Armed))]
    );
    assert_eq!(feed.snap(&[armed, born_armed]), []);

    // KindFlipped: a warning becomes its beam, or the bolt it announced, on the same id.
    for (id, live) in [(SkillId::DawnRay, K::Beam), (SkillId::HorizonWave, K::Bolt)] {
        let mut feed = Feed::new(target());
        let warning = effect(4, id, K::BeamWarning);
        assert_eq!(feed.one(&warning), []);
        assert_eq!(feed.one(&warning), []);
        let fired = with(&warning, |e| {
            e.kind = live;
            e.armed = true;
        });
        assert_eq!(feed.one(&fired), [(4, step(Transition::KindFlipped))]);
        assert_eq!(feed.snap(&[fired]), []);
    }

    // SegmentBroken: each new bit of the cage, once, with its side.
    let mut feed = Feed::new(target());
    let cage = effect(5, SkillId::IronBoundary, K::Cage);
    assert_eq!(feed.one(&cage), []);
    let broken = with(&cage, |e| e.consumed_segments = 0b10010);
    assert_eq!(
        feed.one(&broken),
        [
            (5, step(Transition::SegmentBroken(1))),
            (5, step(Transition::SegmentBroken(4)))
        ]
    );
    assert_eq!(feed.one(&broken), []);
    let more = with(&broken, |e| e.consumed_segments = 0b10011);
    assert_eq!(
        feed.snap(&[more]),
        [(5, step(Transition::SegmentBroken(0)))]
    );

    // Turned and Renewed: the ember comes home. Both are reported for that one snapshot.
    let mut feed = Feed::new(target());
    let ember = with(&effect(6, SkillId::WanderingEmber, K::Bolt), |e| {
        e.end = [2.0, 4.0];
        e.remaining_secs = 2.2;
    });
    assert_eq!(feed.one(&ember), []);
    let home = with(&ember, |e| {
        e.position = [2.0, 3.5];
        e.end = [2.0, 2.5];
        e.remaining_secs = 3.0;
    });
    assert_eq!(
        feed.one(&home),
        [
            (6, step(Transition::Turned)),
            (6, step(Transition::Renewed))
        ]
    );
    assert_eq!(feed.snap(&[home]), []);
}

#[test]
fn transitions_have_exact_thresholds_and_need_their_rule() {
    let bolt = with(&effect(1, SkillId::WinterShard, K::Bolt), |e| {
        e.end = [2.0, 4.0];
    });
    let turned = |degrees: f32| {
        let heading = Vec2::from_angle((90.0 + degrees).to_radians());
        with(&bolt, |e| e.end = [2.0 + heading.x, 3.0 + heading.y])
    };
    let of =
        |before: &SkillEffectState, now: &SkillEffectState| transitions(rule(now), before, now);
    // A turn is a change of heading above 45 degrees, to either side.
    assert_eq!(of(&bolt, &turned(44.0)), []);
    assert_eq!(of(&bolt, &turned(-44.0)), []);
    assert_eq!(of(&bolt, &turned(46.0)), [Transition::Turned]);
    assert_eq!(of(&bolt, &turned(-46.0)), [Transition::Turned]);
    assert_eq!(of(&bolt, &turned(180.0)), [Transition::Turned]);
    // A body without a heading in either snapshot has not turned.
    let still = with(&bolt, |e| e.end = e.position);
    assert_eq!(of(&still, &turned(180.0)), []);
    assert_eq!(of(&bolt, &still), []);
    // Only a kind replicated with a heading can turn: the far end of a lane is a place.
    let lane = effect(2, SkillId::WinterDivide, K::BeamWarning);
    let swapped = with(&lane, |e| std::mem::swap(&mut e.position, &mut e.end));
    assert_eq!(of(&lane, &swapped), []);

    // A renewal is a remaining time that rose by more than 0.25 s.
    let renewed = |by: f32| with(&bolt, |e| e.remaining_secs += by);
    assert_eq!(of(&bolt, &renewed(0.25)), []);
    assert_eq!(of(&bolt, &renewed(0.26)), [Transition::Renewed]);
    assert_eq!(of(&bolt, &renewed(-1.0)), []);

    // The armed flag is an event only where it gates the stage.
    let arms = |id: SkillId, kind: K| {
        let before = effect(3, id, kind);
        of(&before, &with(&before, |e| e.armed = true))
    };
    assert_eq!(arms(SkillId::WildTraps, K::Trap), [Transition::Armed]);
    assert_eq!(arms(SkillId::FaultLine, K::Trap), [Transition::Armed]);
    assert_eq!(arms(SkillId::WinterShard, K::Bolt), []);
    assert_eq!(arms(SkillId::DawnRay, K::BeamWarning), []);
    assert_eq!(arms(SkillId::FurnaceBreath, K::BeamWarning), []);
    // A flag that drops is no event.
    let trap = with(&effect(3, SkillId::WildTraps, K::Trap), |e| e.armed = true);
    assert_eq!(of(&trap, &with(&trap, |e| e.armed = false)), []);

    // A kind flips forward only, and only on a skill that warns.
    let ray = effect(4, SkillId::DawnRay, K::BeamWarning);
    let beam = with(&ray, |e| e.kind = K::Beam);
    assert_eq!(of(&ray, &beam), [Transition::KindFlipped]);
    assert_eq!(of(&beam, &ray), []);
    assert_eq!(of(&beam, &beam), []);

    // Only a cage breaks, only by a new bit, and only its five sides count.
    let cage = with(&effect(5, SkillId::IronBoundary, K::Cage), |e| {
        e.consumed_segments = 0b00110;
    });
    assert_eq!(
        of(&cage, &with(&cage, |e| e.consumed_segments = 0b00100)),
        []
    );
    assert_eq!(
        of(&cage, &with(&cage, |e| e.consumed_segments = 0b1110_0110)),
        []
    );
    let wall = effect(6, SkillId::Northwall, K::ShieldWall);
    assert_eq!(of(&wall, &with(&wall, |e| e.consumed_segments = 1)), []);

    // An auxiliary object has no transitions, and neither has an id that changed its skill.
    let orb = with(&effect(7, SkillId::OrbitalCommand, K::Orb), |e| {
        e.end = [2.0, 4.0];
    });
    let other = with(&orb, |e| {
        e.end = [2.0, 2.0];
        e.remaining_secs += 9.0;
        e.armed = true;
    });
    assert_eq!(of(&orb, &other), []);
    assert_eq!(
        of(
            &bolt,
            &with(&turned(180.0), |e| e.skill = SkillId::RiftNeedle)
        ),
        []
    );
}

#[test]
fn end_classification_is_silent_by_default() {
    // Whatever skill and kind: an effect that leaves the snapshot in the middle of its
    // life (fog, a hit, a trigger, a cancel) ends silently, with or without its owner.
    for id in SkillId::ALL {
        for kind in category::own_kinds(id)
            .iter()
            .chain(category::aux_kinds(id))
        {
            let live = effect(1, id, *kind);
            for ctx in [
                seen_out(),
                EndContext {
                    owner: None,
                    ..seen_out()
                },
            ] {
                assert_eq!(ended(&live, ctx), EndKind::Silent, "{} {kind:?}", id.id());
            }
        }
    }

    // The same last sightings that would be classified are silent after a long gap.
    let zone = with(&effect(1, SkillId::OrbitalField, K::Field), |e| {
        e.remaining_secs = 0.05;
    });
    let fuse = with(&effect(2, SkillId::OrbitalCollapse, K::BeamWarning), |e| {
        e.remaining_secs = 0.22;
    });
    let burst = effect(3, SkillId::DawnField, K::Field);
    let recast = EndContext {
        recast_edge: true,
        ..seen_out()
    };
    assert_eq!(ended(&zone, seen_out()), EndKind::TrueExpiry);
    assert_eq!(ended(&fuse, seen_out()), EndKind::Released);
    assert_eq!(ended(&burst, recast), EndKind::Detonated);
    for gap_secs in [MAX_GAP_SECS + 0.01, 3.0, -0.5, f64::NAN] {
        for (last, ctx) in [(&zone, seen_out()), (&fuse, seen_out()), (&burst, recast)] {
            let late = EndContext { gap_secs, ..ctx };
            assert_eq!(ended(last, late), EndKind::Silent, "{gap_secs}");
        }
    }
    assert_eq!(
        ended(
            &zone,
            EndContext {
                gap_secs: MAX_GAP_SECS,
                ..seen_out()
            }
        ),
        EndKind::TrueExpiry
    );

    // A dead owner, and for an inferred release a hidden one.
    let dead = EndContext {
        owner: Some(OwnerSeen {
            alive: false,
            ..owner()
        }),
        ..seen_out()
    };
    let hidden = EndContext {
        owner: Some(OwnerSeen {
            visible: false,
            ..owner()
        }),
        ..seen_out()
    };
    assert_eq!(ended(&zone, dead), EndKind::Silent);
    assert_eq!(ended(&fuse, dead), EndKind::Silent);
    assert_eq!(ended(&fuse, hidden), EndKind::Silent);
    assert_eq!(
        ended(
            &burst,
            EndContext {
                recast_edge: true,
                ..dead
            }
        ),
        EndKind::Silent
    );

    // Travelling bodies end silently even when their time ran out with everything in view.
    for (id, kind) in [
        (SkillId::WinterShard, K::Bolt),
        (SkillId::MountainEcho, K::Bolt),
        (SkillId::WanderingEmber, K::Bolt),
        (SkillId::HorizonWave, K::Bolt),
        (SkillId::WildRocket, K::Rocket),
        (SkillId::DawnBarrier, K::Barrier),
    ] {
        let spent = with(&effect(1, id, kind), |e| e.remaining_secs = 0.0);
        assert_eq!(ended(&spent, seen_out()), EndKind::Silent, "{}", id.id());
        assert_eq!(ended(&spent, recast), EndKind::Silent, "{}", id.id());
    }
    // So do auxiliary objects: their keys do not follow one instance.
    for (id, kind) in [
        (SkillId::OrbitalCommand, K::Orb),
        (SkillId::OrbitalGuard, K::Orb),
        (SkillId::IronHook, K::Soul),
        (SkillId::AnchorStep, K::Anchor),
        (SkillId::FourfoldDuel, K::Healing),
    ] {
        let spent = with(&effect(1, id, kind), |e| e.remaining_secs = 0.0);
        assert_eq!(ended(&spent, seen_out()), EndKind::Silent, "{}", id.id());
    }

    // Through the memory: a round change forgets everything without an event, and so does
    // a stream that stalled.
    let heroes = [hero(HeroClass::Orbitwright)];
    let mut feed = Feed::new(target());
    feed.snap_with(&[zone.clone(), fuse.clone()], &heroes, &[]);
    feed.now += STEP;
    let next_round = feed.memory.take(&Frame {
        round: Some((3, 2)),
        now: feed.now,
        running: true,
        effects: &[],
        heroes: &heroes,
        casts: &[],
        registry: Some(&feed.registry),
    });
    assert_eq!(next_round, Taken::default());
    assert!(feed.memory.get(EffectKey::Runtime(1)).is_none());

    let mut feed = Feed::new(target());
    feed.snap_with(&[zone.clone(), fuse.clone()], &heroes, &[]);
    feed.now += 1.0;
    assert_eq!(feed.snap_with(&[], &heroes, &[]), Taken::default());

    // The same two effects, dropped by the very next snapshot with their owner in view.
    let mut feed = Feed::new(target());
    feed.snap_with(&[zone.clone(), fuse.clone()], &heroes, &[]);
    let ends: Vec<_> = feed
        .snap_with(&[], &heroes, &[])
        .events
        .into_iter()
        .map(|event| (event.effect.id, event.change))
        .collect();
    assert_eq!(
        ends,
        [
            (1, StageChange::Ended(EndKind::TrueExpiry)),
            (2, StageChange::Ended(EndKind::Released))
        ]
    );
    // An end is reported once.
    assert_eq!(feed.snap_with(&[], &heroes, &[]), Taken::default());
}

#[test]
fn released_needs_a_visible_living_owner() {
    for id in [
        SkillId::FurnaceBreath,
        SkillId::MirrorGuard,
        SkillId::OrbitalCollapse,
    ] {
        let tail = category::tail_secs(id).unwrap();
        let kind = category::own_kinds(id)[0];
        let last = with(&effect(1, id, kind), |e| {
            e.remaining_secs = tail + 0.05;
        });
        assert_eq!(ended(&last, seen_out()), EndKind::Released, "{}", id.id());
        let seen = |change: fn(&mut OwnerSeen)| {
            let mut owner = owner();
            change(&mut owner);
            EndContext {
                owner: Some(owner),
                ..seen_out()
            }
        };
        // The owner must be in the dropping snapshot, visible and alive.
        assert_eq!(
            ended(
                &last,
                EndContext {
                    owner: None,
                    ..seen_out()
                }
            ),
            EndKind::Silent
        );
        assert_eq!(ended(&last, seen(|o| o.visible = false)), EndKind::Silent);
        assert_eq!(ended(&last, seen(|o| o.alive = false)), EndKind::Silent);
        // An effect whose owner the server withheld releases nothing.
        let ownerless = with(&last, |e| e.owner_id = 0);
        assert_eq!(ended(&ownerless, seen_out()), EndKind::Silent);
        // Only an end at the tail is the firing tick.
        let at = |remaining: f32| ended(&with(&last, |e| e.remaining_secs = remaining), seen_out());
        assert_eq!(at(tail + RELEASE_SLACK_SECS), EndKind::Released);
        assert_eq!(at(tail + RELEASE_SLACK_SECS + 0.01), EndKind::Silent);
        assert_eq!(at(tail + 0.4), EndKind::Silent);
        // A telegraph never runs out or detonates: an end without a release is silent.
        let recast = EndContext {
            recast_edge: true,
            ..seen(|o| o.alive = false)
        };
        assert_eq!(
            ended(&with(&last, |e| e.remaining_secs = 0.0), recast),
            EndKind::Silent
        );
        // The parry stance releases only once the replicated stance is over.
        let parry = id == SkillId::MirrorGuard;
        let still = ended(&last, seen(|o| o.parrying = Some(true)));
        let unknown = ended(&last, seen(|o| o.parrying = None));
        let expected = if parry {
            EndKind::Silent
        } else {
            EndKind::Released
        };
        assert_eq!((still, unknown), (expected, expected), "{}", id.id());
    }
}

#[test]
fn true_expiry_only_inside_0_11() {
    for (id, kind) in [
        (SkillId::OrbitalField, K::Field),
        (SkillId::GuidingLantern, K::Lantern),
        (SkillId::IronBoundary, K::Cage),
        (SkillId::Northwall, K::ShieldWall),
        (SkillId::WildTraps, K::Trap),
        (SkillId::FaultLine, K::Trap),
        (SkillId::WinterDivide, K::BeamWarning),
        (SkillId::DawnRay, K::Beam),
    ] {
        let at = |remaining: f32, ctx: EndContext| {
            ended(
                &with(&effect(1, id, kind), |e| e.remaining_secs = remaining),
                ctx,
            )
        };
        assert_eq!(at(0.0, seen_out()), EndKind::TrueExpiry, "{}", id.id());
        assert_eq!(at(EXPIRY_SECS, seen_out()), EndKind::TrueExpiry);
        assert_eq!(at(EXPIRY_SECS + 0.01, seen_out()), EndKind::Silent);
        assert_eq!(at(1.0, seen_out()), EndKind::Silent);
        // A recast edge of the owner proves nothing here.
        let recast = EndContext {
            recast_edge: true,
            ..seen_out()
        };
        assert_eq!(at(1.0, recast), EndKind::Silent, "{}", id.id());
        assert_eq!(at(0.05, recast), EndKind::TrueExpiry);
        // A known owner must be seen alive; it need not be visible.
        let owner_is = |change: fn(&mut OwnerSeen)| {
            let mut owner = owner();
            change(&mut owner);
            EndContext {
                owner: Some(owner),
                ..seen_out()
            }
        };
        assert_eq!(at(0.05, owner_is(|o| o.alive = false)), EndKind::Silent);
        assert_eq!(
            at(0.05, owner_is(|o| o.visible = false)),
            EndKind::TrueExpiry
        );
        let unheld = EndContext {
            owner: None,
            ..seen_out()
        };
        assert_eq!(at(0.05, unheld), EndKind::Silent);
        // An effect of an owner the server withheld may still run out.
        let ownerless = with(&effect(1, id, kind), |e| {
            e.owner_id = 0;
            e.remaining_secs = 0.05;
        });
        assert_eq!(ended(&ownerless, unheld), EndKind::TrueExpiry);
    }
}

#[test]
fn detonated_only_for_recast_zones() {
    let zone = effect(1, SkillId::DawnField, K::Field);
    let recast = EndContext {
        recast_edge: true,
        ..seen_out()
    };
    // The owner's recast bursts the zone at any time of its life.
    assert_eq!(ended(&zone, recast), EndKind::Detonated);
    assert_eq!(ended(&zone, seen_out()), EndKind::Silent);
    // It also bursts when its time runs out, never just fades.
    let spent = with(&zone, |e| e.remaining_secs = 0.1);
    assert_eq!(ended(&spent, seen_out()), EndKind::Detonated);
    assert_eq!(
        ended(&with(&zone, |e| e.remaining_secs = 0.12), seen_out()),
        EndKind::Silent
    );
    // An edge of a hero the client does not hold, or of a zone without an owner, is none.
    assert_eq!(
        ended(
            &zone,
            EndContext {
                owner: None,
                ..recast
            }
        ),
        EndKind::Silent
    );
    assert_eq!(
        ended(&with(&zone, |e| e.owner_id = 0), recast),
        EndKind::Silent
    );
    // No other skill detonates, whatever its owner cast in that snapshot.
    for id in SkillId::ALL {
        for kind in category::own_kinds(id) {
            if id == SkillId::DawnField {
                continue;
            }
            for remaining in [3.0, 0.05] {
                let last = with(&effect(1, id, *kind), |e| e.remaining_secs = remaining);
                assert_ne!(ended(&last, recast), EndKind::Detonated, "{}", id.id());
            }
        }
    }

    // Through the memory: the edge the observer reported in the dropping snapshot.
    let class = HeroClass::Dawnweaver;
    let heroes = [hero(class)];
    let edge = |recast: bool, key: SkillId| SkillCastObserved {
        actor_id: OWNER,
        key: CastKey::Skill(SkillKey::Modular(key)),
        slot: slot_of(class, key),
        sequence: 9,
        recast,
        origin: HOME,
        position: HOME,
        yaw: None,
        forward: Vec3::NEG_Z,
        local: false,
    };
    let dropped = |casts: &[SkillCastObserved]| {
        let mut feed = Feed::new(target());
        feed.snap_with(std::slice::from_ref(&zone), &heroes, &[]);
        feed.snap_with(&[], &heroes, casts)
            .events
            .into_iter()
            .map(|event| event.change)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        dropped(&[edge(true, SkillId::DawnField)]),
        [StageChange::Ended(EndKind::Detonated)]
    );
    // A first cast, a recast of another skill and a recast of another hero are not it.
    assert_eq!(dropped(&[edge(false, SkillId::DawnField)]), []);
    assert_eq!(dropped(&[edge(true, SkillId::DawnRay)]), []);
    let mut other = edge(true, SkillId::DawnField);
    other.actor_id = 8;
    assert_eq!(dropped(&[other]), []);
    assert_eq!(dropped(&[]), []);
}

#[test]
fn lane_fade_needs_an_observed_beam() {
    // A warning that vanishes never fired, however little time it had left.
    let warning = with(&effect(1, SkillId::DawnRay, K::BeamWarning), |e| {
        e.remaining_secs = 0.05;
    });
    assert_eq!(ended(&warning, seen_out()), EndKind::Silent);
    // The beam it became runs out.
    let beam = with(&warning, |e| e.kind = K::Beam);
    assert_eq!(ended(&beam, seen_out()), EndKind::TrueExpiry);
    assert_eq!(
        ended(&with(&beam, |e| e.remaining_secs = 0.14), seen_out()),
        EndKind::Silent
    );
    // The wave's warning ends silently, and so does the wave: it travels.
    let wave = with(&effect(2, SkillId::HorizonWave, K::BeamWarning), |e| {
        e.remaining_secs = 0.05;
    });
    assert_eq!(ended(&wave, seen_out()), EndKind::Silent);
    assert_eq!(
        ended(&with(&wave, |e| e.kind = K::Bolt), seen_out()),
        EndKind::Silent
    );

    // Through the memory, with the row of the ray: the fade is drawn for the beam only.
    let registry = target();
    let heroes = [hero(HeroClass::Dawnweaver)];
    let faded = |last: &SkillEffectState| {
        let mut feed = Feed::new(target());
        feed.snap_with(std::slice::from_ref(last), &heroes, &[]);
        let taken = feed.snap_with(&[], &heroes, &[]);
        taken
            .events
            .iter()
            .map(|event| accents::stage_shot(&registry, event))
            .collect::<Vec<_>>()
    };
    assert_eq!(faded(&warning), []);
    assert_eq!(faded(&beam), [Some(OneShot::Fade)]);
}

#[test]
fn the_trail_holds_observed_positions_only() {
    let mut feed = Feed::new(target());
    let key = EffectKey::Runtime(1);
    let at = |z: f32| {
        with(&effect(1, SkillId::WanderingEmber, K::Bolt), |e| {
            e.position = [2.0, z];
            e.end = [2.0, z + 1.0];
        })
    };
    let trail = |feed: &Feed| feed.memory.get(key).unwrap().trail().to_vec();
    // The first sighting has no earlier position.
    feed.snap(&[at(0.0)]);
    assert_eq!(trail(&feed), []);
    assert_eq!(feed.memory.get(key).unwrap().last_seen_secs, feed.now);
    // Earlier positions, newest first; a snapshot without a step adds none.
    feed.snap(&[at(1.0)]);
    feed.snap(&[at(1.0)]);
    feed.snap(&[at(2.0)]);
    assert_eq!(trail(&feed), [Vec2::new(2.0, 1.0), Vec2::new(2.0, 0.0)]);
    // At most six are kept.
    for z in 3..10 {
        feed.snap(&[at(z as f32)]);
    }
    assert_eq!(
        trail(&feed),
        [8.0, 7.0, 6.0, 5.0, 4.0, 3.0].map(|z| Vec2::new(2.0, z))
    );
    assert!(!feed.memory.get(key).unwrap().renewed);

    // A turn clears them, and the renewal that came with it is remembered for good.
    let back = with(&at(9.5), |e| {
        e.end = [2.0, 8.5];
        e.remaining_secs += 1.0;
    });
    assert_eq!(
        feed.one(&back),
        [
            (1, step(Transition::Turned)),
            (1, step(Transition::Renewed))
        ]
    );
    assert_eq!(trail(&feed), []);
    assert!(feed.memory.get(key).unwrap().renewed);
    let home = with(&back, |e| e.position = [2.0, 9.0]);
    feed.one(&home);
    assert_eq!(trail(&feed), [Vec2::new(2.0, 9.5)]);
    assert!(feed.memory.get(key).unwrap().renewed);

    // A step no body travels in one snapshot is a relocation: nothing is laid across it.
    let far = with(&home, |e| {
        e.position = [2.0, 9.0 - JUMP_UNITS - 0.1];
        e.end = [2.0, 8.0 - JUMP_UNITS - 0.1];
    });
    assert_eq!(feed.one(&far), []);
    assert_eq!(trail(&feed), []);

    // After a gap in the sightings the path between them was not seen, and what changed
    // meanwhile is not reported late.
    let near = with(&far, |e| {
        e.position = [2.0, 2.0];
        e.end = [2.0, 1.0];
    });
    feed.one(&near);
    assert_eq!(trail(&feed).len(), 1);
    feed.now += 1.0;
    let late = with(&near, |e| {
        e.position = [2.0, 1.0];
        e.end = [2.0, 2.0];
        e.remaining_secs += 2.0;
    });
    assert_eq!(feed.one(&late), []);
    assert_eq!(trail(&feed), []);
    assert_eq!(feed.memory.get(key).unwrap().last, late);
    // The stream goes on from there.
    assert_eq!(feed.snap(&[with(&late, |e| e.position = [2.0, 1.5])]), []);
    assert_eq!(trail(&feed), [Vec2::new(2.0, 1.0)]);

    // An instance first seen after its renewal was never seen to renew.
    let mut feed = Feed::new(target());
    feed.snap(&[back]);
    assert!(!feed.memory.get(key).unwrap().renewed);
}

/// What the body renderer reads beside the trail: whether the instance rests, and the
/// longest remaining time it was seen with.
#[test]
fn the_memory_knows_a_rest_and_the_longest_remaining_time() {
    let mut feed = Feed::new(target());
    let key = EffectKey::Runtime(1);
    let at = |z: f32, remaining: f32| {
        with(&effect(1, SkillId::WanderingEmber, K::Bolt), |e| {
            e.position = [2.0, z];
            e.end = [2.0, z + 1.0];
            e.remaining_secs = remaining;
        })
    };
    let held = |feed: &Feed| {
        let memory = feed.memory.get(key).unwrap();
        (memory.resting(), memory.peak_remaining_secs)
    };
    // A first sighting is not known to rest.
    feed.one(&at(0.0, 2.0));
    assert_eq!(held(&feed), (false, 2.0));
    feed.one(&at(1.0, 1.95));
    assert_eq!(held(&feed), (false, 2.0));
    // One or two snapshots with the same position are no rest yet: a snapshot can repeat
    // a position when the simulation steps less often than it is published.
    feed.one(&at(1.0, 1.9));
    feed.one(&at(1.0, 1.85));
    assert_eq!(held(&feed), (false, 2.0));
    // The third is; the positions it came through stay remembered.
    feed.one(&at(1.0, 1.8));
    assert_eq!(held(&feed), (true, 2.0));
    assert_eq!(feed.memory.get(key).unwrap().trail(), [Vec2::new(2.0, 0.0)]);
    feed.one(&at(2.0, 1.75));
    assert_eq!(held(&feed), (false, 2.0));
    // A renewal raises the longest time; the time that then runs down does not lower it.
    feed.one(&at(3.0, 3.0));
    assert_eq!(held(&feed), (false, 3.0));
    feed.one(&at(4.0, 2.95));
    assert_eq!(held(&feed), (false, 3.0));
    // A paused sandbox repeats one moment: nothing moves and no time passes. That is not
    // a rest, so a still of a body in flight keeps its trail.
    for _ in 0..8 {
        feed.now -= STEP;
        feed.one(&at(4.0, 2.95));
        assert_eq!(held(&feed), (false, 3.0));
    }
    assert_eq!(feed.memory.get(key).unwrap().trail().len(), 4);
    // After a gap in the sightings a rest is not known either.
    for _ in 0..3 {
        feed.one(&at(4.0, 2.9));
    }
    assert_eq!(held(&feed), (true, 3.0));
    feed.now += 1.0;
    feed.one(&at(4.0, 1.9));
    assert_eq!(held(&feed), (false, 3.0));
}

#[test]
fn auxiliary_objects_are_keyed_by_owner_kind_and_order() {
    let orb = |id: u64, owner: u64, skill: SkillId| {
        with(&effect(id, skill, K::Orb), |e| e.owner_id = owner)
    };
    let effects = [
        effect(40, SkillId::OrbitalField, K::Field),
        orb(41, 7, SkillId::OrbitalCommand),
        with(&effect(42, SkillId::IronHook, K::Soul), |e| e.owner_id = 7),
        orb(43, 8, SkillId::OrbitalCommand),
        with(&effect(44, SkillId::IronHook, K::Soul), |e| e.owner_id = 7),
        // A malformed effect has no key.
        with(&effect(45, SkillId::OrbitalField, K::Field), |e| {
            e.radius = f32::NAN;
        }),
    ];
    let aux = |owner: u64, kind: K, ordinal: u8| EffectKey::Aux {
        owner,
        kind,
        ordinal,
    };
    let keys: Vec<_> = keyed(&effects).into_iter().map(|(key, _)| key).collect();
    assert_eq!(
        keys,
        [
            EffectKey::Runtime(40),
            aux(7, K::Orb, 0),
            aux(7, K::Soul, 0),
            aux(8, K::Orb, 0),
            aux(7, K::Soul, 1),
        ]
    );

    // The orb keeps its memory when its positional id and its skill change, reports
    // nothing, and ends without a word.
    let mut feed = Feed::new(target());
    let heroes = [hero(HeroClass::Orbitwright)];
    let key = aux(7, K::Orb, 0);
    feed.snap_with(&[orb(41, 7, SkillId::OrbitalCommand)], &heroes, &[]);
    let guard = with(&orb(77, 7, SkillId::OrbitalGuard), |e| {
        e.position = [3.0, 3.0];
        e.remaining_secs = 60.0;
    });
    assert_eq!(
        feed.snap_with(std::slice::from_ref(&guard), &heroes, &[]),
        Taken::default()
    );
    let memory = feed.memory.get(key).unwrap();
    assert_eq!(memory.last, guard);
    assert_eq!(memory.trail(), [Vec2::new(2.0, 3.0)]);
    assert!(!memory.renewed);
    let spent = with(&guard, |e| e.remaining_secs = 0.0);
    feed.snap_with(std::slice::from_ref(&spent), &heroes, &[]);
    assert_eq!(feed.snap_with(&[], &heroes, &[]), Taken::default());
    assert!(feed.memory.get(key).is_none());
}

/// The cast the tracker derives from the first sight of `warning` in a stream that already
/// runs, with the heroes of both snapshots and the edges the observer reported.
fn warned(
    registry: SkillPresentation,
    before: &[HeroSeen],
    now: &[HeroSeen],
    warning: &SkillEffectState,
    casts: &[SkillCastObserved],
) -> Vec<SkillCastObserved> {
    let mut feed = Feed::new(registry);
    feed.snap_with(&[], before, &[]);
    let taken = feed.snap_with(std::slice::from_ref(warning), now, casts);
    assert!(taken.events.is_empty());
    // The same effect is never a first sight again.
    assert_eq!(
        feed.snap_with(std::slice::from_ref(warning), now, &[]),
        Taken::default()
    );
    taken.casts
}

#[test]
fn a_fresh_own_warning_stands_in_for_a_cast_edge_the_snapshot_hid() {
    for (class, id) in [
        (HeroClass::Dawnweaver, SkillId::DawnRay),
        (HeroClass::Riftshot, SkillId::HorizonWave),
    ] {
        let seen = [hero(class)];
        let (telegraph, tail) = category::telegraph_secs(id)
            .zip(category::tail_secs(id))
            .unwrap();
        // The warning one snapshot after the accepted cast, aimed along +X.
        let warning = with(&effect(50, id, K::BeamWarning), |e| {
            e.position = [HOME.x, HOME.z];
            e.end = [HOME.x + 20.0, HOME.z];
            e.remaining_secs = tail + telegraph - 0.05;
        });
        let casts = warned(target(), &seen, &seen, &warning, &[]);
        assert_eq!(
            casts,
            [SkillCastObserved {
                actor_id: OWNER,
                key: CastKey::Skill(SkillKey::Modular(id)),
                slot: slot_of(class, id),
                // The sequence of the hidden edge never reached the client.
                sequence: 50,
                recast: false,
                origin: HOME,
                position: HOME,
                yaw: Some(shared::math::hero_yaw_towards(1.0, 0.0)),
                forward: Vec3::NEG_Z,
                local: false,
            }],
            "{}",
            id.id()
        );
        // It draws the charge accent of the row at the hero, as the edge would have.
        let registry = target();
        let charge = accents::cast_burst(&registry, &casts[0], &[]);
        assert!(!charge.is_empty(), "{}", id.id());
        for spec in &charge {
            assert_eq!((spec.event_id, spec.source), (50, ParticleSource::Accent));
            assert!(spec.reach(HOME) <= accents::DECORATIVE_REACH + 1e-3);
        }

        // Beside the edge itself nothing is added; a later action of the same snapshot
        // (here a basic attack, or a recast) does not count as that edge.
        let mut edge = casts[0].clone();
        edge.sequence = 12;
        assert_eq!(
            warned(target(), &seen, &seen, &warning, &[edge.clone()]),
            []
        );
        let mut basic = edge.clone();
        basic.key = CastKey::Basic(class);
        basic.slot = shared::BASIC_ATTACK_ACTION_SLOT;
        let mut other = edge.clone();
        other.actor_id = 8;
        let mut again = edge.clone();
        again.recast = true;
        assert_eq!(
            warned(target(), &seen, &seen, &warning, &[basic, other, again]),
            casts
        );

        // An edge that reached the client one snapshot before the warning accounts for it
        // as well; one from long ago does not.
        for (wait, stands_in) in [(0.0, false), (2.0, true)] {
            let mut feed = Feed::new(target());
            feed.snap_with(&[], &seen, std::slice::from_ref(&edge));
            feed.now += wait;
            feed.snap_with(&[], &seen, &[]);
            let taken = feed.snap_with(std::slice::from_ref(&warning), &seen, &[]);
            assert_eq!(!taken.casts.is_empty(), stands_in, "{wait}");
        }

        // A warning already well into its time was not cast just now.
        let late = with(&warning, |e| {
            e.remaining_secs = tail + telegraph - FRESH_SECS - 0.05;
        });
        assert_eq!(warned(target(), &seen, &seen, &late, &[]), []);
        // The beam or the wave itself is no warning.
        let fired = with(&warning, |e| {
            e.kind = if id == SkillId::DawnRay {
                K::Beam
            } else {
                K::Bolt
            };
        });
        assert_eq!(warned(target(), &seen, &seen, &fired, &[]), []);

        // The hero must be alive, visible now and visible in the snapshot before; a
        // warning without an owner belongs to nobody the client sees.
        let hidden = [HeroSeen {
            visible: false,
            ..hero(class)
        }];
        let dead = [HeroSeen {
            alive: false,
            ..hero(class)
        }];
        assert_eq!(warned(target(), &hidden, &seen, &warning, &[]), []);
        assert_eq!(warned(target(), &[], &seen, &warning, &[]), []);
        assert_eq!(warned(target(), &seen, &hidden, &warning, &[]), []);
        assert_eq!(warned(target(), &seen, &dead, &warning, &[]), []);
        assert_eq!(warned(target(), &seen, &[], &warning, &[]), []);
        let ownerless = with(&warning, |e| e.owner_id = 0);
        assert_eq!(warned(target(), &seen, &seen, &ownerless, &[]), []);
        // A hero whose kit does not hold the skill has no slot to cast it from.
        let stranger = [hero(HeroClass::Frostguard)];
        assert_eq!(warned(target(), &stranger, &stranger, &warning, &[]), []);

        // The first snapshot of a stream is a baseline: what it holds may be old.
        let mut feed = Feed::new(target());
        assert_eq!(
            feed.snap_with(std::slice::from_ref(&warning), &seen, &[]),
            Taken::default()
        );
        // So is the first one after the stream stalled, and one outside a running match.
        let mut feed = Feed::new(target());
        feed.snap_with(&[], &seen, &[]);
        feed.now += 1.0;
        assert_eq!(
            feed.snap_with(std::slice::from_ref(&warning), &seen, &[]),
            Taken::default()
        );
        let mut feed = Feed::new(target());
        feed.snap_with(&[], &seen, &[]);
        feed.now += STEP;
        let paused = feed.memory.take(&Frame {
            round: ROUND,
            now: feed.now,
            running: false,
            effects: std::slice::from_ref(&warning),
            heroes: &seen,
            casts: &[],
            registry: Some(&feed.registry),
        });
        assert_eq!(paused, Taken::default());

        // A registry without the row knows no windup to release: nothing is reported.
        assert_eq!(
            warned(SkillPresentation::default(), &seen, &seen, &warning, &[]),
            []
        );
    }

    // A telegraph of another kind of skill is no stand-in: its cast edge releases nothing
    // that waits for a warning.
    for (class, id) in [
        (HeroClass::Cinderforge, SkillId::FurnaceBreath),
        (HeroClass::Orbitwright, SkillId::OrbitalCollapse),
        (HeroClass::Frostguard, SkillId::WinterDivide),
    ] {
        let seen = [hero(class)];
        let fuse = with(&effect(51, id, K::BeamWarning), |e| {
            e.remaining_secs = category::telegraph_secs(id).unwrap_or(4.0) + 0.15;
        });
        assert_eq!(
            warned(target(), &seen, &seen, &fuse, &[]),
            [],
            "{}",
            id.id()
        );
    }
}

/// A stage event of `effect` with a living, visible owner.
fn event(effect: &SkillEffectState, change: StageChange) -> StageEvent {
    StageEvent {
        effect: effect.clone(),
        change,
        owner: Some(owner()),
    }
}

#[test]
fn kind_flip_pops_only_after_an_observed_warning() {
    let registry = target();
    let heroes = [hero(HeroClass::Riftshot)];
    let warning = with(&effect(60, SkillId::HorizonWave, K::BeamWarning), |e| {
        e.remaining_secs = 11.0;
    });
    let wave = with(&warning, |e| {
        e.kind = K::Bolt;
        e.position = [2.0, 3.5];
        e.end = [2.0, 4.5];
    });
    let shots = |snapshots: &[&[SkillEffectState]]| {
        let mut feed = Feed::new(target());
        snapshots
            .iter()
            .flat_map(|effects| feed.snap_with(effects, &heroes, &[]).events)
            .map(|event| {
                let burst = accents::stage_burst(&registry, &event, 0.25);
                (accents::stage_shot(&registry, &event), burst)
            })
            .collect::<Vec<_>>()
    };
    // The wave that was a warning one snapshot ago pops, at its replicated position.
    let popped = shots(&[
        std::slice::from_ref(&warning),
        std::slice::from_ref(&wave),
        std::slice::from_ref(&wave),
    ]);
    assert_eq!(popped.len(), 1);
    assert_eq!(popped[0].0, Some(OneShot::ArmPop));
    assert!(!popped[0].1.is_empty());
    for spec in &popped[0].1 {
        assert_eq!((spec.event_id, spec.source), (60, ParticleSource::Stage));
        assert!(spec.reach(Vec3::new(2.0, 0.25, 3.5)) <= wave.radius + 1e-3);
        assert!(spec.origin.y >= 0.25);
    }
    // A wave first seen as a wave, and a warning that disappears without one, pop nothing.
    assert_eq!(shots(&[&[], std::slice::from_ref(&wave)]).len(), 0);
    assert_eq!(shots(&[std::slice::from_ref(&warning), &[]]).len(), 0);
    // The ray becomes a beam: that flip is carried by the body and pops nothing.
    let ray = effect(61, SkillId::DawnRay, K::BeamWarning);
    let flip = event(
        &with(&ray, |e| e.kind = K::Beam),
        step(Transition::KindFlipped),
    );
    assert_eq!(accents::stage_shot(&registry, &flip), None);
    assert!(accents::stage_burst(&registry, &flip, 0.0).is_empty());
}

#[test]
fn stage_oneshots_follow_the_row_of_the_effect() {
    let registry = target();
    let shot = |effect: &SkillEffectState, change: StageChange| {
        accents::stage_shot(&registry, &event(effect, change))
    };
    let end = |kind: EndKind| StageChange::Ended(kind);

    // Transitions: the engine's one-shot for each, on any row with a body.
    let trap = effect(1, SkillId::WildTraps, K::Trap);
    assert_eq!(shot(&trap, step(Transition::Armed)), Some(OneShot::ArmPop));
    let cage = effect(2, SkillId::IronBoundary, K::Cage);
    for side in 0..5 {
        assert_eq!(
            shot(&cage, step(Transition::SegmentBroken(side))),
            Some(OneShot::SegmentSnap(side))
        );
    }
    let ember = effect(3, SkillId::WanderingEmber, K::Bolt);
    for change in [Transition::Turned, Transition::Renewed] {
        assert_eq!(shot(&ember, step(change)), Some(OneShot::TurnSpark));
    }

    // Ends: the row's `expire` kind, and only for the end it names.
    let mut drawn = Vec::new();
    for (id, profile) in registry.rows() {
        let Some(skill) = SkillId::from_id(id) else {
            continue;
        };
        let bodies = category::own_kinds(skill)
            .first()
            .map(|kind| (*kind, profile.body.as_ref()))
            .into_iter()
            .chain(
                category::aux_kinds(skill)
                    .iter()
                    .map(|kind| (*kind, profile.aux.get(category::kind_id(*kind)))),
            );
        for (kind, body) in bodies {
            // Received whole: the axis of a cone is as long as its cast range.
            let range = shared::loadout::skill(skill).ability.cast_range;
            let seen = with(&effect(9, skill, kind), |e| {
                e.end[1] = e.position[1] + range.max(6.0);
            });
            let expire = body.map_or(ExpireKind::None, |body| body.expire);
            for ending in [EndKind::TrueExpiry, EndKind::Released, EndKind::Detonated] {
                let expected = match (ending, expire) {
                    (EndKind::TrueExpiry, ExpireKind::Fade) => Some(OneShot::Fade),
                    (EndKind::TrueExpiry, ExpireKind::Crumble) => Some(OneShot::Crumble),
                    (EndKind::Released, ExpireKind::Discharge) => Some(OneShot::Discharge),
                    (EndKind::Detonated, ExpireKind::Detonate) => Some(OneShot::Detonate),
                    _ => None,
                };
                assert_eq!(shot(&seen, end(ending)), expected, "{id} {ending:?}");
                if expected.is_some() {
                    drawn.push((id, expire.id()));
                }
            }
        }
    }
    drawn.sort();
    assert_eq!(
        drawn,
        [
            ("dawn_field", "detonate"),
            ("dawn_ray", "fade"),
            ("fault_line", "crumble"),
            ("furnace_breath", "discharge"),
            ("guiding_lantern", "fade"),
            ("iron_boundary", "fade"),
            ("mirror_guard", "discharge"),
            ("northwall", "fade"),
            ("orbital_collapse", "discharge"),
            ("orbital_field", "fade"),
            ("wild_traps", "crumble"),
            ("winter_divide", "fade"),
        ]
    );

    // Rule F: a cone the fog cut down to a line is not known whole. It ends without a
    // flash, whatever was observed of its owner.
    let cone = effect(7, SkillId::FurnaceBreath, K::BeamWarning);
    let range = shared::loadout::skill(cone.skill).ability.cast_range;
    let whole = with(&cone, |e| e.end[1] = e.position[1] + range);
    let cut = with(&cone, |e| e.end[1] = e.position[1] + range - 0.06);
    assert_eq!(
        shot(&whole, end(EndKind::Released)),
        Some(OneShot::Discharge)
    );
    for ending in [EndKind::Released, EndKind::TrueExpiry, EndKind::Detonated] {
        assert_eq!(shot(&cut, end(ending)), None, "{ending:?}");
        assert!(accents::stage_burst(&registry, &event(&cut, end(ending)), 0.0).is_empty());
    }

    // The burst: the colours of the row the effect's `skill` names, inside the replicated
    // geometry.
    let collapse = with(&effect(4, SkillId::OrbitalCollapse, K::BeamWarning), |e| {
        e.end = e.position;
        e.radius = 5.0;
    });
    let released = event(&collapse, end(EndKind::Released));
    let burst = accents::stage_burst(&registry, &released, 0.5);
    let look = registry
        .look(CastKey::Skill(SkillKey::Modular(SkillId::OrbitalCollapse)))
        .unwrap();
    let geo = super::super::geometry::boundary_shape(collapse.skill, collapse.kind, &collapse);
    assert_eq!(
        burst,
        accents::stage_oneshot(OneShot::Discharge, &look.palette, &geo, 0.5, 4)
    );
    assert!(!burst.is_empty() && burst.len() <= accents::STAGE_MAX);
    for spec in &burst {
        assert_eq!(spec.source, ParticleSource::Stage);
        assert!(spec.reach(Vec3::new(2.0, 0.5, 3.0)) <= 5.0 + 1e-3);
    }
    // Another skill on the same geometry is painted with its own row.
    let field = with(&collapse, |e| {
        e.skill = SkillId::OrbitalField;
        e.kind = K::Field;
    });
    let faded = accents::stage_burst(&registry, &event(&field, end(EndKind::TrueExpiry)), 0.5);
    assert!(!faded.is_empty());
    assert_ne!(faded[0].color, burst[0].color);

    // A registry without the rows draws nothing for any event of their effects.
    let empty = SkillPresentation::default();
    for (seen, change) in [
        (&trap, step(Transition::Armed)),
        (&cage, step(Transition::SegmentBroken(0))),
        (&ember, step(Transition::Turned)),
        (&collapse, end(EndKind::Released)),
        (&field, end(EndKind::TrueExpiry)),
        (
            &effect(5, SkillId::DawnField, K::Field),
            end(EndKind::Detonated),
        ),
    ] {
        let event = event(seen, change);
        assert_eq!(accents::stage_shot(&empty, &event), None);
        assert!(accents::stage_burst(&empty, &event, 0.0).is_empty());
    }
    // Neither does an auxiliary object, whatever is reported for it.
    let orb = effect(6, SkillId::OrbitalCommand, K::Orb);
    assert_eq!(shot(&orb, end(EndKind::TrueExpiry)), None);
}

/// AC10: a strip is replicated with its caster-side end first. When the viewer does not see
/// its owner, its fade is the same wisps in the same order whichever end that is, and the
/// ground is sampled where its body stands.
#[test]
fn a_hidden_owner_strip_fades_the_same_from_either_end() {
    let registry = target();
    for (skill, kind) in [
        (SkillId::DawnRay, K::Beam),
        (SkillId::WinterDivide, K::BeamWarning),
    ] {
        let strip = with(&effect(9, skill, kind), |e| {
            e.owner_id = 0;
            e.position = [2.0, 3.0];
            e.end = [8.0, -1.0];
        });
        let turned = with(&strip, |e| std::mem::swap(&mut e.position, &mut e.end));
        let wisps = |effect: &SkillEffectState| {
            let event = StageEvent {
                effect: effect.clone(),
                change: StageChange::Ended(EndKind::TrueExpiry),
                owner: None,
            };
            assert_eq!(
                accents::stage_shot(&registry, &event),
                Some(OneShot::Fade),
                "{}",
                skill.id()
            );
            let mut wisps: Vec<_> = accents::stage_burst(&registry, &event, 0.25)
                .into_iter()
                .map(|wisp| {
                    (
                        (wisp.origin * 1e3).round().to_array().map(|v| v as i64),
                        (wisp.delay * 1e4).round() as i64,
                    )
                })
                .collect();
            wisps.sort();
            wisps
        };
        let drawn = wisps(&strip);
        assert!(drawn.iter().any(|wisp| wisp.1 > 0), "{}", skill.id());
        assert_eq!(drawn, wisps(&turned), "{}", skill.id());
        assert_eq!(
            super::super::bodies::root_at(&strip),
            super::super::bodies::root_at(&turned)
        );
    }
}

#[test]
fn a_released_telegraph_links_to_its_receipts() {
    let registry = target();
    let hit = |receipt: u64, source: u64, slot: u8| ConfirmedHit {
        receipt,
        source,
        slot,
        position: Vec3::new(9.0, 0.5, 3.0),
    };
    let ground = Vec3::new(2.0, 0.5, 3.0);
    let starts = |link: &[ParticleSpec]| -> Vec<Vec2> {
        link.iter()
            .map(|spec| {
                spec.pose_at(0.0, true, Quat::IDENTITY)
                    .translation
                    .truncate()
            })
            .collect()
    };
    for (class, id, from) in [
        // The riposte is resolved from the barrier, the breath from its caster.
        (HeroClass::Edgeweaver, SkillId::MirrorGuard, ground),
        (HeroClass::Cinderforge, SkillId::FurnaceBreath, HOME),
    ] {
        let slot = slot_of(class, id);
        let seen = effect(1, id, category::own_kinds(id)[0]);
        let owner = OwnerSeen {
            slot: Some(slot),
            ..owner()
        };
        let mut book = LinkBook::default();
        book.turn(ROUND, 10);
        // Nothing waits before the release, and nothing is drawn without a receipt.
        assert_eq!(book.link(&hit(1, OWNER, slot)), None);
        book.release(&registry, &seen, &owner, ground);
        assert_eq!(book.link(&hit(1, 8, slot)), None);
        assert_eq!(book.link(&hit(1, OWNER, (slot + 1) % 4)), None);
        let (link, local) = book.link(&hit(2, OWNER, slot)).unwrap();
        assert!(!local);
        assert_eq!(link.len(), accents::LINK_MAX);
        for (spec, start) in link.iter().zip(starts(&link)) {
            assert_eq!((spec.event_id, spec.source), (2, ParticleSource::Link));
            let expected = crate::world2d::simulation_xz_to_render_xy(from);
            assert!(start.distance(expected) < 0.2, "{}", id.id());
        }
        // At most three receipts, and only in the snapshot of the release or the two
        // after it.
        assert!(book.link(&hit(3, OWNER, slot)).is_some());
        assert!(book.link(&hit(4, OWNER, slot)).is_some());
        assert_eq!(book.link(&hit(5, OWNER, slot)), None);
        for (later, linked) in [(2, true), (3, false)] {
            let mut book = LinkBook::default();
            book.turn(ROUND, 10);
            book.release(&registry, &seen, &owner, ground);
            for tick in 1..=later {
                book.turn(ROUND, 10 + tick);
            }
            assert_eq!(book.link(&hit(1, OWNER, slot)).is_some(), linked);
        }
        // An owner whose kit does not hold the skill has no receipts to wait for.
        let mut book = LinkBook::default();
        book.turn(ROUND, 10);
        book.release(
            &registry,
            &seen,
            &OwnerSeen {
                slot: None,
                ..owner
            },
            ground,
        );
        assert!((0..4).all(|slot| book.link(&hit(1, OWNER, slot)).is_none()));
        // The cast edge of such a skill opens nothing: its hit is seconds away.
        let mut book = LinkBook::default();
        book.turn(ROUND, 10);
        book.open(
            &registry,
            &SkillCastObserved {
                actor_id: OWNER,
                key: CastKey::Skill(SkillKey::Modular(id)),
                slot,
                sequence: 4,
                recast: false,
                origin: HOME,
                position: HOME,
                yaw: None,
                forward: Vec3::NEG_Z,
                local: false,
            },
        );
        assert_eq!(book.link(&hit(1, OWNER, slot)), None, "{}", id.id());
        // A registry without the row links nothing.
        let mut book = LinkBook::default();
        book.turn(ROUND, 10);
        book.release(&SkillPresentation::default(), &seen, &owner, ground);
        assert_eq!(book.link(&hit(1, OWNER, slot)), None);
    }
    // The collapse names no link.
    let collapse = effect(1, SkillId::OrbitalCollapse, K::BeamWarning);
    let mut book = LinkBook::default();
    book.turn(ROUND, 10);
    book.release(&registry, &collapse, &owner(), ground);
    assert_eq!(book.link(&hit(1, OWNER, 3)), None);
}

/// An app with the tracker and the emitters it feeds, a running round and one hero of
/// `class` that owns every effect of these tests.
fn stage(registry: SkillPresentation, class: HeroClass) -> (App, Entity) {
    let mut app = App::new();
    app.insert_resource(registry)
        .insert_resource(GameStateSnapshot {
            meta: shared::protocol::SnapshotMeta::new(3, 1, 1),
            state: GameState::Running,
            ..default()
        })
        .insert_resource(VfxClock {
            now: 100.0,
            delta: 0.016,
        })
        .init_resource::<EffectMemory>()
        .add_message::<SkillCastObserved>()
        .add_message::<StageEvent>()
        .add_message::<ConfirmedHit>()
        .add_message::<SkillBurst>()
        .add_message::<ConfirmedBurst>()
        .add_systems(Update, track_effects)
        .add_systems(
            PostUpdate,
            (
                accents::emit_cast,
                accents::emit_stage_oneshots,
                accents::emit_links,
            )
                .chain(),
        );
    let hero = app
        .world_mut()
        .spawn((
            NetworkPlayerId(OWNER),
            Transform::from_translation(HOME),
            InheritedVisibility::VISIBLE,
            CombatStats::default(),
            NetworkHeroClass(class),
            PlayerLoadout(Some(LoadoutState::default())),
        ))
        .id();
    (app, hero)
}

/// Applies a snapshot one interval later, runs the frame and returns its bursts.
fn apply(app: &mut App, effects: &[SkillEffectState]) -> Vec<Vec<ParticleSpec>> {
    app.world_mut().resource_mut::<VfxClock>().now += STEP;
    let mut game = app.world_mut().resource_mut::<GameStateSnapshot>();
    game.meta.snapshot_tick += 1;
    game.skill_effects = effects.to_vec();
    frame(app)
}

/// Runs a frame without a new snapshot.
fn frame(app: &mut App) -> Vec<Vec<ParticleSpec>> {
    app.update();
    app.world_mut()
        .resource_mut::<Messages<SkillBurst>>()
        .drain()
        .map(|burst| burst.0)
        .collect()
}

fn sources(bursts: &[Vec<ParticleSpec>]) -> Vec<ParticleSource> {
    bursts
        .iter()
        .map(|burst| {
            let source = burst[0].source;
            assert!(burst.iter().all(|spec| spec.source == source));
            source
        })
        .collect()
}

#[test]
fn the_tracker_feeds_the_one_shots_and_the_release_link() {
    let class = HeroClass::Cinderforge;
    let (mut app, hero) = stage(target(), class);
    let range = skill(SkillId::FurnaceBreath).ability.cast_range;
    let breath = with(&effect(70, SkillId::FurnaceBreath, K::BeamWarning), |e| {
        e.position = [HOME.x, HOME.z];
        e.end = [HOME.x + range, HOME.z];
        e.remaining_secs = 0.3;
    });
    // First sight, and frames between two snapshots: nothing.
    assert_eq!(apply(&mut app, std::slice::from_ref(&breath)), NOTHING);
    assert_eq!(frame(&mut app), NOTHING);
    let last = with(&breath, |e| e.remaining_secs = 0.25);
    assert_eq!(apply(&mut app, std::slice::from_ref(&last)), NOTHING);
    // A frame that brings no snapshot ends nothing, however long it takes.
    app.world_mut().resource_mut::<VfxClock>().now += 0.1;
    assert_eq!(frame(&mut app), NOTHING);
    assert!(
        app.world()
            .resource::<EffectMemory>()
            .get(EffectKey::Runtime(70))
            .is_some()
    );

    // The firing tick removes the telegraph: the cone discharges, once, and the receipt
    // of that snapshot is linked from the smith.
    let slot = slot_of(class, SkillId::FurnaceBreath);
    app.world_mut().write_message(ConfirmedHit {
        receipt: 90,
        source: OWNER,
        slot,
        position: HOME + Vec3::X * 6.0,
    });
    let bursts = apply(&mut app, &[]);
    assert_eq!(
        sources(&bursts),
        [ParticleSource::Stage, ParticleSource::Link]
    );
    let look = SkillPresentation::target();
    let look = look
        .look(CastKey::Skill(SkillKey::Modular(SkillId::FurnaceBreath)))
        .unwrap();
    let cone = super::super::geometry::boundary_shape(last.skill, last.kind, &last);
    assert!(matches!(
        cone,
        super::super::geometry::GeoShape::Sector { .. }
    ));
    let mut discharge = accents::stage_oneshot(OneShot::Discharge, &look.palette, &cone, 0.0, 70);
    // A remote hero's effect is admitted after the local hero's.
    for spec in &mut discharge {
        spec.sort_key = 1;
    }
    assert_eq!(bursts[0], discharge);
    assert!(bursts[1].iter().all(|spec| spec.event_id == 90));
    // A release is decoration: it never enters the pool as a confirmed hit.
    assert!(
        app.world()
            .resource::<Messages<ConfirmedBurst>>()
            .is_empty()
    );
    assert_eq!(frame(&mut app), NOTHING);
    assert_eq!(apply(&mut app, &[]), NOTHING);

    // A telegraph that vanishes early, or whose owner died with it, discharges nothing.
    let early = with(&breath, |e| e.id = 71);
    apply(&mut app, std::slice::from_ref(&early));
    assert_eq!(apply(&mut app, &[]), NOTHING);
    let doomed = with(&last, |e| e.id = 72);
    apply(&mut app, std::slice::from_ref(&doomed));
    app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp = 0.0;
    assert_eq!(apply(&mut app, &[]), NOTHING);
    app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp = 100.0;

    // A hidden owner: the server withholds nothing here, but the client draws no release
    // at a hero it does not show.
    let unseen = with(&last, |e| e.id = 73);
    apply(&mut app, std::slice::from_ref(&unseen));
    app.world_mut()
        .entity_mut(hero)
        .insert(InheritedVisibility::HIDDEN);
    assert_eq!(apply(&mut app, &[]), NOTHING);
    app.world_mut()
        .entity_mut(hero)
        .insert(InheritedVisibility::VISIBLE);

    // A new round forgets the effects of the old one.
    let carried = with(&last, |e| e.id = 74);
    apply(&mut app, std::slice::from_ref(&carried));
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .meta
        .match_id = 2;
    assert_eq!(apply(&mut app, &[]), NOTHING);
}

/// The Thorn Volley recast strikes at once, but while the spike of the first cast is in
/// the snapshot, or was in the one before, a receipt may be the spike's own. The emitter
/// shows the effects of every snapshot to its book, so a recast in that time opens no lash.
#[test]
fn the_emitter_holds_the_recast_lash_until_the_spike_has_left_the_snapshots() {
    let class = HeroClass::Veilstalker;
    let slot = slot_of(class, SkillId::ThornVolley);
    let spike = with(&effect(90, SkillId::ThornVolley, K::Bolt), |e| {
        e.position = [HOME.x + 2.0, HOME.z];
        e.end = [HOME.x + 3.0, HOME.z];
    });
    let recast = SkillCastObserved {
        actor_id: OWNER,
        key: CastKey::Skill(SkillKey::Modular(SkillId::ThornVolley)),
        slot,
        sequence: 5,
        recast: true,
        origin: HOME,
        position: HOME,
        yaw: None,
        forward: Vec3::NEG_Z,
        local: false,
    };
    // (snapshots without the spike since it was last seen, whether the lash is drawn)
    for (since, lashed) in [(0, false), (1, false), (2, true)] {
        let (mut app, _) = stage(target(), class);
        assert_eq!(apply(&mut app, std::slice::from_ref(&spike)), NOTHING);
        for _ in 0..since {
            assert_eq!(apply(&mut app, &[]), NOTHING);
        }
        app.world_mut().write_message(recast.clone());
        app.world_mut().write_message(ConfirmedHit {
            receipt: 91,
            source: OWNER,
            slot,
            position: HOME + Vec3::X * 4.0,
        });
        let links = sources(&frame(&mut app))
            .into_iter()
            .filter(|source| *source == ParticleSource::Link)
            .count();
        assert_eq!(links, usize::from(lashed), "{since} snapshots later");
    }
}

#[test]
fn a_turn_and_a_renewal_of_one_snapshot_spark_once() {
    let (mut app, _) = stage(target(), HeroClass::Emberveil);
    let ember = with(&effect(80, SkillId::WanderingEmber, K::Bolt), |e| {
        e.end = [2.0, 4.0];
        e.remaining_secs = 2.2;
    });
    assert_eq!(apply(&mut app, std::slice::from_ref(&ember)), NOTHING);
    let home = with(&ember, |e| {
        e.end = [2.0, 2.0];
        e.remaining_secs = 3.0;
    });
    let bursts = apply(&mut app, std::slice::from_ref(&home));
    assert_eq!(sources(&bursts), [ParticleSource::Stage]);
    assert!(bursts[0].len() <= accents::STAGE_MAX);
    assert_eq!(apply(&mut app, std::slice::from_ref(&home)), NOTHING);
    // The ember ends without a word wherever it ends.
    assert_eq!(apply(&mut app, &[]), NOTHING);

    // The cage loses two sides in one snapshot: each side snaps.
    let (mut app, _) = stage(target(), HeroClass::Chainkeeper);
    let cage = with(&effect(81, SkillId::IronBoundary, K::Cage), |e| {
        e.radius = 6.0
    });
    apply(&mut app, std::slice::from_ref(&cage));
    let broken = with(&cage, |e| e.consumed_segments = 0b00101);
    let bursts = apply(&mut app, std::slice::from_ref(&broken));
    assert_eq!(
        sources(&bursts),
        [ParticleSource::Stage, ParticleSource::Stage]
    );
    assert_ne!(bursts[0][0].origin, bursts[1][0].origin);

    // Without the rows the same snapshots draw nothing.
    let (mut app, _) = stage(SkillPresentation::default(), HeroClass::Chainkeeper);
    apply(&mut app, std::slice::from_ref(&cage));
    assert_eq!(apply(&mut app, std::slice::from_ref(&broken)), NOTHING);
}

#[test]
fn the_tracker_reports_a_hidden_cast_edge_to_the_accent() {
    let class = HeroClass::Dawnweaver;
    let (mut app, _) = stage(target(), class);
    // A running stream with the hero in view.
    assert_eq!(apply(&mut app, &[]), NOTHING);
    let warning = with(&effect(85, SkillId::DawnRay, K::BeamWarning), |e| {
        e.position = [HOME.x, HOME.z];
        e.end = [HOME.x, HOME.z - 30.0];
        e.remaining_secs = 0.9;
    });
    // The snapshot shows the warning and no edge of the ray: the charge accent is drawn
    // from the warning, once.
    let bursts = apply(&mut app, std::slice::from_ref(&warning));
    assert_eq!(sources(&bursts), [ParticleSource::Accent]);
    assert!(bursts[0].iter().all(|spec| spec.event_id == 85));
    assert_eq!(frame(&mut app), NOTHING);
    let next = with(&warning, |e| e.remaining_secs = 0.85);
    assert_eq!(apply(&mut app, std::slice::from_ref(&next)), NOTHING);

    // With the edge in the same frame the accent is the edge's alone.
    let (mut app, _) = stage(target(), class);
    apply(&mut app, &[]);
    app.world_mut().write_message(SkillCastObserved {
        actor_id: OWNER,
        key: CastKey::Skill(SkillKey::Modular(SkillId::DawnRay)),
        slot: slot_of(class, SkillId::DawnRay),
        sequence: 31,
        recast: false,
        origin: HOME,
        position: HOME,
        yaw: None,
        forward: Vec3::NEG_Z,
        local: false,
    });
    let bursts = apply(&mut app, std::slice::from_ref(&warning));
    assert_eq!(sources(&bursts), [ParticleSource::Accent]);
    assert!(bursts[0].iter().all(|spec| spec.event_id == 31));
}

fn join(class: HeroClass) -> ClientPacket {
    ClientPacket::Join {
        handheld: Default::default(),
        prematch: false,
        team: shared::map::Team::Green,
        character: CharacterChoice::Ipfs,
        hero_class: class,
        avatar: None,
        sprite_character: None,
        session_id: None,
        passport_ticket: None,
    }
}

/// Every `remaining_secs` the in-process authority replicated for the hero's own effect of
/// `id` and `kind`, one value per snapshot of `dt` seconds, until the effect left the
/// snapshot or changed its kind.
fn replicated_remaining(class: HeroClass, id: SkillId, kind: K, dt: f32) -> Vec<f32> {
    let mut session = PracticeSession::new(Instant::now());
    session.command(join(class));
    session.command(ClientPacket::Practice {
        command: PracticeCommand::ClearBots,
    });
    // Let the kit settle (the Orbitwright's orb is created by the first ticks).
    for _ in 0..4 {
        session.advance(dt);
    }
    let hero = &session.world.players[&LOCAL_ADDR].hero;
    let (owner, aim) = (hero.identity.id, [hero.x + 3.0, hero.z]);
    session.command(ClientPacket::CastSkill {
        slot: slot_of(class, id),
        aim,
        server_epoch: EPOCH,
        match_id: 1,
        request_id: 1,
    });
    let mut seen = Vec::new();
    for _ in 0..400 {
        let ServerPacket::Snapshot { skill_effects, .. } = session.snapshot() else {
            panic!("practice publishes a snapshot");
        };
        let own = skill_effects
            .iter()
            .find(|e| e.owner_id == owner && e.skill == id && e.kind == kind);
        match own {
            Some(effect) => seen.push(effect.remaining_secs),
            None if seen.is_empty() => {}
            None => break,
        }
        session.advance(dt);
    }
    assert!(!seen.is_empty(), "{} was not replicated", id.id());
    seen
}

/// Parity with the in-process authority: the three telegraphs that fire on their own are
/// removed by their firing tick while `remaining_secs` still reads the tail, so the last
/// replicated value lies within one snapshot above the derived `tail_secs`. The release
/// rule and the fill of the telegraph both rest on it.
#[test]
fn the_last_replicated_remaining_of_a_fuse_is_its_tail() {
    for (class, id) in [
        (HeroClass::Cinderforge, SkillId::FurnaceBreath),
        (HeroClass::Edgeweaver, SkillId::MirrorGuard),
        (HeroClass::Orbitwright, SkillId::OrbitalCollapse),
    ] {
        let kind = category::own_kinds(id)[0];
        let (telegraph, tail) = category::telegraph_secs(id)
            .zip(category::tail_secs(id))
            .unwrap();
        for dt in [0.05, 1.0 / 60.0] {
            let seen = replicated_remaining(class, id, kind, dt);
            let (first, last) = (seen[0], *seen.last().unwrap());
            // Replicated from the accepted cast with its whole telegraph ahead.
            assert!(
                (first - (tail + telegraph)).abs() < 1e-3,
                "{} {first}",
                id.id()
            );
            assert!(
                (tail - 1e-3..=tail + 0.05 + 1e-3).contains(&last),
                "{} at {dt}: last {last}, tail {tail}",
                id.id()
            );
            // The whole telegraph is replicated without a gap.
            let expected = (telegraph / dt).round() as usize;
            assert!(
                seen.len().abs_diff(expected) <= 1,
                "{} {}",
                id.id(),
                seen.len()
            );
            // That last value is inside the release window, and it completes the fill.
            let last_seen = with(&effect(1, id, kind), |e| e.remaining_secs = last);
            assert_eq!(
                ended(&last_seen, seen_out()),
                EndKind::Released,
                "{}",
                id.id()
            );
            assert!(view(&last_seen).progress >= 1.0 - (dt + 1e-3) / telegraph);
        }
    }
}

/// The same parity for the two skills that warn: the warning flips while `remaining_secs`
/// reads the derived tail.
#[test]
fn the_last_replicated_remaining_of_a_warning_is_its_tail() {
    for (class, id) in [
        (HeroClass::Dawnweaver, SkillId::DawnRay),
        (HeroClass::Riftshot, SkillId::HorizonWave),
    ] {
        let (telegraph, tail) = category::telegraph_secs(id)
            .zip(category::tail_secs(id))
            .unwrap();
        let seen = replicated_remaining(class, id, K::BeamWarning, 0.05);
        let (first, last) = (seen[0], *seen.last().unwrap());
        assert!(
            (first - (tail + telegraph)).abs() < 1e-3,
            "{} {first}",
            id.id()
        );
        assert!(
            (tail - 1e-3..=tail + 0.05 + 1e-3).contains(&last),
            "{}: last {last}, tail {tail}",
            id.id()
        );
    }
}
