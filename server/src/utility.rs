//! Match-scoped utility requests, using the normal movement collision authority.
use super::*;
use shared::utility::*;

pub(crate) fn utility_movement_multiplier(player: &ConnectedPlayer, now: Instant) -> f32 {
    if player.state.hp > 0.0
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

pub(crate) fn handle_utility_request(
    player: &mut ConnectedPlayer,
    map: &MapLayoutState,
    structures: &HashMap<u64, Structure>,
    phase: &GameState,
    action: UtilityAction,
    direction: [f32; 2],
    request_id: u64,
    now: Instant,
) {
    if !player.joined || request_id == 0 || request_id <= player.state.utility.last_request_id {
        return;
    }
    // The packet receiver validates epoch/match before consuming the request.
    // Failed requests are consumed too; a cooldown/death replay cannot activate later.
    player.state.utility.last_request_id = request_id;
    player.last_seen = now;
    if !matches!(phase, GameState::Running) || player.state.hp <= 0.0 {
        return;
    }
    match action {
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
            let from = [player.state.x, player.state.z];
            let to = map.clamp_player_position(Vec3f::new(
                from[0] + direction[0] / length * DASH_DISTANCE,
                PLAYER_GROUND_Y,
                from[1] + direction[1] / length * DASH_DISTANCE,
            ));
            let to = shared::navigation::world_navigation().clip_movement(from, [to.x, to.z]);
            let to = clip_live_structures(from, to, structures);
            player.state.x = to[0];
            player.state.y = PLAYER_GROUND_Y;
            player.state.z = to[1];
            player.timers.last_movement_at = now;
            player.timers.dash_ready_at = Some(now + Duration::from_secs_f32(DASH_COOLDOWN_SECS));
            player.state.utility.dash_sequence =
                player.state.utility.dash_sequence.saturating_add(1);
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

#[cfg(test)]
mod tests;
