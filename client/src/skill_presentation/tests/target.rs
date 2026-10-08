//! The final data of the roster as a fixture: 68 skill rows, 17 basic attacks and the
//! projectile profiles of the class designs, parsed and held to the identity rules before
//! any renderer draws them. Engine packages test their generators against these rows;
//! content packages move them into the packaged files.
use super::super::category::{self, Category, SkillKey};
use super::super::schema::{Body, CastAccent, ImpactRecipe, SoundCue};
use super::super::signature::{self, RatchetCounts};
use super::super::vocab::{
    AccentPattern, Altitude, Archetype, AudioBase, AudioSlice, Behaviour, ExpireKind, ImpactKind,
    Marker, Model, MotionPhase, MovePattern, PaletteSlot, ParticleShape, PreviewShape,
    ProjectileForm, ProjectilePresentation, RecastMarker, SatelliteLayout, Silhouette, StageRule,
    Trail,
};
use super::super::*;
use super::schema_rules;
use crate::combat_visuals::{CombatVisualRegistry, ProjectileShape};
use crate::humanoid::SharedHumanoidMotion;
use shared::combat::ProjectileStyle;
use shared::loadout::AttackProfileId;
use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};
use std::collections::{BTreeMap, BTreeSet};

const TARGET: &str = include_str!("../fixtures/target.skillfx");
const TARGET_VISUALS: &str = include_str!("../fixtures/target_combat_visuals.json");

/// The final `skills.skillfx`.
pub(in crate::skill_presentation) fn target() -> SkillPresentation {
    SkillPresentation::parse(TARGET).unwrap_or_else(|error| panic!("target.skillfx: {error}"))
}

/// The final `combat_visuals.json`.
pub(in crate::skill_presentation) fn target_visuals() -> CombatVisualRegistry {
    CombatVisualRegistry::from_json(TARGET_VISUALS)
        .unwrap_or_else(|error| panic!("target_combat_visuals.json: {error}"))
}

/// The classes whose rows the content packages have moved into the packaged files.
pub(in crate::skill_presentation) const PROMOTED: [HeroClass; 17] = [
    HeroClass::Warrior,
    HeroClass::Mage,
    HeroClass::Ranger,
    HeroClass::Cleric,
    HeroClass::Warden,
    HeroClass::Dawnweaver,
    HeroClass::Emberveil,
    HeroClass::Orbitwright,
    HeroClass::Veilstalker,
    HeroClass::Cinderforge,
    HeroClass::Edgeweaver,
    HeroClass::Stormfist,
    HeroClass::Adventurer,
    HeroClass::Wildspark,
    HeroClass::Riftshot,
    HeroClass::Chainkeeper,
    HeroClass::Frostguard,
];

fn shipped() -> SkillPresentation {
    SkillPresentation::parse(include_str!("../../../assets/config/skills.skillfx")).unwrap()
}

fn shipped_visuals() -> CombatVisualRegistry {
    CombatVisualRegistry::from_json(include_str!("../../../assets/config/combat_visuals.json"))
        .unwrap()
}

/// Every row with its key, in registry order.
fn rows(registry: &SkillPresentation) -> Vec<(&str, SkillKey, &SkillProfile)> {
    registry
        .rows()
        .map(|(id, profile)| (id, SkillKey::from_id(id).unwrap(), profile))
        .collect()
}

/// The profile a class throws for one action.
fn projectile(
    visuals: &CombatVisualRegistry,
    class: HeroClass,
    style: ProjectileStyle,
    slot: u8,
) -> &crate::combat_visuals::CombatVisualProfile {
    visuals.resolve(Some(class), style, Some(slot), None, None)
}

fn repeater(class: HeroClass) -> bool {
    shared::loadout::preset_for_class(class)
        .is_some_and(|kit| kit.attack_profile() == AttackProfileId::Repeater)
}

