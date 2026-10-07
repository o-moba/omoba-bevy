//! Packaged skill-owned presentation. Recipes select skills; buttons do not select motions.
//! This module never changes movement, damage, cooldowns or authoritative geometry.
pub(crate) mod accents;
pub(crate) mod bodies;
pub(crate) mod cast;
mod category;
mod effects;
pub(crate) mod geometry;
pub(crate) mod impacts;
mod schema;
mod signature;
pub(crate) mod stage;
pub(crate) mod vocab;

pub(crate) use schema::{BasicProfile, SkillProfile, Theme};

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
use shared::loadout::{EffectVisualKind, LoadoutState, SkillEffectState, SkillId};
use std::collections::BTreeMap;

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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MotionCue {
    pub motion: String,
    pub hold: bool,
}

pub(crate) fn motion_cue(
    registry: &SkillPresentation,
    class: shared::HeroClass,
    loadout: Option<&LoadoutState>,
    slot: u8,
    owner: u64,
    effects: &[SkillEffectState],
) -> Option<MotionCue> {
    if slot == shared::BASIC_ATTACK_ACTION_SLOT {
        let equipped = crate::equipped_skills::resolve_state(class, loadout)?;
        let motion = match equipped.resolved() {
            Some(kit) if kit.core() == shared::loadout::CoreId::Adventurer => "dagger_stab",
            Some(kit)
                if kit.attack_profile() == shared::loadout::AttackProfileId::Repeater
                    || kit.core() == shared::loadout::CoreId::Riftshot =>
            {
                "pistol_shoot"
            }
            Some(kit) if kit.attack_profile() == shared::loadout::AttackProfileId::LightBolt => {
                "cast"
            }
            None if class == shared::HeroClass::Ranger => "pistol_shoot",
            None if matches!(class, shared::HeroClass::Mage | shared::HeroClass::Cleric) => "cast",
            _ => return None,
        };
        return Some(MotionCue {
            motion: motion.into(),
            hold: false,
        });
    }
    let skill = equipped_skill(class, loadout, slot)?;
    let profile = registry.profile(skill)?;
    if let Some(windup) = &profile.windup {
        // A vanished warning is not proof that a beam fired (cancel/fog/round change).
        let phase = effects.iter().find(|e| {
            e.owner_id == owner
                && owner != 0
                && e.skill == skill
                && matches!(
                    e.kind,
                    EffectVisualKind::BeamWarning | EffectVisualKind::Beam | EffectVisualKind::Bolt
                )
        })?;
        return Some(MotionCue {
            motion: if phase.kind == EffectVisualKind::BeamWarning {
                windup
            } else {
                &profile.release
            }
            .clone(),
            hold: phase.kind == EffectVisualKind::BeamWarning,
        });
    }
    Some(MotionCue {
        motion: profile.release.clone(),
        hold: false,
    })
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
            .add_plugins(effects::SkillEffectsPlugin);
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
    /// The packaged `skills.skillfx`.
    pub(crate) fn packaged() -> Self {
        Self::parse(include_str!("../assets/config/skills.skillfx")).unwrap()
    }
    /// The final data of the roster (`fixtures/target.skillfx`).
    pub(crate) fn target() -> Self {
        tests::target::target()
    }
}
