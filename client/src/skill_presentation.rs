//! Packaged skill-owned presentation. Recipes select skills; buttons do not select motions.
//! This module never changes movement, damage, cooldowns or authoritative geometry.
pub(crate) mod accents;
pub(crate) mod bodies;
pub(crate) mod cast;
mod category;
mod effects;
#[cfg(feature = "qa")]
pub(crate) mod evidence;
pub(crate) mod geometry;
pub(crate) mod impacts;
mod schema;
mod signature;
pub(crate) mod stage;
pub(crate) mod status;
pub(crate) mod vocab;

pub(crate) use schema::{BasicProfile, SkillProfile, SoundCue, Theme};

/// Evidence and scene readiness for each replicated world effect, independent of its owner.
#[cfg(feature = "qa")]
#[derive(bevy::prelude::Component)]
pub(crate) struct SkillEffectVisual {
    pub id: u64,
    pub model_ready: bool,
}

/// Evidence of an effect drawn through the `body` of its row: the archetype, the boundary
/// the engine drew for it, and its mesh parts.
#[cfg(feature = "qa")]
#[derive(bevy::prelude::Component)]
pub(crate) struct SkillBodyVisual {
    pub archetype: vocab::Archetype,
    pub boundary: geometry::GeoShape,
    /// Boundary parts.
    pub engine: usize,
    /// Authored parts the row gives the body: core, shell, satellites, trail and model.
    pub authored: usize,
    /// Trail parts shown in this frame.
    pub trail: usize,
    /// Authored parts the part budget hid in this frame.
    pub budget_hidden: usize,
}

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::Deserialize;
use shared::HeroClass;
use shared::loadout::{EffectVisualKind, LoadoutState, SkillEffectState, SkillId, WeaponMode};
use std::collections::BTreeMap;

use category::SkillKey;
use vocab::MotionPhase;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EffectStyle {
    Lance,
    Aegis,
    Field,
    Beam,
    Repeater,
    Shock,
    Trap,
    Rocket,
    Orb,
    Hook,
    Lantern,
    Pillar,
    Colossus,
    Wall,
    Cage,
    Fissure,
    Cone,
    Slash,
    Pulse,
    Needle,
    Ember,
}

/// The registry as packaged: raw rows only. Resolved colours and handles live in consumers.
#[derive(Resource, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillPresentation {
    schema_version: u32,
    /// Class colours, one entry per hero class.
    themes: BTreeMap<String, Theme>,
    /// Basic attacks by class; a class without a row keeps the built-in motion table.
    basic_attacks: BTreeMap<String, BasicProfile>,
    skills: BTreeMap<String, SkillProfile>,
}

