//! Opt-in developer capture setup. Only initial join placement and ambient AI
//! are changed. Player movement, health, mana, strikes and damage stay real.
use crate::*;

fn enabled_for(debug: bool, mode: MatchMode, mode_env: Option<&str>, flag: Option<&str>) -> bool {
    debug && matches!(mode, MatchMode::Dev) && mode_env == Some("dev") && flag == Some("1")
}

pub(crate) fn enabled(mode: MatchMode) -> bool {
    enabled_for(
        cfg!(debug_assertions),
        mode,
        std::env::var("OMOBA_MATCH_MODE").ok().as_deref(),
        std::env::var("OMOBA_TARGETING_QA").ok().as_deref(),
    )
}

pub(crate) const GREEN: [f32; 2] = [-8.0, -8.0];
pub(crate) const BLUE: [[f32; 2]; 2] = [[-4.0, -8.0], [-8.0, -4.0]];

pub(crate) fn place_initial_join(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    addr: SocketAddr,
) {
    let Some(player) = players.get(&addr) else {
        return;
    };
    if !player.joined {
        return;
    }
    let team = player.state.team;
    let index = players
        .values()
        .filter(|p| p.joined && p.state.team == team && p.state.id < player.state.id)
        .count();
    let position = match team {
        Team::Green if index == 0 => Some(GREEN),
        Team::Blue => BLUE.get(index).copied(),
        _ => None,
    };
    let Some([x, z]) = position else { return };
    // Refuse setup if a later map revision makes these points obstructed.
    if !shared::navigation::world_navigation().point_clear([x, z]) {
        return;
    }
    let player = players.get_mut(&addr).unwrap();
    player.state.x = x;
    player.state.y = PLAYER_GROUND_Y;
    player.state.z = z;
    println!(
        "TARGETING_QA initial_join player={} team={team:?} x={x} z={z}; ambient AI disabled; player damage authoritative",
        player.state.id
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_all_explicit_development_gates_and_walkable_anchors() {
        assert!(enabled_for(true, MatchMode::Dev, Some("dev"), Some("1")));
        assert!(!enabled_for(false, MatchMode::Dev, Some("dev"), Some("1")));
        assert!(!enabled_for(
            true,
            MatchMode::Release,
            Some("dev"),
            Some("1")
        ));
        assert!(!enabled_for(true, MatchMode::Dev, None, Some("1")));
        assert!(!enabled_for(true, MatchMode::Dev, Some("dev"), None));
        assert!(!enabled_for(
            true,
            MatchMode::Dev,
            Some("dev"),
            Some("true")
        ));
        for position in [GREEN, BLUE[0], BLUE[1]] {
            assert!(shared::navigation::world_navigation().point_clear(position));
        }
    }
}
