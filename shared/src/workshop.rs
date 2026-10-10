//! Private named class documents and game-owned web authoring metadata.
//! A document is a recipe, never a new HeroClass or a multiplayer admission.
use crate::loadout::{
    self, BuildRecipe, CoreId, LoadoutError, PassiveId, ResolvedLoadout, SkillId,
};
use crate::sandbox::SandboxConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DOCUMENT_SCHEMA: &str = "omoba.class-build.v1";
pub const CATALOG_SCHEMA: &str = "omoba.workshop.v1";
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024;
pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_DESCRIPTION_CHARS: usize = 1000;
pub const CORES: [CoreId; 12] = [
    CoreId::Dawnweaver,
    CoreId::Wildspark,
    CoreId::Cinderforge,
    CoreId::Edgeweaver,
    CoreId::Stormfist,
    CoreId::Veilstalker,
    CoreId::Emberveil,
    CoreId::Orbitwright,
    CoreId::Riftshot,
    CoreId::Chainkeeper,
    CoreId::Frostguard,
    CoreId::Adventurer,
];
pub const PASSIVES: [PassiveId; 12] = [
    PassiveId::Radiance,
    PassiveId::Momentum,
    PassiveId::Tempered,
    PassiveId::Vitals,
    PassiveId::Flow,
    PassiveId::Shroud,
    PassiveId::Essence,
    PassiveId::Clockwork,
    PassiveId::Resonance,
    PassiveId::Souls,
    PassiveId::Concussion,
    PassiveId::DaggerMastery,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassBuildDocument {
    pub schema: String,
    pub name: String,
    pub description: String,
    pub recipe: BuildRecipe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkshopError {
    TooLarge,
    Parse(String),
    Schema,
    Name,
    Description,
    Recipe(LoadoutError),
}
impl std::fmt::Display for WorkshopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "Class build exceeds 8 KiB"),
            Self::Parse(error) => write!(f, "Invalid class build JSON: {error}"),
            Self::Schema => write!(
                f,
                "Unsupported class build schema; expected {DOCUMENT_SCHEMA}"
            ),
            Self::Name => write!(
                f,
                "Build name must contain 1–80 characters and cannot be blank or contain control characters"
            ),
            Self::Description => {
                write!(f, "Build description must contain at most 1000 characters")
            }
            Self::Recipe(error) => write!(f, "Invalid class recipe: {error}"),
        }
    }
}
impl std::error::Error for WorkshopError {}

impl ClassBuildDocument {
    /// Parses a bounded untrusted file; never repairs invalid or stale imports.
    pub fn parse(bytes: &[u8]) -> Result<Self, WorkshopError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(WorkshopError::TooLarge);
        }
        let document: Self = serde_json::from_slice(bytes)
            .map_err(|error| WorkshopError::Parse(error.to_string()))?;
        document.validate()?;
        Ok(document)
    }

    /// Used equally by Account API persistence and local native import.
    pub fn validate(&self) -> Result<ResolvedLoadout, WorkshopError> {
        if self.schema != DOCUMENT_SCHEMA {
            return Err(WorkshopError::Schema);
        }
        if self.name.trim().is_empty()
            || self.name.chars().count() > MAX_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(WorkshopError::Name);
        }
        if self.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(WorkshopError::Description);
        }
        loadout::resolve(&self.recipe).map_err(WorkshopError::Recipe)
    }

    /// A level-six local test with a target. Uploads cannot provide actor stats,
    /// arbitrary appearance paths, cheats, AI settings or service credentials.
    pub fn sandbox_config(&self) -> Result<SandboxConfig, WorkshopError> {
        let resolved = self.validate()?;
        let mut config = sandbox_template();
        config.player.hero = resolved.core().class();
        config.player.max_hp = crate::hero_balance::base_hp(config.player.hero);
        config.player.recipe = Some(resolved.recipe());
        Ok(config)
    }
}

