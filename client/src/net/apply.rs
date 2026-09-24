//! Applies the staged server snapshot to the ECS world (spawn, reconcile, despawn).

use bevy::ecs::query::Or;
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use std::collections::{HashMap, HashSet};

use shared::protocol::SnapshotMeta;
use shared::wire::{MinionState, NeutralState, PlayerState, ProjectileState, StructureState};

use crate::bosses::BossVisual;
use crate::camera::{CameraState, MainCamera, locked_camera_offset_for_team};
use crate::combat::CombatStats;
use crate::domain::RoundId;
use crate::model_scale::{ModelScaleSource, NormalizeModelScale, model_scale_key};
use crate::player::{
    DEBUG_SPEED_MULTIPLIER, DebugSpeedBoost, PLAYER_SIZE, Player, PlayerBody, VerticalVelocity,
};
use crate::sprite::PlayerVisualMode;
use crate::team::{Team, TeamSelection};
use crate::world::{PlayerAssets, PlayerModelResolver};

use super::components::*;
use super::ingest::{PendingServerSnapshotFrame, PendingSnapshotData};
use super::interpolate::{NetEntityInterpolation, RemotePlayerInterpolation};
use super::session::{ClientConnectionState, ClientSession, SessionEvent, flush_session_events};
use super::{ClientNetPipeline, SnapshotApply, UPDATE_INTERVAL_SECONDS};

const LOCAL_SNAP_DISTANCE: f32 = 4.0;

/// A remote player's Ekza store avatar is resolved when the player spawns. If
/// the catalogue entry or the verified model only arrives later, drop the
/// stand-in entity; the next snapshot spawns it again with the real model.
pub(in crate::net) fn respawn_players_with_new_store_models(
    mut commands: Commands,
    mut network_state: ResMut<NetworkState>,
    remote_query: Query<
        (Entity, &NetworkPlayerId, &NetworkAvatar),
        Or<(With<RemotePlayer>, With<Player>)>,
    >,
) {
    let changed = omoba_passport::store::take_changed();
    if changed.is_empty() {
        return;
    }
    for (entity, id, avatar) in &remote_query {
        if avatar.0.as_ref().is_some_and(|slug| changed.contains(slug)) {
            network_state.remote_players.remove(&id.0);
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
    }
}

// Replace the loaded scene as well as its cosmetic component when a developer
// changes appearance. The ordinary next snapshot reconstructs the actor.
pub(in crate::net) fn respawn_sandbox_models(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    mut state: ResMut<NetworkState>,
    players: Query<(Entity, &NetworkPlayerId, &NetworkAvatar)>,
    mut seen: Local<HashMap<u64, Option<String>>>,
) {
    if game.sandbox.is_none() {
        seen.clear();
        return;
    }
    seen.retain(|id, _| players.iter().any(|(_, current, _)| current.0 == *id));
    for (entity, id, avatar) in &players {
        let changed = seen
            .insert(id.0, avatar.0.clone())
            .is_some_and(|old| old != avatar.0);
        if changed {
            state.remote_players.remove(&id.0);
            commands.entity(entity).despawn();
        }
    }
}

fn accept_dash_ack(
    previous: &mut Option<(u64, u64, u64, u64)>,
    meta: SnapshotMeta,
    player: &PlayerState,
) -> bool {
    let next = (
        meta.server_epoch,
        meta.match_id,
        player.id,
        player.utility.dash_sequence,
    );
    let changed = player.utility.dash_sequence > 0 && *previous != Some(next);
    *previous = Some(next);
    changed
}

/// Minions render at this multiple of the shared normalized hero height.
const MINION_MODEL_HEIGHT_SCALE: f32 = 0.9;

fn network_projectile(state: &ProjectileState) -> NetworkProjectile {
    let direction = Vec3::from_array(state.direction);
    NetworkProjectile {
        id: state.id,
        owner_id: state.owner_id,
        owner_team: state.owner_team.into(),
        source_kind: state.source_kind,
        style: state.style,
        action_slot: state.action_slot,
        direction: if direction.is_finite() {
            direction.normalize_or_zero()
        } else {
            Vec3::ZERO
        },
    }
}

pub(in crate::net) fn mirror_debug_flags_to_network_state(
    speed_boost: Res<DebugSpeedBoost>,
    mut network_state: ResMut<NetworkState>,
) {
    if network_state.speed_boost_active != speed_boost.0 {
        network_state.speed_boost_active = speed_boost.0;
    }
}

fn choose_authoritative_local_player<T: Copy + Eq>(
    candidates: &[(T, Option<u64>)],
    your_id: u64,
) -> Option<T> {
    let mut chosen = None;
    for (entity, maybe_id) in candidates {
        if maybe_id.is_some_and(|id| id == your_id) {
            return Some(*entity);
        }
        chosen.get_or_insert(*entity);
    }
    chosen
}

/// The frame being applied this update: moved out of
/// [`PendingServerSnapshotFrame`] by `SnapshotApply::Begin`, read (and its
/// fields taken) by the later stages, cleared by `SnapshotApply::Finish`.
#[derive(Resource, Default)]
pub(in crate::net) struct StagedSnapshot {
    data: Option<PendingSnapshotData>,
    /// Which entity work still runs; `Finish` reports it as the outcome.
    gate: ApplyOutcome,
}

/// How far snapshot application got for one snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Every stage ran.
    #[default]
    Full,
    /// Prematch Draft: the heroes were despawned and no other world entity
    /// (projectile, structure, minion, neutral) was touched.
    Draft,
    /// The server lists the local hero, but no hero exists, no team is
    /// selected and no join is committed: the local spawn, remote players and
    /// world entities were skipped.
    LocalPending,
}

/// Written by `SnapshotApply::Finish` once per applied snapshot, after every
/// stage (and their Commands) ran. Session and world resources are already
/// updated when it is read.
#[derive(Message, Clone, Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no reader yet: the camera slice (15f) and QA frame loops are the first"
    )
)]
pub struct SnapshotApplied {
    pub meta: SnapshotMeta,
    pub your_id: u64,
    pub round: Option<RoundId>,
    pub outcome: ApplyOutcome,
}

/// Snapshot application as chained stages, each a member of
/// `ClientNetPipeline::ApplySnapshot` (so every `.after(ApplySnapshot)`
/// reader still sees the whole application), followed by the session-event
/// flush. Used by the plugin and by the test apps that apply snapshots.
pub(in crate::net) fn snapshot_apply_systems() -> ScheduleConfigs<ScheduleSystem> {
    (
        begin_snapshot_apply.in_set(SnapshotApply::Begin),
        apply_snapshot_session
            .run_if(snapshot_staged)
            .in_set(SnapshotApply::Session),
        apply_snapshot_resources
            .run_if(snapshot_staged)
            .in_set(SnapshotApply::Resources),
        apply_snapshot_entities
            .run_if(snapshot_staged)
            .in_set(SnapshotApply::Entities),
        finish_snapshot_apply
            .run_if(snapshot_staged)
            .in_set(SnapshotApply::Finish),
        flush_session_events,
    )
        .chain()
        .in_set(ClientNetPipeline::ApplySnapshot)
}

