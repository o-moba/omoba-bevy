//! Versioned, local-only combat cosmetics. No field enters the simulation.
use std::collections::HashMap;

use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::Deserialize;
use shared::{HeroClass, combat::ProjectileStyle};

// Deliberately independent of the editable packaged file: even malformed user
// JSON must leave usable offline class visuals after recompiling the client.
const BUILT_INS: &str = r#"{"schema_version":1,"profiles":{
"standard":{"shape":"bolt","color":[0.8,0.9,1,1]},
"ranger_arrow":{"shape":"arrow","color":[0.65,1,0.72,1]},
"mage_arcane":{"shape":"arcane","color":[0.68,0.42,1,1]},
"cleric_holy":{"shape":"holy","color":[1,0.84,0.35,1]},
"warrior_crescent":{"shape":"crescent","color":[1,0.63,0.3,1]},
"caster_bolt":{"shape":"arcane","color":[0.4,0.95,1,1],"scale":0.7},
"tower_bolt":{"shape":"bolt","color":[1,0.43,0.24,1],"scale":1.4}},
"defaults":{"standard":"standard","arrow":"ranger_arrow","arcane":"mage_arcane","holy":"cleric_holy","crescent":"warrior_crescent","caster_bolt":"caster_bolt","tower_bolt":"tower_bolt"},
"classes":{"ranger":{"default":"ranger_arrow"},"mage":{"default":"mage_arcane"},"cleric":{"default":"cleric_holy"},"warrior":{"default":"warrior_crescent"}}}"#;
const CONFIG_PATH: &str = "config/combat_visuals.json";

