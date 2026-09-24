//! Lane minion AI, marching and combat.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;

use shared::combat::{CombatEntityKind, CombatEvent, MinionKind, ProjectileStyle};
use shared::map::Team;
use shared::wire::{
    GameState, MinionBrainState, MinionTargetKind, StructureKind, TargetId, TargetKind,
};

use crate::balance::{
    AIM_HEIGHT, MINION_KILL_GOLD, MINION_KILL_XP, MINION_RADIUS, MINION_SPEED, MINION_VISION_RANGE,
    PLAYER_HIT_RADIUS,
};
use crate::combat_feedback::{
    HitSource, apply_player_damage, damage_receipt, minion_stats, spawn_caster_projectile,
};
use crate::entities::{ConnectedPlayer, Minion, MinionAggroTarget, StructureRole, Vec3f};
use crate::game_world::{GameWorld, TickCtx};
use crate::progression::grant_player_xp;
use crate::shop::award_gold;
use crate::sim::towers::{apply_structure_damage, structure_is_protected};
use crate::vision;
use crate::world::structure_radius;

pub(crate) fn apply_minion_damage(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    minions: &mut HashMap<u64, Minion>,
    target_id: u64,
    damage: f32,
    attacker_team: Team,
) -> Option<CombatEvent> {
    if !damage.is_finite() || damage <= 0.0 {
        return None;
    }
    let target_minion = minions.get_mut(&target_id)?;
    if target_minion.state.hp <= 0.0 || target_minion.state.team == attacker_team {
        return None;
    }
    let before = target_minion.state.hp;
    target_minion.state.hp = (before - damage).max(0.0);
    if target_minion.state.hp <= 0.0 {
        target_minion.state.state = MinionBrainState::Dead;
        target_minion.state.target_kind = None;
        target_minion.state.target_id = None;
        award_minion_kill_rewards(players, attacker_team);
    }
    damage_receipt(
        CombatEntityKind::Minion,
        target_id,
        before,
        target_minion.state.hp,
        Vec3f::new(
            target_minion.state.x,
            target_minion.state.y,
            target_minion.state.z,
        ),
    )
}

pub(crate) fn award_minion_kill_rewards(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    attacker_team: Team,
) {
    let mut recipients = players
        .iter()
        .filter(|(_, player)| player.joined && player.hero.identity.team == attacker_team)
        .map(|(addr, _)| *addr)
        .collect::<Vec<_>>();
    if recipients.is_empty() {
        return;
    }

    // Stable identity order makes integer remainder allocation reproducible.
    recipients.sort_unstable_by_key(|addr| players[addr].hero.identity.id);
    let per_player_gold = MINION_KILL_GOLD / recipients.len() as u32;
    let per_player_xp = MINION_KILL_XP / recipients.len() as u32;
    let bonus_gold_receivers = MINION_KILL_GOLD % recipients.len() as u32;
    let bonus_xp_receivers = MINION_KILL_XP % recipients.len() as u32;

    for (index, addr) in recipients.into_iter().enumerate() {
        let Some(player) = players.get_mut(&addr) else {
            continue;
        };
        let mut gold = per_player_gold;
        if (index as u32) < bonus_gold_receivers {
            gold += 1;
        }
        let mut xp = per_player_xp;
        if (index as u32) < bonus_xp_receivers {
            xp += 1;
        }
        award_gold(player, gold);
        if player.modifiers.grant_xp {
            grant_player_xp(&mut player.hero, xp);
        }
    }
}

