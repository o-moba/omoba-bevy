//! Match-scoped utilities. Dash blinks through obstacles to a clear landing.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use shared::utility::*;
use shared::wire::GameState;

use crate::balance::PLAYER_GROUND_Y;
use crate::entities::{ConnectedPlayer, MapLayoutState, Structure, Vec3f};
use crate::hero_timers;

pub fn utility_movement_multiplier(player: &ConnectedPlayer, now: Instant) -> f32 {
    if player.hero.hp > 0.0
        && player
            .timers
            .haste_expires_at
            .is_some_and(|until| now < until)
    {
        HASTE_SPEED_MULTIPLIER
    } else {
        1.0
    }
}

pub fn handle_utility_request(
    player: &mut ConnectedPlayer,
    map: &MapLayoutState,
    structures: &HashMap<u64, Structure>,
    phase: &GameState,
    action: UtilityAction,
    direction: [f32; 2],
    request_id: u64,
    now: Instant,
) {
    handle_utility_request_with_terrain(
        player,
        map,
        structures,
        phase,
        action,
        direction,
        request_id,
        now,
        &[],
    );
}

pub fn handle_utility_request_with_terrain(
    player: &mut ConnectedPlayer,
    map: &MapLayoutState,
    structures: &HashMap<u64, Structure>,
    phase: &GameState,
    action: UtilityAction,
    direction: [f32; 2],
    request_id: u64,
    now: Instant,
    terrain: &[shared::navigation::Disc],
) {
    if !player.joined || request_id == 0 || request_id <= player.hero.utility.last_request_id {
        return;
    }
    // The packet receiver validates epoch/match before consuming the request.
    // Failed requests are consumed too; a cooldown/death replay cannot activate later.
    player.hero.utility.last_request_id = request_id;
    player.last_seen = now;
    if action == UtilityAction::CancelRecall {
        crate::recall::cancel(player);
        return;
    }
    if !matches!(phase, GameState::Running) || player.hero.hp <= 0.0 {
        return;
    }
    if action != UtilityAction::Recall {
        crate::recall::cancel(player);
    }
    if player.hero.skills.control.movement(now) == 0.0 {
        return;
    }
    match action {
        UtilityAction::Recall => crate::recall::start(player, now),
        UtilityAction::CancelRecall => unreachable!("handled before action gates"),
        UtilityAction::Dash => {
            if hero_timers::dash_remaining(player, now) > 0.0
                || !direction.iter().all(|x| x.is_finite())
            {
                return;
            }
            let length = direction[0].hypot(direction[1]);
            if !length.is_finite() || length <= 0.0001 {
                return;
            }
            let from = [player.hero.x, player.hero.z];
            let to = map.clamp_player_position(Vec3f::new(
                from[0] + direction[0] / length * DASH_DISTANCE,
                PLAYER_GROUND_Y,
                from[1] + direction[1] / length * DASH_DISTANCE,
            ));
            let mut blocked = terrain.to_vec();
            blocked.extend(structures.values().filter(|s| s.state.hp > 0.0).map(|s| {
                shared::navigation::Disc {
                    center: [s.state.x, s.state.z],
                    radius: crate::world::structure_collision_radius(s.state.kind),
                }
            }));
            let to =
                shared::navigation::world_navigation().blink_landing(from, [to.x, to.z], &blocked);
            player.hero.x = to[0];
            player.hero.y = PLAYER_GROUND_Y;
            player.hero.z = to[1];
            player.timers.last_movement_at = now;
            player.timers.dash_ready_at = Some(now + Duration::from_secs_f32(DASH_COOLDOWN_SECS));
            player.hero.utility.dash_sequence = player.hero.utility.dash_sequence.saturating_add(1);
        }
        UtilityAction::Haste => {
            if hero_timers::haste_remaining(player, now) > 0.0 {
                return;
            }
            player.timers.haste_ready_at = Some(now + Duration::from_secs_f32(HASTE_COOLDOWN_SECS));
            player.timers.haste_expires_at =
                Some(now + Duration::from_secs_f32(HASTE_DURATION_SECS));
        }
    }
}
