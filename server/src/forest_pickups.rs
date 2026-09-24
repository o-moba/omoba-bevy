//! Server-owned finite healing resources. Movement is the only collection input.
use std::time::Instant;
use std::net::SocketAddr;
use shared::wire::GameState;
use std::time::Duration;
use crate::entities::ConnectedPlayer;
use std::collections::HashMap;
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
                    let hero = &player.hero;
                    player.joined
                        && player.timers.respawn_at.is_none()
                        && hero.hp.is_finite()
                        && hero.max_hp.is_finite()
                        && hero.hp > 0.0
                        && hero.hp < hero.max_hp
                        && (hero.x - pickup.state.position[0])
                            .hypot(hero.z - pickup.state.position[1])
                            <= PICKUP_RADIUS
                })
                .min_by_key(|(_, player)| player.hero.identity.id)
                .map(|(addr, _)| *addr);
            let Some(player) = collector.and_then(|addr| players.get_mut(&addr)) else {
                continue;
            };
            let hero = &mut player.hero;
            let healed = (hero.max_hp - hero.hp).min(hero.max_hp * HEAL_FRACTION);
            hero.hp = (hero.hp + healed).min(hero.max_hp);
            pickup.state.available = false;
            pickup.state.collection_sequence = pickup.state.collection_sequence.saturating_add(1);
            pickup.state.last_collector_id = Some(hero.identity.id);
            pickup.state.healed_amount = healed;
            pickup.respawn_at = Some(now + Duration::from_secs_f32(RESPAWN_SECS));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
