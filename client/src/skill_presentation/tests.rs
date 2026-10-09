use super::*;
use shared::loadout::WeaponMode;

pub(super) mod schema_rules;
pub(super) mod target;

fn profiles() -> SkillPresentation {
    SkillPresentation::parse(include_str!("../../assets/config/skills.skillfx")).unwrap()
}

/// A file of the first schema: rows of a release, a legacy style and a colour.
const SCHEMA_1: &str = r#"{
  "schema_version": 1,
  "skills": {
    "dawn_bind": {"release":"cast","effect":"lance","color":[1.0,0.65,0.1],"hdr_gain":3.5}
  }
}"#;

#[test]
fn dagger_skills_keep_distinct_motions_when_mixed_into_another_core() {
    let mut recipe = shared::loadout::CoreId::Dawnweaver.preset();
    recipe.skills = [
        SkillId::DaggerLethalBlow,
        SkillId::DaggerBackstab,
        SkillId::DaggerBluff,
        SkillId::DaggerDeadlyBlow,
    ];
    let state = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    for (slot, expected) in [
        "dagger_heavy_thrust",
        "dagger_backstab",
        "dagger_feint",
        "dagger_stab",
    ]
    .into_iter()
    .enumerate()
    {
        let cue = motion_cue(
            &profiles(),
            state.recipe.as_ref().unwrap().core.class(),
            Some(&state),
            slot as u8,
            7,
            &[],
        )
        .unwrap();
        assert_eq!(cue.motion, expected);
        assert!(!cue.hold);
    }
}

#[test]
fn recipe_slot_selects_motion_even_when_skill_is_moved_to_another_button() {
    let mut recipe = shared::loadout::CoreId::Wildspark.preset();
    recipe.skills[0] = SkillId::DawnRay;
    let state = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    assert_eq!(
        equipped_skill(state.recipe.as_ref().unwrap().core.class(), Some(&state), 0),
        Some(SkillId::DawnRay)
    );
    let registry = profiles();
    let profile = registry.profile(SkillId::DawnRay).unwrap();
    let mut e = effect(SkillId::DawnRay, EffectVisualKind::BeamWarning);
    let cue = motion_cue(
        &profiles(),
        state.recipe.as_ref().unwrap().core.class(),
        Some(&state),
        0,
        7,
        &[e.clone()],
    )
    .unwrap();
    assert_eq!(Some(&cue.motion), profile.windup.as_ref());
    assert!(cue.hold);
    e.kind = EffectVisualKind::Beam;
    let cue = motion_cue(
        &profiles(),
        state.recipe.as_ref().unwrap().core.class(),
        Some(&state),
        0,
        7,
        &[e],
    )
    .unwrap();
    assert_eq!(cue.motion, profile.release);
    assert_ne!(Some(&profile.release), profile.windup.as_ref());
    assert!(!cue.hold);
    // Cancellation, packet omission and fog do not manufacture a release.
    assert!(
        motion_cue(
            &profiles(),
            state.recipe.as_ref().unwrap().core.class(),
            Some(&state),
            0,
            7,
            &[]
        )
        .is_none()
    );
}

fn effect(skill: SkillId, kind: EffectVisualKind) -> SkillEffectState {
    SkillEffectState {
        id: 1,
        owner_id: 7,
        owner_team: shared::map::Team::Green,
        skill,
        kind,
        position: [0.0; 2],
        end: [10.0, 0.0],
        radius: 0.8,
        remaining_secs: 0.5,
        armed: true,
        consumed_segments: 0,
    }
}

#[test]
fn hidden_or_other_caster_warning_does_not_drive_this_hero() {
    let state = LoadoutState {
        recipe: Some(shared::loadout::CoreId::Dawnweaver.preset()),
        ..default()
    };
    let mut e = effect(SkillId::DawnRay, EffectVisualKind::BeamWarning);
    e.owner_id = 0;
    assert!(
        motion_cue(
            &profiles(),
            state.recipe.as_ref().unwrap().core.class(),
            Some(&state),
            3,
            7,
            &[e.clone()]
        )
        .is_none()
    );
    e.owner_id = 8;
    assert!(
        motion_cue(
            &profiles(),
            state.recipe.as_ref().unwrap().core.class(),
            Some(&state),
            3,
            7,
            &[e]
        )
        .is_none()
    );
}

#[test]
fn own_windup_cue_is_independent_of_the_latest_action_slot() {
    use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};
    let registry = profiles();
    let class = HeroClass::Dawnweaver;
    let state = LoadoutState {
        recipe: Some(shared::loadout::CoreId::Dawnweaver.preset()),
        ..default()
    };
    let ray = registry.profile(SkillId::DawnRay).unwrap();
    let own =
        |effects: &[SkillEffectState]| own_windup_cue(&registry, class, Some(&state), 7, effects);
    let warning = SkillEffectState {
        id: 9,
        ..effect(SkillId::DawnRay, EffectVisualKind::BeamWarning)
    };

    // The warning alone gives the held windup and names its effect.
    let (id, held) = own(std::slice::from_ref(&warning)).unwrap();
    assert_eq!(id, 9);
    assert_eq!(Some(&held.motion), ray.windup.as_ref());
    assert!(held.hold);
    // It is what the ray's own slot gives, and no other slot gives it: a basic attack, a
    // recast or another cast as the latest action would each replace it.
    let by_slot = |slot: u8| {
        motion_cue(
            &registry,
            class,
            Some(&state),
            slot,
            7,
            std::slice::from_ref(&warning),
        )
    };
    assert_eq!(by_slot(3), Some(held.clone()));
    for slot in [BASIC_ATTACK_ACTION_SLOT, 0, 1, 2] {
        assert!(by_slot(slot).is_some_and(|cue| !cue.hold), "slot {slot}");
    }

    // The same effect as a beam is the release, once it has fired.
    let beam = SkillEffectState {
        kind: EffectVisualKind::Beam,
        ..warning.clone()
    };
    let (id, release) = own(std::slice::from_ref(&beam)).unwrap();
    assert_eq!((id, release.hold), (9, false));
    assert_eq!(release.motion, ray.release);
    // The newest effect speaks: a second warning outranks the beam of the first cast.
    let second = SkillEffectState {
        id: 12,
        ..warning.clone()
    };
    assert_eq!(
        own(&[second.clone(), beam.clone()]),
        Some((12, held.clone()))
    );
    assert_eq!(own(&[beam, second]), Some((12, held.clone())));

    // Nothing replicated, a hidden owner, another hero's warning: no cue.
    assert_eq!(own(&[]), None);
    for owner in [0, 8] {
        let other = SkillEffectState {
            owner_id: owner,
            ..warning.clone()
        };
        assert_eq!(own(std::slice::from_ref(&other)), None, "owner {owner}");
        assert_eq!(
            own_windup_cue(&registry, class, Some(&state), 0, &[other]),
            None,
            "a hero without an id owns nothing"
        );
    }
    // Only a skill of the accepted kit moves its hero, and a recipe that does not resolve
    // for the class moves nobody.
    let foreign = effect(SkillId::HorizonWave, EffectVisualKind::BeamWarning);
    assert_eq!(own(&[foreign]), None);
    assert_eq!(
        own_windup_cue(
            &registry,
            HeroClass::Stormfist,
            Some(&state),
            7,
            std::slice::from_ref(&warning)
        ),
        None
    );
    // An effect of a row without a windup asks nothing of the body.
    let field = effect(SkillId::DawnField, EffectVisualKind::Field);
    assert_eq!(own(&[field]), None);

    // A wave is its release only when it has just fired; it travels for seconds after.
    let rift = LoadoutState {
        recipe: Some(shared::loadout::CoreId::Riftshot.preset()),
        ..default()
    };
    let tail = category::tail_secs(SkillId::HorizonWave).unwrap();
    let wave = |remaining_secs: f32, kind: EffectVisualKind| {
        let effect = SkillEffectState {
            remaining_secs,
            ..effect(SkillId::HorizonWave, kind)
        };
        own_windup_cue(&registry, HeroClass::Riftshot, Some(&rift), 7, &[effect])
    };
    assert!(wave(tail + 0.4, EffectVisualKind::BeamWarning).is_some_and(|(_, cue)| cue.hold));
    assert!(wave(tail - 0.05, EffectVisualKind::Bolt).is_some_and(|(_, cue)| !cue.hold));
    assert!(wave(tail - 0.2, EffectVisualKind::Bolt).is_some());
    assert_eq!(wave(tail - 0.3, EffectVisualKind::Bolt), None);
    assert_eq!(wave(0.5, EffectVisualKind::Bolt), None);
}

