//! Authoritative gameplay command entry, after host admission and pause policy.
use crate::game_world::GameWorld;
use shared::wire::{ClientPacket, GameState};
use std::{net::SocketAddr, ops::ControlFlow, time::Instant};

pub fn apply(
    world: &mut GameWorld,
    addr: SocketAddr,
    packet: &ClientPacket,
    epoch: u64,
    round: u64,
    now: Instant,
) -> Option<ControlFlow<()>> {
    use ClientPacket::*;
    match packet {
        BasicAttack {
            server_epoch,
            match_id,
            ..
        }
        | CastSkill {
            server_epoch,
            match_id,
            ..
        }
        | Utility {
            server_epoch,
            match_id,
            ..
        }
        | BuyItem {
            server_epoch,
            match_id,
            ..
        } if *server_epoch != epoch || *match_id != round => return Some(ControlFlow::Break(())),
        _ => {}
    }
    match packet {
        Transform {
            x,
            y,
            z,
            yaw,
            dash_sequence,
        } => {
            world.ensure_connected(addr, now);
            let terrain = crate::skills::advanced::terrain(world, now);
            if let Some(player) = world.players.get_mut(&addr) {
                player.last_seen = now;
                if matches!(world.game_state, GameState::Running)
                    && player.hero.hp > 0.0
                    && *dash_sequence == player.hero.utility.dash_sequence
                {
                    let from = [player.hero.x, player.hero.z];
                    crate::session::handle_transform_request_with_structures(
                        player,
                        &world.map_layout,
                        &world.structures,
                        *x,
                        *y,
                        *z,
                        *yaw,
                        now,
                    );
                    let to = shared::navigation::clip_discs(
                        from,
                        [player.hero.x, player.hero.z],
                        &terrain,
                    );
                    player.hero.x = to[0];
                    player.hero.z = to[1];
                }
            }
        }
        Cast { target, slot } => {
            world.ensure_connected(addr, now);
            if let Some(p) = world.players.get_mut(&addr) {
                p.last_seen = now;
            }
            crate::sim::cast::handle_cast_request(world, addr, *target, *slot, now);
        }
        CastSkill {
            slot,
            aim,
            request_id,
            ..
        } => {
            crate::skills::cast(world, addr, *slot, *aim, *request_id, now);
        }
        BasicAttack {
            target, request_id, ..
        } => {
            crate::basic_attack::handle_basic_attack_request(
                world,
                addr,
                *target,
                *request_id,
                now,
            );
        }
        Interact {
            object_id,
            server_epoch,
            match_id,
            request_id,
        } => {
            if *server_epoch == epoch && *match_id == round {
                crate::skills::advanced::interact(world, addr, *object_id, *request_id, now);
            }
        }
        Utility {
            action,
            direction,
            request_id,
            ..
        } => {
            if let Some(p) = world.players.get_mut(&addr) {
                crate::utility::handle_utility_request(
                    p,
                    &world.map_layout,
                    &world.structures,
                    &world.game_state,
                    *action,
                    *direction,
                    *request_id,
                    now,
                );
            }
        }
        UpgradeSkill { slot } => {
            world.ensure_connected(addr, now);
            if let Some(p) = world.players.get_mut(&addr) {
                p.last_seen = now;
                if p.joined {
                    crate::sim::cast::apply_skill_upgrade(p, *slot);
                }
            }
        }
        BuyItem {
            item_id,
            request_id,
            ..
        } => {
            world.ensure_connected(addr, now);
            if let Some(p) = world.players.get_mut(&addr) {
                p.last_seen = now;
                crate::shop::handle_purchase(
                    p,
                    &world.map_layout,
                    &world.game_state,
                    item_id,
                    *request_id,
                    round,
                );
            }
        }
        _ => return None,
    }
    Some(ControlFlow::Continue(()))
}
