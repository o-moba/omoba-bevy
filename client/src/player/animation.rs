use crate::combat::CombatStats;
use crate::domain::{MovementTarget, Player};
use crate::net::{
    GameStateSnapshot, NetworkAvatar, NetworkCharacterChoice, PlayerCosmeticAction, RemotePlayer,
};
use crate::team::CharacterChoice;
use crate::world::{AvatarAssetCache, PlayerModelCatalog, model_assets_for_choice};
use bevy::{gltf::Gltf, prelude::*};
use std::collections::{HashMap, HashSet};

use super::motion::Jumping;

/// The real animation pipeline is also used by the opt-in native motion audit.
pub(crate) fn register_hero_animation_systems(app: &mut App) {
    app.init_resource::<PlayerAnimationLibrary>()
        .init_resource::<crate::humanoid::HumanoidRuntimeLibrary>()
        .add_systems(
            // SceneSpawner runs after Update and may restore imported targets.
            // Bind after scene writes, before animation consumes those targets.
            PostUpdate,
            (
                setup_player_animation_library,
                prepare_runtime_humanoid_requests,
                crate::humanoid::bind_runtime_humanoids,
                bind_player_animation_players,
                sync_player_animation_state,
            )
                .chain()
                .before(bevy::app::AnimationSystems),
        );
}

/// Identity of a player's visual model for animation purposes: either a
/// legacy SDK character or a roster avatar slug.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum AvatarKey {
    Character(CharacterChoice),
    Roster(String),
}

/// Resolves the animation identity of a player entity from its replicated
/// character + optional roster avatar components.
pub(super) fn avatar_key(character: CharacterChoice, avatar: Option<&NetworkAvatar>) -> AvatarKey {
    match avatar.and_then(|avatar| avatar.0.as_deref()) {
        Some(slug) => AvatarKey::Roster(slug.to_owned()),
        None => AvatarKey::Character(character),
    }
}

#[derive(Resource, Default)]
pub(super) struct PlayerAnimationLibrary {
    pub(super) sets: HashMap<AvatarKey, CharacterAnimationSet>,
    source_gltfs: HashMap<AvatarKey, Handle<Gltf>>,
    evaluated_keys: HashSet<AvatarKey>,
    cosmetic_revision: u64,
}

impl PlayerAnimationLibrary {
    fn has_locomotion_animations(&self) -> bool {
        !self.sets.is_empty()
    }

    pub(super) fn should_use_jump_fallback(&self, key: &AvatarKey) -> bool {
        // Jumping is purely a fallback locomotion "effect" for non-skeletal models.
        // It never applies to animated characters or roster avatars.
        *key == AvatarKey::Character(CharacterChoice::Cube)
    }

    fn get_set(&self, key: &AvatarKey) -> Option<&CharacterAnimationSet> {
        self.sets.get(key)
    }
}

#[derive(Clone)]
pub(super) struct CharacterAnimationSet {
    pub(super) graph: Handle<AnimationGraph>,
    pub(super) idle_node: AnimationNodeIndex,
    pub(super) run_node: AnimationNodeIndex,
    pub(super) walk_node: Option<AnimationNodeIndex>,
    pub(super) runtime: bool,
    pub(super) attack_node: Option<AnimationNodeIndex>,
    pub(super) cast_node: Option<AnimationNodeIndex>,
    pub(super) death_node: Option<AnimationNodeIndex>,
}

/// Grace period before Run falls back to Idle. Remote players advance in
/// snapshot-interpolation bursts with still frames in between; without this
/// hysteresis the animation flaps Run<->Idle several times per second.
const LOCOMOTION_IDLE_GRACE_SECS: f32 = 0.25;

#[derive(Component)]
pub(crate) struct PlayerAnimationBinding {
    owner: Entity,
    key: AvatarKey,
    pub(super) playback: HeroAnimationPlayback,
    last_owner_position: Vec3,
    /// Seconds since the owner last visibly moved (drives the idle grace).
    seconds_since_movement: f32,
    sandbox_preview: Option<u64>,
    sandbox_paused: bool,
    sandbox_time: Option<((u64, u64), f64)>,
}

