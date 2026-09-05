use bevy::{
    gltf::Gltf,
    input::mouse::MouseButton,
    math::{Dir3, primitives::InfinitePlane3d},
    prelude::*,
    window::PrimaryWindow,
};
use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;

use crate::camera::MainCamera;
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
    PlayerCosmeticAction, RemotePlayer, StructureKind,
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
                handle_player_input.after(crate::input_context::InputContextSet::Resolve),
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
            )
                .chain(),
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
    attack_node: Option<AnimationNodeIndex>,
    cast_node: Option<AnimationNodeIndex>,
    death_node: Option<AnimationNodeIndex>,
}

/// Grace period before Walk falls back to Idle. Remote players advance in
/// snapshot-interpolation bursts with still frames in between; without this
/// hysteresis the animation flaps Walk<->Idle several times per second.
const LOCOMOTION_IDLE_GRACE_SECS: f32 = 0.25;

#[derive(Component)]
struct PlayerAnimationBinding {
    owner: Entity,
    key: AvatarKey,
    playback: HeroAnimationPlayback,
    last_owner_position: Vec3,
    /// Seconds since the owner last visibly moved (drives the idle grace).
    seconds_since_movement: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HeroAnimationState {
    Idle,
    Walk,
    Attack,
    Cast,
    Death,
}

impl CharacterAnimationSet {
    fn node(&self, state: HeroAnimationState) -> AnimationNodeIndex {
        match state {
            HeroAnimationState::Idle => self.idle_node,
            HeroAnimationState::Walk => self.walk_node,
            HeroAnimationState::Attack => self.attack_node.unwrap_or(self.idle_node),
            HeroAnimationState::Cast => self.cast_node.unwrap_or(self.idle_node),
            HeroAnimationState::Death => self.death_node.unwrap_or(self.idle_node),
        }
    }

    fn available(&self, state: HeroAnimationState) -> bool {
        match state {
            HeroAnimationState::Attack => self.attack_node.is_some(),
            HeroAnimationState::Cast => self.cast_node.is_some(),
            HeroAnimationState::Death => self.death_node.is_some(),
            _ => true,
        }
    }
}

struct HeroAnimationPlayback {
    state: HeroAnimationState,
    last_action_sequence: u64,
    alive: bool,
    round: Option<(u64, u64)>,
}

impl HeroAnimationPlayback {
    fn new(sequence: u64) -> Self {
        Self {
            state: HeroAnimationState::Idle,
            last_action_sequence: sequence,
            alive: true,
            round: None,
        }
    }

    fn observe_round(&mut self, round: (u64, u64)) -> bool {
        let changed = self.round.is_some_and(|previous| previous != round);
        if changed {
            *self = Self::new(0);
        }
        self.round = Some(round);
        changed
    }

    /// Returns true when playback must restart, including two consecutive attacks.
    /// A round reset explicitly carries sequence zero. Older nonzero actions do
    /// not replay; the network layer independently orders authoritative snapshots.
    fn advance(
        &mut self,
        alive: bool,
        moving: bool,
        action: PlayerCosmeticAction,
        finished: bool,
        available: impl Fn(HeroAnimationState) -> bool,
    ) -> bool {
        let incoming = action.sequence > self.last_action_sequence;
        if incoming || action.sequence == 0 {
            self.last_action_sequence = action.sequence;
        }
        let respawned = !self.alive && alive;
        self.alive = alive;
        let locomotion = if moving {
            HeroAnimationState::Walk
        } else {
            HeroAnimationState::Idle
        };
        let action_state = match action.kind {
            shared::PlayerActionKind::Attack => Some(HeroAnimationState::Attack),
            shared::PlayerActionKind::Cast => Some(HeroAnimationState::Cast),
            shared::PlayerActionKind::None => None,
        };
        let new_action = alive && !respawned && incoming && action_state.is_some_and(&available);
        let next = if !alive {
            HeroAnimationState::Death
        } else if respawned {
            locomotion
        } else if new_action {
            action_state.unwrap_or(locomotion)
        } else if matches!(
            self.state,
            HeroAnimationState::Attack | HeroAnimationState::Cast
        ) && !finished
            && available(self.state)
        {
            self.state
        } else {
            locomotion
        };
        let restart = self.state != next || new_action;
        self.state = next;
        restart
    }
}

fn start_hero_animation(
    player: &mut AnimationPlayer,
    set: &CharacterAnimationSet,
    state: HeroAnimationState,
) {
    player.stop_all();
    let active = player.start(set.node(state));
    if matches!(state, HeroAnimationState::Idle | HeroAnimationState::Walk) {
        active.repeat();
    } else if state == HeroAnimationState::Death && !set.available(state) {
        // A missing death clip freezes a safe pose until authoritative respawn.
        active.pause();
    }
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
            let attack = find_clip(&["attack"]);
            let cast = find_clip(&["cast", "spell"]);
            let death = find_clip(&["death", "die"]);
            let mut clips = vec![idle_clip, walk_clip];
            let mut optional_indices = [None; 3];
            for (index, clip) in [attack, cast, death].into_iter().enumerate() {
                if let Some((_name, handle)) = clip {
                    optional_indices[index] = Some(clips.len());
                    clips.push(handle);
                }
            }
            let (graph, nodes) = AnimationGraph::from_clips(clips);
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
                    attack_node: optional_indices[0].and_then(|index| nodes.get(index).copied()),
                    cast_node: optional_indices[1].and_then(|index| nodes.get(index).copied()),
                    death_node: optional_indices[2].and_then(|index| nodes.get(index).copied()),
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
    owner_transform_query: Query<
        (&Transform, Option<&PlayerCosmeticAction>),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
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
        let (last_owner_position, sequence) = owner_transform_query
            .get(owner)
            .map(|(transform, action)| {
                (
                    transform.translation,
                    action.map_or(0, |value| value.sequence),
                )
            })
            .unwrap_or((Vec3::ZERO, 0));
        // Ensure we don't accidentally blend leftover animations from a previous graph.
        animation_player.stop_all();
        animation_player.play(set.idle_node).repeat();
        commands.entity(animation_entity).insert((
            AnimationGraphHandle(set.graph.clone()),
            PlayerAnimationBinding {
                owner,
                key,
                playback: HeroAnimationPlayback::new(sequence),
                last_owner_position,
                seconds_since_movement: LOCOMOTION_IDLE_GRACE_SECS,
            },
        ));
    }
}

