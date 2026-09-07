//! Authoritative base-shop validation, payment and passive income.
use super::*;
use shared::shop::{
    GOLD_PER_SECOND, INVENTORY_CAPACITY, PurchaseError, SHOP_RADIUS, item, item_bonuses,
};

fn phase_allows_shop(phase: &GameState) -> bool {
    !matches!(phase, GameState::Victory { .. })
}

fn inside_own_shop(state: &PlayerState, map: &MapLayoutState) -> bool {
    let base = match state.team {
        Team::Green => map.home,
        Team::Blue => map.away,
    };
    (state.x - base.x).powi(2) + (state.z - base.z).powi(2) <= SHOP_RADIUS * SHOP_RADIUS
}

pub(crate) fn shop_is_available(
    state: &PlayerState,
    map: &MapLayoutState,
    phase: &GameState,
) -> bool {
    state.hp > 0.0 && phase_allows_shop(phase) && inside_own_shop(state, map)
}

pub(crate) fn handle_purchase(
    player: &mut ConnectedPlayer,
    map: &MapLayoutState,
    phase: &GameState,
    raw_item: &str,
    request_id: u64,
    match_id: u64,
) {
    if request_id == 0 || request_id <= player.purchase_sequence {
        return;
    }
    player.purchase_sequence = request_id;
    let item_id = ItemId::from_id(raw_item);
    let error = if !player.joined || !phase_allows_shop(phase) {
        Some(PurchaseError::Unavailable)
    } else if player.state.hp <= 0.0 {
        Some(PurchaseError::Dead)
    } else if !inside_own_shop(&player.state, map) {
        Some(PurchaseError::OutsideBase)
    } else if let Some(id) = item_id {
        if player.state.inventory.len() >= INVENTORY_CAPACITY {
            Some(PurchaseError::InventoryFull)
        } else if player.state.inventory.contains(&id) {
            Some(PurchaseError::AlreadyOwned)
        } else if player.state.gold < item(id).cost {
            Some(PurchaseError::InsufficientGold)
        } else {
            None
        }
    } else {
        Some(PurchaseError::UnknownItem)
    };
    if error.is_none() {
        let id = item_id.expect("validated item");
        let old = player.state.item_bonuses;
        player.state.gold -= item(id).cost;
        player.state.inventory.push(id);
        player.state.item_bonuses = item_bonuses(&player.state.inventory);
        let hp_bonus = player.state.item_bonuses.max_hp - old.max_hp;
        let mana_bonus = player.state.item_bonuses.max_mana - old.max_mana;
        player.state.max_hp += hp_bonus;
        player.state.hp = (player.state.hp + hp_bonus).min(player.state.max_hp);
        player.state.max_mana += mana_bonus;
        player.state.mana = (player.state.mana + mana_bonus).min(player.state.max_mana);
        println!(
            "MATCH_METRIC event=purchase match={match_id} player={} request={request_id} item={} cost={} gold={}",
            player.state.id,
            id.id(),
            item(id).cost,
            player.state.gold
        );
    }
    player.state.last_purchase = Some(PurchaseReceipt {
        request_id,
        match_id,
        item_id,
        error,
    });
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
        player.gold_income_remainder += dt * GOLD_PER_SECOND;
        let earned = player.gold_income_remainder.floor() as u32;
        player.gold_income_remainder -= earned as f32;
        player.state.gold = player.state.gold.saturating_add(earned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(team: Team, session: &str) -> ClientPacket {
        ClientPacket::Join {
            team,
            character: CharacterChoice::Ipfs,
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            session_id: Some(session.to_owned()),
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
            .send_to(&bytes, rt.socket.local_addr().unwrap())
            .unwrap();
        client
            .send_to(&bytes, rt.socket.local_addr().unwrap())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while rt.players[&addr].state.last_purchase.is_none() && Instant::now() < deadline {
            rt.receive_packets();
        }
        rt.receive_packets();
        let player = &rt.players[&addr];
        assert_eq!(player.state.gold, 0);
        assert_eq!(player.state.inventory, [ItemId::VitalityGem]);
        assert_eq!((player.state.hp, player.state.max_hp), (130.0, 130.0));
        assert_eq!(player.state.last_purchase.as_ref().unwrap().error, None);
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
            .send_to(&stale, rt.socket.local_addr().unwrap())
            .unwrap();
        rt.receive_packets();
        assert_eq!(rt.players[&addr].purchase_sequence, 0);
        assert_eq!(rt.players[&addr].state.gold, STARTING_GOLD);
        assert!(rt.players[&addr].state.inventory.is_empty());
        assert!(rt.players[&addr].state.last_purchase.is_none());
        buy(&mut rt, addr, ItemId::EmberBlade, 1, Instant::now());
        assert_eq!(rt.players[&addr].state.inventory, [ItemId::EmberBlade]);
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
            let player = rt.players.get_mut(&addr).unwrap();
            match expected {
                PurchaseError::Unavailable => {
                    rt.game_state = GameState::Victory {
                        winner: Team::Green,
                    }
                }
                PurchaseError::Dead => player.state.hp = 0.0,
                PurchaseError::OutsideBase => {
                    player.state.x = rt.map_layout.away.x;
                    player.state.z = rt.map_layout.away.z;
                }
                PurchaseError::InsufficientGold => player.state.gold = 79,
                PurchaseError::AlreadyOwned => player.state.inventory.push(ItemId::EmberBlade),
                PurchaseError::InventoryFull => {
                    player.state.inventory =
                        shared::shop::ITEMS.iter().map(|item| item.id).collect()
                }
                PurchaseError::UnknownItem => {}
            }
            let before = rt.players[&addr].state.clone();
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
            let after = &rt.players[&addr].state;
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
            rt.players[&prejoin]
                .state
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
        let mut state = rt.players[&addr].state.clone();
        for (team, base) in [
            (Team::Green, rt.map_layout.home),
            (Team::Blue, rt.map_layout.away),
        ] {
            state.team = team;
            state.x = base.x + SHOP_RADIUS;
            state.z = base.z;
            assert!(shop_is_available(
                &state,
                &rt.map_layout,
                &GameState::Running
            ));
            state.x += 0.01;
            assert!(!shop_is_available(
                &state,
                &rt.map_layout,
                &GameState::Running
            ));
            state.x = base.x;
            state.hp = 0.0;
            assert!(!shop_is_available(
                &state,
                &rt.map_layout,
                &GameState::Running
            ));
            state.hp = 100.0;
            assert!(!shop_is_available(
                &state,
                &rt.map_layout,
                &GameState::Victory { winner: team }
            ));
        }
    }

    #[test]
    fn purchases_survive_death_respawn_reconnect_and_reset_before_next_round() {
        let (mut rt, addr, now) = fixture();
        buy(&mut rt, addr, ItemId::VitalityGem, 9, now);
        let player = rt.players.get_mut(&addr).unwrap();
        player.state.hp = 0.0;
        player.respawn_at = Some(now);
        handle_respawns(
            &mut rt.players,
            &rt.structures,
            &rt.map_layout,
            &rt.game_state,
            now,
        );
        assert_eq!(rt.players[&addr].state.hp, 130.0);
        let new_addr = "127.0.0.1:57002".parse().unwrap();
        rt.handle_packet(
            new_addr,
            join(Team::Blue, "shopper"),
            now + PLAYER_TIMEOUT + Duration::from_millis(1),
        );
        let player = &rt.players[&new_addr];
        assert_eq!(player.state.inventory, [ItemId::VitalityGem]);
        assert_eq!(player.purchase_sequence, 9);
        assert_eq!(player.state.gold, 0);
        let old_round = rt.match_id;
        rt.restart_round(now + PLAYER_TIMEOUT + Duration::from_millis(2));
        let player = &rt.players[&new_addr];
        assert!(player.state.inventory.is_empty() && player.state.last_purchase.is_none());
        assert_eq!(player.state.item_bonuses, ItemBonuses::NONE);
        assert_eq!(player.state.gold, STARTING_GOLD);
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
        assert_eq!(rt.players[&new_addr].purchase_sequence, 0);
        buy(
            &mut rt,
            new_addr,
            ItemId::EmberBlade,
            1,
            now + PLAYER_TIMEOUT + Duration::from_millis(4),
        );
        assert_eq!(rt.players[&new_addr].state.inventory, [ItemId::EmberBlade]);
    }

    #[test]
    fn equipment_changes_authoritative_projectile_damage_and_both_cooldown_groups() {
        let (mut rt, addr, now) = fixture();
        rt.players.get_mut(&addr).unwrap().state.gold = 1000;
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
        let (x, z) = (rt.players[&addr].state.x, rt.players[&addr].state.z);
        let enemy = rt.players.get_mut(&enemy_addr).unwrap();
        enemy.state.x = x + 1.0;
        enemy.state.z = z;
        let target = TargetId {
            kind: TargetKind::Player,
            id: enemy.state.id,
        };
        rt.players.get_mut(&addr).unwrap().state.level = 6;
        for slot in [SkillSlot::Q, SkillSlot::E] {
            let def = ability_for_class_slot(HeroClass::Mage, slot);
            let packet = ClientPacket::Cast {
                target,
                slot: slot.index() as u8,
            };
            rt.players.get_mut(&addr).unwrap().state.mana = 1000.0;
            rt.handle_packet(addr, packet.clone(), now);
            let count = rt.projectiles.len();
            let cooldown = item_cooldown(def, 1, slot, rt.players[&addr].state.item_bonuses);
            let later = now + cooldown + Duration::from_millis(1);
            assert!(later < now + scaled_cooldown(def, 1));
            rt.handle_packet(addr, packet, later);
            assert_eq!(
                rt.projectiles.len(),
                count + 1,
                "item haste must change accepted casts"
            );
        }
        for projectile in rt.projectiles.values() {
            let expected = if projectile.state.id <= 2 {
                ability_for_class_slot(HeroClass::Mage, SkillSlot::Q)
            } else {
                ability_for_class_slot(HeroClass::Mage, SkillSlot::E)
            };
            assert!((projectile.damage - expected.projectile_damage.unwrap() * 1.18).abs() < 0.001);
        }
        assert_eq!(rt.players[&addr].state.max_mana, 120.0);
        assert_eq!(rt.players[&addr].state.max_hp, 115.0);
    }

    #[test]
    fn boots_change_authoritative_movement_allowance_and_level_growth_keeps_items() {
        let (mut rt, addr, now) = fixture();
        rt.players.get_mut(&addr).unwrap().state.gold = 1000;
        buy(&mut rt, addr, ItemId::TrailBoots, 1, now);
        buy(&mut rt, addr, ItemId::VitalityGem, 2, now);
        let player = rt.players.get_mut(&addr).unwrap();
        let start = player.state.x;
        handle_transform_request(
            player,
            &rt.map_layout,
            start + 100.0,
            0.5,
            player.state.z,
            0.0,
            now + Duration::from_millis(100),
        );
        let expected = PLAYER_SPEED * 1.08 * 0.1 + MOVEMENT_POSITION_TOLERANCE;
        assert!((player.state.x - start - expected).abs() < 0.001);
        apply_level_up(&mut player.state);
        assert_eq!(player.state.max_hp, MAX_HP + 30.0 + LEVEL_UP_HP_BONUS);
        assert_eq!(
            player.state.inventory,
            [ItemId::TrailBoots, ItemId::VitalityGem]
        );
    }

    #[test]
    fn passive_income_accumulates_fractions_only_during_running_including_dead_players() {
        let (mut rt, addr, now) = fixture();
        let prejoin = "127.0.0.1:57004".parse().unwrap();
        rt.handle_packet(prejoin, ClientPacket::Ping, now);
        rt.players.get_mut(&addr).unwrap().state.hp = 0.0;
        for _ in 0..4 {
            accrue_passive_gold(&mut rt.players, &GameState::Running, 0.25);
        }
        assert_eq!(rt.players[&addr].state.gold, STARTING_GOLD + 1);
        assert_eq!(rt.players[&prejoin].state.gold, STARTING_GOLD);
        for phase in [
            GameState::Lobby,
            GameState::Starting { countdown_ms: 3000 },
            GameState::Victory {
                winner: Team::Green,
            },
        ] {
            accrue_passive_gold(&mut rt.players, &phase, 10.0);
        }
        assert_eq!(rt.players[&addr].state.gold, STARTING_GOLD + 1);
    }

    #[test]
    fn rejected_request_retries_need_a_new_id_and_reserved_sessions_do_not_earn() {
        let (mut rt, addr, now) = fixture();
        buy(&mut rt, addr, ItemId::FocusCharm, 1, now);
        assert_eq!(
            rt.players[&addr]
                .state
                .last_purchase
                .as_ref()
                .unwrap()
                .error,
            Some(PurchaseError::InsufficientGold)
        );
        rt.players.get_mut(&addr).unwrap().state.gold = 100;
        buy(&mut rt, addr, ItemId::FocusCharm, 1, now);
        assert_eq!(rt.players[&addr].state.gold, 100);
        assert!(rt.players[&addr].state.inventory.is_empty());
        buy(&mut rt, addr, ItemId::FocusCharm, 2, now);
        assert_eq!(rt.players[&addr].state.inventory, [ItemId::FocusCharm]);
        rt.maintain_roster(now + PLAYER_TIMEOUT + Duration::from_millis(1));
        assert!(!rt.players.contains_key(&addr));
        assert_eq!(rt.disconnected_sessions["shopper"].player.state.gold, 0);
        accrue_passive_gold(&mut rt.players, &GameState::Running, 3.0);
        assert_eq!(rt.disconnected_sessions["shopper"].player.state.gold, 0);
    }
}