impl PlayerAnimationBinding {
    pub(crate) fn is_running(&self) -> bool {
        self.playback.state == HeroAnimationState::Run
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum HeroAnimationState {
    Idle,
    Run,
    /// Reserved for an explicit future debuff locomotion policy.
    #[allow(dead_code)]
    Walk,
    Attack,
    Cast,
    Death,
}

impl CharacterAnimationSet {
    pub(super) fn node(&self, state: HeroAnimationState) -> AnimationNodeIndex {
        match state {
            HeroAnimationState::Idle => self.idle_node,
            HeroAnimationState::Run => self.run_node,
            HeroAnimationState::Walk => self.walk_node.unwrap_or(self.idle_node),
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
            HeroAnimationState::Walk => self.walk_node.is_some(),
            _ => true,
        }
    }
}

pub(super) struct HeroAnimationPlayback {
    pub(super) state: HeroAnimationState,
    last_action_sequence: u64,
    pub(super) alive: bool,
    round: Option<(u64, u64)>,
}

impl HeroAnimationPlayback {
    pub(super) fn new(sequence: u64) -> Self {
        Self {
            state: HeroAnimationState::Idle,
            last_action_sequence: sequence,
            alive: true,
            round: None,
        }
    }

    pub(super) fn observe_round(&mut self, round: (u64, u64)) -> bool {
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
    pub(super) fn advance(
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
            HeroAnimationState::Run
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

pub(super) fn start_hero_animation(
    player: &mut AnimationPlayer,
    set: &CharacterAnimationSet,
    state: HeroAnimationState,
) {
    player.stop_all();
    let active = player.start(set.node(state));
    if matches!(
        state,
        HeroAnimationState::Idle | HeroAnimationState::Run | HeroAnimationState::Walk
    ) {
        active.repeat();
    } else if state == HeroAnimationState::Death && !set.available(state) {
        // A missing death clip freezes a safe pose until authoritative respawn.
        active.pause();
    }
}

fn setup_player_animation_library(
    mut library: ResMut<PlayerAnimationLibrary>,
    cosmetics: Option<Res<crate::combat_visuals::CombatVisualRegistry>>,
    catalog: Option<Res<PlayerModelCatalog>>,
    avatar_cache: Res<AvatarAssetCache>,
    gltf_assets: Res<Assets<Gltf>>,
    mut animation_graphs: ResMut<Assets<AnimationGraph>>,
    mut runtime: ResMut<crate::humanoid::HumanoidRuntimeLibrary>,
    mut animation_clips: ResMut<Assets<AnimationClip>>,
    actual_models: Query<
        (
            &NetworkCharacterChoice,
            Option<&NetworkAvatar>,
            &crate::model_scale::ModelScaleSource,
        ),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
) {
    let revision = cosmetics.as_ref().map_or(0, |registry| registry.revision());
    if library.cosmetic_revision != revision {
        library.cosmetic_revision = revision;
        library.evaluated_keys.clear();
    }
    // Candidates: legacy animated SDK characters + every roster avatar that has
    // been requested (local selection or a remote player wearing it).
    let mut candidates: Vec<(AvatarKey, Option<Handle<Gltf>>)> = [
        CharacterChoice::Ipfs,
        CharacterChoice::Toka,
        CharacterChoice::Wang,
    ]
    .into_iter()
    .filter_map(|character| {
        let catalog = catalog.as_ref()?;
        let (_scene, maybe_gltf) = model_assets_for_choice(catalog, character);
        Some((AvatarKey::Character(character), maybe_gltf))
    })
    .collect();
    for (slug, gltf_handle) in avatar_cache.requested() {
        candidates.push((
            AvatarKey::Roster(slug.to_owned()),
            Some(gltf_handle.clone()),
        ));
    }

    // The instantiated model wins over a stale catalogue handle during replacement.
    for (choice, avatar, source) in &actual_models {
        let key = avatar_key(choice.0, avatar);
        candidates.retain(|(candidate, _)| candidate != &key);
        candidates.push((key, Some(source.gltf.clone())));
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

        let cosmetic_key = match &key {
            AvatarKey::Roster(slug) => slug.clone(),
            AvatarKey::Character(character) => {
                format!("character:{character:?}").to_ascii_lowercase()
            }
        };
        let aliases = cosmetics
            .as_ref()
            .and_then(|registry| registry.animation_aliases(&cosmetic_key));
        let is_humanoid = gltf.source.as_ref().is_some_and(|source| {
            source.extension_value("VRM").is_some() || source.extension_value("VRMC_vrm").is_some()
        });
        if is_humanoid {
            match runtime.ensure(&gltf_handle, gltf, &mut animation_clips) {
                Ok(mut motion) => {
                    // Keep explicit cosmetic overrides by translating their
                    // original glTF targets into this runtime rig's namespace.
                    // Unsupported aliases fall back as a whole, never as a
                    // partially animated pose.
                    let mut apply_alias =
                        |state: &str,
                         names: Option<&Vec<String>>,
                         destination: &mut Handle<AnimationClip>| {
                            let Some(names) = names else {
                                return;
                            };
                            for name in names {
                                if state == "run" && name.to_ascii_lowercase().contains("walk") {
                                    warn!("Ignoring walk clip as ordinary Run alias for {:?}", key);
                                    continue;
                                }
                                let Some(original) = gltf
                                    .named_animations
                                    .get(name.as_str())
                                    .and_then(|handle| animation_clips.get(handle))
                                    .cloned()
                                else {
                                    continue;
                                };
                                match runtime.remap_embedded_clip(&gltf_handle, gltf, &original) {
                                    Ok(clip) => {
                                        *destination = animation_clips.add(clip);
                                        return;
                                    }
                                    Err(error) => warn!(
                                        "Unsupported {state} alias {name:?} for {:?}: {error}",
                                        key
                                    ),
                                }
                            }
                        };
                    apply_alias("idle", aliases.map(|a| &a.idle), &mut motion.idle);
                    apply_alias("run", aliases.map(|a| &a.run), &mut motion.run);
                    apply_alias("walk", aliases.map(|a| &a.walk), &mut motion.walk);
                    apply_alias("attack", aliases.map(|a| &a.attack), &mut motion.attack);
                    apply_alias("cast", aliases.map(|a| &a.cast), &mut motion.cast);
                    apply_alias("death", aliases.map(|a| &a.death), &mut motion.death);
                    let (graph, nodes) = AnimationGraph::from_clips([
                        motion.idle,
                        motion.run,
                        motion.walk,
                        motion.attack,
                        motion.cast,
                        motion.death,
                    ]);
                    library.sets.insert(
                        key.clone(),
                        CharacterAnimationSet {
                            graph: animation_graphs.add(graph),
                            idle_node: nodes[0],
                            run_node: nodes[1],
                            walk_node: Some(nodes[2]),
                            runtime: true,
                            attack_node: Some(nodes[3]),
                            cast_node: Some(nodes[4]),
                            death_node: Some(nodes[5]),
                        },
                    );
                    info!(
                        "Shared humanoid motion ready for {:?}: Run = Sprint_Loop (runtime retarget)",
                        key
                    );
                }
                Err(error) => {
                    library.sets.remove(&key);
                    warn!("Humanoid motion unavailable for {:?}: {}", key, error);
                }
            }
            continue;
        }

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

        let exact_clip = |names: Option<&Vec<String>>| -> Option<(String, Handle<AnimationClip>)> {
            names?.iter().find_map(|name| {
                gltf.named_animations
                    .get(name.as_str())
                    .map(|handle| (name.clone(), handle.clone()))
            })
        };
        let mut idle =
            exact_clip(aliases.map(|aliases| &aliases.idle)).or_else(|| find_clip(&["idle"]));
        let reserved_walk = exact_clip(aliases.map(|aliases| &aliases.walk))
            .or_else(|| find_clip(&["walkcycle", "walk_cycle", "walk"]));
        let mut walk = exact_clip(aliases.map(|aliases| &aliases.run))
            .or_else(|| find_clip(&["sprint", "run", "jog"]))
            .or_else(|| reserved_walk.clone());
        if matches!((&idle, &walk), (Some((_, idle)), Some((_, walk))) if idle == walk) {
            // A conflicting cosmetic alias must not disable valid built-in locomotion.
            idle = find_clip(&["idle"]);
            walk = find_clip(&["sprint", "run", "jog", "walkcycle", "walk_cycle", "walk"]);
        }
        if let (Some((idle_name, idle_clip)), Some((walk_name, walk_clip))) = (idle, walk) {
            if idle_clip == walk_clip {
                warn!(
                    "idle/walk matched the same clip for {:?}: {:?}",
                    key, idle_name
                );
                library.sets.remove(&key);
                continue;
            }
            let attack = exact_clip(aliases.map(|aliases| &aliases.attack))
                .or_else(|| find_clip(&["attack"]));
            let cast = exact_clip(aliases.map(|aliases| &aliases.cast))
                .or_else(|| find_clip(&["cast", "spell"]));
            let death = exact_clip(aliases.map(|aliases| &aliases.death))
                .or_else(|| find_clip(&["death", "die"]));
            let mut clips = vec![idle_clip, walk_clip];
            let mut optional_indices = [None; 4];
            for (index, clip) in [attack, cast, death, reserved_walk].into_iter().enumerate() {
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
                    run_node: walk_node,
                    walk_node: optional_indices[3].and_then(|index| nodes.get(index).copied()),
                    runtime: false,
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

pub(super) fn sync_jump_fallback_mode(
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

fn prepare_runtime_humanoid_requests(
    mut commands: Commands,
    library: Res<PlayerAnimationLibrary>,
    models: Query<
        (
            Entity,
            &NetworkCharacterChoice,
            Option<&NetworkAvatar>,
            &crate::model_scale::ModelScaleSource,
            Option<&crate::humanoid::RuntimeHumanoidRequest>,
        ),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
) {
    for (entity, choice, avatar, source, request) in &models {
        let key = avatar_key(choice.0, avatar);
        if library.get_set(&key).is_some_and(|set| set.runtime)
            && library.source_gltfs.get(&key) == Some(&source.gltf)
            && request.is_none_or(|request| request.model != source.gltf)
        {
            commands
                .entity(entity)
                .insert(crate::humanoid::RuntimeHumanoidRequest {
                    model: source.gltf.clone(),
                });
            commands.entity(entity).remove::<PlayerAnimationBinding>();
        } else if request.is_some()
            && (!library.get_set(&key).is_some_and(|set| set.runtime)
                || library.source_gltfs.get(&key) != Some(&source.gltf))
        {
            // The runtime binder clears owned targets and its player atomically.
            commands.entity(entity).remove::<(
                crate::humanoid::RuntimeHumanoidRequest,
                PlayerAnimationBinding,
            )>();
        }
    }
}

pub(super) fn bind_player_animation_players(
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
        (
            Entity,
            &mut AnimationPlayer,
            Option<&crate::humanoid::RuntimeHumanoidPlayer>,
        ),
        (With<AnimationPlayer>, Without<PlayerAnimationBinding>),
    >,
) {
    if !library.has_locomotion_animations() {
        return;
    }

    for (animation_entity, mut animation_player, runtime_player) in &mut animation_players {
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
        if set.runtime != runtime_player.is_some() {
            animation_player.stop_all();
            continue;
        }
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
                sandbox_preview: None,
                sandbox_paused: false,
                sandbox_time: None,
            },
        ));
    }
}

/// Preview chooses an actual graph node and reports missing clips explicitly.
pub(super) fn sandbox_preview_state(
    kind: crate::sandbox::PreviewKind,
    set: &CharacterAnimationSet,
) -> (HeroAnimationState, String) {
    use crate::sandbox::PreviewKind;
    let requested = match kind {
        PreviewKind::Idle => HeroAnimationState::Idle,
        PreviewKind::Run => HeroAnimationState::Run,
        PreviewKind::Walk => HeroAnimationState::Walk,
        PreviewKind::Attack => HeroAnimationState::Attack,
        PreviewKind::Cast => HeroAnimationState::Cast,
        PreviewKind::Hit => HeroAnimationState::Attack,
        PreviewKind::Death => HeroAnimationState::Death,
    };
    if kind == PreviewKind::Hit {
        let fallback = if set.available(HeroAnimationState::Attack) {
            HeroAnimationState::Attack
        } else {
            HeroAnimationState::Idle
        };
        return (
            fallback,
            format!("Hit → {fallback:?} (Hit clip unavailable)"),
        );
    }
    if !set.available(requested) {
        // Retain Death's existing safe-pose freeze rather than pretending Idle is death.
        let state = if requested == HeroAnimationState::Death {
            requested
        } else {
            HeroAnimationState::Idle
        };
        return (
            state,
            format!(
                "{kind:?} → Idle (clip unavailable{})",
                if requested == HeroAnimationState::Death {
                    "; frozen"
                } else {
                    ""
                }
            ),
        );
    }
    (requested, format!("{requested:?} · preview"))
}
pub(super) fn sandbox_available_animations(set: &CharacterAnimationSet) -> Vec<String> {
    [
        HeroAnimationState::Idle,
        HeroAnimationState::Run,
        HeroAnimationState::Walk,
        HeroAnimationState::Attack,
        HeroAnimationState::Cast,
        HeroAnimationState::Death,
    ]
    .into_iter()
    .filter(|state| set.available(*state))
    .map(|state| {
        if state == HeroAnimationState::Run && set.walk_node == Some(set.run_node) {
            "Run (shared locomotion clip)".into()
        } else {
            format!("{state:?}")
        }
    })
    .collect()
}
fn animation_clip_duration(
    set: &CharacterAnimationSet,
    state: HeroAnimationState,
    graphs: Option<&Assets<AnimationGraph>>,
    clips: Option<&Assets<AnimationClip>>,
) -> Option<f32> {
    let node = graphs?.get(&set.graph)?.get(set.node(state))?;
    let bevy::animation::graph::AnimationNodeType::Clip(handle) = &node.node_type else {
        return None;
    };
    clips?
        .get(handle)
        .map(AnimationClip::duration)
        .filter(|duration| *duration > 0.0 && duration.is_finite())
}
pub(super) fn sandbox_seek_time(
    current: f32,
    delta: f32,
    duration: Option<f32>,
    looping: bool,
) -> f32 {
    let next = current + delta;
    duration.map_or(next, |duration| {
        if looping {
            next.rem_euclid(duration)
        } else {
            next.min(duration)
        }
    })
}

pub(super) fn sync_player_animation_state(
    time: Res<Time>,
    game_state: Option<Res<GameStateSnapshot>>,
    sandbox: Option<Res<crate::sandbox::SandboxClient>>,
    mut readout: Option<ResMut<crate::sandbox::AnimationReadout>>,
    graphs: Option<Res<Assets<AnimationGraph>>>,
    clips: Option<Res<Assets<AnimationClip>>>,
    library: Res<PlayerAnimationLibrary>,
    character_query: Query<
        (
            &NetworkCharacterChoice,
            Option<&NetworkAvatar>,
            Option<&crate::net::NetworkPlayerId>,
        ),
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
    let snapshot = game_state.as_ref().and_then(|g| g.sandbox.as_ref());
    let paused = snapshot.is_some_and(|s| s.config.environment.paused);
    let speed = snapshot.map_or(1.0, |s| s.config.environment.time_scale);
    if let Some(readout) = &mut readout {
        readout.0.clear();
        for (choice, avatar, id) in &character_query {
            if let Some(id) = id {
                let available = library
                    .get_set(&avatar_key(choice.0, avatar))
                    .map(sandbox_available_animations)
                    .unwrap_or_default();
                readout.0.insert(
                    id.0,
                    (
                        if available.is_empty() {
                            "No skeletal clips available / model loading".into()
                        } else {
                            "Waiting for animation binding".into()
                        },
                        available,
                    ),
                );
            }
        }
    }
    for (mut animation_player, mut binding, mut graph_handle) in &mut animation_query {
        let Ok((owner_transform, stats, action)) = player_state_query.get(binding.owner) else {
            continue;
        };
        let action = action.copied().unwrap_or_default();
        let (desired_key, id) = character_query
            .get(binding.owner)
            .map(|(choice, avatar, id)| (avatar_key(choice.0, avatar), id.map(|id| id.0)))
            .unwrap_or_else(|_| (binding.key.clone(), None));
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
        let sim_time = game_state.as_ref().and_then(|g| {
            g.sandbox
                .as_ref()
                .map(|s| ((g.meta.server_epoch, g.meta.match_id), s.simulation_secs))
        });
        // Only a new authoritative paused timestamp can advance a paused pose.
        // Normal playback uses speed without changing Bevy Time or transport clocks.
        let step_delta = if paused && binding.sandbox_paused {
            match (binding.sandbox_time, sim_time) {
                (Some((previous_round, previous)), Some((round, current)))
                    if previous_round == round =>
                {
                    (current - previous).max(0.0) as f32
                }
                _ => 0.0,
            }
        } else {
            0.0
        };
        binding.sandbox_time = sim_time;
        let simulation_delta = if paused {
            step_delta
        } else {
            time.delta_secs() * speed
        };
        let distance = owner_transform
            .translation
            .distance(binding.last_owner_position);
        let moved = distance / time.delta_secs().max(0.001) > 0.05;
        binding.last_owner_position = owner_transform.translation;
        if moved {
            binding.seconds_since_movement = 0.0;
        } else {
            binding.seconds_since_movement += simulation_delta;
        }
        let moved_recently = binding.seconds_since_movement < LOCOMOTION_IDLE_GRACE_SECS;
        let moving_by_intent = local_movement_query
            .get(binding.owner)
            .map(|(target, jumping)| target.is_some() || jumping.is_some())
            .unwrap_or(false);
        let preview = if snapshot.is_some() {
            sandbox
                .as_ref()
                .and_then(|s| s.preview.as_ref())
                .filter(|p| Some(p.id) == id)
        } else {
            None
        };
        let ended_preview = preview.is_none() && binding.sandbox_preview.is_some();
        if ended_preview {
            binding.playback = HeroAnimationPlayback::new(action.sequence);
        }
        let (restart, label) = if let Some(preview) = preview {
            let (state, label) = sandbox_preview_state(preview.kind, set);
            let restart = binding.sandbox_preview != Some(preview.sequence)
                || binding.playback.state != state;
            binding.playback.state = state;
            binding.sandbox_preview = Some(preview.sequence);
            (restart, label)
        } else {
            binding.sandbox_preview = None;
            let active = animation_player.animation(set.node(binding.playback.state));
            let duration = animation_clip_duration(
                set,
                binding.playback.state,
                graphs.as_deref(),
                clips.as_deref(),
            );
            let finished = active.is_none_or(|active| {
                active.is_finished()
                    || ((!paused || step_delta > 0.0)
                        && duration.is_some_and(|duration| active.seek_time() >= duration)
                        && active.repeat_mode() == bevy::animation::RepeatAnimation::Never)
            });
            let restart = binding.playback.advance(
                stats.is_alive(),
                moving_by_intent || moved || moved_recently,
                action,
                finished,
                |state| set.available(state),
            );
            (
                restart || ended_preview,
                if binding.playback.state == HeroAnimationState::Death
                    && !set.available(HeroAnimationState::Death)
                {
                    "Death → Idle (clip unavailable; frozen)".into()
                } else {
                    format!("{:?}", binding.playback.state)
                },
            )
        };
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
        let duration = animation_clip_duration(
            set,
            binding.playback.state,
            graphs.as_deref(),
            clips.as_deref(),
        );
        let fallback_frozen = binding.playback.state == HeroAnimationState::Death
            && !set.available(HeroAnimationState::Death);
        for (_, active) in animation_player.playing_animations_mut() {
            active.set_speed(speed);
            if paused || fallback_frozen {
                active.pause();
                if step_delta > 0.0 && !fallback_frozen {
                    let looping = active.repeat_mode() == bevy::animation::RepeatAnimation::Forever;
                    active.seek_to(sandbox_seek_time(
                        active.seek_time(),
                        step_delta,
                        duration,
                        looping,
                    ));
                }
            } else if binding.sandbox_paused {
                active.resume();
            }
        }
        binding.sandbox_paused = paused;
        if let (Some(id), Some(readout)) = (id, readout.as_mut()) {
            readout
                .0
                .insert(id, (label, sandbox_available_animations(set)));
        }
    }
}