fn sync_player_animation_state(
    time: Res<Time>,
    game_state: Option<Res<GameStateSnapshot>>,
    library: Res<PlayerAnimationLibrary>,
    character_query: Query<
        (&NetworkCharacterChoice, Option<&NetworkAvatar>),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
    local_movement_query: Query<(Option<&MovementTarget>, Option<&Jumping>), With<Player>>,
    player_state_query: Query<
        (&Transform, &CombatStats, Option<&PlayerCosmeticAction>),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
    mut animation_query: Query<(
        &mut AnimationPlayer,
        &mut PlayerAnimationBinding,
        &mut AnimationGraphHandle,
    )>,
) {
    for (mut animation_player, mut binding, mut graph_handle) in &mut animation_query {
        let Ok((owner_transform, stats, action)) = player_state_query.get(binding.owner) else {
            continue;
        };
        let action = action.copied().unwrap_or_default();
        let desired_key = character_query
            .get(binding.owner)
            .map(|(choice, avatar)| avatar_key(choice.0, avatar))
            .unwrap_or_else(|_| binding.key.clone());
        let key_changed = desired_key != binding.key;
        if key_changed {
            if library.get_set(&desired_key).is_none() {
                continue;
            }
            binding.key = desired_key;
            binding.playback = HeroAnimationPlayback::new(action.sequence);
        }
        let round_changed = game_state.as_ref().is_some_and(|state| {
            state.meta.server_epoch != 0
                && state.meta.match_id != 0
                && binding
                    .playback
                    .observe_round((state.meta.server_epoch, state.meta.match_id))
        });
        let Some(set) = library.get_set(&binding.key) else {
            continue;
        };
        let distance = owner_transform
            .translation
            .distance(binding.last_owner_position);
        let moved = distance / time.delta_secs().max(0.001) > 0.05;
        binding.last_owner_position = owner_transform.translation;
        if moved {
            binding.seconds_since_movement = 0.0;
        } else {
            binding.seconds_since_movement += time.delta_secs();
        }
        let moved_recently = binding.seconds_since_movement < LOCOMOTION_IDLE_GRACE_SECS;
        let moving_by_intent = local_movement_query
            .get(binding.owner)
            .map(|(target, jumping)| target.is_some() || jumping.is_some())
            .unwrap_or(false);
        let active_node = set.node(binding.playback.state);
        let finished = animation_player
            .animation(active_node)
            .is_none_or(|active| active.is_finished());
        let restart = binding.playback.advance(
            stats.is_alive(),
            moving_by_intent || moved || moved_recently,
            action,
            finished,
            |state| set.available(state),
        );
        let expected_graph_handle = AnimationGraphHandle(set.graph.clone());
        if key_changed
            || round_changed
            || restart
            || *graph_handle != expected_graph_handle
            || !animation_player.is_playing_animation(set.node(binding.playback.state))
        {
            *graph_handle = expected_graph_handle;
            start_hero_animation(&mut animation_player, set, binding.playback.state);
        }
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
    context: Res<crate::input_context::GameplayInputContext>,
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
    if !context.gameplay_allowed() {
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
    fn modal_ground_press_never_creates_movement_intent() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Touches>()
            .init_resource::<crate::input_context::GameplayInputContext>()
            .init_resource::<PlayerAnimationLibrary>()
            .init_resource::<PendingCast>()
            .init_resource::<WorldPointerState>()
            .insert_resource(PlayerVisualMode::Models3d)
            .add_systems(Update, handle_player_input);
        let player = app.world_mut().spawn((Player, CombatStats::default())).id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<crate::input_context::GameplayInputContext>()
            .modal_open = true;
        app.update();
        assert!(!app.world().entity(player).contains::<MovementTarget>());
        app.world_mut()
            .resource_mut::<crate::input_context::GameplayInputContext>()
            .modal_open = false;
        app.world_mut()
            .resource_mut::<crate::input_context::GameplayInputContext>()
            .debug_flight = true;
        app.update();
        assert!(!app.world().entity(player).contains::<MovementTarget>());
    }

    #[test]
    fn target_minimap_and_ui_presses_never_leak_ground_movement() {
        assert!(should_issue_ground_move(false, false, false));
        assert!(!should_issue_ground_move(true, false, false));
        assert!(!should_issue_ground_move(false, true, false));
        assert!(!should_issue_ground_move(false, false, true));
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use shared::PlayerActionKind;

    fn action(sequence: u64, kind: PlayerActionKind) -> PlayerCosmeticAction {
        PlayerCosmeticAction {
            sequence,
            kind,
            slot: 0,
        }
    }

    #[test]
    fn combat_animation_transitions_deduplicate_restart_and_hold_death() {
        let mut playback = HeroAnimationPlayback::new(0);
        let attack = action(1, PlayerActionKind::Attack);
        assert!(playback.advance(true, true, attack, false, |_| true));
        assert_eq!(playback.state, HeroAnimationState::Attack);
        assert!(!playback.advance(true, true, attack, false, |_| true));
        assert!(playback.advance(
            true,
            true,
            action(2, PlayerActionKind::Attack),
            false,
            |_| true
        ));
        assert_eq!(playback.state, HeroAnimationState::Attack);
        assert!(
            playback.advance(true, true, action(3, PlayerActionKind::Cast), false, |_| {
                true
            })
        );
        assert_eq!(playback.state, HeroAnimationState::Cast);
        assert!(
            playback.advance(true, true, action(3, PlayerActionKind::Cast), true, |_| {
                true
            })
        );
        assert_eq!(playback.state, HeroAnimationState::Walk);
        assert!(playback.advance(false, false, attack, true, |_| true));
        assert_eq!(playback.state, HeroAnimationState::Death);
        assert!(
            !playback.advance(false, true, action(4, PlayerActionKind::Cast), true, |_| {
                true
            })
        );
        assert_eq!(playback.state, HeroAnimationState::Death);
        assert!(
            playback.advance(true, false, action(4, PlayerActionKind::Cast), true, |_| {
                true
            })
        );
        assert_eq!(playback.state, HeroAnimationState::Idle);
    }

    #[test]
    fn new_round_action_rearms_even_when_initial_zero_snapshot_was_lost() {
        let mut playback = HeroAnimationPlayback::new(42);
        assert!(!playback.observe_round((100, 1)));
        playback.state = HeroAnimationState::Death;
        playback.alive = false;
        assert!(playback.observe_round((100, 2)));
        assert!(playback.advance(
            true,
            false,
            action(1, PlayerActionKind::Cast),
            false,
            |_| true
        ));
        assert_eq!(playback.state, HeroAnimationState::Cast);
        assert!(!playback.observe_round((100, 2)));
        assert!(!playback.advance(
            true,
            false,
            action(1, PlayerActionKind::Cast),
            false,
            |_| true
        ));
        assert!(playback.observe_round((200, 1)));
        assert!(playback.advance(
            true,
            false,
            action(1, PlayerActionKind::Attack),
            false,
            |_| true
        ));
        assert_eq!(playback.state, HeroAnimationState::Attack);
    }

    #[test]
    fn absent_action_clip_falls_back_and_round_zero_rearms_sequences() {
        let mut playback = HeroAnimationPlayback::new(10);
        assert!(!playback.advance(
            true,
            false,
            action(11, PlayerActionKind::Cast),
            false,
            |_| false
        ));
        assert_eq!(playback.state, HeroAnimationState::Idle);
        assert!(!playback.advance(
            true,
            false,
            action(9, PlayerActionKind::Attack),
            false,
            |_| true
        ));
        playback.advance(true, false, PlayerCosmeticAction::default(), true, |_| true);
        assert!(playback.advance(
            true,
            false,
            action(1, PlayerActionKind::Attack),
            false,
            |_| true
        ));
        assert_eq!(playback.state, HeroAnimationState::Attack);
        playback.advance(
            false,
            false,
            action(1, PlayerActionKind::Attack),
            true,
            |_| false,
        );
        assert_eq!(playback.state, HeroAnimationState::Death);
    }

    fn animation_set() -> CharacterAnimationSet {
        let (_, nodes) = AnimationGraph::from_clips([
            Handle::<AnimationClip>::default(),
            Handle::default(),
            Handle::default(),
            Handle::default(),
            Handle::default(),
        ]);
        CharacterAnimationSet {
            graph: Handle::default(),
            idle_node: nodes[0],
            walk_node: nodes[1],
            attack_node: Some(nodes[2]),
            cast_node: Some(nodes[3]),
            death_node: Some(nodes[4]),
        }
    }

    #[test]
    fn local_and_remote_scene_players_play_one_shots_once_and_respawn() {
        let mut app = App::new();
        let set = animation_set();
        let mut library = PlayerAnimationLibrary::default();
        library
            .sets
            .insert(AvatarKey::Roster("agnes".to_owned()), set.clone());
        app.insert_resource(Time::<()>::default())
            .insert_resource(library)
            .add_systems(
                Update,
                (bind_player_animation_players, sync_player_animation_state).chain(),
            );
        let mut entities = Vec::new();
        for local in [true, false] {
            let owner = app
                .world_mut()
                .spawn((
                    Transform::default(),
                    CombatStats::default(),
                    NetworkCharacterChoice(CharacterChoice::Cube),
                    NetworkAvatar(Some("agnes".to_owned())),
                    PlayerCosmeticAction::default(),
                ))
                .id();
            if local {
                app.world_mut().entity_mut(owner).insert(Player);
            } else {
                app.world_mut().entity_mut(owner).insert(RemotePlayer);
            }
            let child = app
                .world_mut()
                .spawn((AnimationPlayer::default(), ChildOf(owner)))
                .id();
            entities.push((owner, child));
        }
        app.update();
        for (owner, _) in &entities {
            app.world_mut()
                .entity_mut(*owner)
                .insert(action(1, PlayerActionKind::Attack));
        }
        app.update();
        for (_, child) in &entities {
            let player = app.world().get::<AnimationPlayer>(*child).unwrap();
            let active = player.animation(set.attack_node.unwrap()).unwrap();
            assert_eq!(
                active.repeat_mode(),
                bevy::animation::RepeatAnimation::Never
            );
            app.world_mut()
                .get_mut::<AnimationPlayer>(*child)
                .unwrap()
                .animation_mut(set.attack_node.unwrap())
                .unwrap()
                .seek_to(0.3);
        }
        app.update();
        for (owner, child) in &entities {
            assert_eq!(
                app.world()
                    .get::<AnimationPlayer>(*child)
                    .unwrap()
                    .animation(set.attack_node.unwrap())
                    .unwrap()
                    .seek_time(),
                0.3
            );
            app.world_mut()
                .entity_mut(*owner)
                .insert(action(2, PlayerActionKind::Cast));
        }
        app.update();
        for (owner, child) in &entities {
            assert!(
                app.world()
                    .get::<AnimationPlayer>(*child)
                    .unwrap()
                    .is_playing_animation(set.cast_node.unwrap())
            );
            app.world_mut().get_mut::<CombatStats>(*owner).unwrap().hp = 0.0;
        }
        app.update();
        for (_, child) in &entities {
            let mut player = app.world_mut().get_mut::<AnimationPlayer>(*child).unwrap();
            let active = player.animation_mut(set.death_node.unwrap()).unwrap();
            assert_eq!(
                active.repeat_mode(),
                bevy::animation::RepeatAnimation::Never
            );
            active.seek_to(0.9);
        }
        app.update();
        for (owner, child) in &entities {
            assert_eq!(
                app.world()
                    .get::<AnimationPlayer>(*child)
                    .unwrap()
                    .animation(set.death_node.unwrap())
                    .unwrap()
                    .seek_time(),
                0.9
            );
            app.world_mut().get_mut::<CombatStats>(*owner).unwrap().hp = 100.0;
        }
        app.update();
        for (_, child) in &entities {
            assert!(
                app.world()
                    .get::<AnimationPlayer>(*child)
                    .unwrap()
                    .is_playing_animation(set.idle_node)
            );
        }
    }

    #[test]
    fn unavailable_death_uses_a_paused_safe_pose() {
        let mut set = animation_set();
        set.death_node = None;
        let mut player = AnimationPlayer::default();
        start_hero_animation(&mut player, &set, HeroAnimationState::Death);
        assert!(player.animation(set.idle_node).unwrap().is_paused());
    }
}
