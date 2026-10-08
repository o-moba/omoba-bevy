use super::super::accents::{self, CastContext, LinkBook, Palette};
use super::super::geometry::{self, AreaContext, GeoShape};
use super::super::vocab::ParticleShape;
use super::super::{SkillPresentation, tests::target::target};
use super::*;
use crate::combat_feedback::ConfirmedHit;
use crate::game_vfx::{ParticleSource, ParticleSpec, SkillBurst, unit_radius};
use crate::sprite::PlayerVisualMode;
use shared::PlayerActionKind;
use shared::loadout::{CoreId, EffectVisualKind, SkillEffectState, SkillId, SkillSlotState};

const ROUND: Option<(u64, u64)> = Some((3, 1));
const HOME: Vec3 = Vec3::new(2.0, 0.5, -4.0);
const AWAY: Vec3 = Vec3::new(8.0, 0.5, -4.0);

fn entity() -> Entity {
    World::new().spawn_empty().id()
}

fn hero(class: HeroClass) -> Sighting<'static> {
    Sighting {
        actor: entity(),
        actor_id: 7,
        local: false,
        visible: true,
        alive: true,
        position: HOME,
        forward: Vec3::NEG_Z,
        class,
        loadout: None,
        action: PlayerCosmeticAction::default(),
        facing: PlayerActionFacing::default(),
        utility: UtilityState::default(),
    }
}

/// The hero with action `sequence` accepted on `slot`.
fn acted<'a>(mut hero: Sighting<'a>, sequence: u64, slot: u8) -> Sighting<'a> {
    hero.action = PlayerCosmeticAction {
        sequence,
        kind: PlayerActionKind::Cast,
        slot,
    };
    hero.facing = PlayerActionFacing {
        sequence,
        yaw: None,
    };
    hero
}

/// The hero after the server relocated it to `to`.
fn relocated<'a>(mut hero: Sighting<'a>, to: Vec3) -> Sighting<'a> {
    hero.position = to;
    hero.utility.dash_sequence += 1;
    hero
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

/// A hero that was seen once and then casts `skill` as action 2.
fn first_cast(class: HeroClass, skill: SkillId) -> (CastObserver, Sighting<'static>) {
    let mut observer = CastObserver::default();
    let hero = hero(class);
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 1, 0)]),
        Observed::default()
    );
    (observer, acted(hero, 2, slot_of(class, skill)))
}

fn effect(id: u64, owner: u64, skill: SkillId, at: Vec3) -> SkillEffectState {
    SkillEffectState {
        id,
        owner_id: owner,
        owner_team: shared::map::Team::Green,
        skill,
        kind: EffectVisualKind::Trap,
        position: [at.x, at.z],
        end: [at.x, at.z],
        radius: 1.0,
        remaining_secs: 5.0,
        armed: false,
        consumed_segments: 0,
    }
}

/// Where a particle is on the ground when it starts and when it ends.
fn ground_span(spec: &ParticleSpec) -> [Vec2; 2] {
    [0.0, spec.lifetime].map(|age| {
        spec.pose_at(age, true, Quat::IDENTITY)
            .translation
            .truncate()
    })
}

fn sources(specs: &[ParticleSpec]) -> Vec<ParticleSource> {
    let mut sources: Vec<_> = specs.iter().map(|spec| spec.source).collect();
    sources.dedup();
    sources
}

#[test]
fn cast_keys_follow_the_accepted_recipe() {
    let skill = |id: SkillId| CastKey::Skill(SkillKey::Modular(id));
    let stormfist = HeroClass::Stormfist;
    assert_eq!(
        CastKey::of(stormfist, None, slot_of(stormfist, SkillId::ThunderKick)),
        Some(skill(SkillId::ThunderKick))
    );
    assert_eq!(
        CastKey::of(HeroClass::Warrior, None, 0),
        Some(CastKey::Skill(
            SkillKey::from_id(HeroClass::Warrior.ability(SkillSlot::Q).id).unwrap()
        ))
    );
    // A basic attack belongs to the core of the kit; a legacy class has only itself.
    assert_eq!(
        CastKey::of(stormfist, None, BASIC_ATTACK_ACTION_SLOT),
        Some(CastKey::Basic(stormfist))
    );
    assert_eq!(
        CastKey::of(HeroClass::Mage, None, BASIC_ATTACK_ACTION_SLOT),
        Some(CastKey::Basic(HeroClass::Mage))
    );
    // The recipe decides, not the default slot of a skill.
    let mut recipe = CoreId::Stormfist.preset();
    recipe.skills[0] = SkillId::DawnRay;
    let mixed = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    assert_eq!(
        CastKey::of(stormfist, Some(&mixed), 0),
        Some(skill(SkillId::DawnRay))
    );
    assert_eq!(CastKey::of(stormfist, Some(&mixed), 4), None);
    // A recipe of another class is malformed and selects nothing.
    assert_eq!(CastKey::of(HeroClass::Riftshot, Some(&mixed), 0), None);
    assert_eq!(
        CastKey::of(HeroClass::Riftshot, Some(&mixed), BASIC_ATTACK_ACTION_SLOT),
        None
    );
}

#[test]
fn observer_baselines_on_reveal() {
    let mut observer = CastObserver::default();
    let hero = hero(HeroClass::Stormfist);
    // First sight: whatever the hero did before is not replayed.
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 9, 1)]),
        Observed::default()
    );
    let seen = observer.observe(ROUND, true, [acted(hero, 10, 1)]);
    assert_eq!(seen.casts.len(), 1);
    assert_eq!((seen.casts[0].sequence, seen.casts[0].slot), (10, 1));

    // Hidden while it acts, then revealed with a newer action: the reveal is a baseline.
    let mut hidden = acted(hero, 11, 1);
    hidden.visible = false;
    assert_eq!(observer.observe(ROUND, true, [hidden]), Observed::default());
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 12, 1)]),
        Observed::default()
    );
    assert_eq!(
        observer
            .observe(ROUND, true, [acted(hero, 13, 1)])
            .casts
            .len(),
        1
    );

    // A hero that left the client's view and returns is seen for the first time again.
    assert_eq!(observer.observe(ROUND, true, []), Observed::default());
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 20, 1)]),
        Observed::default()
    );
    // A new round baselines everybody, even with the action counter restarted.
    assert_eq!(
        observer.observe(Some((3, 2)), true, [acted(hero, 1, 1)]),
        Observed::default()
    );
    assert_eq!(
        observer
            .observe(Some((3, 2)), true, [acted(hero, 2, 1)])
            .casts
            .len(),
        1
    );
}

#[test]
fn a_new_connection_is_a_baseline_not_a_replay() {
    let mut observer = CastObserver::default();
    let hero = hero(HeroClass::Stormfist);
    observer.observe(ROUND, true, [acted(hero, 4, 0)]);
    // The connection dropped; the hero acted three times before the next snapshot of the
    // same round arrived.
    observer.forget();
    assert_eq!(
        observer.observe(ROUND, true, [relocated(acted(hero, 7, 0), AWAY)]),
        Observed::default()
    );
    assert_eq!(
        observer
            .observe(ROUND, true, [acted(hero, 8, 0)])
            .casts
            .len(),
        1
    );
}

#[test]
fn actions_count_once_and_never_backwards() {
    let mut observer = CastObserver::default();
    let hero = hero(HeroClass::Stormfist);
    // The cases of the legacy detector: the initial action, a duplicate, an older one.
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 12, 0)]),
        Observed::default()
    );
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 12, 0)]),
        Observed::default()
    );
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 11, 0)]),
        Observed::default()
    );
    // Not an edge either: the highest sequence seen stays the mark.
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 12, 0)]),
        Observed::default()
    );
    assert_eq!(
        observer
            .observe(ROUND, true, [acted(hero, 13, 0)])
            .casts
            .len(),
        1
    );
    assert_eq!(
        observer.observe(ROUND, true, [acted(hero, 13, 0)]),
        Observed::default()
    );
}

