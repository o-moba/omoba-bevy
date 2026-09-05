//! Minion model animations (TASK-24).
//!
//! The lane minions render as VRM-staged slime GLBs (see
//! `client/assets/minions/`) with retargeted UAL clips embedded. This module
//! builds per-team walk/attack/idle animation graphs once the GLTFs load,
//! binds every minion scene's `AnimationPlayer` to its minion root, and
//! switches clips from the replicated minion AI state (marching/chasing →
//! walk, attacking → attack, otherwise idle). Same wiring pattern as the
//! raid-boss animation library in `bosses.rs`.

use bevy::gltf::Gltf;
use bevy::prelude::*;

use crate::net::{MinionBrainState, NetworkMinion, NetworkMinionBrainState, NetworkVisualAssets};
use crate::team::Team;

pub struct MinionVisualsPlugin;

impl Plugin for MinionVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinionAnimationLibrary>().add_systems(
            Update,
            (
                setup_minion_animation_library,
                bind_minion_animation_players,
                sync_minion_animation_state,
            ),
        );
    }
}

struct MinionAnimationSet {
    graph: Handle<AnimationGraph>,
    idle_node: AnimationNodeIndex,
    walk_node: AnimationNodeIndex,
    attack_node: AnimationNodeIndex,
}

/// Per-team animation graphs, built lazily once the slime GLTFs load.
#[derive(Resource, Default)]
struct MinionAnimationLibrary {
    evaluated: Vec<Team>,
    sets: Vec<(Team, MinionAnimationSet)>,
}

impl MinionAnimationLibrary {
    fn set_for(&self, team: Team) -> Option<&MinionAnimationSet> {
        self.sets
            .iter()
            .find(|(set_team, _)| *set_team == team)
            .map(|(_, set)| set)
    }
}

/// Marks a bound `AnimationPlayer` under a minion scene.
#[derive(Component)]
struct MinionAnimationBinding {
    owner: Entity,
    team: Team,
    state: MinionBrainState,
}

fn setup_minion_animation_library(
    visuals: Option<Res<NetworkVisualAssets>>,
    mut library: ResMut<MinionAnimationLibrary>,
    gltf_assets: Res<Assets<Gltf>>,
    mut animation_graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Some(visuals) = visuals else {
        return;
    };
    for (team, gltf_handle) in [
        (Team::Green, &visuals.green_minion_gltf),
        (Team::Blue, &visuals.blue_minion_gltf),
    ] {
        if library.evaluated.contains(&team) {
            continue;
        }
        let Some(gltf) = gltf_assets.get(gltf_handle) else {
            continue;
        };
        library.evaluated.push(team);

        let find_clip = |needle: &str| -> Option<Handle<AnimationClip>> {
            gltf.named_animations
                .iter()
                .find(|(name, _)| name.to_ascii_lowercase().contains(needle))
                .map(|(_, handle)| handle.clone())
        };
        let (Some(idle), Some(walk), Some(attack)) =
            (find_clip("idle"), find_clip("walk"), find_clip("attack"))
        else {
            warn!("idle/walk/attack clips not found for {team:?} minion model");
            continue;
        };
        let (graph, nodes) = AnimationGraph::from_clips([idle, walk, attack]);
        let (Some(idle_node), Some(walk_node), Some(attack_node)) = (
            nodes.first().copied(),
            nodes.get(1).copied(),
            nodes.get(2).copied(),
        ) else {
            continue;
        };
        library.sets.push((
            team,
            MinionAnimationSet {
                graph: animation_graphs.add(graph),
                idle_node,
                walk_node,
                attack_node,
            },
        ));
        info!("Minion animation set ready for {team:?} (idle/walk/attack)");
    }
}

/// Binds freshly spawned minion scene `AnimationPlayer`s and starts walk
/// (minions march from the moment they exist).
fn bind_minion_animation_players(
    mut commands: Commands,
    library: Res<MinionAnimationLibrary>,
    minion_roots: Query<&Team, With<NetworkMinion>>,
    child_of_query: Query<&ChildOf>,
    mut animation_players: Query<(Entity, &mut AnimationPlayer), Without<MinionAnimationBinding>>,
) {
    if library.sets.is_empty() {
        return;
    }
    for (animation_entity, mut animation_player) in &mut animation_players {
        let mut current = animation_entity;
        let owner = loop {
            if let Ok(team) = minion_roots.get(current) {
                break Some((current, *team));
            }
            let Ok(child_of) = child_of_query.get(current) else {
                break None;
            };
            current = child_of.parent();
        };
        let Some((owner, team)) = owner else {
            continue;
        };
        let Some(set) = library.set_for(team) else {
            continue;
        };
        animation_player.stop_all();
        animation_player.play(set.walk_node).repeat();
        commands.entity(animation_entity).insert((
            AnimationGraphHandle(set.graph.clone()),
            MinionAnimationBinding {
                owner,
                team,
                state: MinionBrainState::Marching,
            },
        ));
    }
}

/// Picks the clip from the replicated minion AI state.
fn sync_minion_animation_state(
    library: Res<MinionAnimationLibrary>,
    brain_states: Query<&NetworkMinionBrainState, With<NetworkMinion>>,
    mut animation_query: Query<(&mut AnimationPlayer, &mut MinionAnimationBinding)>,
) {
    for (mut animation_player, mut binding) in &mut animation_query {
        let Ok(brain) = brain_states.get(binding.owner) else {
            continue;
        };
        let Some(set) = library.set_for(binding.team) else {
            continue;
        };
        let desired = brain.0;
        if desired == binding.state {
            continue;
        }
        let node = match desired {
            MinionBrainState::Marching | MinionBrainState::Chasing => set.walk_node,
            MinionBrainState::Attacking => set.attack_node,
            MinionBrainState::Dead => set.idle_node,
        };
        animation_player.stop_all();
        animation_player.play(node).repeat();
        binding.state = desired;
    }
}
