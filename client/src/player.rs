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
use crate::combat::{CombatStats, MAX_HP};
use crate::debug_console::DebugConsole;
use crate::maps::MapLayout;
use crate::minimap::MinimapNavigationState;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCharacterChoice, NetworkStructure, PlayerProgression,
    RemotePlayer, StructureKind,
};
use crate::team::{CharacterChoice, Team};
use crate::world::{PlayerModelCatalog, model_assets_for_choice};

pub const PLAYER_SPEED: f32 = 5.0;
pub const PLAYER_SIZE: f32 = 1.0;
pub const JUMP_HEIGHT: f32 = 1.5;
pub const JUMP_DURATION: f32 = 0.6;
const GRAVITY: f32 = 20.0;
const GROUND_EPSILON: f32 = 0.001;
const RESPAWN_DELAY_SECONDS: f32 = 5.0;

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
                .chain(),
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
        .init_resource::<PlayerAnimationLibrary>()
        .add_systems(Startup, (setup_respawn_ui, setup_progression_ui))
        .add_systems(Update, (respawn_countdown_system, progression_hud_system));
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
struct ProgressionHudText;

#[derive(Component)]
struct MovementTarget {
    target: Vec3,
}

#[derive(Component)]
struct Jumping {
    timer: Timer,
    start_y: f32,
}

#[derive(Resource, Default)]
struct PlayerAnimationLibrary {
    sets: HashMap<CharacterChoice, CharacterAnimationSet>,
    source_gltfs: HashMap<CharacterChoice, Handle<Gltf>>,
    evaluated_characters: HashSet<CharacterChoice>,
}

impl PlayerAnimationLibrary {
    fn has_locomotion_animations(&self) -> bool {
        !self.sets.is_empty()
    }

    fn should_use_jump_fallback(&self, character: CharacterChoice) -> bool {
        // Jumping is purely a fallback locomotion "effect" for non-skeletal models.
        // It should never apply to animated characters like Toka/Wang.
        character == CharacterChoice::Cube
    }

    fn get_set(&self, character: CharacterChoice) -> Option<&CharacterAnimationSet> {
        self.sets.get(&character)
    }
}

#[derive(Clone)]
struct CharacterAnimationSet {
    graph: Handle<AnimationGraph>,
    idle_node: AnimationNodeIndex,
    walk_node: AnimationNodeIndex,
}

#[derive(Component)]
struct PlayerAnimationBinding {
    owner: Entity,
    character: CharacterChoice,
    state: LocomotionAnimState,
    last_owner_position: Vec3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LocomotionAnimState {
    Idle,
    Walk,
}

fn setup_player_animation_library(
    mut library: ResMut<PlayerAnimationLibrary>,
    catalog: Option<Res<PlayerModelCatalog>>,
    gltf_assets: Res<Assets<Gltf>>,
    mut animation_graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Some(catalog) = catalog else {
        return;
    };
    for character in [
        CharacterChoice::Ipfs,
        CharacterChoice::Toka,
        CharacterChoice::Wang,
    ] {
        let (_scene, maybe_gltf) = model_assets_for_choice(&catalog, character);
        let Some(gltf_handle) = maybe_gltf else {
            // No GLTF means no skeletal animations — mark evaluated so jump fallback activates.
            library.evaluated_characters.insert(character);
            continue;
        };
        if library.source_gltfs.get(&character) == Some(&gltf_handle)
            && library.sets.contains_key(&character)
        {
            continue;
        }

        let Some(gltf) = gltf_assets.get(&gltf_handle) else {
            continue;
        };
        library.evaluated_characters.insert(character);
        library.source_gltfs.insert(character, gltf_handle.clone());

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
                    character, idle_name
                );
                library.sets.remove(&character);
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
                character,
                CharacterAnimationSet {
                    graph: graph_handle,
                    idle_node,
                    walk_node,
                },
            );
            info!(
                "Animation set ready for {:?}: idle={:?}, walk={:?}",
                character, idle_name, walk_name
            );
        } else {
            library.sets.remove(&character);
            warn!("idle/walk animations were not found for {:?}", character);
        }
    }
}

fn sync_jump_fallback_mode(
    mut commands: Commands,
    animation_library: Res<PlayerAnimationLibrary>,
    players: Query<(Entity, Option<&NetworkCharacterChoice>, Option<&Jumping>), With<Player>>,
) {
    for (entity, character, jumping) in &players {
        let character = character
            .map(|selected| selected.0)
            .unwrap_or(CharacterChoice::Ipfs);
        let should_jump_fallback = animation_library.should_use_jump_fallback(character);
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
    character_query: Query<&NetworkCharacterChoice, Or<(With<Player>, With<RemotePlayer>)>>,
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

        let Ok(character_choice) = character_query.get(owner) else {
            continue;
        };
        let character = character_choice.0;
        let Some(set) = library.get_set(character) else {
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
                character,
                state: LocomotionAnimState::Idle,
                last_owner_position,
            },
        ));
    }
}

