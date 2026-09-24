//! Structure protection, damage and tower fire.

use crate::balance::PROJECTILE_SPEED;
use crate::entities::Vec3f;

use shared::wire::GameState;

use crate::entities::Projectile;

use crate::balance::AIM_HEIGHT;

use shared::wire::ProjectileState;

use std::time::Instant;

use shared::combat::CombatEntityKind;

use crate::game_world::GameWorld;

use crate::balance::BASE_TOWER_SHOT_HEIGHT;

use shared::combat::CombatEvent;

use crate::combat_feedback::HitSource;

use shared::map::Team;

use crate::vision;

use crate::balance::PROJECTILE_LIFETIME;

use shared::map::Lane;

use shared::wire::TargetId;

use shared::wire::TargetKind;

use shared::combat::ProjectileStyle;

use crate::combat_feedback::damage_receipt;

use crate::balance::PROJECTILE_RADIUS;

use std::collections::HashMap;

use crate::balance::TOWER_SHOT_HEIGHT;

use shared::wire::StructureKind;

use crate::entities::StructureRole;

use crate::sim::minions::apply_minion_damage;

use crate::entities::Structure;

/// Each defending lane unlocks front-to-back. A base unlocks when any
/// configured nonempty lane is cleared; an arena with no lane towers is open.
pub(crate) fn structure_is_protected(structures: &HashMap<u64, Structure>, target_id: u64) -> bool {
    let Some(target) = structures.get(&target_id) else {
        return false;
    };
    match target.role {
        StructureRole::LaneTower { lane } => structures.values().any(|other| {
            other.state.team == target.state.team
                && other.role == StructureRole::LaneTower { lane }
                && other.state.tier < target.state.tier
                && other.state.hp > 0.0
        }),
        StructureRole::BaseTower => {
            let mut has_lane = false;
            for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
                let towers: Vec<_> = structures
                    .values()
                    .filter(|other| {
                        other.state.team == target.state.team
                            && other.role == StructureRole::LaneTower { lane }
                    })
                    .collect();
                if towers.is_empty() {
                    continue;
                }
                has_lane = true;
                if towers.iter().all(|tower| tower.state.hp <= 0.0) {
                    return false;
                }
            }
            has_lane
        }
    }
}

pub(crate) fn apply_structure_damage(
    structures: &mut HashMap<u64, Structure>,
    target_id: u64,
    damage: f32,
    attacker_team: Team,
    game_state: &mut GameState,
) -> Option<CombatEvent> {
    if !matches!(game_state, GameState::Running) || !damage.is_finite() || damage <= 0.0 {
        return None;
    }
    if structure_is_protected(structures, target_id) {
        return None;
    }
    let target = structures.get_mut(&target_id)?;
    if target.state.hp <= 0.0 || target.state.team == attacker_team {
        return None;
    }
    let before = target.state.hp;
    target.state.hp = (before - damage).max(0.0);
    if target.state.hp <= 0.0 && target.state.kind == StructureKind::BaseTower {
        *game_state = GameState::Victory {
            winner: attacker_team,
        };
    }
    damage_receipt(
        CombatEntityKind::Structure,
        target_id,
        before,
        target.state.hp,
        Vec3f::new(target.state.x, target.state.y, target.state.z),
    )
}