#[test]
fn two_actions_in_one_snapshot_collapse_into_one_message() {
    let mut observer = CastObserver::default();
    let hero = hero(HeroClass::Stormfist);
    observer.observe(ROUND, true, [acted(hero, 5, 0)]);
    // A cast and a basic attack inside one snapshot: only the later action is on the wire.
    let seen = observer.observe(ROUND, true, [acted(hero, 7, BASIC_ATTACK_ACTION_SLOT)]);
    assert_eq!(seen.casts.len(), 1);
    assert_eq!(seen.casts[0].sequence, 7);
    assert_eq!(seen.casts[0].key, CastKey::Basic(HeroClass::Stormfist));
    assert!(!seen.casts[0].recast);
}

#[test]
fn the_recast_flag_comes_from_the_previous_observation() {
    let class = HeroClass::Stormfist;
    let slot = slot_of(class, SkillId::EchoStrike);
    let offering = |open: bool| {
        let mut slots = [SkillSlotState::default(); 4];
        slots[usize::from(slot)].can_recast = open;
        LoadoutState { slots, ..default() }
    };
    let (closed, open) = (offering(false), offering(true));
    let mut observer = CastObserver::default();
    let mut hero = hero(class);
    hero.loadout = Some(&closed);
    observer.observe(ROUND, true, [acted(hero, 1, slot)]);
    // The first cast opens the recast in the same snapshot: it is still a first cast.
    hero.loadout = Some(&open);
    let first = observer.observe(ROUND, true, [acted(hero, 2, slot)]);
    assert!(!first.casts[0].recast);
    // The recast closes it in the same snapshot: it is a recast all the same.
    hero.loadout = Some(&closed);
    let second = observer.observe(ROUND, true, [acted(hero, 3, slot)]);
    assert!(second.casts[0].recast);
    // Another slot that offers nothing is a first cast, and so is a basic attack.
    hero.loadout = Some(&open);
    observer.observe(ROUND, true, [acted(hero, 3, slot)]);
    let other = observer.observe(ROUND, true, [acted(hero, 4, (slot + 1) % 4)]);
    assert!(!other.casts[0].recast);
    let basic = observer.observe(ROUND, true, [acted(hero, 5, BASIC_ATTACK_ACTION_SLOT)]);
    assert!(!basic.casts[0].recast);
}

#[test]
fn hidden_or_dead_actors_emit_nothing() {
    let class = HeroClass::Stormfist;
    for spoil in [
        (|hero: &mut Sighting| hero.alive = false) as fn(&mut Sighting),
        |hero: &mut Sighting| hero.visible = false,
    ] {
        let (mut observer, cast) = first_cast(class, SkillId::ThunderPulse);
        let mut spoiled = cast;
        spoil(&mut spoiled);
        let seen = observer.observe(ROUND, true, [spoiled]);
        assert!(seen.casts.is_empty());
        // Nothing is owed later: the action was seen and is not replayed.
        assert_eq!(observer.observe(ROUND, true, [cast]), Observed::default());
    }
    // Outside a running match nothing is reported, and nothing is replayed once it runs.
    let (mut observer, cast) = first_cast(class, SkillId::AnchorStep);
    assert_eq!(
        observer.observe(ROUND, false, [relocated(cast, AWAY)]),
        Observed::default()
    );
    assert_eq!(
        observer.observe(ROUND, true, [relocated(cast, AWAY)]),
        Observed::default()
    );
    // A dead hero is not reported as moved either (a respawn is not a dash).
    let (mut observer, cast) = first_cast(class, SkillId::AnchorStep);
    let mut dead = relocated(cast, AWAY);
    dead.alive = false;
    assert_eq!(observer.observe(ROUND, true, [dead]), Observed::default());
    // A hero the client never saw and does not see now moved nowhere it could draw.
    let mut observer = CastObserver::default();
    let mut unseen = hero(class);
    unseen.visible = false;
    observer.observe(ROUND, true, [unseen]);
    assert_eq!(
        observer.observe(ROUND, true, [relocated(unseen, AWAY)]),
        Observed::default()
    );
}

#[test]
fn classify_move_truth_table() {
    let rest = UtilityState {
        dash_remaining_secs: 6.0,
        ..default()
    };
    let recalled = UtilityState {
        recall_sequence: 1,
        ..rest
    };
    let dashed = UtilityState {
        dash_remaining_secs: shared::utility::DASH_COOLDOWN_SECS,
        ..default()
    };
    for edge in [false, true] {
        for capable in [false, true] {
            // A recall wins over everything, then the utility dash.
            assert_eq!(
                classify_move(&rest, &recalled, edge, capable),
                MoveCause::Recall
            );
            assert_eq!(
                classify_move(&default(), &dashed, edge, capable),
                MoveCause::UtilityDash
            );
            assert_eq!(
                classify_move(
                    &default(),
                    &UtilityState {
                        recall_sequence: 1,
                        ..dashed
                    },
                    edge,
                    capable
                ),
                MoveCause::Recall
            );
            // Only an own cast of a skill that can move its caster is a skill move.
            assert_eq!(
                classify_move(&rest, &rest, edge, capable),
                if edge && capable {
                    MoveCause::SkillCast
                } else {
                    MoveCause::Forced
                }
            );
        }
    }
    // The cooldown is aged locally between snapshots: a small correction upwards is not a
    // new dash.
    let corrected = UtilityState {
        dash_remaining_secs: 6.4,
        ..rest
    };
    assert_eq!(
        classify_move(&rest, &corrected, true, true),
        MoveCause::SkillCast
    );
    assert_eq!(
        classify_move(&rest, &corrected, false, true),
        MoveCause::Forced
    );
}

#[test]
fn a_utility_request_marks_the_dash_when_cooldowns_are_off() {
    // With cooldowns switched off the server reports no dash cooldown; the request the hero
    // made in the same snapshot is then the only sign that the dash was its own.
    let rest = UtilityState {
        last_request_id: 4,
        ..default()
    };
    let requested = UtilityState {
        last_request_id: 5,
        ..rest
    };
    for edge in [false, true] {
        assert_eq!(
            classify_move(&rest, &requested, edge, false),
            MoveCause::UtilityDash
        );
        // Without a request the same snapshot is a displacement by something else.
        assert_eq!(classify_move(&rest, &rest, edge, false), MoveCause::Forced);
    }
    // A cast that moves its caster is still the stronger evidence, and so is a recall.
    assert_eq!(
        classify_move(&rest, &requested, true, true),
        MoveCause::SkillCast
    );
    assert_eq!(
        classify_move(
            &rest,
            &UtilityState {
                recall_sequence: 1,
                ..requested
            },
            false,
            false
        ),
        MoveCause::Recall
    );

    // Through the observer: the dash keeps its own look and is not painted as a skid.
    let mut observer = CastObserver::default();
    let mut hero = hero(HeroClass::Stormfist);
    hero.utility = rest;
    observer.observe(ROUND, true, [hero]);
    let mut dashed = relocated(hero, AWAY);
    dashed.utility.last_request_id = 5;
    let seen = observer.observe(ROUND, true, [dashed]);
    assert_eq!(seen.moves.len(), 1);
    assert_eq!(seen.moves[0].cause, MoveCause::UtilityDash);
    assert_eq!(accents::move_burst(Some(&target()), &seen.moves[0]), None);
    // The next relocation of that hero without a request is forced again.
    let kicked = relocated(dashed, HOME);
    let seen = observer.observe(ROUND, true, [kicked]);
    assert_eq!(seen.moves[0].cause, MoveCause::Forced);
    assert!(accents::move_burst(Some(&target()), &seen.moves[0]).is_some());
}

