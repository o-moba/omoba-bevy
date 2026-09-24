//! Authoritative hero state owned by the server.
//!
//! The wire `PlayerState` is never stored: `ConnectedPlayer::owner_view` and
//! `public_view` (`entities.rs`) build it from these structs, `HeroTimers`
//! and the tick's `now`.
use crate::*;

/// Wallet, income and inventory. Reconnects keep it (it lives inside the
/// retained `ConnectedPlayer`); `reset_player_round` clears it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeroEconomy {
    pub(crate) gold: u32,
    pub(crate) earned_gold: u32,
    pub(crate) inventory: Vec<ItemId>,
    pub(crate) item_bonuses: ItemBonuses,
    pub(crate) last_purchase: Option<PurchaseReceipt>,
    /// Replay high-water mark, retained across reconnect and respawn in this round.
    pub(crate) basic_attack_request_id: u64,
    /// Purchase request high-water mark, including rejected requests.
    pub(crate) purchase_sequence: u64,
    /// Fractional passive income not yet paid out as whole gold.
    pub(crate) gold_income_remainder: f32,
}

impl HeroEconomy {
    /// A fresh wallet at round start.
    pub(crate) fn starting() -> Self {
        Self {
            gold: STARTING_GOLD,
            earned_gold: 0,
            inventory: Vec::new(),
            item_bonuses: ItemBonuses::NONE,
            last_purchase: None,
            basic_attack_request_id: 0,
            purchase_sequence: 0,
            gold_income_remainder: 0.0,
        }
    }
}

/// Experience, level and skill ranks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HeroProgress {
    pub(crate) xp: u32,
    pub(crate) level: u32,
    pub(crate) next_level_xp: u32,
    pub(crate) skill_points: u32,
    pub(crate) ranks: [u8; 4],
}

impl HeroProgress {
    /// Level one with every ability at rank one.
    pub(crate) fn starting() -> Self {
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
pub(crate) struct HeroUtility {
    /// High-water request mark, including rejected requests in this round.
    pub(crate) last_request_id: u64,
    /// Advances only on an accepted dash, including fully blocked dashes.
    pub(crate) dash_sequence: u64,
}

/// The last accepted cosmetic action, replicated so clients can play it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct HeroAction {
    /// Monotonic event id; advances after an accepted skill or basic attack.
    pub(crate) sequence: u64,
    pub(crate) kind: PlayerActionKind,
    /// Q/W/E/R index, or `BASIC_ATTACK_ACTION_SLOT` for a basic strike.
    pub(crate) slot: u8,
}

/// Who the hero is: set at join (or reconnect) and never by the simulation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeroIdentity {
    pub(crate) id: u64,
    pub(crate) is_bot: bool,
    pub(crate) team: Team,
    /// Authoritative class assigned at join time (kit resolution key).
    pub(crate) hero_class: HeroClass,
    pub(crate) character: CharacterChoice,
    /// Cosmetic roster avatar slug; `None` means the legacy `character` model.
    pub(crate) avatar: Option<String>,
    pub(crate) sprite_character: Option<String>,
    /// Cosmetic only, authorized from persisted profile grants.
    pub(crate) supporter_aura: Option<shared::supporter::AuraStyle>,
}

/// Authoritative hero core: what the simulation reads and writes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hero {
    pub(crate) identity: HeroIdentity,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) yaw: f32,
    pub(crate) hp: f32,
    pub(crate) max_hp: f32,
    pub(crate) mana: f32,
    pub(crate) max_mana: f32,
    pub(crate) progress: HeroProgress,
    pub(crate) utility: HeroUtility,
    pub(crate) last_action: HeroAction,
}

impl Hero {
    /// The pre-join placeholder: green team, default class, full legacy
    /// pools, at `spawn`. `handle_join_request_with_sprite` fills the
    /// identity and `reset_player_round` re-derives the pools from the class.
    pub(crate) fn new(id: u64, spawn: Vec3f) -> Self {
        Self {
            identity: HeroIdentity {
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
        }
    }
}