pub fn sandbox_template() -> SandboxConfig {
    let mut config = SandboxConfig::default();
    config.player.hero = CoreId::Dawnweaver.class();
    config.player.recipe = Some(CoreId::Dawnweaver.preset());
    config.player.max_hp = crate::hero_balance::base_hp(config.player.hero);
    config.player.level = 6;
    config.dummy.enabled = true;
    config
}

fn passive_name(id: PassiveId) -> &'static str {
    match id {
        PassiveId::Radiance => "Radiance",
        PassiveId::Momentum => "Momentum",
        PassiveId::Tempered => "Tempered",
        PassiveId::Vitals => "Vitals",
        PassiveId::Flow => "Flow",
        PassiveId::Shroud => "Shroud",
        PassiveId::Essence => "Essence",
        PassiveId::Clockwork => "Clockwork",
        PassiveId::Resonance => "Resonance",
        PassiveId::Souls => "Souls",
        PassiveId::Concussion => "Concussion",
        PassiveId::DaggerMastery => "Dagger Mastery",
    }
}

/// Deterministic metadata generated from compiled definitions and the same
/// capability requirements used by `loadout::resolve`; balance is informational.
pub fn catalog() -> Value {
    crate::catalog::ensure_loaded();
    let cores: Vec<_> = CORES
        .into_iter()
        .map(|core| {
            let hero = core.class();
            let preset = core.preset();
            json!({"id": core, "name": hero.display_name(), "description": hero.tagline(),
            "defaultPassive": preset.passive, "attackProfile": core.attack_profile(),
            "baseHp": crate::hero_balance::base_hp(hero), "preset": preset})
        })
        .collect();
    let passives: Vec<_> = PASSIVES
        .into_iter()
        .map(|id| json!({"id": id, "name": passive_name(id)}))
        .collect();
    let skills: Vec<_> = SkillId::ALL
        .into_iter()
        .map(|id| {
            let skill = loadout::skill(id);
            let ability = skill.ability;
            let requirements = loadout::requirements(id);
            json!({"id": id, "name": ability.name, "description": ability.description,
            "defaultSlot": skill.slot, "unlockLevel": crate::SLOT_UNLOCK_LEVELS[skill.slot.index()],
            "manaCost": ability.base_mana_cost, "recastManaCost": skill.recast_mana_cost,
            "cooldownSecs": ability.base_cooldown_secs, "castRange": ability.cast_range,
            "targeting": ability.targeting, "maxRank": ability.max_rank,
            "requirements": {"attackProfile": requirements.attack_profile,
                "anyOfSkills": requirements.any_of_skills}})
        })
        .collect();
    json!({"schema": CATALOG_SCHEMA, "schemaVersion": 1,
        "recipeSchemaVersion": loadout::RECIPE_SCHEMA_VERSION,
        "gameplayRevision": loadout::CATALOG_REVISION, "protocolVersion": crate::protocol::PROTOCOL_VERSION,
        "limits": {"nameChars": MAX_NAME_CHARS, "descriptionChars": MAX_DESCRIPTION_CHARS,
            "documentBytes": MAX_DOCUMENT_BYTES, "skillCount": 4},
        "cores": cores, "passives": passives, "skills": skills, "sandboxTemplate": sandbox_template()})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> ClassBuildDocument {
        ClassBuildDocument {
            schema: DOCUMENT_SCHEMA.into(),
            name: "Mixed light".into(),
            description: "A local experiment".into(),
            recipe: CoreId::Dawnweaver.preset(),
        }
    }

    #[test]
    fn strict_document_roundtrip_preserves_metadata_and_recipe() {
        let mut doc = document();
        doc.name = "星火 🦊".into();
        doc.recipe.skills = [
            SkillId::WildRocket,
            SkillId::DawnField,
            SkillId::DawnBarrier,
            SkillId::DawnBind,
        ];
        assert_eq!(
            ClassBuildDocument::parse(&serde_json::to_vec(&doc).unwrap()).unwrap(),
            doc
        );
        let mut unknown = serde_json::to_value(&doc).unwrap();
        unknown["accountId"] = json!("injected");
        assert!(ClassBuildDocument::parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
        let mut unknown = serde_json::to_value(&doc).unwrap();
        unknown["recipe"]["damage"] = json!(9999);
        assert!(ClassBuildDocument::parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
        assert!(
            ClassBuildDocument::parse(
                br#"{"schema":"omoba.class-build.v1","schema":"omoba.class-build.v1"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn metadata_and_size_limits_do_not_silently_trim() {
        let mut doc = document();
        for bad in [String::new(), " ".into(), "a\nb".into(), "界".repeat(81)] {
            doc.name = bad;
            assert_eq!(doc.validate(), Err(WorkshopError::Name));
        }
        doc.name = "界".repeat(80);
        doc.description = "界".repeat(1000);
        assert!(doc.validate().is_ok());
        doc.description.push('!');
        assert_eq!(doc.validate(), Err(WorkshopError::Description));
        assert_eq!(
            ClassBuildDocument::parse(&vec![b' '; MAX_DOCUMENT_BYTES + 1]),
            Err(WorkshopError::TooLarge)
        );
    }

    #[test]
    fn stale_duplicate_unknown_and_dependency_recipes_are_rejected() {
        let mut doc = document();
        doc.recipe.catalog_revision = "old".into();
        assert_eq!(
            doc.validate(),
            Err(WorkshopError::Recipe(LoadoutError::CatalogRevision))
        );
        doc = document();
        doc.recipe.skills[1] = doc.recipe.skills[0];
        assert!(matches!(
            doc.validate(),
            Err(WorkshopError::Recipe(LoadoutError::DuplicateSkill { .. }))
        ));
        doc = document();
        doc.recipe.skills[0] = SkillId::WildSwitch;
        assert!(matches!(
            doc.validate(),
            Err(WorkshopError::Recipe(LoadoutError::RequiresRepeater { .. }))
        ));
        doc.recipe.skills[0] = SkillId::OrbitalCollapse;
        assert!(matches!(
            doc.validate(),
            Err(WorkshopError::Recipe(
                LoadoutError::RequiresOrbController { .. }
            ))
        ));
        doc.recipe.skills[1] = SkillId::OrbitalGuard;
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn safe_preset_sets_real_core_stats_and_keeps_normal_resources() {
        for core in CORES {
            let mut doc = document();
            doc.recipe = core.preset();
            let preset = doc.sandbox_config().unwrap();
            assert_eq!(preset.player.hero, core.class());
            assert_eq!(
                preset.player.max_hp,
                crate::hero_balance::base_hp(core.class())
            );
            assert_eq!(preset.player.recipe, Some(core.preset()));
            assert_eq!(preset.player.level, 6);
            assert_eq!(preset.player.ranks, [1; 4]);
            assert!(preset.dummy.enabled);
            assert!(
                !preset.player.unlock_all
                    && !preset.player.infinite_resource
                    && !preset.player.no_cooldowns
                    && !preset.player.god_mode
            );
            assert_eq!(preset.player.avatar, None);
            assert!(preset.player.handheld.is_default());
        }
    }

    #[test]
    fn catalogue_uses_canonical_presets_and_dependency_rules() {
        let exported = catalog();
        assert_eq!(exported["cores"].as_array().unwrap().len(), 12);
        assert_eq!(exported["skills"].as_array().unwrap().len(), 48);
        for (core, exported_core) in CORES.iter().zip(exported["cores"].as_array().unwrap()) {
            assert_eq!(
                exported_core["preset"],
                serde_json::to_value(core.preset()).unwrap()
            );
        }
        for id in SkillId::ALL {
            let dependency = loadout::requirements(id);
            let entry = exported["skills"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == json!(id))
                .unwrap();
            assert_eq!(
                entry["requirements"]["anyOfSkills"],
                json!(dependency.any_of_skills)
            );
            assert_eq!(
                entry["requirements"]["attackProfile"],
                json!(dependency.attack_profile)
            );
        }
    }
}