#[test]
fn target_parses() {
    // Both files are read by the loaders an overlay run goes through.
    assert!(TARGET.len() < schema::MAX_BYTES);
    assert_eq!(
        LoadedPresentation::from_bytes(TARGET.as_bytes())
            .unwrap()
            .registry
            .rows()
            .count(),
        68
    );
    let registry = target();
    let visuals = target_visuals();
    let packaged = shipped();

    // The target changes rows, not the roster: the same 68 skills and the packaged themes.
    assert_eq!(
        registry.rows().map(|(id, _)| id).collect::<Vec<_>>(),
        packaged.rows().map(|(id, _)| id).collect::<Vec<_>>()
    );
    assert_eq!(registry.themes, packaged.themes);
    assert_eq!(registry.basic_attacks.len(), HeroClass::ALL.len());
    for class in HeroClass::ALL {
        let basic = registry.basic(class).unwrap();
        assert_eq!(basic.rockets.is_some(), repeater(class), "{}", class.id());
        assert!(
            basic.impact.is_some() && basic.sound.is_some(),
            "{}",
            class.id()
        );
    }

    // Every row is final: drawn by its own blocks, nothing left to the legacy style.
    let rows = rows(&registry);
    let count = |pick: fn(&SkillProfile) -> bool| {
        rows.iter().filter(|(_, _, profile)| pick(profile)).count()
    };
    assert_eq!(count(|row| row.migrated() && row.effect.is_none()), 68);
    assert_eq!(count(|row| row.body.is_some()), 26);
    assert_eq!(count(|row| row.impact.is_some()), 51);
    assert_eq!(count(|row| row.windup.is_some()), 5);
    assert_eq!(count(|row| row.motion.fit_windup), 1);
    assert_eq!(count(|row| row.motion.recast.is_some()), 8);
    assert_eq!(
        rows.iter()
            .map(|(_, _, profile)| profile.aux.len())
            .sum::<usize>(),
        6
    );
    let phases = |phase: MotionPhase| {
        rows.iter()
            .filter(|(_, key, profile)| profile.phase(*key) == phase)
            .count()
    };
    assert_eq!(
        [
            MotionPhase::Instant,
            MotionPhase::WarnFire,
            MotionPhase::Fuse,
            MotionPhase::Parry
        ]
        .map(phases),
        [63, 2, 2, 1]
    );
    let cast = |pick: fn(&CastAccent) -> bool| {
        rows.iter()
            .filter(|(_, _, profile)| profile.cast.as_ref().is_some_and(pick))
            .count()
    };
    assert_eq!(cast(|cast| cast.movement.is_some()), 10);
    assert_eq!(cast(|cast| cast.link.is_some()), 14);
    assert_eq!(cast(|cast| cast.area), 3);
    assert_eq!(cast(|cast| cast.recast.is_some()), 7);
    assert_eq!(cast(|cast| cast.recast == Some(AccentPattern::None)), 5);
    assert_eq!(cast(|cast| cast.recast_marker.is_some()), 7);
    let voices = |pick: fn(&schema::Sound) -> bool| {
        rows.iter()
            .filter(|(_, _, profile)| profile.sound.as_ref().is_some_and(pick))
            .count()
    };
    assert_eq!(voices(|sound| sound.cast.is_some()), 68);
    assert_eq!(voices(|sound| sound.impact.is_some()), 51);
    assert_eq!(voices(|sound| sound.recast.is_some()), 8);
    assert_eq!(voices(|sound| sound.release.is_some()), 5);

    // The thirteen thrown legacy abilities have a form, four profiles hug the ground as a
    // wave, and the two melee basics throw nothing.
    let mut forms = 0;
    for (_, key, _) in &rows {
        if let SkillKey::Legacy(class, slot) = *key
            && category::category(*key) == Category::LegacyProjectile
        {
            let profile = projectile(
                &visuals,
                class,
                ProjectileStyle::for_class(class),
                slot.index() as u8,
            );
            assert!(
                profile.form.is_some() && profile.silhouette.is_some(),
                "{}",
                profile.id
            );
            forms += 1;
        }
    }
    assert_eq!(forms, 13);
    let waves: BTreeSet<&str> = HeroClass::ALL
        .into_iter()
        .flat_map(|class| {
            [0, 1, 2, 3, BASIC_ATTACK_ACTION_SLOT]
                .map(|slot| projectile(&visuals, class, ProjectileStyle::for_class(class), slot))
        })
        .filter(|profile| profile.presentation == ProjectilePresentation::Wave)
        .map(|profile| profile.id.as_str())
        .collect();
    assert_eq!(
        waves,
        BTreeSet::from(["chainkeeper_links", "heroic_strike", "primal_maul", "smite"])
    );
    for class in [HeroClass::Warrior, HeroClass::Warden] {
        let basic = projectile(
            &visuals,
            class,
            ProjectileStyle::for_class(class),
            BASIC_ATTACK_ACTION_SLOT,
        );
        assert_eq!(
            basic.presentation,
            ProjectilePresentation::MeleeContact,
            "{}",
            basic.id
        );
    }
    // Hits of hidden or unresolved sources keep the packaged arcane and holy looks.
    let packaged_visuals = shipped_visuals();
    for style in [ProjectileStyle::Arcane, ProjectileStyle::Holy] {
        let (now, before) = (
            visuals.resolve_style(style),
            packaged_visuals.resolve_style(style),
        );
        assert_eq!(
            (&now.id, now.shape, now.color, now.scale, now.form),
            (&before.id, before.shape, before.color, before.scale, None)
        );
    }
}

/// A promoted class ships the final data: its theme, its basic row, its four skill rows and
/// the projectile profiles its actions resolve to are the target's, value for value. A
/// content package tunes a promoted row in both files; the rest of the roster still waits.
#[test]
fn shipped_rows_equal_target_for_promoted_classes() {
    let (registry, packaged) = (target(), shipped());
    let (visuals, packaged_visuals) = (target_visuals(), shipped_visuals());
    // Parsed rows are compared as the client holds them, whatever the files spell.
    let own = |registry: &SkillPresentation, class: HeroClass| -> Vec<(String, String)> {
        rows(registry)
            .into_iter()
            .filter(|(_, key, _)| key.home() == class)
            .map(|(id, _, row)| (id.to_string(), format!("{row:?}")))
            .collect()
    };
    for class in PROMOTED {
        let name = class.id();
        assert_eq!(packaged.theme(class), registry.theme(class), "{name}");
        assert!(packaged.basic(class).is_some(), "{name}");
        assert_eq!(packaged.basic(class), registry.basic(class), "{name}");
        let kit = own(&packaged, class);
        assert_eq!(kit.len(), 4, "{name}");
        for (shipped, target) in kit.iter().zip(own(&registry, class)) {
            assert_eq!(*shipped, target, "{name}");
        }
        // Every body the class throws, and the look of a hit or a projectile of its wire
        // style whose owner the client cannot resolve.
        let mut styles = vec![ProjectileStyle::for_class(class)];
        if repeater(class) {
            styles = vec![ProjectileStyle::Bullet, ProjectileStyle::Rocket];
        }
        for style in styles {
            for slot in [0, 1, 2, 3, BASIC_ATTACK_ACTION_SLOT] {
                assert_eq!(
                    format!("{:?}", projectile(&packaged_visuals, class, style, slot)),
                    format!("{:?}", projectile(&visuals, class, style, slot)),
                    "{name} {style:?} {slot}"
                );
            }
            assert_eq!(
                format!("{:?}", packaged_visuals.resolve_style(style)),
                format!("{:?}", visuals.resolve_style(style)),
                "{name} {style:?}"
            );
        }
    }
    // Rows are promoted by class and whole: a row is final exactly when its class is.
    for (id, key, row) in rows(&packaged) {
        let promoted = PROMOTED.contains(&key.home());
        assert_eq!(row.migrated(), promoted, "{id}");
        assert_eq!(row.effect.is_none(), promoted, "{id}");
    }
    for class in HeroClass::ALL {
        assert_eq!(
            packaged.basic(class).is_some(),
            PROMOTED.contains(&class),
            "{}",
            class.id()
        );
    }
}

