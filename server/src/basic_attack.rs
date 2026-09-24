//! Individual server-authorized basic strikes. Clients own repeat/chase intent;
//! the server owns target legality, range, timing, equipment and damage.
use crate::*;
#[cfg(test)]
use shared::shop::basic_attack_cooldown;
use shared::{BASIC_ATTACK_ACTION_SLOT, basic_attack_for_class};

pub(crate) fn resolve_hostile_target(
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
                    && player.state.id == target.id
                    && player.state.hp > 0.0
                    && player.state.team != team
            })?;
            Some((
                Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z),
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

pub(crate) fn handle_basic_attack_request(
    world: &mut GameWorld,
    addr: SocketAddr,
    target: TargetId,
    request_id: u64,
    now: Instant,
) {
    let Some(attacker) = world.players.get_mut(&addr) else {
        return;
    };
    if !attacker.joined || request_id == 0 || request_id <= attacker.state.basic_attack_request_id {
        return;
    }
    // Identity was checked by the packet receiver before this high-water mark.
    // Consume even a rejected strike: it cannot be replayed later after walking
    // into range, recovering from death, or waiting out the cooldown.
    attacker.state.basic_attack_request_id = request_id;
    attacker.last_seen = now;
    if !matches!(world.game_state, GameState::Running) || attacker.state.hp <= 0.0 {
        return;
    }
    let definition = basic_attack_for_class(attacker.state.hero_class);
    let cooldown = sandbox::effective_basic_attack_cooldown(attacker);
    if !attacker.sandbox.as_ref().is_some_and(|c| c.no_cooldowns)
        && attacker
            .timers
            .last_basic_attack_at
            .is_some_and(|last| now.saturating_duration_since(last) < cooldown)
    {
        return;
    }
    let team = attacker.state.team;
    let sandbox = attacker.sandbox.is_some();
    let origin = Vec3f::new(
        attacker.state.x,
        attacker.state.y + CAST_SPAWN_HEIGHT,
        attacker.state.z,
    );
    let damage = sandbox::effective_basic_attack_damage(attacker)
        * world.team_buffs.damage_multiplier(team, now);
    if !sandbox && !vision::target_visible(team, target, world, now) {
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
    if !distance.is_finite() || distance > definition.range + radius {
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
    attacker.timers.last_basic_attack_at = Some(now);
    attacker.state.action_sequence = attacker.state.action_sequence.wrapping_add(1).max(1);
    attacker.state.action_kind = PlayerActionKind::Attack;
    attacker.state.action_slot = BASIC_ATTACK_ACTION_SLOT;
    let id = world.next_projectile_id;
    world.next_projectile_id += 1;
    world.projectiles.insert(
        id,
        Projectile {
            state: ProjectileState {
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::for_class(attacker.state.hero_class),
                action_slot: Some(BASIC_ATTACK_ACTION_SLOT),
                direction: [direction.x, direction.y, direction.z],
                id,
                owner_id: attacker.state.id,
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
mod tests;