#[test]
fn relocations_are_classified_from_what_the_snapshot_shows() {
    let class = HeroClass::Stormfist;
    // A leap of the hero's own cast: the cast is anchored where it was seen before.
    let (mut observer, cast) = first_cast(class, SkillId::AnchorStep);
    let seen = observer.observe(ROUND, true, [relocated(cast, AWAY)]);
    assert_eq!(
        seen.moves,
        [MoveObserved {
            actor_id: 7,
            from: Some(HOME),
            to: Some(AWAY),
            cause: MoveCause::SkillCast,
            skill: Some(SkillKey::Modular(SkillId::AnchorStep)),
            seed: 2,
            local: false,
        }]
    );
    assert_eq!((seen.casts[0].origin, seen.casts[0].position), (HOME, AWAY));
    // The same relocation without an action of its own was done to the hero.
    let (mut observer, cast) = first_cast(class, SkillId::AnchorStep);
    observer.observe(ROUND, true, [cast]);
    let seen = observer.observe(ROUND, true, [relocated(cast, AWAY)]);
    assert_eq!(seen.casts, []);
    assert_eq!(seen.moves.len(), 1);
    assert_eq!(
        (seen.moves[0].cause, seen.moves[0].skill, seen.moves[0].seed),
        (MoveCause::Forced, None, 7 << 16 | 1)
    );
    // A cast that cannot move its caster does not explain a relocation of the same snapshot.
    let (mut observer, cast) = first_cast(class, SkillId::ThunderKick);
    let seen = observer.observe(ROUND, true, [relocated(cast, AWAY)]);
    assert_eq!(seen.moves[0].cause, MoveCause::Forced);
    assert_eq!(seen.casts[0].origin, HOME);
    // Echo Strike moves on its recast only.
    let slot = slot_of(class, SkillId::EchoStrike);
    let open = LoadoutState {
        slots: std::array::from_fn(|index| SkillSlotState {
            can_recast: index == usize::from(slot),
            ..default()
        }),
        ..default()
    };
    for (loadout, cause) in [
        (None, MoveCause::Forced),
        (Some(&open), MoveCause::SkillCast),
    ] {
        let mut observer = CastObserver::default();
        let mut hero = hero(class);
        hero.loadout = loadout;
        observer.observe(ROUND, true, [acted(hero, 1, slot)]);
        let seen = observer.observe(ROUND, true, [relocated(acted(hero, 2, slot), AWAY)]);
        assert_eq!(seen.moves[0].cause, cause);
    }
    // The utility dash and a recall are reported as what they are.
    let (mut observer, cast) = first_cast(class, SkillId::AnchorStep);
    let mut dashed = relocated(cast, AWAY);
    dashed.utility.dash_remaining_secs = shared::utility::DASH_COOLDOWN_SECS;
    assert_eq!(
        observer.observe(ROUND, true, [dashed]).moves[0].cause,
        MoveCause::UtilityDash
    );
    let mut recalled = relocated(dashed, HOME);
    recalled.utility.recall_sequence = 1;
    assert_eq!(
        observer.observe(ROUND, true, [recalled]).moves[0].cause,
        MoveCause::Recall
    );
    // A hero that did not relocate is not reported, however far it walked.
    let mut walked = recalled;
    walked.position = AWAY;
    assert_eq!(observer.observe(ROUND, true, [walked]).moves, []);
}

#[test]
fn hidden_destination_draws_departure_only() {
    let registry = target();
    let class = HeroClass::Stormfist;
    let burst = |to: Vec3, visible: bool| {
        let (mut observer, cast) = first_cast(class, SkillId::AnchorStep);
        let mut gone = relocated(cast, to);
        gone.visible = visible;
        let seen = observer.observe(ROUND, true, [gone]);
        // A hero that is hidden now shows no accent of the cast that moved it.
        assert_eq!(seen.casts.is_empty(), !visible);
        assert_eq!(seen.moves.len(), 1);
        assert_eq!(seen.moves[0].to, visible.then_some(to));
        accents::move_burst(Some(&registry), &seen.moves[0]).unwrap()
    };
    let puff = burst(AWAY, false);
    assert_eq!(puff.len(), 4);
    assert_eq!(sources(&puff), [ParticleSource::Move]);
    for spec in &puff {
        // Everything stays where the hero was last seen.
        for point in ground_span(spec) {
            assert!(point.distance(HOME.xz()) < 1.0, "{point}");
        }
    }
    // The puff is the same wherever the hero went: it points nowhere.
    assert_eq!(puff, burst(Vec3::new(-9.0, 0.5, 11.0), false));
    // With the arrival in sight the whole pattern is drawn, to the arrival.
    let whole = burst(AWAY, true);
    assert_ne!(whole, puff);
    assert!(
        whole
            .iter()
            .any(|spec| ground_span(spec)[0].distance(AWAY.xz()) < 1.0)
    );

    // A hero that was hidden before the move gets the arrival half, which says nothing
    // about where it came from.
    let arrival = |from: Vec3| {
        let mut observer = CastObserver::default();
        let mut hidden = hero(class);
        hidden.position = from;
        hidden.visible = false;
        observer.observe(ROUND, true, [acted(hidden, 1, 0)]);
        let mut shown = relocated(acted(hidden, 2, slot_of(class, SkillId::AnchorStep)), AWAY);
        shown.visible = true;
        let seen = observer.observe(ROUND, true, [shown]);
        assert_eq!(seen.casts, []);
        assert_eq!(seen.moves[0].from, None);
        assert_eq!(seen.moves[0].cause, MoveCause::SkillCast);
        accents::move_burst(Some(&registry), &seen.moves[0]).unwrap()
    };
    assert_eq!(arrival(HOME), arrival(Vec3::new(-20.0, 0.5, 3.0)));
    for spec in &arrival(HOME) {
        assert!(ground_span(spec)[0].distance(AWAY.xz()) < 1.5);
    }
}

#[test]
fn moves_are_painted_by_their_cause() {
    let (target, unmigrated) = (target(), SkillPresentation::unmigrated());
    let moved = |cause: MoveCause, skill: Option<SkillId>| MoveObserved {
        actor_id: 7,
        from: Some(HOME),
        to: Some(AWAY),
        cause,
        skill: skill.map(SkillKey::Modular),
        seed: 41,
        local: false,
    };
    // The utility dash and a recall keep their own look.
    for cause in [MoveCause::Recall, MoveCause::UtilityDash] {
        assert_eq!(
            accents::move_burst(Some(&target), &moved(cause, None)),
            None
        );
    }
    // A displacement by something else needs no data: neutral skid marks.
    let forced = moved(MoveCause::Forced, None);
    let dragged = accents::move_burst(None, &forced).unwrap();
    assert_eq!(dragged, accents::drag_streak(Some(HOME), Some(AWAY), 41));
    assert_eq!(
        accents::move_burst(Some(&unmigrated), &forced),
        Some(dragged)
    );
    // A skill move is the pattern of the row, in its colours, seeded by the action.
    let leap = moved(MoveCause::SkillCast, Some(SkillId::AnchorStep));
    let row = target.profile(SkillId::AnchorStep).unwrap();
    let palette = Palette::of(row, target.theme(HeroClass::Stormfist).unwrap());
    let spec = row.cast.as_ref().unwrap().movement.as_ref().unwrap();
    let themed = accents::move_burst(Some(&target), &leap).unwrap();
    assert_eq!(
        themed,
        accents::move_particles(spec, &palette, Some(HOME), Some(AWAY), 41)
    );
    assert!(themed.len() <= accents::MOVE_MAX);
    assert!(themed.iter().all(|spec| spec.event_id == 41));
    // A row without the block keeps the built-in dash, and so does a missing registry.
    assert_eq!(accents::move_burst(Some(&unmigrated), &leap), None);
    assert_eq!(accents::move_burst(None, &leap), None);
    assert_eq!(
        accents::move_burst(
            Some(&target),
            &moved(MoveCause::SkillCast, Some(SkillId::ThunderKick))
        ),
        None
    );
}

