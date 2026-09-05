use bevy::{
    gltf::Gltf,
    input::mouse::MouseButton,
    math::{Dir3, primitives::InfinitePlane3d},
    prelude::*,
    window::PrimaryWindow,
};
use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;

use crate::camera::{CameraState, MainCamera};
use crate::combat::{
    CombatPointerInputSet, CombatStats, MAX_HP, PendingCast, WorldMovementInputSet,
    WorldPointerState,
};
use crate::debug_console::DebugConsole;
use crate::maps::MapLayout;
use crate::minimap::MinimapNavigationState;
use crate::model_scale::NormalizeModelScale;
use crate::net::{
    GameState, GameStateSnapshot, NetworkAvatar, NetworkCharacterChoice, NetworkStructure,
    RemotePlayer, StructureKind,
};
use crate::sprite::PlayerVisualMode;
use crate::team::{CharacterChoice, Team};
use crate::world::{AvatarAssetCache, PlayerModelCatalog, model_assets_for_choice};
use crate::world2d::render_xy_to_simulation_xz;

pub const PLAYER_SPEED: f32 = 5.0;
/// Debug movement multiplier; mirror of `server/src/balance.rs::DEBUG_SPEED_MULTIPLIER`.
pub const DEBUG_SPEED_MULTIPLIER: f32 = 2.6;

/// Debug speed-boost toggle, shared by the boost button and local movement.
#[derive(Resource, Default)]
pub struct DebugSpeedBoost(pub bool);
pub const PLAYER_SIZE: f32 = 1.0;
pub const JUMP_HEIGHT: f32 = 1.5;
pub const JUMP_DURATION: f32 = 0.6;
const GRAVITY: f32 = 20.0;
const GROUND_EPSILON: f32 = 0.001;
const RESPAWN_DELAY_SECONDS: f32 = 5.0;

/// Entity-origin height that puts a character's feet on the walkable surface
/// at (x, z): terrain height plus the model's measured foot offset. Entities
/// without a measured model (primitive cube stand-in, or a GLB that is still
/// loading) keep the legacy half-cube offset.
pub(crate) fn ground_origin_y(
    layout: &MapLayout,
    normalization: Option<&NormalizeModelScale>,
    x: f32,
    z: f32,
) -> f32 {
    let terrain = layout.terrain_height(x, z);
    let offset = match normalization.and_then(NormalizeModelScale::foot_local_y) {
        Some(foot_local_y) => -foot_local_y,
        None => PLAYER_SIZE * 0.5,
    };
    terrain + offset
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                sync_jump_fallback_mode,
                handle_player_input,
                animate_jump,
                move_player,
            )
                .chain()
                .after(CombatPointerInputSet)
                .in_set(WorldMovementInputSet),
        )
        .add_systems(
            Update,
            (
                setup_player_animation_library,
                bind_player_animation_players,
                sync_player_animation_state.after(move_player),
            ),
        )
        .add_systems(Update, resolve_player_structure_overlap.after(move_player))
        .add_systems(PostUpdate, apply_gravity)
        .init_resource::<RespawnCountdown>()
        .init_resource::<DebugSpeedBoost>()
        .init_resource::<PlayerAnimationLibrary>()
        .add_systems(Startup, setup_respawn_ui)
        .add_systems(Update, respawn_countdown_system);
    }
}

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct PlayerBody;

#[derive(Component, Default)]
pub struct VerticalVelocity(pub f32);

#[derive(Resource)]
struct RespawnCountdown {
    end_time: Option<f32>,
    last_shown: i32,
    last_hp: f32,
}

impl Default for RespawnCountdown {
    fn default() -> Self {
        Self {
            end_time: None,
            last_shown: -1,
            last_hp: MAX_HP,
        }
    }
}

#[derive(Component)]
struct RespawnCountdownText;

#[derive(Component)]
pub(crate) struct MovementTarget {
    pub(crate) target: Vec3,
}

#[derive(Component)]
struct Jumping {
    timer: Timer,
}

/// Identity of a player's visual model for animation purposes: either a
/// legacy SDK character or a roster avatar slug.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum AvatarKey {
    Character(CharacterChoice),
    Roster(String),
}

