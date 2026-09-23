//! Server-owned finite healing resources. Movement is the only collection input.
use super::*;
use shared::forest_pickups::{
    FOREST_PICKUP_COUNT, ForestPickupState, HEAL_FRACTION, PICKUP_RADIUS, RESPAWN_SECS,
    pickup_layout,
};

struct Pickup {
    state: ForestPickupState,
    respawn_at: Option<Instant>,
}

pub(crate) struct ForestPickups {
    slots: [Pickup; FOREST_PICKUP_COUNT],
}

impl Default for ForestPickups {
    fn default() -> Self {
        let positions = pickup_layout();
        Self {
            slots: std::array::from_fn(|index| Pickup {
                state: ForestPickupState {
                    id: index as u64 + 1,
                    position: positions[index],
                    available: true,
                    collection_sequence: 0,
                    last_collector_id: None,
                    healed_amount: 0.0,
                },
                respawn_at: None,
            }),
        }
    }
}

impl ForestPickups {
    /// Sandbox resets retain the match id, so their receipt high-water marks
    /// must remain monotonic. A new round instead constructs a fresh instance.
    pub(crate) fn reset_availability(&mut self) {
        for pickup in &mut self.slots {
            pickup.state.available = true;
            pickup.respawn_at = None;
        }
    }

    pub(crate) fn snapshot(&self, game_state: &GameState) -> Vec<ForestPickupState> {
        if !matches!(game_state, GameState::Running) {
            return Vec::new();
        }
        self.slots.iter().map(|slot| slot.state.clone()).collect()
    }

