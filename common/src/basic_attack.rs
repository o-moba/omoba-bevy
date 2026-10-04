//! Individual server-authorized basic strikes. Clients own repeat/chase intent;
//! the server owns target legality, range, timing, equipment and damage.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;

use shared::combat::{CombatEntityKind, ProjectileStyle};
use shared::map::Team;
use shared::wire::{GameState, ProjectileState, TargetId, TargetKind};
use shared::{BASIC_ATTACK_ACTION_SLOT, PlayerActionKind};

use crate::balance::{
    AIM_HEIGHT, CAST_SPAWN_HEIGHT, MINION_RADIUS, NEUTRAL_RADIUS, PLAYER_HIT_RADIUS,
    PROJECTILE_LIFETIME, PROJECTILE_RADIUS, PROJECTILE_SPEED,
};
use crate::entities::{ConnectedPlayer, Minion, Neutral, Projectile, Structure, Vec3f};
use crate::game_world::GameWorld;
use crate::sim::towers::structure_is_protected;
use crate::world::structure_radius;
use crate::{hero_stats, vision};

pub fn resolve_hostile_target(
    team: Team,
    target: TargetId,
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    minions: &HashMap<u64, Minion>,
    structures: &HashMap<u64, Structure>,
    neutrals: &HashMap<u64, Neutral>,
) -> Option<(Vec3f, f32)> {
    match target.kind {
        TargetKind::Player => {
            let player = players.values().find(|player| {
                player.joined
                    && player.hero.identity.id == target.id
                    && player.hero.hp > 0.0
                    && player.hero.identity.team != team
            })?;
            Some((
                Vec3f::new(player.hero.x, player.hero.y + AIM_HEIGHT, player.hero.z),
                PLAYER_HIT_RADIUS,
            ))
        }
        TargetKind::Minion => {
            let minion = minions.get(&target.id)?;
            (minion.state.hp > 0.0 && minion.state.team != team).then_some((
                Vec3f::new(
                    minion.state.x,
                    minion.state.y + MINION_RADIUS * 0.8,
                    minion.state.z,
                ),
                MINION_RADIUS,
            ))
        }
        TargetKind::Structure => {
            let structure = structures.get(&target.id)?;
            (structure.state.hp > 0.0
                && structure.state.team != team
                && !structure_is_protected(structures, target.id))
            .then_some((
                Vec3f::new(structure.state.x, structure.state.y, structure.state.z),
                structure_radius(structure.state.kind),
            ))
        }
        TargetKind::Neutral => {
            let neutral = neutrals.get(&target.id)?;
            (neutral.dead_until.is_none() && neutral.state.hp > 0.0).then_some((
                Vec3f::new(
                    neutral.state.x,
                    neutral.state.y + NEUTRAL_RADIUS * 0.85,
                    neutral.state.z,
                ),
                NEUTRAL_RADIUS,
            ))
        }
    }
}