#[test]
fn accents_point_along_the_accepted_yaw() {
    let registry = target();
    let class = HeroClass::Warrior;
    let (mut observer, cast) = first_cast_legacy(class, SkillSlot::Q);
    let yaw = shared::math::hero_yaw_towards(0.6, 0.8);
    let mut aimed = cast;
    aimed.facing.yaw = Some(yaw);
    // The model still looks the other way: the accepted yaw decides.
    aimed.forward = Vec3::new(-0.6, 0.0, -0.8);
    let seen = observer.observe(ROUND, true, [aimed]);
    assert_eq!(seen.casts[0].yaw, Some(yaw));
    let row = registry.row(class.ability(SkillSlot::Q).id).unwrap();
    let palette = Palette::of(row, registry.theme(class).unwrap());
    let along = |direction: Vec2| {
        accents::accent_particles(
            row.cast.as_ref().unwrap(),
            &palette,
            &CastContext {
                origin: HOME,
                direction,
                recast: false,
                area: None,
                strike_to: None,
                sequence: 2,
            },
        )
    };
    let accepted = CastContext::aim(Some(yaw), Vec3::X);
    let faced = CastContext::aim(None, aimed.forward);
    assert!(accepted.distance(Vec2::new(0.6, 0.8)) < 1e-5);
    assert!(faced.distance(Vec2::new(-0.6, -0.8)) < 1e-5);
    let burst = accents::cast_burst(&registry, &seen.casts[0], &[]);
    assert!(!burst.is_empty());
    assert_eq!(sources(&burst), [ParticleSource::Accent]);
    assert_eq!(burst, along(accepted));
    assert_ne!(burst, along(faced));
    // Every particle carries the action sequence (rule E-14).
    assert!(burst.iter().all(|spec| spec.event_id == 2));

    // A yaw of an older action is not the yaw of this one; the hero's facing is used.
    let mut stale = seen.casts[0].clone();
    stale.yaw = action_yaw(
        &aimed.action,
        &PlayerActionFacing {
            sequence: 1,
            yaw: Some(yaw),
        },
    );
    assert_eq!(stale.yaw, None);
    assert_eq!(accents::cast_burst(&registry, &stale, &[]), along(faced));
}

/// A legacy hero that was seen once and then casts the ability of `slot` as action 2.
fn first_cast_legacy(class: HeroClass, slot: SkillSlot) -> (CastObserver, Sighting<'static>) {
    let mut observer = CastObserver::default();
    let hero = hero(class);
    observer.observe(ROUND, true, [acted(hero, 1, 0)]);
    (observer, acted(hero, 2, slot.index() as u8))
}

#[test]
fn rows_without_the_block_and_basic_rows_resolve_as_data_says() {
    let (target, unmigrated) = (target(), SkillPresentation::unmigrated());
    let (mut observer, cast) = first_cast_legacy(HeroClass::Warrior, SkillSlot::Q);
    let seen = observer.observe(ROUND, true, [cast]);
    // No unmigrated row carries `cast` yet: the built-in accent stays.
    assert_eq!(accents::cast_burst(&unmigrated, &seen.casts[0], &[]), []);
    assert!(!unmigrated.themed_cast(HeroClass::Warrior, None, 0));
    assert!(target.themed_cast(HeroClass::Warrior, None, 0));
    assert!(!unmigrated.themed_cast(HeroClass::Warrior, None, BASIC_ATTACK_ACTION_SLOT));
    assert!(target.themed_cast(HeroClass::Warrior, None, BASIC_ATTACK_ACTION_SLOT));

    // A basic attack is drawn from the row of its class, in the class colours.
    let basic = observer.observe(ROUND, true, [acted(cast, 3, BASIC_ATTACK_ACTION_SLOT)]);
    assert_eq!(basic.casts[0].key, CastKey::Basic(HeroClass::Warrior));
    let burst = accents::cast_burst(&target, &basic.casts[0], &[]);
    let row = target.basic(HeroClass::Warrior).unwrap();
    assert_eq!(
        burst,
        accents::accent_particles(
            row.accent.as_ref().unwrap(),
            &Palette::of_class(target.theme(HeroClass::Warrior).unwrap()),
            &CastContext {
                origin: HOME,
                direction: Vec2::NEG_Y,
                recast: false,
                area: None,
                strike_to: None,
                sequence: 3,
            },
        )
    );
    assert_eq!(burst.len(), 2);
    assert_eq!(accents::cast_burst(&unmigrated, &basic.casts[0], &[]), []);
}

/// A repeater has a row for each of its two rounds. An accepted attack is the round of
/// the weapon mode the snapshot of its edge replicates; a receipt is the round its wire
/// style names. No other class has a second round, whatever its state says.
#[test]
fn rockets_variant_follows_the_mode_at_the_edge_and_the_style_of_the_receipt() {
    use shared::combat::ProjectileStyle;
    let (target, unmigrated) = (target(), SkillPresentation::unmigrated());
    let wildspark = HeroClass::Wildspark;
    let (bullets, rockets) = (CastKey::Basic(wildspark), CastKey::Rockets(wildspark));
    let armed = |core: CoreId, weapon_mode| LoadoutState {
        recipe: Some(core.preset()),
        weapon_mode,
        ..default()
    };
    let launcher = armed(CoreId::Wildspark, WeaponMode::Rockets);
    let repeater = armed(CoreId::Wildspark, WeaponMode::Repeater);
    fn basic(class: HeroClass, loadout: Option<&LoadoutState>) -> Option<CastKey> {
        CastKey::of(class, loadout, BASIC_ATTACK_ACTION_SLOT)
    }
    assert_eq!(basic(wildspark, Some(&launcher)), Some(rockets));
    assert_eq!(basic(wildspark, Some(&repeater)), Some(bullets));
    assert_eq!(basic(wildspark, None), Some(bullets));
    // The mode says nothing about a skill, and nothing for a kit that is no repeater.
    for slot in 0..4 {
        assert_eq!(
            CastKey::of(wildspark, Some(&launcher), slot),
            CastKey::of(wildspark, Some(&repeater), slot)
        );
    }
    for class in HeroClass::ALL {
        let Some(kit) = shared::loadout::preset_for_class(class) else {
            assert_eq!(basic(class, None), Some(CastKey::Basic(class)));
            continue;
        };
        let state = armed(kit.core(), WeaponMode::Rockets);
        let fires_rockets = kit.attack_profile() == AttackProfileId::Repeater;
        assert_eq!(fires_rockets, class == wildspark, "{}", class.id());
        assert_eq!(
            basic(class, Some(&state)),
            Some(if fires_rockets {
                CastKey::Rockets(class)
            } else {
                CastKey::Basic(class)
            }),
            "{}",
            class.id()
        );
    }

    // The two rounds are two rows: the nested entry for the rocket, the row for the bullet.
    let row = target.basic(wildspark).unwrap();
    let nested = row.rockets.as_deref().unwrap();
    assert_eq!(target.basic_round(bullets), Some(row));
    assert_eq!(target.basic_round(rockets), Some(nested));
    assert_ne!(row.accent, nested.accent);
    assert_ne!(row.impact, nested.impact);
    assert_ne!(row.sound, nested.sound);
    let look = |key| {
        let look = target.look(key).unwrap();
        (look.accent.cloned(), look.impact.cloned())
    };
    assert_eq!(look(bullets), (row.accent.clone(), row.impact.clone()));
    assert_eq!(
        look(rockets),
        (nested.accent.clone(), nested.impact.clone())
    );
    // A row without the entry serves both rounds, and a registry without the row neither.
    let riftshot = HeroClass::Riftshot;
    assert_eq!(
        target.basic_round(CastKey::Rockets(riftshot)),
        target.basic(riftshot)
    );
    assert!(target.basic(riftshot).unwrap().rockets.is_none());
    assert_eq!(unmigrated.basic_round(rockets), None);
    assert!(unmigrated.look(rockets).is_none());
    assert_eq!(
        target.basic_round(CastKey::Skill(SkillKey::Modular(SkillId::WildRocket))),
        None
    );

    // The edge: the accent of an accepted attack is that of the mode its snapshot carries.
    let shot = |before: &LoadoutState, now: &LoadoutState| {
        let mut observer = CastObserver::default();
        let mut hero = hero(wildspark);
        hero.loadout = Some(before);
        observer.observe(ROUND, true, [acted(hero, 1, 0)]);
        hero.loadout = Some(now);
        let seen = observer.observe(ROUND, true, [acted(hero, 2, BASIC_ATTACK_ACTION_SLOT)]);
        assert_eq!(seen.casts.len(), 1);
        seen.casts[0].clone()
    };
    let accent = |row: &crate::skill_presentation::BasicProfile, sequence| {
        accents::accent_particles(
            row.accent.as_ref().unwrap(),
            &Palette::of_class(target.theme(wildspark).unwrap()),
            &CastContext {
                origin: HOME,
                direction: Vec2::NEG_Y,
                recast: false,
                area: None,
                strike_to: None,
                sequence,
            },
        )
    };
    for (before, now, key, fired) in [
        (&repeater, &repeater, bullets, row),
        (&launcher, &launcher, rockets, nested),
        // The mode of the snapshot of the edge decides, not the one before it.
        (&repeater, &launcher, rockets, nested),
        (&launcher, &repeater, bullets, row),
    ] {
        let cast = shot(before, now);
        assert_eq!(cast.key, key);
        let burst = accents::cast_burst(&target, &cast, &[]);
        assert_eq!(burst, accent(fired, 2));
        assert!(!burst.is_empty());
        assert_eq!(accents::cast_burst(&unmigrated, &cast, &[]), []);
    }
    assert_ne!(accent(row, 2), accent(nested, 2));

    // The receipt: the wire style of the hit names the round, in whichever mode the hero
    // is when it lands. Only `Rocket` is the rocket.
    for key in [bullets, rockets] {
        assert_eq!(key.struck_as(ProjectileStyle::Rocket), rockets);
        for style in [
            ProjectileStyle::Bullet,
            ProjectileStyle::Standard,
            ProjectileStyle::Arcane,
            ProjectileStyle::TowerBolt,
        ] {
            assert_eq!(key.struck_as(style), bullets, "{style:?}");
        }
    }
    let zap = CastKey::Skill(SkillKey::Modular(SkillId::WildZap));
    assert_eq!(zap.struck_as(ProjectileStyle::Rocket), zap);
}