/// Resolves the animation identity of a player entity from its replicated
/// character + optional roster avatar components.
fn avatar_key(character: CharacterChoice, avatar: Option<&NetworkAvatar>) -> AvatarKey {
    match avatar.and_then(|avatar| avatar.0.as_deref()) {
        Some(slug) => AvatarKey::Roster(slug.to_owned()),
        None => AvatarKey::Character(character),
    }
}

#[derive(Resource, Default)]
struct PlayerAnimationLibrary {
    sets: HashMap<AvatarKey, CharacterAnimationSet>,
    source_gltfs: HashMap<AvatarKey, Handle<Gltf>>,
    evaluated_keys: HashSet<AvatarKey>,
}

impl PlayerAnimationLibrary {
    fn has_locomotion_animations(&self) -> bool {
        !self.sets.is_empty()
    }

    fn should_use_jump_fallback(&self, key: &AvatarKey) -> bool {
        // Jumping is purely a fallback locomotion "effect" for non-skeletal models.
        // It never applies to animated characters or roster avatars.
        *key == AvatarKey::Character(CharacterChoice::Cube)
    }

    fn get_set(&self, key: &AvatarKey) -> Option<&CharacterAnimationSet> {
        self.sets.get(key)
    }
}

#[derive(Clone)]
struct CharacterAnimationSet {
    graph: Handle<AnimationGraph>,
    idle_node: AnimationNodeIndex,
    walk_node: AnimationNodeIndex,
}

/// Grace period before Walk falls back to Idle. Remote players advance in
/// snapshot-interpolation bursts with still frames in between; without this
/// hysteresis the animation flaps Walk<->Idle several times per second.
const LOCOMOTION_IDLE_GRACE_SECS: f32 = 0.25;

