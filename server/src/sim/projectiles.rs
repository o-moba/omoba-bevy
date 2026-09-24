//! Homing projectile flight and impact resolution.
use crate::*;

pub(crate) fn simulate_projectiles(world: &mut GameWorld, tick: TickCtx) -> Vec<CombatEvent> {
    let TickCtx { now, dt } = tick;
    let GameWorld {
        players,
        structures,
        neutrals,
        team_buffs,
        projectiles,
        game_state,
        ..
    } = world;
    if !matches!(game_state, GameState::Running) {
        return Vec::new();
    }
    let mut damage_events: Vec<(u64, TargetId, f32, Team, HitSource)> = Vec::new();

    projectiles.retain(|_, projectile| {
        if !projectile.guaranteed_hit && now >= projectile.expires_at {
            return false;
        }

        match projectile.target.kind {
            TargetKind::Player => {
                let Some(target) = players.values().find(|player| {
                    player.state.id == projectile.target.id && player.state.hp > 0.0
                }) else {
                    return false;
                };

                let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
                let target_pos =
                    Vec3f::new(target.state.x, target.state.y + AIM_HEIGHT, target.state.z);
                if projectile.homing {
                    let direction = Vec3f::new(
                        target_pos.x - start.x,
                        target_pos.y - start.y,
                        target_pos.z - start.z,
                    )
                    .normalize_or_zero();
                    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                        damage_events.push((
                            projectile.state.id,
                            projectile.target,
                            projectile.damage,
                            projectile.state.owner_team,
                            HitSource::projectile(&projectile.state),
                        ));
                        return false;
                    }
                    projectile.state.direction = [direction.x, direction.y, direction.z];
                    projectile.velocity = Vec3f::new(
                        direction.x * PROJECTILE_SPEED,
                        direction.y * PROJECTILE_SPEED,
                        direction.z * PROJECTILE_SPEED,
                    );
                }
                let end = start.add_scaled(projectile.velocity, dt);
                projectile.state.x = end.x;
                projectile.state.y = end.y;
                projectile.state.z = end.z;

                let combined_radius = projectile.radius + PLAYER_HIT_RADIUS;
                if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
                    damage_events.push((
                        projectile.state.id,
                        projectile.target,
                        projectile.damage,
                        projectile.state.owner_team,
                        HitSource::projectile(&projectile.state),
                    ));
                    return false;
                }
            }
            TargetKind::Minion => {
                // Projectile-vs-minion collisions are handled by ECS combat systems.
            }
            TargetKind::Structure => {
                let Some(structure) = structures.get(&projectile.target.id) else {
                    return false;
                };
                if structure.state.hp <= 0.0 {
                    return false;
                }
                let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
                let target_pos =
                    Vec3f::new(structure.state.x, structure.state.y, structure.state.z);
                if projectile.homing {
                    let direction = Vec3f::new(
                        target_pos.x - start.x,
                        target_pos.y - start.y,
                        target_pos.z - start.z,
                    )
                    .normalize_or_zero();
                    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                        damage_events.push((
                            projectile.state.id,
                            projectile.target,
                            projectile.damage,
                            projectile.state.owner_team,
                            HitSource::projectile(&projectile.state),
                        ));
                        return false;
                    }
                    projectile.state.direction = [direction.x, direction.y, direction.z];
                    projectile.velocity = Vec3f::new(
                        direction.x * PROJECTILE_SPEED,
                        direction.y * PROJECTILE_SPEED,
                        direction.z * PROJECTILE_SPEED,
                    );
                }
                let end = start.add_scaled(projectile.velocity, dt);
                projectile.state.x = end.x;
                projectile.state.y = end.y;
                projectile.state.z = end.z;

                let target_radius = match structure.state.kind {
                    StructureKind::Tower => TOWER_SIZE * 0.5,
                    StructureKind::BaseTower => BASE_TOWER_SIZE * 0.5,
                };
                let combined_radius = projectile.radius + target_radius;
                if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
                    damage_events.push((
                        projectile.state.id,
                        projectile.target,
                        projectile.damage,
                        projectile.state.owner_team,
                        HitSource::projectile(&projectile.state),
                    ));
                    return false;
                }
            }
            TargetKind::Neutral => {
                let Some(target_neutral) = neutrals.get(&projectile.target.id) else {
                    return false;
                };
                if target_neutral.dead_until.is_some() || target_neutral.state.hp <= 0.0 {
                    return false;
                }

                let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
                let target_pos = Vec3f::new(
                    target_neutral.state.x,
                    target_neutral.state.y + NEUTRAL_RADIUS * 0.85,
                    target_neutral.state.z,
                );
                if projectile.homing {
                    let direction = Vec3f::new(
                        target_pos.x - start.x,
                        target_pos.y - start.y,
                        target_pos.z - start.z,
                    )
                    .normalize_or_zero();
                    if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                        damage_events.push((
                            projectile.state.id,
                            projectile.target,
                            projectile.damage,
                            projectile.state.owner_team,
                            HitSource::projectile(&projectile.state),
                        ));
                        return false;
                    }
                    projectile.state.direction = [direction.x, direction.y, direction.z];
                    projectile.velocity = Vec3f::new(
                        direction.x * PROJECTILE_SPEED,
                        direction.y * PROJECTILE_SPEED,
                        direction.z * PROJECTILE_SPEED,
                    );
                }
                let end = start.add_scaled(projectile.velocity, dt);
                projectile.state.x = end.x;
                projectile.state.y = end.y;
                projectile.state.z = end.z;

                let combined_radius = projectile.radius + NEUTRAL_RADIUS;
                if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
                    damage_events.push((
                        projectile.state.id,
                        projectile.target,
                        projectile.damage,
                        projectile.state.owner_team,
                        HitSource::projectile(&projectile.state),
                    ));
                    return false;
                }
            }
        }

        true
    });

    // HashMap traversal must not decide which simultaneous base hit wins.
    // Apply impacts in projectile creation order and retain the final blow receipt.
    damage_events.sort_unstable_by_key(|(id, ..)| *id);
    let mut receipts = Vec::new();
    for (_, target, damage, attacker_team, source) in damage_events {
        if !matches!(game_state, GameState::Running) {
            break;
        }
        let event = match target.kind {
            TargetKind::Player => apply_player_damage_typed(
                players,
                target.id,
                damage,
                now,
                source.action_slot.is_some_and(|slot| slot < 4),
            ),
            TargetKind::Structure => {
                apply_structure_damage(structures, target.id, damage, attacker_team, game_state)
            }
            TargetKind::Neutral => apply_neutral_damage(
                players,
                neutrals,
                team_buffs,
                target.id,
                damage,
                source.entity.id,
                now,
            ),
            TargetKind::Minion => None, // Applied in the earlier ECS combat phase.
        };
        receipts.extend(event.map(|event| source.annotate(event)));
    }
    receipts
}

pub(crate) fn swept_sphere_intersects_target(
    start: Vec3f,
    end: Vec3f,
    target: Vec3f,
    radius: f32,
) -> bool {
    let seg_x = end.x - start.x;
    let seg_y = end.y - start.y;
    let seg_z = end.z - start.z;
    let seg_len_sq = seg_x * seg_x + seg_y * seg_y + seg_z * seg_z;
    if seg_len_sq <= 0.000_001 {
        return start.distance_squared(target) <= radius * radius;
    }

    let to_target_x = target.x - start.x;
    let to_target_y = target.y - start.y;
    let to_target_z = target.z - start.z;
    let t = ((to_target_x * seg_x + to_target_y * seg_y + to_target_z * seg_z) / seg_len_sq)
        .clamp(0.0, 1.0);
    let closest = Vec3f::new(
        start.x + seg_x * t,
        start.y + seg_y * t,
        start.z + seg_z * t,
    );

    closest.distance_squared(target) <= radius * radius
}