fn snapshot_staged(staged: Res<StagedSnapshot>) -> bool {
    staged.data.is_some()
}

fn begin_snapshot_apply(
    mut pending: ResMut<PendingServerSnapshotFrame>,
    mut staged: ResMut<StagedSnapshot>,
) {
    staged.data = pending.frame.take();
    staged.gate = ApplyOutcome::Full;
}

fn apply_snapshot_session(staged: Res<StagedSnapshot>, mut client_session: ResMut<ClientSession>) {
    let Some(data) = staged.data.as_ref() else {
        return;
    };
    if client_session.state != ClientConnectionState::Connected {
        client_session.state = ClientConnectionState::Connected;
        client_session.waiting_since = None;
        client_session.outbox.push(SessionEvent::Connected);
    }
    client_session.last_qualifying_snapshot_wall = Some(data.wall_time);
    if client_session.join_confirmed() && !client_session.announced_join {
        client_session.announced_join = true;
        client_session.outbox.push(SessionEvent::Joined {
            your_id: data.your_id,
        });
    }
}

fn apply_snapshot_resources(
    mut staged: ResMut<StagedSnapshot>,
    mut client_session: ResMut<ClientSession>,
    mut network_state: ResMut<NetworkState>,
    mut game_state_snapshot: ResMut<GameStateSnapshot>,
    mut team_selection: ResMut<TeamSelection>,
) {
    let StagedSnapshot { data, gate } = &mut *staged;
    let Some(data) = data.as_mut() else {
        return;
    };
    let your_id = data.your_id;
    let meta = data.meta;

    network_state.local_id = Some(your_id);
    game_state_snapshot.your_id = your_id;
    game_state_snapshot.prematch = data.prematch.take();
    game_state_snapshot.match_mode = std::mem::take(&mut data.match_mode);
    game_state_snapshot.geometry_id = std::mem::take(&mut data.geometry_id);
    game_state_snapshot.map_profile = std::mem::take(&mut data.map_profile);
    game_state_snapshot.meta = meta;
    game_state_snapshot.state = std::mem::take(&mut data.game_state);
    game_state_snapshot.rematch_in_secs = data.rematch_in_secs;
    game_state_snapshot.team_buffs = std::mem::take(&mut data.team_buffs);
    game_state_snapshot.combat_events = std::mem::take(&mut data.combat_events);
    game_state_snapshot.scoreboard = data.scoreboard.take();
    game_state_snapshot.sandbox = data.sandbox.take();
    game_state_snapshot.forest_pickups = std::mem::take(&mut data.forest_pickups);
    game_state_snapshot.vision = data.vision.take();

    // Zero ids have no round; a reconnect to the same round is no change.
    if let Some(current) = RoundId::from_meta(&meta) {
        if let Some(previous) = network_state.last_round
            && previous != current
        {
            client_session
                .outbox
                .push(SessionEvent::RoundChanged { previous, current });
        }
        network_state.last_round = Some(current);
    }

    // Reconnect uses the accepted draft loadout, never a stale pre-search choice.
    if let Some(own) = game_state_snapshot
        .prematch
        .as_ref()
        .and_then(|draft| draft.players.iter().find(|p| p.player_id == your_id))
    {
        team_selection.character = own.character;
        team_selection.hero_class = own.hero_class;
        team_selection.avatar = own.avatar.clone();
        if let Some(sprite) = &own.sprite_character {
            team_selection.sprite_character = sprite.clone();
        }
        if let Some(join) = client_session.last_join.as_mut() {
            join.character = own.character;
            join.hero_class = own.hero_class;
            join.avatar = own.avatar.clone();
            join.sprite_character = own.sprite_character.clone();
            join.team = own.team.into();
        }
    }
    // The Draft roster is not final: the entity stage only clears the heroes.
    if game_state_snapshot
        .prematch
        .as_ref()
        .is_some_and(|p| p.phase == shared::prematch::PrematchPhase::Draft)
    {
        *gate = ApplyOutcome::Draft;
    }
}

fn finish_snapshot_apply(
    mut staged: ResMut<StagedSnapshot>,
    mut applied: MessageWriter<SnapshotApplied>,
) {
    let Some(data) = staged.data.take() else {
        return;
    };
    let outcome = std::mem::take(&mut staged.gate);
    applied.write(SnapshotApplied {
        meta: data.meta,
        your_id: data.your_id,
        round: RoundId::from_meta(&data.meta),
        outcome,
    });
}

/// Grouped UI-side resources for [`apply_snapshot_entities`] (Bevy caps
/// system functions at 16 parameters).
#[derive(bevy::ecs::system::SystemParam)]
pub(in crate::net) struct SnapshotUiState<'w> {
    cam_state: ResMut<'w, CameraState>,
    team_selection: ResMut<'w, TeamSelection>,
    visual_mode: Res<'w, PlayerVisualMode>,
}