/// A hero of one class with the preset kit of its core.
fn preset(core: shared::loadout::CoreId) -> (shared::HeroClass, LoadoutState) {
    let recipe = core.preset();
    (
        recipe.core.class(),
        LoadoutState {
            recipe: Some(recipe),
            ..default()
        },
    )
}

/// The slot that holds a skill in a kit.
fn slot_of(loadout: &LoadoutState, skill: SkillId) -> u8 {
    let slot = loadout
        .recipe
        .as_ref()
        .unwrap()
        .skills
        .iter()
        .position(|equipped| *equipped == skill);
    slot.unwrap() as u8
}

/// For the two skills that warn and then fire, the plan is what the slot alone gave before,
/// case by case. It differs in one place only: a warning keeps its windup when another
/// action is the latest.
#[test]
fn motion_plan_equals_motion_cue_for_warn_fire() {
    use shared::BASIC_ATTACK_ACTION_SLOT;
    use shared::loadout::CoreId;
    let registry = SkillPresentation::target();
    for (core, skill) in [
        (CoreId::Dawnweaver, SkillId::DawnRay),
        (CoreId::Riftshot, SkillId::HorizonWave),
    ] {
        let (class, state) = preset(core);
        let own_slot = slot_of(&state, skill);
        let profile = registry.profile(skill).unwrap();
        let fired = category::own_kinds(skill).last().copied().unwrap();
        let at = |kind, owner_id, remaining_secs| SkillEffectState {
            owner_id,
            remaining_secs,
            ..effect(skill, kind)
        };
        let tail = category::tail_secs(skill).unwrap();
        let cases: [(&str, Vec<SkillEffectState>); 8] = [
            ("nothing replicated", vec![]),
            (
                "warning",
                vec![at(EffectVisualKind::BeamWarning, 7, tail + 0.4)],
            ),
            ("just fired", vec![at(fired, 7, tail)]),
            ("fired a while ago", vec![at(fired, 7, tail - 1.0)]),
            (
                "hidden owner",
                vec![at(EffectVisualKind::BeamWarning, 0, tail + 0.4)],
            ),
            (
                "another hero",
                vec![at(EffectVisualKind::BeamWarning, 8, tail + 0.4)],
            ),
            ("another hero fired", vec![at(fired, 8, tail)]),
            (
                "warning beside another hero's beam",
                vec![
                    at(fired, 8, tail),
                    at(EffectVisualKind::BeamWarning, 7, tail + 0.4),
                ],
            ),
        ];
        for (case, effects) in &cases {
            let plan = |slot, sequence, recast| {
                motion_plan(&MotionInputs {
                    registry: &registry,
                    class,
                    loadout: Some(&state),
                    slot,
                    sequence,
                    recast,
                    owner: 7,
                    effects,
                })
            };
            let by_slot = |slot| motion_cue(&registry, class, Some(&state), slot, 7, effects);
            let name = format!("{} {case}", skill.id());
            // The skill's own slot: identical, whatever the sequence.
            for sequence in [1, 2] {
                assert_eq!(plan(own_slot, sequence, false), by_slot(own_slot), "{name}");
            }
            let held = by_slot(own_slot).filter(|cue| cue.hold);
            if effects
                .iter()
                .any(|own| own.owner_id == 7 && own.kind == EffectVisualKind::BeamWarning)
            {
                assert_eq!(
                    held.as_ref().map(|cue| &cue.motion),
                    profile.windup.as_ref(),
                    "{name}"
                );
            } else {
                assert_eq!(held, None, "{name}");
            }
            // Any other slot: identical too, except that a live warning keeps the body.
            for slot in (0..4).filter(|slot| *slot != own_slot) {
                assert_eq!(
                    plan(slot, 1, false),
                    held.clone().or_else(|| by_slot(slot)),
                    "{name} slot {slot}"
                );
            }
            assert_eq!(
                plan(BASIC_ATTACK_ACTION_SLOT, 1, false),
                held.clone().or_else(|| by_slot(BASIC_ATTACK_ACTION_SLOT)),
                "{name} basic"
            );
            // A recast accepted during the warning does not take the body either.
            if let Some(held) = &held {
                for slot in 0..4 {
                    assert_eq!(plan(slot, 2, true).as_ref(), Some(held), "{name} recast");
                }
            }
        }
        // The release is the row's clip at the row's rate and start; a fired effect is
        // never a hold.
        let release = motion_cue(
            &registry,
            class,
            Some(&state),
            own_slot,
            7,
            &[at(fired, 7, tail)],
        );
        assert_eq!(
            release,
            Some(MotionCue::action(
                &profile.release,
                profile.motion.rate,
                profile.motion.start
            )),
            "{}",
            skill.id()
        );
    }
}

/// The windup of a row: a loop repeats under the telegraph, and a clip the row fits spans
/// the telegraph once.
#[test]
fn a_windup_is_a_loop_or_fitted_to_its_telegraph() {
    let library = crate::humanoid::SharedHumanoidMotion::embedded().unwrap();
    let target = SkillPresentation::target();
    let mut held = 0;
    for (id, profile) in target.rows() {
        let Some(windup) = &profile.windup else {
            continue;
        };
        let skill = SkillId::from_id(id).unwrap();
        let cue = MotionCue::windup(profile, skill).unwrap();
        let clip = &library.clips[windup];
        assert_eq!(
            (&cue.motion, cue.hold, cue.start),
            (windup, true, 0.0),
            "{id}"
        );
        assert_eq!(cue.looping, clip.looping, "{id}");
        if profile.motion.fit_windup {
            let telegraph = category::telegraph_secs(skill).unwrap();
            assert!(!cue.looping, "{id}");
            assert!((clip.duration / cue.rate - telegraph).abs() < 1e-4, "{id}");
        } else {
            assert!(
                cue.looping,
                "{id}: a clip that is not fitted would freeze on its last key"
            );
            assert_eq!(cue.rate, 1.0, "{id}");
        }
        held += 1;
    }
    assert_eq!(held, 5);
    // Dawn Ray: a half-second clip under a 0.8 s warning.
    let ray = target.profile(SkillId::DawnRay).unwrap();
    assert_eq!(
        MotionCue::windup(ray, SkillId::DawnRay).unwrap().rate,
        0.625
    );
    // A row that does not ask for the fit plays its windup as the clip is.
    let unfitted = SkillPresentation::target_with(|config| {
        config["skills"]["dawn_ray"]["motion"]["fit_windup"] = false.into();
    });
    let ray = unfitted.profile(SkillId::DawnRay).unwrap();
    assert_eq!(MotionCue::windup(ray, SkillId::DawnRay).unwrap().rate, 1.0);
    // Any clip can be fitted, and the fit is clamped to the rates a row may author: a
    // 0.17 s pose is not stretched to a fifth of its speed, nor a 1.5 s swing rushed past
    // twice its speed.
    assert_eq!(FITTED_RATES, (0.5, 2.0));
    for (skill, clip, rate) in [
        (SkillId::DawnRay, "cleave_slam", 1.125),
        (SkillId::DawnRay, "rally_raise", 0.75),
        (SkillId::DawnRay, "pistol_aim", 0.5),
        (SkillId::HorizonWave, "attack", 2.0),
    ] {
        let mut config = schema_rules::samples();
        config["skills"][skill.id()]["windup"] = clip.into();
        config["skills"][skill.id()]["motion"]["fit_windup"] = true.into();
        let registry = schema_rules::parse(&config).unwrap();
        let cue = MotionCue::windup(registry.profile(skill).unwrap(), skill).unwrap();
        assert!((cue.rate - rate).abs() < 1e-4, "{clip}: {}", cue.rate);
    }
}

