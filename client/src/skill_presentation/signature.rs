//! The identity of a skill as a player reads it, and the ratchet that keeps two skills from
//! sharing one. Three axes: the motion family of the release, the body, the impact. Colour,
//! rate, counts, sizes and sounds are not axes.
// Read by the identity tests and, through `evidence`, by the capture harness of a QA build.
#![cfg_attr(not(test), allow(dead_code))]

use super::category::{self, Category, SkillKey};
use super::schema::{self, Body, SkillProfile};
use super::vocab::{
    AccentPattern, Archetype, AudioBase, AudioSlice, ImpactKind, Model, MovePattern, ParticleShape,
    ProjectileForm, ProjectilePresentation, SatelliteLayout, Silhouette, Trail,
};
use super::{EffectStyle, SkillPresentation};
use crate::combat_visuals::{CombatVisualProfile, CombatVisualRegistry, ProjectileShape};
use crate::humanoid::SharedHumanoidMotion;
use shared::HeroClass;
use shared::combat::ProjectileStyle;
use shared::loadout::AttackProfileId;
use std::collections::{BTreeMap, BTreeSet};

/// A baked mirror shows the movement of its source, so it is the same family. Look gate G1
/// failed: `aim_loose_r` reaches the pose of `cast_thrust_r` at its contact key, so the two
/// count as one family as well.
pub(crate) fn motion_family(clip: &str) -> &str {
    match clip {
        "slash_down_m" => "slash_down",
        "slash_rising_m" => "slash_rising",
        "aim_loose_r" => "cast_thrust_r",
        other => other,
    }
}

/// What stands at the centre of a world body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BodyLead {
    Mesh(Silhouette),
    Model(Model),
    None,
}

