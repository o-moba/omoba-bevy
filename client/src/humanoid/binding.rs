use super::{HumanoidRuntimeLibrary, humanoid_target_id};
use bevy::{
    animation::{AnimatedBy, AnimationTargetId},
    gltf::{Gltf, GltfNode, GltfSkin},
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use std::collections::HashMap;

/// Attach to the entity containing this model's SceneRoot. The loaded GLTF
/// handle is also the cache identity, so a replacement never reuses another rig.
#[derive(Component, Clone, Debug)]
pub(crate) struct RuntimeHumanoidRequest {
    pub model: Handle<Gltf>,
}

/// Only this player owns the runtime targets within its scene instance.
#[derive(Component, Clone, Debug)]
pub(crate) struct RuntimeHumanoidPlayer {
    pub model: AssetId<Gltf>,
    targets: HashMap<usize, Entity>,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct RuntimeHumanoidBindingError(pub String);

pub(crate) fn bind_runtime_humanoids(
    mut commands: Commands,
    library: Res<HumanoidRuntimeLibrary>,
    gltfs: Res<Assets<Gltf>>,
    skins: Res<Assets<GltfSkin>>,
    nodes: Res<Assets<GltfNode>>,
    requests: Query<(
        Entity,
        &RuntimeHumanoidRequest,
        Option<&RuntimeHumanoidPlayer>,
        Option<&RuntimeHumanoidBindingError>,
    )>,
    orphans: Query<Entity, (With<RuntimeHumanoidPlayer>, Without<RuntimeHumanoidRequest>)>,
    parents: Query<&ChildOf>,
    skin_instances: Query<(Entity, &SkinnedMesh)>,
    transforms: Query<(), With<Transform>>,
    animated_targets: Query<(Entity, &AnimatedBy, Option<&AnimationTargetId>)>,
    mut animation_players: Query<(Entity, &mut AnimationPlayer)>,
) {
    for root in &orphans {
        for (entity, owner, _) in &animated_targets {
            if owner.0 == root {
                commands
                    .entity(entity)
                    .remove::<(AnimatedBy, AnimationTargetId)>();
            }
        }
        if let Ok((_, mut player)) = animation_players.get_mut(root) {
            player.stop_all();
        }
        commands.entity(root).remove::<(
            RuntimeHumanoidPlayer,
            RuntimeHumanoidBindingError,
            AnimationPlayer,
            AnimationGraphHandle,
        )>();
    }
    for (root, request, bound, last_error) in &requests {
        let same_model = bound.is_some_and(|bound| bound.model == request.model.id());
        // SceneSpawner can refresh imported target components after our first
        // binding. The external root marker survives that refresh, so validate
        // the actual targets before considering the binding complete.
        if same_model
            && animation_players.get(root).is_ok()
            && bound.is_some_and(|bound| {
                bound.targets.iter().all(|(&node, &entity)| {
                    animated_targets.get(entity).is_ok_and(|(_, owner, id)| {
                        owner.0 == root && id == Some(&humanoid_target_id(node))
                    })
                })
            })
        {
            continue;
        }
        // Clear stale ownership before waiting for replacement assets. The old
        // scene cannot receive the new model's identically numbered targets.
        if bound.is_some() && !same_model {
            for (entity, owner, _) in &animated_targets {
                if owner.0 == root {
                    commands
                        .entity(entity)
                        .remove::<(AnimatedBy, AnimationTargetId)>();
                }
            }
            if let Ok((_, mut player)) = animation_players.get_mut(root) {
                player.stop_all();
            }
            commands
                .entity(root)
                .remove::<(RuntimeHumanoidPlayer, AnimationGraphHandle)>();
        }
        let Some(rig) = library.models.get(&request.model.id()) else {
            continue;
        };
        let Some(gltf) = gltfs.get(&request.model) else {
            continue;
        };
        let mut known_skins = HashMap::new();
        let mut waiting = false;
        for skin_handle in &gltf.skins {
            let Some(skin) = skins.get(skin_handle) else {
                waiting = true;
                break;
            };
            let indices: Option<Vec<_>> = skin
                .joints
                .iter()
                .map(|joint| nodes.get(joint).map(|node| node.index))
                .collect();
            let Some(indices) = indices else {
                waiting = true;
                break;
            };
            known_skins.insert(skin.inverse_bind_matrices.id(), indices);
        }
        if waiting {
            continue;
        }
        let instances: Vec<_> = skin_instances
            .iter()
            .filter(|(entity, _)| is_descendant(*entity, root, &parents))
            .map(|(_, skin)| (skin.inverse_bindposes.id(), skin.joints.clone()))
            .collect();
        // Scene instantiation is asynchronous. No partial target set is ever
        // installed; the next update retries until every mapped joint exists.
        if instances.is_empty() {
            continue;
        }
        let mapping = resolve_joint_entities(&known_skins, &instances, &rig.animated_nodes)
            .and_then(|mapping| {
                for (&node, &entity) in &mapping {
                    if !is_descendant(entity, root, &parents) || transforms.get(entity).is_err() {
                        return Err(format!(
                            "Humanoid node {node} resolves outside its model scene"
                        ));
                    }
                    if node >= rig.rig.nodes.len() {
                        return Err(format!("Humanoid node {node} exceeds metadata bounds"));
                    }
                }
                Ok(mapping)
            });
        let mapping = match mapping {
            Ok(mapping) => mapping,
            Err(error) => {
                if last_error.is_none_or(|previous| previous.0 != error) {
                    warn!("Runtime humanoid scene {:?}: {error}", request.model.id());
                    commands
                        .entity(root)
                        .insert(RuntimeHumanoidBindingError(error));
                }
                continue;
            }
        };
        // Source GLTF clips may have installed nested players/targets. Stop
        // those players before changing the namespace, otherwise they could
        // write onto the same pose during graph transitions.
        for (entity, mut player) in &mut animation_players {
            if entity != root && is_descendant(entity, root, &parents) {
                player.stop_all();
                commands.entity(entity).remove::<AnimationGraphHandle>();
            }
        }
        for (&node, &entity) in &mapping {
            commands
                .entity(entity)
                .insert((humanoid_target_id(node), AnimatedBy(root)));
        }
        commands
            .entity(root)
            .insert(RuntimeHumanoidPlayer {
                model: request.model.id(),
                targets: mapping,
            })
            .remove::<RuntimeHumanoidBindingError>();
        // Repairing target ownership must not restart an already active state
        // or discard the graph installed by the local/remote animation system.
        if !same_model || animation_players.get(root).is_err() {
            commands.entity(root).insert(AnimationPlayer::default());
        }
    }
}

fn is_descendant(mut entity: Entity, root: Entity, parents: &Query<&ChildOf>) -> bool {
    // The ECS hierarchy API prevents normal cycles. A bound also protects this
    // traversal from malformed externally constructed scene/test hierarchies.
    for _ in 0..2048 {
        if entity == root {
            return true;
        }
        let Ok(parent) = parents.get(entity) else {
            return false;
        };
        entity = parent.parent();
    }
    false
}

fn resolve_joint_entities(
    known_skins: &HashMap<AssetId<SkinnedMeshInverseBindposes>, Vec<usize>>,
    instances: &[(AssetId<SkinnedMeshInverseBindposes>, Vec<Entity>)],
    required: &[usize],
) -> Result<HashMap<usize, Entity>, String> {
    let mut result = HashMap::new();
    let mut entities = HashMap::new();
    for (skin, joints) in instances {
        let Some(indices) = known_skins.get(skin) else {
            continue;
        };
        if indices.len() != joints.len() {
            return Err("Humanoid skin joint count differs from the loaded model".into());
        }
        for (&node, &entity) in indices.iter().zip(joints) {
            if result.insert(node, entity).is_some_and(|old| old != entity) {
                return Err(format!(
                    "Humanoid node {node} maps to multiple scene instances"
                ));
            }
            if entities.insert(entity, node).is_some_and(|old| old != node) {
                return Err(format!(
                    "Humanoid entity {entity:?} maps to multiple node indices"
                ));
            }
        }
    }
    for node in required {
        if !result.contains_key(node) {
            return Err(format!(
                "Humanoid mapped node {node} is absent from the instantiated skins"
            ));
        }
    }
    result.retain(|node, _| required.contains(node));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_instances_and_missing_nodes_are_rejected_atomically() {
        let skin = AssetId::<SkinnedMeshInverseBindposes>::Uuid {
            uuid: bevy::asset::uuid::Uuid::from_u128(11),
        };
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let known = HashMap::from([(skin, vec![7])]);
        assert!(resolve_joint_entities(&known, &[(skin, vec![a]), (skin, vec![b])], &[7]).is_err());
        assert!(resolve_joint_entities(&known, &[(skin, vec![a])], &[8]).is_err());
        assert_eq!(
            resolve_joint_entities(&known, &[(skin, vec![a])], &[7]).unwrap()[&7],
            a
        );
    }
}