/// A fuse and a parry: the windup is asked for from the accepted cast and held while the
/// telegraph of the hero is replicated, whatever is accepted during it. The release is not
/// in the plan: a fuse fires by leaving the snapshot, and the stage tracker names that.
#[test]
fn fuse_and_parry_plans_hold_the_windup_from_the_accepted_cast() {
    use shared::BASIC_ATTACK_ACTION_SLOT;
    use shared::loadout::CoreId;
    let registry = SkillPresentation::target();
    for (core, skill) in [
        (CoreId::Cinderforge, SkillId::FurnaceBreath),
        (CoreId::Orbitwright, SkillId::OrbitalCollapse),
        (CoreId::Edgeweaver, SkillId::MirrorGuard),
    ] {
        let (class, mut state) = preset(core);
        let own_slot = slot_of(&state, skill);
        let profile = registry.profile(skill).unwrap();
        let parry = profile.phase(SkillKey::Modular(skill)) == MotionPhase::Parry;
        let windup = MotionCue::windup(profile, skill).unwrap();
        assert!(windup.hold && windup.looping, "{}", skill.id());
        let kind = category::own_kinds(skill)[0];
        let telegraph = SkillEffectState {
            id: 9,
            ..effect(skill, kind)
        };
        let plan = |state: &LoadoutState, slot, effects: &[SkillEffectState]| {
            motion_plan(&MotionInputs {
                registry: &registry,
                class,
                loadout: Some(state),
                slot,
                sequence: 3,
                recast: false,
                owner: 7,
                effects,
            })
        };
        let name = skill.id();

        // The accepted cast alone, before any effect reaches the client. A parry also
        // needs the replicated stance: an edge without it holds nothing.
        if parry {
            assert_eq!(plan(&state, own_slot, &[]), None, "{name}");
            state.parrying = true;
        }
        assert_eq!(plan(&state, own_slot, &[]), Some(windup.clone()), "{name}");
        // Another slot is the latest action and nothing is replicated: that action plays.
        assert!(
            plan(&state, BASIC_ATTACK_ACTION_SLOT, &[]).is_some_and(|cue| !cue.hold),
            "{name}"
        );

        // The telegraph is replicated: the windup, from every slot, and with its id.
        let effects = [telegraph.clone()];
        assert_eq!(
            own_windup_cue(&registry, class, Some(&state), 7, &effects),
            Some((9, windup.clone())),
            "{name}"
        );
        for slot in [own_slot, BASIC_ATTACK_ACTION_SLOT, (own_slot + 1) % 4] {
            assert_eq!(
                plan(&state, slot, &effects),
                Some(windup.clone()),
                "{name} {slot}"
            );
        }
        // The stance flag is not needed while the barrier itself is seen.
        state.parrying = false;
        assert_eq!(
            plan(&state, own_slot, &effects),
            Some(windup.clone()),
            "{name}"
        );

        // Another hero's telegraph, a hidden owner's, and a kind the skill does not
        // replicate as its telegraph hold nobody.
        for other in [
            SkillEffectState {
                owner_id: 8,
                ..telegraph.clone()
            },
            SkillEffectState {
                owner_id: 0,
                ..telegraph.clone()
            },
            SkillEffectState {
                kind: EffectVisualKind::Orb,
                ..telegraph.clone()
            },
        ] {
            assert_eq!(
                own_windup_cue(&registry, class, Some(&state), 7, &[other]),
                None,
                "{name}"
            );
        }
        // The release of the row, for the moment the tracker reports.
        assert_eq!(
            release_cue(&registry, skill),
            Some(MotionCue::action(
                &profile.release,
                profile.motion.rate,
                profile.motion.start
            )),
            "{name}"
        );
    }
}

/// A recast edge plays the recast clip of its row at the recast rate, from its first key.
#[test]
fn a_recast_edge_plays_the_recast_clip() {
    let target = SkillPresentation::target();
    let mut recasts = 0;
    for class in shared::HeroClass::ALL {
        for slot in 0..4 {
            let profile = target.action_profile(class, None, slot).unwrap();
            let plan = |registry, recast| {
                motion_plan(&MotionInputs {
                    registry,
                    class,
                    loadout: None,
                    slot,
                    sequence: 5,
                    recast,
                    owner: 7,
                    effects: &[],
                })
            };
            let first = plan(&target, false);
            match &profile.motion.recast {
                Some(clip) => {
                    recasts += 1;
                    assert_eq!(
                        plan(&target, true),
                        Some(MotionCue::action(clip, profile.motion.recast_rate, 0.0)),
                        "{} {slot}",
                        class.id()
                    );
                    assert_ne!(plan(&target, true), first, "{} {slot}", class.id());
                }
                // A row without a recast clip plays what its slot plays.
                None => assert_eq!(plan(&target, true), first, "{} {slot}", class.id()),
            }
        }
    }
    assert_eq!(recasts, 8);
}

/// AC12 as the engine plays it: every clip that starts on an accepted edge of the final
/// data shows its contact pose within 0.15 s, measured on the cue the plan returns and not
/// on the fields of the row. A cue that is held starts no contact: it waits for a telegraph.
#[test]
fn contact_rule_holds() {
    use shared::BASIC_ATTACK_ACTION_SLOT;
    let target = SkillPresentation::target();
    let library = crate::humanoid::SharedHumanoidMotion::embedded().unwrap();
    let mut edges = 0;
    let mut held = Vec::new();
    let mut check = |user: String, cue: Option<MotionCue>| {
        let cue = cue.unwrap_or_else(|| panic!("{user}: the edge plays nothing"));
        if cue.hold {
            held.push(user);
            return;
        }
        let clip = &library.clips[&cue.motion];
        let contact = library
            .contact(&cue.motion)
            .unwrap_or_else(|| panic!("{user}: {} has no contact", cue.motion));
        let delay = (contact - cue.start * clip.duration) / cue.rate;
        assert!(
            delay <= schema::CONTACT_LIMIT_SECS + 1e-4,
            "{user}: {} reaches its contact {delay:.3} s after the edge",
            cue.motion
        );
        assert!(!cue.looping && cue.rate > 0.0, "{user}");
        edges += 1;
    };
    for class in shared::HeroClass::ALL {
        let kit = shared::loadout::preset_for_class(class);
        // The accepted cast of a parry comes with the replicated stance.
        let cast = LoadoutState {
            recipe: kit.map(|kit| kit.recipe()),
            parrying: true,
            ..default()
        };
        // A warn_fire row plays nothing on its edge; its own fired effect starts the release.
        for slot in 0..4u8 {
            let modular = kit.map(|kit| kit.skills()[usize::from(slot)]);
            let profile = target.action_profile(class, None, slot).unwrap();
            let warns = modular.is_some_and(|skill| {
                profile.phase(SkillKey::Modular(skill)) == MotionPhase::WarnFire
            });
            let fired: Vec<SkillEffectState> = modular
                .filter(|_| warns)
                .map(|skill| SkillEffectState {
                    remaining_secs: category::tail_secs(skill).unwrap(),
                    ..effect(skill, *category::own_kinds(skill).last().unwrap())
                })
                .into_iter()
                .collect();
            for recast in [false, true] {
                if recast && !modular.is_some_and(category::has_recast) {
                    continue;
                }
                let cue = motion_plan(&MotionInputs {
                    registry: &target,
                    class,
                    loadout: Some(&cast),
                    slot,
                    sequence: 1,
                    recast,
                    owner: 7,
                    effects: &fired,
                });
                check(format!("{} {slot} recast {recast}", class.id()), cue);
            }
        }
        for (mode, weapon_mode) in [
            ("", WeaponMode::Repeater),
            (":rockets", WeaponMode::Rockets),
        ] {
            let state = LoadoutState {
                recipe: kit.map(|kit| kit.recipe()),
                weapon_mode,
                ..default()
            };
            for sequence in [1, 2] {
                let cue = motion_plan(&MotionInputs {
                    registry: &target,
                    class,
                    loadout: Some(&state),
                    slot: BASIC_ATTACK_ACTION_SLOT,
                    sequence,
                    recast: false,
                    owner: 7,
                    effects: &[],
                });
                check(format!("basic:{}{mode} {sequence}", class.id()), cue);
            }
        }
    }
    // 68 first casts less the three that hold a fuse or a stance, 8 recasts, and the basic
    // attack of 17 classes on both turns in both weapon modes.
    assert_eq!(edges, 68 - 3 + 8 + 17 * 4);
    assert_eq!(
        held,
        [
            "cinderforge 1 recast false",
            "edgeweaver 1 recast false",
            "orbitwright 3 recast false"
        ]
    );
}

