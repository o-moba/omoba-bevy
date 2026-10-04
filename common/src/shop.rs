//! Authoritative base-shop validation, payment and passive income.
use std::collections::HashMap;
use std::net::SocketAddr;

use shared::map::Team;
use shared::shop::{
    GOLD_PER_SECOND, ItemId, PurchaseError, PurchaseReceipt, SHOP_RADIUS, item_bonuses,
    purchase_quote,
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
        purchase_quote(id, player.economy.gold, &player.economy.inventory).err()
    } else {
        Some(PurchaseError::UnknownItem)
    };
    if error.is_none() {
        let id = item_id.expect("validated item");
        let old = player.economy.item_bonuses;
        let quote = purchase_quote(id, player.economy.gold, &player.economy.inventory)
            .expect("validated quote");
        player.economy.gold -= quote.cost;
        player
            .economy
            .inventory
            .retain(|item| !quote.consumed.contains(item));
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
            quote.cost,
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

/// Called after authoritative per-life death deduplication, never from replayed snapshots.
pub fn award_hero_kill(
    world: &mut crate::game_world::GameWorld,
    event: &shared::combat::CombatEvent,
    participants: &[u64],
) {
    use shared::combat::CombatEntityKind;
    if !event.killed
        || event.target.kind != CombatEntityKind::Player
        || event.source.kind != CombatEntityKind::Player
        || event.source.id == event.target.id
    {
        return;
    }
    let Some((victim_team, streak)) = world
        .players
        .values()
        .find(|p| p.hero.identity.id == event.target.id)
        .map(|p| (p.hero.identity.team, p.economy.death_streak))
    else {
        return;
    };
    let Some(killer_team) = world
        .players
        .values()
        .find(|p| p.joined && p.hero.identity.id == event.source.id)
        .map(|p| p.hero.identity.team)
        .filter(|team| *team != victim_team)
    else {
        return;
    };
    let mut assistants: Vec<_> = world
        .players
        .values()
        .filter(|p| {
            p.joined
                && p.hero.identity.team == killer_team
                && p.hero.identity.id != event.source.id
                && participants.contains(&p.hero.identity.id)
        })
        .map(|p| p.hero.identity.id)
        .collect();
    assistants.sort_unstable();
    for player in world.players.values_mut() {
        if player.hero.identity.id == event.target.id {
            player.economy.death_streak = player.economy.death_streak.saturating_add(1);
        } else if player.hero.identity.id == event.source.id {
            award_gold(player, shared::shop::hero_kill_bounty(streak));
            player.economy.death_streak = 0;
        } else if let Some(index) = assistants
            .iter()
            .position(|id| *id == player.hero.identity.id)
        {
            let pool = shared::shop::HERO_ASSIST_POOL;
            award_gold(
                player,
                pool / assistants.len() as u32
                    + u32::from((index as u32) < pool % assistants.len() as u32),
            );
        }
    }
}

pub fn basic_lifesteal(world: &mut crate::game_world::GameWorld, owner: u64, damage: f32) {
    if !damage.is_finite() || damage <= 0.0 {
        return;
    }
    if let Some(player) = world
        .players
        .values_mut()
        .find(|p| p.joined && p.hero.identity.id == owner && p.hero.hp > 0.0)
    {
        let healing = damage * player.economy.item_bonuses.lifesteal.clamp(0.0, 0.35);
        player.hero.hp = (player.hero.hp + healing).min(player.hero.max_hp);
    }
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

#[cfg(test)]
mod economy_tests {
    use super::*;
    use crate::game_world::GameWorld;
    use shared::combat::{CombatEntity, CombatEntityKind, CombatEvent};
    use shared::shop::ItemId::*;
    use std::time::{Duration, Instant};

    fn fixture() -> (GameWorld, SocketAddr, Instant) {
        let now = Instant::now();
        let mut world = GameWorld::empty();
        let address = "127.0.0.1:61001".parse().unwrap();
        world.ensure_connected(address, now);
        let player = world.players.get_mut(&address).unwrap();
        player.joined = true;
        player.hero.identity.team = Team::Green;
        player.hero.x = world.map_layout.home.x;
        player.hero.z = world.map_layout.home.z;
        player.hero.hp = 100.0;
        player.hero.max_hp = 100.0;
        (world, address, now)
    }

    #[test]
    fn recipe_credit_is_atomic_replay_safe_and_upgrades_a_full_inventory() {
        let (mut world, address, _) = fixture();
        let player = world.players.get_mut(&address).unwrap();
        player.economy.inventory = vec![
            EmberBlade,
            CritShard,
            SwiftGrip,
            TrailBoots,
            FocusCharm,
            SiphonStone,
        ];
        player.economy.item_bonuses = item_bonuses(&player.economy.inventory);
        player.economy.gold = 470;
        handle_purchase(
            player,
            &world.map_layout,
            &GameState::Running,
            DuelistEdge.id(),
            1,
            7,
        );
        assert!(
            player
                .economy
                .last_purchase
                .as_ref()
                .unwrap()
                .error
                .is_none()
        );
        assert_eq!(player.economy.gold, 0);
        assert_eq!(player.economy.inventory.len(), 5);
        assert!(
            !player.economy.inventory.contains(&EmberBlade)
                && !player.economy.inventory.contains(&CritShard)
        );
        assert!((player.economy.item_bonuses.damage_multiplier - 1.24).abs() < 0.0001);
        assert_eq!(player.economy.item_bonuses.crit_chance, 0.20);
        let economy = player.economy.clone();
        handle_purchase(
            player,
            &world.map_layout,
            &GameState::Running,
            DuelistEdge.id(),
            1,
            7,
        );
        assert_eq!(player.economy, economy);
        handle_purchase(
            player,
            &world.map_layout,
            &GameState::Running,
            TempestBlade.id(),
            2,
            7,
        );
        assert_eq!(
            player.economy.last_purchase.as_ref().unwrap().error,
            Some(PurchaseError::InsufficientGold)
        );
        assert_eq!(player.economy.inventory, economy.inventory);
        assert_eq!(player.economy.gold, 0);
    }

    #[test]
    fn hero_bounty_replays_pay_once_death_streak_falls_and_assists_share_one_pool() {
        let (mut world, killer, now) = fixture();
        let victim: SocketAddr = "127.0.0.1:61002".parse().unwrap();
        let helper: SocketAddr = "127.0.0.1:61003".parse().unwrap();
        for address in [victim, helper] {
            world.ensure_connected(address, now);
            let player = world.players.get_mut(&address).unwrap();
            player.joined = true;
            player.hero.identity.team = if address == victim {
                Team::Blue
            } else {
                Team::Green
            };
        }
        let killer_id = world.players[&killer].hero.identity.id;
        let victim_id = world.players[&victim].hero.identity.id;
        let helper_id = world.players[&helper].hero.identity.id;
        let event = |source, killed| CombatEvent {
            source: CombatEntity {
                kind: CombatEntityKind::Player,
                id: source,
            },
            target: CombatEntity {
                kind: CombatEntityKind::Player,
                id: victim_id,
            },
            amount: 10.0,
            killed,
            ..Default::default()
        };
        for (index, expected) in [500, 400, 300, 250, 250].into_iter().enumerate() {
            world.players.get_mut(&victim).unwrap().hero.hp = 100.0;
            crate::skills::normalize(&mut world, now);
            crate::skills::observe(&mut world, &[event(helper_id, false)], now);
            world.players.get_mut(&victim).unwrap().hero.hp = 0.0;
            let before = world.players[&killer].economy.earned_gold;
            crate::skills::observe(&mut world, &[event(killer_id, true)], now);
            crate::skills::observe(&mut world, &[event(killer_id, true)], now);
            assert_eq!(
                world.players[&killer].economy.earned_gold - before,
                expected
            );
            assert_eq!(
                world.players[&helper].economy.earned_gold,
                (index as u32 + 1) * shared::shop::HERO_ASSIST_POOL
            );
        }
        assert_eq!(world.players[&victim].economy.death_streak, 5);
        // A legitimate opposing hero kill resets the victim's reduced bounty.
        let event = CombatEvent {
            source: CombatEntity {
                kind: CombatEntityKind::Player,
                id: victim_id,
            },
            target: CombatEntity {
                kind: CombatEntityKind::Player,
                id: killer_id,
            },
            amount: 10.0,
            killed: true,
            ..Default::default()
        };
        crate::skills::observe(&mut world, &[event], now);
        assert_eq!(world.players[&victim].economy.death_streak, 0);
    }

    #[test]
    fn running_clock_scales_death_deadline_without_rewriting_existing_deaths() {
        let (mut world, address, now) = fixture();
        let id = world.players[&address].hero.identity.id;
        world.game_state = GameState::Lobby;
        crate::tick::advance_match_clock(&mut world, 600.0);
        assert_eq!(world.match_elapsed_secs, 0.0);
        world.game_state = GameState::Running;
        crate::tick::advance_match_clock(&mut world, 600.0);
        assert_eq!(world.match_elapsed_secs, 600.0);
        assert_eq!(
            world.players[&address].timers.respawn_delay,
            Duration::from_secs(17)
        );
        crate::combat_feedback::apply_player_damage(&mut world.players, id, 10_000.0, now);
        assert_eq!(
            world.players[&address].timers.respawn_at,
            Some(now + Duration::from_secs(17))
        );
        crate::tick::advance_match_clock(&mut world, 600.0);
        assert_eq!(
            world.players[&address].timers.respawn_at,
            Some(now + Duration::from_secs(17))
        );
        crate::session::handle_respawns(&mut world, now + Duration::from_secs(16));
        assert_eq!(world.players[&address].hero.hp, 0.0);
        crate::session::handle_respawns(&mut world, now + Duration::from_secs(17));
        assert!(world.players[&address].hero.hp > 0.0);
        let elapsed = world.match_elapsed_secs;
        world.game_state = GameState::Victory {
            winner: Team::Green,
        };
        crate::tick::advance_match_clock(&mut world, 500.0);
        assert_eq!(world.match_elapsed_secs, elapsed);
        world.reset_round(now);
        assert_eq!(world.match_elapsed_secs, 0.0);
    }

    #[test]
    fn basic_lifesteal_uses_actual_primary_damage_and_spells_do_not_heal() {
        let (mut world, attacker, now) = fixture();
        let target: SocketAddr = "127.0.0.1:61002".parse().unwrap();
        world.ensure_connected(target, now);
        let owner = world.players[&attacker].hero.identity.id;
        let victim = world.players.get_mut(&target).unwrap();
        victim.joined = true;
        victim.hero.identity.team = Team::Blue;
        victim.hero.hp = 10.0;
        let target = shared::wire::TargetId {
            kind: shared::wire::TargetKind::Player,
            id: victim.hero.identity.id,
        };
        let source = crate::combat_feedback::HitSource::new(
            CombatEntityKind::Player,
            owner,
            shared::combat::ProjectileStyle::Arrow,
        );
        let hero = world.players.get_mut(&attacker).unwrap();
        hero.hero.hp = 40.0;
        hero.economy.item_bonuses.lifesteal = 0.20;
        let events = crate::skills::apply_hit(
            &mut world,
            target,
            1_000.0,
            shared::loadout::DamageType::Physical,
            source,
            Team::Green,
            true,
            false,
            false,
            now,
        );
        assert_eq!(events.iter().map(|event| event.amount).sum::<f32>(), 10.0);
        assert_eq!(
            world.players[&attacker].hero.hp, 42.0,
            "overkill is not healing"
        );
        world
            .players
            .values_mut()
            .find(|p| p.hero.identity.id == target.id)
            .unwrap()
            .hero
            .hp = 100.0;
        crate::skills::apply_hit(
            &mut world,
            target,
            10.0,
            shared::loadout::DamageType::Magic,
            source,
            Team::Green,
            false,
            false,
            false,
            now,
        );
        assert_eq!(world.players[&attacker].hero.hp, 42.0);
    }
}