/// Where the active registry came from, so evidence can tell the packaged file from the
/// copy compiled into the client.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SkillPresentationOrigin {
    #[default]
    Embedded,
    /// The packaged file, with the FNV-1a hash of its bytes.
    Packaged { fnv64: u64 },
}

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl SkillPresentation {
    fn parse(json: &str) -> Result<Self, String> {
        if json.len() > schema::MAX_BYTES {
            return Err("Skill presentation exceeds 256 KiB".into());
        }
        #[derive(Deserialize)]
        struct Version {
            schema_version: u32,
        }
        let config: Self = serde_json::from_str(json).map_err(|error| {
            // The rows of an outdated file fail on fields they cannot have; name the cause.
            match serde_json::from_str::<Version>(json) {
                Ok(Version { schema_version }) if schema_version != schema::SCHEMA_VERSION => {
                    schema::unsupported_version(schema_version)
                }
                _ => error.to_string(),
            }
        })?;
        schema::validate(&config)?;
        Ok(config)
    }
    pub(crate) fn rows(&self) -> impl Iterator<Item = (&str, &SkillProfile)> {
        self.skills
            .iter()
            .map(|(id, profile)| (id.as_str(), profile))
    }
    pub(crate) fn row(&self, id: &str) -> Option<&SkillProfile> {
        self.skills.get(id)
    }
    pub(crate) fn theme(&self, class: HeroClass) -> Option<&Theme> {
        self.themes.get(class.id())
    }
    pub(crate) fn basic(&self, class: HeroClass) -> Option<&BasicProfile> {
        self.basic_attacks.get(class.id())
    }
    /// Accepted recipes take precedence; legacy ability IDs share the same registry.
    pub(crate) fn action_profile(
        &self,
        class: shared::HeroClass,
        loadout: Option<&LoadoutState>,
        slot: u8,
    ) -> Option<&SkillProfile> {
        let skills = crate::equipped_skills::resolve_state(class, loadout)?;
        self.skills
            .get(skills.ability(shared::SkillSlot::from_index(slot)?).id)
    }
    pub(crate) fn profile(&self, skill: SkillId) -> Option<&SkillProfile> {
        self.skills.get(skill.id())
    }
    /// The body a row gives one replicated effect: `body` for the effect of the first cast,
    /// else the `aux` entry of its kind.
    pub(crate) fn body_for(&self, effect: &SkillEffectState) -> Option<&schema::Body> {
        let profile = self.profile(effect.skill)?;
        if category::own_kinds(effect.skill).contains(&effect.kind) {
            profile.body.as_ref()
        } else {
            profile.aux.get(category::kind_id(effect.kind))
        }
    }
    /// What the row of an accepted action draws with its particles, resolved with the theme
    /// of its class. A basic attack without a row resolves to nothing.
    pub(crate) fn look(&self, key: cast::CastKey) -> Option<Look<'_>> {
        match key {
            cast::CastKey::Skill(key) => {
                let row = self.row(key.id())?;
                Some(Look {
                    palette: accents::Palette::of(row, self.theme(key.home())?),
                    accent: row.cast.as_ref(),
                    impact: row.impact.as_ref(),
                })
            }
            cast::CastKey::Basic(class) => {
                let row = self.basic(class)?;
                Some(Look {
                    palette: accents::Palette::of_class(self.theme(class)?),
                    accent: row.accent.as_ref(),
                    impact: row.impact.as_ref(),
                })
            }
        }
    }
    /// Whether the accent of an accepted action is drawn from its row: a skill row with
    /// `cast`, or a basic attack whose row has an `accent`. Every other action keeps the
    /// built-in accent.
    pub(crate) fn themed_cast(
        &self,
        class: HeroClass,
        loadout: Option<&LoadoutState>,
        slot: u8,
    ) -> bool {
        cast::CastKey::of(class, loadout, slot)
            .and_then(|key| self.look(key))
            .is_some_and(|look| look.accent.is_some())
    }
    /// Size of the active registry, recorded as capture evidence.
    #[cfg(feature = "qa")]
    pub(crate) fn profile_count(&self) -> usize {
        self.skills.len()
    }
}

/// The accepted recipe, not the hero class or a skill's default catalogue slot.
pub(crate) fn equipped_skill(
    class: shared::HeroClass,
    loadout: Option<&LoadoutState>,
    slot: u8,
) -> Option<SkillId> {
    let skills = crate::equipped_skills::resolve_state(class, loadout)?;
    skills
        .skill(shared::SkillSlot::from_index(slot)?)
        .map(|skill| skill.id)
}

/// The particle blocks of one row and the colours they are drawn in.
pub(crate) struct Look<'a> {
    pub palette: accents::Palette,
    /// The `cast` block of a skill row, or the `accent` of a basic attack.
    pub accent: Option<&'a accents::CastAccent>,
    pub impact: Option<&'a impacts::ImpactRecipe>,
}

/// What a hero's body is asked to play.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MotionCue {
    pub motion: String,
    /// The pose is held against a telegraph instead of running out.
    pub hold: bool,
    /// Multiplies the playback speed of the clip.
    pub rate: f32,
    /// Fraction of the clip skipped at its start.
    pub start: f32,
    /// The clip repeats for as long as it is played.
    pub looping: bool,
}

/// The playback rates a windup fitted to its telegraph may take.
const FITTED_RATES: (f32, f32) = (0.5, 2.0);

impl MotionCue {
    /// A clip played once from its trigger: a release, a recast or a basic attack.
    fn action(motion: &str, rate: f32, start: f32) -> Self {
        Self {
            motion: motion.into(),
            hold: false,
            rate,
            start,
            looping: false,
        }
    }

    fn release(profile: &SkillProfile) -> Self {
        Self::action(&profile.release, profile.motion.rate, profile.motion.start)
    }

    /// The windup of a row, held against the telegraph of its skill: a loop repeats under
    /// it, and a clip the row fits to it spans it once.
    fn windup(profile: &SkillProfile, skill: SkillId) -> Option<Self> {
        let motion = profile.windup.as_ref()?;
        let clip = crate::humanoid::SharedHumanoidMotion::embedded()
            .ok()
            .and_then(|library| library.clips.get(motion));
        let fitted = clip
            .zip(category::telegraph_secs(skill))
            .filter(|(_, telegraph)| profile.motion.fit_windup && *telegraph > 0.0)
            .map(|(clip, telegraph)| {
                (clip.duration / telegraph).clamp(FITTED_RATES.0, FITTED_RATES.1)
            });
        Some(Self {
            motion: motion.clone(),
            hold: true,
            rate: fitted.unwrap_or(1.0),
            start: 0.0,
            looping: clip.is_some_and(|clip| clip.looping),
        })
    }
}