/// The thrown body of a legacy projectile in 3D.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectileBody {
    Form(ProjectileForm),
    /// The default body of a profile without a form: its shape, or its packaged model.
    Shape(ProjectileShape, Option<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BodySig {
    /// A replicated effect drawn through its `body`.
    World {
        archetype: Archetype,
        lead: BodyLead,
        satellites: Option<(Silhouette, SatelliteLayout)>,
        trail: Trail,
    },
    /// A legacy projectile, read from `combat_visuals.json`.
    Projectile {
        body: ProjectileBody,
        silhouette: Option<Silhouette>,
        presentation: ProjectilePresentation,
    },
    /// A skill without a body of its own: its cast choreography.
    Choreography {
        pattern: AccentPattern,
        lead: Option<ParticleShape>,
        movement: Option<MovePattern>,
    },
    /// A row that still draws through its legacy style.
    Legacy(Option<EffectStyle>),
}

impl BodySig {
    /// The shape a player would name: core mesh, else model, else satellites; the
    /// projectile silhouette; the lead shape of the cast.
    pub(crate) fn silhouette(&self) -> String {
        match self {
            Self::World {
                lead, satellites, ..
            } => match (lead, satellites) {
                (BodyLead::Mesh(mesh), _) => mesh.id().into(),
                (BodyLead::Model(model), _) => format!("model:{}", model.id()),
                (BodyLead::None, Some((mesh, _))) => mesh.id().into(),
                (BodyLead::None, None) => "-".into(),
            },
            Self::Projectile {
                body, silhouette, ..
            } => match (silhouette, body) {
                (Some(mesh), _) => mesh.id().into(),
                (None, ProjectileBody::Shape(_, Some(model))) => format!("model:{model}"),
                (None, _) => "-".into(),
            },
            Self::Choreography { lead, .. } => lead.map_or("-", ParticleShape::id).into(),
            Self::Legacy(style) => format!("legacy:{style:?}"),
        }
    }

    /// `(archetype | form | pattern, silhouette)`; unique across the roster when final.
    pub(crate) fn key(&self) -> (String, String) {
        let head = match self {
            Self::World { archetype, .. } => archetype.id().to_string(),
            Self::Projectile {
                body: ProjectileBody::Form(form),
                ..
            } => form.id().to_string(),
            Self::Projectile {
                body: ProjectileBody::Shape(shape, _),
                ..
            } => format!("shape:{shape:?}"),
            Self::Choreography { pattern, .. } => pattern.id().to_string(),
            Self::Legacy(_) => "legacy".to_string(),
        };
        (head, self.silhouette())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImpactSig {
    /// The skill cannot produce a damage receipt.
    None,
    /// A damaging row without a recipe: the shared wire-style burst.
    Unthemed,
    Themed {
        kind: ImpactKind,
        lead: ParticleShape,
    },
}

impl ImpactSig {
    fn kind(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Unthemed => Some("unthemed"),
            Self::Themed { kind, .. } => Some(kind.id()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    /// Motion family of the release clip.
    pub motion: String,
    pub body: BodySig,
    pub impact: ImpactSig,
}

fn world_body(body: &Body) -> BodySig {
    BodySig::World {
        archetype: body.archetype,
        lead: match (&body.core, body.model) {
            (Some(core), _) => BodyLead::Mesh(core.mesh),
            (None, Some(model)) => BodyLead::Model(model),
            (None, None) => BodyLead::None,
        },
        satellites: body
            .satellites
            .as_ref()
            .map(|satellites| (satellites.mesh, satellites.layout)),
        trail: body.trail,
    }
}

fn projectile_body(profile: &CombatVisualProfile) -> BodySig {
    BodySig::Projectile {
        body: match profile.form {
            // A form is drawn instead of a packaged model.
            Some(form) => ProjectileBody::Form(form),
            None => ProjectileBody::Shape(
                profile.shape,
                profile.model.as_ref().map(|model| model.path.clone()),
            ),
        },
        // A form without a named silhouette is drawn with the one of its form.
        silhouette: profile
            .silhouette
            .or(profile.form.map(crate::combat_visuals::default_silhouette)),
        presentation: profile.presentation,
    }
}

fn legacy_projectile(
    projectiles: &CombatVisualRegistry,
    key: SkillKey,
) -> Option<&CombatVisualProfile> {
    match key {
        SkillKey::Legacy(class, slot) if category::category(key) == Category::LegacyProjectile => {
            Some(projectiles.resolve(
                Some(class),
                ProjectileStyle::for_class(class),
                Some(slot.index() as u8),
                None,
                None,
            ))
        }
        _ => None,
    }
}

fn row_identity(
    profile: &SkillProfile,
    key: SkillKey,
    projectiles: &CombatVisualRegistry,
) -> Identity {
    let body = if let Some(projectile) = legacy_projectile(projectiles, key) {
        projectile_body(projectile)
    } else if category::category(key) == Category::ReplicatedEffect {
        profile
            .body
            .as_ref()
            .map_or(BodySig::Legacy(profile.effect), world_body)
    } else {
        profile
            .cast
            .as_ref()
            .map_or(BodySig::Legacy(profile.effect), |cast| {
                BodySig::Choreography {
                    pattern: cast.pattern,
                    lead: cast.lead(),
                    movement: cast.movement.as_ref().map(|movement| movement.pattern),
                }
            })
    };
    let impact = match &profile.impact {
        _ if !category::can_damage(key) => ImpactSig::None,
        Some(impact) => ImpactSig::Themed {
            kind: impact.kind,
            lead: impact.lead(),
        },
        None => ImpactSig::Unthemed,
    };
    Identity {
        motion: motion_family(&profile.release).to_string(),
        body,
        impact,
    }
}

/// The identity of one row, or `None` when the registry has no such skill.
pub(crate) fn identity(
    reg: &SkillPresentation,
    projectiles: &CombatVisualRegistry,
    id: &str,
) -> Option<Identity> {
    let key = SkillKey::from_id(id)?;
    Some(row_identity(reg.row(id)?, key, projectiles))
}

/// How far a registry is from the final rules. Every counter is zero when all rows are
/// migrated and no two skills can be mistaken for each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RatchetCounts {
    /// U2: pairs of skills that differ on fewer than two of motion, body, impact.
    pub pairs_under_two_axes: usize,
    /// U1: pairs of skills with the same motion, body and impact.
    pub full_tuple_duplicates: usize,
    /// U4: rows whose body key another row already has.
    pub duplicate_body_keys: usize,
    /// U3: classes whose skills share a motion family.
    pub classes_repeating_motion_family: usize,
    /// U3: classes whose skills share a body silhouette.
    pub classes_repeating_body_silhouette: usize,
    /// U3: classes whose skills share an impact kind.
    pub classes_repeating_impact_kind: usize,
    /// U5: motion families that release more than three skills.
    pub families_over_three_skills: usize,
    /// U5: impact kinds used by more than five skills.
    pub impact_kinds_over_five_skills: usize,
    pub rows_without_cast: usize,
    /// Rows of a replicated effect without `body`.
    pub effect_rows_without_body: usize,
    pub damaging_rows_without_impact: usize,
    pub rows_with_effect: usize,
    pub rows_breaking_contact_rule: usize,
    pub rows_breaking_luminance: usize,
    pub legacy_projectiles_without_form: usize,
    pub rows_without_cast_voice: usize,
    /// U6: rows whose cast voice another row already has.
    pub duplicate_cast_voices: usize,
    pub classes_without_basic_row: usize,
    /// Thrown basic-attack bodies that another class already has.
    pub basic_projectile_key_duplicates: usize,
}

impl RatchetCounts {
    pub(crate) fn entries(&self) -> [(&'static str, usize); 19] {
        [
            ("pairs_under_two_axes", self.pairs_under_two_axes),
            ("full_tuple_duplicates", self.full_tuple_duplicates),
            ("duplicate_body_keys", self.duplicate_body_keys),
            (
                "classes_repeating_motion_family",
                self.classes_repeating_motion_family,
            ),
            (
                "classes_repeating_body_silhouette",
                self.classes_repeating_body_silhouette,
            ),
            (
                "classes_repeating_impact_kind",
                self.classes_repeating_impact_kind,
            ),
            (
                "families_over_three_skills",
                self.families_over_three_skills,
            ),
            (
                "impact_kinds_over_five_skills",
                self.impact_kinds_over_five_skills,
            ),
            ("rows_without_cast", self.rows_without_cast),
            ("effect_rows_without_body", self.effect_rows_without_body),
            (
                "damaging_rows_without_impact",
                self.damaging_rows_without_impact,
            ),
            ("rows_with_effect", self.rows_with_effect),
            (
                "rows_breaking_contact_rule",
                self.rows_breaking_contact_rule,
            ),
            ("rows_breaking_luminance", self.rows_breaking_luminance),
            (
                "legacy_projectiles_without_form",
                self.legacy_projectiles_without_form,
            ),
            ("rows_without_cast_voice", self.rows_without_cast_voice),
            ("duplicate_cast_voices", self.duplicate_cast_voices),
            ("classes_without_basic_row", self.classes_without_basic_row),
            (
                "basic_projectile_key_duplicates",
                self.basic_projectile_key_duplicates,
            ),
        ]
    }
}

/// The shipped registry today. A data package lowers these as it migrates rows; the last
/// one sets every counter to zero.
pub(crate) const SHIPPED_RATCHET: RatchetCounts = RatchetCounts {
    pairs_under_two_axes: 61,
    full_tuple_duplicates: 3,
    duplicate_body_keys: 16,
    classes_repeating_motion_family: 4,
    classes_repeating_body_silhouette: 4,
    classes_repeating_impact_kind: 8,
    families_over_three_skills: 4,
    impact_kinds_over_five_skills: 1,
    rows_without_cast: 32,
    effect_rows_without_body: 17,
    damaging_rows_without_impact: 24,
    rows_with_effect: 32,
    rows_breaking_contact_rule: 23,
    rows_breaking_luminance: 10,
    legacy_projectiles_without_form: 0,
    rows_without_cast_voice: 32,
    duplicate_cast_voices: 0,
    classes_without_basic_row: 8,
    basic_projectile_key_duplicates: 1,
};

/// How many items repeat one that came before.
fn repeats<T: Ord>(items: impl IntoIterator<Item = T>) -> usize {
    let mut seen = BTreeSet::new();
    let mut repeated = 0;
    for item in items {
        repeated += usize::from(!seen.insert(item));
    }
    repeated
}

fn users_over<T: Ord>(items: impl IntoIterator<Item = T>, limit: usize) -> usize {
    let mut users = BTreeMap::new();
    for item in items {
        *users.entry(item).or_insert(0usize) += 1;
    }
    users.values().filter(|count| **count > limit).count()
}

/// The body of every basic attack that is thrown. Melee cores resolve contact at once and
/// throw nothing (`common/src/basic_attack.rs:187-202`).
fn basic_projectile_bodies(projectiles: &CombatVisualRegistry) -> Vec<BodySig> {
    let mut bodies = Vec::new();
    for class in HeroClass::ALL {
        let styles = match shared::loadout::preset_for_class(class).map(|kit| kit.attack_profile())
        {
            Some(AttackProfileId::Melee) => vec![],
            Some(AttackProfileId::Repeater) => {
                vec![ProjectileStyle::Bullet, ProjectileStyle::Rocket]
            }
            Some(AttackProfileId::LightBolt) | None => vec![ProjectileStyle::for_class(class)],
        };
        for style in styles {
            bodies.push(projectile_body(projectiles.resolve(
                Some(class),
                style,
                Some(shared::BASIC_ATTACK_ACTION_SLOT),
                None,
                None,
            )));
        }
    }
    bodies
}

pub(crate) fn ratchet(
    reg: &SkillPresentation,
    projectiles: &CombatVisualRegistry,
) -> Result<RatchetCounts, String> {
    let motion = SharedHumanoidMotion::embedded()?;
    let rows: Vec<(SkillKey, &SkillProfile, Identity)> = reg
        .rows()
        .filter_map(|(id, profile)| {
            let key = SkillKey::from_id(id)?;
            Some((key, profile, row_identity(profile, key, projectiles)))
        })
        .collect();
    let mut counts = RatchetCounts::default();

    for (index, (_, _, a)) in rows.iter().enumerate() {
        for (_, _, b) in &rows[index + 1..] {
            let shared = usize::from(a.motion == b.motion)
                + usize::from(a.body == b.body)
                + usize::from(a.impact == b.impact);
            counts.pairs_under_two_axes += usize::from(shared >= 2);
            counts.full_tuple_duplicates += usize::from(shared == 3);
        }
    }
    counts.duplicate_body_keys = repeats(rows.iter().map(|(_, _, identity)| identity.body.key()));
    for class in HeroClass::ALL {
        let kit: Vec<_> = rows
            .iter()
            .filter(|(key, _, _)| key.home() == class)
            .map(|(_, _, identity)| identity)
            .collect();
        counts.classes_repeating_motion_family +=
            usize::from(repeats(kit.iter().map(|identity| &identity.motion)) > 0);
        counts.classes_repeating_body_silhouette +=
            usize::from(repeats(kit.iter().map(|identity| identity.body.silhouette())) > 0);
        counts.classes_repeating_impact_kind +=
            usize::from(repeats(kit.iter().filter_map(|identity| identity.impact.kind())) > 0);
        counts.classes_without_basic_row += usize::from(reg.basic(class).is_none());
    }
    counts.families_over_three_skills =
        users_over(rows.iter().map(|(_, _, identity)| &identity.motion), 3);
    counts.impact_kinds_over_five_skills = users_over(
        rows.iter()
            .filter_map(|(_, _, identity)| identity.impact.kind()),
        5,
    );

    let mut voices: Vec<(AudioBase, u32, AudioSlice)> = Vec::new();
    for (key, profile, _) in &rows {
        let category = category::category(*key);
        counts.rows_without_cast += usize::from(profile.cast.is_none());
        counts.effect_rows_without_body +=
            usize::from(category == Category::ReplicatedEffect && profile.body.is_none());
        counts.damaging_rows_without_impact +=
            usize::from(category::can_damage(*key) && profile.impact.is_none());
        counts.rows_with_effect += usize::from(profile.effect.is_some());
        counts.rows_breaking_contact_rule +=
            usize::from(schema::contact_violation(profile, *key, motion).is_some());
        counts.rows_breaking_luminance += usize::from(
            reg.theme(key.home())
                .is_none_or(|theme| schema::luminance_violation(profile, theme).is_some()),
        );
        counts.legacy_projectiles_without_form += usize::from(
            legacy_projectile(projectiles, *key).is_some_and(|profile| profile.form.is_none()),
        );
        match profile.sound.as_ref().and_then(|sound| sound.cast.as_ref()) {
            // Speeds lie on a 0.05 grid, so the step number names the speed exactly.
            Some(cue) => voices.push((cue.base, (cue.speed * 20.0).round() as u32, cue.slice)),
            None => counts.rows_without_cast_voice += 1,
        }
    }
    counts.duplicate_cast_voices = repeats(voices);
    // The two melee-contact basics share the engine's reach streak by design.
    counts.basic_projectile_key_duplicates = {
        let thrown: Vec<_> = basic_projectile_bodies(projectiles)
            .into_iter()
            .filter(|body| {
                !matches!(
                    body,
                    BodySig::Projectile {
                        presentation: ProjectilePresentation::MeleeContact,
                        ..
                    }
                )
            })
            .collect();
        thrown
            .iter()
            .enumerate()
            .filter(|(index, body)| thrown[..*index].contains(body))
            .count()
    };
    Ok(counts)
}

/// Items whose key an earlier item already has, each named with the first holder.
#[cfg(test)]
fn repeated<K: Ord + std::fmt::Debug>(items: impl IntoIterator<Item = (K, String)>) -> Vec<String> {
    let mut first: BTreeMap<K, String> = BTreeMap::new();
    let mut found = Vec::new();
    for (key, name) in items {
        match first.get(&key) {
            Some(holder) => found.push(format!("{name} repeats {key:?} of {holder}")),
            None => {
                first.insert(key, name);
            }
        }
    }
    found
}

/// Keys with more than `limit` users, each with its users.
#[cfg(test)]
fn crowded<K: Ord + std::fmt::Debug>(
    items: impl IntoIterator<Item = (K, String)>,
    limit: usize,
) -> Vec<String> {
    let mut users: BTreeMap<K, Vec<String>> = BTreeMap::new();
    for (key, name) in items {
        users.entry(key).or_default().push(name);
    }
    users
        .into_iter()
        .filter(|(_, users)| users.len() > limit)
        .map(|(key, users)| format!("{key:?} has {}: {}", users.len(), users.join(", ")))
        .collect()
}

/// What [`ratchet`] counts, item by item: `(counter, the rows it is about)`. A failing
/// identity test prints these, so it names the skills that collide.
#[cfg(test)]
pub(crate) fn findings(
    reg: &SkillPresentation,
    projectiles: &CombatVisualRegistry,
) -> Result<Vec<(&'static str, String)>, String> {
    let motion = SharedHumanoidMotion::embedded()?;
    let rows: Vec<(&str, SkillKey, &SkillProfile, Identity)> = reg
        .rows()
        .filter_map(|(id, profile)| {
            let key = SkillKey::from_id(id)?;
            Some((id, key, profile, row_identity(profile, key, projectiles)))
        })
        .collect();
    let mut found: Vec<(&'static str, String)> = Vec::new();
    let mut note = |counter: &'static str, items: Vec<String>| {
        found.extend(items.into_iter().map(|item| (counter, item)));
    };

    for (index, (a_id, _, _, a)) in rows.iter().enumerate() {
        for (b_id, _, _, b) in &rows[index + 1..] {
            let shared: Vec<&str> = [
                ("motion", a.motion == b.motion),
                ("body", a.body == b.body),
                ("impact", a.impact == b.impact),
            ]
            .into_iter()
            .filter_map(|(axis, same)| same.then_some(axis))
            .collect();
            if shared.len() >= 2 {
                let pair = format!("{a_id} / {b_id} share {}", shared.join(", "));
                note("pairs_under_two_axes", vec![pair.clone()]);
                if shared.len() == 3 {
                    note("full_tuple_duplicates", vec![pair]);
                }
            }
        }
    }
    let named = |name: &str| name.to_string();
    note(
        "duplicate_body_keys",
        repeated(
            rows.iter()
                .map(|(id, _, _, identity)| (identity.body.key(), named(id))),
        ),
    );
    for class in HeroClass::ALL {
        let kit: Vec<_> = rows
            .iter()
            .filter(|(_, key, _, _)| key.home() == class)
            .collect();
        let per_class = |repeats: Vec<String>| {
            if repeats.is_empty() {
                vec![]
            } else {
                vec![format!("{}: {}", class.id(), repeats.join("; "))]
            }
        };
        note(
            "classes_repeating_motion_family",
            per_class(repeated(
                kit.iter()
                    .map(|(id, _, _, identity)| (identity.motion.clone(), named(id))),
            )),
        );
        note(
            "classes_repeating_body_silhouette",
            per_class(repeated(kit.iter().map(|(id, _, _, identity)| {
                (identity.body.silhouette(), named(id))
            }))),
        );
        note(
            "classes_repeating_impact_kind",
            per_class(repeated(kit.iter().filter_map(|(id, _, _, identity)| {
                identity.impact.kind().map(|kind| (kind, named(id)))
            }))),
        );
        if reg.basic(class).is_none() {
            note("classes_without_basic_row", vec![named(class.id())]);
        }
    }
    note(
        "families_over_three_skills",
        crowded(
            rows.iter()
                .map(|(id, _, _, identity)| (identity.motion.clone(), named(id))),
            3,
        ),
    );
    note(
        "impact_kinds_over_five_skills",
        crowded(
            rows.iter().filter_map(|(id, _, _, identity)| {
                identity.impact.kind().map(|kind| (kind, named(id)))
            }),
            5,
        ),
    );

    let mut voices = Vec::new();
    for (id, key, profile, _) in &rows {
        let category = category::category(*key);
        let mut row = |counter: &'static str, counted: bool| {
            if counted {
                note(counter, vec![named(id)]);
            }
        };
        row("rows_without_cast", profile.cast.is_none());
        row(
            "effect_rows_without_body",
            category == Category::ReplicatedEffect && profile.body.is_none(),
        );
        row(
            "damaging_rows_without_impact",
            category::can_damage(*key) && profile.impact.is_none(),
        );
        row("rows_with_effect", profile.effect.is_some());
        row(
            "legacy_projectiles_without_form",
            legacy_projectile(projectiles, *key).is_some_and(|profile| profile.form.is_none()),
        );
        if let Some(late) = schema::contact_violation(profile, *key, motion) {
            note("rows_breaking_contact_rule", vec![format!("{id}: {late}")]);
        }
        match reg.theme(key.home()) {
            Some(theme) => {
                if let Some(flat) = schema::luminance_violation(profile, theme) {
                    note("rows_breaking_luminance", vec![format!("{id}: {flat}")]);
                }
            }
            None => note("rows_breaking_luminance", vec![format!("{id}: no theme")]),
        }
        match profile.sound.as_ref().and_then(|sound| sound.cast.as_ref()) {
            Some(cue) => voices.push((
                (cue.base, (cue.speed * 20.0).round() as u32, cue.slice),
                named(id),
            )),
            None => note("rows_without_cast_voice", vec![named(id)]),
        }
    }
    note("duplicate_cast_voices", repeated(voices));
    let thrown: Vec<BodySig> = basic_projectile_bodies(projectiles)
        .into_iter()
        .filter(|body| {
            !matches!(
                body,
                BodySig::Projectile {
                    presentation: ProjectilePresentation::MeleeContact,
                    ..
                }
            )
        })
        .collect();
    note(
        "basic_projectile_key_duplicates",
        thrown
            .iter()
            .enumerate()
            .filter(|(index, body)| thrown[..*index].contains(body))
            .map(|(_, body)| format!("{body:?}"))
            .collect(),
    );
    Ok(found)
}
