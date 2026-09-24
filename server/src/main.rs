mod balance;
#[cfg(test)]
mod balance_probe;
mod basic_attack;
mod bots;
mod career_backend;
mod career_runtime;
#[cfg(test)]
mod career_runtime_tests;
#[cfg(test)]
mod objective_balance_tests;
use omoba_career_store::career_store;
mod combat_feedback;
mod entities;
mod forest_pickups;
mod formation;
mod game_world;
mod hero;
mod hero_stats;
mod hero_timers;
#[cfg(test)]
mod map_config_tests;
mod match_allocation;
mod match_pool;
mod match_rules;
mod match_service;
mod match_stats;
mod public_transport;
use omoba_career_store::matchmaking;
#[cfg(test)]
mod minion_path_tests;
#[cfg(test)]
mod navigation_tests;
mod neutrals;
mod passport_admission;
#[cfg(test)]
mod practice_tests;
mod prematch;
mod progression;
#[cfg(test)]
mod release_tests;
mod runtime;
mod sandbox;
mod session;
mod shop;
mod sim;
mod snapshot;
mod social;
mod targeting_qa;
#[cfg(test)]
mod terminal_result_tests;
#[cfg(test)]
mod tests;
mod utility;
mod vision;
mod world;

use balance::*;
use basic_attack::*;
use combat_feedback::*;
use ekza_bevy_sdk::EkzaCharacter as CharacterChoice;
use neutrals::*;
use progression::*;
use session::*;
use shared::combat::{CombatEntity, CombatEntityKind, CombatEvent, MinionKind, ProjectileStyle};
use shared::map::{Lane, Team};
#[cfg(test)]
use shared::scaled_cooldown;
use shared::shop::{ItemBonuses, ItemId, PurchaseReceipt, STARTING_GOLD};
use shared::wire::{
    ClientPacket, GameState, MinionBrainState, MinionState, MinionTargetKind, NeutralAiState,
    NeutralCampType, NeutralState, PlayerState, ProjectileState, ServerPacket, StructureKind,
    StructureState, TargetId, TargetKind, TeamBuffKind, TeamBuffState, default_character_choice,
};
use shared::{
    HeroClass, PlayerActionKind, SkillSlot, TargetingMode, ability_for_class_slot,
    rank_effect_scale, scaled_cast_range, scaled_mana_cost, unlocked_slots_for_level,
};
use shop::*;
use std::{
    collections::{HashMap, HashSet},
    fmt, io,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};
use utility::*;
use world::*;

pub(crate) use entities::*;
pub(crate) use formation::*;
pub(crate) use game_world::*;
pub(crate) use hero::*;
pub(crate) use hero_stats::StatModifiers;
pub(crate) use hero_timers::HeroTimers;
pub(crate) use match_rules::*;
pub(crate) use runtime::dispatch::*;
pub(crate) use runtime::*;
pub(crate) use sim::{cast::*, minions::*, neutrals::*, projectiles::*, towers::*, *};
pub(crate) use snapshot::*;

fn main() -> io::Result<()> {
    runtime::run()
}