/// What the action in one slot asks of the body, with the effects the hero has replicated.
/// A basic attack gives the first motion of its class; `motion_plan` alternates them.
pub(crate) fn motion_cue(
    registry: &SkillPresentation,
    class: shared::HeroClass,
    loadout: Option<&LoadoutState>,
    slot: u8,
    owner: u64,
    effects: &[SkillEffectState],
) -> Option<MotionCue> {
    if slot == shared::BASIC_ATTACK_ACTION_SLOT {
        return basic_cue(registry, class, loadout, 1);
    }
    let skill = equipped_skill(class, loadout, slot)?;
    let profile = registry.profile(skill)?;
    if profile.windup.is_none() {
        return Some(MotionCue::release(profile));
    }
    let own = effects
        .iter()
        .filter(|effect| owner != 0 && effect.owner_id == owner && effect.skill == skill)
        .find_map(|effect| telegraph_cue(profile, effect));
    match profile.phase(SkillKey::Modular(skill)) {
        // A vanished warning is not proof that a beam fired (cancel/fog/round change).
        MotionPhase::WarnFire | MotionPhase::Instant => own,
        // A fuse burns from the accepted cast on. Whoever plays the windup bounds a hold
        // whose telegraph it never sees.
        MotionPhase::Fuse => MotionCue::windup(profile, skill),
        // The stance lasts while the hero parries.
        MotionPhase::Parry => own.or_else(|| {
            loadout
                .filter(|loadout| loadout.parrying)
                .and_then(|_| MotionCue::windup(profile, skill))
        }),
    }
}

/// The motion of a basic attack: the row of the class of the kit's core (rule E-13), for a
/// repeater in rocket mode its `rockets` entry. Two motions alternate, the first on odd
/// action sequences. A class without a row keeps the built-in table.
fn basic_cue(
    registry: &SkillPresentation,
    class: shared::HeroClass,
    loadout: Option<&LoadoutState>,
    sequence: u64,
) -> Option<MotionCue> {
    let kit = crate::equipped_skills::resolve_state(class, loadout)?.resolved();
    if let Some(row) = registry.basic(kit.map_or(class, |kit| kit.core().class())) {
        let rockets = loadout.is_some_and(|loadout| loadout.weapon_mode == WeaponMode::Rockets);
        let row = row.rockets.as_deref().filter(|_| rockets).unwrap_or(row);
        let turn = if sequence % 2 == 1 {
            0
        } else {
            row.motions.len().saturating_sub(1)
        };
        return row
            .motions
            .get(turn)
            .map(|motion| MotionCue::action(motion, row.rate, row.start));
    }
    let motion = match kit {
        Some(kit) if kit.core() == shared::loadout::CoreId::Adventurer => "dagger_stab",
        Some(kit)
            if kit.attack_profile() == shared::loadout::AttackProfileId::Repeater
                || kit.core() == shared::loadout::CoreId::Riftshot =>
        {
            "pistol_shoot"
        }
        Some(kit) if kit.attack_profile() == shared::loadout::AttackProfileId::LightBolt => "cast",
        None if class == shared::HeroClass::Ranger => "pistol_shoot",
        None if matches!(class, shared::HeroClass::Mage | shared::HeroClass::Cleric) => "cast",
        _ => return None,
    };
    Some(MotionCue::action(motion, 1.0, 0.0))
}

/// What a row with a windup asks of its caster while this effect of the skill is
/// replicated. A warning is the held windup and, once it is the beam or the bolt it
/// announced, the release. A fuse and a parry are the held windup for as long as their
/// effect is replicated; the stage tracker names the moment they fire.
fn telegraph_cue(profile: &SkillProfile, effect: &SkillEffectState) -> Option<MotionCue> {
    profile.windup.as_ref()?;
    let windup = || MotionCue::windup(profile, effect.skill);
    match profile.phase(SkillKey::Modular(effect.skill)) {
        MotionPhase::WarnFire => match effect.kind {
            EffectVisualKind::BeamWarning => windup(),
            EffectVisualKind::Beam | EffectVisualKind::Bolt => Some(MotionCue::release(profile)),
            _ => None,
        },
        MotionPhase::Fuse | MotionPhase::Parry => category::own_kinds(effect.skill)
            .contains(&effect.kind)
            .then(windup)
            .flatten(),
        MotionPhase::Instant => None,
    }
}