/// Local hero, remote players, projectiles, structures, minions and neutrals
/// (roadmap step 15b2 splits it further).
fn apply_snapshot_entities(
    mut commands: Commands,
    mut staged: ResMut<StagedSnapshot>,
    client_session: Res<ClientSession>,
    mut network_state: ResMut<NetworkState>,
    mut transform_sets: ParamSet<(
        Query<&mut Transform>,
        Query<&mut Transform, With<MainCamera>>,
    )>,
    mut remote_query: Query<
        (&mut RemotePlayerInterpolation, Option<&PlayerUtility>),
        With<RemotePlayer>,
    >,
    projectile_query: Query<&NetworkProjectile>,
    structure_query: Query<&NetworkStructure>,
    minion_query: Query<&NetworkMinion>,
    neutral_query: Query<&NetworkNeutral>,
    local_player_query: Query<(Entity, Option<&NetworkPlayerId>), With<Player>>,
    action_query: Query<Option<&PlayerCosmeticAction>>,
    player_assets: Res<PlayerAssets>,
    mut models: PlayerModelResolver,
    mut ui_state: SnapshotUiState,
    mut utility_vfx: MessageWriter<crate::game_vfx::UtilityVfx>,
) {
    let SnapshotUiState {
        cam_state,
        team_selection,
        visual_mode,
    } = &mut ui_state;
    let StagedSnapshot { data, gate } = &mut *staged;
    let Some(data) = data.as_mut() else {
        return;
    };
    let meta = data.meta;
    let snapshot_wall_time = data.wall_time;
    let your_id = data.your_id;
    let selected_team_for_spawn = data.selected_team_for_spawn;
    let players = std::mem::take(&mut data.players);
    let projectiles = std::mem::take(&mut data.projectiles);
    let structures = std::mem::take(&mut data.structures);
    let minions = std::mem::take(&mut data.minions);
    let neutrals = std::mem::take(&mut data.neutrals);

    // Spawn the final roster only once frozen; redraft discards old models.
    if *gate == ApplyOutcome::Draft {
        for (entity, _) in &local_player_query {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
        for (_, entity) in network_state.remote_players.drain() {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }
        return;
    }
    let local_player_state = players.iter().find(|player| player.id == your_id);
    let local_players = local_player_query
        .iter()
        .map(|(entity, maybe_id)| (entity, maybe_id.map(|id| id.0)))
        .collect::<Vec<_>>();
    // IMPORTANT: we must tolerate temporary duplication of `Player` entities (e.g. during loading /
    // restart races). Many gameplay systems use `Query::single()` and will break if we allow >1.
    let chosen_local = choose_authoritative_local_player(&local_players, your_id);

    if let Some(local_entity) = chosen_local {
        // Keep exactly one local `Player` alive to avoid `single()` query failures.
        for &(entity, maybe_id) in &local_players {
            if entity == local_entity {
                continue;
            }
            // If this extra player happens to have our id, prefer the chosen one anyway and despawn.
            if maybe_id.is_some_and(|id| id == your_id) {
                warn!("Found duplicate local Player for id={your_id}; despawning extra entity");
            }
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
        }

        // Always ensure the local player is tagged with the server-provided id.
        commands
            .entity(local_entity)
            .insert(NetworkPlayerId(your_id));

        // Only apply character/team/stats once we actually have a state entry for our id.
        // Otherwise we'd oscillate between the locally selected character and the server default.
        if let Some(local_player_state) = local_player_state {
            commands.entity(local_entity).insert((
                Team::from(local_player_state.team),
                player_state_to_combat_stats(local_player_state),
                NetworkCharacterChoice(local_player_state.character),
                NetworkAvatar(local_player_state.avatar.clone()),
                NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                NetworkHeroClass(local_player_state.hero_class),
                crate::supporter::NetworkSupporterAura(local_player_state.supporter_aura),
                player_state_to_progression(local_player_state),
                player_state_to_equipment(local_player_state),
                (
                    PlayerBasicAttackCooldown::from(local_player_state),
                    PlayerSkillCooldowns::from(local_player_state),
                    PlayerUtility::from(local_player_state),
                ),
            ));
            let next_action = PlayerCosmeticAction::from(local_player_state);
            if action_query.get(local_entity).ok().flatten().copied() != Some(next_action) {
                commands.entity(local_entity).insert(next_action);
            }
            network_state.local_team = Some(local_player_state.team.into());

            let server_translation = Vec3::new(
                local_player_state.x,
                local_player_state.y,
                local_player_state.z,
            );
            let dash_accepted =
                accept_dash_ack(&mut network_state.local_dash_ack, meta, local_player_state);
            if dash_accepted {
                commands
                    .entity(local_entity)
                    .remove::<(crate::player::MovementTarget, crate::player::MovementRoute)>();
            }
            if let Ok(mut local_transform) = transform_sets.p0().get_mut(local_entity) {
                // Snap on meaningful server corrections (first team spawn, respawn, etc.).
                // While speed-boosting, the local player legitimately leads the last
                // server-acked position further, so widen the threshold to avoid
                // rubber-banding the boosted movement.
                let snap_distance = if network_state.speed_boost_active {
                    LOCAL_SNAP_DISTANCE * DEBUG_SPEED_MULTIPLIER
                } else {
                    LOCAL_SNAP_DISTANCE
                };
                if dash_accepted {
                    utility_vfx.write(crate::game_vfx::UtilityVfx::Dash {
                        from: local_transform.translation,
                        to: server_translation,
                        seed: local_player_state.utility.dash_sequence,
                    });
                }
                if dash_accepted
                    || local_transform
                        .translation
                        .distance_squared(server_translation)
                        > snap_distance * snap_distance
                {
                    local_transform.translation = server_translation;
                    local_transform.rotation = Quat::from_rotation_y(local_player_state.yaw);
                }
            }
        }
    } else if let Some(local_player_state) = local_player_state {
        // Spawn only after the local join was committed (a team was picked).
        // Snapshots list joined players only, so our presence in the list is
        // the server's join ack. The server may have assigned a different
        // team than requested (release-mode balancing) - adopt it as truth.
        if selected_team_for_spawn.is_none() && !client_session.has_committed_join() {
            *gate = ApplyOutcome::LocalPending;
            return;
        }
        let assigned_team = Team::from(local_player_state.team);
        if team_selection.team != Some(assigned_team) {
            info!(
                "Server assigned team {} (matchmaking)",
                assigned_team.as_str()
            );
            team_selection.team = Some(assigned_team);
        }
        let spawn = Vec3::new(
            local_player_state.x,
            local_player_state.y,
            local_player_state.z,
        );
        let (local_scene, local_gltf) = if **visual_mode == PlayerVisualMode::Models3d {
            models.resolve(
                local_player_state.character,
                local_player_state.avatar.as_deref(),
            )
        } else {
            (None, None)
        };
        let entity = if **visual_mode == PlayerVisualMode::Sprite2d {
            commands
                .spawn((
                    Transform::from_translation(spawn),
                    Visibility::default(),
                    Player,
                    PlayerBody,
                    VerticalVelocity::default(),
                    Team::from(local_player_state.team),
                    (
                        NetworkPlayerId(your_id),
                        NetworkCharacterChoice(local_player_state.character),
                        NetworkAvatar(local_player_state.avatar.clone()),
                        NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                        PlayerCosmeticAction::from(local_player_state),
                        NetworkHeroClass(local_player_state.hero_class),
                        crate::supporter::NetworkSupporterAura(local_player_state.supporter_aura),
                    ),
                    player_state_to_combat_stats(local_player_state),
                    player_state_to_progression(local_player_state),
                    player_state_to_equipment(local_player_state),
                    (
                        PlayerBasicAttackCooldown::from(local_player_state),
                        PlayerSkillCooldowns::from(local_player_state),
                        PlayerUtility::from(local_player_state),
                    ),
                    Name::new("Player"),
                ))
                .id()
        } else if let Some(scene_handle) = local_scene {
            let mut entity_commands = commands.spawn((
                SceneRoot(scene_handle),
                Transform {
                    translation: spawn,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::splat(1.0),
                },
                GlobalTransform::default(),
                Visibility::default(),
                Player,
                PlayerBody,
                VerticalVelocity::default(),
                Team::from(local_player_state.team),
                NormalizeModelScale::for_player_model(),
                (
                    NetworkPlayerId(your_id),
                    NetworkCharacterChoice(local_player_state.character),
                    NetworkAvatar(local_player_state.avatar.clone()),
                    NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                    PlayerCosmeticAction::from(local_player_state),
                    NetworkHeroClass(local_player_state.hero_class),
                    crate::supporter::NetworkSupporterAura(local_player_state.supporter_aura),
                ),
                player_state_to_combat_stats(local_player_state),
                player_state_to_progression(local_player_state),
                player_state_to_equipment(local_player_state),
                (
                    PlayerBasicAttackCooldown::from(local_player_state),
                    PlayerSkillCooldowns::from(local_player_state),
                    PlayerUtility::from(local_player_state),
                ),
                Name::new("Player"),
            ));
            if let Some(gltf) = local_gltf {
                entity_commands.insert(ModelScaleSource {
                    gltf,
                    key: model_scale_key(
                        local_player_state.character,
                        local_player_state.avatar.as_deref(),
                    ),
                });
            }
            entity_commands.id()
        } else {
            commands
                .spawn((
                    Mesh3d(player_assets.mesh.clone()),
                    MeshMaterial3d(player_assets.material.clone()),
                    Transform::from_translation(spawn),
                    Player,
                    PlayerBody,
                    VerticalVelocity::default(),
                    Team::from(local_player_state.team),
                    (
                        NetworkPlayerId(your_id),
                        NetworkCharacterChoice(local_player_state.character),
                        NetworkAvatar(local_player_state.avatar.clone()),
                        NetworkSpriteCharacter(local_player_state.sprite_character.clone()),
                        PlayerCosmeticAction::from(local_player_state),
                        NetworkHeroClass(local_player_state.hero_class),
                        crate::supporter::NetworkSupporterAura(local_player_state.supporter_aura),
                    ),
                    player_state_to_combat_stats(local_player_state),
                    player_state_to_progression(local_player_state),
                    player_state_to_equipment(local_player_state),
                    (
                        PlayerBasicAttackCooldown::from(local_player_state),
                        PlayerSkillCooldowns::from(local_player_state),
                        PlayerUtility::from(local_player_state),
                    ),
                    Name::new("Player"),
                ))
                .id()
        };

        network_state.local_team = Some(local_player_state.team.into());
        accept_dash_ack(&mut network_state.local_dash_ack, meta, local_player_state);
        if let Ok(mut camera_transform) = transform_sets.p1().single_mut() {
            cam_state.locked = true;
            if **visual_mode == PlayerVisualMode::Sprite2d {
                let xy = crate::world2d::simulation_xz_to_render_xy(spawn);
                camera_transform.translation.x = xy.x;
                camera_transform.translation.y = xy.y;
            } else {
                let zoom = cam_state.zoom;
                camera_transform.translation =
                    spawn + locked_camera_offset_for_team(zoom, local_player_state.team.into());
                let look_target = Vec3::new(spawn.x, PLAYER_SIZE * 0.5, spawn.z);
                *camera_transform = camera_transform.looking_at(look_target, Vec3::Y);
            }
        }

        commands.entity(entity);
    }

    let mut seen_remote_ids = HashSet::new();

    for player in &players {
        if player.id == your_id {
            continue;
        }
        seen_remote_ids.insert(player.id);

        if let Some(entity) = network_state.remote_players.get(&player.id).copied() {
            if let Ok((mut interpolation, utility)) = remote_query.get_mut(entity) {
                let translation = Vec3::new(player.x, player.y, player.z);
                let rotation = Quat::from_rotation_y(player.yaw);
                let dashed =
                    utility.is_some_and(|u| player.utility.dash_sequence > u.state.dash_sequence);
                if dashed {
                    if let Some(from) = interpolation.latest_translation() {
                        utility_vfx.write(crate::game_vfx::UtilityVfx::Dash {
                            from,
                            to: translation,
                            seed: player.id << 16 | player.utility.dash_sequence,
                        });
                    }
                    interpolation.teleport(translation, rotation, snapshot_wall_time);
                } else {
                    interpolation.push(translation, rotation, snapshot_wall_time);
                }
            }
            commands.entity(entity).insert((
                NetworkPlayerId(player.id),
                NetworkBot(player.is_bot),
                Team::from(player.team),
                NetworkCharacterChoice(player.character),
                NetworkAvatar(player.avatar.clone()),
                NetworkSpriteCharacter(player.sprite_character.clone()),
                NetworkHeroClass(player.hero_class),
                crate::supporter::NetworkSupporterAura(player.supporter_aura),
                player_state_to_combat_stats(player),
                player_state_to_progression(player),
                player_state_to_equipment(player),
                (
                    PlayerBasicAttackCooldown::from(player),
                    PlayerSkillCooldowns::from(player),
                    PlayerUtility::from(player),
                ),
            ));
            let next_action = PlayerCosmeticAction::from(player);
            if action_query.get(entity).ok().flatten().copied() != Some(next_action) {
                commands.entity(entity).insert(next_action);
            }
            continue;
        }

        let (scene_handle, gltf_handle) = if **visual_mode == PlayerVisualMode::Models3d {
            models.resolve(player.character, player.avatar.as_deref())
        } else {
            (None, None)
        };
        let mesh_handle = player_assets.mesh.clone();
        let material_handle = player_assets.material.clone();
        let spawn_translation = Vec3::new(player.x, player.y, player.z);
        let spawn_rotation = Quat::from_rotation_y(player.yaw);
        let mut entity_commands = commands.spawn((
            Transform::from_translation(spawn_translation).with_rotation(spawn_rotation),
            Visibility::default(),
            RemotePlayer,
            PlayerBody,
            Team::from(player.team),
            NetworkPlayerId(player.id),
            NetworkCharacterChoice(player.character),
            NetworkAvatar(player.avatar.clone()),
            NetworkSpriteCharacter(player.sprite_character.clone()),
            PlayerCosmeticAction::from(player),
            NetworkHeroClass(player.hero_class),
            player_state_to_combat_stats(player),
            player_state_to_progression(player),
            RemotePlayerInterpolation::new(spawn_translation, spawn_rotation, snapshot_wall_time),
            Name::new(format!("RemotePlayer-{}", player.id)),
        ));
        entity_commands.insert((
            NetworkBot(player.is_bot),
            crate::supporter::NetworkSupporterAura(player.supporter_aura),
            player_state_to_equipment(player),
            (
                PlayerBasicAttackCooldown::from(player),
                PlayerSkillCooldowns::from(player),
                PlayerUtility::from(player),
            ),
        ));
        if **visual_mode == PlayerVisualMode::Models3d {
            entity_commands.insert(NormalizeModelScale::for_player_model());
            if let Some(gltf) = gltf_handle {
                entity_commands.insert(ModelScaleSource {
                    gltf,
                    key: model_scale_key(player.character, player.avatar.as_deref()),
                });
            }
            entity_commands.with_children(|parent| {
                if let Some(scene_handle) = scene_handle {
                    parent.spawn((
                        SceneRoot(scene_handle),
                        Transform::default(),
                        Visibility::default(),
                    ));
                } else {
                    parent.spawn((
                        Mesh3d(mesh_handle),
                        MeshMaterial3d(material_handle),
                        Transform::default(),
                        Visibility::default(),
                    ));
                }
            });
        }
        let entity = entity_commands.id();

        network_state.remote_players.insert(player.id, entity);
    }

    let stale_ids = network_state
        .remote_players
        .keys()
        .copied()
        .filter(|id| !seen_remote_ids.contains(id))
        .collect::<Vec<_>>();

    for player_id in stale_ids {
        if let Some(entity) = network_state.remote_players.remove(&player_id) {
            if remote_query.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }

    let mut seen_projectile_ids = HashSet::new();
    for projectile in &projectiles {
        seen_projectile_ids.insert(projectile.id);

        if let Some(entity) = network_state.projectiles.get(&projectile.id).copied() {
            if let Ok(mut transform) = transform_sets.p0().get_mut(entity) {
                transform.translation = Vec3::new(projectile.x, projectile.y, projectile.z);
            }
            commands
                .entity(entity)
                .insert(network_projectile(projectile));
            continue;
        }

        let entity = commands
            .spawn((
                Transform::from_xyz(projectile.x, projectile.y, projectile.z),
                Visibility::default(),
                network_projectile(projectile),
                Name::new(format!("Projectile-{}", projectile.id)),
            ))
            .id();
        network_state.projectiles.insert(projectile.id, entity);
    }

    let stale_projectile_ids = network_state
        .projectiles
        .keys()
        .copied()
        .filter(|id| !seen_projectile_ids.contains(id))
        .collect::<Vec<_>>();
    for projectile_id in stale_projectile_ids {
        if let Some(entity) = network_state.projectiles.remove(&projectile_id) {
            if projectile_query.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }

    let mut seen_structure_ids = HashSet::new();
    for structure in &structures {
        seen_structure_ids.insert(structure.id);

        if let Some(entity) = network_state.structures.get(&structure.id).copied() {
            if let Ok(mut transform) = transform_sets.p0().get_mut(entity) {
                transform.translation = Vec3::new(structure.x, structure.y, structure.z);
            }
            commands.entity(entity).insert((
                StructureKind::from(structure.kind),
                Team::from(structure.team),
                NetworkStructureId(structure.id),
                NetworkStructureProtected(structure.protected),
                NetworkMapStructure::from(structure),
                structure_state_to_combat_stats(structure),
            ));
            continue;
        }

        let entity_commands = commands.spawn((
            Transform::from_xyz(structure.x, structure.y, structure.z),
            Visibility::default(),
            NetworkStructure,
            NetworkStructureId(structure.id),
            NetworkStructureProtected(structure.protected),
            NetworkMapStructure::from(structure),
            StructureKind::from(structure.kind),
            Team::from(structure.team),
            structure_state_to_combat_stats(structure),
            Name::new(format!("Structure-{}", structure.id)),
        ));
        let entity = entity_commands.id();

        network_state.structures.insert(structure.id, entity);
    }

    let stale_structure_ids = network_state
        .structures
        .keys()
        .copied()
        .filter(|id| !seen_structure_ids.contains(id))
        .collect::<Vec<_>>();
    for structure_id in stale_structure_ids {
        if let Some(entity) = network_state.structures.remove(&structure_id) {
            if structure_query.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }

    let mut seen_minion_ids = HashSet::new();
    for minion in &minions {
        seen_minion_ids.insert(minion.id);
        let target_translation = Vec3::new(minion.x, minion.y, minion.z);
        let target_rotation = Quat::from_rotation_y(minion.yaw);

        if let Some(entity) = network_state.minions.get(&minion.id).copied() {
            if let Ok(transform) = transform_sets.p0().get_mut(entity) {
                let interpolation = NetEntityInterpolation {
                    from_translation: transform.translation,
                    to_translation: target_translation,
                    from_rotation: transform.rotation,
                    to_rotation: target_rotation,
                    elapsed: 0.0,
                    duration: UPDATE_INTERVAL_SECONDS.max(0.001),
                };
                commands.entity(entity).insert(interpolation);
            }
            commands.entity(entity).insert((
                NetworkMinionId(minion.id),
                Team::from(minion.team),
                minion_state_to_combat_stats(minion),
                NetworkMinionBrainState(minion.state),
                NetworkMinionKind(minion.kind),
                NetworkMinionAction(minion.attack_sequence),
            ));
            continue;
        }

        let mut entity_commands = commands.spawn((
            Transform::from_translation(target_translation).with_rotation(target_rotation),
            Visibility::default(),
            NetworkMinion,
            NetworkMinionId(minion.id),
            NetworkMinionBrainState(minion.state),
            NetworkMinionKind(minion.kind),
            NetworkMinionAction(minion.attack_sequence),
            NetEntityInterpolation {
                from_translation: target_translation,
                to_translation: target_translation,
                from_rotation: target_rotation,
                to_rotation: target_rotation,
                elapsed: UPDATE_INTERVAL_SECONDS,
                duration: UPDATE_INTERVAL_SECONDS.max(0.001),
            },
            Team::from(minion.team),
            minion_state_to_combat_stats(minion),
            Name::new(format!("Minion-{}-{:?}", minion.id, minion.lane)),
        ));
        if **visual_mode == PlayerVisualMode::Models3d {
            // Original procedural meshes attach once in MinionVisualsPlugin.
            entity_commands.insert(NormalizeModelScale::scaled_by(MINION_MODEL_HEIGHT_SCALE));
        }
        let entity = entity_commands.id();
        network_state.minions.insert(minion.id, entity);
    }

    let mut seen_neutral_ids = HashSet::new();
    for neutral in &neutrals {
        seen_neutral_ids.insert(neutral.id);
        let target_translation = Vec3::new(neutral.x, neutral.y, neutral.z);
        let target_rotation = Quat::from_rotation_y(neutral.yaw);

        if let Some(entity) = network_state.neutrals.get(&neutral.id).copied() {
            if let Ok(transform) = transform_sets.p0().get_mut(entity) {
                let interpolation = NetEntityInterpolation {
                    from_translation: transform.translation,
                    to_translation: target_translation,
                    from_rotation: transform.rotation,
                    to_rotation: target_rotation,
                    elapsed: 0.0,
                    duration: UPDATE_INTERVAL_SECONDS.max(0.001),
                };
                commands.entity(entity).insert(interpolation);
            }
            commands.entity(entity).insert((
                NetworkNeutralId(neutral.id),
                NetworkNeutralCampType(neutral.camp_type),
                neutral_state_to_combat_stats(neutral),
                NeutralAiStateTag(neutral.ai_state),
            ));
            continue;
        }

        let base_components = (
            Transform::from_translation(target_translation).with_rotation(target_rotation),
            Visibility::default(),
            NetworkNeutral,
            NetworkNeutralId(neutral.id),
            NetworkNeutralCampType(neutral.camp_type),
            NetEntityInterpolation {
                from_translation: target_translation,
                to_translation: target_translation,
                from_rotation: target_rotation,
                to_rotation: target_rotation,
                elapsed: UPDATE_INTERVAL_SECONDS,
                duration: UPDATE_INTERVAL_SECONDS.max(0.001),
            },
            neutral_state_to_combat_stats(neutral),
            NeutralAiStateTag(neutral.ai_state),
        );

        // Bosses and ordinary camps attach their role-specific visuals independently.
        let entity = if neutral.camp_type.is_boss() {
            commands
                .spawn((
                    base_components,
                    BossVisual {
                        camp_type: neutral.camp_type,
                    },
                    Name::new(format!(
                        "Boss-{}-{}",
                        neutral.id,
                        crate::bosses::boss_display_name(neutral.camp_type)
                    )),
                ))
                .id()
        } else {
            commands
                .spawn((
                    base_components,
                    Name::new(format!("Neutral-{}", neutral.id)),
                ))
                .id()
        };
        network_state.neutrals.insert(neutral.id, entity);
    }

    let stale_neutral_ids = network_state
        .neutrals
        .keys()
        .copied()
        .filter(|id| !seen_neutral_ids.contains(id))
        .collect::<Vec<_>>();
    for neutral_id in stale_neutral_ids {
        if let Some(entity) = network_state.neutrals.remove(&neutral_id) {
            if neutral_query.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }

    let stale_minion_ids = network_state
        .minions
        .keys()
        .copied()
        .filter(|id| !seen_minion_ids.contains(id))
        .collect::<Vec<_>>();
    for minion_id in stale_minion_ids {
        if let Some(entity) = network_state.minions.remove(&minion_id) {
            if minion_query.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }
}

fn player_state_to_combat_stats(player: &PlayerState) -> CombatStats {
    CombatStats {
        hp: player.hp,
        max_hp: player.max_hp.max(1.0),
        mana: player.mana,
        max_mana: player.max_mana.max(1.0),
    }
}

fn player_state_to_equipment(player: &PlayerState) -> PlayerEquipment {
    PlayerEquipment {
        gold: player.gold,
        inventory: player.inventory.clone(),
        item_bonuses: player.item_bonuses,
        shop_available: player.shop_available,
        last_purchase: player.last_purchase.clone(),
    }
}

fn player_state_to_progression(player: &PlayerState) -> PlayerProgression {
    PlayerProgression {
        sandbox_unlocked: None,
        level: player.level.max(1),
        xp: player.xp,
        next_level_xp: player.next_level_xp,
        skill_points: player.skill_points,
        ranks: player.ranks,
    }
}

fn structure_state_to_combat_stats(structure: &StructureState) -> CombatStats {
    CombatStats {
        hp: structure.hp,
        max_hp: structure.max_hp.max(1.0),
        mana: 0.0,
        max_mana: 1.0,
    }
}

fn minion_state_to_combat_stats(minion: &MinionState) -> CombatStats {
    CombatStats {
        hp: minion.hp,
        max_hp: minion.max_hp.max(1.0),
        mana: 0.0,
        max_mana: 1.0,
    }
}

fn neutral_state_to_combat_stats(neutral: &NeutralState) -> CombatStats {
    CombatStats {
        hp: neutral.hp,
        max_hp: neutral.max_hp.max(1.0),
        mana: 0.0,
        max_mana: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::session::{TeardownQueries, TeardownReason, perform_network_teardown};
    use crate::net::test_fixtures::*;
    use crate::net::transport::forward_complete_server_datagram;
    use serde_json::json;

    #[test]
    fn utility_snapshot_defaults_and_dash_ack_force_even_short_reconciliation_once() {
        let mut player: PlayerState =
            serde_json::from_value(json!({"id":1,"x":0.0,"y":0.5,"z":0.0,"yaw":0.0})).unwrap();
        assert_eq!(PlayerUtility::from(&player), PlayerUtility::default());
        let meta = SnapshotMeta::new(4, 8, 1);
        let mut previous = None;
        assert!(!accept_dash_ack(&mut previous, meta, &player));
        player.x = 0.25; // well below the normal 4-unit reconciliation threshold
        player.utility.dash_sequence = 1;
        assert!(accept_dash_ack(&mut previous, meta, &player));
        assert!(!accept_dash_ack(&mut previous, meta, &player));
        assert!(accept_dash_ack(
            &mut previous,
            SnapshotMeta::new(4, 9, 1),
            &player
        ));
        player.utility.dash_sequence = 0;
        assert!(!accept_dash_ack(
            &mut previous,
            SnapshotMeta::new(4, 10, 1),
            &player
        ));
        player.utility.haste_active_secs = shared::utility::HASTE_DURATION_SECS;
        assert_eq!(
            PlayerUtility::from(&player).state.movement_multiplier(),
            1.4
        );
    }

    #[test]
    fn team_vision_omission_despawns_visual_tree_and_reacquires_once() {
        let (mut app, incoming) = snapshot_app();
        incoming.send(team_vision_snapshot(1, true, false)).unwrap();
        app.update();
        let local = network_hero_entity(&mut app, 1);
        let ally = network_hero_entity(&mut app, 3);
        let mut previous_enemy = network_hero_entity(&mut app, 2);
        for hidden_tick in [2, 4, 6] {
            let attached_visual = app
                .world_mut()
                .spawn((
                    Name::new("test attached actor visual"),
                    Mesh3d::default(),
                    Transform::default(),
                ))
                .id();
            let attached_trail = app.world_mut().spawn(Name::new("test nested trail")).id();
            app.world_mut()
                .entity_mut(attached_visual)
                .add_child(attached_trail);
            app.world_mut()
                .entity_mut(previous_enemy)
                .add_child(attached_visual);
            incoming
                .send(team_vision_snapshot(hidden_tick, false, false))
                .unwrap();
            app.update();
            for gone in [previous_enemy, attached_visual, attached_trail] {
                assert!(
                    app.world().get_entity(gone).is_err(),
                    "hidden actor tree remains: {gone:?}"
                );
            }
            assert!(
                !app.world()
                    .resource::<NetworkState>()
                    .remote_players
                    .contains_key(&2)
            );
            assert_eq!(
                network_hero_entity(&mut app, 1),
                local,
                "local actor must survive enemy omission"
            );
            assert_eq!(
                network_hero_entity(&mut app, 3),
                ally,
                "shared-sight ally must survive enemy omission"
            );
            incoming
                .send(team_vision_snapshot(hidden_tick + 1, true, false))
                .unwrap();
            app.update();
            let reacquired = network_hero_entity(&mut app, 2);
            assert_ne!(
                reacquired, previous_enemy,
                "reacquisition must not revive a stale entity handle"
            );
            let enemies = app
                .world_mut()
                .query::<&NetworkPlayerId>()
                .iter(app.world())
                .filter(|id| id.0 == 2)
                .count();
            assert_eq!(enemies, 1, "repeated reacquisition duplicated the actor");
            assert_eq!(
                app.world().resource::<NetworkState>().remote_players[&2],
                reacquired
            );
            previous_enemy = reacquired;
        }
    }

    #[test]
    fn team_vision_wire_updates_preserve_living_ally_after_local_death() {
        let (mut app, incoming) = snapshot_app();
        incoming.send(team_vision_snapshot(1, true, false)).unwrap();
        app.update();
        let local = network_hero_entity(&mut app, 1);
        let ally = network_hero_entity(&mut app, 3);
        for tick in 2..=3 {
            incoming
                .send(team_vision_snapshot(tick, false, true))
                .unwrap();
            app.update();
            assert_eq!(network_hero_entity(&mut app, 1), local);
            assert_eq!(network_hero_entity(&mut app, 3), ally);
            assert_eq!(
                app.world()
                    .get::<crate::combat::CombatStats>(local)
                    .unwrap()
                    .hp,
                0.0
            );
            assert!(
                app.world()
                    .get::<crate::combat::CombatStats>(ally)
                    .unwrap()
                    .hp
                    > 0.0
            );
            let vision = app
                .world()
                .resource::<GameStateSnapshot>()
                .vision
                .as_ref()
                .unwrap();
            assert_eq!(
                vision.sources.len(),
                1,
                "client must preserve server-authorized shared sources after local death"
            );
            assert_eq!(vision.sources[0].position, [tick as f32, -6.0]);
            assert_eq!(
                vision.local_brush,
                if tick % 2 == 0 { Some(1) } else { None }
            );
            assert_eq!(vision.local_hidden, tick % 2 == 0);
        }
    }

    #[test]
    fn team_vision_snapshot_none_and_session_teardown_clear_authoritative_state() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut app, incoming) = snapshot_app();
        incoming.send(team_vision_snapshot(1, true, false)).unwrap();
        app.update();
        assert!(app.world().resource::<GameStateSnapshot>().vision.is_some());
        let mut omitted = serde_json::to_value(team_vision_snapshot(2, false, false)).unwrap();
        omitted.as_object_mut().unwrap().remove("vision");
        incoming
            .send(serde_json::from_value(omitted).unwrap())
            .unwrap();
        app.update();
        assert!(
            app.world().resource::<GameStateSnapshot>().vision.is_none(),
            "legacy/no-vision packet must not retain stale sources"
        );
        incoming.send(team_vision_snapshot(3, true, false)).unwrap();
        app.update();
        let local = network_hero_entity(&mut app, 1);
        let enemy = network_hero_entity(&mut app, 2);
        let attached = app.world_mut().spawn(Name::new("session visual")).id();
        app.world_mut().entity_mut(enemy).add_child(attached);
        app.world_mut()
            .run_system_once(
                |mut commands: Commands,
                 mut session: ResMut<ClientSession>,
                 mut network: ResMut<NetworkState>,
                 mut snapshot: ResMut<GameStateSnapshot>,
                 mut team: ResMut<TeamSelection>,
                 mut camera: ResMut<crate::camera::CameraState>,
                 queries: TeardownQueries| {
                    perform_network_teardown(
                        TeardownReason::TransportFailure,
                        &mut commands,
                        &mut session,
                        &mut network,
                        &mut snapshot,
                        &mut team,
                        &mut camera,
                        &queries.remote_query,
                        &queries.projectile_query,
                        &queries.structure_query,
                        &queries.minion_query,
                        &queries.neutral_query,
                        &queries.player_query,
                    );
                },
            )
            .unwrap();
        assert!(app.world().resource::<GameStateSnapshot>().vision.is_none());
        assert!(
            app.world()
                .resource::<NetworkState>()
                .remote_players
                .is_empty()
        );
        assert_eq!(
            app.world().resource::<ClientSession>().state,
            ClientConnectionState::Disconnected
        );
        for gone in [local, enemy, attached] {
            assert!(
                app.world().get_entity(gone).is_err(),
                "teardown left actor or attached visual {gone:?}"
            );
        }
    }

    #[test]
    fn wire_scoreboard_updates_reach_the_hud_resource_and_local_identity() {
        let (mut app, incoming) = snapshot_app();
        for tick in 1..=2 {
            let mut value = serde_json::to_value(admission_snapshot(1, tick, true, None)).unwrap();
            value["scoreboard"] = json!({"players":[{"player_id":1,"nickname":"Player","team":"green","hero_class":"mage",
                "kills":tick-1,"deaths":0,"assists":tick,"earned_gold":15,"level":1,"connected":true}]});
            forward_complete_server_datagram(&serde_json::to_vec(&value).unwrap(), &incoming)
                .unwrap();
            app.update();
            let game = app.world().resource::<GameStateSnapshot>();
            let own = &game.scoreboard.as_ref().unwrap().players[0];
            assert_eq!(
                (own.kills, own.deaths, own.assists),
                ((tick - 1) as u32, 0, tick as u32)
            );
            assert_eq!(own.player_id, game.your_id);
        }
    }

    #[test]
    fn admitted_snapshot_and_world_fallback_keep_one_local_root_in_the_same_frame() {
        use crate::player::Player;
        let (mut app, incoming) = snapshot_app();
        for tick in 1..=3 {
            let mut snapshot =
                serde_json::to_value(admission_snapshot(1, tick, true, None)).unwrap();
            let mut hero = snapshot["players"][0].clone();
            hero["avatar"] = serde_json::Value::Null;
            snapshot["players"] = json!([hero]);
            for field in ["structures", "minions", "neutrals", "projectiles"] {
                snapshot[field] = json!([]);
            }
            incoming
                .send(serde_json::from_value(snapshot).unwrap())
                .unwrap();
            app.update();
            assert!(app.world().resource::<ClientSession>().join_confirmed());
            let mut players = app
                .world_mut()
                .query_filtered::<Option<&NetworkPlayerId>, With<Player>>();
            let ids: Vec<_> = players
                .iter(app.world())
                .map(|id| id.map(|id| id.0))
                .collect();
            assert_eq!(
                ids,
                vec![Some(1)],
                "the admitted snapshot must be the sole creator; no transient untagged world duplicate"
            );
        }
    }

    #[test]
    fn configured_structure_identity_reconciles_add_move_hp_profile_and_remove() {
        use crate::combat::CombatStats;
        let (mut app, incoming) = snapshot_app();
        let mut first_root = None;
        for tick in 1..=4 {
            let mut snapshot =
                serde_json::to_value(admission_snapshot(1, tick, true, None)).unwrap();
            let count = if tick == 4 { 0 } else { tick as usize };
            snapshot["geometry_id"] = json!(shared::map::GEOMETRY_ID);
            snapshot["map_profile"] = json!("custom-siege");
            snapshot["structures"] = json!(
                (0..count)
                    .map(|index| json!({
                        "id": 101 + index, "map_key": format!("mid-{}", index),
                        "visual_profile": if tick == 1 { "tower_blue" } else { "tower_alt" },
                        "kind": "tower", "team": "blue", "x": tick as f32 * 5.0,
                        "y": 3.0, "z": index as f32 * 10.0, "hp": 400.0 - tick as f32,
                        "max_hp": 400.0, "protected": index > 0
                    }))
                    .collect::<Vec<_>>()
            );
            for field in ["players", "minions", "neutrals", "projectiles"] {
                snapshot[field] = json!([]);
            }
            incoming
                .send(serde_json::from_value(snapshot).unwrap())
                .unwrap();
            app.update();
            assert_eq!(
                app.world().resource::<GameStateSnapshot>().map_profile,
                "custom-siege"
            );
            let roots = app.world().resource::<NetworkState>().structures.clone();
            assert_eq!(roots.len(), count);
            if count == 0 {
                assert!(app.world().get_entity(first_root.unwrap()).is_err());
                continue;
            }
            let entity = roots[&101];
            if let Some(previous) = first_root {
                assert_eq!(entity, previous);
            }
            first_root = Some(entity);
            assert_eq!(
                app.world().get::<NetworkStructureId>(entity).unwrap().0,
                101
            );
            let identity = app.world().get::<NetworkMapStructure>(entity).unwrap();
            assert_eq!(identity.key, "mid-0");
            assert_eq!(
                identity.visual_profile,
                if tick == 1 { "tower_blue" } else { "tower_alt" }
            );
            assert_eq!(
                app.world().get::<Transform>(entity).unwrap().translation.x,
                tick as f32 * 5.0
            );
            let stats = app.world().get::<CombatStats>(entity).unwrap();
            assert_eq!(stats.hp, 400.0 - tick as f32);
            assert_eq!(stats.max_hp, 400.0);
        }
    }

    // Early-return gates of the entity stage (roadmap step 15b1, hazard 2).

    #[test]
    fn draft_snapshot_clears_heroes_and_leaves_world_entities_untouched() {
        use crate::combat::CombatStats;
        use crate::net::session::CommittedJoin;
        let (mut app, incoming) = snapshot_app();
        // A prematch join: the world fallback leaves the hero to the draft.
        app.world_mut().resource_mut::<ClientSession>().last_join = Some(CommittedJoin {
            prematch: true,
            ..CommittedJoin::for_test()
        });
        incoming.send(admission_snapshot(1, 1, true, None)).unwrap();
        app.update();
        assert_eq!(
            drain_snapshot_applied(&mut app)
                .iter()
                .map(|applied| applied.outcome)
                .collect::<Vec<_>>(),
            vec![ApplyOutcome::Full]
        );
        let before = app.world().resource::<NetworkState>();
        let (structures, minions) = (before.structures.clone(), before.minions.clone());
        let (projectiles, neutrals) = (before.projectiles.clone(), before.neutrals.clone());
        assert!(!before.remote_players.is_empty());
        assert!(!structures.is_empty() && !minions.is_empty());
        let structure = structures[&1];
        let structure_x = app
            .world()
            .get::<Transform>(structure)
            .unwrap()
            .translation
            .x;
        let structure_hp = app.world().get::<CombatStats>(structure).unwrap().hp;

        // Every structure moved and damaged, every minion gone: Draft ignores it.
        let mut draft = serde_json::to_value(admission_snapshot(1, 2, true, None)).unwrap();
        draft["prematch"] = json!({"generation": 1, "phase": "draft", "remaining_ms": 1000,
            "needed": 10, "players": [], "last_request_id": 0, "error": null});
        for structure in draft["structures"].as_array_mut().unwrap() {
            structure["x"] = json!(999.0);
            structure["hp"] = json!(1.0);
        }
        draft["minions"] = json!([]);
        incoming
            .send(serde_json::from_value(draft).unwrap())
            .unwrap();
        app.update();

        let after = app.world().resource::<NetworkState>();
        assert!(
            after.remote_players.is_empty(),
            "draft clears remote heroes"
        );
        assert_eq!(after.structures, structures);
        assert_eq!(after.minions, minions);
        assert_eq!(after.projectiles, projectiles);
        assert_eq!(after.neutrals, neutrals);
        for entity in minions.values().chain(structures.values()) {
            assert!(app.world().get_entity(*entity).is_ok());
        }
        assert_eq!(
            app.world()
                .get::<Transform>(structure)
                .unwrap()
                .translation
                .x,
            structure_x
        );
        assert_eq!(
            app.world().get::<CombatStats>(structure).unwrap().hp,
            structure_hp
        );
        let local_heroes = app
            .world_mut()
            .query_filtered::<Entity, With<Player>>()
            .iter(app.world())
            .count();
        assert_eq!(local_heroes, 0, "draft clears the local hero");
        // Resources are written even though the entity work is skipped.
        let game = app.world().resource::<GameStateSnapshot>();
        assert!(game.prematch.is_some());
        assert_eq!(game.meta.snapshot_tick, 2);
        let applied = drain_snapshot_applied(&mut app);
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].outcome, ApplyOutcome::Draft);
        assert_eq!(applied[0].your_id, 1);
        assert_eq!(applied[0].meta.snapshot_tick, 2);
        assert_eq!(applied[0].round, RoundId::from_meta(&applied[0].meta));
    }

    #[test]
    fn listed_local_hero_without_team_or_join_spawns_no_remote_or_world_entity() {
        let (mut app, incoming) = snapshot_app();
        app.world_mut().resource_mut::<TeamSelection>().team = None;
        incoming.send(admission_snapshot(1, 1, true, None)).unwrap();
        app.update();

        let state = app.world().resource::<NetworkState>();
        assert!(state.remote_players.is_empty());
        assert!(state.projectiles.is_empty() && state.structures.is_empty());
        assert!(state.minions.is_empty() && state.neutrals.is_empty());
        let heroes = app
            .world_mut()
            .query_filtered::<Entity, Or<(With<Player>, With<RemotePlayer>)>>()
            .iter(app.world())
            .count();
        assert_eq!(heroes, 0);
        // Session and resources run before the entity stage and still apply.
        assert_eq!(app.world().resource::<GameStateSnapshot>().your_id, 1);
        let session = app.world().resource::<ClientSession>();
        assert!(session.last_qualifying_snapshot_wall.is_some());
        let applied = drain_snapshot_applied(&mut app);
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].outcome, ApplyOutcome::LocalPending);
    }

    #[test]
    fn choose_authoritative_local_player_prefers_matching_network_id() {
        let candidates = [(1_u32, None), (2_u32, Some(77)), (3_u32, Some(13))];

        let chosen = choose_authoritative_local_player(&candidates, 77);

        assert_eq!(chosen, Some(2));
    }

    #[test]
    fn choose_authoritative_local_player_falls_back_to_first_candidate() {
        let candidates = [(11_u32, None), (12_u32, Some(7)), (13_u32, None)];

        let chosen = choose_authoritative_local_player(&candidates, 99);

        assert_eq!(chosen, Some(11));
    }

    #[test]
    fn choose_authoritative_local_player_returns_none_when_empty() {
        let candidates: [(u32, Option<u64>); 0] = [];

        let chosen = choose_authoritative_local_player(&candidates, 1);

        assert_eq!(chosen, None);
    }
}