#[test]
fn area_only_on_a_signed_first_cast() {
    let registry = target();
    let class = HeroClass::Stormfist;
    let area = |id: SkillId, origin: Vec3, arrival: Vec3| {
        geometry::instant_area(
            id,
            &AreaContext {
                origin: origin.xz(),
                arrival: Some(arrival.xz()),
                direction: Vec2::NEG_Y,
                recast: false,
            },
        )
    };
    /// The ring a burst holds on the ground: its centre and its radius.
    fn held_ring(burst: &[ParticleSpec]) -> Option<(Vec2, f32)> {
        burst
            .iter()
            .find(|spec| spec.shape == ParticleShape::Ringlet && spec.velocity == Vec3::ZERO)
            .map(|spec| {
                (
                    spec.origin.xz(),
                    spec.size * unit_radius(ParticleShape::Ringlet) * spec.curve.peak(),
                )
            })
    }
    let slot = slot_of(class, SkillId::ThunderPulse);
    let open = LoadoutState {
        slots: std::array::from_fn(|index| SkillSlotState {
            can_recast: index == usize::from(slot),
            ..default()
        }),
        ..default()
    };
    let mut observer = CastObserver::default();
    let mut pulse = hero(class);
    observer.observe(ROUND, true, [acted(pulse, 1, slot)]);
    let first = observer.observe(ROUND, true, [acted(pulse, 2, slot)]);
    let burst = accents::cast_burst(&registry, &first.casts[0], &[]);
    let Some(GeoShape::Ring { center, radius }) = area(SkillId::ThunderPulse, HOME, HOME) else {
        panic!("thunder_pulse has a ring area");
    };
    let (ring_centre, ring_radius) = held_ring(&burst).unwrap();
    assert!(ring_centre.distance(center) < 1e-5);
    assert!(
        (ring_radius - radius).abs() < 1e-4,
        "{ring_radius} vs {radius}"
    );
    assert_eq!(center, HOME.xz());

    // The recast of the same row flashes no area: it draws `cast.recast` (none here).
    pulse.loadout = Some(&open);
    observer.observe(ROUND, true, [acted(pulse, 2, slot)]);
    let again = observer.observe(ROUND, true, [acted(pulse, 3, slot)]);
    assert!(again.casts[0].recast);
    assert_eq!(accents::cast_burst(&registry, &again.casts[0], &[]), []);
    // Even a recast that repeats the first accent holds no area.
    let mut repeated = first.casts[0].clone();
    repeated.recast = true;
    let mut plain = registry.clone();
    plain
        .skills
        .get_mut(SkillId::ThunderPulse.id())
        .unwrap()
        .cast
        .as_mut()
        .unwrap()
        .recast = None;
    let echo = accents::cast_burst(&plain, &repeated, &[]);
    assert!(!echo.is_empty());
    assert!(held_ring(&echo).is_none_or(|(_, held)| (held - radius).abs() > 0.5));

    // Anvil Charge resolves at its landing: the ring is held there, not where it started.
    let cinder = HeroClass::Cinderforge;
    let (mut observer, cast) = first_cast(cinder, SkillId::AnvilCharge);
    let seen = observer.observe(ROUND, true, [relocated(cast, AWAY)]);
    let burst = accents::cast_burst(&registry, &seen.casts[0], &[]);
    let Some(GeoShape::Ring { center, radius }) = area(SkillId::AnvilCharge, HOME, AWAY) else {
        panic!("anvil_charge has a ring area");
    };
    assert_eq!(center, AWAY.xz());
    let (ring_centre, ring_radius) = held_ring(&burst).unwrap();
    assert!(ring_centre.distance(AWAY.xz()) < 1e-5);
    assert!((ring_radius - radius).abs() < 1e-4);

    // A row that asks for an area without the sign-off draws its plain accent. The parser
    // refuses such a row; the emitter does not rely on it.
    assert!(!category::AREA_FLASH_SIGNED_OFF.contains(&SkillId::Nightfall));
    let veil = HeroClass::Veilstalker;
    let mut unsigned = registry.clone();
    let row = unsigned
        .skills
        .get_mut(SkillId::Nightfall.id())
        .unwrap()
        .cast
        .as_mut()
        .unwrap();
    assert!(!row.area);
    row.area = true;
    let (mut observer, cast) = first_cast(veil, SkillId::Nightfall);
    let seen = observer.observe(ROUND, true, [cast]);
    assert_eq!(
        accents::cast_burst(&unsigned, &seen.casts[0], &[]),
        accents::cast_burst(&registry, &seen.casts[0], &[])
    );
    // Every row that flashes an area is on the sign-off list.
    for (id, row) in registry.rows() {
        if row.cast.as_ref().is_some_and(|cast| cast.area) {
            let skill = SkillId::from_id(id).unwrap();
            assert!(category::AREA_FLASH_SIGNED_OFF.contains(&skill), "{id}");
        }
    }
}

#[test]
fn strike_line_needs_the_own_effect() {
    let registry = target();
    let class = HeroClass::Cinderforge;
    let (mut observer, cast) = first_cast(class, SkillId::FaultLine);
    let seen = observer.observe(ROUND, true, [cast]);
    let cast = &seen.casts[0];
    let pillar = HOME + Vec3::new(0.0, 0.0, -7.0);
    // No effect in the snapshot, the effect of another hero, or of another skill: no line.
    assert_eq!(accents::cast_burst(&registry, cast, &[]), []);
    for foreign in [
        effect(31, 8, SkillId::FaultLine, pillar),
        effect(31, 0, SkillId::FaultLine, pillar),
        effect(31, 7, SkillId::WildTraps, pillar),
    ] {
        assert_eq!(accents::cast_burst(&registry, cast, &[foreign]), []);
    }
    // The caster's own new effect ends the line.
    let own = effect(31, 7, SkillId::FaultLine, pillar);
    let line = accents::cast_burst(&registry, cast, std::slice::from_ref(&own));
    assert_eq!(line.len(), 8);
    assert_eq!(sources(&line), [ParticleSource::Accent]);
    let reach = line
        .iter()
        .map(|spec| spec.origin.xz().distance(HOME.xz()))
        .fold(0.0, f32::max);
    assert!(reach > 5.0 && reach <= 7.0 + 1e-3, "{reach}");
    for spec in &line {
        // Laid on the segment from the caster to the effect.
        assert!((spec.origin.x - HOME.x).abs() < 1e-3);
        assert!(spec.origin.z <= HOME.z + 1e-3 && spec.origin.z >= pillar.z - 1e-3);
    }
    // Of two new effects the newest one is the one this cast made.
    let older = effect(30, 7, SkillId::FaultLine, HOME + Vec3::X * 3.0);
    assert_eq!(accents::cast_burst(&registry, cast, &[older, own]), line);
}