pub(crate) fn simulate_minions(world: &mut GameWorld, tick: TickCtx) -> Vec<CombatEvent> {
    let TickCtx { now, dt } = tick;
    if !matches!(world.game_state, GameState::Running) {
        return Vec::new();
    }

    let green_sight = vision::sources(Team::Green, world);
    let blue_sight = vision::sources(Team::Blue, world);
    let GameWorld {
        players,
        minions,
        structures,
        projectiles,
        next_projectile_id,
        game_state,
        ..
    } = world;
    let visible_players: HashSet<u64> = players
        .values()
        .filter(|p| {
            vision::player_visible(
                if p.hero.identity.team == Team::Green {
                    &blue_sight
                } else {
                    &green_sight
                },
                p,
                now,
            )
        })
        .map(|p| p.hero.identity.id)
        .collect();
    let player_targets = players
        .values()
        .filter(|player| {
            player.joined
                && player.hero.hp > 0.0
                && visible_players.contains(&player.hero.identity.id)
        })
        .map(|player| {
            (
                player.hero.identity.id,
                player.hero.identity.team,
                Vec3f::new(player.hero.x, player.hero.y + AIM_HEIGHT, player.hero.z),
            )
        })
        .collect::<Vec<_>>();
    let minion_targets = minions
        .values()
        .filter(|minion| minion.state.hp > 0.0)
        .map(|minion| {
            (
                minion.state.id,
                minion.state.team,
                Vec3f::new(minion.state.x, minion.state.y, minion.state.z),
            )
        })
        .collect::<Vec<_>>();

    let mut player_damage_events: Vec<(u64, f32, HitSource)> = Vec::new();
    let mut minion_damage_events: Vec<(u64, f32, Team, HitSource)> = Vec::new();
    let mut structure_damage_events: Vec<(u64, f32, Team, HitSource)> = Vec::new();
    let minion_vision_sq = MINION_VISION_RANGE * MINION_VISION_RANGE;

    for minion in minions.values_mut() {
        if minion.state.hp <= 0.0 {
            minion.state.state = MinionBrainState::Dead;
            minion.state.target_kind = None;
            minion.state.target_id = None;
            continue;
        }
        minion.state.state = MinionBrainState::Marching;
        minion.state.target_kind = None;
        minion.state.target_id = None;

        let (_, attack_damage, attack_range, attack_cooldown) = minion_stats(minion.state.kind);
        let source = HitSource::new(
            CombatEntityKind::Minion,
            minion.state.id,
            ProjectileStyle::Standard,
        );
        let minion_position = Vec3f::new(minion.state.x, minion.state.y, minion.state.z);

        // Enemy minions always take priority. A minion never targets a player while
        // any enemy minion is within vision; players are only considered when no
        // enemy minion is in range. This overrides sticky player aggro too.
        let best_minion = minion_targets
            .iter()
            .filter(|(id, team, _)| *id != minion.state.id && *team != minion.state.team)
            .map(|(id, _, position)| {
                (
                    MinionAggroTarget::Minion(*id),
                    *position,
                    MINION_RADIUS,
                    minion_position.distance_squared(*position),
                )
            })
            .filter(|(_, _, _, dist_sq)| *dist_sq <= minion_vision_sq)
            .min_by(|left, right| {
                left.3
                    .partial_cmp(&right.3)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.0.id().cmp(&right.0.id()))
            });

        let aggro_target = if let Some(minion_target) = best_minion {
            Some((minion_target.0, minion_target.1, minion_target.2))
        } else {
            // No enemy minion in range: keep sticky player aggro if still valid,
            // otherwise pick the nearest enemy player in vision.
            let sticky_player = minion.aggro_target.and_then(|target| match target {
                MinionAggroTarget::Player(target_id) => player_targets
                    .iter()
                    .find(|(id, team, position)| {
                        *id == target_id
                            && *team != minion.state.team
                            && minion_position.distance_squared(*position) <= minion_vision_sq
                    })
                    .map(|(_, _, position)| (target, *position, PLAYER_HIT_RADIUS)),
                MinionAggroTarget::Minion(_) => None,
            });

            sticky_player.or_else(|| {
                player_targets
                    .iter()
                    .filter(|(_, team, _)| *team != minion.state.team)
                    .map(|(id, _, position)| {
                        (
                            MinionAggroTarget::Player(*id),
                            *position,
                            PLAYER_HIT_RADIUS,
                            minion_position.distance_squared(*position),
                        )
                    })
                    .filter(|(_, _, _, dist_sq)| *dist_sq <= minion_vision_sq)
                    .min_by(|left, right| {
                        left.3
                            .partial_cmp(&right.3)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(target, position, radius, _)| (target, position, radius))
            })
        };

        if let Some((target, target_pos, target_radius)) = aggro_target {
            minion.aggro_target = Some(target);
            minion.state.target_kind = Some(target.kind());
            minion.state.target_id = Some(target.id());
            let dir_x = target_pos.x - minion.state.x;
            let dir_z = target_pos.z - minion.state.z;
            let distance_sq = dir_x * dir_x + dir_z * dir_z;
            let attack_distance = attack_range + target_radius;
            if distance_sq <= attack_distance * attack_distance {
                minion.state.state = MinionBrainState::Attacking;
                let can_attack = minion
                    .last_attack_at
                    .is_none_or(|last| now.duration_since(last) >= attack_cooldown);
                if can_attack {
                    minion.last_attack_at = Some(now);
                    minion.state.attack_sequence =
                        minion.state.attack_sequence.wrapping_add(1).max(1);
                    if minion.state.kind == MinionKind::Caster {
                        let target_kind = match target {
                            MinionAggroTarget::Player(_) => TargetKind::Player,
                            MinionAggroTarget::Minion(_) => TargetKind::Minion,
                        };
                        spawn_caster_projectile(
                            minion,
                            TargetId {
                                kind: target_kind,
                                id: target.id(),
                            },
                            target_pos,
                            projectiles,
                            next_projectile_id,
                            now,
                        );
                    } else {
                        match target {
                            MinionAggroTarget::Player(target_id) => {
                                player_damage_events.push((target_id, attack_damage, source));
                            }
                            MinionAggroTarget::Minion(target_id) => {
                                minion_damage_events.push((
                                    target_id,
                                    attack_damage,
                                    minion.state.team,
                                    source,
                                ));
                            }
                        }
                    }
                }
            } else {
                minion.state.state = MinionBrainState::Chasing;
                let distance = distance_sq.sqrt();
                let travel = (MINION_SPEED * dt).min(distance);
                if distance > 0.0001 {
                    let inv_distance = distance.recip();
                    minion.state.x += dir_x * inv_distance * travel;
                    minion.state.z += dir_z * inv_distance * travel;
                }
            }
            if distance_sq > 0.0001 {
                minion.state.yaw = shared::math::unit_yaw_towards(dir_x, dir_z);
            }
            continue;
        }

        minion.aggro_target = None;
        let target = structures
            .values()
            .filter(|structure| {
                if structure.state.hp <= 0.0
                    || structure.state.team == minion.state.team
                    || structure_is_protected(structures, structure.state.id)
                {
                    return false;
                }
                match structure.role {
                    StructureRole::LaneTower { lane } => lane == minion.state.lane,
                    StructureRole::BaseTower => true,
                }
            })
            .min_by(|left, right| {
                let left_base = left.state.kind == StructureKind::BaseTower;
                let right_base = right.state.kind == StructureKind::BaseTower;
                if left_base != right_base {
                    return left_base.cmp(&right_base);
                }
                let left_pos = Vec3f::new(left.state.x, left.state.y, left.state.z);
                let right_pos = Vec3f::new(right.state.x, right.state.y, right.state.z);
                minion_position
                    .distance_squared(left_pos)
                    .partial_cmp(&minion_position.distance_squared(right_pos))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|structure| {
                let position = Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
                // Ground units use horizontal reach. The legacy box-center Y
                // is presentation/aim data, not extra distance to a lane tower.
                let dx = position.x - minion_position.x;
                let dz = position.z - minion_position.z;
                let distance_sq = dx * dx + dz * dz;
                (
                    structure.state.id,
                    structure.state.kind,
                    position,
                    distance_sq,
                )
            });

        if let Some((target_id, target_kind, target_pos, distance_sq)) = target {
            minion.state.target_kind = Some(MinionTargetKind::Structure);
            minion.state.target_id = Some(target_id);
            let attack_distance = attack_range + structure_radius(target_kind);
            if distance_sq <= attack_distance * attack_distance {
                minion.state.state = MinionBrainState::Attacking;
                let can_attack = minion
                    .last_attack_at
                    .is_none_or(|last| now.duration_since(last) >= attack_cooldown);
                if can_attack {
                    minion.last_attack_at = Some(now);
                    minion.state.attack_sequence =
                        minion.state.attack_sequence.wrapping_add(1).max(1);
                    if minion.state.kind == MinionKind::Caster {
                        spawn_caster_projectile(
                            minion,
                            TargetId {
                                kind: TargetKind::Structure,
                                id: target_id,
                            },
                            target_pos,
                            projectiles,
                            next_projectile_id,
                            now,
                        );
                    } else {
                        structure_damage_events.push((
                            target_id,
                            attack_damage,
                            minion.state.team,
                            source,
                        ));
                    }
                }
                let dir_x = target_pos.x - minion.state.x;
                let dir_z = target_pos.z - minion.state.z;
                if dir_x * dir_x + dir_z * dir_z > 0.0001 {
                    minion.state.yaw = shared::math::unit_yaw_towards(dir_x, dir_z);
                }
                continue;
            }
            minion.state.state = MinionBrainState::Chasing;
        }

        while minion.next_waypoint < minion.path.len() {
            let waypoint = minion.path[minion.next_waypoint];
            let dir_x = waypoint.x - minion.state.x;
            let dir_z = waypoint.z - minion.state.z;
            let distance_sq = dir_x * dir_x + dir_z * dir_z;
            if distance_sq <= 0.01 {
                minion.next_waypoint += 1;
                continue;
            }

            let distance = distance_sq.sqrt();
            let travel = (MINION_SPEED * dt).min(distance);
            let inv_distance = distance.recip();
            minion.state.x += dir_x * inv_distance * travel;
            minion.state.z += dir_z * inv_distance * travel;
            minion.state.yaw = shared::math::unit_yaw_towards(dir_x, dir_z);
            if travel >= distance - 0.001 {
                minion.next_waypoint += 1;
            }
            break;
        }
    }

    let mut receipts = Vec::new();
    for (target_id, damage, source) in player_damage_events {
        receipts.extend(
            apply_player_damage(players, target_id, damage, now)
                .map(|event| source.annotate(event)),
        );
    }
    for (target_id, damage, attacker_team, source) in minion_damage_events {
        receipts.extend(
            apply_minion_damage(players, minions, target_id, damage, attacker_team)
                .map(|event| source.annotate(event)),
        );
    }
    // Stable source order resolves simultaneous melee base hits consistently.
    structure_damage_events.sort_unstable_by_key(|(_, _, _, source)| source.entity.id);
    for (target_id, damage, attacker_team, source) in structure_damage_events {
        if !matches!(game_state, GameState::Running) {
            break;
        }
        receipts.extend(
            apply_structure_damage(structures, target_id, damage, attacker_team, game_state)
                .map(|event| source.annotate(event)),
        );
    }
    receipts
}
