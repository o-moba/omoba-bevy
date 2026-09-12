//! Individual server-authorized basic strikes. Clients own repeat/chase intent;
//! the server owns target legality, range, timing, equipment and damage.
use crate::*;
use shared::shop::{basic_attack_cooldown, basic_attack_damage};
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_basic_attack_request(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    projectiles: &mut HashMap<u64, Projectile>,
    minions: &HashMap<u64, Minion>,
    structures: &HashMap<u64, Structure>,
    neutrals: &HashMap<u64, Neutral>,
    team_buffs: &TeamBuffs,
    addr: SocketAddr,
    target: TargetId,
    request_id: u64,
    next_projectile_id: &mut u64,
    game_state: &GameState,
    now: Instant,
) {
    let Some(attacker) = players.get_mut(&addr) else {
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
    if !matches!(game_state, GameState::Running) || attacker.state.hp <= 0.0 {
        return;
    }
    let definition = basic_attack_for_class(attacker.state.hero_class);
    let cooldown = basic_attack_cooldown(definition, attacker.state.item_bonuses);
    if attacker
        .last_basic_attack_at
        .is_some_and(|last| now.saturating_duration_since(last) < cooldown)
    {
        return;
    }
    let team = attacker.state.team;
    let origin = Vec3f::new(
        attacker.state.x,
        attacker.state.y + CAST_SPAWN_HEIGHT,
        attacker.state.z,
    );
    let damage = basic_attack_damage(definition, attacker.state.item_bonuses)
        * team_buffs.damage_multiplier(team, now);
    let Some((position, radius)) =
        resolve_hostile_target(team, target, players, minions, structures, neutrals)
    else {
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
    let attacker = players.get_mut(&addr).unwrap();
    attacker.last_basic_attack_at = Some(now);
    attacker.state.basic_attack_cooldown_secs = cooldown.as_secs_f32();
    attacker.state.basic_attack_remaining_secs = cooldown.as_secs_f32();
    attacker.state.action_sequence = attacker.state.action_sequence.wrapping_add(1).max(1);
    attacker.state.action_kind = PlayerActionKind::Attack;
    attacker.state.action_slot = BASIC_ATTACK_ACTION_SLOT;
    let id = *next_projectile_id;
    *next_projectile_id += 1;
    projectiles.insert(
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

pub(crate) fn refresh_basic_attack_cooldowns(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    now: Instant,
) {
    for player in players.values_mut() {
        if player.state.hp <= 0.0 {
            player.last_basic_attack_at = None;
        }
        let duration = basic_attack_cooldown(
            basic_attack_for_class(player.state.hero_class),
            player.state.item_bonuses,
        );
        player.state.basic_attack_cooldown_secs = duration.as_secs_f32();
        player.state.basic_attack_remaining_secs = player
            .last_basic_attack_at
            .map(|last| {
                duration
                    .saturating_sub(now.saturating_duration_since(last))
                    .as_secs_f32()
            })
            .unwrap_or(0.0);
    }
}

#[cfg(test)]
mod tests;