pub fn handle_basic_attack_request(
    world: &mut GameWorld,
    addr: SocketAddr,
    target: TargetId,
    request_id: u64,
    now: Instant,
) {
    let Some(attacker) = world.players.get_mut(&addr) else {
        return;
    };
    if !attacker.joined || request_id == 0 || request_id <= attacker.economy.basic_attack_request_id
    {
        return;
    }
    // Identity was checked by the packet receiver before this high-water mark.
    // Consume even a rejected strike: it cannot be replayed later after walking
    // into range, recovering from death, or waiting out the cooldown.
    attacker.economy.basic_attack_request_id = request_id;
    attacker.last_seen = now;
    crate::recall::cancel(attacker);
    if !matches!(world.game_state, GameState::Running) || attacker.hero.hp <= 0.0 {
        return;
    }
    if attacker
        .hero
        .skills
        .control
        .stun_until
        .is_some_and(|until| until > now)
        || attacker
            .hero
            .skills
            .advanced
            .charm
            .is_some_and(|(_, until)| until > now)
        || attacker.hero.skills.advanced.immune(now)
    {
        return;
    }
    let (range, mode_damage, _, mana_cost, splash) = crate::skills::attack_modifiers(attacker);
    if !attacker.modifiers.infinite_resource && attacker.hero.mana < mana_cost {
        return;
    }
    let cooldown = hero_stats::basic_attack_cooldown_at(attacker, now);
    if !attacker.modifiers.no_cooldowns
        && attacker
            .timers
            .last_basic_attack_at
            .is_some_and(|last| now.saturating_duration_since(last) < cooldown)
    {
        return;
    }
    let team = attacker.hero.identity.team;
    let bypass_vision = attacker.modifiers.bypass_vision;
    let origin = Vec3f::new(
        attacker.hero.x,
        attacker.hero.y + CAST_SPAWN_HEIGHT,
        attacker.hero.z,
    );
    let mut damage = hero_stats::basic_attack_damage(attacker)
        * mode_damage
        * world.team_buffs.damage_multiplier(team, now);
    if !bypass_vision && !vision::target_visible(team, target, world, now) {
        return;
    }
    let Some((position, radius)) = resolve_hostile_target(
        team,
        target,
        &world.players,
        &world.minions,
        &world.structures,
        &world.neutrals,
    ) else {
        return;
    };
    let distance = ((position.x - origin.x).powi(2) + (position.z - origin.z).powi(2)).sqrt();
    if !distance.is_finite() || distance > range + radius {
        return;
    }
    let direction = Vec3f::new(
        position.x - origin.x,
        position.y - origin.y,
        position.z - origin.z,
    )
    .normalize_or_zero();
    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
        return;
    }
    let attacker = world.players.get_mut(&addr).unwrap();
    // Critical hits are reproducible server decisions, independent of request IDs.
    if target.kind != TargetKind::Structure {
        attacker.economy.basic_crit_meter +=
            attacker.economy.item_bonuses.crit_chance.clamp(0.0, 0.75);
        if attacker.economy.basic_crit_meter + f32::EPSILON >= 1.0 {
            attacker.economy.basic_crit_meter = (attacker.economy.basic_crit_meter - 1.0).max(0.0);
            damage *= shared::shop::CRITICAL_DAMAGE_MULTIPLIER;
        }
    }
    if !attacker.modifiers.infinite_resource {
        attacker.hero.mana -= mana_cost;
    }
    crate::skills::accepted_basic(attacker, now);
    attacker.timers.last_basic_attack_at = Some(now);
    attacker.hero.last_action.sequence = attacker.hero.last_action.sequence.wrapping_add(1).max(1);
    attacker.hero.last_action.kind = PlayerActionKind::Attack;
    attacker.hero.last_action.slot = BASIC_ATTACK_ACTION_SLOT;
    crate::sim::cast::face_player_action(attacker, direction.x, direction.z);
    let id = world.next_projectile_id;
    world.next_projectile_id += 1;
    if splash > 0.0 {
        world.skill_runtime.attack_splash.insert(id, splash);
    }
    world.projectiles.insert(
        id,
        Projectile {
            state: ProjectileState {
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::for_class(attacker.hero.identity.hero_class),
                action_slot: Some(BASIC_ATTACK_ACTION_SLOT),
                direction: [direction.x, direction.y, direction.z],
                id,
                owner_id: attacker.hero.identity.id,
                owner_team: team,
                x: origin.x,
                y: origin.y,
                z: origin.z,
            },
            target,
            velocity: Vec3f::new(
                direction.x * PROJECTILE_SPEED,
                direction.y * PROJECTILE_SPEED,
                direction.z * PROJECTILE_SPEED,
            ),
            homing: true,
            guaranteed_hit: true,
            damage,
            radius: PROJECTILE_RADIUS,
            expires_at: now + PROJECTILE_LIFETIME,
        },
    );
}

#[cfg(test)]
mod critical_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn accepted_basic_strikes_accumulate_crit_but_replays_and_rejections_do_not() {
        let now = Instant::now();
        let mut world = GameWorld::empty();
        let a: SocketAddr = "127.0.0.1:61201".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:61202".parse().unwrap();
        for (address, team, x) in [(a, Team::Green, -8.0), (b, Team::Blue, -5.0)] {
            world.ensure_connected(address, now);
            let p = world.players.get_mut(&address).unwrap();
            p.joined = true;
            p.hero.identity.team = team;
            p.hero.x = x;
            p.hero.z = -8.0;
        }
        world
            .players
            .get_mut(&a)
            .unwrap()
            .economy
            .item_bonuses
            .crit_chance = 0.25;
        let target = TargetId {
            kind: TargetKind::Player,
            id: world.players[&b].hero.identity.id,
        };
        let mut damages = Vec::new();
        for request in 1..=4 {
            handle_basic_attack_request(
                &mut world,
                a,
                target,
                request,
                now + Duration::from_secs(request * 2),
            );
            assert_eq!(world.projectiles.len(), 1);
            damages.push(world.projectiles.values().next().unwrap().damage);
            let meter = world.players[&a].economy.basic_crit_meter;
            handle_basic_attack_request(
                &mut world,
                a,
                target,
                request,
                now + Duration::from_secs(request * 2),
            );
            assert_eq!(world.players[&a].economy.basic_crit_meter, meter);
            world.projectiles.clear();
        }
        assert_eq!(damages[0], damages[1]);
        assert_eq!(damages[0], damages[2]);
        assert!(
            (damages[3] - damages[0] * shared::shop::CRITICAL_DAMAGE_MULTIPLIER).abs() < 0.0001
        );
        let meter = world.players[&a].economy.basic_crit_meter;
        handle_basic_attack_request(
            &mut world,
            a,
            TargetId {
                id: u64::MAX,
                ..target
            },
            999,
            now + Duration::from_secs(20),
        );
        assert_eq!(world.players[&a].economy.basic_crit_meter, meter);
    }
}