#[test]
fn malformed_profile_cannot_introduce_gameplay_or_unknown_motion() {
    assert_eq!(profiles().skills.len(), shared::HeroClass::ALL.len() * 4);
    let rejected = |edit: fn(&mut serde_json::Value)| {
        let mut config = schema_rules::samples();
        edit(&mut config);
        schema_rules::parse(&config).is_err()
    };
    assert!(!rejected(|_| {}));
    assert!(rejected(|config| {
        config["skills"]["wild_zap"]["release"] = "missing_clip".into();
    }));
    // No level of a profile accepts a field the schema does not name.
    assert!(rejected(|config| config["damage"] = 999.into()));
    for block in ["", "motion", "cast", "body", "impact", "sound"] {
        let mut config = schema_rules::samples();
        let row = &mut config["skills"]["winter_shard"];
        let target = if block.is_empty() {
            row
        } else {
            &mut row[block]
        };
        assert!(target.is_object(), "{block}");
        target["damage"] = 999.into();
        assert!(
            schema_rules::parse(&config).is_err_and(|error| error.contains("unknown field")),
            "{block}"
        );
    }
    let mut e = effect(SkillId::WildRocket, EffectVisualKind::Rocket);
    assert!(effects::valid_effect(&e));
    e.end[0] = f32::NAN;
    assert!(!effects::valid_effect(&e));
}

#[test]
fn hdr_gain_is_bounded_and_older_manifests_keep_a_default() {
    let mut config: serde_json::Value =
        serde_json::from_str(include_str!("../../assets/config/skills.skillfx")).unwrap();
    for gain in [-1.0, 0.0, 8.01, 1e20] {
        config["skills"]["dawn_bind"]["hdr_gain"] = serde_json::json!(gain);
        assert!(SkillPresentation::parse(&config.to_string()).is_err());
    }
    config["skills"]["dawn_bind"]
        .as_object_mut()
        .unwrap()
        .remove("hdr_gain");
    let parsed = SkillPresentation::parse(&config.to_string()).unwrap();
    assert_eq!(parsed.profile(SkillId::DawnBind).unwrap().hdr_gain, 3.0);
}

#[test]
fn all_roster_abilities_have_profiles_and_valid_recipes_override_default_slots() {
    let registry = profiles();
    for class in shared::HeroClass::ALL {
        let loadout = shared::loadout::preset_for_class(class).map(|recipe| LoadoutState {
            recipe: Some(recipe.recipe()),
            ..default()
        });
        for slot in 0..4 {
            assert!(
                registry
                    .action_profile(class, loadout.as_ref(), slot)
                    .is_some(),
                "{} / {slot}",
                class.id()
            );
        }
    }
    let mut recipe = shared::loadout::CoreId::Wildspark.preset();
    recipe.skills = [
        SkillId::IronBoundary,
        SkillId::WinterDivide,
        SkillId::MountainEcho,
        SkillId::HorizonWave,
    ];
    let state = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    for slot in 0..4 {
        let a = registry
            .action_profile(shared::HeroClass::Wildspark, Some(&state), slot)
            .unwrap();
        let b = registry
            .profile(state.recipe.as_ref().unwrap().skills[slot as usize])
            .unwrap();
        assert!(std::ptr::eq(a, b));
        assert_eq!(a.release, b.release);
    }
    assert!(
        registry
            .action_profile(shared::HeroClass::Warrior, Some(&state), 0)
            .is_none(),
        "mismatched core must not substitute a class skill"
    );
    assert!(
        registry
            .action_profile(shared::HeroClass::Warrior, None, 255)
            .is_none()
    );
}

#[test]
fn horizon_release_needs_the_authoritative_projectile_not_a_missing_warning() {
    let state = LoadoutState {
        recipe: Some(shared::loadout::CoreId::Riftshot.preset()),
        ..default()
    };
    let warning = effect(SkillId::HorizonWave, EffectVisualKind::BeamWarning);
    assert!(
        motion_cue(
            &profiles(),
            state.recipe.as_ref().unwrap().core.class(),
            Some(&state),
            3,
            7,
            &[warning]
        )
        .unwrap()
        .hold
    );
    let bolt = effect(SkillId::HorizonWave, EffectVisualKind::Bolt);
    assert!(
        !motion_cue(
            &profiles(),
            state.recipe.as_ref().unwrap().core.class(),
            Some(&state),
            3,
            7,
            &[bolt]
        )
        .unwrap()
        .hold
    );
    assert!(
        motion_cue(
            &profiles(),
            state.recipe.as_ref().unwrap().core.class(),
            Some(&state),
            3,
            7,
            &[]
        )
        .is_none()
    );
}

#[test]
fn mismatched_or_malformed_recipes_cannot_select_skill_or_basic_motion() {
    use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};
    let mut recipe = shared::loadout::CoreId::Adventurer.preset();
    let state = LoadoutState {
        recipe: Some(recipe.clone()),
        ..default()
    };
    assert!(equipped_skill(HeroClass::Dawnweaver, Some(&state), 0).is_none());
    assert!(motion_cue(&profiles(), HeroClass::Dawnweaver, Some(&state), 0, 7, &[]).is_none());
    assert!(
        motion_cue(
            &profiles(),
            HeroClass::Dawnweaver,
            Some(&state),
            BASIC_ATTACK_ACTION_SLOT,
            7,
            &[]
        )
        .is_none()
    );
    recipe.skills[1] = recipe.skills[0];
    let malformed = LoadoutState {
        recipe: Some(recipe),
        ..default()
    };
    assert!(
        motion_cue(
            &profiles(),
            HeroClass::Adventurer,
            Some(&malformed),
            BASIC_ATTACK_ACTION_SLOT,
            7,
            &[]
        )
        .is_none()
    );
}

/// The motion table of the basic attacks is data: the row of the class is what plays.
#[test]
fn ranged_basic_attacks_use_aimed_motion_and_dagger_keeps_the_right_hand_thrust() {
    use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};
    let registry = SkillPresentation::target();
    let basic = |class| motion_cue(&registry, class, None, BASIC_ATTACK_ACTION_SLOT, 1, &[]);
    for class in HeroClass::ALL {
        let row = registry.basic(class).unwrap();
        assert_eq!(
            basic(class),
            Some(MotionCue::action(&row.motions[0], row.rate, row.start)),
            "{}",
            class.id()
        );
    }
    // The classes that throw a body aim or throw it; none swings a blade at range.
    for (class, motion) in [
        (HeroClass::Ranger, "aim_loose_r"),
        (HeroClass::Wildspark, "pistol_shoot"),
        (HeroClass::Riftshot, "cast_thrust_r"),
        (HeroClass::Mage, "toss_underhand"),
        (HeroClass::Cleric, "cast"),
    ] {
        assert_eq!(basic(class).unwrap().motion, motion, "{}", class.id());
    }
    assert_eq!(basic(HeroClass::Adventurer).unwrap().motion, "dagger_stab");
}

/// Rule E-13 and the alternation of two motions: the row is that of the kit's core, the
/// first motion plays on odd action sequences, and a repeater in rocket mode plays its
/// `rockets` entry.
#[test]
fn basic_attack_motions_follow_the_core_the_sequence_and_the_weapon_mode() {
    use shared::loadout::CoreId;
    use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};
    let target = SkillPresentation::target();
    let plan = |class, loadout: Option<&LoadoutState>, sequence| {
        motion_plan(&MotionInputs {
            registry: &target,
            class,
            loadout,
            slot: BASIC_ATTACK_ACTION_SLOT,
            sequence,
            recast: false,
            owner: 7,
            effects: &[],
        })
    };
    for class in HeroClass::ALL {
        let row = target.basic(class).unwrap();
        let (odd, even) = (&row.motions[0], row.motions.last().unwrap());
        for sequence in [1, 2, 3, 4, 41, 42] {
            let turn = if sequence % 2 == 1 { odd } else { even };
            assert_eq!(
                plan(class, None, sequence),
                Some(MotionCue::action(turn, row.rate, row.start)),
                "{} {sequence}",
                class.id()
            );
        }
    }
    // Four classes of the final data alternate two motions.
    let two: Vec<_> = HeroClass::ALL
        .into_iter()
        .filter(|class| target.basic(*class).unwrap().motions.len() == 2)
        .map(HeroClass::id)
        .collect();
    assert_eq!(two, ["warden", "dawnweaver", "stormfist", "veilstalker"]);

    // The core decides, also with the skills of other kits in its slots.
    let mut mixed = CoreId::Stormfist.preset();
    mixed.skills = [
        SkillId::DawnRay,
        SkillId::WildZap,
        SkillId::DawnField,
        SkillId::DawnBind,
    ];
    let mixed = LoadoutState {
        recipe: Some(mixed),
        ..default()
    };
    assert_eq!(
        plan(HeroClass::Stormfist, Some(&mixed), 1),
        plan(HeroClass::Stormfist, None, 1)
    );
    // A recipe that does not resolve for the class moves nothing.
    assert_eq!(plan(HeroClass::Dawnweaver, Some(&mixed), 1), None);

    // The launcher: the `rockets` entry of the repeater row, for that mode only.
    let wildspark = target.basic(HeroClass::Wildspark).unwrap();
    let rockets = wildspark.rockets.as_deref().unwrap();
    assert_ne!(rockets.rate, wildspark.rate);
    let armed = |weapon_mode| LoadoutState {
        recipe: Some(CoreId::Wildspark.preset()),
        weapon_mode,
        ..default()
    };
    assert_eq!(
        plan(HeroClass::Wildspark, Some(&armed(WeaponMode::Rockets)), 1),
        Some(MotionCue::action(
            &rockets.motions[0],
            rockets.rate,
            rockets.start
        ))
    );
    assert_eq!(
        plan(HeroClass::Wildspark, Some(&armed(WeaponMode::Repeater)), 1),
        Some(MotionCue::action(
            &wildspark.motions[0],
            wildspark.rate,
            wildspark.start
        ))
    );
    // A class without a `rockets` entry plays its one row in either mode.
    let mut riftshot = armed(WeaponMode::Rockets);
    riftshot.recipe = Some(CoreId::Riftshot.preset());
    assert_eq!(
        plan(HeroClass::Riftshot, Some(&riftshot), 1),
        plan(HeroClass::Riftshot, None, 1)
    );
}

