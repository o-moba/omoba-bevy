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
                    direction,
                    heading,
                    area_damage,
                    receipt: 4,
                };
                let specs = impacts::impact_particles(recipe, palette, &ctx);
                assert!(!specs.is_empty(), "{what}: impact");
                within(&specs, impacts::IMPACT_MAX, impacts::IMPACT_SECS, what);
                if let Some(count) = recipe.count {
                    assert_eq!(specs.len(), usize::from(count), "{what}: impact");
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
/// file and of the final rows stays inside its budget with finite poses in both backends.
#[test]
fn budget_from_data() {
    // The packaged rows gain their blocks class by class; whatever they have is held to
    // the same budgets.
    let shipped = profiles();
    let packaged = draw_registry(&shipped);
    assert_eq!(
        packaged.accents,
        shipped.rows().filter(|(_, row)| row.cast.is_some()).count()
            + shared::HeroClass::ALL
                .into_iter()
                .filter_map(|class| shipped.basic(class))
                .map(|basic| {
                    usize::from(basic.accent.is_some())
                        + usize::from(
                            basic
                                .rockets
                                .as_ref()
                                .is_some_and(|row| row.accent.is_some()),
                        )
                })
                .sum::<usize>()
    );
    // The final rows: 68 skill accents and 14 basic ones, 51 skill impacts and 18 basic
    // ones, and every modifier of the data contract.
    assert_eq!(
        draw_registry(&target::target()),
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
                                direction: Vec2::new(0.0, -1.0),
                                heading: Some(Vec2::X),
                                area_damage,
                                receipt: u64::from(count) * 13,
                            };
                            let specs = impacts::impact_particles(&recipe, &palette, &ctx);
                            let name =
                                format!("{} {} x{count} {scale} {lifetime}", kind.id(), shape.id());
                            assert_eq!(specs.len(), usize::from(count), "{name}");
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
                    direction: Vec2::Y,
                    heading: None,
                    area_damage: category::area_damage(key),
                    receipt: 3,
                },
            );
            assert!(!specs.is_empty() && specs.iter().all(flat), "{id}: impact");
        }
        rows += 1;
    }
    assert_eq!(rows, 68);
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