#[derive(Component)]
struct PlayerAnimationBinding {
    owner: Entity,
    key: AvatarKey,
    state: LocomotionAnimState,
    last_owner_position: Vec3,
    /// Seconds since the owner last visibly moved (drives the idle grace).
    seconds_since_movement: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LocomotionAnimState {
    Idle,
    Walk,
}

fn setup_player_animation_library(
    mut library: ResMut<PlayerAnimationLibrary>,
    catalog: Option<Res<PlayerModelCatalog>>,
    avatar_cache: Res<AvatarAssetCache>,
    gltf_assets: Res<Assets<Gltf>>,
    mut animation_graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Some(catalog) = catalog else {
        return;
    };
    // Candidates: legacy animated SDK characters + every roster avatar that has
    // been requested (local selection or a remote player wearing it).
    let mut candidates: Vec<(AvatarKey, Option<Handle<Gltf>>)> = [
        CharacterChoice::Ipfs,
        CharacterChoice::Toka,
        CharacterChoice::Wang,
    ]
    .into_iter()
    .map(|character| {
        let (_scene, maybe_gltf) = model_assets_for_choice(&catalog, character);
        (AvatarKey::Character(character), maybe_gltf)
    })
    .collect();
    for (slug, gltf_handle) in avatar_cache.requested() {
        candidates.push((
            AvatarKey::Roster(slug.to_owned()),
            Some(gltf_handle.clone()),
        ));
    }

    for (key, maybe_gltf) in candidates {
        let Some(gltf_handle) = maybe_gltf else {
            // No GLTF means no skeletal animations — mark evaluated so jump fallback activates.
            library.evaluated_keys.insert(key);
            continue;
        };
        // Skip once this exact GLTF has been evaluated, whether or not it produced an
        // animation set. Gating on `sets` instead re-ran every frame for models that
        // lack idle/walk clips, spamming the "animations were not found" warning.
        if library.source_gltfs.get(&key) == Some(&gltf_handle)
            && library.evaluated_keys.contains(&key)
        {
            continue;
        }

        let Some(gltf) = gltf_assets.get(&gltf_handle) else {
            continue;
        };
        library.evaluated_keys.insert(key.clone());
        library
            .source_gltfs
            .insert(key.clone(), gltf_handle.clone());

        let find_clip = |substrings: &[&str]| -> Option<(String, Handle<AnimationClip>)> {
            for needle in substrings {
                if let Some((animation_name, handle)) =
                    gltf.named_animations
                        .iter()
                        .find(|(animation_name, _handle)| {
                            animation_name.to_ascii_lowercase().contains(needle)
                        })
                {
                    return Some((animation_name.to_string(), handle.clone()));
                }
            }
            None
        };

        let idle = find_clip(&["idle"]);
        // Prefer explicit walkcycle naming to avoid accidentally matching non-locomotion "walk*".
        let walk = find_clip(&["walkcycle", "walk_cycle", "walk"]);
        if let (Some((idle_name, idle_clip)), Some((walk_name, walk_clip))) = (idle, walk) {
            if idle_clip == walk_clip {
                warn!(
                    "idle/walk matched the same clip for {:?}: {:?}",
                    key, idle_name
                );
                library.sets.remove(&key);
                continue;
            }
            let (graph, nodes) = AnimationGraph::from_clips([idle_clip, walk_clip]);
            let Some(idle_node) = nodes.first().copied() else {
                continue;
            };
            let Some(walk_node) = nodes.get(1).copied() else {
                continue;
            };
            let graph_handle = animation_graphs.add(graph);
            library.sets.insert(
                key.clone(),
                CharacterAnimationSet {
                    graph: graph_handle,
                    idle_node,
                    walk_node,
                },
            );
            info!(
                "Animation set ready for {:?}: idle={:?}, walk={:?}",
                key, idle_name, walk_name
            );
        } else {
            library.sets.remove(&key);
            warn!("idle/walk animations were not found for {:?}", key);
        }
    }
}

fn sync_jump_fallback_mode(
    mut commands: Commands,
    animation_library: Res<PlayerAnimationLibrary>,
    players: Query<
        (
            Entity,
            Option<&NetworkCharacterChoice>,
            Option<&NetworkAvatar>,
            Option<&Jumping>,
        ),
        With<Player>,
    >,
) {
    for (entity, character, avatar, jumping) in &players {
        let character = character
            .map(|selected| selected.0)
            .unwrap_or(CharacterChoice::Ipfs);
        let key = avatar_key(character, avatar);
        let should_jump_fallback = animation_library.should_use_jump_fallback(&key);
        if !should_jump_fallback && jumping.is_some() {
            commands.entity(entity).remove::<Jumping>();
        }
    }
}

fn bind_player_animation_players(
    mut commands: Commands,
    library: Res<PlayerAnimationLibrary>,
    player_roots: Query<(), Or<(With<Player>, With<RemotePlayer>)>>,
    owner_transform_query: Query<&Transform, Or<(With<Player>, With<RemotePlayer>)>>,
    character_query: Query<
        (&NetworkCharacterChoice, Option<&NetworkAvatar>),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
    child_of_query: Query<&ChildOf>,
    mut animation_players: Query<
        (Entity, &mut AnimationPlayer),
        (With<AnimationPlayer>, Without<PlayerAnimationBinding>),
    >,
) {
    if !library.has_locomotion_animations() {
        return;
    }

    for (animation_entity, mut animation_player) in &mut animation_players {
        let mut current = animation_entity;
        let mut owner = None;
        loop {
            if player_roots.get(current).is_ok() {
                owner = Some(current);
                break;
            }
            let Ok(child_of) = child_of_query.get(current) else {
                break;
            };
            current = child_of.parent();
        }

        let Some(owner) = owner else {
            continue;
        };

        let Ok((character_choice, avatar)) = character_query.get(owner) else {
            continue;
        };
        let key = avatar_key(character_choice.0, avatar);
        let Some(set) = library.get_set(&key) else {
            continue;
        };
        let last_owner_position = owner_transform_query
            .get(owner)
            .map(|transform| transform.translation)
            .unwrap_or(Vec3::ZERO);
        // Ensure we don't accidentally blend leftover animations from a previous graph.
        animation_player.stop_all();
        animation_player.play(set.idle_node).repeat();
        commands.entity(animation_entity).insert((
            AnimationGraphHandle(set.graph.clone()),
            PlayerAnimationBinding {
                owner,
                key,
                state: LocomotionAnimState::Idle,
                last_owner_position,
                seconds_since_movement: LOCOMOTION_IDLE_GRACE_SECS,
            },
        ));
    }
}

fn sync_player_animation_state(
    time: Res<Time>,
    library: Res<PlayerAnimationLibrary>,
    character_query: Query<
        (&NetworkCharacterChoice, Option<&NetworkAvatar>),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
    local_player_query: Query<(), With<Player>>,
    local_movement_query: Query<(Option<&MovementTarget>, Option<&Jumping>), With<Player>>,
    player_state_query: Query<(&Transform, &CombatStats), Or<(With<Player>, With<RemotePlayer>)>>,
    mut animation_query: Query<(
        &mut AnimationPlayer,
        &mut PlayerAnimationBinding,
        &mut AnimationGraphHandle,
    )>,
) {
    if !library.has_locomotion_animations() {
        return;
    }

    for (mut animation_player, mut binding, mut graph_handle) in &mut animation_query {
        let Ok((owner_transform, stats)) = player_state_query.get(binding.owner) else {
            continue;
        };
        let desired_key = character_query
            .get(binding.owner)
            .map(|(choice, avatar)| avatar_key(choice.0, avatar))
            .unwrap_or_else(|_| binding.key.clone());
        if desired_key != binding.key {
            let Some(new_set) = library.get_set(&desired_key) else {
                continue;
            };
            animation_player.stop_all();
            binding.key = desired_key;
            binding.state = LocomotionAnimState::Idle;
            *graph_handle = AnimationGraphHandle(new_set.graph.clone());
            animation_player.play(new_set.idle_node).repeat();
        }

        let Some(set) = library.get_set(&binding.key) else {
            continue;
        };
        let active_node = match binding.state {
            LocomotionAnimState::Idle => set.idle_node,
            LocomotionAnimState::Walk => set.walk_node,
        };
        let expected_graph_handle = AnimationGraphHandle(set.graph.clone());
        if *graph_handle != expected_graph_handle {
            *graph_handle = expected_graph_handle;
            animation_player.stop_all();
            animation_player.play(active_node).repeat();
        } else if !animation_player.is_playing_animation(active_node) {
            // If the graph handle / player got reset (scene reload, late component insert, etc.),
            // force the expected animation to actually start.
            animation_player.stop_all();
            animation_player.play(active_node).repeat();
        }

        let distance = owner_transform
            .translation
            .distance(binding.last_owner_position);
        let speed = distance / time.delta_secs().max(0.001);
        let moved = speed > 0.05;
        binding.last_owner_position = owner_transform.translation;
        if moved {
            binding.seconds_since_movement = 0.0;
        } else {
            binding.seconds_since_movement += time.delta_secs();
        }
        let moved_recently = binding.seconds_since_movement < LOCOMOTION_IDLE_GRACE_SECS;
        let moving_by_intent = if local_player_query.get(binding.owner).is_ok() {
            local_movement_query
                .get(binding.owner)
                .map(|(movement_target, jumping)| movement_target.is_some() || jumping.is_some())
                .unwrap_or(false)
        } else {
            false
        };
        let moving = stats.is_alive() && (moving_by_intent || moved || moved_recently);
        let desired_state = if moving {
            LocomotionAnimState::Walk
        } else {
            LocomotionAnimState::Idle
        };

        if desired_state == binding.state {
            continue;
        }

        let node = match desired_state {
            LocomotionAnimState::Idle => set.idle_node,
            LocomotionAnimState::Walk => set.walk_node,
        };
        animation_player.stop_all();
        animation_player.play(node).repeat();
        info!(
            "Locomotion animation {:?} -> {:?} for {:?}",
            binding.state, desired_state, binding.key
        );
        binding.state = desired_state;
    }
}

fn handle_player_input(
    mut commands: Commands,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    player_query: Query<
        (
            Entity,
            &CombatStats,
            Option<&NetworkCharacterChoice>,
            Option<&NetworkAvatar>,
        ),
        With<Player>,
    >,
    cam_state: Res<CameraState>,
    animation_library: Res<PlayerAnimationLibrary>,
    minimap_nav: Option<Res<MinimapNavigationState>>,
    map_layout: Option<Res<MapLayout>>,
    game_state: Option<Res<GameStateSnapshot>>,
    visual_mode: Res<PlayerVisualMode>,
    pointer_state: Res<WorldPointerState>,
    mut pending_cast: ResMut<PendingCast>,
    ui_interactions: Query<&Interaction, With<Button>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    if !accepts_ground_movement_input(*visual_mode, cam_state.locked) {
        return;
    }
    let Ok(window) = window_query.single() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    let Ok((player_entity, stats, character, avatar)) = player_query.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }

    let Some(pointer_position) =
        primary_world_press_position(&mouse_button_input, &touches, window)
    else {
        return;
    };
    if !should_issue_ground_move(
        pointer_state.consumed_primary_press,
        minimap_nav
            .as_ref()
            .is_some_and(|nav_state| nav_state.consumed_primary_click),
        ui_interactions
            .iter()
            .any(|interaction| *interaction != Interaction::None),
    ) {
        return;
    }
    pending_cast.cancel();
    if let Some(mut target_pos) = viewport_to_simulation_world(
        camera,
        camera_transform,
        pointer_position,
        *visual_mode,
        0.0,
    ) {
        if let Some(map_layout) = map_layout.as_ref() {
            target_pos = map_layout.clamp_position(target_pos);
        }
        commands
            .entity(player_entity)
            .insert(MovementTarget { target: target_pos });
        let character = character
            .map(|selected| selected.0)
            .unwrap_or(CharacterChoice::Ipfs);
        let key = avatar_key(character, avatar);
        if !animation_library.should_use_jump_fallback(&key) {
            commands.entity(player_entity).remove::<Jumping>();
        } else {
            commands.entity(player_entity).insert(Jumping {
                timer: Timer::from_seconds(JUMP_DURATION, TimerMode::Repeating),
            });
        }
    }
}

