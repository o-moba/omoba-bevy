//! Authoritative base-shop validation, payment and passive income.
use std::collections::HashMap;
use std::net::SocketAddr;

use shared::map::Team;
use shared::shop::{
    GOLD_PER_SECOND, INVENTORY_CAPACITY, ItemId, PurchaseError, PurchaseReceipt, SHOP_RADIUS, item,
    item_bonuses,
};
use shared::wire::GameState;

use crate::entities::{ConnectedPlayer, MapLayoutState};
use crate::hero::Hero;

fn phase_allows_shop(phase: &GameState) -> bool {
    !matches!(phase, GameState::Victory { .. })
}

fn inside_own_shop(hero: &Hero, map: &MapLayoutState) -> bool {
    let base = match hero.identity.team {
        Team::Green => map.home,
        Team::Blue => map.away,
    };
    (hero.x - base.x).powi(2) + (hero.z - base.z).powi(2) <= SHOP_RADIUS * SHOP_RADIUS
}

pub fn shop_is_available(hero: &Hero, map: &MapLayoutState, phase: &GameState) -> bool {
    hero.hp > 0.0
        && phase_allows_shop(phase)
        && (inside_own_shop(hero, map) || hero.skills.advanced.forge_ready)
}

pub fn handle_purchase(
    player: &mut ConnectedPlayer,
    map: &MapLayoutState,
    phase: &GameState,
    raw_item: &str,
    request_id: u64,
    match_id: u64,
) {
    if request_id == 0 || request_id <= player.economy.purchase_sequence {
        return;
    }
    player.economy.purchase_sequence = request_id;
    let item_id = ItemId::from_id(raw_item);
    let error = if !player.joined || !phase_allows_shop(phase) {
        Some(PurchaseError::Unavailable)
    } else if player.hero.hp <= 0.0 {
        Some(PurchaseError::Dead)
    } else if !inside_own_shop(&player.hero, map) && !player.hero.skills.advanced.forge_ready {
        Some(PurchaseError::OutsideBase)
    } else if let Some(id) = item_id {
        if player.economy.inventory.len() >= INVENTORY_CAPACITY {
            Some(PurchaseError::InventoryFull)
        } else if player.economy.inventory.contains(&id) {
            Some(PurchaseError::AlreadyOwned)
        } else if player.economy.gold < item(id).cost {
            Some(PurchaseError::InsufficientGold)
        } else {
            None
        }
    } else {
        Some(PurchaseError::UnknownItem)
    };
    if error.is_none() {
        let id = item_id.expect("validated item");
        let old = player.economy.item_bonuses;
        player.economy.gold -= item(id).cost;
        player.economy.inventory.push(id);
        player.economy.item_bonuses = item_bonuses(&player.economy.inventory);
        let hp_bonus = player.economy.item_bonuses.max_hp - old.max_hp;
        let mana_bonus = if player
            .hero
            .skills
            .loadout
            .is_some_and(|l| l.core() == shared::loadout::CoreId::Stormfist)
        {
            0.0
        } else {
            player.economy.item_bonuses.max_mana - old.max_mana
        };
        player.hero.max_hp += hp_bonus;
        player.hero.hp = (player.hero.hp + hp_bonus).min(player.hero.max_hp);
        player.hero.max_mana += mana_bonus;
        player.hero.mana = (player.hero.mana + mana_bonus).min(player.hero.max_mana);
        println!(
            "MATCH_METRIC event=purchase match={match_id} player={} request={request_id} item={} cost={} gold={}",
            player.hero.identity.id,
            id.id(),
            item(id).cost,
            player.economy.gold
        );
    }
    player.economy.last_purchase = Some(PurchaseReceipt {
        request_id,
        match_id,
        item_id,
        error,
    });
}

pub fn award_gold(player: &mut ConnectedPlayer, amount: u32) {
    player.economy.gold = player.economy.gold.saturating_add(amount);
    player.economy.earned_gold = player.economy.earned_gold.saturating_add(amount);
}

pub fn accrue_passive_gold(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    phase: &GameState,
    dt: f32,
) {
    if !matches!(phase, GameState::Running) || !dt.is_finite() || dt <= 0.0 {
        return;
    }
    for player in players.values_mut().filter(|player| player.joined) {
        player.economy.gold_income_remainder += dt * GOLD_PER_SECOND;
        let earned = player.economy.gold_income_remainder.floor() as u32;
        player.economy.gold_income_remainder -= earned as f32;
        award_gold(player, earned);
    }
}