#[test]
fn no_accent_without_an_observed_arrival() {
    let registry = target();
    let class = HeroClass::Riftshot;
    // Rift Step resolves where it lands. Seen at the landing, the accent is drawn there.
    let (mut observer, cast) = first_cast(class, SkillId::RiftStep);
    let seen = observer.observe(ROUND, true, [relocated(cast, AWAY)]);
    let burst = accents::cast_burst(&registry, &seen.casts[0], &[]);
    assert!(!burst.is_empty());
    for spec in &burst {
        for point in ground_span(spec) {
            assert!(point.distance(AWAY.xz()) <= accents::DECORATIVE_REACH + 1e-3);
            assert!(
                point.distance(HOME.xz()) > 3.0,
                "nothing at the departure point"
            );
        }
    }
    // Blinking out of sight: no accent anywhere, and only the puff where it stood.
    let (mut observer, cast) = first_cast(class, SkillId::RiftStep);
    let mut gone = relocated(cast, AWAY);
    gone.visible = false;
    let seen = observer.observe(ROUND, true, [gone]);
    assert_eq!(seen.casts, []);
    let drawn: Vec<ParticleSpec> = seen
        .moves
        .iter()
        .filter_map(|moved| accents::move_burst(Some(&registry), moved))
        .flatten()
        .collect();
    assert_eq!(sources(&drawn), [ParticleSource::Move]);
    assert_eq!(drawn.len(), 4);
    // A skill that resolves where it was cast keeps its accent there although it moved.
    let veil = HeroClass::Veilstalker;
    let (mut observer, cast) = first_cast(veil, SkillId::Nightfall);
    let seen = observer.observe(ROUND, true, [relocated(cast, AWAY)]);
    for spec in &accents::cast_burst(&registry, &seen.casts[0], &[]) {
        assert!(ground_span(spec)[0].distance(HOME.xz()) <= accents::DECORATIVE_REACH + 1e-3);
    }
}

#[test]
fn link_needs_a_matching_receipt() {
    let registry = target();
    let class = HeroClass::Stormfist;
    let kick = slot_of(class, SkillId::ThunderKick);
    let cast_of = |class: HeroClass, skill: SkillId| {
        let (mut observer, cast) = first_cast(class, skill);
        observer.observe(ROUND, true, [cast]).casts.remove(0)
    };
    let hit = |receipt: u64, source: u64, slot: u8| ConfirmedHit {
        receipt,
        source,
        slot,
        position: AWAY,
    };
    let opened = |tick: u64| {
        let mut book = LinkBook::default();
        book.turn(ROUND, tick);
        book.open(&registry, &cast_of(class, SkillId::ThunderKick));
        book
    };
    // No receipt, no link; a receipt of another hero or another slot is not this cast's.
    let mut book = opened(10);
    assert_eq!(book.link(&hit(1, 8, kick)), None);
    assert_eq!(book.link(&hit(1, 7, (kick + 1) % 4)), None);
    assert_eq!(book.link(&hit(1, 7, BASIC_ATTACK_ACTION_SLOT)), None);
    // The matching receipt draws three streaks from the caster that end at it.
    let (link, local) = book.link(&hit(55, 7, kick)).unwrap();
    assert!(!local);
    assert_eq!(link.len(), accents::LINK_MAX);
    assert_eq!(sources(&link), [ParticleSource::Link]);
    for spec in &link {
        assert_eq!(spec.event_id, 55);
        let [start, end] = ground_span(spec);
        assert!(start.distance(HOME.xz()) < 0.2);
        assert!(end.distance(AWAY.xz()) < 1e-3);
        assert!(spec.end_secs() <= accents::LINK_SECS + 1e-5);
    }
    // At most three receipts of one cast are linked.
    assert!(book.link(&hit(56, 7, kick)).is_some());
    assert!(book.link(&hit(57, 7, kick)).is_some());
    assert_eq!(book.link(&hit(58, 7, kick)), None);

    // The receipt may come with the snapshot of the cast or one of the two after it.
    for (later, linked) in [(0, true), (1, true), (2, true), (3, false)] {
        let mut book = opened(10);
        for tick in 1..=later {
            // Frames without a new snapshot do not count.
            book.turn(ROUND, 10 + tick);
            book.turn(ROUND, 10 + tick);
        }
        assert_eq!(book.link(&hit(1, 7, kick)).is_some(), linked, "{later}");
    }
    // A new round closes every link, and a newer cast of the slot replaces the older one.
    let mut book = opened(10);
    book.turn(Some((3, 2)), 11);
    assert_eq!(book.link(&hit(1, 7, kick)), None);
    let mut book = opened(10);
    assert!(book.link(&hit(1, 7, kick)).is_some());
    book.open(&registry, &cast_of(class, SkillId::ThunderKick));
    for receipt in 2..5 {
        assert!(book.link(&hit(receipt, 7, kick)).is_some());
    }
    assert_eq!(book.link(&hit(5, 7, kick)), None);

    // Rows that open no link from a cast: none named, a travelling body (its hit comes
    // later), a skill that strikes from its effect, a basic attack, an unmigrated row.
    let none_opened = |registry: &SkillPresentation, cast: SkillCastObserved| {
        let mut book = LinkBook::default();
        book.turn(ROUND, 10);
        book.open(registry, &cast);
        (0..4).all(|slot| book.link(&hit(1, 7, slot)).is_none())
    };
    assert!(none_opened(&registry, cast_of(class, SkillId::AnchorStep)));
    let volley = registry.profile(SkillId::ThornVolley).unwrap();
    assert!(volley.cast.as_ref().unwrap().link.is_some());
    assert!(none_opened(
        &registry,
        cast_of(HeroClass::Veilstalker, SkillId::ThornVolley)
    ));
    let guard = registry.profile(SkillId::MirrorGuard).unwrap();
    assert!(guard.cast.as_ref().unwrap().link.is_some());
    assert!(none_opened(
        &registry,
        cast_of(HeroClass::Edgeweaver, SkillId::MirrorGuard)
    ));
    let mut basic = cast_of(class, SkillId::ThunderKick);
    basic.key = CastKey::Basic(class);
    assert!(none_opened(&registry, basic));
    assert!(none_opened(
        &SkillPresentation::unmigrated(),
        cast_of(class, SkillId::ThunderKick)
    ));

    // A skill that moves first is linked from its landing, never from where it left.
    let charge = registry.profile(SkillId::AnvilCharge).unwrap();
    assert_eq!(charge.cast.as_ref().unwrap().link, None);
    let mut landed = registry.clone();
    landed
        .skills
        .get_mut(SkillId::AnvilCharge.id())
        .unwrap()
        .cast
        .as_mut()
        .unwrap()
        .link = Some(ParticleShape::Streak);
    let cinder = HeroClass::Cinderforge;
    let (mut observer, cast) = first_cast(cinder, SkillId::AnvilCharge);
    let charge = observer
        .observe(ROUND, true, [relocated(cast, AWAY)])
        .casts
        .remove(0);
    let mut book = LinkBook::default();
    book.turn(ROUND, 10);
    book.open(&landed, &charge);
    let far = ConfirmedHit {
        receipt: 9,
        source: 7,
        slot: slot_of(cinder, SkillId::AnvilCharge),
        position: AWAY + Vec3::X * 2.0,
    };
    let (link, _) = book.link(&far).unwrap();
    for spec in &link {
        assert!(ground_span(spec)[0].distance(AWAY.xz()) < 0.2);
    }
}

