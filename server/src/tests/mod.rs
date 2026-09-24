use crate::*;

mod bosses;
mod cast;
mod formation;
mod minions;
mod movement;
mod neutrals;
mod progression;
mod sessions;
mod snapshot;

pub(super) const EPSILON: f32 = 0.0001;

/// Sets up two joined enemy players (caster at `caster_addr` with the given
/// class) standing `gap` apart on the x axis, returning the target's id.
pub(super) fn setup_caster_and_target(
    world: &mut GameWorld,
    caster_addr: SocketAddr,
    target_addr: SocketAddr,
    caster_class: HeroClass,
    gap: f32,
    now: Instant,
) -> u64 {
    world.ensure_connected(caster_addr, now);
    world.ensure_connected(target_addr, now);
    handle_join_request(
        world.players.get_mut(&caster_addr).unwrap(),
        Team::Green,
        CharacterChoice::Ipfs,
        caster_class,
        None,
        &world.map_layout,
        now,
    );
    handle_join_request(
        world.players.get_mut(&target_addr).unwrap(),
        Team::Blue,
        CharacterChoice::Wang,
        HeroClass::default(),
        None,
        &world.map_layout,
        now,
    );
    let caster_pos = {
        let caster = world.players.get(&caster_addr).unwrap();
        (caster.state.x, caster.state.z)
    };
    {
        let target = world.players.get_mut(&target_addr).unwrap();
        target.state.x = caster_pos.0 + gap;
        target.state.z = caster_pos.1;
    }
    world.players.get(&target_addr).unwrap().state.id
}

/// Casts `slot` from `caster_addr` at `target` in a running world.
pub(super) fn cast_slot(
    world: &mut GameWorld,
    caster_addr: SocketAddr,
    target: TargetId,
    slot: u8,
    now: Instant,
) {
    handle_cast_request(world, caster_addr, target, slot, now);
}