fn primary_world_press_position(
    mouse_input: &ButtonInput<MouseButton>,
    touches: &Touches,
    window: &Window,
) -> Option<Vec2> {
    if mouse_input.just_pressed(MouseButton::Left) {
        return window.cursor_position();
    }
    touches
        .iter_just_pressed()
        .next()
        .map(|touch| touch.position())
}

const fn should_issue_ground_move(
    target_consumed: bool,
    minimap_consumed: bool,
    pointer_over_ui: bool,
) -> bool {
    !target_consumed && !minimap_consumed && !pointer_over_ui
}

fn accepts_ground_movement_input(mode: PlayerVisualMode, camera_locked: bool) -> bool {
    mode == PlayerVisualMode::Sprite2d || camera_locked
}

/// Maps the active camera viewport into authoritative simulation XZ.
pub(crate) fn viewport_to_simulation_world(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    viewport_position: Vec2,
    mode: PlayerVisualMode,
    simulation_y: f32,
) -> Option<Vec3> {
    let ray = camera
        .viewport_to_world(camera_transform, viewport_position)
        .ok()?;
    if mode == PlayerVisualMode::Sprite2d {
        return Some(render_xy_to_simulation_xz(ray.origin.xy(), simulation_y));
    }
    let plane_normal = Dir3::new(Vec3::Y).ok()?;
    let distance = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(plane_normal))?;
    (distance >= 0.0).then(|| ray.get_point(distance))
}

fn move_player(
    mut commands: Commands,
    time: Res<Time>,
    mut transform_sets: ParamSet<(
        Query<(Entity, &mut Transform, &MovementTarget, &CombatStats), With<Player>>,
        Query<&Transform, (With<PlayerBody>, Without<Player>)>,
        Query<(&Transform, &StructureKind), With<NetworkStructure>>,
    )>,
    speed_boost: Res<DebugSpeedBoost>,
    map_layout: Option<Res<MapLayout>>,
    game_state: Option<Res<GameStateSnapshot>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let other_players = transform_sets
        .p1()
        .iter()
        .map(|transform| transform.translation)
        .collect::<Vec<_>>();
    let structures = transform_sets
        .p2()
        .iter()
        .map(|(transform, kind)| (transform.translation, *kind))
        .collect::<Vec<_>>();

    let mut player_query = transform_sets.p0();
    for (entity, mut transform, movement_target, stats) in player_query.iter_mut() {
        if !stats.is_alive() {
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
            continue;
        }
        let current_pos = transform.translation;

        let target_pos_flat = Vec3::new(
            movement_target.target.x,
            current_pos.y,
            movement_target.target.z,
        );
        let direction = (target_pos_flat - current_pos).normalize_or_zero();
        let distance = current_pos.xz().distance(target_pos_flat.xz());
        let speed = if speed_boost.0 {
            PLAYER_SPEED * DEBUG_SPEED_MULTIPLIER
        } else {
            PLAYER_SPEED
        };
        let move_delta = speed * time.delta_secs();

        if distance < move_delta || distance < 0.01 {
            let mut desired = Vec3::new(
                movement_target.target.x,
                current_pos.y,
                movement_target.target.z,
            );
            desired = resolve_player_collisions(desired, &other_players, &structures);
            if let Some(map_layout) = map_layout.as_ref() {
                desired = map_layout.clamp_position(desired);
            }
            transform.translation.x = desired.x;
            transform.translation.z = desired.z;
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
        } else {
            let mut desired = current_pos + direction * move_delta;
            desired = resolve_player_collisions(desired, &other_players, &structures);
            if let Some(map_layout) = map_layout.as_ref() {
                desired = map_layout.clamp_position(desired);
            }
            transform.translation.x = desired.x;
            transform.translation.z = desired.z;

            if direction.length_squared() > 0.001 {
                // Character models face the entity's -Z (Bevy forward), so
                // yaw must point -Z along the movement direction; aligning +Z
                // renders every model walking backwards.
                let target_y_angle = (-direction.x).atan2(-direction.z);
                let target_rotation = Quat::from_rotation_y(target_y_angle);

                transform.rotation = transform
                    .rotation
                    .slerp(target_rotation, time.delta_secs() * 10.0);
            }
        }
    }
}

