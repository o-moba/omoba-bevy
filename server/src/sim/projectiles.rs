//! Homing projectile flight and impact resolution.

use crate::balance::PROJECTILE_SPEED;
use shared::wire::TargetId;

use shared::wire::TargetKind;

use crate::game_world::TickCtx;

use crate::balance::PLAYER_HIT_RADIUS;

use crate::balance::BASE_TOWER_SIZE;

use crate::sim::minions::apply_minion_damage;

use crate::sim::neutrals::apply_neutral_damage;

use crate::balance::NEUTRAL_RADIUS;

use crate::balance::TOWER_SIZE;

use crate::entities::Vec3f;

use crate::balance::MINION_RADIUS;

use shared::wire::GameState;

use crate::entities::Projectile;

use crate::balance::AIM_HEIGHT;

use crate::game_world::GameWorld;

use shared::combat::CombatEvent;

use crate::combat_feedback::apply_player_damage_typed;

use crate::combat_feedback::HitSource;

use shared::map::Team;

use shared::wire::StructureKind;

use crate::sim::towers::apply_structure_damage;

/// Flies every projectile one step and resolves the impacts. The tick still
/// runs the minion-targeted and the other projectiles as two filtered passes
/// at their historical points in the frame; tests exercise the whole path.
#[cfg(test)]
pub(crate) fn simulate_projectiles(world: &mut GameWorld, tick: TickCtx) -> Vec<CombatEvent> {
    simulate_projectiles_filtered(world, tick, |_| true)
}

/// Flies only the projectiles whose target kind passes `targets`; the others
/// are left untouched. The tick uses it to keep the minion-targeted pass at
/// its historical point in the frame, ahead of the rest of the simulation.
pub(crate) fn simulate_projectiles_filtered(
    world: &mut GameWorld,
    tick: TickCtx,
    targets: impl Fn(TargetKind) -> bool,
) -> Vec<CombatEvent> {
    let TickCtx { now, dt } = tick;
    let GameWorld {
        players,
        structures,
        minions,
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
        if !targets(projectile.target.kind) {
            return true;
        }
        if !projectile.guaranteed_hit && now >= projectile.expires_at {
            return false;
        }

        // A projectile whose target is gone or dead is dropped without impact.
        let (target_pos, target_radius) = match projectile.target.kind {
            TargetKind::Player => {
                let Some(target) = players.values().find(|player| {
                    player.hero.identity.id == projectile.target.id && player.hero.hp > 0.0
                }) else {
                    return false;
                };
                (
                    Vec3f::new(target.hero.x, target.hero.y + AIM_HEIGHT, target.hero.z),
                    PLAYER_HIT_RADIUS,
                )
            }
            TargetKind::Minion => {
                let Some(minion) = minions.get(&projectile.target.id) else {
                    return false;
                };
                if minion.state.hp <= 0.0 {
                    return false;
                }
                (
                    Vec3f::new(
                        minion.state.x,
                        minion.state.y + MINION_RADIUS * 0.8,
                        minion.state.z,
                    ),
                    MINION_RADIUS,
                )
            }
            TargetKind::Structure => {
                let Some(structure) = structures.get(&projectile.target.id) else {
                    return false;
                };
                if structure.state.hp <= 0.0 {
                    return false;
                }
                let target_radius = match structure.state.kind {
                    StructureKind::Tower => TOWER_SIZE * 0.5,
                    StructureKind::BaseTower => BASE_TOWER_SIZE * 0.5,
                };
                (
                    Vec3f::new(structure.state.x, structure.state.y, structure.state.z),
                    target_radius,
                )
            }
            TargetKind::Neutral => {
                let Some(neutral) = neutrals.get(&projectile.target.id) else {
                    return false;
                };
                if neutral.dead_until.is_some() || neutral.state.hp <= 0.0 {
                    return false;
                }
                (
                    Vec3f::new(
                        neutral.state.x,
                        neutral.state.y + NEUTRAL_RADIUS * 0.85,
                        neutral.state.z,
                    ),
                    NEUTRAL_RADIUS,
                )
            }
        };

        if step_homing(projectile, target_pos, target_radius, dt) {
            damage_events.push((
                projectile.state.id,
                projectile.target,
                projectile.damage,
                projectile.state.owner_team,
                HitSource::projectile(&projectile.state),
            ));
            return false;
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
            TargetKind::Minion => {
                apply_minion_damage(players, minions, target.id, damage, attacker_team)
            }
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
        };
        receipts.extend(event.map(|event| source.annotate(event)));
    }
    receipts
}

/// Steers a homing projectile at `target_pos`, advances it by `dt` and reports
/// whether its swept sphere reached the target. A homing projectile already
/// sitting on its target counts as a hit.
fn step_homing(
    projectile: &mut Projectile,
    target_pos: Vec3f,
    target_radius: f32,
    dt: f32,
) -> bool {
    let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
    if projectile.homing {
        let direction = Vec3f::new(
            target_pos.x - start.x,
            target_pos.y - start.y,
            target_pos.z - start.z,
        )
        .normalize_or_zero();
        if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
            return true;
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

    swept_sphere_intersects_target(start, end, target_pos, projectile.radius + target_radius)
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
