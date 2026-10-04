//! Authoritative hero state owned by the server.
//!
//! The wire `PlayerState` is never stored: `ConnectedPlayer::owner_view` and
//! `public_view` (`entities.rs`) build it from these structs, `HeroTimers`
//! and the tick's `now`.

use shared::map::Team;
use shared::shop::{ItemBonuses, ItemId, PurchaseReceipt, STARTING_GOLD};
use shared::wire::{CharacterChoice, default_character_choice};
use shared::{HeroClass, PlayerActionKind};

use crate::balance::{MAX_HP, MAX_MANA, PLAYER_GROUND_Y, STARTING_LEVEL};
use crate::entities::Vec3f;
use crate::progression::xp_threshold_for_level;

/// Wallet, income and inventory. Reconnects keep it (it lives inside the
/// retained `ConnectedPlayer`); `reset_player_round` clears it.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroEconomy {
    pub gold: u32,
    pub earned_gold: u32,
    pub inventory: Vec<ItemId>,
    pub item_bonuses: ItemBonuses,
    pub last_purchase: Option<PurchaseReceipt>,
    /// Replay high-water mark, retained across reconnect and respawn in this round.
    pub basic_attack_request_id: u64,
    /// Purchase request high-water mark, including rejected requests.
    pub purchase_sequence: u64,
    /// Fractional passive income not yet paid out as whole gold.
    pub gold_income_remainder: f32,
    /// Deterministic accepted-strike credit: rejected/replayed commands cannot advance it.
    pub basic_crit_meter: f32,
    pub death_streak: u32,
}

impl HeroEconomy {
    /// A fresh wallet at round start.
    pub fn starting() -> Self {
        Self {
            gold: STARTING_GOLD,
            earned_gold: 0,
            inventory: Vec::new(),
            item_bonuses: ItemBonuses::NONE,
            last_purchase: None,
            basic_attack_request_id: 0,
            purchase_sequence: 0,
            gold_income_remainder: 0.0,
            basic_crit_meter: 0.0,
            death_streak: 0,
        }
    }
}

/// Experience, level and skill ranks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeroProgress {
    pub xp: u32,
    pub level: u32,
    pub next_level_xp: u32,
    pub skill_points: u32,
    pub ranks: [u8; 4],
}

impl HeroProgress {
    /// Level one with every ability at rank one.
    pub fn starting() -> Self {
        Self {
            xp: 0,
            level: STARTING_LEVEL,
            next_level_xp: xp_threshold_for_level(STARTING_LEVEL),
            skill_points: 0,
            ranks: [1; 4],
        }
    }
}

/// Utility request bookkeeping; the utility clocks themselves are `HeroTimers`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HeroUtility {
    /// High-water request mark, including rejected requests in this round.
    pub last_request_id: u64,
    /// Monotonic movement correction barrier for dash and other teleports.
    pub dash_sequence: u64,
    pub recall_sequence: u64,
}

/// The last accepted cosmetic action, replicated so clients can play it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HeroAction {
    /// Monotonic event id; advances after an accepted skill or basic attack.
    pub sequence: u64,
    pub kind: PlayerActionKind,
    /// Q/W/E/R index, or `BASIC_ATTACK_ACTION_SLOT` for a basic strike.
    pub slot: u8,
    pub yaw: Option<f32>,
}

/// Who the hero is: set at join (or reconnect) and never by the simulation.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroIdentity {
    pub id: u64,
    pub is_bot: bool,
    pub team: Team,
    /// Authoritative class assigned at join time (kit resolution key).
    pub hero_class: HeroClass,
    pub character: CharacterChoice,
    /// Cosmetic roster avatar slug; `None` means the legacy `character` model.
    pub avatar: Option<String>,
    pub sprite_character: Option<String>,
    /// Cosmetic only, authorized from persisted profile grants.
    pub supporter_aura: Option<shared::supporter::AuraStyle>,
    pub handheld: shared::handheld::HandheldSelection,
}

/// Authoritative hero core: what the simulation reads and writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Hero {
    pub identity: HeroIdentity,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub hp: f32,
    pub max_hp: f32,
    pub mana: f32,
    pub max_mana: f32,
    pub progress: HeroProgress,
    pub utility: HeroUtility,
    pub last_action: HeroAction,
    pub skills: crate::skills::HeroSkills,
}

impl Hero {
    /// The pre-join placeholder: green team, default class, full legacy
    /// pools, at `spawn`. `handle_join_request_with_sprite` fills the
    /// identity and `reset_player_round` re-derives the pools from the class.
    pub fn new(id: u64, spawn: Vec3f) -> Self {
        Self {
            identity: HeroIdentity {
                handheld: Default::default(),
                id,
                is_bot: false,
                team: Team::Green,
                hero_class: HeroClass::default(),
                character: default_character_choice(),
                avatar: None,
                sprite_character: None,
                supporter_aura: None,
            },
            x: spawn.x,
            y: PLAYER_GROUND_Y,
            z: spawn.z,
            yaw: 0.0,
            hp: MAX_HP,
            max_hp: MAX_HP,
            mana: MAX_MANA,
            max_mana: MAX_MANA,
            progress: HeroProgress::starting(),
            utility: HeroUtility::default(),
            last_action: HeroAction::default(),
            skills: crate::skills::HeroSkills::default(),
        }
    }
}
