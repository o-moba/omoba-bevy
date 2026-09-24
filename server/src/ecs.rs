//! Bevy resources and systems that drive the server tick.
use crate::*;

#[derive(Resource, Default)]
pub(crate) struct EcsPlayerEntities {
    pub(crate) by_player_id: HashMap<u64, Entity>,
}

#[derive(Resource, Default)]
pub(crate) struct SimulationDeltaSeconds {
    pub(crate) value: f32,
}

#[derive(Resource, Default)]
pub(crate) struct TickContext {
    pub(crate) now: Option<Instant>,
    pub(crate) dt: f32,
}

pub(crate) fn server_prepare_tick_system(
    mut runtime: ResMut<ServerRuntime>,
    mut tick: ResMut<TickContext>,
    mut simulation_delta: ResMut<SimulationDeltaSeconds>,
) {
    let (now, dt) = runtime.prepare_tick();
    tick.now = Some(now);
    tick.dt = dt;
    simulation_delta.value = dt;
}

pub(crate) fn sync_players_into_ecs_system(
    runtime: Res<ServerRuntime>,
    mut commands: Commands,
    mut entities: ResMut<EcsPlayerEntities>,
    mut query: Query<(&mut Transform3D, &mut Health, &mut Mana, &mut TeamMarker), With<Player>>,
) {
    let live_ids = runtime
        .world
        .players
        .values()
        .filter(|player| player.joined)
        .map(|player| player.state.id)
        .collect::<HashSet<_>>();
    let stale_ids = entities
        .by_player_id
        .keys()
        .copied()
        .filter(|player_id| !live_ids.contains(player_id))
        .collect::<Vec<_>>();

    for stale_player_id in stale_ids {
        if let Some(entity) = entities.by_player_id.remove(&stale_player_id) {
            commands.entity(entity).despawn();
        }
    }

    for connected_player in runtime.world.players.values() {
        if !connected_player.joined {
            continue;
        }
        let state = &connected_player.state;
        let Some(entity) = entities.by_player_id.get(&state.id).copied() else {
            let entity = commands
                .spawn((
                    Player,
                    Transform3D {
                        position: Vec3::new(state.x, state.y, state.z),
                        yaw: state.yaw,
                    },
                    Health {
                        current: state.hp,
                        max: state.max_hp,
                    },
                    Mana {
                        current: state.mana,
                        max: state.max_mana,
                    },
                    TeamMarker(state.team),
                ))
                .id();
            entities.by_player_id.insert(state.id, entity);
            continue;
        };

        if let Ok((mut transform, mut health, mut mana, mut team)) = query.get_mut(entity) {
            transform.position = Vec3::new(state.x, state.y, state.z);
            transform.yaw = state.yaw;
            health.current = state.hp;
            health.max = state.max_hp;
            mana.current = state.mana;
            mana.max = state.max_mana;
            team.0 = state.team;
        } else {
            commands.entity(entity).despawn();
            let replacement = commands
                .spawn((
                    Player,
                    Transform3D {
                        position: Vec3::new(state.x, state.y, state.z),
                        yaw: state.yaw,
                    },
                    Health {
                        current: state.hp,
                        max: state.max_hp,
                    },
                    Mana {
                        current: state.mana,
                        max: state.max_mana,
                    },
                    TeamMarker(state.team),
                ))
                .id();
            entities.by_player_id.insert(state.id, replacement);
        }
    }
}

pub(crate) fn regenerate_mana_system(
    simulation_delta: Res<SimulationDeltaSeconds>,
    mut query: Query<(&mut Mana, &Health), With<Player>>,
) {
    if simulation_delta.value <= 0.0 {
        return;
    }

    for (mut mana, health) in &mut query {
        if health.current <= 0.0 {
            continue;
        }
        if mana.max <= 0.0 {
            mana.max = MAX_MANA;
        }
        mana.current =
            (mana.current + MANA_REGEN_PER_SECOND * simulation_delta.value).clamp(0.0, mana.max);
    }
}

pub(crate) fn sync_players_from_ecs_system(
    mut runtime: ResMut<ServerRuntime>,
    entities: Res<EcsPlayerEntities>,
    query: Query<&Mana, With<Player>>,
) {
    for connected_player in runtime.world.players.values_mut() {
        let player_id = connected_player.state.id;
        let Some(entity) = entities.by_player_id.get(&player_id).copied() else {
            continue;
        };
        let Ok(mana) = query.get(entity) else {
            continue;
        };
        connected_player.state.mana = mana.current;
        connected_player.state.max_mana = mana.max;
    }
}

pub(crate) fn server_finalize_tick_system(
    mut runtime: ResMut<ServerRuntime>,
    tick: Res<TickContext>,
) {
    let Some(now) = tick.now else {
        return;
    };
    runtime.simulate_after_mana(now, tick.dt);
}