    /// `now` is the simulation clock (including sandbox pause/time scaling).
    pub(crate) fn tick(
        &mut self,
        players: &mut HashMap<SocketAddr, ConnectedPlayer>,
        game_state: &GameState,
        now: Instant,
    ) {
        if !matches!(game_state, GameState::Running) {
            return;
        }
        for pickup in &mut self.slots {
            if pickup.respawn_at.is_some_and(|deadline| now >= deadline) {
                pickup.respawn_at = None;
                pickup.state.available = true;
            }
            if !pickup.state.available {
                continue;
            }
            // HashMap iteration never decides a contested claim. The lowest
            // stable player id among eligible overlapping players wins once.
            let collector = players
                .iter()
                .filter(|(_, player)| {
                    let state = &player.state;
                    player.joined
                        && player.respawn_at.is_none()
                        && state.hp.is_finite()
                        && state.max_hp.is_finite()
                        && state.hp > 0.0
                        && state.hp < state.max_hp
                        && (state.x - pickup.state.position[0])
                            .hypot(state.z - pickup.state.position[1])
                            <= PICKUP_RADIUS
                })
                .min_by_key(|(_, player)| player.state.id)
                .map(|(addr, _)| *addr);
            let Some(player) = collector.and_then(|addr| players.get_mut(&addr)) else {
                continue;
            };
            let state = &mut player.state;
            let healed = (state.max_hp - state.hp).min(state.max_hp * HEAL_FRACTION);
            state.hp = (state.hp + healed).min(state.max_hp);
            pickup.state.available = false;
            pickup.state.collection_sequence = pickup.state.collection_sequence.saturating_add(1);
            pickup.state.last_collector_id = Some(state.id);
            pickup.state.healed_amount = healed;
            pickup.respawn_at = Some(now + Duration::from_secs_f32(RESPAWN_SECS));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (ServerRuntime, SocketAddr, Instant) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
        rt.targeting_qa = true;
        let addr = "127.0.0.1:59241".parse().unwrap();
        let now = Instant::now();
        ensure_player_connected(
            &mut rt.players,
            &rt.map_layout,
            addr,
            &mut rt.next_player_id,
            now,
        );
        let player = rt.players.get_mut(&addr).unwrap();
        player.joined = true;
        player.state.x = pickup_layout()[0][0];
        player.state.z = pickup_layout()[0][1];
        player.state.max_hp = 200.0;
        player.state.hp = 100.0;
        rt.game_state = GameState::Running;
        (rt, addr, now)
    }

    #[test]
    fn forest_pickup_heals_five_percent_once_then_respawns_after_thirty_seconds() {
        let (mut rt, addr, now) = fixture();
        rt.forest_pickups.tick(&mut rt.players, &rt.game_state, now);
        assert_eq!(rt.players[&addr].state.hp, 110.0);
        let receipt = &rt.forest_pickups.snapshot(&rt.game_state)[0];
        assert!(!receipt.available);
        assert_eq!(receipt.collection_sequence, 1);
        assert_eq!(receipt.healed_amount, 10.0);
        assert_eq!(receipt.last_collector_id, Some(rt.players[&addr].state.id));
        for seconds in [0, 1, 29] {
            rt.forest_pickups.tick(
                &mut rt.players,
                &rt.game_state,
                now + Duration::from_secs(seconds),
            );
            assert_eq!(rt.players[&addr].state.hp, 110.0);
        }
        rt.players.get_mut(&addr).unwrap().state.x += 10.0;
        rt.forest_pickups.tick(
            &mut rt.players,
            &rt.game_state,
            now + Duration::from_secs(30),
        );
        assert!(rt.forest_pickups.snapshot(&rt.game_state)[0].available);
        rt.players.get_mut(&addr).unwrap().state.x -= 10.0;
        rt.forest_pickups.tick(
            &mut rt.players,
            &rt.game_state,
            now + Duration::from_secs(30),
        );
        assert_eq!(rt.players[&addr].state.hp, 120.0);
        assert_eq!(
            rt.forest_pickups.snapshot(&rt.game_state)[0].collection_sequence,
            2
        );
    }

    #[test]
    fn forest_pickup_caps_healing_and_excludes_ineligible_players() {
        for scenario in ["dead", "full", "outside", "unjoined", "respawning", "nan"] {
            let (mut rt, addr, now) = fixture();
            let player = rt.players.get_mut(&addr).unwrap();
            match scenario {
                "dead" => player.state.hp = 0.0,
                "full" => player.state.hp = player.state.max_hp,
                "outside" => player.state.x += PICKUP_RADIUS + 0.01,
                "unjoined" => player.joined = false,
                "respawning" => player.respawn_at = Some(now),
                "nan" => player.state.x = f32::NAN,
                _ => unreachable!(),
            }
            let hp = player.state.hp;
            rt.forest_pickups.tick(&mut rt.players, &rt.game_state, now);
            assert_eq!(rt.players[&addr].state.hp, hp, "{scenario}");
            assert!(
                rt.forest_pickups.snapshot(&rt.game_state)[0].available,
                "{scenario}"
            );
        }
        let (mut rt, addr, now) = fixture();
        rt.players.get_mut(&addr).unwrap().state.hp = 199.0;
        rt.players.get_mut(&addr).unwrap().state.x += PICKUP_RADIUS;
        rt.forest_pickups.tick(&mut rt.players, &rt.game_state, now);
        assert_eq!(rt.players[&addr].state.hp, 200.0);
        assert_eq!(
            rt.forest_pickups.snapshot(&rt.game_state)[0].healed_amount,
            1.0
        );
    }

    #[test]
    fn forest_pickup_contest_uses_lowest_eligible_id_for_one_claim() {
        let (mut rt, first, now) = fixture();
        let other = "127.0.0.1:59242".parse().unwrap();
        ensure_player_connected(
            &mut rt.players,
            &rt.map_layout,
            other,
            &mut rt.next_player_id,
            now,
        );
        let first_state = rt.players[&first].state.clone();
        let player = rt.players.get_mut(&other).unwrap();
        player.joined = true;
        player.state.hp = first_state.hp;
        player.state.max_hp = first_state.max_hp;
        player.state.x = first_state.x;
        player.state.z = first_state.z;
        rt.forest_pickups.tick(&mut rt.players, &rt.game_state, now);
        assert_eq!(rt.players[&first].state.hp, 110.0);
        assert_eq!(rt.players[&other].state.hp, 100.0);
        assert_eq!(
            rt.forest_pickups.snapshot(&rt.game_state)[0].last_collector_id,
            Some(first_state.id)
        );
    }

    #[test]
    fn forest_pickups_are_disabled_outside_running_and_reset_with_round() {
        let (mut rt, addr, now) = fixture();
        for phase in [
            GameState::Lobby,
            GameState::Victory {
                winner: Team::Green,
            },
        ] {
            rt.forest_pickups.tick(&mut rt.players, &phase, now);
            assert_eq!(rt.players[&addr].state.hp, 100.0);
            assert!(rt.forest_pickups.snapshot(&phase).is_empty());
        }
        rt.forest_pickups.tick(&mut rt.players, &rt.game_state, now);
        rt.restart_round(now);
        let states = rt.forest_pickups.snapshot(&GameState::Running);
        assert_eq!(states.len(), FOREST_PICKUP_COUNT);
        assert!(
            states
                .iter()
                .all(|state| state.available && state.collection_sequence == 0)
        );
    }

    #[test]
    fn forest_pickup_sandbox_pause_freezes_collection_and_respawn_and_reset_keeps_sequence() {
        let (mut rt, addr, now) = fixture();
        rt.sandbox = Some(sandbox::SandboxRuntime::new(now));
        rt.sandbox.as_mut().unwrap().config.player.position = pickup_layout()[0];
        rt.sandbox.as_mut().unwrap().config.player.max_hp = 200.0;
        rt.initialize_sandbox_players();
        rt.players.get_mut(&addr).unwrap().state.hp = 100.0;
        rt.sandbox.as_mut().unwrap().config.environment.paused = true;
        let (paused_now, dt) = rt.sandbox.as_mut().unwrap().advance(90.0);
        rt.simulate_after_mana(paused_now, dt);
        assert!(rt.forest_pickups.snapshot(&rt.game_state)[0].available);
        rt.sandbox.as_mut().unwrap().config.environment.paused = false;
        let (unpaused_now, dt) = rt.sandbox.as_mut().unwrap().advance(0.01);
        rt.simulate_after_mana(unpaused_now, dt);
        assert!(!rt.forest_pickups.snapshot(&rt.game_state)[0].available);
        rt.sandbox.as_mut().unwrap().config.environment.paused = true;
        let (paused_now, dt) = rt.sandbox.as_mut().unwrap().advance(90.0);
        rt.simulate_after_mana(paused_now, dt);
        assert!(!rt.forest_pickups.snapshot(&rt.game_state)[0].available);
        rt.reset_sandbox_duel(paused_now);
        assert!(rt.forest_pickups.snapshot(&rt.game_state)[0].available);
        assert_eq!(
            rt.forest_pickups.snapshot(&rt.game_state)[0].collection_sequence,
            1
        );
        let player = rt.players.get_mut(&addr).unwrap();
        player.state.hp = 100.0;
        player.state.max_hp = 200.0;
        player.state.x = pickup_layout()[0][0];
        player.state.z = pickup_layout()[0][1];
        rt.forest_pickups
            .tick(&mut rt.players, &rt.game_state, paused_now);
        assert_eq!(
            rt.forest_pickups.snapshot(&rt.game_state)[0].collection_sequence,
            2
        );
    }

    #[test]
    fn forest_pickup_runtime_broadcasts_authoritative_collection_state() {
        let (mut rt, original, now) = fixture();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let addr = client.local_addr().unwrap();
        let player = rt.players.remove(&original).unwrap();
        rt.players.insert(addr, player);
        rt.last_snapshot_at = now - SNAPSHOT_INTERVAL;
        rt.simulate_after_mana(now, 0.01);
        let mut buffer = [0; 65536];
        let (size, _) = client.recv_from(&mut buffer).unwrap();
        let packet: ServerPacket = serde_json::from_slice(&buffer[..size]).unwrap();
        let ServerPacket::Snapshot {
            forest_pickups,
            players,
            combat_events,
            ..
        } = packet
        else {
            panic!()
        };
        assert_eq!(forest_pickups.len(), FOREST_PICKUP_COUNT);
        assert!(!forest_pickups[0].available);
        assert_eq!(forest_pickups[0].last_collector_id, Some(players[0].id));
        assert_eq!(forest_pickups[0].healed_amount, 10.0);
        assert_eq!(players[0].hp, 110.0);
        assert!(
            combat_events.is_empty(),
            "healing must not become damage telemetry"
        );
    }

    #[test]
    fn forest_pickup_snapshot_roundtrips_and_legacy_snapshot_defaults_empty() {
        let (mut rt, _, now) = fixture();
        rt.forest_pickups.tick(&mut rt.players, &rt.game_state, now);
        let mut packet: ServerPacket = serde_json::from_value(serde_json::json!({
            "type": "snapshot", "your_id": 1, "players": [], "projectiles": [],
            "structures": [], "minions": [], "game_state": { "type": "running" }
        }))
        .unwrap();
        if let ServerPacket::Snapshot { forest_pickups, .. } = &mut packet {
            *forest_pickups = rt.forest_pickups.snapshot(&rt.game_state);
        }
        let mut value = serde_json::to_value(&packet).unwrap();
        let decoded: ServerPacket = serde_json::from_value(value.clone()).unwrap();
        let ServerPacket::Snapshot { forest_pickups, .. } = decoded else {
            panic!()
        };
        assert_eq!(forest_pickups.len(), FOREST_PICKUP_COUNT);
        assert!(!forest_pickups[0].available);
        value.as_object_mut().unwrap().remove("forest_pickups");
        let ServerPacket::Snapshot { forest_pickups, .. } = serde_json::from_value(value).unwrap()
        else {
            panic!()
        };
        assert!(forest_pickups.is_empty());
    }
}