/// Render container whose visible drawable descendants represent this network root.
#[derive(Component, Clone, Copy)]
pub(crate) struct ProjectilePresentationRoot {
    pub owner: Entity,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProjectileShape {
    Arrow,
    Arcane,
    Holy,
    Crescent,
    Bolt,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrailSettings {
    pub seconds: f32,
    pub width: f32,
    pub samples: usize,
}

impl Default for TrailSettings {
    fn default() -> Self {
        Self {
            seconds: 0.18,
            width: 0.08,
            samples: 8,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactSettings {
    pub color: [f32; 4],
    pub scale: f32,
    pub lifetime: f32,
}

impl Default for ImpactSettings {
    fn default() -> Self {
        Self {
            color: [1.0, 0.8, 0.5, 1.0],
            scale: 0.8,
            lifetime: 0.3,
        }
    }
}

impl ImpactSettings {
    pub fn color(&self) -> Color {
        Color::srgba(self.color[0], self.color[1], self.color[2], self.color[3])
    }
}

fn unit_scale() -> f32 {
    1.0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectileModel {
    pub path: String,
    #[serde(default)]
    pub scene: usize,
    #[serde(default = "unit_scale")]
    pub scale: f32,
    #[serde(default)]
    pub rotation_degrees: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectileSprite {
    pub path: String,
    pub frame_size: [u32; 2],
    pub columns: u32,
    pub rows: u32,
    #[serde(default)]
    pub first_frame: usize,
    pub frames: usize,
    pub fps: f32,
    pub world_height: f32,
}

impl ProjectileSprite {
    pub fn matches_image(&self, image: &Image) -> bool {
        image.width() == self.frame_size[0] * self.columns
            && image.height() == self.frame_size[1] * self.rows
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombatVisualProfile {
    #[serde(skip)]
    pub id: String,
    pub shape: ProjectileShape,
    pub color: [f32; 4],
    #[serde(default = "unit_scale")]
    pub scale: f32,
    #[serde(default)]
    pub trail: TrailSettings,
    #[serde(default)]
    pub impact: ImpactSettings,
    #[serde(default)]
    pub model: Option<ProjectileModel>,
    #[serde(default)]
    pub sprite: Option<ProjectileSprite>,
}

impl CombatVisualProfile {
    pub fn color(&self) -> Color {
        Color::srgba(self.color[0], self.color[1], self.color[2], self.color[3])
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimationAliases {
    pub idle: Vec<String>,
    pub walk: Vec<String>,
    pub run: Vec<String>,
    pub attack: Vec<String>,
    pub cast: Vec<String>,
    pub death: Vec<String>,
}

type ActionProfiles = HashMap<String, String>;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryConfig {
    schema_version: u32,
    #[serde(default)]
    profiles: HashMap<String, CombatVisualProfile>,
    #[serde(default)]
    defaults: HashMap<String, String>,
    #[serde(default)]
    classes: HashMap<String, ActionProfiles>,
    #[serde(default)]
    avatar_overrides: HashMap<String, ActionProfiles>,
    #[serde(default)]
    sprite_overrides: HashMap<String, ActionProfiles>,
    #[serde(default)]
    animation_aliases: HashMap<String, AnimationAliases>,
}

#[derive(Resource, Clone, Debug)]
pub struct CombatVisualRegistry {
    config: RegistryConfig,
    revision: u64,
}

impl Default for CombatVisualRegistry {
    fn default() -> Self {
        let mut registry = Self {
            config: serde_json::from_str(BUILT_INS).expect("embedded combat cosmetics JSON"),
            revision: 0,
        };
        registry
            .validate()
            .expect("embedded combat cosmetics are valid");
        registry
    }
}

fn in_range(value: f32, min: f32, max: f32) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}
fn valid_color(color: [f32; 4]) -> bool {
    color.into_iter().all(|value| in_range(value, 0.0, 1.0)) && color[3] >= 0.25
}
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 96 && !key.chars().any(char::is_control)
}

/// Asset-root-relative paths only: no URLs, labels, traversal or drive prefixes.
pub fn safe_asset_path(path: &str, extension: &str) -> bool {
    path.len() <= 200
        && path.ends_with(extension)
        && !path.starts_with('/')
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn style_key(style: ProjectileStyle) -> &'static str {
    match style {
        ProjectileStyle::Standard => "standard",
        ProjectileStyle::Arrow => "arrow",
        ProjectileStyle::Arcane => "arcane",
        ProjectileStyle::Holy => "holy",
        ProjectileStyle::Crescent => "crescent",
        ProjectileStyle::CasterBolt => "caster_bolt",
        ProjectileStyle::TowerBolt => "tower_bolt",
    }
}

fn action_key(slot: Option<u8>) -> &'static str {
    match slot {
        None | Some(255) => "basic",
        Some(0) => "q",
        Some(1) => "w",
        Some(2) => "e",
        Some(3) => "r",
        _ => "default",
    }
}

impl CombatVisualRegistry {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Partial manifests extend the embedded defaults; any invalid field rejects the manifest.
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > 256 * 1024 {
            return Err("cosmetic manifest exceeds 256 KiB".into());
        }
        let custom: RegistryConfig =
            serde_json::from_str(json).map_err(|error| error.to_string())?;
        if custom.schema_version != 1 {
            return Err("unsupported cosmetic schema_version".into());
        }
        let mut registry = Self::default();
        registry.config.profiles.extend(custom.profiles);
        registry.config.defaults.extend(custom.defaults);
        registry.config.classes.extend(custom.classes);
        registry
            .config
            .avatar_overrides
            .extend(custom.avatar_overrides);
        registry
            .config
            .sprite_overrides
            .extend(custom.sprite_overrides);
        registry
            .config
            .animation_aliases
            .extend(custom.animation_aliases);
        registry.validate()?;
        Ok(registry)
    }

    pub fn resolve_style(&self, style: ProjectileStyle) -> &CombatVisualProfile {
        let key = self
            .config
            .defaults
            .get(style_key(style))
            .map(String::as_str)
            .unwrap_or("standard");
        self.config
            .profiles
            .get(key)
            .unwrap_or_else(|| &self.config.profiles["standard"])
    }

    pub fn resolve(
        &self,
        class: Option<HeroClass>,
        style: ProjectileStyle,
        action_slot: Option<u8>,
        avatar: Option<&str>,
        sprite: Option<&str>,
    ) -> &CombatVisualProfile {
        let action = action_key(action_slot);
        for candidate in [
            sprite.and_then(|key| self.config.sprite_overrides.get(key)),
            avatar.and_then(|key| self.config.avatar_overrides.get(key)),
            class.and_then(|class| self.config.classes.get(class.id())),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(profile) = candidate
                .get(action)
                .or_else(|| candidate.get("default"))
                .and_then(|key| self.config.profiles.get(key))
            {
                return profile;
            }
        }
        self.resolve_style(style)
    }

    /// Exact, case-sensitive named animation clips; callers retain their existing heuristics.
    pub fn animation_aliases(&self, avatar_key: &str) -> Option<&AnimationAliases> {
        self.config.animation_aliases.get(avatar_key)
    }

    fn validate(&mut self) -> Result<(), String> {
        let config = &mut self.config;
        if config.profiles.len() > 128
            || config.avatar_overrides.len() > 256
            || config.sprite_overrides.len() > 256
            || config.animation_aliases.len() > 256
        {
            return Err("too many cosmetic profiles/overrides".into());
        }
        for (id, profile) in &mut config.profiles {
            if !valid_key(id)
                || !valid_color(profile.color)
                || !in_range(profile.scale, 0.25, 3.0)
                || !in_range(profile.trail.seconds, 0.0, 0.6)
                || !in_range(profile.trail.width, 0.02, 0.35)
                || !(1..=12).contains(&profile.trail.samples)
                || !valid_color(profile.impact.color)
                || !in_range(profile.impact.scale, 0.1, 2.0)
                || !in_range(profile.impact.lifetime, 0.08, 1.2)
            {
                return Err(format!("invalid or unbounded cosmetic profile {id}"));
            }
            if let Some(model) = &profile.model {
                if !safe_asset_path(&model.path, ".glb")
                    || model.scene > 31
                    || !in_range(model.scale, 0.01, 4.0)
                    || !model
                        .rotation_degrees
                        .into_iter()
                        .all(|v| in_range(v, -360.0, 360.0))
                {
                    return Err(format!("invalid packaged projectile model in {id}"));
                }
            }
            if let Some(sprite) = &profile.sprite {
                let cells = sprite.columns.saturating_mul(sprite.rows) as usize;
                if !safe_asset_path(&sprite.path, ".png")
                    || !(1..=16).contains(&sprite.columns)
                    || !(1..=16).contains(&sprite.rows)
                    || !sprite
                        .frame_size
                        .into_iter()
                        .all(|v| (1..=1024).contains(&v))
                    || sprite.frame_size[0].saturating_mul(sprite.columns) > 4096
                    || sprite.frame_size[1].saturating_mul(sprite.rows) > 4096
                    || sprite.frames == 0
                    || sprite.first_frame.saturating_add(sprite.frames) > cells
                    || !in_range(sprite.fps, 1.0, 60.0)
                    || !in_range(sprite.world_height, 0.3, 4.0)
                {
                    return Err(format!("invalid packaged projectile sprite in {id}"));
                }
            }
            profile.id.clone_from(id);
        }
        for (style, profile) in &config.defaults {
            if ![
                "standard",
                "arrow",
                "arcane",
                "holy",
                "crescent",
                "caster_bolt",
                "tower_bolt",
            ]
            .contains(&style.as_str())
                || !config.profiles.contains_key(profile)
            {
                return Err(format!("invalid style default {style}"));
            }
        }
        for class in config.classes.keys() {
            if HeroClass::from_id(class).is_none() {
                return Err(format!("unknown cosmetic class {class}"));
            }
        }
        for (key, actions) in config
            .classes
            .iter()
            .chain(&config.avatar_overrides)
            .chain(&config.sprite_overrides)
        {
            if !valid_key(key) || actions.len() > 6 {
                return Err("invalid cosmetic override key/count".into());
            }
            for (action, profile) in actions {
                if !["basic", "q", "w", "e", "r", "default"].contains(&action.as_str())
                    || !config.profiles.contains_key(profile)
                {
                    return Err(format!("invalid action/profile reference {key}/{action}"));
                }
            }
        }
        for (key, aliases) in &config.animation_aliases {
            if !valid_key(key) {
                return Err("invalid avatar animation key".into());
            }
            for names in [
                &aliases.idle,
                &aliases.walk,
                &aliases.run,
                &aliases.attack,
                &aliases.cast,
                &aliases.death,
            ] {
                if names.len() > 8 || names.iter().any(|name| !valid_key(name)) {
                    return Err("invalid animation clip aliases".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Asset, TypePath)]
struct LoadedCombatVisuals(CombatVisualRegistry);
#[derive(Default, TypePath)]
struct CombatVisualLoader;
impl AssetLoader for CombatVisualLoader {
    type Asset = LoadedCombatVisuals;
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
        let json = std::str::from_utf8(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        CombatVisualRegistry::from_json(json)
            .map(LoadedCombatVisuals)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }
    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}

#[derive(Resource, Default)]
struct PendingConfig(Option<Handle<LoadedCombatVisuals>>);

pub struct CombatVisualsPlugin;
impl Plugin for CombatVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CombatVisualRegistry>()
            .init_resource::<PendingConfig>()
            .init_asset::<LoadedCombatVisuals>()
            .init_asset_loader::<CombatVisualLoader>()
            .add_systems(Startup, request_config)
            .add_systems(Update, apply_config);
    }
}
fn request_config(server: Res<AssetServer>, mut pending: ResMut<PendingConfig>) {
    pending.0 = Some(server.load(CONFIG_PATH));
}
fn apply_config(
    server: Res<AssetServer>,
    loaded: Res<Assets<LoadedCombatVisuals>>,
    mut pending: ResMut<PendingConfig>,
    mut registry: ResMut<CombatVisualRegistry>,
) {
    let Some(handle) = pending.0.as_ref() else {
        return;
    };
    if let Some(config) = loaded.get(handle) {
        *registry = config.0.clone();
        registry.revision = 1;
        pending.0 = None;
    } else if matches!(
        server.get_load_state(handle.id()),
        Some(bevy::asset::LoadState::Failed(_))
    ) {
        warn!("Packaged combat cosmetics unavailable; using embedded procedural profiles");
        pending.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_classes_are_distinct_and_legacy_style_has_fallback() {
        let registry = CombatVisualRegistry::default();
        let shapes: std::collections::HashSet<_> = HeroClass::ALL
            .into_iter()
            .map(|class| {
                registry
                    .resolve(Some(class), ProjectileStyle::Standard, None, None, None)
                    .shape
            })
            .collect();
        assert_eq!(shapes.len(), 4);
        assert_eq!(
            registry.resolve_style(ProjectileStyle::Standard).shape,
            ProjectileShape::Bolt
        );
    }
    #[test]
    fn override_precedence_actions_and_animation_aliases_are_real() {
        let registry = CombatVisualRegistry::from_json(
            r#"{"schema_version":1,
          "avatar_overrides":{"agnes":{"basic":"mage_arcane","q":"warrior_crescent"}},
          "sprite_overrides":{"ranger":{"default":"cleric_holy"}},
          "animation_aliases":{"agnes":{"attack":["Sword_Swing"]}}}"#,
        )
        .unwrap();
        let resolve = |slot, avatar, sprite| {
            &registry
                .resolve(
                    Some(HeroClass::Ranger),
                    ProjectileStyle::Arrow,
                    slot,
                    avatar,
                    sprite,
                )
                .id
        };
        assert_eq!(resolve(None, Some("agnes"), Some("ranger")), "cleric_holy");
        assert_eq!(resolve(None, Some("agnes"), None), "mage_arcane");
        assert_eq!(resolve(Some(255), Some("agnes"), None), "mage_arcane");
        assert_eq!(resolve(Some(0), Some("agnes"), None), "warrior_crescent");
        assert_eq!(resolve(Some(2), Some("missing"), None), "ranger_arrow");
        assert_eq!(
            registry.animation_aliases("agnes").unwrap().attack,
            ["Sword_Swing"]
        );
    }
    #[test]
    fn invalid_config_rejects_gameplay_values_paths_and_unbounded_settings() {
        for json in [
            r#"{"schema_version":2}"#,
            r#"{"schema_version":1,"damage":99}"#,
            r#"{"schema_version":1,"classes":{"mage":{"q":"missing"}}}"#,
            r#"{"schema_version":1,"profiles":{"bad":{"shape":"arrow","color":[1,1,1,1],"scale":100}}}"#,
            r#"{"schema_version":1,"profiles":{"bad":{"shape":"arrow","color":[1,1,1,1],"model":{"path":"../secret.glb"}}}}"#,
            r#"{"schema_version":1,"profiles":{"bad":{"shape":"arrow","color":[1,1,1,1],"sprite":{"path":"shot.png","frame_size":[64,64],"columns":1,"rows":1,"frames":2,"fps":12,"world_height":1}}}}"#,
        ] {
            assert!(CombatVisualRegistry::from_json(json).is_err(), "{json}");
        }
        for path in [
            "https://evil/shot.glb",
            "/shot.glb",
            "../shot.glb",
            "a/../shot.glb",
            "a\\shot.glb",
            "a.glb#Scene0",
            "file://a.glb",
        ] {
            assert!(!safe_asset_path(path, ".glb"), "{path}");
        }
        assert!(safe_asset_path("cosmetics/arrow-v2.glb", ".glb"));
    }
}
