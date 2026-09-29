use crate::game_world::GameWorld;
use shared::wire::PlayerState;
use std::time::Instant;
/// Replicated player list for the recipient with hero id `recipient`: their
/// own entry is the `owner_view`, everyone else (teammates included) the
/// redacted `public_view`; `None` redacts every entry. Only joined players
/// are visible to clients. Pre-join endpoints are still addressable (public
/// lobby views, standalone status replies) but must not appear in the world
/// as ghost players.
pub fn build_players_snapshot(
    world: &GameWorld,
    recipient: Option<u64>,
    now: Instant,
) -> Vec<PlayerState> {
    let mut snapshot = world
        .players
        .values()
        .filter(|player| player.joined)
        .map(|player| {
            if recipient == Some(player.hero.identity.id) {
                player.owner_view(now, &world.map_layout, &world.game_state)
            } else {
                player.public_view(now, &world.map_layout, &world.game_state)
            }
        })
        .collect::<Vec<_>>();
    snapshot.sort_unstable_by_key(|player| player.id);
    snapshot
}