#[test]
fn version_1_files_are_rejected_with_the_reason() {
    assert_eq!(
        SkillPresentation::parse(SCHEMA_1).err().as_deref(),
        Some("Unsupported skill presentation schema_version 1 (expected 2)")
    );
    // Any other failure of the file keeps its own message.
    assert!(SkillPresentation::parse("{").is_err_and(|error| !error.contains("schema_version")));
}

#[test]
fn the_packaged_registry_has_a_theme_and_a_basic_row_for_every_class() {
    let registry = profiles();
    for class in shared::HeroClass::ALL {
        assert!(registry.theme(class).is_some(), "{}", class.id());
        assert!(registry.basic(class).is_some(), "{}", class.id());
    }
    assert_eq!(registry.themes.len(), shared::HeroClass::ALL.len());
    assert_eq!(registry.basic_attacks.len(), shared::HeroClass::ALL.len());
    let frost = registry.theme(shared::HeroClass::Frostguard).unwrap();
    assert_eq!(frost.secondary, [0.86, 0.94, 1.0]);
    assert_eq!(frost.accent, [0.55, 0.6, 1.0]);
}

#[test]
fn the_origin_fingerprint_is_fnv_1a_of_the_packaged_bytes() {
    assert_eq!(fnv64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv64(b"foobar"), 0x8594_4171_f739_67e8);
    let bytes = include_bytes!("../../assets/config/skills.skillfx");
    let loaded = LoadedPresentation::from_bytes(bytes).unwrap();
    assert_eq!(loaded.fnv64, fnv64(bytes));
    assert_eq!(loaded.registry.rows().count(), 68);
    assert!(LoadedPresentation::from_bytes(&[0xff, 0xfe]).is_err());
    assert!(LoadedPresentation::from_bytes(SCHEMA_1.as_bytes()).is_err());

    // The packaged file replaces the embedded copy and says so.
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<LoadedPresentation>()
        .init_resource::<SkillPresentation>()
        .init_resource::<SkillPresentationOrigin>()
        .init_resource::<Pending>()
        .add_systems(Update, apply_config);
    assert_eq!(
        *app.world().resource::<SkillPresentationOrigin>(),
        SkillPresentationOrigin::Embedded
    );
    let handle = app
        .world_mut()
        .resource_mut::<Assets<LoadedPresentation>>()
        .add(loaded);
    app.world_mut().resource_mut::<Pending>().0 = Some(handle);
    app.update();
    assert_eq!(
        *app.world().resource::<SkillPresentationOrigin>(),
        SkillPresentationOrigin::Packaged {
            fnv64: fnv64(bytes)
        }
    );
    assert_eq!(
        app.world().resource::<SkillPresentation>().rows().count(),
        68
    );
    assert!(app.world().resource::<Pending>().0.is_none());
}

#[test]
fn identity_reads_motion_family_body_and_impact() {
    use signature::{BodyLead, BodySig, ImpactSig, ProjectileBody};
    assert_eq!(signature::motion_family("slash_down_m"), "slash_down");
    assert_eq!(signature::motion_family("slash_rising_m"), "slash_rising");
    assert_eq!(signature::motion_family("aim_loose_r"), "cast_thrust_r");
    assert_eq!(signature::motion_family("punch"), "punch");

    let registry = schema_rules::parse(&schema_rules::samples()).unwrap();
    let projectiles = crate::combat_visuals::CombatVisualRegistry::from_json(include_str!(
        "../../assets/config/combat_visuals.json"
    ))
    .unwrap();
    let identity = |id: &str| signature::identity(&registry, &projectiles, id).unwrap();

    // A replicated effect: archetype, core mesh, satellites and trail.
    let shard = identity("winter_shard");
    assert_eq!(shard.motion, "punch");
    assert_eq!(
        shard.body,
        BodySig::World {
            archetype: vocab::Archetype::Traveller,
            lead: BodyLead::Mesh(vocab::Silhouette::Shard),
            satellites: Some((vocab::Silhouette::Diamond, vocab::SatelliteLayout::Halo)),
            trail: vocab::Trail::Motes,
        }
    );
    assert_eq!(shard.body.key(), ("traveller".into(), "shard".into()));
    assert_eq!(
        shard.impact,
        ImpactSig::Themed {
            kind: vocab::ImpactKind::ShardBurst,
            lead: vocab::ParticleShape::Diamond
        }
    );
    // A body without a core is named by its satellites.
    assert_eq!(
        identity("furnace_breath").body.key(),
        ("sector".into(), "drop".into())
    );
    // An instant skill: its cast choreography; a default lead counts as the lead.
    assert_eq!(
        identity("anchor_step").body,
        BodySig::Choreography {
            pattern: vocab::AccentPattern::ShieldFlash,
            lead: Some(vocab::ParticleShape::Kite),
            movement: Some(vocab::MovePattern::LeapArc),
        }
    );
    assert_eq!(identity("anchor_step").impact, ImpactSig::None);
    assert_eq!(
        identity("orbital_guard").impact,
        ImpactSig::Themed {
            kind: vocab::ImpactKind::GlowPop,
            lead: vocab::ParticleShape::Glow
        }
    );
    // A legacy projectile is read from the cosmetics file, whatever the row says.
    assert_eq!(
        identity("heroic_strike").body,
        BodySig::Projectile {
            body: ProjectileBody::Form(vocab::ProjectileForm::Wavefront),
            silhouette: Some(vocab::Silhouette::Crescent),
            presentation: vocab::ProjectilePresentation::Wave,
        }
    );
    assert_eq!(identity("heroic_strike").motion, "slash_down");
    // A prop is named by its model, and a skill that deals no damage has no impact.
    assert_eq!(
        identity("guiding_lantern").body.key(),
        ("zone".into(), "model:lantern".into())
    );
    assert_eq!(identity("guiding_lantern").impact, ImpactSig::None);
    assert!(signature::identity(&registry, &projectiles, "fireball").is_none());
}

/// The ratchet: every counter of the packaged registry is closed and stays closed.
#[test]
fn shipped_identity_ratchet() {
    let projectiles = crate::combat_visuals::CombatVisualRegistry::from_json(include_str!(
        "../../assets/config/combat_visuals.json"
    ))
    .unwrap();
    let counts = signature::ratchet(&profiles(), &projectiles);
    assert_eq!(
        signature::SHIPPED_RATCHET,
        signature::RatchetCounts::default()
    );
    for ((name, count), (_, ceiling)) in counts
        .entries()
        .into_iter()
        .zip(signature::SHIPPED_RATCHET.entries())
    {
        assert!(
            count <= ceiling,
            "{name} rose from {ceiling} to {count}: the packaged skills became less distinct"
        );
    }
}

/// What one registry draws with pooled particles, block by block.
#[derive(Debug, Default, PartialEq, Eq)]
struct Drawn {
    accents: usize,
    silent_accents: usize,
    recasts: usize,
    areas: usize,
    moves: usize,
    links: usize,
    impacts: usize,
    expires: usize,
}

