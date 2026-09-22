//! Preserve explicitly configured GLTF animation aliases after target rebinding.
use super::humanoid_target_id;
use bevy::{animation::AnimationTargetId, prelude::*};
use omoba_passport::humanoid::HumanoidRig;
use std::collections::HashMap;

/// Bevy's GLTF loader creates target IDs from the scene-root-to-node name path,
/// using `GltfNode{index}` for unnamed nodes. Reconstruct that exact path from
/// validated metadata, then translate the curves into the runtime namespace.
/// These are source GLTF clips, which do not contain engine animation events.
pub(super) fn remap(
    rig: &HumanoidRig,
    animated_nodes: &[usize],
    names: &[String],
    clip: &AnimationClip,
) -> Result<AnimationClip, String> {
    if names.len() != rig.nodes.len() || clip.curves().is_empty() {
        return Err("Embedded humanoid alias has no supported skeletal curves".into());
    }
    let mut targets = HashMap::new();
    for &node in animated_nodes {
        let mut path = vec![names[node].as_str()];
        let mut cursor = node;
        while let Some(parent) = rig.nodes[cursor].parent {
            path.push(names[parent].as_str());
            cursor = parent;
        }
        if !rig.scene_roots.contains(&cursor) {
            return Err("Embedded humanoid alias targets a different scene".into());
        }
        path.reverse();
        let original: AnimationTargetId = path.into_iter().collect();
        if targets.insert(original, humanoid_target_id(node)).is_some() {
            return Err("Embedded humanoid alias has ambiguous duplicate bone name paths".into());
        }
    }
    // Validate the entire alias before replacing any channel; a partial clip
    // could silently lose a critical root, morph or accessory animation.
    if clip
        .curves()
        .keys()
        .any(|target| !targets.contains_key(target))
    {
        return Err(
            "Embedded humanoid alias targets an unsupported root, morph or non-humanoid node"
                .into(),
        );
    }
    let mut remapped = clip.clone();
    let original = std::mem::take(remapped.curves_mut());
    for (target, curves) in original {
        remapped.curves_mut().insert(targets[&target], curves);
    }
    Ok(remapped)
}