/// Thorn Volley throws a spike and then recasts without one: only a recast is linked to its
/// receipts, and never while the spike may still be what struck.
#[test]
fn recast_link_only_after_a_recast_edge() {
    let registry = target();
    let class = HeroClass::Veilstalker;
    let slot = slot_of(class, SkillId::ThornVolley);
    let first = {
        let (mut observer, cast) = first_cast(class, SkillId::ThornVolley);
        observer.observe(ROUND, true, [cast]).casts.remove(0)
    };
    assert!(!first.recast);
    let recast = SkillCastObserved {
        sequence: first.sequence + 1,
        recast: true,
        ..first.clone()
    };
    let hit = |receipt: u64| ConfirmedHit {
        receipt,
        source: 7,
        slot,
        position: AWAY,
    };
    let spike = |owner: u64, skill: SkillId| SkillEffectState {
        kind: EffectVisualKind::Bolt,
        ..effect(40, owner, skill, HOME + Vec3::X)
    };
    let opened = || {
        let mut book = LinkBook::default();
        book.turn(ROUND, 10);
        book
    };
    let row = registry.profile(SkillId::ThornVolley).unwrap();
    let shape = row.cast.as_ref().unwrap().link.unwrap();

    // The first cast throws the spike: its hit, even one at point-blank range that never
    // shows a body, is not a lash.
    let mut thrown = opened();
    thrown.open(&registry, &first);
    assert_eq!(thrown.link(&hit(1)), None);

    // A recast with no spike in flight: the receipt of its own source and slot is linked
    // from the caster, in the shape the row names.
    let mut lashed = opened();
    lashed.open(&registry, &recast);
    assert_eq!(
        lashed.link(&ConfirmedHit {
            source: 8,
            ..hit(1)
        }),
        None
    );
    assert_eq!(
        lashed.link(&ConfirmedHit {
            slot: (slot + 1) % 4,
            ..hit(1)
        }),
        None
    );
    let (link, _) = lashed.link(&hit(2)).unwrap();
    assert_eq!(link.len(), accents::LINK_MAX);
    assert_eq!(sources(&link), [ParticleSource::Link]);
    for spec in &link {
        assert_eq!(spec.shape, shape);
        let [start, end] = ground_span(spec);
        assert!(start.distance(HOME.xz()) < 0.2);
        assert!(end.distance(AWAY.xz()) < 1e-3);
    }
    // The bonus of the mark is a second receipt of the same recast.
    assert!(lashed.link(&hit(3)).is_some());

    // The spike in this snapshot or in the one before: a receipt may be its own, so the
    // recast opens nothing. Two snapshots after the spike was last seen it does.
    for (since, linked) in [(0, false), (1, false), (2, true)] {
        let mut book = opened();
        book.sight(&[spike(7, SkillId::ThornVolley)]);
        for tick in 1..=since {
            book.turn(ROUND, 10 + tick);
            book.sight(&[]);
        }
        book.open(&registry, &recast);
        assert_eq!(book.link(&hit(1)).is_some(), linked, "{since}");
    }
    // A recast that was refused for a spike stays refused when its receipt comes late.
    let mut late = opened();
    late.sight(&[spike(7, SkillId::ThornVolley)]);
    late.turn(ROUND, 11);
    late.sight(&[]);
    late.open(&registry, &recast);
    late.turn(ROUND, 12);
    late.sight(&[]);
    assert_eq!(late.link(&hit(1)), None);
    // The spike of another hero, or a body of another skill, is not this caster's.
    let mut book = opened();
    book.sight(&[
        spike(8, SkillId::ThornVolley),
        spike(7, SkillId::DawnBind),
        effect(41, 7, SkillId::ThornVolley, HOME),
    ]);
    book.open(&registry, &recast);
    assert!(book.link(&hit(1)).is_some());

    // A link that is open is held back as well while a spike is seen, and for the snapshot
    // after it; by then the link has run out.
    let mut held = opened();
    held.open(&registry, &recast);
    held.turn(ROUND, 11);
    held.sight(&[spike(7, SkillId::ThornVolley)]);
    assert_eq!(held.link(&hit(1)), None);
    held.turn(ROUND, 12);
    held.sight(&[]);
    assert_eq!(held.link(&hit(1)), None);
    held.turn(ROUND, 13);
    assert_eq!(held.link(&hit(1)), None);

    // A new first cast ends what a recast left open; a new round forgets the spike.
    let mut again = opened();
    again.open(&registry, &recast);
    again.open(&registry, &first);
    assert_eq!(again.link(&hit(1)), None);
    let mut next = opened();
    next.sight(&[spike(7, SkillId::ThornVolley)]);
    next.turn(Some((3, 2)), 11);
    next.open(&registry, &recast);
    assert!(next.link(&hit(1)).is_some());

    // No other thrown skill is linked on a recast edge, whatever its row would say.
    let mut hooked = registry.clone();
    hooked
        .skills
        .get_mut(SkillId::EchoStrike.id())
        .unwrap()
        .cast
        .as_mut()
        .unwrap()
        .link = Some(ParticleShape::Streak);
    let storm = HeroClass::Stormfist;
    let (mut observer, cast) = first_cast(storm, SkillId::EchoStrike);
    let mut echo = observer.observe(ROUND, true, [cast]).casts.remove(0);
    echo.recast = true;
    let mut book = opened();
    book.open(&hooked, &echo);
    assert!((0..4).all(|slot| { book.link(&ConfirmedHit { slot, ..hit(1) }).is_none() }));
}

/// An app with the observer and the three emitters, a running round and one remote hero.
fn stage(registry: SkillPresentation, mode: PlayerVisualMode) -> (App, Entity) {
    stage_as(registry, mode, HeroClass::Stormfist)
}

fn stage_as(
    registry: SkillPresentation,
    mode: PlayerVisualMode,
    class: HeroClass,
) -> (App, Entity) {
    let mut app = App::new();
    app.insert_resource(registry)
        .insert_resource(mode)
        .insert_resource(GameStateSnapshot {
            meta: shared::protocol::SnapshotMeta::new(3, 1, 1),
            state: GameState::Running,
            ..default()
        })
        .init_resource::<ThemedDashes>()
        .add_message::<SessionEvent>()
        .add_message::<SkillCastObserved>()
        .add_message::<MoveObserved>()
        .add_message::<ConfirmedHit>()
        .add_message::<super::super::stage::StageEvent>()
        .add_message::<SkillBurst>()
        .add_systems(Update, observe_skill_casts)
        .add_systems(
            PostUpdate,
            (accents::emit_cast, accents::emit_moves, accents::emit_links).chain(),
        );
    let hero = app
        .world_mut()
        .spawn((
            NetworkPlayerId(7),
            Transform::from_translation(HOME),
            InheritedVisibility::VISIBLE,
            CombatStats::default(),
            PlayerCosmeticAction::default(),
            PlayerActionFacing::default(),
            NetworkHeroClass(class),
            PlayerUtility::default(),
        ))
        .id();
    assert!(app.world().get::<CombatStats>(hero).unwrap().is_alive());
    app.update();
    (app, hero)
}

/// Applies an accepted action to the staged hero, runs a frame and returns its bursts.
fn act(
    app: &mut App,
    hero: Entity,
    sequence: u64,
    slot: u8,
    yaw: Option<f32>,
) -> Vec<Vec<ParticleSpec>> {
    app.world_mut().entity_mut(hero).insert((
        PlayerCosmeticAction {
            sequence,
            kind: PlayerActionKind::Cast,
            slot,
        },
        PlayerActionFacing { sequence, yaw },
    ));
    frame(app)
}