pub(crate) fn simulate_tower_attacks(world: &mut GameWorld, now: Instant) -> Vec<CombatEvent> {
    if !matches!(world.game_state, GameState::Running) {
        return Vec::new();
    }
    let green_sight = vision::sources(Team::Green, world);
    let blue_sight = vision::sources(Team::Blue, world);
    let GameWorld {
        players,
        minions,
        projectiles,
        structures,
        next_projectile_id,
        ..
    } = world;
    let mut towers_to_fire: Vec<(u64, Team, Vec3f, u64, Vec3f, f32, f32)> = Vec::new();
    let mut minion_damage_events: Vec<(u64, f32, Team, HitSource)> = Vec::new();

    for structure in structures.values_mut() {
        if structure.state.hp <= 0.0 {
            continue;
        }
        if structure.attack_damage <= 0.0 || structure.attack_range <= 0.0 {
            continue;
        }
        if structure
            .last_attack_at
            .is_some_and(|last| now.duration_since(last) < structure.attack_cooldown)
        {
            continue;
        }

        let tower_position = Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
        let range_sq = structure.attack_range * structure.attack_range;

        let best_minion = minions
            .values()
            .filter(|minion| minion.state.hp > 0.0 && minion.state.team != structure.state.team)
            .map(|minion| {
                let pos = Vec3f::new(minion.state.x, minion.state.y, minion.state.z);
                (minion.state.id, pos, tower_position.distance_squared(pos))
            })
            .filter(|(_, _, dist_sq)| *dist_sq <= range_sq)
            .min_by(|left, right| {
                left.2
                    .partial_cmp(&right.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some((target_id, _, _)) = best_minion {
            structure.last_attack_at = Some(now);
            minion_damage_events.push((
                target_id,
                structure.attack_damage,
                structure.state.team,
                HitSource::new(
                    CombatEntityKind::Structure,
                    structure.state.id,
                    ProjectileStyle::TowerBolt,
                ),
            ));
            continue;
        }

        let mut best_target: Option<(u64, Vec3f, f32)> = None;
        for player in players.values() {
            if !vision::player_visible(
                if structure.state.team == Team::Green {
                    &green_sight
                } else {
                    &blue_sight
                },
                player,
                now,
            ) || !player.joined
                || player.hero.hp <= 0.0
                || player.hero.identity.team == structure.state.team
            {
                continue;
            }
            let target_pos = Vec3f::new(player.hero.x, player.hero.y + AIM_HEIGHT, player.hero.z);
            let dist_sq = tower_position.distance_squared(target_pos);
            if dist_sq <= range_sq && best_target.is_none_or(|(_, _, best)| dist_sq < best) {
                best_target = Some((player.hero.identity.id, target_pos, dist_sq));
            }
        }

        if let Some((target_id, target_pos, _)) = best_target {
            structure.last_attack_at = Some(now);
            towers_to_fire.push((
                structure.state.id,
                structure.state.team,
                tower_position,
                target_id,
                target_pos,
                structure.attack_damage * structure.hero_damage_multiplier,
                match structure.state.kind {
                    StructureKind::Tower => TOWER_SHOT_HEIGHT,
                    StructureKind::BaseTower => BASE_TOWER_SHOT_HEIGHT,
                },
            ));
        }
    }

    let mut receipts = Vec::new();
    for (target_id, damage, attacker_team, source) in minion_damage_events {
        receipts.extend(
            apply_minion_damage(players, minions, target_id, damage, attacker_team)
                .map(|event| source.annotate(event)),
        );
    }

    for (tower_id, team, tower_position, target_id, target_pos, damage, shot_height) in
        towers_to_fire
    {
        let origin = Vec3f::new(
            tower_position.x,
            tower_position.y + shot_height,
            tower_position.z,
        );
        let direction = Vec3f::new(
            target_pos.x - origin.x,
            target_pos.y - origin.y,
            target_pos.z - origin.z,
        )
        .normalize_or_zero();

        if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
            continue;
        }

        let projectile_id = *next_projectile_id;
        *next_projectile_id += 1;

        projectiles.insert(
            projectile_id,
            Projectile {
                state: ProjectileState {
                    source_kind: CombatEntityKind::Structure,
                    style: ProjectileStyle::TowerBolt,
                    action_slot: None,
                    direction: [direction.x, direction.y, direction.z],
                    id: projectile_id,
                    owner_id: tower_id,
                    owner_team: team,
                    x: origin.x,
                    y: origin.y,
                    z: origin.z,
                },
                target: TargetId {
                    kind: TargetKind::Player,
                    id: target_id,
                },
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
    receipts
}
