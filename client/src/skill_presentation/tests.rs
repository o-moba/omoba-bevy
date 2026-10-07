use super::*;

pub(super) mod schema_rules;
pub(super) mod target;

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
        assert_eq!(a.effect, b.effect);
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

#[test]
fn ranged_basic_attacks_use_aimed_motion_and_dagger_keeps_the_right_hand_thrust() {
    use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};
    for (class, expected) in [
        (HeroClass::Ranger, "pistol_shoot"),
        (HeroClass::Wildspark, "pistol_shoot"),
        (HeroClass::Riftshot, "pistol_shoot"),
        (HeroClass::Mage, "cast"),
        (HeroClass::Dawnweaver, "cast"),
        (HeroClass::Adventurer, "dagger_stab"),
    ] {
        assert_eq!(
            motion_cue(&profiles(), class, None, BASIC_ATTACK_ACTION_SLOT, 1, &[])
                .unwrap()
                .motion,
            expected
        );
    }
}

#[test]
fn version_1_files_are_rejected_with_the_reason() {
    let v1 = include_str!("fixtures/v1.skillfx");
    assert_eq!(
        SkillPresentation::parse(v1).err().as_deref(),
        Some("Unsupported skill presentation schema_version 1 (expected 2)")
    );
    // Any other failure of the file keeps its own message.
    assert!(SkillPresentation::parse("{").is_err_and(|error| !error.contains("schema_version")));
}

#[test]
fn migration_preserves_v1_fields() {
    let v1: serde_json::Value = serde_json::from_str(include_str!("fixtures/v1.skillfx")).unwrap();
    let rows = v1["skills"].as_object().unwrap();
    let registry = profiles();
    assert_eq!(registry.rows().count(), rows.len());
    for (id, old) in rows {
        let new = registry
            .row(id)
            .unwrap_or_else(|| panic!("{id} was dropped"));
        assert_eq!(old["release"], new.release.as_str(), "{id}");
        assert_eq!(
            old.get("windup").and_then(|windup| windup.as_str()),
            new.windup.as_deref(),
            "{id}"
        );
        let effect: EffectStyle = serde_json::from_value(old["effect"].clone()).unwrap();
        assert_eq!(new.effect, Some(effect), "{id}");
        let color: [f32; 3] = serde_json::from_value(old["color"].clone()).unwrap();
        assert_eq!(new.color, color, "{id}");
        let hdr_gain: f32 = serde_json::from_value(old["hdr_gain"].clone()).unwrap();
        assert_eq!(new.hdr_gain, hdr_gain, "{id}");
        // The migration adds the home class and nothing that a consumer reads.
        assert!(
            !new.migrated()
                && new.body.is_none()
                && new.aux.is_empty()
                && new.impact.is_none()
                && new.sound.is_none()
                && new.secondary.is_none()
                && new.accent.is_none()
                && new.motion == schema::MotionPlayback::default(),
            "{id}"
        );
        assert_eq!(
            new.home,
            category::SkillKey::from_id(id).unwrap().home().id(),
            "{id}"
        );
    }
}

#[test]
fn the_packaged_registry_has_a_theme_for_every_class_and_no_basic_rows_yet() {
    let registry = profiles();
    for class in shared::HeroClass::ALL {
        assert!(registry.theme(class).is_some(), "{}", class.id());
        assert!(registry.basic(class).is_none(), "{}", class.id());
    }
    assert_eq!(registry.themes.len(), shared::HeroClass::ALL.len());
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
    assert!(LoadedPresentation::from_bytes(include_bytes!("fixtures/v1.skillfx")).is_err());

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
            body: ProjectileBody::Shape(crate::combat_visuals::ProjectileShape::Crescent, None),
            silhouette: None,
            presentation: vocab::ProjectilePresentation::Projectile,
        }
    );
    assert_eq!(identity("heroic_strike").motion, "slash_down");
    // Rows that are not migrated keep the look of their legacy style and the shared burst.
    let hook = identity("iron_hook");
    assert_eq!(hook.body, BodySig::Legacy(Some(EffectStyle::Hook)));
    assert_eq!(
        hook.body.key(),
        ("legacy".into(), "legacy:Some(Hook)".into())
    );
    assert_eq!(hook.impact, ImpactSig::Unthemed);
    assert_eq!(identity("guiding_lantern").impact, ImpactSig::None);
    assert!(signature::identity(&registry, &projectiles, "fireball").is_none());

    // Migrating twelve rows can only lower the counters that describe unmigrated rows.
    let before = signature::ratchet(&profiles(), &projectiles).unwrap();
    let after = signature::ratchet(&registry, &projectiles).unwrap();
    assert_eq!(after.rows_without_cast, before.rows_without_cast - 12);
    assert_eq!(after.rows_with_effect, before.rows_with_effect - 12);
    assert_eq!(
        after.rows_without_cast_voice,
        before.rows_without_cast_voice - 12
    );
    assert_eq!(
        after.classes_without_basic_row,
        before.classes_without_basic_row - 2
    );
    assert_eq!(after.duplicate_cast_voices, 0);
    assert!(after.pairs_under_two_axes < before.pairs_under_two_axes);
    assert!(after.rows_breaking_luminance <= before.rows_breaking_luminance);
}

/// The ratchet: the packaged registry may not move away from the final rules, and a
/// package that moves it closer lowers the checked-in counters.
#[test]
fn shipped_identity_ratchet() {
    let projectiles = crate::combat_visuals::CombatVisualRegistry::from_json(include_str!(
        "../../assets/config/combat_visuals.json"
    ))
    .unwrap();
    let counts = signature::ratchet(&profiles(), &projectiles).unwrap();
    for ((name, count), (_, ceiling)) in counts
        .entries()
        .into_iter()
        .zip(signature::SHIPPED_RATCHET.entries())
    {
        assert!(
            count <= ceiling,
            "{name} rose from {ceiling} to {count}: the packaged skills became less distinct"
        );
        assert!(
            count >= ceiling,
            "{name} fell from {ceiling} to {count}: lower it in SHIPPED_RATCHET"
        );
    }
}