fn frame(app: &mut App) -> Vec<Vec<ParticleSpec>> {
    app.update();
    app.world_mut()
        .resource_mut::<Messages<SkillBurst>>()
        .drain()
        .map(|burst| burst.0)
        .collect()
}

#[test]
fn the_flat_origin_is_the_simulation_translation() {
    let slot = slot_of(HeroClass::Stormfist, SkillId::ThunderPulse);
    let bursts = [PlayerVisualMode::Sprite2d, PlayerVisualMode::Models3d].map(|mode| {
        let (mut app, hero) = stage(target(), mode);
        let mut bursts = act(&mut app, hero, 1, slot, None);
        assert_eq!(bursts.len(), 1);
        bursts.remove(0)
    });
    // The same particles in both render modes: simulation coordinates, anchored at the
    // hero's translation.
    assert_eq!(bursts[0], bursts[1]);
    let ring = bursts[0]
        .iter()
        .find(|spec| spec.shape == ParticleShape::Ringlet && spec.velocity == Vec3::ZERO)
        .unwrap();
    assert_eq!(ring.origin.xz(), HOME.xz());
    // The flat view draws the ring where the sprite of the hero stands.
    assert_eq!(
        ring.pose_at(0.0, true, Quat::IDENTITY)
            .translation
            .truncate(),
        crate::world2d::simulation_xz_to_render_xy(HOME)
    );
}

#[test]
fn the_observer_feeds_the_emitters_once_per_action() {
    let class = HeroClass::Stormfist;
    let (mut app, hero) = stage(target(), PlayerVisualMode::Models3d);
    let yaw = shared::math::hero_yaw_towards(1.0, 0.0);
    let kick = slot_of(class, SkillId::ThunderKick);
    let bursts = act(&mut app, hero, 1, kick, Some(yaw));
    assert_eq!(bursts.len(), 1);
    assert_eq!(sources(&bursts[0]), [ParticleSource::Accent]);
    assert!(bursts[0].iter().all(|spec| spec.event_id == 1));
    // A remote hero is admitted after the local one.
    assert!(bursts[0].iter().all(|spec| spec.sort_key == 1));
    // The fan opens along the accepted yaw.
    let ahead = bursts[0]
        .iter()
        .map(|spec| ground_span(spec)[1].x - HOME.x)
        .fold(f32::MIN, f32::max);
    assert!(ahead > 0.8, "{ahead}");
    // Nothing more without a new action.
    assert_eq!(frame(&mut app), Vec::<Vec<ParticleSpec>>::new());

    // The receipt of that cast draws its link, once.
    app.world_mut().write_message(ConfirmedHit {
        receipt: 90,
        source: 7,
        slot: kick,
        position: AWAY,
    });
    let bursts = frame(&mut app);
    assert_eq!(bursts.len(), 1);
    assert_eq!(sources(&bursts[0]), [ParticleSource::Link]);
    assert_eq!(frame(&mut app), Vec::<Vec<ParticleSpec>>::new());

    // The leap of the hero's own cast is themed; the accent sits at the landing.
    let leap = slot_of(class, SkillId::AnchorStep);
    app.world_mut()
        .entity_mut(hero)
        .insert(Transform::from_translation(AWAY));
    app.world_mut()
        .get_mut::<PlayerUtility>(hero)
        .unwrap()
        .state
        .dash_sequence = 1;
    let bursts = act(&mut app, hero, 2, leap, None);
    assert_eq!(
        bursts
            .iter()
            .map(|burst| sources(burst))
            .collect::<Vec<_>>(),
        [[ParticleSource::Accent], [ParticleSource::Move]]
    );
    assert!(app.world().resource::<ThemedDashes>().0.contains(&7));
    for spec in &bursts[0] {
        assert!(ground_span(spec)[0].distance(AWAY.xz()) <= accents::DECORATIVE_REACH + 1e-3);
    }
    // The set is refilled every frame.
    frame(&mut app);
    assert!(app.world().resource::<ThemedDashes>().0.is_empty());

    // Kicked back: no action of its own, so neutral skid marks, and no generic dash.
    app.world_mut()
        .entity_mut(hero)
        .insert(Transform::from_translation(HOME));
    app.world_mut()
        .get_mut::<PlayerUtility>(hero)
        .unwrap()
        .state
        .dash_sequence = 2;
    let bursts = frame(&mut app);
    assert_eq!(bursts.len(), 1);
    assert_eq!(
        bursts[0],
        accents::drag_streak(Some(AWAY), Some(HOME), 7 << 16 | 2)
            .into_iter()
            .map(|mut spec| {
                spec.sort_key = 1;
                spec
            })
            .collect::<Vec<_>>()
    );
    assert!(app.world().resource::<ThemedDashes>().0.contains(&7));

    // The utility dash keeps its own look.
    let mut utility = app.world_mut().get_mut::<PlayerUtility>(hero).unwrap();
    utility.state.dash_sequence = 3;
    utility.state.dash_remaining_secs = shared::utility::DASH_COOLDOWN_SECS;
    assert_eq!(frame(&mut app), Vec::<Vec<ParticleSpec>>::new());
    assert!(app.world().resource::<ThemedDashes>().0.is_empty());

    // A new connection: the next snapshot is a baseline, whatever happened meanwhile.
    app.world_mut().write_message(SessionEvent::Connected);
    assert_eq!(
        act(&mut app, hero, 9, kick, None),
        Vec::<Vec<ParticleSpec>>::new()
    );
    assert_eq!(act(&mut app, hero, 10, kick, None).len(), 1);
}

#[test]
fn the_local_hero_is_admitted_first() {
    let (mut app, hero) = stage(target(), PlayerVisualMode::Models3d);
    let slot = slot_of(HeroClass::Stormfist, SkillId::ThunderKick);
    app.world_mut().spawn((
        Player,
        NetworkPlayerId(1),
        Transform::from_translation(HOME + Vec3::X * 10.4),
        InheritedVisibility::VISIBLE,
        CombatStats::default(),
        PlayerCosmeticAction::default(),
        PlayerActionFacing::default(),
        NetworkHeroClass(HeroClass::Stormfist),
        PlayerUtility::default(),
    ));
    app.update();
    let local = app
        .world_mut()
        .query_filtered::<Entity, With<Player>>()
        .single(app.world())
        .unwrap();
    assert!(
        act(&mut app, local, 1, slot, None)[0]
            .iter()
            .all(|spec| spec.sort_key == 0)
    );
    // Others follow by their distance to the local hero.
    assert!(
        act(&mut app, hero, 1, slot, None)[0]
            .iter()
            .all(|spec| spec.sort_key == 11)
    );
}

#[test]
fn a_strike_line_ends_only_at_an_effect_that_appeared_with_the_cast() {
    let class = HeroClass::Cinderforge;
    let slot = slot_of(class, SkillId::FaultLine);
    let pillar = HOME + Vec3::new(0.0, 0.0, -7.0);
    let (mut app, hero) = stage_as(target(), PlayerVisualMode::Models3d, class);
    // The pillar of an earlier cast is still in the snapshot when the hero casts again and
    // the new one is not (fog took it): no line is drawn to the old pillar.
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .skill_effects = vec![effect(30, 7, SkillId::FaultLine, pillar)];
    assert_eq!(frame(&mut app), Vec::<Vec<ParticleSpec>>::new());
    assert_eq!(
        act(&mut app, hero, 1, slot, None),
        Vec::<Vec<ParticleSpec>>::new()
    );
    // The new pillar arrives with the edge: the line ends at it.
    let fresh = HOME + Vec3::new(6.0, 0.0, 0.0);
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .skill_effects
        .push(effect(31, 7, SkillId::FaultLine, fresh));
    let bursts = act(&mut app, hero, 2, slot, None);
    assert_eq!(bursts.len(), 1);
    for spec in &bursts[0] {
        assert!((spec.origin.z - HOME.z).abs() < 1e-3);
        assert!(spec.origin.x >= HOME.x && spec.origin.x <= fresh.x);
    }
}