/// Runs every particle block of a registry the way the client will and holds each burst to
/// its budget, in both render modes.
fn draw_registry(registry: &SkillPresentation) -> Drawn {
    use crate::game_vfx::ParticleSpec;
    use accents::{CastContext, OneShot, Palette};
    use impacts::ImpactContext;

    const AT: Vec3 = Vec3::new(-7.0, 2.0, 5.0);
    let far = AT + Vec3::new(8.0, 0.0, -6.0);
    let direction = Vec2::new(0.8, -0.6);
    let mut drawn = Drawn::default();
    let within = |specs: &[ParticleSpec], most: usize, secs: f32, what: &str| {
        assert!(specs.len() <= most, "{what}: {} particles", specs.len());
        for spec in specs {
            // `is_sound` poses the particle at the start, the middle and the end of its
            // life in the flat and in the 3D view.
            assert!(spec.is_sound(), "{what}: a pose is not finite");
            assert!(
                spec.end_secs() <= secs + 1e-5,
                "{what}: {} s",
                spec.end_secs()
            );
            assert!(spec.delay <= 0.25, "{what}: waits {} s", spec.delay);
        }
        schema::burst_budget(specs, most, secs, what).unwrap();
    };
    let mut accent =
        |cast: &schema::CastAccent, palette: &Palette, id: Option<SkillId>, what: &str| {
            let area = id.filter(|_| cast.area).and_then(|id| {
                geometry::instant_area(
                    id,
                    &geometry::AreaContext {
                        origin: AT.xz(),
                        arrival: Some(far.xz()),
                        direction,
                        recast: false,
                    },
                )
            });
            assert_eq!(
                area.is_some(),
                cast.area,
                "{what}: a flagged area must exist"
            );
            for recast in [false, true] {
                let ctx = CastContext {
                    origin: AT,
                    direction,
                    recast,
                    area,
                    strike_to: Some(far),
                    sequence: 9,
                };
                let specs = accents::accent_particles(cast, palette, &ctx);
                within(&specs, accents::ACCENT_MAX, accents::ACCENT_SECS, what);
                // A recast pattern is drawn with its own defaults, not the row's timing.
                let own = cast.recast.filter(|_| recast);
                let lifetime = own.map_or(cast.lifetime, |pattern| {
                    schema::CastAccent::plain(pattern).lifetime
                });
                assert!(specs.iter().all(|spec| spec.end_secs() <= lifetime + 1e-5));
                let pattern = own.unwrap_or(cast.pattern);
                // The count a row names is the number of particles it gets; `none` gets none.
                let expected = match (pattern, own, cast.count) {
                    (vocab::AccentPattern::None, ..) => Some(0),
                    (_, None, Some(count)) => Some(usize::from(count)),
                    _ => None,
                };
                if let Some(expected) = expected {
                    assert_eq!(specs.len(), expected, "{what} recast {recast}");
                }
                assert_eq!(
                    specs.is_empty(),
                    pattern == vocab::AccentPattern::None,
                    "{what}"
                );
                if recast {
                    drawn.recasts += usize::from(cast.recast.is_some());
                    continue;
                }
                drawn.accents += 1;
                drawn.silent_accents += usize::from(specs.is_empty());
                drawn.areas += usize::from(area.is_some());
                // A plain accent stays at the caster; an outline and a strike line end at
                // replicated geometry.
                if area.is_none() && pattern != vocab::AccentPattern::StrikeLine {
                    let reach = specs.iter().map(|spec| spec.reach(AT)).fold(0.0, f32::max);
                    assert!(
                        reach <= pattern.base_extent() * cast.scale + 1e-3
                            && reach <= accents::DECORATIVE_REACH + 1e-3,
                        "{what}: reaches {reach}"
                    );
                }
            }
            if let Some(step) = &cast.movement {
                drawn.moves += 1;
                for (from, to) in [(Some(AT), Some(far)), (None, Some(far)), (Some(AT), None)] {
                    let specs = accents::move_particles(step, palette, from, to, 9);
                    assert!(!specs.is_empty(), "{what}: move");
                    within(&specs, accents::MOVE_MAX, accents::MOVE_SECS, what);
                }
            }
            if let Some(shape) = cast.link {
                drawn.links += 1;
                let specs = accents::link_particles(shape, palette, AT, far, 9);
                assert_eq!(specs.len(), accents::LINK_MAX, "{what}: link");
                within(&specs, accents::LINK_MAX, accents::LINK_SECS, what);
            }
        };
    let mut impact =
        |recipe: &schema::ImpactRecipe, palette: &Palette, area_damage: bool, what: &str| {
            drawn.impacts += 1;
            for heading in [None, Some(Vec2::Y)] {
                let ctx = ImpactContext {
                    position: far,
                    ground: far.y,
                    direction,
                    heading,
                    area_damage,
                    receipt: 4,
                    reserved: 0,
                };
                let specs = impacts::impact_particles(recipe, palette, &ctx);
                assert!(!specs.is_empty(), "{what}: impact");
                within(&specs, impacts::IMPACT_MAX, impacts::IMPACT_SECS, what);
                // The particles of the kind and the flash of the hit.
                if let Some(count) = recipe.count {
                    let own = usize::from(count).min(impacts::IMPACT_MAX - 1);
                    assert_eq!(specs.len(), own + 1, "{what}: impact");
                }
                let reach = specs.iter().map(|spec| spec.reach(far)).fold(0.0, f32::max);
                assert!(
                    area_damage || reach <= impacts::SINGLE_TARGET_REACH + 1e-3,
                    "{what}: the impact reaches {reach}"
                );
            }
        };
    for (id, profile) in registry.rows() {
        let key = category::SkillKey::from_id(id).unwrap();
        let palette = Palette::of(profile, registry.theme(key.home()).unwrap());
        if let Some(cast) = &profile.cast {
            accent(cast, &palette, key.modular(), id);
        }
        if let Some(recipe) = &profile.impact {
            impact(recipe, &palette, category::area_damage(key), id);
        }
        let Some(skill) = key.modular() else {
            continue;
        };
        let own = profile
            .body
            .as_ref()
            .zip(category::own_kinds(skill).first().copied());
        let aux = profile.aux.iter().filter_map(|(name, body)| {
            category::aux_kinds(skill)
                .iter()
                .find(|kind| category::kind_id(**kind) == name)
                .map(|kind| (body, *kind))
        });
        for (body, kind) in own.into_iter().chain(aux) {
            let Some(oneshot) = OneShot::of(body.expire) else {
                continue;
            };
            drawn.expires += 1;
            let mut effect = effect(skill, kind);
            effect.radius = 3.0;
            effect.end = [shared::loadout::skill(skill).ability.cast_range, 0.0];
            let geo = geometry::boundary_shape(skill, kind, &effect);
            let specs = accents::stage_oneshot(oneshot, &palette, &geo, 0.0, 9);
            assert!(
                !specs.is_empty(),
                "{id}: expire {:?} on {geo:?}",
                body.expire
            );
            within(&specs, accents::STAGE_MAX, accents::STAGE_SECS, id);
        }
    }
    for class in shared::HeroClass::ALL {
        let Some(basic) = registry.basic(class) else {
            continue;
        };
        let palette = Palette::of_class(registry.theme(class).unwrap());
        for row in std::iter::once(basic).chain(basic.rockets.as_deref()) {
            if let Some(cast) = &row.accent {
                accent(cast, &palette, None, class.id());
            }
            if let Some(recipe) = &row.impact {
                impact(recipe, &palette, false, class.id());
            }
        }
    }
    drawn
}

/// AC13 from data: every accent, move, link, impact and expire one-shot of the packaged
/// file stays inside its budget with finite poses in both backends.
#[test]
fn budget_from_data() {
    // 68 skill accents and 14 basic ones, 51 skill impacts and 18 basic ones, and every
    // modifier of the data contract.
    assert_eq!(
        draw_registry(&profiles()),
        Drawn {
            accents: 68 + 14,
            silent_accents: 1,
            recasts: 7,
            areas: 3,
            moves: 10,
            links: 14,
            impacts: 51 + 18,
            expires: 12,
        }
    );
}

