//! `skills.skillfx` schema 2: the rows and their validation. Designers author palette,
//! silhouette, motion, accents, impact and voice; every pick that would contradict a fact
//! derived from the catalog (`category.rs`, `geometry.rs`) is rejected here, and every
//! particle block is run through its generator, because a declared count proves nothing.
use super::accents::{self, CastContext, OneShot, Palette};
use super::category::{self, Category, SkillKey};
use super::impacts::{self, ImpactContext};
use super::vocab::{
    AccentPattern, Altitude, Archetype, AudioBase, AudioSlice, Behaviour, ExpireKind, ImpactKind,
    Marker, Model, MotionPhase, MovePattern, PaletteSlot, ParticleShape, RecastMarker,
    SatelliteLayout, Silhouette, StageRule, Trail,
};
use super::{SkillPresentation, bodies, geometry};
use crate::game_vfx::ParticleSpec;
use crate::humanoid::SharedHumanoidMotion;
use bevy::math::{Vec2, Vec3, Vec3Swizzles};
use serde::Deserialize;
use shared::HeroClass;
use shared::loadout::{EffectVisualKind, SkillEffectState, SkillId};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

pub(super) const SCHEMA_VERSION: u32 = 2;
pub(super) const MAX_BYTES: usize = 256 * 1024;
const MAX_SKILLS: usize = 80;
/// An edge-released clip must show its contact pose within this time of the accepted cast.
pub(crate) const CONTACT_LIMIT_SECS: f32 = 0.15;
/// The skill colour and the matter colour must differ by this much relative luminance.
pub(crate) const MIN_LUMINANCE_GAP: f32 = 0.25;

/// Class colours shared by the four skills of a kit.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Theme {
    /// The matter colour, drawn without HDR gain.
    pub secondary: [f32; 3],
    /// The spark colour, drawn with the skill's HDR gain.
    pub accent: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillProfile {
    /// The class whose default kit owns the skill; it selects the theme.
    pub home: String,
    pub release: String,
    #[serde(default)]
    pub windup: Option<String>,
    pub color: [f32; 3],
    /// Linear radiance for small cores/rims, independent of broad field opacity.
    #[serde(default = "default_hdr_gain")]
    pub hdr_gain: f32,
    /// Per-skill overrides of the theme colours.
    #[serde(default)]
    pub secondary: Option<[f32; 3]>,
    #[serde(default)]
    pub accent: Option<[f32; 3]>,
    #[serde(default)]
    pub motion: MotionPlayback,
    /// What the accepted cast draws at the caster. Every row has one.
    #[serde(default)]
    pub cast: Option<CastAccent>,
    /// The body of the effect of the first cast; a row of a replicated effect has one.
    #[serde(default)]
    pub body: Option<Body>,
    /// Bodies of the secondary objects of the skill, keyed by their replicated kind.
    #[serde(default)]
    pub aux: BTreeMap<String, Body>,
    /// The hit of a confirmed receipt; a row that can deal damage has one.
    #[serde(default)]
    pub impact: Option<ImpactRecipe>,
    /// The voices of the row. Every row has `sound.cast`.
    #[serde(default)]
    pub sound: Option<Sound>,
}

fn default_hdr_gain() -> f32 {
    3.0
}
fn one() -> f32 {
    1.0
}

impl SkillProfile {
    /// The phase the release follows: the authored one, else the skill's own telegraph when
    /// the row holds a windup against it, else the accepted cast.
    pub(crate) fn phase(&self, key: SkillKey) -> MotionPhase {
        self.motion.phase.unwrap_or_else(|| {
            if self.windup.is_some() {
                derived_phase(key)
            } else {
                MotionPhase::Instant
            }
        })
    }
}

