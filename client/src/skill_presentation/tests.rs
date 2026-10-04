use super::*;

fn profiles() -> SkillPresentation {
    SkillPresentation::parse(include_str!("../../assets/config/skills.skillfx")).unwrap()
}

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
        let cue = motion_cue(&profiles(), Some(&state), slot as u8, 7, &[]).unwrap();
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
    assert_eq!(equipped_skill(Some(&state), 0), Some(SkillId::DawnRay));
    let mut e = effect(SkillId::DawnRay, EffectVisualKind::BeamWarning);
    let cue = motion_cue(&profiles(), Some(&state), 0, 7, &[e.clone()]).unwrap();
    assert_eq!(cue.motion, "spell_prepare");
    assert!(cue.hold);
    e.kind = EffectVisualKind::Beam;
    let cue = motion_cue(&profiles(), Some(&state), 0, 7, &[e]).unwrap();
    assert_eq!(cue.motion, "cast");
    assert!(!cue.hold);
    // Cancellation, packet omission and fog do not manufacture a release.
    assert!(motion_cue(&profiles(), Some(&state), 0, 7, &[]).is_none());
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
    assert!(motion_cue(&profiles(), Some(&state), 3, 7, &[e.clone()]).is_none());
    e.owner_id = 8;
    assert!(motion_cue(&profiles(), Some(&state), 3, 7, &[e]).is_none());
}

#[test]
fn malformed_profile_cannot_introduce_gameplay_or_unknown_motion() {
    let valid = include_str!("../../assets/config/skills.skillfx");
    assert_eq!(profiles().skills.len(), shared::HeroClass::ALL.len() * 4);
    assert!(SkillPresentation::parse(&valid.replace("pistol_shoot", "missing_clip")).is_err());
    assert!(
        SkillPresentation::parse(&valid.replace(
            "\"release\":\"cast\"",
            "\"damage\":999,\"release\":\"cast\""
        ))
        .is_err()
    );
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
fn all_roster_abilities_have_profiles_and_recipes_override_class_and_slot() {
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
            .action_profile(shared::HeroClass::Warrior, Some(&state), slot)
            .unwrap();
        let b = registry
            .profile(state.recipe.as_ref().unwrap().skills[slot as usize])
            .unwrap();
        assert_eq!(a.effect, b.effect);
        assert_eq!(a.release, b.release);
    }
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
        motion_cue(&profiles(), Some(&state), 3, 7, &[warning])
            .unwrap()
            .hold
    );
    let bolt = effect(SkillId::HorizonWave, EffectVisualKind::Bolt);
    assert!(
        !motion_cue(&profiles(), Some(&state), 3, 7, &[bolt])
            .unwrap()
            .hold
    );
    assert!(motion_cue(&profiles(), Some(&state), 3, 7, &[]).is_none());
}