/// Every pattern, kind, move and one-shot with every lead shape, count, scale and lifetime
/// a row could name: the count is exact, the time and the reach are bounded.
#[test]
fn every_pattern_and_kind_is_bounded() {
    use accents::{CastContext, Palette};
    use impacts::ImpactContext;
    use vocab::{AccentPattern, ImpactKind, MovePattern, ParticleShape};

    let palette = Palette::of_class(profiles().theme(shared::HeroClass::Mage).unwrap());
    let at = Vec3::new(2.0, 0.0, 1.0);
    for pattern in AccentPattern::ALL {
        if *pattern == AccentPattern::None {
            continue;
        }
        let base = pattern.base_extent();
        // The largest scale the parser lets this pattern have.
        let most = if base > 0.0 {
            (2.0 / base).min(2.0)
        } else {
            2.0
        };
        for shape in ParticleShape::ALL {
            for count in 1..=8 {
                for scale in [0.4, 1.0, most] {
                    for lifetime in [0.12, 0.35, 0.5] {
                        let cast = schema::CastAccent {
                            shape: Some(*shape),
                            count: Some(count),
                            scale,
                            lifetime,
                            ..schema::CastAccent::plain(*pattern)
                        };
                        let ctx = CastContext {
                            origin: at,
                            direction: Vec2::new(-0.6, 0.8),
                            recast: false,
                            area: None,
                            strike_to: Some(at + Vec3::new(3.0, 0.5, 4.0)),
                            sequence: u64::from(count) * 7,
                        };
                        let specs = accents::accent_particles(&cast, &palette, &ctx);
                        let name = format!(
                            "{} {} x{count} {scale} {lifetime}",
                            pattern.id(),
                            shape.id()
                        );
                        assert_eq!(specs.len(), usize::from(count), "{name}");
                        for spec in &specs {
                            assert!(spec.is_sound(), "{name}");
                            assert!(spec.end_secs() <= lifetime + 1e-5, "{name}");
                            assert!(spec.delay <= 0.25 + 1e-6, "{name}");
                            // A decorative accent stays inside its extent.
                            assert!(
                                *pattern == AccentPattern::StrikeLine
                                    || spec.reach(at) <= base * scale + 1e-3,
                                "{name}: {}",
                                spec.reach(at)
                            );
                        }
                    }
                }
            }
        }
        // Without a count the pattern has a small one of its own.
        let plain = schema::CastAccent::plain(*pattern);
        let ctx = CastContext {
            origin: at,
            direction: Vec2::X,
            recast: false,
            area: None,
            strike_to: Some(at + Vec3::X * 5.0),
            sequence: 1,
        };
        let specs = accents::accent_particles(&plain, &palette, &ctx);
        assert!((1..=6).contains(&specs.len()), "{}", pattern.id());
    }
    for kind in ImpactKind::ALL {
        for shape in ParticleShape::ALL {
            for count in 1..=12 {
                for scale in [0.3, 1.0, 2.0] {
                    for lifetime in [0.08, 0.45, 1.2] {
                        for area_damage in [false, true] {
                            let recipe = schema::ImpactRecipe {
                                kind: *kind,
                                shape: Some(*shape),
                                count: Some(count),
                                scale,
                                lifetime,
                                slots: None,
                            };
                            let ctx = ImpactContext {
                                position: at,
                                ground: at.y,
                                direction: Vec2::new(0.0, -1.0),
                                heading: Some(Vec2::X),
                                area_damage,
                                receipt: u64::from(count) * 13,
                                reserved: 0,
                            };
                            let specs = impacts::impact_particles(&recipe, &palette, &ctx);
                            let name =
                                format!("{} {} x{count} {scale} {lifetime}", kind.id(), shape.id());
                            let own = usize::from(count).min(impacts::IMPACT_MAX - 1);
                            assert_eq!(specs.len(), own + 1, "{name}");
                            for spec in &specs {
                                assert!(spec.is_sound(), "{name}");
                                assert!(spec.end_secs() <= lifetime * 1.4 + 1e-5, "{name}");
                                assert!(spec.end_secs() <= impacts::IMPACT_SECS, "{name}");
                                assert!(spec.delay <= 0.25 + 1e-6, "{name}");
                            }
                        }
                    }
                }
            }
        }
    }
    for pattern in MovePattern::ALL {
        for shape in ParticleShape::ALL.iter().copied().map(Some).chain([None]) {
            for travel in [0.0, 0.4, 3.0, 12.0, 60.0] {
                let to = at + Vec3::new(0.6, 0.0, -0.8) * travel;
                let step = schema::MoveSpec {
                    pattern: *pattern,
                    shape,
                };
                for (from, to) in [(Some(at), Some(to)), (None, Some(to)), (Some(at), None)] {
                    let specs = accents::move_particles(&step, &palette, from, to, 3);
                    assert!(
                        (1..=accents::MOVE_MAX).contains(&specs.len()),
                        "{}",
                        pattern.id()
                    );
                    schema::burst_budget(&specs, accents::MOVE_MAX, accents::MOVE_SECS, "move")
                        .unwrap();
                    assert!(
                        specs.iter().all(|spec| spec.delay <= 0.25),
                        "{}",
                        pattern.id()
                    );
                }
            }
        }
    }
    for travel in [0.0, 0.4, 3.0, 12.0, 60.0] {
        let to = at + Vec3::X * travel;
        for (from, to) in [(Some(at), Some(to)), (None, Some(to)), (Some(at), None)] {
            let specs = accents::drag_streak(from, to, 3);
            schema::burst_budget(&specs, accents::MOVE_MAX, accents::MOVE_SECS, "drag").unwrap();
            assert!(!specs.is_empty());
        }
    }
}

/// AC16: in the flat view every final row still draws its accent and its impact, on the
/// ground point of the cast or of the receipt, with a visible size.
#[test]
fn every_target_row_draws_a_flat_accent_and_impact() {
    use accents::{CastContext, Palette};
    use impacts::ImpactContext;

    let registry = target::target();
    let at = Vec3::new(12.0, 3.0, -4.0);
    let flat = |spec: &crate::game_vfx::ParticleSpec| {
        let pose = spec.pose_at(spec.lifetime * 0.5, true, Quat::IDENTITY);
        // The flat view drops the height: the particle is drawn over its ground point.
        let ground = spec
            .pose_at(spec.lifetime * 0.5, false, Quat::IDENTITY)
            .translation
            .xz();
        pose.translation.truncate().distance(ground) < 1e-4
            && pose.translation.z > 0.0
            && pose.scale.truncate().min_element() > 0.0
            && pose.rotation.is_finite()
    };
    let mut rows = 0;
    for (id, profile) in registry.rows() {
        let key = category::SkillKey::from_id(id).unwrap();
        let palette = Palette::of(profile, registry.theme(key.home()).unwrap());
        let cast = profile.cast.as_ref().unwrap();
        let specs = accents::accent_particles(
            cast,
            &palette,
            &CastContext {
                origin: at,
                direction: Vec2::Y,
                recast: false,
                area: None,
                strike_to: Some(at + Vec3::Z * 6.0),
                sequence: 2,
            },
        );
        assert_eq!(
            specs.is_empty(),
            cast.pattern == vocab::AccentPattern::None,
            "{id}"
        );
        assert!(specs.iter().all(flat), "{id}: accent");
        if let Some(recipe) = &profile.impact {
            let specs = impacts::impact_particles(
                recipe,
                &palette,
                &ImpactContext {
                    position: at,
                    ground: at.y,
                    direction: Vec2::Y,
                    heading: None,
                    area_damage: category::area_damage(key),
                    receipt: 3,
                    reserved: 0,
                },
            );
            assert!(!specs.is_empty() && specs.iter().all(flat), "{id}: impact");
        }
        rows += 1;
    }
    assert_eq!(rows, 68);
}

/// One body draws one effect: the `body` of the row for a kind of the first cast, its `aux`
/// entry for a secondary object, and nothing for a kind the skill does not replicate.
#[test]
fn a_replicated_effect_resolves_to_the_body_its_row_gives_that_kind() {
    use EffectVisualKind as K;
    let registry = target::target();
    let seen = |skill: SkillId, kind: K| registry.body_for(&effect(skill, kind));
    for (id, profile) in registry.rows() {
        let Some(skill) = SkillId::from_id(id) else {
            continue;
        };
        // Every kind of the first cast is drawn by `body`, every auxiliary kind by its own
        // `aux` entry, and no other kind by anything.
        for kind in category::own_kinds(skill) {
            assert_eq!(seen(skill, *kind), profile.body.as_ref(), "{id} {kind:?}");
        }
        for kind in category::aux_kinds(skill) {
            assert_eq!(
                seen(skill, *kind),
                profile.aux.get(category::kind_id(*kind)),
                "{id} {kind:?}"
            );
        }
        let foreign = [K::Cage, K::ShieldWall, K::Rocket]
            .into_iter()
            .find(|kind| {
                !category::own_kinds(skill).contains(kind)
                    && !category::aux_kinds(skill).contains(kind)
            });
        assert_eq!(seen(skill, foreign.unwrap()), None, "{id}");
    }
    // The wave and its warning are two bodies of one row.
    let wave = seen(SkillId::HorizonWave, K::Bolt).unwrap();
    let warning = seen(SkillId::HorizonWave, K::BeamWarning).unwrap();
    assert_eq!(
        (wave.archetype, warning.archetype),
        (vocab::Archetype::Traveller, vocab::Archetype::Lane)
    );
    // Both orders of the orb share its body.
    assert_eq!(
        seen(SkillId::OrbitalCommand, K::Orb),
        seen(SkillId::OrbitalGuard, K::Orb)
    );
    assert!(seen(SkillId::OrbitalCommand, K::Orb).is_some());
}