use crate::snapshot::SNAPSHOT_INTERVAL;
use std::net::SocketAddr;
use std::net::UdpSocket;
use crate::runtime::ServerRuntime;
use crate::match_rules::MatchConfig;
use shared::map::Team;
use shared::wire::GameState;
use crate::sandbox;
use std::time::Instant;
use shared::wire::ServerPacket;
use super::*;

    fn fixture() -> (ServerRuntime, SocketAddr, Instant) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
        rt.targeting_qa = true;
        let addr = "127.0.0.1:59241".parse().unwrap();
        let now = Instant::now();
        rt.world.ensure_connected(addr, now);
        let player = rt.world.players.get_mut(&addr).unwrap();
        player.joined = true;
        player.hero.x = pickup_layout()[0][0];
        player.hero.z = pickup_layout()[0][1];
        player.hero.max_hp = 200.0;
        player.hero.hp = 100.0;
        rt.world.game_state = GameState::Running;
        (rt, addr, now)
    }

    #[test]
    fn forest_pickup_heals_five_percent_once_then_respawns_after_thirty_seconds() {
        let (mut rt, addr, now) = fixture();
        rt.world
            .forest_pickups
            .tick(&mut rt.world.players, &rt.world.game_state, now);
        assert_eq!(rt.world.players[&addr].hero.hp, 110.0);
        let receipt = &rt.world.forest_pickups.snapshot(&rt.world.game_state)[0];
        assert!(!receipt.available);
        assert_eq!(receipt.collection_sequence, 1);
        assert_eq!(receipt.healed_amount, 10.0);
        assert_eq!(
            receipt.last_collector_id,
            Some(rt.world.players[&addr].hero.identity.id)
        );
        for seconds in [0, 1, 29] {
            rt.world.forest_pickups.tick(
                &mut rt.world.players,
                &rt.world.game_state,
                now + Duration::from_secs(seconds),
            );
            assert_eq!(rt.world.players[&addr].hero.hp, 110.0);
        }
        rt.world.players.get_mut(&addr).unwrap().hero.x += 10.0;
        rt.world.forest_pickups.tick(
            &mut rt.world.players,
            &rt.world.game_state,
            now + Duration::from_secs(30),
        );
        assert!(rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].available);
        rt.world.players.get_mut(&addr).unwrap().hero.x -= 10.0;
        rt.world.forest_pickups.tick(
            &mut rt.world.players,
            &rt.world.game_state,
            now + Duration::from_secs(30),
        );
        assert_eq!(rt.world.players[&addr].hero.hp, 120.0);
        assert_eq!(
            rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].collection_sequence,
            2
        );
    }

    #[test]
    fn forest_pickup_caps_healing_and_excludes_ineligible_players() {
        for scenario in ["dead", "full", "outside", "unjoined", "respawning", "nan"] {
            let (mut rt, addr, now) = fixture();
            let player = rt.world.players.get_mut(&addr).unwrap();
            match scenario {
                "dead" => player.hero.hp = 0.0,
                "full" => player.hero.hp = player.hero.max_hp,
                "outside" => player.hero.x += PICKUP_RADIUS + 0.01,
                "unjoined" => player.joined = false,
                "respawning" => player.timers.respawn_at = Some(now),
                "nan" => player.hero.x = f32::NAN,
                _ => unreachable!(),
            }
            let hp = player.hero.hp;
            rt.world
                .forest_pickups
                .tick(&mut rt.world.players, &rt.world.game_state, now);
            assert_eq!(rt.world.players[&addr].hero.hp, hp, "{scenario}");
            assert!(
                rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].available,
                "{scenario}"
            );
        }
        let (mut rt, addr, now) = fixture();
        rt.world.players.get_mut(&addr).unwrap().hero.hp = 199.0;
        rt.world.players.get_mut(&addr).unwrap().hero.x += PICKUP_RADIUS;
        rt.world
            .forest_pickups
            .tick(&mut rt.world.players, &rt.world.game_state, now);
        assert_eq!(rt.world.players[&addr].hero.hp, 200.0);
        assert_eq!(
            rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].healed_amount,
            1.0
        );
    }

    #[test]
    fn forest_pickup_contest_uses_lowest_eligible_id_for_one_claim() {
        let (mut rt, first, now) = fixture();
        let other = "127.0.0.1:59242".parse().unwrap();
        rt.world.ensure_connected(other, now);
        let first_state = rt.world.players[&first].hero.clone();
        let player = rt.world.players.get_mut(&other).unwrap();
        player.joined = true;
        player.hero.hp = first_state.hp;
        player.hero.max_hp = first_state.max_hp;
        player.hero.x = first_state.x;
        player.hero.z = first_state.z;
        rt.world
            .forest_pickups
            .tick(&mut rt.world.players, &rt.world.game_state, now);
        assert_eq!(rt.world.players[&first].hero.hp, 110.0);
        assert_eq!(rt.world.players[&other].hero.hp, 100.0);
        assert_eq!(
            rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].last_collector_id,
            Some(first_state.identity.id)
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
            rt.world
                .forest_pickups
                .tick(&mut rt.world.players, &phase, now);
            assert_eq!(rt.world.players[&addr].hero.hp, 100.0);
            assert!(rt.world.forest_pickups.snapshot(&phase).is_empty());
        }
        rt.world
            .forest_pickups
            .tick(&mut rt.world.players, &rt.world.game_state, now);
        rt.restart_round(now);
        let states = rt.world.forest_pickups.snapshot(&GameState::Running);
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
        rt.world.players.get_mut(&addr).unwrap().hero.hp = 100.0;
        rt.sandbox.as_mut().unwrap().config.environment.paused = true;
        let (paused_now, dt) = rt.sandbox.as_mut().unwrap().advance(90.0);
        rt.tick(paused_now, dt);
        assert!(rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].available);
        rt.sandbox.as_mut().unwrap().config.environment.paused = false;
        let (unpaused_now, dt) = rt.sandbox.as_mut().unwrap().advance(0.01);
        rt.tick(unpaused_now, dt);
        assert!(!rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].available);
        rt.sandbox.as_mut().unwrap().config.environment.paused = true;
        let (paused_now, dt) = rt.sandbox.as_mut().unwrap().advance(90.0);
        rt.tick(paused_now, dt);
        assert!(!rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].available);
        rt.reset_sandbox_duel(paused_now);
        assert!(rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].available);
        assert_eq!(
            rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].collection_sequence,
            1
        );
        let player = rt.world.players.get_mut(&addr).unwrap();
        player.hero.hp = 100.0;
        player.hero.max_hp = 200.0;
        player.hero.x = pickup_layout()[0][0];
        player.hero.z = pickup_layout()[0][1];
        rt.world
            .forest_pickups
            .tick(&mut rt.world.players, &rt.world.game_state, paused_now);
        assert_eq!(
            rt.world.forest_pickups.snapshot(&rt.world.game_state)[0].collection_sequence,
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
        let player = rt.world.players.remove(&original).unwrap();
        rt.world.players.insert(addr, player);
        rt.last_snapshot_at = now - SNAPSHOT_INTERVAL;
        rt.tick(now, 0.01);
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
        assert_eq!(
            forest_pickups.len(),
            1,
            "only the nearby pickup is in team sight"
        );
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
        rt.world
            .forest_pickups
            .tick(&mut rt.world.players, &rt.world.game_state, now);
        let mut packet: ServerPacket = serde_json::from_value(serde_json::json!({
            "type": "snapshot", "your_id": 1, "players": [], "projectiles": [],
            "structures": [], "minions": [], "game_state": { "type": "running" }
        }))
        .unwrap();
        if let ServerPacket::Snapshot { forest_pickups, .. } = &mut packet {
            *forest_pickups = rt.world.forest_pickups.snapshot(&rt.world.game_state);
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