fn derived_phase(key: SkillKey) -> MotionPhase {
    key.modular()
        .map_or(MotionPhase::Instant, category::derived_phase)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionPlayback {
    /// Multiplies the playback speed of the action clip.
    #[serde(default = "one")]
    pub rate: f32,
    /// Fraction of the clip skipped at its start.
    #[serde(default)]
    pub start: f32,
    #[serde(default)]
    pub phase: Option<MotionPhase>,
    /// Fit a non-looping windup to the skill's telegraph time.
    #[serde(default)]
    pub fit_windup: bool,
    /// Played instead of `release` on a recast.
    #[serde(default)]
    pub recast: Option<String>,
    #[serde(default = "one")]
    pub recast_rate: f32,
}

impl Default for MotionPlayback {
    fn default() -> Self {
        Self {
            rate: 1.0,
            start: 0.0,
            phase: None,
            fit_windup: false,
            recast: None,
            recast_rate: 1.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CastAccent {
    pub pattern: AccentPattern,
    /// Lead particle shape; the pattern's default when absent.
    #[serde(default)]
    pub shape: Option<ParticleShape>,
    #[serde(default)]
    pub count: Option<u8>,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default = "default_accent_lifetime")]
    pub lifetime: f32,
    /// Colours of the lead and the secondary particles; `[primary, accent]` when absent.
    #[serde(default)]
    pub slots: Option<[PaletteSlot; 2]>,
    /// Accent of a recast, drawn with that pattern's default lead and slots.
    #[serde(default)]
    pub recast: Option<AccentPattern>,
    #[serde(default, rename = "move")]
    pub movement: Option<MoveSpec>,
    /// Streaks from the caster to each matching accepted receipt.
    #[serde(default)]
    pub link: Option<ParticleShape>,
    /// Flash the exact instant area of a signed-off skill.
    #[serde(default)]
    pub area: bool,
    #[serde(default)]
    pub recast_marker: Option<RecastMarker>,
}

fn default_accent_lifetime() -> f32 {
    0.35
}

impl CastAccent {
    /// The accent a pattern draws when a row names nothing else. A recast accent is always
    /// this one, whatever the row names for its first cast.
    pub(crate) fn plain(pattern: AccentPattern) -> Self {
        Self {
            pattern,
            shape: None,
            count: None,
            scale: one(),
            lifetime: default_accent_lifetime(),
            slots: None,
            recast: None,
            movement: None,
            link: None,
            area: false,
            recast_marker: None,
        }
    }

    pub(crate) fn lead(&self) -> Option<ParticleShape> {
        self.shape.or(self.pattern.default_lead())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveSpec {
    pub pattern: MovePattern,
    #[serde(default)]
    pub shape: Option<ParticleShape>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Body {
    pub archetype: Archetype,
    #[serde(default)]
    pub core: Option<Part>,
    #[serde(default)]
    pub shell: Option<Part>,
    #[serde(default)]
    pub satellites: Option<Satellites>,
    #[serde(default)]
    pub trail: Trail,
    /// Factor on the engine's size of the trail parts; 1.0 when absent.
    #[serde(default)]
    pub trail_scale: Option<f32>,
    /// One translucent interior layer of an area archetype; the archetype decides when absent.
    #[serde(default)]
    pub fill: Option<bool>,
    /// How strongly that layer tints the ground; the engine's strength when absent.
    #[serde(default)]
    pub fill_strength: Option<f32>,
    #[serde(default)]
    pub marker: Marker,
    #[serde(default)]
    pub model: Option<Model>,
    /// The archetype decides when absent.
    #[serde(default)]
    pub altitude: Option<Altitude>,
    #[serde(default)]
    pub expire: ExpireKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Part {
    pub mesh: Silhouette,
    #[serde(default)]
    pub slot: PaletteSlot,
    /// Lateral, vertical and along the heading.
    pub size: [f32; 3],
    #[serde(default)]
    pub behave: Behaviour,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Satellites {
    pub mesh: Silhouette,
    pub layout: SatelliteLayout,
    pub count: u8,
    pub size: [f32; 3],
    #[serde(default)]
    pub slot: PaletteSlot,
    #[serde(default)]
    pub behave: Behaviour,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImpactRecipe {
    pub kind: ImpactKind,
    /// Lead particle shape; the kind's default when absent.
    #[serde(default)]
    pub shape: Option<ParticleShape>,
    #[serde(default)]
    pub count: Option<u8>,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default = "default_impact_lifetime")]
    pub lifetime: f32,
    /// `[primary, accent]` when absent.
    #[serde(default)]
    pub slots: Option<[PaletteSlot; 2]>,
}

fn default_impact_lifetime() -> f32 {
    0.45
}

impl ImpactRecipe {
    pub(crate) fn lead(&self) -> ParticleShape {
        self.shape.unwrap_or(self.kind.default_lead())
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sound {
    #[serde(default)]
    pub cast: Option<SoundCue>,
    /// Played when a telegraphed skill releases.
    #[serde(default)]
    pub release: Option<SoundCue>,
    #[serde(default)]
    pub impact: Option<SoundCue>,
    #[serde(default)]
    pub recast: Option<SoundCue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SoundCue {
    pub base: AudioBase,
    /// Playback speed, 0.70..=1.40 in steps of 0.05.
    #[serde(default = "one")]
    pub speed: f32,
    #[serde(default)]
    pub slice: AudioSlice,
    #[serde(default = "one")]
    pub gain: f32,
    /// Extra notes of the same base, for the local hero only.
    #[serde(default)]
    pub notes: Vec<SoundNote>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SoundNote {
    pub delay_ms: u16,
    pub speed: f32,
    pub gain: f32,
}

/// The basic attack of a class. It belongs to the class body, not to a skill.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct BasicProfile {
    /// One clip, or two that alternate on odd and even action sequences.
    pub motions: Vec<String>,
    #[serde(default = "one")]
    pub rate: f32,
    #[serde(default)]
    pub start: f32,
    #[serde(default)]
    pub accent: Option<CastAccent>,
    #[serde(default)]
    pub impact: Option<ImpactRecipe>,
    /// Played on the confirmed hit, and for an enemy's accepted attack in place of the cue
    /// of its wire style.
    #[serde(default)]
    pub sound: Option<SoundCue>,
    /// The rocket mode of the repeater attack profile.
    #[serde(default)]
    pub rockets: Option<Box<BasicProfile>>,
}

fn in_range(value: f32, range: RangeInclusive<f32>, name: &str) -> Result<(), String> {
    if value.is_finite() && range.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{name} must be within {}..={}",
            range.start(),
            range.end()
        ))
    }
}

fn unit_color(color: [f32; 3]) -> bool {
    color
        .iter()
        .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
}

/// WCAG relative luminance of an sRGB colour.
fn luminance(color: [f32; 3]) -> f32 {
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

/// Why the skill colour cannot be told from the matter colour it is drawn next to.
pub(crate) fn luminance_violation(profile: &SkillProfile, theme: &Theme) -> Option<String> {
    let gap =
        (luminance(profile.color) - luminance(profile.secondary.unwrap_or(theme.secondary))).abs();
    (gap < MIN_LUMINANCE_GAP).then(|| {
        format!("`color` and the secondary colour differ by {gap:.2} luminance (at least {MIN_LUMINANCE_GAP})")
    })
}

/// A clip a row may play as an action: in the library, not a base state, not a loop.
fn action_clip(motion: &SharedHumanoidMotion, field: &str, name: &str) -> Result<(), String> {
    if pickable_clip(motion, field, name)? {
        return Err(format!(
            "{field}: {name} is a loop and can only be a windup"
        ));
    }
    Ok(())
}

/// Whether the named clip loops; base states cannot be picked at all.
fn pickable_clip(motion: &SharedHumanoidMotion, field: &str, name: &str) -> Result<bool, String> {
    let clip = motion
        .clips
        .get(name)
        .ok_or_else(|| format!("Unknown motion {name}"))?;
    if matches!(name, "idle" | "walk" | "run" | "death") {
        return Err(format!("{field}: {name} is a base state"));
    }
    Ok(clip.looping)
}

/// Why a clip started on an accepted edge shows its contact pose too late (or never).
fn late_contact(
    motion: &SharedHumanoidMotion,
    field: &str,
    name: &str,
    rate: f32,
    start: f32,
) -> Option<String> {
    let Some((contact, clip)) = motion.contact(name).zip(motion.clips.get(name)) else {
        return Some(format!("{field}: {name} has no contact time"));
    };
    let delay = (contact - start * clip.duration) / rate;
    (delay > CONTACT_LIMIT_SECS + 1e-4).then(|| {
        format!("{field}: {name} reaches its contact {delay:.3} s after the cast (at most {CONTACT_LIMIT_SECS})")
    })
}

/// The contact rule over the edge-released clips of a skill row: the release when nothing
/// holds it back, and the recast clip.
pub(crate) fn contact_violation(
    profile: &SkillProfile,
    key: SkillKey,
    motion: &SharedHumanoidMotion,
) -> Option<String> {
    let playback = &profile.motion;
    let release = (profile.phase(key) == MotionPhase::Instant)
        .then(|| {
            late_contact(
                motion,
                "release",
                &profile.release,
                playback.rate,
                playback.start,
            )
        })
        .flatten();
    release.or_else(|| {
        playback.recast.as_ref().and_then(|recast| {
            late_contact(motion, "motion.recast", recast, playback.recast_rate, 0.0)
        })
    })
}

pub(super) fn unsupported_version(found: u32) -> String {
    format!("Unsupported skill presentation schema_version {found} (expected {SCHEMA_VERSION})")
}

pub(super) fn validate(config: &SkillPresentation) -> Result<(), String> {
    if config.schema_version != SCHEMA_VERSION {
        return Err(unsupported_version(config.schema_version));
    }
    if config.skills.len() > MAX_SKILLS {
        return Err(format!("Too many skill profiles (at most {MAX_SKILLS})"));
    }
    let motion = SharedHumanoidMotion::embedded()?;
    themes(config)?;
    for class in HeroClass::ALL {
        if !config.basic_attacks.contains_key(class.id()) {
            return Err(format!("basic_attacks: no row for {}", class.id()));
        }
    }
    for (class, basic) in &config.basic_attacks {
        basic_row(config, class, basic, motion, false)
            .map_err(|error| format!("basic_attacks.{class}: {error}"))?;
    }
    for (id, profile) in &config.skills {
        let key = SkillKey::from_id(id).ok_or_else(|| format!("Unknown skill {id}"))?;
        skill_row(config, key, profile, motion).map_err(|error| format!("{id}: {error}"))?;
    }
    shared_aux(config)?;
    material_budget(config)
}

/// Bodies share their materials by colour. Every skill colour needs three (lit, fill and
/// telegraph fill) for each HDR gain it is drawn with and two more for each further fill
/// strength its bodies name, every matter and every spark colour one, and the engine keeps
/// a fixed few of its own.
fn material_budget(config: &SkillPresentation) -> Result<(), String> {
    let bits = |color: [f32; 3]| color.map(f32::to_bits);
    let mut primaries = std::collections::BTreeSet::new();
    let mut secondaries = std::collections::BTreeSet::new();
    let mut accents = std::collections::BTreeSet::new();
    let mut strengths = std::collections::BTreeSet::new();
    for theme in config.themes.values() {
        secondaries.insert(bits(theme.secondary));
        accents.insert(bits(theme.accent));
    }
    for profile in config.skills.values() {
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
    let materials = 3 * primaries.len()
        + 2 * strengths.len()
        + secondaries.len()
        + accents.len()
        + bodies::SHARED_MATERIALS;
    if materials > bodies::MATERIAL_BUDGET {
        return Err(format!(
            "The rows need {materials} effect materials (at most {}): {} skill colours, {} secondary and {} accent colours",
            bodies::MATERIAL_BUDGET,
            primaries.len(),
            secondaries.len(),
            accents.len()
        ));
    }
    Ok(())
}

fn themes(config: &SkillPresentation) -> Result<(), String> {
    for class in HeroClass::ALL {
        let theme = config
            .themes
            .get(class.id())
            .ok_or_else(|| format!("themes: no theme for {}", class.id()))?;
        if !unit_color(theme.secondary) || !unit_color(theme.accent) {
            return Err(format!("themes.{}: invalid color", class.id()));
        }
    }
    match config
        .themes
        .keys()
        .find(|id| HeroClass::from_id(id).is_none())
    {
        Some(unknown) => Err(format!("themes: unknown class {unknown}")),
        None => Ok(()),
    }
}

fn skill_row(
    config: &SkillPresentation,
    key: SkillKey,
    profile: &SkillProfile,
    motion: &SharedHumanoidMotion,
) -> Result<(), String> {
    let home = key.home();
    if profile.home != home.id() {
        return Err(format!("`home` must be {}", home.id()));
    }
    if !unit_color(profile.color)
        || profile.secondary.is_some_and(|c| !unit_color(c))
        || profile.accent.is_some_and(|c| !unit_color(c))
    {
        return Err("Invalid color".into());
    }
    if !profile.hdr_gain.is_finite() || !(1.0..=8.0).contains(&profile.hdr_gain) {
        return Err("Invalid HDR gain (expected 1..=8)".into());
    }
    motion_block(profile, key, motion)?;

    let category = category::category(key);
    let replicated = category == Category::ReplicatedEffect;
    let damaging = category::can_damage(key);
    if profile.impact.is_some() && !damaging {
        return Err("`impact` needs a skill that can deal damage".into());
    }
    let Some(accent) = &profile.cast else {
        return Err("a row needs `cast`".into());
    };
    cast_accent(
        accent,
        AccentOwner::Skill {
            key,
            windup: profile.windup.is_some(),
        },
    )?;
    match (&profile.body, key.modular().filter(|_| replicated)) {
        (Some(body), Some(id)) => {
            body_block(body, id, Binding::Own).map_err(|error| format!("body: {error}"))?;
        }
        (Some(_), None) => {
            return Err("`body` needs a skill that replicates a world effect".into());
        }
        (None, _) => {}
    }
    for (name, body) in &profile.aux {
        let Some((id, kind)) = aux_kind(key, name) else {
            return Err(format!(
                "aux.{name}: the skill has no such secondary object"
            ));
        };
        body_block(body, id, Binding::Aux(kind)).map_err(|error| format!("aux.{name}: {error}"))?;
    }
    if let Some(impact) = &profile.impact {
        impact_recipe(impact, Some(key))?;
    }
    if let Some(sound) = &profile.sound {
        sound_block(sound, key, profile.phase(key), damaging)?;
    }

    if replicated && profile.body.is_none() {
        return Err("a row of a replicated effect needs `body`".into());
    }
    if damaging && profile.impact.is_none() {
        return Err("a damaging row needs `impact`".into());
    }
    if profile
        .sound
        .as_ref()
        .is_none_or(|sound| sound.cast.is_none())
    {
        return Err("a row needs `sound.cast`".into());
    }
    if let Some(late) = contact_violation(profile, key, motion) {
        return Err(late);
    }
    // `themes` was checked to cover every class.
    let Some(theme) = config.themes.get(home.id()) else {
        return Ok(());
    };
    if let Some(flat) = luminance_violation(profile, theme) {
        return Err(flat);
    }
    generated(profile, key, &Palette::of(profile, theme))
}

/// Why a generated burst cannot be drawn within its budget.
pub(super) fn burst_budget(
    specs: &[ParticleSpec],
    most: usize,
    secs: f32,
    block: &str,
) -> Result<(), String> {
    if specs.len() > most {
        return Err(format!(
            "{block} draws {} particles (at most {most})",
            specs.len()
        ));
    }
    if specs.iter().any(|spec| !spec.is_sound()) {
        return Err(format!("{block} draws a particle without a finite pose"));
    }
    let longest = specs.iter().map(ParticleSpec::end_secs).fold(0.0, f32::max);
    if longest > secs + 1e-4 {
        return Err(format!("{block} lasts {longest:.2} s (at most {secs})"));
    }
    Ok(())
}

/// Farthest any particle of a burst is drawn from `from` on the ground plane.
fn burst_reach(specs: &[ParticleSpec], from: Vec3) -> f32 {
    specs
        .iter()
        .map(|spec| spec.reach(from))
        .fold(0.0, f32::max)
}

/// Where the generators are run for validation; any finite place gives the same counts,
/// lifetimes and extents.
const PROBE: Vec3 = Vec3::new(3.0, 0.5, -2.0);
/// A long observed displacement or link: no skill moves or reaches farther.
const PROBE_TRAVEL: f32 = 30.0;

/// Runs the accent of a row for a first cast and for a recast, with the area the row would
/// flash and the farthest strike line the skill could draw, then its move and its link.
fn accent_output(
    accent: &CastAccent,
    palette: &Palette,
    id: Option<SkillId>,
    block: &str,
) -> Result<(), String> {
    let direction = Vec2::X;
    let area = id.filter(|_| accent.area).and_then(|id| {
        geometry::instant_area(
            id,
            &geometry::AreaContext {
                origin: PROBE.xz(),
                arrival: Some(PROBE.xz()),
                direction,
                recast: false,
            },
        )
    });
    for recast in [false, true] {
        let specs = accents::accent_particles(
            accent,
            palette,
            &CastContext {
                origin: PROBE,
                direction,
                recast,
                area,
                strike_to: Some(PROBE + Vec3::X * PROBE_TRAVEL),
                sequence: 1,
            },
        );
        burst_budget(&specs, accents::ACCENT_MAX, accents::ACCENT_SECS, block)?;
        // An outline and a strike line end at replicated geometry. Everything else is
        // decoration and stays at the caster.
        let drawn = accent.recast.filter(|_| recast).unwrap_or(accent.pattern);
        let derived = (area.is_some() && !recast) || drawn == AccentPattern::StrikeLine;
        let reach = burst_reach(&specs, PROBE);
        if !derived && reach > accents::DECORATIVE_REACH + 1e-3 {
            return Err(format!(
                "{block} reaches {reach:.2} units from the caster (at most {})",
                accents::DECORATIVE_REACH
            ));
        }
    }
    let far = PROBE + Vec3::X * PROBE_TRAVEL;
    if let Some(movement) = &accent.movement {
        for (from, to) in [
            (Some(PROBE), Some(far)),
            (None, Some(far)),
            (Some(PROBE), None),
        ] {
            burst_budget(
                &accents::move_particles(movement, palette, from, to, 1),
                accents::MOVE_MAX,
                accents::MOVE_SECS,
                "cast.move",
            )?;
        }
    }
    if let Some(shape) = accent.link {
        burst_budget(
            &accents::link_particles(shape, palette, PROBE, far, 1),
            accents::LINK_MAX,
            accents::LINK_SECS,
            "cast.link",
        )?;
    }
    Ok(())
}

/// Runs the impact of a row with and without a live effect heading. A skill without area
/// damage must keep the burst at the unit it hit.
fn impact_output(
    impact: &ImpactRecipe,
    palette: &Palette,
    area_damage: bool,
) -> Result<(), String> {
    for heading in [None, Some(Vec2::Y)] {
        let specs = impacts::impact_particles(
            impact,
            palette,
            &ImpactContext {
                position: PROBE,
                ground: PROBE.y,
                direction: Vec2::X,
                heading,
                area_damage,
                receipt: 1,
                reserved: 0,
            },
        );
        burst_budget(&specs, impacts::IMPACT_MAX, impacts::IMPACT_SECS, "impact")?;
        let reach = burst_reach(&specs, PROBE);
        if !area_damage && reach > impacts::SINGLE_TARGET_REACH + 1e-3 {
            return Err(format!(
                "impact reaches {reach:.2} units from the hit (at most {} without area damage)",
                impacts::SINGLE_TARGET_REACH
            ));
        }
    }
    Ok(())
}

/// An effect of the skill as the parser imagines it, `length` long.
fn probe_effect(id: SkillId, kind: EffectVisualKind, radius: f32, length: f32) -> SkillEffectState {
    SkillEffectState {
        id: 1,
        owner_id: 1,
        owner_team: shared::map::Team::Green,
        skill: id,
        kind,
        position: [PROBE.x, PROBE.z],
        end: [PROBE.x + length, PROBE.z],
        radius,
        remaining_secs: 0.0,
        armed: true,
        consumed_segments: 0,
    }
}

/// Runs the one-shot of a body's `expire` over small, usual and large effects of its kind.
fn expire_output(
    body: &Body,
    id: SkillId,
    kind: EffectVisualKind,
    palette: &Palette,
) -> Result<(), String> {
    let Some(oneshot) = OneShot::of(body.expire) else {
        return Ok(());
    };
    for (radius, length) in [(0.5, 2.0), (3.0, 12.0), (8.0, PROBE_TRAVEL)] {
        let geo = geometry::boundary_shape(id, kind, &probe_effect(id, kind, radius, length));
        burst_budget(
            &accents::stage_oneshot(oneshot, palette, &geo, PROBE.y, 1),
            accents::STAGE_MAX,
            accents::STAGE_SECS,
            "expire",
        )?;
    }
    Ok(())
}

/// Output validation of a skill row: every particle block is generated and measured.
fn generated(profile: &SkillProfile, key: SkillKey, palette: &Palette) -> Result<(), String> {
    let id = key.modular();
    if let Some(accent) = &profile.cast {
        accent_output(accent, palette, id, "cast")?;
    }
    if let Some(impact) = &profile.impact {
        impact_output(impact, palette, category::area_damage(key))?;
    }
    let Some(id) = id else {
        return Ok(());
    };
    if let Some((body, kind)) = profile.body.as_ref().zip(category::own_kinds(id).first()) {
        expire_output(body, id, *kind, palette).map_err(|error| format!("body: {error}"))?;
    }
    for (name, body) in &profile.aux {
        if let Some((_, kind)) = aux_kind(key, name) {
            expire_output(body, id, kind, palette)
                .map_err(|error| format!("aux.{name}: {error}"))?;
        }
    }
    Ok(())
}

fn motion_block(
    profile: &SkillProfile,
    key: SkillKey,
    motion: &SharedHumanoidMotion,
) -> Result<(), String> {
    action_clip(motion, "release", &profile.release)?;
    let playback = &profile.motion;
    in_range(playback.rate, 0.5..=2.0, "motion.rate")?;
    in_range(playback.start, 0.0..=0.8, "motion.start")?;
    in_range(playback.recast_rate, 0.5..=2.0, "motion.recast_rate")?;
    let derived = derived_phase(key);
    if let Some(phase) = playback.phase
        && phase != MotionPhase::Instant
        && phase != derived
    {
        return Err(format!(
            "motion.phase {} is not the phase of this skill ({})",
            phase.id(),
            derived.id()
        ));
    }
    if let Some(windup) = &profile.windup {
        pickable_clip(motion, "windup", windup)?;
        if derived == MotionPhase::Instant {
            return Err("`windup` needs a skill with a telegraph of its own".into());
        }
        if playback.phase == Some(MotionPhase::Instant) {
            return Err("`windup` needs a non-instant motion.phase".into());
        }
    } else if playback
        .phase
        .is_some_and(|phase| phase != MotionPhase::Instant)
    {
        // The release of such a row would wait for a telegraph with no pose to hold.
        return Err("a non-instant motion.phase needs a `windup`".into());
    }
    if playback.fit_windup {
        let fitted = profile
            .windup
            .as_ref()
            .and_then(|windup| motion.clips.get(windup))
            .is_some_and(|clip| !clip.looping)
            && key
                .modular()
                .is_some_and(|id| category::telegraph_secs(id).is_some());
        if !fitted {
            return Err(
                "motion.fit_windup needs a non-looping `windup` and a timed telegraph".into(),
            );
        }
    }
    if let Some(recast) = &playback.recast {
        if !key.modular().is_some_and(category::has_recast) {
            return Err("motion.recast needs a skill with a recast".into());
        }
        action_clip(motion, "motion.recast", recast)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum AccentOwner {
    Skill { key: SkillKey, windup: bool },
    Basic,
}

/// `rune_mark` circles the caster; a diamond or a star around a hero would read as a status.
fn accent_lead_legal(pattern: AccentPattern, lead: ParticleShape) -> bool {
    pattern != AccentPattern::RuneMark
        || matches!(
            lead,
            ParticleShape::Arc
                | ParticleShape::Chevron
                | ParticleShape::Crescent
                | ParticleShape::Ringlet
        )
}

/// A basic attack has no colour of its own: its particles take the class theme.
fn theme_slots(slots: Option<[PaletteSlot; 2]>, block: &str) -> Result<(), String> {
    if slots.is_some_and(|slots| !slots.contains(&PaletteSlot::Primary)) {
        Ok(())
    } else {
        Err(format!(
            "{block}.slots must be named without `primary` on a basic attack"
        ))
    }
}

fn cast_accent(accent: &CastAccent, owner: AccentOwner) -> Result<(), String> {
    let pattern = accent.pattern;
    // The block is `cast` on a skill row and `accent` on a basic attack.
    let block = match owner {
        AccentOwner::Skill { .. } => "cast",
        AccentOwner::Basic => "accent",
    };
    if accent.count.is_some_and(|count| !(1..=8).contains(&count)) {
        return Err(format!("{block}.count must be within 1..=8"));
    }
    in_range(accent.scale, 0.4..=2.0, &format!("{block}.scale"))?;
    if pattern.base_extent() * accent.scale > 2.0 + 1e-4 {
        return Err(format!(
            "{block}.scale lets {} reach beyond 2.0 units",
            pattern.id()
        ));
    }
    in_range(accent.lifetime, 0.12..=0.5, &format!("{block}.lifetime"))?;
    let lead = accent.lead();
    if lead.is_some_and(|lead| !accent_lead_legal(pattern, lead)) {
        return Err(format!(
            "{block}.shape: {} needs a lead of arc, chevron, crescent or ringlet",
            pattern.id()
        ));
    }
    match owner {
        AccentOwner::Basic => {
            for (field, set) in [
                ("recast", accent.recast.is_some()),
                ("move", accent.movement.is_some()),
                ("link", accent.link.is_some()),
                ("area", accent.area),
                ("recast_marker", accent.recast_marker.is_some()),
            ] {
                if set {
                    return Err(format!("a basic attack accent has no `{field}`"));
                }
            }
            if pattern == AccentPattern::StrikeLine {
                return Err("strike_line needs a skill that strikes to its own effect".into());
            }
            theme_slots(accent.slots, block)
        }
        AccentOwner::Skill { key, windup } => {
            let id = key.modular();
            if windup && !pattern.is_charge() {
                return Err(format!(
                    "cast.pattern {} implies an immediate release; a row with `windup` needs a charge pattern",
                    pattern.id()
                ));
            }
            let strikes_own_effect = id.is_some_and(category::own_effect_strike);
            if (pattern == AccentPattern::StrikeLine
                || accent.recast == Some(AccentPattern::StrikeLine))
                && !strikes_own_effect
            {
                return Err("strike_line needs a skill that strikes to its own effect".into());
            }
            if category::self_heal(key)
                && (pattern == AccentPattern::ShieldFlash || lead == Some(ParticleShape::Kite))
            {
                return Err("a self heal may not use a shield shape".into());
            }
            let recast = id.is_some_and(category::has_recast);
            if let Some(pattern) = accent.recast {
                if !recast {
                    return Err("cast.recast needs a skill with a recast".into());
                }
                // A recast accent always draws its pattern's default lead.
                if pattern
                    .default_lead()
                    .is_some_and(|lead| !accent_lead_legal(pattern, lead))
                {
                    return Err(format!(
                        "cast.recast: the default lead of {} is not legal",
                        pattern.id()
                    ));
                }
            }
            if accent.recast_marker.is_some() && !recast {
                return Err("cast.recast_marker needs a skill with a recast".into());
            }
            if accent.movement.is_some()
                && !id.is_some_and(|id| {
                    category::movement_capable(id, false) || category::movement_capable(id, true)
                })
            {
                return Err("cast.move needs a skill that can move its caster".into());
            }
            if accent.link.is_some() {
                if !category::can_damage(key) {
                    return Err("cast.link needs a skill that can deal damage".into());
                }
                if category::travelling_body(key) && !id.is_some_and(category::recast_instant_hit) {
                    return Err("cast.link would precede the hit of a travelling body".into());
                }
            }
            if accent.area && !id.is_some_and(|id| category::AREA_FLASH_SIGNED_OFF.contains(&id)) {
                return Err("cast.area is not signed off for this skill".into());
            }
            Ok(())
        }
    }
}

/// `owner` is the skill row, or `None` for a basic attack.
fn impact_recipe(impact: &ImpactRecipe, owner: Option<SkillKey>) -> Result<(), String> {
    if impact.count.is_some_and(|count| !(1..=12).contains(&count)) {
        return Err("impact.count must be within 1..=12".into());
    }
    in_range(impact.scale, 0.3..=2.0, "impact.scale")?;
    in_range(impact.lifetime, 0.08..=1.2, "impact.lifetime")?;
    if matches!(impact.kind, ImpactKind::ChainSnap | ImpactKind::FacetPop)
        && impact.lead() == ParticleShape::Arc
    {
        return Err(format!(
            "impact.shape: arc may not lead {}",
            impact.kind.id()
        ));
    }
    if impact.kind == ImpactKind::Blast && !owner.is_some_and(category::area_damage) {
        return Err("impact.kind blast needs a skill with area damage".into());
    }
    if impact.kind == ImpactKind::PierceThrough && !owner.is_some_and(category::pierces) {
        return Err("impact.kind pierce_through needs a skill that pierces".into());
    }
    match owner {
        Some(_) => Ok(()),
        None => theme_slots(impact.slots, "impact"),
    }
}

fn speed_step(speed: f32) -> bool {
    let steps = speed * 20.0;
    speed.is_finite()
        && (0.70 - 1e-4..=1.40 + 1e-4).contains(&speed)
        && (steps - steps.round()).abs() < 1e-3
}

fn sound_cue(cue: &SoundCue) -> Result<(), String> {
    if !speed_step(cue.speed) {
        return Err("speed must be 0.70..=1.40 in steps of 0.05".into());
    }
    in_range(cue.gain, 0.2..=1.0, "gain")?;
    if cue.notes.len() > 2 {
        return Err("at most 2 extra notes".into());
    }
    for note in &cue.notes {
        if note.delay_ms > 240 {
            return Err("a note may be delayed by at most 240 ms".into());
        }
        if !speed_step(note.speed) {
            return Err("note speed must be 0.70..=1.40 in steps of 0.05".into());
        }
        in_range(note.gain, 0.2..=1.0, "note gain")?;
    }
    Ok(())
}

fn sound_block(
    sound: &Sound,
    key: SkillKey,
    phase: MotionPhase,
    damaging: bool,
) -> Result<(), String> {
    for (slot, cue) in [
        ("cast", &sound.cast),
        ("release", &sound.release),
        ("impact", &sound.impact),
        ("recast", &sound.recast),
    ] {
        let Some(cue) = cue else { continue };
        sound_cue(cue).map_err(|error| format!("sound.{slot}: {error}"))?;
        if cue.base == AudioBase::Bluff && key != SkillKey::Modular(SkillId::DaggerBluff) {
            return Err(format!("sound.{slot}: bluff belongs to dagger_bluff"));
        }
    }
    if sound.release.is_some() && phase == MotionPhase::Instant {
        return Err("sound.release needs a non-instant phase".into());
    }
    if sound.recast.is_some() && !key.modular().is_some_and(category::has_recast) {
        return Err("sound.recast needs a skill with a recast".into());
    }
    if sound.impact.is_some() && !damaging {
        return Err("sound.impact needs a skill that can deal damage".into());
    }
    Ok(())
}

/// The secondary object of the skill that an `aux` key names.
fn aux_kind(key: SkillKey, name: &str) -> Option<(SkillId, EffectVisualKind)> {
    let id = key.modular()?;
    category::aux_kinds(id)
        .iter()
        .find(|kind| category::kind_id(**kind) == name)
        .map(|kind| (id, *kind))
}

/// What a body is drawn for: the effect of the first cast, or a secondary object.
#[derive(Clone, Copy)]
enum Binding {
    Own,
    Aux(EffectVisualKind),
}

fn layout_legal(layout: SatelliteLayout, archetype: Archetype) -> bool {
    use Archetype as A;
    match layout {
        SatelliteLayout::Orbit | SatelliteLayout::Column => {
            matches!(archetype, A::Zone | A::Orbiter | A::Prop)
        }
        SatelliteLayout::Halo => matches!(archetype, A::Traveller | A::Orbiter),
        SatelliteLayout::Helix => archetype == A::Traveller,
        SatelliteLayout::QuadX => matches!(archetype, A::Zone | A::Prop | A::Traveller),
        SatelliteLayout::Rim => matches!(archetype, A::Zone | A::Prop | A::Cage),
        SatelliteLayout::Line => matches!(archetype, A::Lane | A::Wall),
        SatelliteLayout::Stagger => archetype == A::Lane,
        SatelliteLayout::Fan => matches!(archetype, A::Sector | A::Traveller),
    }
}

fn body_block(body: &Body, id: SkillId, binding: Binding) -> Result<(), String> {
    use Archetype as A;
    let archetype = body.archetype;
    // A body bound to several kinds of one instance shares their stage rule.
    let (fits, rule, runtime_id, orb) = match binding {
        Binding::Own => (
            geometry::body_fits(archetype, id),
            category::own_kinds(id)
                .first()
                .map_or(StageRule::Active, |kind| category::stage_rule(id, *kind)),
            true,
            false,
        ),
        Binding::Aux(kind) => (
            geometry::archetype_fits(archetype, id, kind),
            category::stage_rule(id, kind),
            !category::unstable_id(kind),
            kind == EffectVisualKind::Orb,
        ),
    };
    // The kinds a body is bound to share their radius.
    let kind = match binding {
        Binding::Own => category::own_kinds(id).first().copied(),
        Binding::Aux(kind) => Some(kind),
    };
    let Some(kind) = kind.filter(|_| fits) else {
        return Err(format!(
            "archetype {} cannot show this effect",
            archetype.id()
        ));
    };
    let radius = geometry::replicated_radius(id, kind);
    if body.core.is_none() && body.model.is_none() && body.satellites.is_none() {
        return Err("needs `core`, `model` or `satellites`".into());
    }
    let moving = bodies::moving(archetype);
    let on_ground = bodies::altitude(body) == Altitude::Ground;

    let parts = [("core", &body.core), ("shell", &body.shell)]
        .into_iter()
        .filter_map(|(name, part)| {
            part.as_ref()
                .map(|part| (name, part.mesh, part.size, part.behave))
        })
        .chain(
            body.satellites
                .as_ref()
                .map(|part| ("satellites", part.mesh, part.size, part.behave)),
        );
    for (name, mesh, size, behave) in parts {
        if size
            .iter()
            .any(|extent| !extent.is_finite() || *extent <= 0.0)
        {
            return Err(format!("{name}.size must be finite and positive"));
        }
        let [lateral, vertical, along] = size;
        if moving {
            // Metres. The part is no wider than the hit circle it flies with.
            let widest = bodies::flight_width(radius);
            if lateral > widest + 1e-4 {
                return Err(format!(
                    "{name}.size is {lateral} wide (at most {widest} for a radius of {radius})"
                ));
            }
            if vertical > 2.0 || along > 3.0 {
                return Err(format!("{name}.size may be at most 2.0 high and 3.0 long"));
            }
            if on_ground && lateral.max(along) * 0.5 > radius + 1e-4 {
                return Err(format!(
                    "{name}.size: a part on the ground may reach at most the radius ({radius}) from the centre"
                ));
            }
        } else {
            // Multiples of the replicated radius, as full extents. A part that turns end
            // over end sweeps its height across the ground as well.
            let turns_over = matches!(behave, Behaviour::Tumble | Behaviour::Gyro);
            let halved = lateral.max(along) <= 1.0 && !(turns_over && vertical > 1.0);
            // A `core` or a `shell` stands in the middle of a circle, so it may be larger:
            // as far as the inner edge of the boundary line, measured on its own mesh.
            let centred = name != "satellites"
                && matches!(archetype, A::Zone | A::Prop)
                && bodies::centred_reach(
                    &Part {
                        mesh,
                        slot: PaletteSlot::Primary,
                        size,
                        behave,
                    },
                    !on_ground,
                    bodies::plate_lead(archetype, kind, mesh),
                ) <= bodies::MARKER_REACH + 1e-4;
            if !(halved || centred) {
                return Err(format!(
                    "{name}.size leaves the boundary: an extent on the ground is at most 1.0 of the radius, and a core or a shell in the middle of a circle reaches at most {} of it",
                    bodies::MARKER_REACH
                ));
            }
        }
        match behave {
            Behaviour::Gyro if !orb => {
                return Err(format!("{name}: gyro belongs to the orb"));
            }
            Behaviour::OnlyAfterRenew
                if !(moving && runtime_id)
                    || matches!(mesh, Silhouette::Torus | Silhouette::Ring) =>
            {
                return Err(format!(
                    "{name}: only_after_renew needs a travelling body with a runtime id and a part that is not a ring"
                ));
            }
            Behaviour::RiseOnSpawn
                if !matches!(binding, Binding::Own)
                    || category::spawn_lifetime_secs(id).is_none() =>
            {
                return Err(format!(
                    "{name}: rise_on_spawn needs a skill with a fixed spawn lifetime"
                ));
            }
            _ => {}
        }
    }
    if let Some(satellites) = &body.satellites {
        if !(1..=8).contains(&satellites.count) {
            return Err("satellites.count must be within 1..=8".into());
        }
        if !layout_legal(satellites.layout, archetype) {
            return Err(format!(
                "satellites.layout {} is not legal for {}",
                satellites.layout.id(),
                archetype.id()
            ));
        }
    }
    if body.trail != Trail::None && !moving {
        return Err("trail needs a traveller or an orbiter".into());
    }
    if let Some(scale) = body.trail_scale {
        if body.trail == Trail::None {
            return Err("trail_scale needs a trail".into());
        }
        in_range(scale, bodies::TRAIL_SCALES, "trail_scale")?;
        // A trail part is held to the hit circle it follows, like every part in flight.
        let width = bodies::trail_width(body.trail, radius, scale);
        let widest = bodies::flight_width(radius);
        if width > widest + 1e-4 || (on_ground && width * 0.5 > radius + 1e-4) {
            return Err(format!(
                "trail_scale makes the trail {width:.2} wide (at most {:.2} for a radius of {radius})",
                if on_ground {
                    widest.min(2.0 * radius)
                } else {
                    widest
                }
            ));
        }
    }
    if body.fill.is_some() && moving {
        return Err("fill needs an area archetype".into());
    }
    if let Some(strength) = body.fill_strength {
        if !bodies::fills(body) {
            return Err("fill_strength needs a body with a fill".into());
        }
        in_range(strength, bodies::FILL_STRENGTHS, "fill_strength")?;
    }
    if body.model.is_some() && !matches!(archetype, A::Traveller | A::Orbiter | A::Prop | A::Zone) {
        return Err(format!("model is not legal for {}", archetype.id()));
    }
    let marker_legal = match body.marker {
        Marker::None => true,
        Marker::RemainingRing => {
            matches!(archetype, A::Zone | A::Prop | A::Cage) && rule != StageRule::Fuse
        }
        Marker::ArmingPips => rule == StageRule::ArmedGate,
        Marker::FillToEdge => matches!(rule, StageRule::KindGate | StageRule::Fuse),
        Marker::OwnerTether => orb || (archetype == A::Traveller && runtime_id),
    };
    if !marker_legal {
        return Err(format!(
            "marker {} is not legal for this body",
            body.marker.id()
        ));
    }
    let expire_legal = match body.expire {
        ExpireKind::None => true,
        // A travelling body or a positional id ends without a classified one-shot.
        _ if moving || !runtime_id => false,
        ExpireKind::Fade => matches!(archetype, A::Zone | A::Wall | A::Cage | A::Prop | A::Lane),
        ExpireKind::Crumble => matches!(archetype, A::Wall | A::Cage | A::Prop),
        ExpireKind::Discharge => rule == StageRule::Fuse,
        ExpireKind::Detonate => category::detonates(id),
    };
    if !expire_legal {
        return Err(format!(
            "expire {} is not legal for this body",
            body.expire.id()
        ));
    }
    // The most parts the body can have: over its boundary at full length.
    let def = shared::loadout::skill(id);
    let shape = geometry::boundary_shape(
        id,
        kind,
        &probe_effect(id, kind, radius, def.ability.cast_range.max(1.0)),
    );
    let parts = bodies::part_total(body, &shape);
    let most = if def.ability.base_cooldown_secs >= bodies::LONG_COOLDOWN_SECS {
        bodies::MAX_PARTS_LONG_COOLDOWN
    } else {
        bodies::MAX_PARTS
    };
    if parts > most {
        return Err(format!(
            "draws {parts} mesh parts (at most {most} for this cooldown)"
        ));
    }
    Ok(())
}

fn basic_row(
    config: &SkillPresentation,
    class: &str,
    basic: &BasicProfile,
    motion: &SharedHumanoidMotion,
    nested: bool,
) -> Result<(), String> {
    let class = HeroClass::from_id(class).ok_or("unknown class")?;
    if !(1..=2).contains(&basic.motions.len()) {
        return Err("`motions` needs one or two clips".into());
    }
    in_range(basic.rate, 0.5..=2.0, "rate")?;
    in_range(basic.start, 0.0..=0.8, "start")?;
    for name in &basic.motions {
        action_clip(motion, "motions", name)?;
        if let Some(late) = late_contact(motion, "motions", name, basic.rate, basic.start) {
            return Err(late);
        }
    }
    if let Some(accent) = &basic.accent {
        cast_accent(accent, AccentOwner::Basic)?;
    }
    if let Some(impact) = &basic.impact {
        impact_recipe(impact, None)?;
    }
    if let Some(cue) = &basic.sound {
        sound_cue(cue).map_err(|error| format!("sound: {error}"))?;
        if cue.base == AudioBase::Bluff {
            return Err("sound: bluff belongs to dagger_bluff".into());
        }
    }
    if let Some(rockets) = &basic.rockets {
        if nested {
            return Err("`rockets` cannot nest".into());
        }
        let repeater = shared::loadout::preset_for_class(class)
            .is_some_and(|kit| kit.attack_profile() == shared::loadout::AttackProfileId::Repeater);
        if !repeater {
            return Err("`rockets` needs the repeater attack profile".into());
        }
        basic_row(config, class.id(), rockets, motion, true)
            .map_err(|error| format!("rockets: {error}"))?;
    }
    // `themes` was checked to cover every class.
    let Some(palette) = config.themes.get(class.id()).map(Palette::of_class) else {
        return Ok(());
    };
    if let Some(accent) = &basic.accent {
        accent_output(accent, &palette, None, "accent")?;
    }
    if let Some(impact) = &basic.impact {
        impact_output(impact, &palette, false)?;
    }
    Ok(())
}

/// The orb's replicated skill flips between the two skills that order it, so both rows must
/// describe the same object.
fn shared_aux(config: &SkillPresentation) -> Result<(), String> {
    let mut declared: BTreeMap<(&str, &str), (&str, &Body)> = BTreeMap::new();
    for (id, profile) in &config.skills {
        let Some(key) = SkillKey::from_id(id) else {
            continue;
        };
        for (name, body) in &profile.aux {
            if !aux_kind(key, name).is_some_and(|(_, kind)| category::unstable_id(kind)) {
                continue;
            }
            match declared.get(&(profile.home.as_str(), name.as_str())) {
                Some((other, first)) if *first != body => {
                    return Err(format!(
                        "{id}: aux.{name} must equal the body declared by {other}"
                    ));
                }
                Some(_) => {}
                None => {
                    declared.insert((&profile.home, name), (id, body));
                }
            }
        }
    }
    Ok(())
}