/// AC13 from data: every body of the final rows stays inside the part budget of its
/// cooldown at its largest boundary, and the rows together inside the material budget.
#[test]
fn target_bodies_fit_the_part_and_material_budgets() {
    use std::collections::BTreeSet;
    let registry = target::target();
    let mut long_cooldown = Vec::new();
    let mut bodies_seen = 0;
    for (id, profile) in registry.rows() {
        let Some(skill) = SkillId::from_id(id) else {
            assert!(profile.body.is_none() && profile.aux.is_empty(), "{id}");
            continue;
        };
        let def = shared::loadout::skill(skill);
        let most = if def.ability.base_cooldown_secs >= bodies::LONG_COOLDOWN_SECS {
            bodies::MAX_PARTS_LONG_COOLDOWN
        } else {
            bodies::MAX_PARTS
        };
        let own = profile.body.iter().flat_map(|body| {
            category::own_kinds(skill)
                .iter()
                .map(move |kind| (*kind, body))
        });
        let aux = profile.aux.iter().map(|(name, body)| {
            let kind = category::aux_kinds(skill)
                .iter()
                .find(|kind| category::kind_id(**kind) == name)
                .unwrap();
            (*kind, body)
        });
        for (kind, body) in own.chain(aux) {
            // The boundary at full length: a whole cone, a strip with both caps.
            let mut e = effect(skill, kind);
            e.radius = geometry::replicated_radius(skill, kind);
            e.end = [
                e.position[0] + def.ability.cast_range.max(1.0),
                e.position[1],
            ];
            let shape = geometry::boundary_shape(skill, kind, &e);
            let parts = bodies::part_total(body, &shape);
            assert!(
                (1..=most).contains(&parts),
                "{id} {kind:?}: {parts} of {most}"
            );
            if parts > bodies::MAX_PARTS {
                long_cooldown.push((id, parts));
            }
            bodies_seen += 1;
        }
    }
    assert_eq!(bodies_seen, 33);
    // Only skills on a long cooldown spend more than twelve parts.
    assert_eq!(
        long_cooldown,
        [
            ("dawn_ray", 16),
            ("dawn_ray", 16),
            ("iron_boundary", 16),
            ("orbital_collapse", 13),
            ("winter_divide", 15)
        ]
    );

    // Three materials for each skill colour and gain, two for each further fill strength
    // of a colour, one for each matter and each spark colour, nine of the engine.
    let bits = |color: [f32; 3]| color.map(f32::to_bits);
    let mut primaries = BTreeSet::new();
    let mut secondaries = BTreeSet::new();
    let mut accents = BTreeSet::new();
    let mut strengths = BTreeSet::new();
    for class in shared::HeroClass::ALL {
        let theme = registry.theme(class).unwrap();
        secondaries.insert(bits(theme.secondary));
        accents.insert(bits(theme.accent));
    }
    for (_, profile) in registry.rows() {
        primaries.insert((bits(profile.color), profile.hdr_gain.to_bits()));
        secondaries.extend(profile.secondary.map(bits));
        accents.extend(profile.accent.map(bits));
        strengths.extend(
            profile
                .body
                .iter()
                .chain(profile.aux.values())
                .filter_map(|body| body.fill_strength)
                .filter(|strength| *strength != bodies::FILL_STRENGTH)
                .map(|strength| (bits(profile.color), strength.to_bits())),
        );
    }
    assert_eq!(
        (
            primaries.len(),
            secondaries.len(),
            accents.len(),
            strengths.len()
        ),
        (65, 25, 24, 2)
    );
    let materials = 3 * primaries.len()
        + 2 * strengths.len()
        + secondaries.len()
        + accents.len()
        + bodies::SHARED_MATERIALS;
    assert_eq!(materials, 257);
    assert!(materials <= bodies::MATERIAL_BUDGET);
    assert_eq!(bodies::MATERIAL_BUDGET, 272);
    assert_eq!(bodies::PART_BUDGET, 400);
    assert_eq!(bodies::MAX_EFFECT_LIGHTS, 2);
}

/// The parser measures what a block draws; a burst over its budget is refused with the
/// number that broke it.
#[test]
fn output_validation_refuses_bursts_over_their_budget() {
    use crate::game_vfx::ParticleSpec;
    let spec = ParticleSpec {
        lifetime: 0.3,
        delay: 0.1,
        ..ParticleSpec::BASE
    };
    assert_eq!(schema::burst_budget(&[], 8, 0.5, "cast"), Ok(()));
    assert_eq!(
        schema::burst_budget(&vec![spec.clone(); 8], 8, 0.4, "cast"),
        Ok(())
    );
    assert_eq!(
        schema::burst_budget(&vec![spec.clone(); 9], 8, 0.5, "cast").unwrap_err(),
        "cast draws 9 particles (at most 8)"
    );
    assert_eq!(
        schema::burst_budget(std::slice::from_ref(&spec), 8, 0.39, "impact").unwrap_err(),
        "impact lasts 0.40 s (at most 0.39)"
    );
    for broken in [
        ParticleSpec {
            origin: Vec3::NAN,
            ..spec.clone()
        },
        ParticleSpec {
            velocity: Vec3::INFINITY,
            ..spec.clone()
        },
        ParticleSpec {
            size: 0.0,
            ..spec.clone()
        },
        ParticleSpec {
            lifetime: -1.0,
            ..spec
        },
    ] {
        assert_eq!(
            schema::burst_budget(&[broken], 8, 0.5, "cast.move").unwrap_err(),
            "cast.move draws a particle without a finite pose"
        );
    }
}

/// AC6: a status is not authorable. No row can name one of the engine's state visuals in
/// any of its blocks, no vocabulary list offers their IDs, and the recast marker is the
/// only state a row selects.
#[test]
fn a_row_cannot_name_a_state_visual() {
    use status::StateVisual;
    let pick_list = vocab::render_markdown();
    let packaged: serde_json::Value =
        serde_json::from_str(include_str!("../../assets/config/skills.skillfx")).unwrap();
    let refused = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut config = packaged.clone();
        edit(&mut config);
        SkillPresentation::parse(&config.to_string()).is_err()
    };
    // The unedited file parses, so every refusal below is caused by its edit.
    assert!(!refused(&|_| {}));
    for state in StateVisual::PRIORITY {
        let id = state.id();
        assert!(!pick_list.contains(&format!("`{id}`")), "{id}");
        // `winter_shard` has every block: an accent, a body and an impact.
        for (block, field) in [
            (None, "state"),
            (None, "status"),
            (Some("cast"), "state"),
            (Some("body"), "state"),
            (Some("impact"), "state"),
        ] {
            assert!(
                refused(&|config| {
                    let row = &mut config["skills"]["winter_shard"];
                    match block {
                        Some(block) => row[block][field] = id.into(),
                        None => row[field] = id.into(),
                    }
                }),
                "{id} as {block:?}.{field}"
            );
        }
        for (block, field) in [
            ("cast", "pattern"),
            ("cast", "shape"),
            ("cast", "recast_marker"),
            ("body", "marker"),
            ("impact", "kind"),
            ("impact", "shape"),
        ] {
            assert!(
                refused(&|config| {
                    config["skills"]["winter_shard"][block][field] = id.into();
                }),
                "{id} as {block}.{field}"
            );
        }
    }
    // The ten states are the ten of the data contract, in its priority order.
    assert_eq!(
        StateVisual::PRIORITY.map(StateVisual::id),
        [
            "stunned",
            "rooted",
            "parry_stance",
            "shielded",
            "marked",
            "brittle",
            "concussed",
            "slowed",
            "camouflage_veil",
            "forging",
        ]
    );
    // Six of the nine engine materials of the material rule belong to them.
    assert_eq!(status::StatePaint::ALL.len() + 3, bodies::SHARED_MATERIALS);
    assert_eq!(status::MAX_PARTS, 4);
}
