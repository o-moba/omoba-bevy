//! OMOBA's game contract: an `ekza.game-contract.v1` document (Ekza Game Protocol 1.0) with
//! the profile `omoba.character.v1`. It lists every place where OMOBA plays a clip on a hero
//! (a *slot*), what plays there by default and the rules a creator's clip for that slot must
//! pass. Ekza Studio reads it to show which animations a hero needs; Ekza Hub serves it; the
//! passport (C3) judges packages against it. `docs/game-contract.md` is the reference.
//!
//! The generator has two layers so that the published shape can change without touching
//! what is read from the game:
//!
//! 1. [`Facts::read`] collects facts from the files and code the game plays: the rows of
//!    `skills.skillfx`, the default kits, the shared motion library, the engine's timing
//!    constants, the class-default handhelds and the rule table
//!    `shared/assets/catalog/game-contract-rules.json`.
//! 2. [`render_contract`] shapes them into the protocol document (animation slots only;
//!    asset slots come with GP5); [`render_asset_notes`] keeps what the game shows besides a
//!    body (handhelds, gun modes, modelled bodies) as notes for GP5.
//!
//! `OMOBA_WRITE_GAME_CONTRACT=1 cargo test -p client --lib game_contract_is_current` writes
//! both files; `python3 scripts/export_game_contract.py --output shared/assets/catalog`
//! then adds the header (see [`HEADER_KEYS`], [`PROVENANCE_KEYS`]). Without the variable the
//! test fails when a committed file is stale.
use crate::held_weapons::{
    MUZZLE_WINDOW_SECS, flashes_muzzle, muzzle_point, shown_model, slot_mode,
};
use crate::humanoid::SharedHumanoidMotion;
use crate::skill_presentation::cast::CastKey;
use crate::skill_presentation::category;
use crate::skill_presentation::schema::{CONTACT_LIMIT_SECS, ROW_RATE, ROW_START};
use crate::skill_presentation::vocab::MotionPhase;
use crate::skill_presentation::{FITTED_RATES, SkillPresentation};
use omoba_passport::humanoid::{
    MAX_HUMANOID_BONES, MAX_HUMANOID_GLB_BYTES, MAX_HUMANOID_JSON_BYTES, MAX_HUMANOID_NODES,
    VrmVersion, required_bones,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use shared::handheld::HandheldSelection;
use shared::loadout::{AttackProfileId, SkillId, WeaponMode};
use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass, SkillSlot};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const CONTRACT_SCHEMA: &str = "ekza.game-contract.v1";
pub(crate) const PROFILE: &str = "omoba.character.v1";
const RIG: &str = "humanoid";
const NOTES_SCHEMA: &str = "omoba.game-contract-asset-notes.draft-1";
const CONTRACT_PATH: &str = "shared/assets/catalog/game-contract.json";
const NOTES_PATH: &str = "shared/assets/catalog/game-contract-asset-notes.json";
const WRITE_VAR: &str = "OMOBA_WRITE_GAME_CONTRACT";
/// Document keys the Python wrapper owns; the generator never writes them.
const HEADER_KEYS: [&str; 3] = ["source", "previousRevision", "revision"];
/// Provenance keys of the profile's document `ext` the wrapper owns as well: hashes of the
/// inputs, so that an input edit that does not change the body moves no revision.
const PROVENANCE_KEYS: [&str; 4] = [
    "gameplayRevision",
    "presentationSha256",
    "motionLibrary",
    "protocolVersion",
];
const RULES_JSON: &str = include_str!("../../shared/assets/catalog/game-contract-rules.json");
const ASSET_REQUIREMENTS_JSON: &str =
    include_str!("../../shared/assets/catalog/asset-requirements.json");
const PRESENTATION: &str = include_str!("../assets/config/skills.skillfx");
/// Library motions that only approximate what their label promises; user-facing notes must
/// say so (`assets-src/animations/README.md`, "stand-ins").
const STAND_INS: [(&str, &str); 3] = [
    ("flying_knee", "kick"),
    ("aim_loose_r", "bow release"),
    ("two_hand_push", "shield bash"),
];
const KEYS: [&str; 4] = ["q", "w", "e", "r"];