/// The release of a skill whose telegraph fires without a change of kind.
pub(crate) fn release_cue(registry: &SkillPresentation, skill: SkillId) -> Option<MotionCue> {
    registry.profile(skill).map(MotionCue::release)
}

/// What decides the motion of a hero's body in one frame.
#[derive(Clone, Copy)]
pub(crate) struct MotionInputs<'a> {
    pub registry: &'a SkillPresentation,
    pub class: shared::HeroClass,
    pub loadout: Option<&'a LoadoutState>,
    /// The latest accepted action: its slot and its sequence.
    pub slot: u8,
    pub sequence: u64,
    /// The slot offered a recast before that action was accepted.
    pub recast: bool,
    pub owner: u64,
    pub effects: &'a [SkillEffectState],
}

/// The motion the body is asked for: the held windup of the hero's own telegraph, whatever
/// was accepted during it, else what the latest accepted action plays. That is the basic
/// attack of its turn, the recast clip of a recast, the cue of the slot, or the release of
/// a legacy ability.
pub(crate) fn motion_plan(inputs: &MotionInputs) -> Option<MotionCue> {
    let MotionInputs {
        registry,
        class,
        loadout,
        slot,
        sequence,
        recast,
        owner,
        effects,
    } = *inputs;
    let held = own_windup_cue(registry, class, loadout, owner, effects)
        .map(|(_, cue)| cue)
        .filter(|cue| cue.hold);
    if held.is_some() {
        return held;
    }
    if slot == shared::BASIC_ATTACK_ACTION_SLOT {
        return basic_cue(registry, class, loadout, sequence);
    }
    let profile = registry.action_profile(class, loadout, slot)?;
    if let Some(clip) = profile.motion.recast.as_ref().filter(|_| recast) {
        return Some(MotionCue::action(clip, profile.motion.recast_rate, 0.0));
    }
    motion_cue(registry, class, loadout, slot, owner, effects).or_else(|| {
        // A legacy ability has no telegraph: its release follows the accepted cast.
        profile
            .windup
            .is_none()
            .then(|| MotionCue::release(profile))
    })
}

/// The yaw a hero has when it looks along a telegraph, from the replicated geometry of the
/// effect; a telegraph without a direction (a ring) gives none.
pub(crate) fn telegraph_yaw(effect: &SkillEffectState) -> Option<f32> {
    (Vec2::from_array(effect.end) - Vec2::from_array(effect.position))
        .try_normalize()
        .map(|aim| shared::math::hero_yaw_towards(aim.x, aim.y))
}

/// A telegraph that fired is its release only this soon after the moment it fired. Later
/// that moment was not observed, and it is not played late: a wave travels for seconds.
fn just_fired(effect: &SkillEffectState) -> bool {
    category::tail_secs(effect.skill)
        .is_none_or(|tail| f64::from(tail - effect.remaining_secs) <= stage::MAX_GAP_SECS)
}

/// The cue of a hero's own telegraph, with the id of the effect that gives it, whatever
/// action the hero had accepted since: the held windup while a skill of its kit warns, the
/// release when that very effect has just fired. The newest effect speaks. The latest
/// action slot plays no part, because a basic attack or a recast accepted during the
/// warning replaces it and the cast edge itself is often never observed.
pub(crate) fn own_windup_cue(
    registry: &SkillPresentation,
    class: shared::HeroClass,
    loadout: Option<&LoadoutState>,
    owner: u64,
    effects: &[SkillEffectState],
) -> Option<(u64, MotionCue)> {
    if owner == 0 {
        return None;
    }
    let kit = crate::equipped_skills::resolve_state(class, loadout)?;
    effects
        .iter()
        .filter(|effect| effect.owner_id == owner)
        // Only a skill of the accepted kit moves its hero.
        .filter(|effect| {
            shared::SkillSlot::ALL
                .into_iter()
                .any(|slot| kit.skill(slot).is_some_and(|def| def.id == effect.skill))
        })
        .filter_map(|effect| {
            let cue = telegraph_cue(registry.profile(effect.skill)?, effect)?;
            (cue.hold || just_fired(effect)).then_some((effect.id, cue))
        })
        .max_by_key(|(id, _)| *id)
}