/// U1 to U6 and the migration counters: nothing is left to do on the target.
#[test]
fn target_identity_is_unique() {
    let registry = target();
    let visuals = target_visuals();
    let found = signature::findings(&registry, &visuals).unwrap();
    assert!(
        found.is_empty(),
        "the target breaks the identity rules:\n{}",
        found
            .iter()
            .map(|(counter, rows)| format!("  {counter}: {rows}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(
        signature::ratchet(&registry, &visuals).unwrap(),
        RatchetCounts::default()
    );

    // Look gate G1 failed, so `aim_loose_r` counts as `cast_thrust_r`. The merged family
    // stays at the cap because Piercing Arrow took its stated fallback.
    let family = |id: &str| signature::identity(&registry, &visuals, id).unwrap().motion;
    assert_eq!(family("piercing_arrow"), "sky_shot");
    let thrusts: Vec<&str> = registry
        .rows()
        .map(|(id, _)| id)
        .filter(|id| family(id) == "cast_thrust_r")
        .collect();
    assert_eq!(thrusts, ["arc_bolt", "dawn_bind", "orbital_guard"]);
    // The clip the Ranger design named first would be the fourth release of that family.
    let mut first_choice: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    first_choice["skills"]["piercing_arrow"]["release"] = "aim_loose_r".into();
    first_choice["skills"]["piercing_arrow"]["motion"] =
        serde_json::json!({ "rate": 0.8, "start": 0.1 });
    let first_choice = schema_rules::parse(&first_choice).unwrap();
    assert_eq!(
        signature::findings(&first_choice, &visuals).unwrap(),
        [(
            "families_over_three_skills",
            r#""cast_thrust_r" has 4: arc_bolt, dawn_bind, orbital_guard, piercing_arrow"#
                .to_string()
        )]
    );
    // The comet of stars that the Mage and the Cleric designs both chose for their basic
    // attack would be one body twice, so the Mage keeps its tumbling star.
    let mut same_spark: serde_json::Value = serde_json::from_str(TARGET_VISUALS).unwrap();
    same_spark["profiles"]["mage_basic"]["form"] = "comet".into();
    let same_spark = CombatVisualRegistry::from_json(&same_spark.to_string()).unwrap();
    assert_eq!(
        signature::ratchet(&registry, &same_spark).unwrap(),
        RatchetCounts {
            basic_projectile_key_duplicates: 1,
            ..RatchetCounts::default()
        }
    );

    // Basic attacks are outside the 68-skill signature. Ten of them throw a body (the
    // repeater throws two) and the ratchet keeps those apart; the eight that throw nothing
    // are told apart by what they draw at the hand and at the hit.
    let silent: Vec<HeroClass> = HeroClass::ALL
        .into_iter()
        .filter(|class| {
            match shared::loadout::preset_for_class(*class).map(|kit| kit.attack_profile()) {
                Some(AttackProfileId::Melee) => true,
                Some(_) => false,
                None => {
                    projectile(
                        &visuals,
                        *class,
                        ProjectileStyle::for_class(*class),
                        BASIC_ATTACK_ACTION_SLOT,
                    )
                    .presentation
                        == ProjectilePresentation::MeleeContact
                }
            }
        })
        .collect();
    assert_eq!(silent.len(), 8);
    let mut accents = BTreeSet::new();
    let mut impacts = BTreeSet::new();
    for class in &silent {
        let basic = registry.basic(*class).unwrap();
        let accent = basic.accent.as_ref().unwrap();
        let impact = basic.impact.as_ref().unwrap();
        assert!(
            accents.insert((accent.pattern, accent.lead())),
            "{}: accent",
            class.id()
        );
        assert!(
            impacts.insert((impact.kind, impact.lead())),
            "{}: impact",
            class.id()
        );
    }
}

/// The bodies the projectile renderer draws for the basic attacks: every thrown one is its
/// own, and only the two melee contacts share the engine's reach streak.
#[test]
fn basic_projectile_bodies_are_unique() {
    use crate::combat_visuals::{FlightBody, form_lateral_extent, known_basic};
    let visuals = target_visuals();
    let mut thrown = Vec::new();
    let mut contacts = Vec::new();
    for class in HeroClass::ALL {
        let styles = match shared::loadout::preset_for_class(class).map(|kit| kit.attack_profile())
        {
            // A melee core resolves contact at once and throws nothing.
            Some(AttackProfileId::Melee) => vec![],
            Some(AttackProfileId::Repeater) => {
                vec![ProjectileStyle::Bullet, ProjectileStyle::Rocket]
            }
            Some(AttackProfileId::LightBolt) | None => vec![ProjectileStyle::for_class(class)],
        };
        for style in styles {
            let profile = projectile(&visuals, class, style, BASIC_ATTACK_ACTION_SLOT);
            assert!(known_basic(Some(class), Some(BASIC_ATTACK_ACTION_SLOT)));
            match profile.flight_body(true, true) {
                FlightBody::Reach => contacts.push(class),
                // A `shape` body is told apart by its shape.
                body => thrown.push((
                    class,
                    style,
                    body,
                    (body == FlightBody::Shape).then_some(profile.shape),
                )),
            }
        }
    }
    assert_eq!(contacts, [HeroClass::Warrior, HeroClass::Warden]);
    assert_eq!(thrown.len(), 10);
    for (index, a) in thrown.iter().enumerate() {
        for b in &thrown[index + 1..] {
            assert_ne!((a.2, a.3), (b.2, b.3), "{a:?} and {b:?}");
            // Different entries of the form table are different meshes in different
            // places: no two of these bodies are drawn from the same parts.
            if a.2 != FlightBody::Shape && b.2 != FlightBody::Shape {
                assert_ne!(a.2.parts(), b.2.parts(), "{a:?} and {b:?}");
            }
        }
    }
    // The launcher round is its form, not the rocket model; the Ranger keeps the arrow.
    let body_of = |class: HeroClass, style: ProjectileStyle| {
        thrown
            .iter()
            .find(|entry| entry.0 == class && entry.1 == style)
            .map(|entry| (entry.2, entry.3))
            .unwrap()
    };
    assert_eq!(
        body_of(HeroClass::Wildspark, ProjectileStyle::Rocket),
        (
            FlightBody::Form(ProjectileForm::Tumbler, Silhouette::Block),
            None
        )
    );
    assert_eq!(
        body_of(HeroClass::Ranger, ProjectileStyle::Arrow),
        (FlightBody::Shape, Some(ProjectileShape::Arrow))
    );
    // The signature of a thrown body is the form and the mesh that are drawn.
    let registry = target();
    for (id, _, _) in rows(&registry) {
        let signature::BodySig::Projectile {
            body, silhouette, ..
        } = signature::identity(&registry, &visuals, id).unwrap().body
        else {
            continue;
        };
        let SkillKey::Legacy(class, slot) = SkillKey::from_id(id).unwrap() else {
            panic!("{id} is not a legacy ability");
        };
        let profile = projectile(
            &visuals,
            class,
            ProjectileStyle::for_class(class),
            slot.index() as u8,
        );
        let FlightBody::Form(form, mesh) = profile.flight_body(true, false) else {
            panic!("{id} is not drawn as a form");
        };
        assert_eq!(
            (body, silhouette),
            (signature::ProjectileBody::Form(form), Some(mesh)),
            "{id}"
        );
        assert!(form_lateral_extent(form, mesh) > 0.0);
    }
    // A profile that leaves the silhouette to its form has the identity of what is drawn:
    // the wavefront of Heroic Strike is a crescent with or without the name.
    let mut unnamed: serde_json::Value = serde_json::from_str(TARGET_VISUALS).unwrap();
    let strike = unnamed["profiles"]["heroic_strike"]
        .as_object_mut()
        .unwrap();
    assert_eq!(strike.remove("silhouette"), Some("crescent".into()));
    let unnamed = CombatVisualRegistry::from_json(&unnamed.to_string()).unwrap();
    assert_eq!(
        signature::identity(&registry, &unnamed, "heroic_strike"),
        signature::identity(&registry, &visuals, "heroic_strike")
    );
}

/// The helper that explains a failing ratchet counts exactly what the ratchet counts.
#[test]
fn findings_name_what_the_ratchet_counts() {
    let samples = schema_rules::parse(&schema_rules::samples()).unwrap();
    for (name, registry, visuals) in [
        ("packaged", shipped(), shipped_visuals()),
        (
            "unmigrated",
            SkillPresentation::unmigrated(),
            shipped_visuals(),
        ),
        ("samples", samples, shipped_visuals()),
        (
            "target on packaged projectiles",
            target(),
            shipped_visuals(),
        ),
        (
            "packaged on target projectiles",
            shipped(),
            target_visuals(),
        ),
        ("target", target(), target_visuals()),
    ] {
        let counts = signature::ratchet(&registry, &visuals).unwrap();
        let found = signature::findings(&registry, &visuals).unwrap();
        for (counter, count) in counts.entries() {
            assert_eq!(
                found.iter().filter(|(of, _)| *of == counter).count(),
                count,
                "{name}: {counter}"
            );
        }
        assert_eq!(
            found.len(),
            counts
                .entries()
                .iter()
                .map(|(_, count)| count)
                .sum::<usize>(),
            "{name}: a finding without a counter"
        );
    }
    // A finding names the rows, not just a number. The packaged file has none left, so
    // the rows it had before the content packages are asked.
    let found = signature::findings(&SkillPresentation::unmigrated(), &shipped_visuals()).unwrap();
    assert!(found.iter().any(|(counter, rows)| {
        *counter == "full_tuple_duplicates" && rows.contains(" / ") && rows.contains("share")
    }));
}

/// The clips a final row plays, as a release, a windup, a recast or a basic attack.
/// `attack` and `pistol_aim` stay in the library without a user; `interact`,
/// `pistol_reload` and `roll` are retired with the legacy paths.
const REQUIRED_MOTIONS: [&str; 49] = [
    "cast",
    "spell_prepare",
    "spell_finish",
    "pistol_shoot",
    "punch",
    "guard",
    "shoulder_drive",
    "dagger_stab",
    "dagger_feint",
    "dagger_backstab",
    "dagger_heavy_thrust",
    "slash_down",
    "slash_rising",
    "cleave_slam",
    "spin_cleave",
    "slash_down_m",
    "slash_rising_m",
    "blade_flourish",
    "blade_ready_loop",
    "thrust_lunge",
    "jab_cross",
    "fist_guard_loop",
    "ground_pound",
    "flying_knee",
    "leap_land",
    "dive_lunge",
    "vault_flip",
    "backflip_retreat",
    "dance_twirl",
    "overhead_plant",
    "rally_raise",
    "raise_from_earth",
    "two_hand_push",
    "shot_heavy",
    "burst_fire",
    "sky_shot",
    "ground_shot",
    "reload_snap",
    "aim_hold_loop",
    "aim_loose_r",
    "cast_thrust_r",
    "point_command",
    "hover_pulse",
    "levitate_loop",
    "draw_in",
    "hurl_overhand",
    "toss_underhand",
    "place_quick",
    "kneel_plant",
];

/// The vocabulary IDs the target draws, by pick list.
#[derive(Default)]
struct Used<'a> {
    ids: BTreeMap<&'static str, BTreeSet<&'a str>>,
    flat_shapes: Vec<ProjectileShape>,
    speed_steps: BTreeSet<u32>,
}

impl<'a> Used<'a> {
    fn add(&mut self, list: &'static str, id: &'a str) {
        self.ids.entry(list).or_default().insert(id);
    }

    fn part(&mut self, mesh: Silhouette, slot: PaletteSlot, behave: Behaviour) {
        self.add("silhouette", mesh.id());
        self.add("palette slot", slot.id());
        self.add("behaviour", behave.id());
    }

    fn body(&mut self, body: &Body) {
        self.add("archetype", body.archetype.id());
        for part in [&body.core, &body.shell].into_iter().flatten() {
            self.part(part.mesh, part.slot, part.behave);
        }
        if let Some(satellites) = &body.satellites {
            self.part(satellites.mesh, satellites.slot, satellites.behave);
            self.add("satellite layout", satellites.layout.id());
        }
        if let Some(model) = body.model {
            self.add("model", model.id());
        }
        if let Some(altitude) = body.altitude {
            self.add("altitude", altitude.id());
        }
        self.add("trail", body.trail.id());
        self.add("marker", body.marker.id());
        self.add("expire", body.expire.id());
    }

    /// `[primary, accent]` is what a recipe without `slots` draws.
    fn slots(&mut self, slots: Option<[PaletteSlot; 2]>) {
        for slot in slots.unwrap_or([PaletteSlot::Primary, PaletteSlot::Accent]) {
            self.add("palette slot", slot.id());
        }
    }

    fn accent(&mut self, accent: &CastAccent) {
        self.add("accent pattern", accent.pattern.id());
        self.slots(accent.slots);
        // A recast accent draws its own pattern's default lead.
        let recast_lead = accent.recast.and_then(AccentPattern::default_lead);
        let moved = accent.movement.as_ref().and_then(|movement| movement.shape);
        for shape in [accent.lead(), recast_lead, moved, accent.link]
            .into_iter()
            .flatten()
        {
            self.add("particle shape", shape.id());
        }
        if let Some(pattern) = accent.recast {
            self.add("accent pattern", pattern.id());
        }
        if let Some(movement) = &accent.movement {
            self.add("move pattern", movement.pattern.id());
        }
        if let Some(marker) = accent.recast_marker {
            self.add("recast marker", marker.id());
        }
    }

    fn impact(&mut self, impact: &ImpactRecipe) {
        self.add("impact kind", impact.kind.id());
        self.add("particle shape", impact.lead().id());
        self.slots(impact.slots);
    }

    fn cue(&mut self, cue: &SoundCue) {
        self.add("audio base", cue.base.id());
        self.add("audio slice", cue.slice.id());
        self.speed_steps.insert((cue.speed * 20.0).round() as u32);
    }

    fn basic(&mut self, basic: &'a BasicProfile) {
        for clip in &basic.motions {
            self.add("motion", clip);
        }
        if let Some(accent) = &basic.accent {
            self.accent(accent);
        }
        if let Some(impact) = &basic.impact {
            self.impact(impact);
        }
        if let Some(cue) = &basic.sound {
            self.cue(cue);
        }
        if let Some(rockets) = &basic.rockets {
            self.basic(rockets);
        }
    }

    fn projectile(&mut self, profile: &crate::combat_visuals::CombatVisualProfile) {
        if let Some(form) = profile.form {
            self.add("projectile form", form.id());
        }
        if let Some(silhouette) = profile.silhouette {
            self.add("silhouette", silhouette.id());
        }
        self.add("projectile presentation", profile.presentation.id());
        self.flat_shapes.push(profile.shape);
    }
}

fn ids<T: Copy>(all: &[T], id: fn(T) -> &'static str) -> BTreeSet<&'static str> {
    all.iter().map(|item| id(*item)).collect()
}

/// Every ID of the pick list (`docs/skill-vocabulary.md`) that a row can name or that is
/// derived for a body or for the aim preview of a skill has a final row that uses it, and no
/// final row plays a clip outside the required set.
#[test]
fn target_uses_every_required_vocabulary_id() {
    let registry = target();
    let visuals = target_visuals();
    let mut used = Used::default();

    for (_, key, profile) in rows(&registry) {
        used.add("motion", &profile.release);
        for clip in [&profile.windup, &profile.motion.recast]
            .into_iter()
            .flatten()
        {
            used.add("motion", clip);
        }
        used.add("motion phase", profile.phase(key).id());
        if let Some(cast) = &profile.cast {
            used.accent(cast);
        }
        if let Some(impact) = &profile.impact {
            used.impact(impact);
        }
        if let Some(sound) = &profile.sound {
            for cue in [&sound.cast, &sound.release, &sound.impact, &sound.recast]
                .into_iter()
                .flatten()
            {
                used.cue(cue);
            }
        }
        let Some(id) = key.modular() else { continue };
        // The preview of the first cast and of the press a recast window offers.
        used.add("preview shape", geometry::preview_kind(id, false).id());
        if category::has_recast(id) {
            used.add("preview shape", geometry::preview_kind(id, true).id());
        }
        if let Some(body) = &profile.body {
            used.body(body);
            for kind in category::own_kinds(id) {
                used.add("stage rule", category::stage_rule(id, *kind).id());
            }
        }
        for (name, body) in &profile.aux {
            used.body(body);
            let kind = category::aux_kinds(id)
                .iter()
                .find(|kind| category::kind_id(**kind) == name)
                .unwrap();
            used.add("stage rule", category::stage_rule(id, *kind).id());
        }
    }
    for class in HeroClass::ALL {
        used.basic(registry.basic(class).unwrap());
        // What the class throws: its four abilities and its basic attack, in both modes of
        // the repeater.
        let style = ProjectileStyle::for_class(class);
        for slot in [0, 1, 2, 3, BASIC_ATTACK_ACTION_SLOT] {
            used.projectile(projectile(&visuals, class, style, slot));
        }
        if repeater(class) {
            for style in [ProjectileStyle::Bullet, ProjectileStyle::Rocket] {
                used.projectile(projectile(&visuals, class, style, BASIC_ATTACK_ACTION_SLOT));
            }
        }
    }

    let required: [(&str, BTreeSet<&str>); 23] = [
        ("motion", REQUIRED_MOTIONS.into_iter().collect()),
        ("motion phase", ids(MotionPhase::ALL, MotionPhase::id)),
        ("archetype", ids(Archetype::ALL, Archetype::id)),
        ("silhouette", ids(Silhouette::ALL, Silhouette::id)),
        ("model", ids(Model::ALL, Model::id)),
        (
            "satellite layout",
            ids(SatelliteLayout::ALL, SatelliteLayout::id),
        ),
        ("trail", ids(Trail::ALL, Trail::id)),
        ("marker", ids(Marker::ALL, Marker::id)),
        ("behaviour", ids(Behaviour::ALL, Behaviour::id)),
        ("altitude", ids(Altitude::ALL, Altitude::id)),
        ("expire", ids(ExpireKind::ALL, ExpireKind::id)),
        ("stage rule", ids(StageRule::ALL, StageRule::id)),
        ("preview shape", ids(PreviewShape::ALL, PreviewShape::id)),
        ("accent pattern", ids(AccentPattern::ALL, AccentPattern::id)),
        ("move pattern", ids(MovePattern::ALL, MovePattern::id)),
        ("recast marker", ids(RecastMarker::ALL, RecastMarker::id)),
        ("impact kind", ids(ImpactKind::ALL, ImpactKind::id)),
        ("particle shape", ids(ParticleShape::ALL, ParticleShape::id)),
        ("palette slot", ids(PaletteSlot::ALL, PaletteSlot::id)),
        ("audio base", ids(AudioBase::ALL, AudioBase::id)),
        ("audio slice", ids(AudioSlice::ALL, AudioSlice::id)),
        (
            "projectile form",
            ids(ProjectileForm::ALL, ProjectileForm::id),
        ),
        (
            "projectile presentation",
            ids(ProjectilePresentation::ALL, ProjectilePresentation::id),
        ),
    ];
    let mut gaps = Vec::new();
    for (list, required) in required {
        let used = used.ids.remove(list).unwrap_or_default();
        let unused: Vec<_> = required.difference(&used).collect();
        let unlisted: Vec<_> = used.difference(&required).collect();
        if !unused.is_empty() {
            gaps.push(format!("{list}: no target row uses {unused:?}"));
        }
        if !unlisted.is_empty() {
            gaps.push(format!("{list}: outside the required set: {unlisted:?}"));
        }
    }
    assert!(gaps.is_empty(), "{}", gaps.join("\n"));
    assert!(
        used.ids.is_empty(),
        "{:?} has no required set",
        used.ids.keys()
    );

    // The clips are the library minus the base states and the five without a final user.
    let library = SharedHumanoidMotion::embedded().unwrap();
    let spare: BTreeSet<&str> = library
        .clips
        .keys()
        .map(String::as_str)
        .filter(|clip| !REQUIRED_MOTIONS.contains(clip))
        .collect();
    assert_eq!(
        spare,
        BTreeSet::from([
            "attack",
            "death",
            "idle",
            "interact",
            "pistol_aim",
            "pistol_reload",
            "roll",
            "run",
            "walk"
        ])
    );
    // The five 2D shapes stay in use, and the voices spread over the whole speed grid.
    for shape in [
        ProjectileShape::Arrow,
        ProjectileShape::Arcane,
        ProjectileShape::Holy,
        ProjectileShape::Crescent,
        ProjectileShape::Bolt,
    ] {
        assert!(used.flat_shapes.contains(&shape), "{shape:?}");
    }
    assert_eq!(used.speed_steps, (14..=28).collect::<BTreeSet<u32>>());
}

/// Seconds from an accepted edge to the contact pose of a clip entered at `start`.
fn contact_delay(library: &SharedHumanoidMotion, clip: &str, rate: f32, start: f32) -> f32 {
    let contact = library
        .contact(clip)
        .unwrap_or_else(|| panic!("{clip} has no contact"));
    (contact - start * library.clips[clip].duration) / rate
}

/// `(contact - start * duration) / rate <= 0.15` for every clip that starts on an accepted
/// edge, computed here from the exported contacts and not through the parser.
#[test]
fn target_contact_rule_holds() {
    let registry = target();
    let library = SharedHumanoidMotion::embedded().unwrap();
    // (user, clip, rate, start)
    let mut edges: Vec<(String, &str, f32, f32)> = Vec::new();
    for (id, key, profile) in rows(&registry) {
        let motion = &profile.motion;
        if profile.phase(key) == MotionPhase::Instant {
            edges.push((id.into(), &profile.release, motion.rate, motion.start));
        } else {
            // A row that waits for its telegraph holds a loop or a clip fitted to it.
            let windup = profile.windup.as_deref().unwrap();
            assert!(
                library.clips[windup].looping || motion.fit_windup,
                "{id}: {windup}"
            );
        }
        if let Some(recast) = &motion.recast {
            edges.push((format!("{id}:recast"), recast, motion.recast_rate, 0.0));
        }
    }
    for class in HeroClass::ALL {
        let basic = registry.basic(class).unwrap();
        for (mode, row) in [("", Some(basic)), (":rockets", basic.rockets.as_deref())] {
            let Some(row) = row else { continue };
            for clip in &row.motions {
                edges.push((
                    format!("basic:{}{mode}", class.id()),
                    clip,
                    row.rate,
                    row.start,
                ));
            }
        }
    }
    // 63 instant releases, 8 recast clips, 22 basic-attack clips.
    assert_eq!(edges.len(), 93);

    let mut past_contact = Vec::new();
    for (user, clip, rate, start) in &edges {
        let delay = contact_delay(library, clip, *rate, *start);
        assert!(
            delay <= schema::CONTACT_LIMIT_SECS + 1e-4,
            "{user}: {clip} reaches its contact {delay:.3} s after the edge"
        );
        if delay < -1e-4 {
            past_contact.push(user.as_str());
        }
    }
    // A clip entered behind its contact key never shows that pose. Quick Shot does it on
    // purpose (it enters `burst_fire` at the second of three recoil beats); Rift Seal
    // enters `sky_shot` 0.03 s behind the kick with the values of its design. Backstab is
    // not in the list: its clip reaches the contact in 0.11 s and is entered at its start.
    assert_eq!(past_contact, ["quick_shot", "rift_seal"]);

    // The Cinderforge basic enters `cleave_slam` at the largest 0.05 step that is not
    // behind the blow.
    let smith = registry.basic(HeroClass::Cinderforge).unwrap();
    let blow = library.contact("cleave_slam").unwrap() / library.clips["cleave_slam"].duration;
    assert_eq!(smith.motions, ["cleave_slam"]);
    assert!(smith.start <= blow && blow < smith.start + 0.05, "{blow}");
}

/// The required and forbidden matrix over the final rows: a block is present exactly where
/// the catalog gives it something to show, and every optional pick names a fact that holds.
#[test]
fn authored_picks_agree_with_the_catalog() {
    let registry = target();
    for (id, key, profile) in rows(&registry) {
        let skill = key.modular();
        let fact = |test: fn(SkillId) -> bool| skill.is_some_and(test);
        let damaging = category::can_damage(key);
        let replicated = category::category(key) == Category::ReplicatedEffect;
        let cast = profile.cast.as_ref().unwrap();
        let sound = profile.sound.as_ref().unwrap();
        let cues = [&sound.cast, &sound.release, &sound.impact, &sound.recast];

        // Required blocks.
        assert_eq!(profile.impact.is_some(), damaging, "{id}: impact");
        assert_eq!(sound.impact.is_some(), damaging, "{id}: sound.impact");
        assert_eq!(profile.body.is_some(), replicated, "{id}: body");
        let declared: BTreeSet<&str> = profile.aux.keys().map(String::as_str).collect();
        let tagged: BTreeSet<&str> = skill
            .map(category::aux_kinds)
            .unwrap_or_default()
            .iter()
            .map(|kind| category::kind_id(*kind))
            .collect();
        assert_eq!(declared, tagged, "{id}: aux");

        // Motion: a held pose exactly for the skills with a telegraph of their own, and a
        // second clip exactly for the skills with a recast.
        let phase = skill.map_or(MotionPhase::Instant, category::derived_phase);
        assert_eq!(profile.phase(key), phase, "{id}: phase");
        assert_eq!(
            profile.windup.is_some(),
            phase != MotionPhase::Instant,
            "{id}: windup"
        );
        assert!(
            !profile.motion.fit_windup || skill.and_then(category::telegraph_secs).is_some(),
            "{id}: fit_windup"
        );
        let recast = fact(category::has_recast);
        assert_eq!(
            profile.motion.recast.is_some(),
            recast,
            "{id}: motion.recast"
        );
        assert_eq!(sound.recast.is_some(), recast, "{id}: sound.recast");
        assert_eq!(
            sound.release.is_some(),
            phase != MotionPhase::Instant,
            "{id}: sound.release"
        );

        // Cast: every modifier names a fact.
        assert!(
            (cast.recast.is_none() && cast.recast_marker.is_none()) || recast,
            "{id}: recast accent"
        );
        let moves = skill.is_some_and(|skill| {
            category::movement_capable(skill, false) || category::movement_capable(skill, true)
        });
        assert_eq!(cast.movement.is_some(), moves, "{id}: cast.move");
        assert!(
            cast.link.is_none()
                || (damaging
                    && (!category::travelling_body(key) || fact(category::recast_instant_hit))),
            "{id}: cast.link"
        );
        assert_eq!(
            cast.area,
            skill.is_some_and(|skill| category::AREA_FLASH_SIGNED_OFF.contains(&skill)),
            "{id}: cast.area"
        );
        assert_eq!(
            cast.pattern == AccentPattern::StrikeLine,
            fact(category::own_effect_strike),
            "{id}: strike_line"
        );
        assert!(
            profile.windup.is_none() || cast.pattern.is_charge(),
            "{id}: charge pattern"
        );
        assert!(
            !category::self_heal(key)
                || (cast.pattern != AccentPattern::ShieldFlash
                    && cast.lead() != Some(ParticleShape::Kite)),
            "{id}: shield shape on a heal"
        );

        // Impact and voice.
        if let Some(impact) = &profile.impact {
            assert!(
                impact.kind != ImpactKind::PierceThrough || category::pierces(key),
                "{id}: pierce_through"
            );
            assert!(
                impact.kind != ImpactKind::Blast || category::area_damage(key),
                "{id}: blast"
            );
        }
        assert_eq!(
            cues.iter()
                .any(|cue| cue.as_ref().is_some_and(|cue| cue.base == AudioBase::Bluff)),
            id == "dagger_bluff",
            "{id}: bluff"
        );

        // Bodies: an archetype that can show the replicated geometry, an ending the client
        // can classify, growth only where the spawn time is known.
        let Some(skill) = skill else { continue };
        let own = profile.body.iter().map(|body| (body, None));
        let aux = profile.aux.iter().map(|(name, body)| {
            let kind = category::aux_kinds(skill)
                .iter()
                .find(|kind| category::kind_id(**kind) == name);
            (body, kind.copied())
        });
        for (body, kind) in own.chain(aux) {
            let fits = match kind {
                None => geometry::body_fits(body.archetype, skill),
                Some(kind) => geometry::archetype_fits(body.archetype, skill, kind),
            };
            assert!(fits, "{id}: {}", body.archetype.id());
            assert_eq!(
                body.expire == ExpireKind::Detonate,
                kind.is_none() && category::detonates(skill),
                "{id}: detonate"
            );
            let rises = [&body.core, &body.shell]
                .into_iter()
                .flatten()
                .map(|part| part.behave)
                .chain(body.satellites.as_ref().map(|part| part.behave))
                .any(|behave| behave == Behaviour::RiseOnSpawn);
            assert!(
                !rises || category::spawn_lifetime_secs(skill).is_some(),
                "{id}: rise_on_spawn"
            );
        }
    }
}

/// The secondary objects of a skill: the orb is one object that two skills order, so both
/// rows carry the same body, and each other object has the body its design gives it.
#[test]
fn target_aux_bodies_agree() {
    let registry = target();
    let aux = |id: &str, kind: &str| {
        registry
            .row(id)
            .unwrap()
            .aux
            .get(kind)
            .unwrap_or_else(|| panic!("{id} has no aux.{kind}"))
    };
    let lead = |body: &Body| body.core.as_ref().map(|part| (part.mesh, part.behave));
    let ring = |body: &Body| {
        body.satellites
            .as_ref()
            .map(|part| (part.mesh, part.layout, part.count, part.behave))
    };

    let orb = aux("orbital_command", "orb");
    assert_eq!(orb, aux("orbital_guard", "orb"));
    assert_eq!(
        (orb.archetype, orb.altitude, orb.model),
        (Archetype::Orbiter, Some(Altitude::High), Some(Model::Orb))
    );
    assert_eq!(lead(orb), Some((Silhouette::Torus, Behaviour::Tumble)));
    assert_eq!(
        orb.shell.as_ref().map(|part| (part.mesh, part.behave)),
        Some((Silhouette::Torus, Behaviour::Gyro))
    );
    assert_eq!(
        ring(orb),
        Some((
            Silhouette::Ball,
            SatelliteLayout::Orbit,
            2,
            Behaviour::Steady
        ))
    );
    assert_eq!(
        (orb.trail, orb.marker, orb.expire),
        (Trail::Ribbon, Marker::OwnerTether, ExpireKind::None)
    );
    // A row that forks the orb is refused, whichever copy changed.
    for changed in ["orbital_command", "orbital_guard"] {
        let mut forked: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        forked["skills"][changed]["aux"]["orb"]["trail"] = "motes".into();
        assert_eq!(
            schema_rules::parse(&forked).err().as_deref(),
            Some("orbital_guard: aux.orb must equal the body declared by orbital_command"),
            "{changed}"
        );
    }

    // The wave's warning says nothing about the side the caster stands on: no core, and
    // satellites without a facing.
    let warning = aux("horizon_wave", "beam_warning");
    assert_eq!(
        (warning.archetype, warning.fill, warning.marker),
        (Archetype::Lane, Some(true), Marker::FillToEdge)
    );
    assert_eq!(lead(warning), None);
    assert_eq!(
        ring(warning),
        Some((
            Silhouette::Diamond,
            SatelliteLayout::Line,
            6,
            Behaviour::Pulse
        ))
    );

    let soul = aux("iron_hook", "soul");
    assert_eq!(
        (soul.archetype, soul.trail, soul.marker),
        (Archetype::Traveller, Trail::None, Marker::None)
    );
    assert_eq!(lead(soul), Some((Silhouette::Drop, Behaviour::Bob)));

    let anchor = aux("anchor_step", "anchor");
    assert_eq!(
        (anchor.archetype, anchor.marker),
        (Archetype::Zone, Marker::RemainingRing)
    );
    assert_eq!(lead(anchor), Some((Silhouette::Chevron, Behaviour::Spin)));

    let healing = aux("fourfold_duel", "healing");
    assert_eq!(
        (healing.archetype, healing.marker),
        (Archetype::Zone, Marker::RemainingRing)
    );
    assert_eq!(lead(healing), Some((Silhouette::Cross, Behaviour::Pulse)));
    assert_eq!(
        ring(healing),
        Some((
            Silhouette::Chevron,
            SatelliteLayout::Rim,
            4,
            Behaviour::Pulse
        ))
    );

    // Bodies on positional ids end without a classified one-shot.
    for (id, kind) in [
        ("orbital_command", "orb"),
        ("iron_hook", "soul"),
        ("anchor_step", "anchor"),
        ("fourfold_duel", "healing"),
    ] {
        assert_eq!(aux(id, kind).expire, ExpireKind::None, "{id}");
    }
}
