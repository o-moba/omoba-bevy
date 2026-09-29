//! Server-owned finite healing resources. Movement is the only collection input.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::forest_pickups::{
    FOREST_PICKUP_COUNT, ForestPickupState, HEAL_FRACTION, PICKUP_RADIUS, RESPAWN_SECS,
    pickup_layout,
};
use shared::wire::GameState;

use crate::entities::ConnectedPlayer;

struct Pickup {
    state: ForestPickupState,
    respawn_at: Option<Instant>,
}

pub struct ForestPickups {
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
    pub fn reset_availability(&mut self) {
        for pickup in &mut self.slots {
            pickup.state.available = true;
            pickup.respawn_at = None;
        }
    }

    pub fn snapshot(&self, game_state: &GameState) -> Vec<ForestPickupState> {
        if !matches!(game_state, GameState::Running) {
            return Vec::new();
        }
        self.slots.iter().map(|slot| slot.state.clone()).collect()
    }

    /// `now` is the simulation clock (including sandbox pause/time scaling).
    pub fn tick(
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