fn animate_jump(
    time: Res<Time>,
    map_layout: Res<MapLayout>,
    mut query: Query<
        (&mut Transform, &mut Jumping, Option<&NormalizeModelScale>),
        (With<Player>, With<MovementTarget>),
    >,
) {
    for (mut transform, mut jumping, normalization) in query.iter_mut() {
        jumping.timer.tick(time.delta());

        let progress = jumping.timer.fraction();

        let jump_offset = (progress * PI).sin() * JUMP_HEIGHT;

        // Base follows the terrain so hops track ramps instead of clipping.
        let base_y = ground_origin_y(
            &map_layout,
            normalization,
            transform.translation.x,
            transform.translation.z,
        );
        transform.translation.y = base_y + jump_offset;
    }
}

fn apply_gravity(
    time: Res<Time>,
    map_layout: Res<MapLayout>,
    mut query: Query<
        (
            &mut Transform,
            &mut VerticalVelocity,
            Option<&Jumping>,
            Option<&NormalizeModelScale>,
        ),
        With<Player>,
    >,
) {
    let dt = time.delta_secs();

    for (mut transform, mut velocity, jumping, normalization) in query.iter_mut() {
        if jumping.is_some() {
            velocity.0 = 0.0;
            continue;
        }

        let ground_y = ground_origin_y(
            &map_layout,
            normalization,
            transform.translation.x,
            transform.translation.z,
        );

        if transform.translation.y <= ground_y + GROUND_EPSILON {
            transform.translation.y = ground_y;
            velocity.0 = 0.0;
            continue;
        }

        velocity.0 -= GRAVITY * dt;
        transform.translation.y += velocity.0 * dt;

        if transform.translation.y <= ground_y {
            transform.translation.y = ground_y;
            velocity.0 = 0.0;
        }
    }
}

fn setup_respawn_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(20.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Name::new("RespawnCountdown"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: 36.0,
                    ..default()
                },
                TextColor::WHITE,
                Visibility::Hidden,
                RespawnCountdownText,
            ));
        });
}

fn respawn_countdown_system(
    time: Res<Time>,
    map_layout: Res<MapLayout>,
    mut commands: Commands,
    mut state: ResMut<RespawnCountdown>,
    mut player_query: Query<
        (
            Entity,
            &mut Transform,
            &CombatStats,
            &Team,
            &mut VerticalVelocity,
        ),
        With<Player>,
    >,
    mut text_query: Query<(&mut Text, &mut Visibility), With<RespawnCountdownText>>,
    mut console: ResMut<DebugConsole>,
    game_state: Option<Res<GameStateSnapshot>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Ok((entity, mut transform, stats, team, mut velocity)) = player_query.single_mut() else {
        return;
    };

    let Ok((mut text, mut visibility)) = text_query.single_mut() else {
        return;
    };

    if stats.is_alive() {
        if state.end_time.is_some() {
            state.end_time = None;
            state.last_shown = -1;
            *visibility = Visibility::Hidden;
            text.0.clear();
            let spawn = map_layout.team_spawn(*team);
            transform.translation = spawn;
            velocity.0 = 0.0;
            commands.entity(entity).remove::<MovementTarget>();
            commands.entity(entity).remove::<Jumping>();
            console.push_line("Respawned.");
            info!("Respawned.");
        }
        state.last_hp = stats.hp;
        return;
    }

    if state.last_hp > 0.0 && stats.hp <= 0.0 {
        state.end_time = Some(time.elapsed_secs() + RESPAWN_DELAY_SECONDS);
        state.last_shown = -1;
    }
    let end_time = state
        .end_time
        .get_or_insert_with(|| time.elapsed_secs() + RESPAWN_DELAY_SECONDS);
    let remaining = (*end_time - time.elapsed_secs()).ceil().max(0.0) as i32;

    *visibility = Visibility::Visible;
    if remaining != state.last_shown {
        state.last_shown = remaining;
        text.0 = remaining.to_string();
        let message = format!("Respawn in {remaining}");
        console.push_line(message.clone());
        info!("{message}");
    }
    state.last_hp = stats.hp;
}

