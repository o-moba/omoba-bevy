//! Packaged skill-owned presentation. Recipes select skills; buttons do not select motions.
//! This module never changes movement, damage, cooldowns or authoritative geometry.
mod effects;

/// Evidence and scene readiness for each replicated world effect, independent of its owner.
#[cfg(feature = "qa")]
#[derive(bevy::prelude::Component)]
pub(crate) struct SkillEffectVisual {
    pub id: u64,
    pub model_ready: bool,
}

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::Deserialize;
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
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillProfile {
    pub release: String,
    #[serde(default)]
    pub windup: Option<String>,
    pub effect: EffectStyle,
    pub color: [f32; 3],
    /// Linear radiance for small cores/rims, independent of broad field opacity.
    #[serde(default = "default_hdr_gain")]
    pub hdr_gain: f32,
}

fn default_hdr_gain() -> f32 {
    3.0
}

#[derive(Resource, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillPresentation {
    schema_version: u32,
    skills: BTreeMap<String, SkillProfile>,
}

impl SkillPresentation {
    fn parse(json: &str) -> Result<Self, String> {
        let config: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if config.schema_version != 1 || config.skills.len() > 64 {
            return Err("Unsupported skill presentation schema/size".into());
        }
        let motion = crate::humanoid::SharedHumanoidMotion::parse(include_str!(
            "../assets/animations/humanoid-motion-v1.json"
        ))?;
        for (id, profile) in &config.skills {
            if SkillId::from_id(id).is_none() {
                return Err(format!("Unknown skill {id}"));
            }
            for name in std::iter::once(&profile.release).chain(profile.windup.iter()) {
                if !motion.clips.contains_key(name) {
                    return Err(format!("Unknown motion {name}"));
                }
            }
            if profile
                .color
                .iter()
                .any(|c| !c.is_finite() || !(0.0..=1.0).contains(c))
            {
                return Err(format!("Invalid color for {id}"));
            }
            if !profile.hdr_gain.is_finite() || !(1.0..=8.0).contains(&profile.hdr_gain) {
                return Err(format!("Invalid HDR gain for {id} (expected 1..=8)"));
            }
        }
        Ok(config)
    }
    pub(crate) fn profile(&self, skill: SkillId) -> Option<&SkillProfile> {
        self.skills.get(skill.id())
    }
}

/// The accepted recipe, not the hero class or a skill's default catalogue slot.
pub(crate) fn equipped_skill(loadout: Option<&LoadoutState>, slot: u8) -> Option<SkillId> {
    loadout?
        .recipe
        .as_ref()?
        .skills
        .get(usize::from(slot))
        .copied()
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MotionCue {
    pub motion: String,
    pub hold: bool,
}

pub(crate) fn motion_cue(
    registry: &SkillPresentation,
    loadout: Option<&LoadoutState>,
    slot: u8,
    owner: u64,
    effects: &[SkillEffectState],
) -> Option<MotionCue> {
    if slot == shared::BASIC_ATTACK_ACTION_SLOT {
        let recipe = loadout?.recipe.as_ref()?;
        return (recipe.core.attack_profile() == shared::loadout::AttackProfileId::Repeater).then(
            || MotionCue {
                motion: "pistol_shoot".into(),
                hold: false,
            },
        );
    }
    let skill = equipped_skill(loadout, slot)?;
    let profile = registry.profile(skill)?;
    if let Some(windup) = &profile.windup {
        // A vanished warning is not proof that a beam fired (cancel/fog/round change).
        let phase = effects.iter().find(|e| {
            e.owner_id == owner
                && owner != 0
                && e.skill == skill
                && matches!(
                    e.kind,
                    EffectVisualKind::BeamWarning | EffectVisualKind::Beam
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
struct LoadedPresentation(SkillPresentation);
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
        let text = std::str::from_utf8(&bytes).map_err(std::io::Error::other)?;
        SkillPresentation::parse(text)
            .map(LoadedPresentation)
            .map_err(std::io::Error::other)
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
            .unwrap_or_default();
        app.insert_resource(registry)
            .init_resource::<Pending>()
            .init_asset::<LoadedPresentation>()
            .init_asset_loader::<PresentationLoader>()
            .add_systems(
                Startup,
                |server: Res<AssetServer>, mut pending: ResMut<Pending>| {
                    pending.0 = Some(server.load("config/skills.skillfx"));
                },
            )
            .add_systems(Update, apply_config)
            .add_plugins(effects::SkillEffectsPlugin);
    }
}
fn apply_config(
    server: Res<AssetServer>,
    loaded: Res<Assets<LoadedPresentation>>,
    mut pending: ResMut<Pending>,
    mut config: ResMut<SkillPresentation>,
) {
    let Some(handle) = &pending.0 else {
        return;
    };
    if let Some(value) = loaded.get(handle) {
        *config = value.0.clone();
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
