use crate::*;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

pub(crate) struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DamageEvent>()
            .init_resource::<EcsMinionEntities>();
    }
}

#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct DamageEvent {
    pub(crate) target_id: u64,
    pub(crate) amount: f32,
    pub(crate) attacker_team: Team,
}

#[derive(Component)]
pub(crate) struct CombatMinion;

#[derive(Resource, Default)]
pub(crate) struct EcsMinionEntities {
    by_minion_id: HashMap<u64, Entity>,
}

pub(crate) fn sync_minions_into_ecs_system(
    runtime: Res<ServerRuntime>,
    mut commands: Commands,
    mut entities: ResMut<EcsMinionEntities>,
    mut query: Query<(&mut Transform3D, &mut Health, &mut TeamMarker), With<CombatMinion>>,
) {
    let live_ids = runtime
        .minions
        .values()
        .filter(|minion| minion.state.hp > 0.0)
        .map(|minion| minion.state.id)
        .collect::<HashSet<_>>();

    let stale_ids = entities
        .by_minion_id
        .keys()
        .copied()
        .filter(|minion_id| !live_ids.contains(minion_id))
        .collect::<Vec<_>>();

    for stale_minion_id in stale_ids {
        if let Some(entity) = entities.by_minion_id.remove(&stale_minion_id) {
            commands.entity(entity).despawn();
        }
    }

    for minion in runtime
        .minions
        .values()
        .filter(|minion| minion.state.hp > 0.0)
    {
        let state = &minion.state;
        let Some(entity) = entities.by_minion_id.get(&state.id).copied() else {
            let entity = commands
                .spawn((
                    CombatMinion,
                    Transform3D {
                        position: Vec3::new(state.x, state.y, state.z),
                        yaw: state.yaw,
                    },
                    Health {
                        current: state.hp,
                        max: state.max_hp,
                    },
                    TeamMarker(state.team),
                ))
                .id();
            entities.by_minion_id.insert(state.id, entity);
            continue;
        };

        if let Ok((mut transform, mut health, mut team)) = query.get_mut(entity) {
            transform.position = Vec3::new(state.x, state.y, state.z);
            transform.yaw = state.yaw;
            health.current = state.hp;
            health.max = state.max_hp;
            team.0 = state.team;
        } else {
            commands.entity(entity).despawn();
            let replacement = commands
                .spawn((
                    CombatMinion,
                    Transform3D {
                        position: Vec3::new(state.x, state.y, state.z),
                        yaw: state.yaw,
                    },
                    Health {
                        current: state.hp,
                        max: state.max_hp,
                    },
                    TeamMarker(state.team),
                ))
                .id();
            entities.by_minion_id.insert(state.id, replacement);
        }
    }
}

pub(crate) fn collect_projectile_minion_damage_system(
    tick: Res<TickContext>,
    entities: Res<EcsMinionEntities>,
    minion_query: Query<(&Transform3D, &Health), With<CombatMinion>>,
    mut runtime: ResMut<ServerRuntime>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    let Some(now) = tick.now else {
        return;
    };
    if !matches!(runtime.game_state, GameState::Running) {
        return;
    }
    if tick.dt <= 0.0 {
        return;
    }

    let dt = tick.dt;
    let mut queued_damage: Vec<DamageEvent> = Vec::new();

    runtime.projectiles.retain(|_, projectile| {
        if projectile.target.kind != TargetKind::Minion {
            return true;
        }
        if !projectile.guaranteed_hit && now >= projectile.expires_at {
            return false;
        }

        let Some(target_entity) = entities.by_minion_id.get(&projectile.target.id).copied() else {
            return false;
        };
        let Ok((target_transform, target_health)) = minion_query.get(target_entity) else {
            return false;
        };
        if target_health.current <= 0.0 {
            return false;
        }

        let start = Vec3f::new(projectile.state.x, projectile.state.y, projectile.state.z);
        let target_pos = Vec3f::new(
            target_transform.position.x,
            target_transform.position.y + MINION_RADIUS * 0.8,
            target_transform.position.z,
        );

        if projectile.homing {
            let direction = Vec3f::new(
                target_pos.x - start.x,
                target_pos.y - start.y,
                target_pos.z - start.z,
            )
            .normalize_or_zero();
            if direction.x == 0.0 && direction.y == 0.0 && direction.z == 0.0 {
                queued_damage.push(DamageEvent {
                    target_id: projectile.target.id,
                    amount: projectile.damage,
                    attacker_team: projectile.state.owner_team,
                });
                return false;
            }
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

        let combined_radius = projectile.radius + MINION_RADIUS;
        if swept_sphere_intersects_target(start, end, target_pos, combined_radius) {
            queued_damage.push(DamageEvent {
                target_id: projectile.target.id,
                amount: projectile.damage,
                attacker_team: projectile.state.owner_team,
            });
            return false;
        }

        true
    });

    for damage_event in queued_damage {
        damage_events.write(damage_event);
    }
}

pub(crate) fn apply_projectile_minion_damage_system(
    mut runtime: ResMut<ServerRuntime>,
    mut damage_events: MessageReader<DamageEvent>,
) {
    let runtime = runtime.as_mut();
    for damage_event in damage_events.read() {
        apply_minion_damage(
            &mut runtime.players,
            &mut runtime.minions,
            damage_event.target_id,
            damage_event.amount,
            damage_event.attacker_team,
        );
    }
}