fn resolve_player_collisions(
    desired: Vec3,
    other_players: &[Vec3],
    structures: &[(Vec3, StructureKind)],
) -> Vec3 {
    let mut resolved = desired;
    let min_distance = PLAYER_SIZE;
    let player_radius = PLAYER_SIZE * 0.5;

    for &other_pos in other_players.iter() {
        let delta = Vec3::new(resolved.x - other_pos.x, 0.0, resolved.z - other_pos.z);
        let distance = delta.length();
        if distance < min_distance {
            let push_dir = if distance > 0.0001 {
                delta / distance
            } else {
                Vec3::X
            };
            resolved.x = other_pos.x + push_dir.x * min_distance;
            resolved.z = other_pos.z + push_dir.z * min_distance;
        }
    }

    for &(obstacle_pos, kind) in structures.iter() {
        let obstacle_radius = match kind {
            StructureKind::Tower => 1.3,
            StructureKind::BaseTower => 3.2,
        };
        let min_distance = player_radius + obstacle_radius;
        let delta = Vec3::new(
            resolved.x - obstacle_pos.x,
            0.0,
            resolved.z - obstacle_pos.z,
        );
        let distance = delta.length();
        if distance < min_distance {
            let push_dir = if distance > 0.0001 {
                delta / distance
            } else {
                Vec3::X
            };
            resolved.x = obstacle_pos.x + push_dir.x * min_distance;
            resolved.z = obstacle_pos.z + push_dir.z * min_distance;
        }
    }

    resolved
}

fn resolve_player_structure_overlap(
    mut player_query: Query<&mut Transform, With<Player>>,
    structures: Query<(&Transform, &StructureKind), (With<NetworkStructure>, Without<Player>)>,
) {
    let Ok(mut player_transform) = player_query.single_mut() else {
        return;
    };
    let player_radius = PLAYER_SIZE * 0.5;
    let mut resolved = player_transform.translation;

    for (structure_transform, kind) in structures.iter() {
        let obstacle_radius = match kind {
            StructureKind::Tower => 1.3,
            StructureKind::BaseTower => 3.2,
        };
        let min_distance = player_radius + obstacle_radius;
        let delta = Vec3::new(
            resolved.x - structure_transform.translation.x,
            0.0,
            resolved.z - structure_transform.translation.z,
        );
        let distance = delta.length();
        if distance < min_distance {
            let push_dir = if distance > 0.0001 {
                delta / distance
            } else {
                Vec3::X
            };
            resolved.x = structure_transform.translation.x + push_dir.x * min_distance;
            resolved.z = structure_transform.translation.z + push_dir.z * min_distance;
        }
    }

    player_transform.translation.x = resolved.x;
    player_transform.translation.z = resolved.z;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_ground_movement_does_not_depend_on_camera_follow() {
        assert!(accepts_ground_movement_input(
            PlayerVisualMode::Sprite2d,
            true
        ));
        assert!(accepts_ground_movement_input(
            PlayerVisualMode::Sprite2d,
            false
        ));
    }

    #[test]
    fn model_ground_movement_preserves_legacy_camera_gate() {
        assert!(accepts_ground_movement_input(
            PlayerVisualMode::Models3d,
            true
        ));
        assert!(!accepts_ground_movement_input(
            PlayerVisualMode::Models3d,
            false
        ));
    }

    #[test]
    fn target_minimap_and_ui_presses_never_leak_ground_movement() {
        assert!(should_issue_ground_move(false, false, false));
        assert!(!should_issue_ground_move(true, false, false));
        assert!(!should_issue_ground_move(false, true, false));
        assert!(!should_issue_ground_move(false, false, true));
    }
}