fn sync_player_animation_state(
    time: Res<Time>,
    library: Res<PlayerAnimationLibrary>,
    character_query: Query<&NetworkCharacterChoice, Or<(With<Player>, With<RemotePlayer>)>>,
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
        let desired_character = character_query
            .get(binding.owner)
            .map(|choice| choice.0)
            .unwrap_or(binding.character);
        if desired_character != binding.character {
            let Some(new_set) = library.get_set(desired_character) else {
                continue;
            };
            animation_player.stop_all();
            binding.character = desired_character;
            binding.state = LocomotionAnimState::Idle;
            *graph_handle = AnimationGraphHandle(new_set.graph.clone());
            animation_player.play(new_set.idle_node).repeat();
        }

        let Some(set) = library.get_set(binding.character) else {
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
        let moving_by_intent = if local_player_query.get(binding.owner).is_ok() {
            local_movement_query
                .get(binding.owner)
                .map(|(movement_target, jumping)| movement_target.is_some() || jumping.is_some())
                .unwrap_or(false)
        } else {
            false
        };
        let moving = stats.is_alive() && (moving_by_intent || moved);
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
        binding.state = desired_state;
    }
}

fn handle_player_input(
    mut commands: Commands,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    player_query: Query<(Entity, &CombatStats, Option<&NetworkCharacterChoice>), With<Player>>,
    cam_state: Res<CameraState>,
    animation_library: Res<PlayerAnimationLibrary>,
    minimap_nav: Option<Res<MinimapNavigationState>>,
    map_layout: Option<Res<MapLayout>>,
    game_state: Option<Res<GameStateSnapshot>>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    if !cam_state.locked {
        return;
    }
    let Ok(window) = window_query.single() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    let Ok((player_entity, stats, character)) = player_query.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }

    if mouse_button_input.just_pressed(MouseButton::Left) {
        if minimap_nav
            .as_ref()
            .is_some_and(|nav_state| nav_state.consumed_primary_click)
        {
            return;
        }
        if let Some(cursor_pos) = window.cursor_position() {
            if let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) {
                if let Ok(plane_normal) = Dir3::new(Vec3::Y) {
                    let plane_origin = Vec3::ZERO;
                    let infinite_plane = InfinitePlane3d::new(plane_normal);
                    if let Some(distance) = ray.intersect_plane(plane_origin, infinite_plane) {
                        if distance >= 0.0 {
                            let mut target_pos = ray.get_point(distance);
                            if let Some(map_layout) = map_layout.as_ref() {
                                target_pos = map_layout.clamp_position(target_pos);
                            }
                            commands
                                .entity(player_entity)
                                .insert(MovementTarget { target: target_pos });
                            let character = character
                                .map(|selected| selected.0)
                                .unwrap_or(CharacterChoice::Ipfs);
                            if !animation_library.should_use_jump_fallback(character) {
                                commands.entity(player_entity).remove::<Jumping>();
                            } else {
                                commands.entity(player_entity).insert(Jumping {
                                    timer: Timer::from_seconds(JUMP_DURATION, TimerMode::Repeating),
                                    start_y: PLAYER_SIZE / 2.0,
                                });
                            }
                        }
                    }
                } else {
                    warn!("Plane normal is zero, cannot raycast");
                }
            }
        }
    }
}

fn move_player(
    mut commands: Commands,
    time: Res<Time>,
    mut transform_sets: ParamSet<(
        Query<(Entity, &mut Transform, &MovementTarget, &CombatStats), With<Player>>,
        Query<&Transform, (With<PlayerBody>, Without<Player>)>,
        Query<(&Transform, &StructureKind), With<NetworkStructure>>,
    )>,
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
        let move_delta = PLAYER_SPEED * time.delta_secs();

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
                let target_y_angle = direction.x.atan2(direction.z);
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
    mut query: Query<(&mut Transform, &mut Jumping), (With<Player>, With<MovementTarget>)>,
) {
    for (mut transform, mut jumping) in query.iter_mut() {
        jumping.timer.tick(time.delta());

        let progress = jumping.timer.fraction();

        let jump_offset = (progress * PI).sin() * JUMP_HEIGHT;

        transform.translation.y = jumping.start_y + jump_offset;
    }
}

fn apply_gravity(
    time: Res<Time>,
    mut query: Query<(&mut Transform, &mut VerticalVelocity, Option<&Jumping>), With<Player>>,
) {
    let ground_y = PLAYER_SIZE / 2.0;
    let dt = time.delta_secs();

    for (mut transform, mut velocity, jumping) in query.iter_mut() {
        if jumping.is_some() {
            velocity.0 = 0.0;
            continue;
        }

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

fn setup_progression_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                top: Val::Px(16.0),
                ..default()
            },
            Name::new("ProgressionHud"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Level --   XP --/--   Skill points --   Ranged rank --"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                ProgressionHudText,
            ));
        });
}

fn progression_hud_system(
    player_query: Query<&PlayerProgression, With<Player>>,
    mut text_query: Query<&mut Text, With<ProgressionHudText>>,
) {
    let Ok(mut text) = text_query.single_mut() else {
        return;
    };

    let Some(progression) = player_query.iter().next() else {
        text.0 = "Level --   XP --/--   Skill points --   Ranged rank --".to_string();
        return;
    };

    if progression.next_level_xp == 0 {
        text.0 = format!(
            "Level {}   XP MAX   Skill points {}   Ranged rank {}",
            progression.level.max(1),
            progression.skill_points,
            progression.ranged_shot_rank.max(1)
        );
    } else {
        let displayed_xp = progression.xp.min(progression.next_level_xp);
        text.0 = format!(
            "Level {}   XP {}/{}   Skill points {}   Ranged rank {}",
            progression.level.max(1),
            displayed_xp,
            progression.next_level_xp,
            progression.skill_points,
            progression.ranged_shot_rank.max(1)
        );
    }
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
