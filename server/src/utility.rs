//! Match-scoped utility requests, using the normal movement collision authority.
use super::*;
use shared::utility::*;

pub(crate) fn utility_movement_multiplier(player: &ConnectedPlayer, now: Instant) -> f32 {
    if player.state.hp > 0.0 && player.haste_expires_at.is_some_and(|until| now < until) {
        HASTE_SPEED_MULTIPLIER
    } else {
        1.0
    }
}

fn remaining(deadline: Option<Instant>, now: Instant) -> f32 {
    deadline.map_or(0.0, |until| {
        until.saturating_duration_since(now).as_secs_f32()
    })
}

fn refresh_utility(player: &mut ConnectedPlayer, now: Instant) {
    if player.sandbox.as_ref().is_some_and(|c| c.no_cooldowns) {
        player.dash_ready_at = None;
        player.haste_ready_at = None;
    }
    if player.state.hp <= 0.0 {
        player.haste_expires_at = None;
    }
    player.state.utility.dash_remaining_secs = remaining(player.dash_ready_at, now);
    player.state.utility.haste_remaining_secs = remaining(player.haste_ready_at, now);
    player.state.utility.haste_active_secs = remaining(player.haste_expires_at, now);
}

pub(crate) fn refresh_utilities(players: &mut HashMap<SocketAddr, ConnectedPlayer>, now: Instant) {
    for player in players.values_mut() {
        refresh_utility(player, now);
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
    refresh_utility(player, now);
    if !matches!(phase, GameState::Running) || player.state.hp <= 0.0 {
        return;
    }
    match action {
        UtilityAction::Dash => {
            if player.state.utility.dash_remaining_secs > 0.0
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
            player.last_movement_at = now;
            player.dash_ready_at = Some(now + Duration::from_secs_f32(DASH_COOLDOWN_SECS));
            player.state.utility.dash_sequence =
                player.state.utility.dash_sequence.saturating_add(1);
        }
        UtilityAction::Haste => {
            if player.state.utility.haste_remaining_secs > 0.0 {
                return;
            }
            player.haste_ready_at = Some(now + Duration::from_secs_f32(HASTE_COOLDOWN_SECS));
            player.haste_expires_at = Some(now + Duration::from_secs_f32(HASTE_DURATION_SECS));
        }
    }
    refresh_utility(player, now);
}

#[cfg(test)]
mod tests;
