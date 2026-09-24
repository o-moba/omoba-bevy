//! Authoritative base-shop validation, payment and passive income.
use super::*;
use shared::shop::{
    GOLD_PER_SECOND, INVENTORY_CAPACITY, PurchaseError, SHOP_RADIUS, item, item_bonuses,
};

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

pub(crate) fn shop_is_available(hero: &Hero, map: &MapLayoutState, phase: &GameState) -> bool {
    hero.hp > 0.0 && phase_allows_shop(phase) && inside_own_shop(hero, map)
}

pub(crate) fn handle_purchase(
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
    } else if !inside_own_shop(&player.hero, map) {
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
        let mana_bonus = player.economy.item_bonuses.max_mana - old.max_mana;
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

pub(crate) fn award_gold(player: &mut ConnectedPlayer, amount: u32) {
    player.economy.gold = player.economy.gold.saturating_add(amount);
    player.economy.earned_gold = player.economy.earned_gold.saturating_add(amount);
}

pub(crate) fn accrue_passive_gold(
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
mod tests {
    use super::*;

    fn join(team: Team, session: &str) -> ClientPacket {
        ClientPacket::Join {
            prematch: false,
            team,
            character: CharacterChoice::Ipfs,
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            session_id: Some(session.to_owned()),
            passport_ticket: None,
        }
    }

    fn fixture() -> (ServerRuntime, SocketAddr, Instant) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
        let addr = "127.0.0.1:57001".parse().unwrap();
        let now = Instant::now();
        rt.handle_packet(addr, join(Team::Green, "shopper"), now);
        (rt, addr, now)
    }

    fn buy(rt: &mut ServerRuntime, addr: SocketAddr, item: ItemId, request_id: u64, now: Instant) {
        rt.handle_packet(
            addr,
            ClientPacket::BuyItem {
                item_id: item.id().to_owned(),
                request_id,
                match_id: rt.match_id,
                server_epoch: rt.server_epoch,
            },
            now,
        );
    }

    #[test]
    fn decoded_udp_purchase_is_atomic_and_retries_cannot_double_spend() {
        let (mut rt, _, _) = fixture();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = client.local_addr().unwrap();
        rt.handle_packet(addr, join(Team::Blue, "udp-shopper"), Instant::now());
        let bytes = serde_json::to_vec(&ClientPacket::BuyItem {
            item_id: "vitality_gem".to_owned(),
            request_id: 1,
            match_id: rt.match_id,
            server_epoch: rt.server_epoch,
        })
        .unwrap();
        client
            .send_to(&bytes, rt.transport.local_addr().unwrap())
            .unwrap();
        client
            .send_to(&bytes, rt.transport.local_addr().unwrap())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while rt.world.players[&addr].economy.last_purchase.is_none() && Instant::now() < deadline {
            rt.receive_packets();
        }
        rt.receive_packets();
        let player = &rt.world.players[&addr];
        assert_eq!(player.economy.gold, 0);
        assert_eq!(player.economy.inventory, [ItemId::VitalityGem]);
        assert_eq!((player.hero.hp, player.hero.max_hp), (210.0, 210.0));
        assert_eq!(player.economy.last_purchase.as_ref().unwrap().error, None);
    }

    #[test]
    fn stale_server_epoch_udp_purchase_cannot_spend_or_poison_a_restarted_round() {
        let (old, _, _) = fixture();
        let (mut rt, _, now) = fixture();
        assert_eq!(old.match_id, rt.match_id);
        assert_ne!(old.server_epoch, rt.server_epoch);
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = client.local_addr().unwrap();
        rt.handle_packet(addr, join(Team::Blue, "restart-shopper"), now);
        let stale = serde_json::to_vec(&ClientPacket::BuyItem {
            item_id: "ember_blade".to_owned(),
            request_id: 999,
            match_id: old.match_id,
            server_epoch: old.server_epoch,
        })
        .unwrap();
        client
            .send_to(&stale, rt.transport.local_addr().unwrap())
            .unwrap();
        rt.receive_packets();
        assert_eq!(rt.world.players[&addr].economy.purchase_sequence, 0);
        assert_eq!(rt.world.players[&addr].economy.gold, STARTING_GOLD);
        assert!(rt.world.players[&addr].economy.inventory.is_empty());
        assert!(rt.world.players[&addr].economy.last_purchase.is_none());
        buy(&mut rt, addr, ItemId::EmberBlade, 1, Instant::now());
        assert_eq!(
            rt.world.players[&addr].economy.inventory,
            [ItemId::EmberBlade]
        );
        assert!(
            serde_json::from_str::<ClientPacket>(
                r#"{"type":"buy_item","item_id":"ember_blade","request_id":1,"match_id":1}"#
            )
            .is_err()
        );
    }

    #[test]
    fn purchase_rejections_never_change_wallet_inventory_or_resources() {
        for expected in [
            PurchaseError::Unavailable,
            PurchaseError::Dead,
            PurchaseError::OutsideBase,
            PurchaseError::InsufficientGold,
            PurchaseError::AlreadyOwned,
            PurchaseError::InventoryFull,
            PurchaseError::UnknownItem,
        ] {
            let (mut rt, addr, now) = fixture();
            let player = rt.world.players.get_mut(&addr).unwrap();
            match expected {
                PurchaseError::Unavailable => {
                    rt.world.game_state = GameState::Victory {
                        winner: Team::Green,
                    }
                }
                PurchaseError::Dead => player.hero.hp = 0.0,
                PurchaseError::OutsideBase => {
                    player.hero.x = rt.world.map_layout.away.x;
                    player.hero.z = rt.world.map_layout.away.z;
                }
                PurchaseError::InsufficientGold => player.economy.gold = 79,
                PurchaseError::AlreadyOwned => player.economy.inventory.push(ItemId::EmberBlade),
                PurchaseError::InventoryFull => {
                    player.economy.inventory =
                        shared::shop::ITEMS.iter().map(|item| item.id).collect()
                }
                PurchaseError::UnknownItem => {}
            }
            let before = rt.player_view(addr, now);
            rt.handle_packet(
                addr,
                ClientPacket::BuyItem {
                    item_id: if expected == PurchaseError::UnknownItem {
                        "unknown"
                    } else {
                        "ember_blade"
                    }
                    .to_owned(),
                    request_id: 1,
                    match_id: rt.match_id,
                    server_epoch: rt.server_epoch,
                },
                now,
            );
            let after = rt.player_view(addr, now);
            assert_eq!(after.last_purchase.as_ref().unwrap().error, Some(expected));
            assert_eq!(
                (
                    after.gold,
                    &after.inventory,
                    after.hp,
                    after.max_hp,
                    after.mana,
                    after.max_mana
                ),
                (
                    before.gold,
                    &before.inventory,
                    before.hp,
                    before.max_hp,
                    before.mana,
                    before.max_mana
                )
            );
        }
        let (mut rt, _, now) = fixture();
        let prejoin = "127.0.0.1:57009".parse().unwrap();
        buy(&mut rt, prejoin, ItemId::EmberBlade, 1, now);
        assert_eq!(
            rt.world.players[&prejoin]
                .economy
                .last_purchase
                .as_ref()
                .unwrap()
                .error,
            Some(PurchaseError::Unavailable)
        );
    }

    #[test]
    fn shop_boundary_is_own_base_and_dead_and_victory_are_unavailable() {
        let (rt, addr, _) = fixture();
        let mut state = rt.world.players[&addr].hero.clone();
        for (team, base) in [
            (Team::Green, rt.world.map_layout.home),
            (Team::Blue, rt.world.map_layout.away),
        ] {
            state.identity.team = team;
            state.x = base.x + SHOP_RADIUS;
            state.z = base.z;
            assert!(shop_is_available(
                &state,
                &rt.world.map_layout,
                &GameState::Running
            ));
            state.x += 0.01;
            assert!(!shop_is_available(
                &state,
                &rt.world.map_layout,
                &GameState::Running
            ));
            state.x = base.x;
            state.hp = 0.0;
            assert!(!shop_is_available(
                &state,
                &rt.world.map_layout,
                &GameState::Running
            ));
            state.hp = 100.0;
            assert!(!shop_is_available(
                &state,
                &rt.world.map_layout,
                &GameState::Victory { winner: team }
            ));
        }
    }

    #[test]
    fn purchases_survive_death_respawn_reconnect_and_reset_before_next_round() {
        let (mut rt, addr, now) = fixture();
        buy(&mut rt, addr, ItemId::VitalityGem, 9, now);
        let player = rt.world.players.get_mut(&addr).unwrap();
        player.hero.hp = 0.0;
        player.timers.respawn_at = Some(now);
        handle_respawns(&mut rt.world, now);
        assert_eq!(rt.world.players[&addr].hero.hp, 210.0);
        let new_addr = "127.0.0.1:57002".parse().unwrap();
        rt.handle_packet(
            new_addr,
            join(Team::Blue, "shopper"),
            now + PLAYER_TIMEOUT + Duration::from_millis(1),
        );
        let player = &rt.world.players[&new_addr];
        assert_eq!(player.economy.inventory, [ItemId::VitalityGem]);
        assert_eq!(player.economy.purchase_sequence, 9);
        assert_eq!(player.economy.gold, 0);
        let old_round = rt.match_id;
        rt.restart_round(now + PLAYER_TIMEOUT + Duration::from_millis(2));
        let player = &rt.world.players[&new_addr];
        assert!(player.economy.inventory.is_empty() && player.economy.last_purchase.is_none());
        assert_eq!(player.economy.item_bonuses, ItemBonuses::NONE);
        assert_eq!(player.economy.gold, STARTING_GOLD);
        rt.handle_packet(
            new_addr,
            ClientPacket::BuyItem {
                item_id: "ember_blade".to_owned(),
                request_id: 999,
                match_id: old_round,
                server_epoch: rt.server_epoch,
            },
            now + PLAYER_TIMEOUT + Duration::from_millis(3),
        );
        assert_eq!(rt.world.players[&new_addr].economy.purchase_sequence, 0);
        buy(
            &mut rt,
            new_addr,
            ItemId::EmberBlade,
            1,
            now + PLAYER_TIMEOUT + Duration::from_millis(4),
        );
        assert_eq!(
            rt.world.players[&new_addr].economy.inventory,
            [ItemId::EmberBlade]
        );
    }

    #[test]
    fn equipment_changes_authoritative_projectile_damage_and_both_cooldown_groups() {
        let (mut rt, addr, now) = fixture();
        rt.world.players.get_mut(&addr).unwrap().economy.gold = 1000;
        for (index, id) in [
            ItemId::EmberBlade,
            ItemId::GuardianCrest,
            ItemId::SwiftGrip,
            ItemId::FocusCharm,
        ]
        .into_iter()
        .enumerate()
        {
            buy(&mut rt, addr, id, index as u64 + 1, now);
        }
        let enemy_addr = "127.0.0.1:57003".parse().unwrap();
        rt.handle_packet(enemy_addr, join(Team::Blue, "target"), now);
        let (x, z) = (
            rt.world.players[&addr].hero.x,
            rt.world.players[&addr].hero.z,
        );
        let enemy = rt.world.players.get_mut(&enemy_addr).unwrap();
        enemy.hero.x = x + 1.0;
        enemy.hero.z = z;
        let target = TargetId {
            kind: TargetKind::Player,
            id: enemy.hero.identity.id,
        };
        rt.world.players.get_mut(&addr).unwrap().hero.progress.level = 6;
        let mut cast_at = now;
        for slot in [SkillSlot::Q, SkillSlot::E] {
            let def = ability_for_class_slot(HeroClass::Mage, slot);
            let packet = ClientPacket::Cast {
                target,
                slot: slot.index() as u8,
            };
            rt.world.players.get_mut(&addr).unwrap().hero.mana = 1000.0;
            rt.handle_packet(addr, packet.clone(), cast_at);
            let count = rt.world.projectiles.len();
            let cooldown = hero_stats::ability_cooldown(&rt.world.players[&addr], slot);
            let later = cast_at + cooldown + Duration::from_millis(1);
            assert!(later < cast_at + scaled_cooldown(def, 1));
            rt.handle_packet(addr, packet, later);
            cast_at = later + Duration::from_secs(1);
            assert_eq!(
                rt.world.projectiles.len(),
                count + 1,
                "item haste must change accepted casts"
            );
        }
        for projectile in rt.world.projectiles.values() {
            let expected = if projectile.state.id <= 2 {
                ability_for_class_slot(HeroClass::Mage, SkillSlot::Q)
            } else {
                ability_for_class_slot(HeroClass::Mage, SkillSlot::E)
            };
            assert!(
                (projectile.damage
                    - expected.projectile_damage.unwrap()
                        * 1.18
                        * shared::hero_balance::ability_power_multiplier(HeroClass::Mage, 6))
                .abs()
                    < 0.001
            );
        }
        assert_eq!(rt.world.players[&addr].hero.max_mana, 120.0);
        assert_eq!(rt.world.players[&addr].hero.max_hp, 195.0);
    }

    #[test]
    fn boots_change_authoritative_movement_allowance_and_level_growth_keeps_items() {
        let (mut rt, addr, now) = fixture();
        rt.world.players.get_mut(&addr).unwrap().economy.gold = 1000;
        buy(&mut rt, addr, ItemId::TrailBoots, 1, now);
        buy(&mut rt, addr, ItemId::VitalityGem, 2, now);
        let player = rt.world.players.get_mut(&addr).unwrap();
        let start = player.hero.x;
        handle_transform_request(
            player,
            &rt.world.map_layout,
            start + 100.0,
            0.5,
            player.hero.z,
            0.0,
            now + Duration::from_millis(100),
        );
        let expected = PLAYER_SPEED * 1.08 * 0.1 + MOVEMENT_POSITION_TOLERANCE;
        assert!((player.hero.x - start - expected).abs() < 0.001);
        apply_level_up(&mut player.hero);
        assert_eq!(
            player.hero.max_hp,
            shared::hero_balance::base_hp(HeroClass::Mage) + 30.0 + LEVEL_UP_HP_BONUS
        );
        assert_eq!(
            player.economy.inventory,
            [ItemId::TrailBoots, ItemId::VitalityGem]
        );
    }

    #[test]
    fn passive_income_accumulates_fractions_only_during_running_including_dead_players() {
        let (mut rt, addr, now) = fixture();
        let prejoin = "127.0.0.1:57004".parse().unwrap();
        rt.handle_packet(prejoin, ClientPacket::Ping, now);
        rt.world.players.get_mut(&addr).unwrap().hero.hp = 0.0;
        for _ in 0..4 {
            accrue_passive_gold(&mut rt.world.players, &GameState::Running, 0.25);
        }
        assert_eq!(rt.world.players[&addr].economy.gold, STARTING_GOLD + 1);
        assert_eq!(rt.world.players[&prejoin].economy.gold, STARTING_GOLD);
        for phase in [
            GameState::Lobby,
            GameState::Starting { countdown_ms: 3000 },
            GameState::Victory {
                winner: Team::Green,
            },
        ] {
            accrue_passive_gold(&mut rt.world.players, &phase, 10.0);
        }
        assert_eq!(rt.world.players[&addr].economy.gold, STARTING_GOLD + 1);
    }

    #[test]
    fn rejected_request_retries_need_a_new_id_and_reserved_sessions_do_not_earn() {
        let (mut rt, addr, now) = fixture();
        buy(&mut rt, addr, ItemId::FocusCharm, 1, now);
        assert_eq!(
            rt.world.players[&addr]
                .economy
                .last_purchase
                .as_ref()
                .unwrap()
                .error,
            Some(PurchaseError::InsufficientGold)
        );
        rt.world.players.get_mut(&addr).unwrap().economy.gold = 100;
        buy(&mut rt, addr, ItemId::FocusCharm, 1, now);
        assert_eq!(rt.world.players[&addr].economy.gold, 100);
        assert!(rt.world.players[&addr].economy.inventory.is_empty());
        buy(&mut rt, addr, ItemId::FocusCharm, 2, now);
        assert_eq!(
            rt.world.players[&addr].economy.inventory,
            [ItemId::FocusCharm]
        );
        rt.maintain_roster(now + PLAYER_TIMEOUT + Duration::from_millis(1));
        assert!(!rt.world.players.contains_key(&addr));
        assert_eq!(
            rt.world.disconnected_sessions["shopper"]
                .player
                .economy
                .gold,
            0
        );
        accrue_passive_gold(&mut rt.world.players, &GameState::Running, 3.0);
        assert_eq!(
            rt.world.disconnected_sessions["shopper"]
                .player
                .economy
                .gold,
            0
        );
    }
    #[test]
    fn earned_gold_is_income_not_wallet_and_survives_purchase_respawn_and_reconnect() {
        let (mut rt, addr, now) = fixture();
        assert_eq!(rt.world.players[&addr].economy.earned_gold, 0);
        accrue_passive_gold(&mut rt.world.players, &GameState::Running, 10.0);
        award_gold(rt.world.players.get_mut(&addr).unwrap(), 200);
        let earned = rt.world.players[&addr].economy.earned_gold;
        assert_eq!(earned, 210);
        buy(&mut rt, addr, ItemId::TrailBoots, 1, now);
        assert_eq!(rt.world.players[&addr].economy.earned_gold, earned);
        assert_ne!(rt.world.players[&addr].economy.gold, STARTING_GOLD + earned);
        rt.world.players.get_mut(&addr).unwrap().hero.hp = 0.0;
        rt.world.players.get_mut(&addr).unwrap().timers.respawn_at = Some(now);
        handle_respawns(&mut rt.world, now);
        assert_eq!(rt.world.players[&addr].economy.earned_gold, earned);
        let reconnect_at = now + PLAYER_TIMEOUT + Duration::from_millis(1);
        rt.maintain_roster(reconnect_at);
        let other = "127.0.0.1:57999".parse().unwrap();
        rt.handle_packet(other, join(Team::Green, "shopper"), reconnect_at);
        assert_eq!(rt.world.players[&other].economy.earned_gold, earned);
        reset_player_round(
            rt.world.players.get_mut(&other).unwrap(),
            &rt.world.map_layout,
            reconnect_at,
        );
        assert_eq!(rt.world.players[&other].economy.earned_gold, 0);
        assert_eq!(rt.world.players[&other].economy.gold, STARTING_GOLD);
    }
}