// ---------------------------------------------------------------------------------------
// The rule table (data, reviewed like code)
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RuleTable {
    schema: String,
    license: String,
    customizable_version: String,
    roles: BTreeMap<String, RolePolicy>,
    packaging: PackagingPolicy,
    rules: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RolePolicy {
    customizable_from: Option<String>,
    reserved: bool,
    window: Window,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PackagingPolicy {
    /// Most slots one character package may fill `custom`.
    character_max_fills: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Window {
    min_secs: f32,
    max_secs: f32,
}

impl RuleTable {
    fn parse() -> Result<Self, String> {
        let table: Self = serde_json::from_str(RULES_JSON)
            .map_err(|error| format!("game-contract-rules.json: {error}"))?;
        if table.schema != "omoba.game-contract-rules.v1" {
            return Err(format!("Unsupported rule table {}", table.schema));
        }
        Ok(table)
    }

    fn policy(&self, role: Role) -> &RolePolicy {
        self.roles
            .get(role.id())
            .unwrap_or_else(|| panic!("game-contract-rules.json has no role {}", role.id()))
    }

    fn rule(&self, id: &str) -> &Value {
        self.rules
            .iter()
            .find(|rule| rule["id"] == id)
            .unwrap_or_else(|| panic!("game-contract-rules.json has no rule {id}"))
    }

    /// A role is custom-able when it opens in this profile version or an earlier one.
    fn customizable(&self, role: Role) -> bool {
        let version = |text: &str| -> Vec<u32> {
            text.split('.')
                .map(|part| part.parse().expect("numeric version"))
                .collect()
        };
        self.policy(role)
            .customizable_from
            .as_deref()
            .is_some_and(|from| version(from) <= version(&self.customizable_version))
    }
}

// ---------------------------------------------------------------------------------------
// Layer 1: facts read from the game
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Base,
    Basic,
    Skill,
}

impl Group {
    fn id(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Basic => "basic",
            Self::Skill => "skill",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Idle,
    Run,
    Walk,
    Death,
    Basic,
    Release,
    Recast,
    Windup,
}

impl Role {
    fn id(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Run => "run",
            Self::Walk => "walk",
            Self::Death => "death",
            Self::Basic => "basic",
            Self::Release => "release",
            Self::Recast => "recast",
            Self::Windup => "windup",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Repeats while its state lasts.
    Loop,
    /// Plays once from its trigger; a newer action restarts the body.
    Action,
    /// Plays once and holds its last pose (death until respawn, a fitted windup).
    Hold,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Loop => "loop",
            Self::Action => "action",
            Self::Hold => "hold",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Locomotion,
    State,
    Skill(MotionPhase),
}

impl Phase {
    fn id(self) -> &'static str {
        match self {
            Self::Locomotion => "locomotion",
            Self::State => "state",
            Self::Skill(phase) => phase.id(),
        }
    }
}

/// What OMOBA plays in a slot when a package leaves it to the game.
#[derive(Clone, Debug)]
struct DefaultPlay {
    motion: String,
    rate: f32,
    start: f32,
}

#[derive(Clone, Debug)]
struct Telegraph {
    secs: f32,
    server_windup_secs: Option<f32>,
    fit_windup: bool,
}

#[derive(Clone, Debug)]
struct SlotHandheld {
    item: String,
    by_mode: BTreeMap<String, String>,
    bone: String,
    muzzle_window: Option<[f32; 2]>,
}

#[derive(Clone, Debug)]
struct SlotFacts {
    group: Group,
    role: Role,
    label: String,
    kind: Kind,
    phase: Phase,
    edge_released: bool,
    default: DefaultPlay,
    telegraph: Option<Telegraph>,
    handheld: Option<SlotHandheld>,
    alternates_with: Option<String>,
    used_by: BTreeSet<String>,
    skill: Option<String>,
    class: Option<String>,
    min_interval_secs: Option<f32>,
    weapon_mode: Option<String>,
}

#[derive(Clone, Debug)]
struct BasicAttack {
    range: f32,
    cooldown_secs: f32,
    attack_rate_cap: f32,
    slots: Vec<String>,
}

#[derive(Clone, Debug)]
struct KitEntry {
    key: &'static str,
    skill: String,
    name: String,
    slots: Vec<String>,
}

#[derive(Clone, Debug)]
struct HeroHandheld {
    default: String,
    by_mode: BTreeMap<String, String>,
    bone: String,
}

/// One handheld a hero draws in one gun mode, as the client resolves the class default.
#[derive(Clone, Debug)]
struct HandheldAsset {
    /// The gun mode, for a hero that switches guns.
    mode: Option<String>,
    item: String,
    model: String,
    bone: String,
    muzzle: Option<[f32; 3]>,
}

/// A model the presentation rows of a hero's kit draw for a replicated world object.
#[derive(Clone, Debug)]
struct BodyModel {
    skill: String,
    part: String,
    archetype: &'static str,
    model: String,
}

#[derive(Clone, Debug)]
struct HeroFacts {
    id: String,
    name: String,
    role: Value,
    basic: BasicAttack,
    kit: Vec<KitEntry>,
    handheld: Option<HeroHandheld>,
    attack_profile: Option<AttackProfileId>,
    projectile_style: Value,
    handheld_assets: Vec<HandheldAsset>,
    weapon_modes: Vec<String>,
    launcher_skills: Vec<String>,
    bodies: Vec<BodyModel>,
}

/// Engine timings a creator needs to know (and Studio's preview emulates).
#[derive(Clone, Debug)]
struct Timing {
    blend_secs: f32,
    idle_grace_secs: f32,
    facing_basic_secs: f32,
    facing_skill_secs: f32,
    contact_limit_secs: f32,
    row_rate: [f32; 2],
    row_start: [f32; 2],
    fit_rate: [f32; 2],
    unseen_hold_secs: f32,
    fresh_secs: f32,
    max_gap_secs: f64,
}

struct Facts {
    motion: &'static SharedHumanoidMotion,
    rules: RuleTable,
    timing: Timing,
    slots: BTreeMap<String, SlotFacts>,
    heroes: Vec<HeroFacts>,
}

impl Facts {
    fn read() -> Result<Self, String> {
        let motion = SharedHumanoidMotion::embedded()?;
        let registry = SkillPresentation::parse(PRESENTATION)?;
        let rules = RuleTable::parse()?;
        let mut facts = Self {
            motion,
            rules,
            timing: Timing {
                blend_secs: crate::player::BLEND_SECS,
                idle_grace_secs: crate::player::LOCOMOTION_IDLE_GRACE_SECS,
                facing_basic_secs: crate::player::BASIC_FACING_HOLD_SECS,
                facing_skill_secs: crate::player::SKILL_FACING_HOLD_SECS,
                contact_limit_secs: CONTACT_LIMIT_SECS,
                row_rate: [*ROW_RATE.start(), *ROW_RATE.end()],
                row_start: [*ROW_START.start(), *ROW_START.end()],
                fit_rate: [FITTED_RATES.0, FITTED_RATES.1],
                unseen_hold_secs: crate::player::UNSEEN_HOLD_SECS,
                fresh_secs: crate::skill_presentation::stage::FRESH_SECS,
                max_gap_secs: crate::skill_presentation::stage::MAX_GAP_SECS,
            },
            slots: BTreeMap::new(),
            heroes: Vec::new(),
        };
        facts.base_slots();
        for class in HeroClass::ALL {
            let hero = facts.hero(class, &registry)?;
            facts.heroes.push(hero);
        }
        Ok(facts)
    }

    fn base_slots(&mut self) {
        for (role, label, kind, phase) in [
            (Role::Idle, "Idle", Kind::Loop, Phase::Locomotion),
            (
                Role::Run,
                "Run (all movement)",
                Kind::Loop,
                Phase::Locomotion,
            ),
            (
                Role::Walk,
                "Walk (reserved; not played yet)",
                Kind::Loop,
                Phase::Locomotion,
            ),
            (Role::Death, "Death", Kind::Hold, Phase::State),
        ] {
            self.slots.insert(
                format!("base.{}", role.id()),
                SlotFacts {
                    group: Group::Base,
                    role,
                    label: label.into(),
                    kind,
                    phase,
                    edge_released: false,
                    default: DefaultPlay {
                        motion: role.id().into(),
                        rate: 1.0,
                        start: 0.0,
                    },
                    telegraph: None,
                    handheld: None,
                    alternates_with: None,
                    used_by: BTreeSet::from(["*".to_owned()]),
                    skill: None,
                    class: None,
                    min_interval_secs: None,
                    weapon_mode: None,
                },
            );
        }
    }

    fn hero(
        &mut self,
        class: HeroClass,
        registry: &SkillPresentation,
    ) -> Result<HeroFacts, String> {
        let cid = class.id();
        let preset = shared::loadout::preset_for_class(class);
        let attack_profile = preset.map(|kit| kit.attack_profile());
        let equipped = crate::equipped_skills::resolve_state(class, None)
            .ok_or_else(|| format!("{cid}: no default kit"))?;
        let definition = shared::basic_attack_for_class(class);
        let cap =
            shared::hero_balance::attack_rate_multiplier(class, shared::hero_balance::MAX_LEVEL);
        let min_interval = definition.cooldown_secs / cap;

        // Basic attack: the row of the class, and for a repeater its rocket round.
        let mut basic_slots = Vec::new();
        let mut rounds = vec![(CastKey::Basic(class), None)];
        if attack_profile == Some(AttackProfileId::Repeater) {
            rounds.push((CastKey::Rockets(class), Some(WeaponMode::Rockets)));
        }
        for (key, mode) in rounds {
            let row = registry
                .basic_round(key)
                .ok_or_else(|| format!("{cid}: no basic attack row"))?;
            let mode_id = mode.map(mode_id);
            let ids: Vec<String> = (0..row.motions.len())
                .map(|index| match &mode_id {
                    Some(mode) => format!("basic.{cid}.{mode}.{index}"),
                    None => format!("basic.{cid}.{index}"),
                })
                .collect();
            for (index, motion) in row.motions.iter().enumerate() {
                let mut label = String::from("Basic attack");
                if ids.len() > 1 {
                    label.push_str(&format!(" {}", index + 1));
                }
                if let Some(mode) = &mode_id {
                    label.push_str(&format!(" ({} mode)", mode.trim_end_matches('s')));
                }
                let alternates_with = (ids.len() == 2).then(|| ids[1 - index].clone());
                let handheld = slot_handheld(
                    class,
                    BASIC_ATTACK_ACTION_SLOT,
                    None,
                    mode.unwrap_or_default(),
                );
                self.add_slot(
                    &ids[index],
                    SlotFacts {
                        group: Group::Basic,
                        role: Role::Basic,
                        label,
                        kind: Kind::Action,
                        phase: Phase::Skill(MotionPhase::Instant),
                        edge_released: true,
                        default: DefaultPlay {
                            motion: motion.clone(),
                            rate: row.rate,
                            start: row.start,
                        },
                        telegraph: None,
                        handheld,
                        alternates_with,
                        used_by: BTreeSet::from([cid.to_owned()]),
                        skill: None,
                        class: Some(cid.into()),
                        min_interval_secs: Some(min_interval),
                        weapon_mode: mode_id.clone(),
                    },
                );
            }
            basic_slots.extend(ids);
        }

        // Q, W, E, R of the default kit: wind-up, release and recast of each skill.
        let mut kit = Vec::new();
        for slot in SkillSlot::ALL {
            let action_slot = slot.index() as u8;
            let key = CastKey::of(class, None, action_slot)
                .and_then(CastKey::skill)
                .ok_or_else(|| format!("{cid}: no skill in slot {}", KEYS[slot.index()]))?;
            let id = key.id();
            let name = equipped.ability(slot).name.to_owned();
            let profile = registry
                .row(id)
                .ok_or_else(|| format!("{cid}: no presentation row for {id}"))?;
            let modular = key.modular();
            let phase = profile.phase(key);
            let label = format!("{name} ({})", KEYS[slot.index()].to_uppercase());
            let telegraph = modular.and_then(|skill| {
                category::telegraph_secs(skill).map(|secs| Telegraph {
                    secs,
                    server_windup_secs: Some(shared::loadout::skill(skill).windup_secs)
                        .filter(|secs| *secs > 0.0),
                    fit_windup: profile.motion.fit_windup,
                })
            });
            let handheld = slot_handheld(class, action_slot, modular, WeaponMode::default());
            let skill_slot =
                |role: Role, kind, phase, edge, default, telegraph: Option<Telegraph>| SlotFacts {
                    group: Group::Skill,
                    role,
                    label: format!(
                        "{label} — {}",
                        if role == Role::Windup {
                            "wind-up"
                        } else {
                            role.id()
                        }
                    ),
                    kind,
                    phase,
                    edge_released: edge,
                    default,
                    telegraph,
                    handheld: handheld.clone(),
                    alternates_with: None,
                    used_by: BTreeSet::from([cid.to_owned()]),
                    skill: Some(id.to_owned()),
                    class: None,
                    min_interval_secs: None,
                    weapon_mode: None,
                };
            let mut slots = Vec::new();
            if let Some(windup) = &profile.windup {
                let clip = self
                    .motion
                    .clips
                    .get(windup)
                    .ok_or_else(|| format!("{id}: unknown windup {windup}"))?;
                // As `MotionCue::windup`: a clip fitted to its telegraph spans it once.
                let rate = telegraph
                    .as_ref()
                    .filter(|t| t.fit_windup && t.secs > 0.0)
                    .map_or(1.0, |t| {
                        (clip.duration / t.secs).clamp(FITTED_RATES.0, FITTED_RATES.1)
                    });
                let kind = if clip.looping { Kind::Loop } else { Kind::Hold };
                let slot_id = format!("skill.{id}.windup");
                self.add_slot(
                    &slot_id,
                    skill_slot(
                        Role::Windup,
                        kind,
                        Phase::Skill(phase),
                        false,
                        DefaultPlay {
                            motion: windup.clone(),
                            rate,
                            start: 0.0,
                        },
                        telegraph.clone(),
                    ),
                );
                slots.push(slot_id);
            }
            let release = format!("skill.{id}.release");
            self.add_slot(
                &release,
                skill_slot(
                    Role::Release,
                    Kind::Action,
                    Phase::Skill(phase),
                    phase == MotionPhase::Instant,
                    DefaultPlay {
                        motion: profile.release.clone(),
                        rate: profile.motion.rate,
                        start: profile.motion.start,
                    },
                    telegraph.clone(),
                ),
            );
            slots.push(release);
            if let Some(recast) = &profile.motion.recast {
                let slot_id = format!("skill.{id}.recast");
                self.add_slot(
                    &slot_id,
                    skill_slot(
                        Role::Recast,
                        Kind::Action,
                        Phase::Skill(MotionPhase::Instant),
                        true,
                        DefaultPlay {
                            motion: recast.clone(),
                            rate: profile.motion.recast_rate,
                            start: 0.0,
                        },
                        None,
                    ),
                );
                slots.push(slot_id);
            }
            kit.push(KitEntry {
                key: KEYS[slot.index()],
                skill: id.to_owned(),
                name,
                slots,
            });
        }

        // Asset needs: the class-default handhelds per gun mode and the modelled bodies.
        let modes: &[WeaponMode] = if attack_profile == Some(AttackProfileId::Repeater) {
            &[WeaponMode::Repeater, WeaponMode::Rockets]
        } else {
            &[WeaponMode::Repeater]
        };
        let switches = modes.len() > 1;
        let mut handheld_assets = Vec::new();
        for &mode in modes {
            let Some(item) = HandheldSelection::ClassDefault.resolve_mode(class, mode) else {
                continue;
            };
            let def = omoba_passport::weapon_store::definition(item)
                .ok_or_else(|| format!("{cid}: unknown handheld {item}"))?;
            let model = shown_model(class, true, &def);
            handheld_assets.push(HandheldAsset {
                mode: switches.then(|| mode_id(mode)),
                item: item.to_owned(),
                muzzle: muzzle_point(&model, mode, None).map(|tip| tip.to_array()),
                model,
                bone: def.grip.bone.clone(),
            });
        }
        let handheld = handheld_assets.first().map(|first| HeroHandheld {
            default: first.item.clone(),
            by_mode: handheld_assets
                .iter()
                .skip(1)
                .filter(|asset| asset.item != first.item)
                .filter_map(|asset| Some((asset.mode.clone()?, asset.item.clone())))
                .collect(),
            bone: first.bone.clone(),
        });
        let launcher_skills = if switches {
            kit.iter()
                .filter(|entry| {
                    SkillId::from_id(&entry.skill).is_some_and(|skill| {
                        slot_mode(class, Some(skill), WeaponMode::Repeater) == WeaponMode::Rockets
                    })
                })
                .map(|entry| entry.skill.clone())
                .collect()
        } else {
            Vec::new()
        };
        let mut bodies = Vec::new();
        for entry in &kit {
            let Some(profile) = registry.row(&entry.skill) else {
                continue;
            };
            let parts = profile
                .body
                .iter()
                .map(|body| ("body".to_owned(), body))
                .chain(
                    profile
                        .aux
                        .iter()
                        .map(|(name, body)| (format!("aux.{name}"), body)),
                );
            for (part, body) in parts {
                if let Some(model) = body.model {
                    bodies.push(BodyModel {
                        skill: entry.skill.clone(),
                        part,
                        archetype: body.archetype.id(),
                        model: format!("cosmetics/standard/{}.glb", model.id()),
                    });
                }
            }
        }

        Ok(HeroFacts {
            id: cid.into(),
            name: class.display_name().into(),
            role: serde_json::to_value(class.primary_role()).map_err(|e| e.to_string())?,
            basic: BasicAttack {
                range: definition.range,
                cooldown_secs: definition.cooldown_secs,
                attack_rate_cap: cap,
                slots: basic_slots,
            },
            kit,
            handheld,
            attack_profile,
            projectile_style: serde_json::to_value(shared::combat::ProjectileStyle::for_class(
                class,
            ))
            .map_err(|e| e.to_string())?,
            handheld_assets,
            weapon_modes: if switches {
                modes.iter().map(|mode| mode_id(*mode)).collect()
            } else {
                Vec::new()
            },
            launcher_skills,
            bodies,
        })
    }

    /// A slot keyed by skill id is shared by every view whose kit holds the skill.
    fn add_slot(&mut self, id: &str, slot: SlotFacts) {
        match self.slots.get_mut(id) {
            Some(existing) => existing.used_by.extend(slot.used_by),
            None => {
                self.slots.insert(id.to_owned(), slot);
            }
        }
    }
}

fn mode_id(mode: WeaponMode) -> String {
    serde_json::to_value(mode)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("weapon modes serialize as names")
}

/// The class-default handheld while a slot plays: the item of the gun mode the client shows
/// for it, the other modes' items, the gripping bone and the barrel-flash window.
fn slot_handheld(
    class: HeroClass,
    action_slot: u8,
    skill: Option<SkillId>,
    mode: WeaponMode,
) -> Option<SlotHandheld> {
    let shown = slot_mode(class, skill, mode);
    let item = HandheldSelection::ClassDefault.resolve_mode(class, shown)?;
    let def = omoba_passport::weapon_store::definition(item)?;
    let model = shown_model(class, true, &def);
    let muzzle = flashes_muzzle(action_slot, skill) && muzzle_point(&model, shown, None).is_some();
    let base = HandheldSelection::ClassDefault.resolve_mode(class, WeaponMode::Repeater);
    let by_mode = [WeaponMode::Rockets]
        .into_iter()
        .filter_map(|other| {
            HandheldSelection::ClassDefault
                .resolve_mode(class, other)
                .filter(|id| Some(*id) != base)
                .map(|id| (mode_id(other), id.to_owned()))
        })
        .collect();
    Some(SlotHandheld {
        item: item.into(),
        by_mode,
        bone: def.grip.bone.clone(),
        muzzle_window: muzzle.then_some(MUZZLE_WINDOW_SECS),
    })
}

// ---------------------------------------------------------------------------------------
// Layer 2: the published shapes
// ---------------------------------------------------------------------------------------

/// Four decimals: what a creator can act on, and stable across f32 noise.
fn secs(value: f32) -> f64 {
    round(f64::from(value))
}

fn round(value: f64) -> f64 {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn pair(values: [f32; 2]) -> Value {
    json!([secs(values[0]), secs(values[1])])
}

fn strings<'a>(values: impl IntoIterator<Item = &'a str>) -> Value {
    Value::Array(values.into_iter().map(Value::from).collect())
}

struct Derived {
    duration: f32,
    looping: bool,
    contact: Option<f32>,
    play: f32,
    effective_contact: Option<f32>,
}

fn derive(facts: &Facts, slot: &SlotFacts) -> Derived {
    let clip = facts
        .motion
        .clips
        .get(&slot.default.motion)
        .unwrap_or_else(|| panic!("unknown motion {}", slot.default.motion));
    let contact = facts.motion.contact(&slot.default.motion);
    let DefaultPlay { rate, start, .. } = slot.default;
    Derived {
        duration: clip.duration,
        looping: clip.looping,
        contact,
        play: clip.duration * (1.0 - start) / rate,
        effective_contact: contact.map(|contact| (contact - start * clip.duration) / rate),
    }
}

/// The as-played length a custom clip aims at: a loop's cycle, a fitted wind-up's
/// telegraph, else the default's play time.
fn target_secs(slot: &SlotFacts, d: &Derived) -> f32 {
    match (slot.kind, &slot.telegraph) {
        (Kind::Loop, _) => d.duration,
        (_, Some(telegraph)) if slot.role == Role::Windup && telegraph.fit_windup => telegraph.secs,
        _ => d.play,
    }
}

fn render_slot(facts: &Facts, id: &str, slot: &SlotFacts, shares: &[&str]) -> Value {
    let rules = &facts.rules;
    let policy = rules.policy(slot.role);
    let d = derive(facts, slot);
    let action = slot.kind == Kind::Action;
    let contact = d.effective_contact.map(|c| secs(c.max(0.0)));
    let mut default_markers = Map::new();
    let mut markers = Map::new();
    if action {
        if let Some(contact) = contact {
            default_markers.insert("contact".into(), Value::from(contact));
        }
        markers.insert(
            "contact".into(),
            json!({
                "required": true,
                "maxSecs": slot.edge_released.then(|| secs(facts.timing.contact_limit_secs)),
                "targetSecs": contact,
            }),
        );
    }
    let lead_in = slot
        .telegraph
        .as_ref()
        .filter(|_| slot.role == Role::Release)
        .map(|telegraph| secs(telegraph.secs));
    let mut ext = json!({
        "group": slot.group.id(),
        "role": slot.role.id(),
        "phase": slot.phase.id(),
        "edgeReleased": slot.edge_released,
        "row": {
            "motion": slot.default.motion,
            "durationSecs": secs(d.duration),
            "contactSecs": d.contact.map(secs),
            "rate": secs(slot.default.rate),
            "start": secs(slot.default.start),
            "effectiveContactSecs": d.effective_contact.map(secs),
            "standInFor": STAND_INS
                .iter()
                .find(|(motion, _)| *motion == slot.default.motion)
                .map(|(_, what)| *what),
        },
        "telegraph": slot.telegraph.as_ref().map(|t| json!({
            "secs": secs(t.secs),
            "serverWindupSecs": t.server_windup_secs.map(secs),
            "fitWindup": t.fit_windup,
        })),
        "handheld": slot.handheld.as_ref().map(|h| json!({
            "item": h.item,
            "byMode": h.by_mode,
            "bone": h.bone,
            "muzzleWindowSecs": h.muzzle_window.map(pair),
        })),
    });
    let profile = ext.as_object_mut().expect("slot ext");
    if let Some(skill) = &slot.skill {
        profile.insert("skill".into(), Value::from(skill.as_str()));
    }
    if let Some(class) = &slot.class {
        profile.insert("class".into(), Value::from(class.as_str()));
    }
    if let Some(mode) = &slot.weapon_mode {
        profile.insert("weaponMode".into(), Value::from(mode.as_str()));
    }
    json!({
        "kind": "animation",
        "label": slot.label,
        "description": null,
        "customizable": rules.customizable(slot.role),
        "customizableFrom": policy.customizable_from,
        "reserved": policy.reserved,
        "required": false,
        "default": {
            "name": slot.default.motion,
            "playSecs": secs(d.play),
            "loop": d.looping,
            "markers": default_markers,
            "preview": null,
        },
        "animation": {
            "rig": RIG,
            "playback": slot.kind.id(),
            "loop": slot.kind == Kind::Loop,
            "inPlace": true,
            "window": {
                "minSecs": secs(policy.window.min_secs),
                "targetSecs": secs(target_secs(slot, &d)),
                "maxSecs": secs(policy.window.max_secs),
            },
            "markers": markers,
            "minIntervalSecs": slot.min_interval_secs.map(secs),
            "leadInSecs": lead_in,
        },
        "asset": null,
        "socket": null,
        "sharesDefaultWith": strings(shares.iter().copied().filter(|other| *other != id)),
        "alternatesWith": strings(slot.alternates_with.as_deref()),
        "usedBy": strings(slot.used_by.iter().map(String::as_str)),
        "ext": { PROFILE: ext },
    })
}

fn render_target(facts: &Facts, hero: Option<&HeroFacts>) -> Value {
    let base = ["base.idle", "base.run", "base.death"];
    let mut sections = vec![json!({"id": "base", "label": "Base", "key": null, "slots": base})];
    let mut slots: Vec<&str> = base.to_vec();
    if let Some(hero) = hero {
        sections.push(json!({
            "id": "basic", "label": "Basic attack", "key": null, "slots": hero.basic.slots,
        }));
        slots.extend(hero.basic.slots.iter().map(String::as_str));
        for entry in &hero.kit {
            sections.push(json!({
                "id": entry.key,
                "label": entry.name,
                "key": entry.key.to_uppercase(),
                "slots": entry.slots,
            }));
            slots.extend(entry.slots.iter().map(String::as_str));
        }
    }
    let customizable: Vec<&str> = slots
        .iter()
        .copied()
        .filter(|slot| facts.rules.customizable(facts.slots[*slot].role))
        .collect();
    let (id, name) = hero.map_or(("any", "Any hero"), |hero| {
        (hero.id.as_str(), hero.name.as_str())
    });
    let label = if hero.is_some() {
        format!("Character for {name}")
    } else {
        "Character for any hero".to_owned()
    };
    json!({
        "id": id,
        "kind": if hero.is_some() { "character" } else { "any" },
        "name": name,
        "subtitle": hero.map(|hero| hero.role.clone()),
        "sections": sections,
        "slots": slots,
        "families": [],
        "accepts": [{
            "package": "character",
            "requirementId": format!("character.{id}"),
            "label": label,
            "slots": customizable,
            "families": [],
            "listing": "every-customizable-slot",
            "mustFill": [],
        }],
        "ext": { PROFILE: hero.map_or_else(
            || json!({"heroKind": "any", "role": null, "basicAttack": null, "kit": [], "handheld": null}),
            |hero| json!({
                "heroKind": "class",
                "role": hero.role,
                "basicAttack": {
                    "rangeM": secs(hero.basic.range),
                    "cooldownSecs": secs(hero.basic.cooldown_secs),
                    "attackRateCap": secs(hero.basic.attack_rate_cap),
                    "minIntervalSecs": secs(hero.basic.cooldown_secs / hero.basic.attack_rate_cap),
                },
                "kit": hero.kit.iter().map(|entry| json!({
                    "key": entry.key, "skill": entry.skill, "name": entry.name,
                })).collect::<Vec<_>>(),
                "handheld": hero.handheld.as_ref().map(|h| json!({
                    "default": h.default, "byMode": h.by_mode, "bone": h.bone,
                })),
            }),
        )},
    })
}

/// The body of OMOBA's `ekza.game-contract.v1` document: everything except the header the
/// Python wrapper adds ([`HEADER_KEYS`], [`PROVENANCE_KEYS`]).
fn render_contract(facts: &Facts) -> Value {
    let motion = facts.motion;
    let timing = &facts.timing;
    // Slots whose default plays the same library motion; one custom clip can serve them all.
    let mut by_motion: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, slot) in &facts.slots {
        if slot.group != Group::Base {
            by_motion
                .entry(slot.default.motion.as_str())
                .or_default()
                .push(id.as_str());
        }
    }
    let slots: Map<String, Value> = facts
        .slots
        .iter()
        .map(|(id, slot)| {
            let shares = if slot.group == Group::Base {
                &[][..]
            } else {
                by_motion[slot.default.motion.as_str()].as_slice()
            };
            (id.clone(), render_slot(facts, id, slot, shares))
        })
        .collect();
    let vrm1 = required_bones(VrmVersion::Vrm1);
    let targets: Vec<Value> = std::iter::once(render_target(facts, None))
        .chain(
            facts
                .heroes
                .iter()
                .map(|hero| render_target(facts, Some(hero))),
        )
        .collect();
    let socket = |side: &str| {
        json!({
            "label": format!("{} hand (palm)", if side == "left" { "Left" } else { "Right" }),
            "rig": RIG,
            "bone": format!("{side}Hand"),
            "frame": "palm",
            "fillable": false,
            "ext": {},
        })
    };
    json!({
        "schema": CONTRACT_SCHEMA,
        "schemaVersion": "1.0",
        "minReaderVersion": "1.0",
        "game": {"id": "omoba", "name": "OMOBA"},
        "license": facts.rules.license,
        "profiles": {"used": [PROFILE], "required": []},
        "rigs": {
            RIG: {
                "kind": "vrm-humanoid",
                "sourceFormats": ["vrm1", "vrm0"],
                "restPose": "t-pose",
                "facing": "as-vrm-spec",
                "requiredBones": strings(vrm1.iter().copied()),
                "requiredBonesVrm0": strings(
                    required_bones(VrmVersion::Vrm0)
                        .into_iter()
                        .filter(|bone| !vrm1.contains(bone))
                ),
                "drivenBones": strings(motion.bones.iter().map(String::as_str)),
                "maxHumanoidBones": MAX_HUMANOID_BONES,
                "maxNodes": MAX_HUMANOID_NODES,
                "uniformScale": "humanoid-bones-and-ancestors",
                "fingers": "optional",
                "thumbsRecommended": true,
                "lengthUnit": {
                    "name": "legLength",
                    "from": "hips-to-feet",
                    "referenceM": secs(motion.source_hips_to_feet_distance),
                },
                "ext": {},
            },
        },
        "sockets": {"hand.left": socket("left"), "hand.right": socket("right")},
        "packaging": {
            "character": {
                "containers": ["vrm1", "vrm0"],
                "manifestKey": "ekza_package_v1",
                "slotKinds": ["animation"],
                "maxBytes": MAX_HUMANOID_GLB_BYTES,
                "maxJsonBytes": MAX_HUMANOID_JSON_BYTES,
                "maxFills": facts.rules.packaging.character_max_fills,
                "binding": "manifest",
                "rendition": {"platform": "desktop", "profile": "humanoid-glb-v2", "version": 1},
                "ext": {},
            },
        },
        "rules": facts.rules.rules,
        "slots": slots,
        "families": {},
        "targets": targets,
        "retired": {"slots": {}, "targets": {}, "families": {}},
        "ext": {
            PROFILE: {
                "timing": {
                    "blendSecs": secs(timing.blend_secs),
                    "idleGraceSecs": secs(timing.idle_grace_secs),
                    "facingHoldSecs": {
                        "basic": secs(timing.facing_basic_secs),
                        "skill": secs(timing.facing_skill_secs),
                    },
                    "contactLimitSecs": secs(timing.contact_limit_secs),
                    "rowRate": pair(timing.row_rate),
                    "rowStart": pair(timing.row_start),
                    "fitRate": pair(timing.fit_rate),
                    "unseenHoldSecs": secs(timing.unseen_hold_secs),
                    "freshSecs": secs(timing.fresh_secs),
                    "maxGapSecs": round(timing.max_gap_secs),
                    "deathCut": "immediate",
                    "movementInterruptsAction": false,
                    "newActionRestarts": true,
                },
                "customPlayback": {"rate": 1, "start": 0},
            },
        },
    })
}

/// What each hero shows besides its body, kept as notes for the asset slots of GP5: the
/// class-default handhelds per gun mode, the sockets they attach to, the basic attack's
/// projectile and the modelled bodies of its kit, linked to the asset catalogue's
/// requirement ids. Not part of the game contract.
fn render_asset_notes(facts: &Facts) -> Result<Value, String> {
    let requirements: Value = serde_json::from_str(ASSET_REQUIREMENTS_JSON)
        .map_err(|error| format!("asset-requirements.json: {error}"))?;
    let rows = requirements["requirements"]
        .as_array()
        .ok_or("asset-requirements.json has no requirements")?;
    let heroes = facts.heroes.iter().map(|hero| {
        let skills: BTreeSet<&str> = hero.kit.iter().map(|entry| entry.skill.as_str()).collect();
        let linked: Vec<Value> = rows
            .iter()
            .filter(|row| {
                row["uses"].as_array().is_some_and(|uses| {
                    uses.iter().any(|edge| {
                        edge["basicAttackId"] == hero.id.as_str()
                            || edge["skillId"]
                                .as_str()
                                .is_some_and(|id| skills.contains(id))
                    })
                })
            })
            .map(|row| json!({"id": row["id"], "role": row["role"], "status": row["status"]}))
            .collect();
        let basic = match hero.attack_profile {
            Some(AttackProfileId::Melee) => json!({"delivery": "contact", "style": "crescent"}),
            Some(AttackProfileId::Repeater) => json!({
                "delivery": "projectile",
                "styleByMode": {"repeater": "bullet", "rockets": "rocket"},
            }),
            _ => json!({"delivery": "projectile", "style": hero.projectile_style}),
        };
        json!({
            "id": hero.id,
            "weaponModes": hero.weapon_modes,
            "launcherShownDuring": hero.launcher_skills,
            "handhelds": hero.handheld_assets.iter().map(|a| json!({
                "mode": a.mode,
                "item": a.item,
                "model": a.model,
                "socket": a.bone,
                "muzzle": a.muzzle.map(|tip| tip.map(secs)),
            })).collect::<Vec<_>>(),
            "basicAttack": basic,
            "bodies": hero.bodies.iter().map(|b| json!({
                "skill": b.skill,
                "part": b.part,
                "archetype": b.archetype,
                "model": b.model,
            })).collect::<Vec<_>>(),
            "assetRequirements": linked,
        })
    });
    Ok(json!({
        "schema": NOTES_SCHEMA,
        "status": "notes for GP5 (asset slots of omoba.character.v1); not part of the game contract",
        "license": facts.rules.license,
        "contract": CONTRACT_SCHEMA,
        "sockets": {
            "rightHand": {
                "frame": "palm grip frame of the rest pose: +Y along the shaft toward the index side, +Z out toward the fingertips; uses the index, middle and little proximal bones when present",
                "source": "client/src/humanoid/sockets.rs (grip_frame)",
            },
        },
        "sources": {
            "handhelds": "shared/src/handheld.rs (HandheldSelection::resolve_mode), client/assets/weapons/manifest.json (grip), client/src/held_weapons.rs (shown_model, slot_mode, muzzle_point)",
            "weaponModes": "shared/src/loadout.rs (AttackProfileId::Repeater, WeaponMode), client/src/held_weapons.rs (slot_mode)",
            "basicAttack": "common/src/basic_attack.rs (melee contact, repeater bullet or rocket, else the catalogue projectile_style), shared/assets/catalog/heroes.json",
            "bodies": "client/assets/config/skills.skillfx (body.model, aux.<kind>.model), client/src/skill_presentation/effects.rs (cosmetics/standard/<model>.glb)",
            "assetRequirements": "shared/assets/catalog/asset-requirements.json (ids of omoba.asset-catalog.v1)",
        },
        "heroes": heroes.collect::<Vec<_>>(),
    }))
}

mod tests;