#[derive(Asset, TypePath)]
struct LoadedPresentation {
    registry: SkillPresentation,
    fnv64: u64,
}
impl LoadedPresentation {
    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            registry: SkillPresentation::parse(text)?,
            fnv64: fnv64(bytes),
        })
    }
}
#[derive(Default, TypePath)]
struct PresentationLoader;
impl AssetLoader for PresentationLoader {
    type Asset = LoadedPresentation;
    type Settings = ();
    type Error = std::io::Error;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        LoadedPresentation::from_bytes(&bytes).map_err(std::io::Error::other)
    }
    fn extensions(&self) -> &[&str] {
        &["skillfx"]
    }
}
#[derive(Resource, Default)]
struct Pending(Option<Handle<LoadedPresentation>>);
pub(crate) struct SkillPresentationPlugin;
impl Plugin for SkillPresentationPlugin {
    fn build(&self, app: &mut App) {
        // Invalid rebuilt defaults fall back to the existing geometric renderer.
        let registry = SkillPresentation::parse(include_str!("../assets/config/skills.skillfx"))
            .unwrap_or_else(|error| {
                error!("Embedded skill presentation is invalid: {error}");
                default()
            });
        app.insert_resource(registry)
            .init_resource::<SkillPresentationOrigin>()
            .init_resource::<Pending>()
            .init_asset::<LoadedPresentation>()
            .init_asset_loader::<PresentationLoader>()
            .add_message::<crate::net::SessionEvent>()
            .add_message::<cast::SkillCastObserved>()
            .add_message::<cast::MoveObserved>()
            .add_message::<stage::StageEvent>()
            .init_resource::<stage::EffectMemory>()
            .add_systems(
                Startup,
                |server: Res<AssetServer>, mut pending: ResMut<Pending>| {
                    pending.0 = Some(server.load("config/skills.skillfx"));
                },
            )
            .add_systems(
                Update,
                (
                    apply_config,
                    (cast::observe_skill_casts, stage::track_effects)
                        .chain()
                        .after(crate::net::ClientNetPipeline::InterpolateRemotePlayers),
                ),
            )
            .add_plugins((effects::SkillEffectsPlugin, status::StatusVisualsPlugin));
    }
}
fn apply_config(
    server: Res<AssetServer>,
    loaded: Res<Assets<LoadedPresentation>>,
    mut pending: ResMut<Pending>,
    mut config: ResMut<SkillPresentation>,
    mut origin: ResMut<SkillPresentationOrigin>,
) {
    let Some(handle) = &pending.0 else {
        return;
    };
    if let Some(value) = loaded.get(handle) {
        *config = value.registry.clone();
        *origin = SkillPresentationOrigin::Packaged { fnv64: value.fnv64 };
        pending.0 = None;
    } else if matches!(
        server.get_load_state(handle.id()),
        Some(bevy::asset::LoadState::Failed(_))
    ) {
        warn!("Skill presentation unavailable; keeping embedded fallback");
        pending.0 = None;
    }
}

#[cfg(test)]
mod tests;

/// Registries for the tests of the modules that draw from one.
#[cfg(test)]
impl SkillPresentation {
    /// The packaged file as it was before any content package: every skill keeps its
    /// legacy `effect` and names no block, and no class has a basic row. Tests of the
    /// paths a row without a block takes read this; the packaged file loses such rows
    /// family by family.
    pub(crate) fn unmigrated_config() -> serde_json::Value {
        let mut config: serde_json::Value =
            serde_json::from_str(include_str!("../assets/config/skills.skillfx")).unwrap();
        let v1: serde_json::Value =
            serde_json::from_str(include_str!("skill_presentation/fixtures/v1.skillfx")).unwrap();
        let mut skills = v1["skills"].clone();
        for (id, row) in skills.as_object_mut().unwrap() {
            row["home"] = category::SkillKey::from_id(id).unwrap().home().id().into();
        }
        config["skills"] = skills;
        config["basic_attacks"] = serde_json::json!({});
        config
    }
    pub(crate) fn unmigrated() -> Self {
        Self::parse(&Self::unmigrated_config().to_string()).unwrap()
    }
    /// The final data of the roster (`fixtures/target.skillfx`).
    pub(crate) fn target() -> Self {
        tests::target::target()
    }
    /// The final data with one edit, parsed as a packaged file is.
    pub(crate) fn target_with(edit: impl FnOnce(&mut serde_json::Value)) -> Self {
        let mut config =
            serde_json::from_str(include_str!("skill_presentation/fixtures/target.skillfx"))
                .unwrap();
        edit(&mut config);
        Self::parse(&config.to_string()).unwrap()
    }
}
