//! Validated skeletal capability for local, clipless VRM rendering.
//!
//! This is deliberately separate from `validate_humanoid_profile`: accepting a
//! rig here does not approve a Studio rendition or change the published five-clip
//! `humanoid-glb-v1` admission contract. The supported subset is self-contained
//! GLB, VRM 0/1, TRS nodes with positive uniform scales, and mapped humanoid bones
//! present in the skins instantiated by scene 0. Materials, springs, expressions
//! and constraints are not implemented by this skeletal adapter. Required glTF
//! extensions are restricted to the verified renderer subset: KHR_materials_unlit
//! and KHR_texture_transform. VRM metadata may be optional extension data, but a
//! required VRM/MToon/spring/constraint extension needs a future loader adapter;
//! this validator never removes required semantics or rewrites source bytes.

use serde_json::Value;
use std::collections::{BTreeMap, HashSet, VecDeque};

pub const MAX_HUMANOID_NODES: usize = 4096;
pub const MAX_HUMANOID_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_HUMANOID_GLB_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VrmVersion {
    Vrm0,
    Vrm1,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RigNode {
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    /// A normalized quaternion in glTF XYZW order.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct HumanoidRig {
    pub version: VrmVersion,
    pub nodes: Vec<RigNode>,
    /// Semantic names from the declared VRM version, independent of glTF node names.
    /// The retargeter adapts VRM1 thumb names to its shared motion convention.
    pub bones: BTreeMap<String, usize>,
    /// The current avatar renderer instantiates `Scene0`.
    pub scene_roots: Vec<usize>,
    pub topological_order: Vec<usize>,
}

/// Check local runtime skeletal compatibility without requiring baked clips.
/// This grants neither project approval nor permission to wear a purchased skin.
pub fn validate_runtime_humanoid(bytes: &[u8]) -> Result<HumanoidRig, String> {
    HumanoidRig::from_glb(bytes)
}

impl HumanoidRig {
    pub fn from_glb(bytes: &[u8]) -> Result<Self, String> {
        if !(20..=MAX_HUMANOID_GLB_BYTES).contains(&bytes.len()) {
            return Err("VRM GLB must be between 20 bytes and 50 MiB".into());
        }
        let u32_at = |offset: usize| {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
        };
        if &bytes[..4] != b"glTF" || u32_at(4) != 2 || u32_at(8) != bytes.len() {
            return Err("VRM must have an exact-length glTF 2.0 binary container".into());
        }
        let mut offset = 12;
        let mut document = None;
        let mut binary_length = None;
        while offset < bytes.len() {
            if bytes.len() - offset < 8 {
                return Err("VRM GLB contains a truncated chunk header".into());
            }
            let length = u32_at(offset);
            let kind = &bytes[offset + 4..offset + 8];
            offset += 8;
            if length % 4 != 0 || length > bytes.len() - offset {
                return Err("VRM GLB contains an invalid chunk length".into());
            }
            match kind {
                b"JSON" if document.is_none() && binary_length.is_none() => {
                    if length > MAX_HUMANOID_JSON_BYTES {
                        return Err("VRM JSON exceeds the 4 MiB runtime limit".into());
                    }
                    document = Some(
                        serde_json::from_slice::<Value>(&bytes[offset..offset + length])
                            .map_err(|_| "VRM GLB contains invalid JSON")?,
                    );
                }
                b"BIN\0" if document.is_some() && binary_length.is_none() => {
                    binary_length = Some(length);
                }
                _ => {
                    return Err(
                        "VRM GLB requires one JSON chunk followed by at most one BIN chunk".into(),
                    );
                }
            }
            offset += length;
        }
        let document = document.ok_or("VRM GLB has no JSON chunk")?;
        let buffers = document["buffers"]
            .as_array()
            .ok_or("VRM GLB has no embedded buffer")?;
        if buffers.len() != 1 {
            return Err("VRM GLB must contain exactly one embedded buffer".into());
        }
        let declared = buffers[0]["byteLength"]
            .as_u64()
            .ok_or("VRM buffer has no valid byteLength")?;
        let actual = binary_length.ok_or("VRM GLB has no BIN chunk")? as u64;
        if declared > actual || actual - declared > 3 {
            return Err("VRM embedded buffer length does not match its BIN chunk".into());
        }
        Self::from_document(&document)
    }

    /// Parse metadata already loaded by Bevy's GLTF loader. Container/accessor
    /// validation remains the GLTF loader's responsibility for this entry point.
    pub fn from_document(document: &Value) -> Result<Self, String> {
        if document["asset"]["version"].as_str() != Some("2.0") {
            return Err("Humanoid runtime requires glTF 2.0".into());
        }
        validate_required_extensions(document)?;
        for field in ["buffers", "images"] {
            if let Some(entries) = document.get(field) {
                let entries = entries
                    .as_array()
                    .ok_or_else(|| format!("VRM {field} must be an array"))?;
                if entries.iter().any(|entry| entry.get("uri").is_some()) {
                    return Err(format!(
                        "VRM {field} must be embedded; external/data URIs are unsupported"
                    ));
                }
            }
        }
        let raw_nodes = document["nodes"]
            .as_array()
            .ok_or("VRM has no skeleton nodes")?;
        if raw_nodes.is_empty() || raw_nodes.len() > MAX_HUMANOID_NODES {
            return Err(format!("VRM requires 1..={MAX_HUMANOID_NODES} nodes"));
        }
        let node_count = raw_nodes.len();
        let mut nodes = Vec::with_capacity(node_count);
        let mut children = vec![Vec::new(); node_count];
        let mut parents = vec![None; node_count];
        for (index, node) in raw_nodes.iter().enumerate() {
            if !node.is_object() {
                return Err(format!("VRM node {index} must be an object"));
            }
            if node.get("matrix").is_some() {
                return Err(format!(
                    "VRM node {index} uses a matrix; export explicit TRS transforms"
                ));
            }
            let translation = vector(node, "translation", [0.0; 3], index)?;
            if translation.iter().any(|value| value.abs() > 1_000_000.0) {
                return Err(format!(
                    "VRM node {index} translation exceeds the supported range"
                ));
            }
            let mut rotation = vector(node, "rotation", [0.0, 0.0, 0.0, 1.0], index)?;
            let length = rotation
                .iter()
                .map(|v| (*v as f64).powi(2))
                .sum::<f64>()
                .sqrt();
            if length < 1e-8 {
                return Err(format!("VRM node {index} has a zero rotation quaternion"));
            }
            // Bevy instantiates the original immutable glTF transforms. Fixing an
            // arbitrary quaternion only in this descriptor would leave the real
            // scene distorted or nonfinite; accept floating-point roundoff only.
            if (length - 1.0).abs() > 1e-4 {
                return Err(format!(
                    "VRM node {index} requires a unit rotation quaternion; normalize the source model before importing"
                ));
            }
            for component in &mut rotation {
                *component = (*component as f64 / length) as f32;
            }
            let scale = vector(node, "scale", [1.0; 3], index)?;
            if scale.iter().any(|v| !(1e-6..=1e6).contains(v))
                || scale
                    .iter()
                    .any(|v| (*v - scale[0]).abs() > scale[0] * 1e-5)
            {
                return Err(format!(
                    "VRM node {index} requires positive uniform scale in 0.000001..=1000000"
                ));
            }
            if let Some(raw_children) = node.get("children") {
                let raw_children = raw_children
                    .as_array()
                    .ok_or_else(|| format!("VRM node {index} children must be an array"))?;
                if raw_children.len() > node_count {
                    return Err(format!("VRM node {index} has too many children"));
                }
                for child in raw_children {
                    let child = node_index(child, node_count, "child")?;
                    if parents[child].replace(index).is_some() {
                        return Err(format!(
                            "VRM node {child} has multiple parents or duplicate child references"
                        ));
                    }
                    children[index].push(child);
                }
            }
            nodes.push(RigNode {
                parent: None,
                translation,
                rotation,
                scale,
            });
        }
        for (node, parent) in nodes.iter_mut().zip(&parents) {
            node.parent = *parent;
        }
        let mut queue: VecDeque<_> = parents
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.is_none().then_some(i))
            .collect();
        let mut topological_order = Vec::with_capacity(node_count);
        let mut world_scales = vec![1.0_f64; node_count];
        while let Some(node) = queue.pop_front() {
            topological_order.push(node);
            let parent_scale = parents[node].map_or(1.0, |parent| world_scales[parent]);
            world_scales[node] = parent_scale * f64::from(nodes[node].scale[0]);
            if !(1e-12..=1e12).contains(&world_scales[node]) {
                return Err(format!(
                    "VRM node {node} accumulated scale exceeds the supported range"
                ));
            }
            queue.extend(children[node].iter().copied());
        }
        if topological_order.len() != node_count {
            return Err("VRM node hierarchy contains a cycle".into());
        }
        let raw_roots = document["scenes"][0]["nodes"]
            .as_array()
            .ok_or("VRM scene 0 has no root nodes")?;
        if raw_roots.is_empty() || raw_roots.len() > node_count {
            return Err("VRM scene 0 has an invalid root count".into());
        }
        let mut scene_roots = Vec::with_capacity(raw_roots.len());
        let mut reachable = HashSet::new();
        for root in raw_roots {
            let root = node_index(root, node_count, "scene root")?;
            if parents[root].is_some() || scene_roots.contains(&root) {
                return Err("VRM scene 0 contains duplicate or non-root nodes".into());
            }
            scene_roots.push(root);
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                reachable.insert(node);
                stack.extend(children[node].iter().copied());
            }
        }
        let (version, bones) = parse_bones(document, node_count)?;
        for bone in required_bones(version) {
            if !bones.contains_key(bone) {
                return Err(format!("VRM humanoid map is missing required bone {bone}"));
            }
        }
        let skins = document["skins"]
            .as_array()
            .filter(|skins| !skins.is_empty())
            .ok_or("VRM runtime requires a skinned humanoid mesh")?;
        if skins.len() > node_count {
            return Err("VRM has too many skins".into());
        }
        let mut skin_joints = Vec::with_capacity(skins.len());
        for (index, skin) in skins.iter().enumerate() {
            let raw_joints = skin["joints"]
                .as_array()
                .filter(|joints| !joints.is_empty())
                .ok_or_else(|| format!("VRM skin {index} has no joints"))?;
            if raw_joints.len() > node_count {
                return Err(format!("VRM skin {index} has too many joints"));
            }
            let mut joints = HashSet::new();
            for joint in raw_joints {
                let joint = node_index(joint, node_count, "skin joint")?;
                if !joints.insert(joint) {
                    return Err(format!("VRM skin {index} contains duplicate joints"));
                }
            }
            skin_joints.push(joints);
        }
        let mut bound_joints: HashSet<usize> = HashSet::new();
        let mesh_count = document["meshes"].as_array().map_or(0, Vec::len);
        for index in &reachable {
            if let Some(skin) = raw_nodes[*index].get("skin") {
                let skin = node_index(skin, skins.len(), "skin")?;
                node_index(&raw_nodes[*index]["mesh"], mesh_count, "skinned mesh")?;
                bound_joints.extend(skin_joints[skin].iter().copied());
            }
        }
        for (bone, node) in &bones {
            if !reachable.contains(node) {
                return Err(format!("VRM bone {bone} is not instantiated in scene 0"));
            }
            if !bound_joints.contains(node) {
                return Err(format!(
                    "VRM bone {bone} is not a skin joint in scene 0; unweighted humanoid nodes are unsupported"
                ));
            }
        }
        validate_bone_hierarchy(&bones, &parents, version)?;
        Ok(Self {
            version,
            nodes,
            bones,
            scene_roots,
            topological_order,
        })
    }
}

/// Keep this conservative list aligned with the pinned Bevy GLTF loader and its
/// enabled gltf-json validators. Understanding a humanoid map alone does not make
/// a required extension safe to ignore during rendering.
fn validate_required_extensions(document: &Value) -> Result<(), String> {
    let Some(required) = document.get("extensionsRequired") else {
        return Ok(());
    };
    let required = required
        .as_array()
        .ok_or("VRM extensionsRequired must be an array of extension names")?;
    for extension in required {
        let extension = extension
            .as_str()
            .ok_or("VRM extensionsRequired must contain extension names")?;
        if !matches!(extension, "KHR_materials_unlit" | "KHR_texture_transform") {
            return Err(format!(
                "Required extension {extension} is unsupported by the current renderer; use a compatible export or a renderer adapter that implements this extension"
            ));
        }
    }
    Ok(())
}

fn vector<const N: usize>(
    node: &Value,
    field: &str,
    default: [f32; N],
    index: usize,
) -> Result<[f32; N], String> {
    let Some(value) = node.get(field) else {
        return Ok(default);
    };
    let raw = value
        .as_array()
        .filter(|array| array.len() == N)
        .ok_or_else(|| format!("VRM node {index} {field} requires {N} components"))?;
    let mut result = default;
    for (target, value) in result.iter_mut().zip(raw) {
        let value = value
            .as_f64()
            .ok_or_else(|| format!("VRM node {index} {field} must be numeric"))?
            as f32;
        if !value.is_finite() {
            return Err(format!("VRM node {index} {field} must be finite"));
        }
        *target = value;
    }
    Ok(result)
}

fn node_index(value: &Value, count: usize, context: &str) -> Result<usize, String> {
    value
        .as_u64()
        .filter(|index| *index < count as u64)
        .map(|index| index as usize)
        .ok_or_else(|| format!("VRM {context} index is missing or out of range"))
}

fn required_bones(version: VrmVersion) -> Vec<&'static str> {
    let mut bones = vec![
        "hips",
        "spine",
        "head",
        "leftUpperArm",
        "leftLowerArm",
        "leftHand",
        "rightUpperArm",
        "rightLowerArm",
        "rightHand",
        "leftUpperLeg",
        "leftLowerLeg",
        "leftFoot",
        "rightUpperLeg",
        "rightLowerLeg",
        "rightFoot",
    ];
    if version == VrmVersion::Vrm0 {
        bones.extend(["chest", "neck"]);
    }
    bones
}

fn parse_bones(
    document: &Value,
    node_count: usize,
) -> Result<(VrmVersion, BTreeMap<String, usize>), String> {
    let vrm0 = document["extensions"].get("VRM");
    let vrm1 = document["extensions"].get("VRMC_vrm");
    let (version, raw): (_, Vec<(&str, &Value)>) = match (vrm0, vrm1) {
        (Some(extension), None) => {
            if extension["specVersion"]
                .as_str()
                .is_none_or(|value| !value.starts_with("0."))
            {
                return Err("Unsupported VRM 0.x specVersion".into());
            }
            let bones = extension["humanoid"]["humanBones"]
                .as_array()
                .ok_or("VRM0 humanBones must be an array")?;
            if bones.len() > 64 {
                return Err("VRM humanoid map exceeds 64 bones".into());
            }
            let entries = bones
                .iter()
                .map(|bone| {
                    Ok((
                        bone["bone"]
                            .as_str()
                            .ok_or("VRM0 bone has no semantic name")?,
                        &bone["node"],
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            (VrmVersion::Vrm0, entries)
        }
        (None, Some(extension)) => {
            if extension["specVersion"].as_str() != Some("1.0") {
                return Err("Unsupported VRM1 specVersion; expected 1.0".into());
            }
            let bones = extension["humanoid"]["humanBones"]
                .as_object()
                .ok_or("VRM1 humanBones must be an object")?;
            if bones.len() > 64 {
                return Err("VRM humanoid map exceeds 64 bones".into());
            }
            (
                VrmVersion::Vrm1,
                bones
                    .iter()
                    .map(|(name, value)| (name.as_str(), &value["node"]))
                    .collect(),
            )
        }
        (None, None) => return Err("Model has no VRM0 or VRM1 humanoid metadata".into()),
        (Some(_), Some(_)) => {
            return Err("Model declares conflicting VRM0 and VRM1 extensions".into());
        }
    };
    let mut bones = BTreeMap::new();
    let mut indices = HashSet::new();
    for (raw_name, raw_index) in raw {
        let name = raw_name.to_owned();
        if semantic_parent(&name, version).is_none() && name != "hips" {
            return Err(format!("Unsupported humanoid bone {raw_name}"));
        }
        let index = node_index(raw_index, node_count, "humanoid bone")?;
        if bones.insert(name.clone(), index).is_some() || !indices.insert(index) {
            return Err(format!("VRM humanoid assignment {name} is duplicated"));
        }
    }
    Ok((version, bones))
}

fn semantic_parent(bone: &str, version: VrmVersion) -> Option<String> {
    let parent = match bone {
        "spine" => "hips",
        "chest" => "spine",
        "upperChest" => "chest",
        "neck" => "upperChest",
        "head" => "neck",
        "leftEye" | "rightEye" | "jaw" => "head",
        _ => {
            let (side, part) = ["left", "right"]
                .into_iter()
                .find_map(|side| bone.strip_prefix(side).map(|part| (side, part)))?;
            let part_parent = match part {
                "Shoulder" => return Some("upperChest".into()),
                "UpperArm" => "Shoulder",
                "LowerArm" => "UpperArm",
                "Hand" => "LowerArm",
                "UpperLeg" => return Some("hips".into()),
                "LowerLeg" => "UpperLeg",
                "Foot" => "LowerLeg",
                "Toes" => "Foot",
                _ => {
                    for finger in ["Thumb", "Index", "Middle", "Ring", "Little"] {
                        let Some(segment) = part.strip_prefix(finger) else {
                            continue;
                        };
                        if finger == "Thumb" && version == VrmVersion::Vrm1 {
                            return match segment {
                                "Metacarpal" => Some(format!("{side}Hand")),
                                "Proximal" => Some(format!("{side}ThumbMetacarpal")),
                                "Distal" => Some(format!("{side}ThumbProximal")),
                                _ => None,
                            };
                        }
                        return match segment {
                            "Proximal" => Some(format!("{side}Hand")),
                            "Intermediate" => Some(format!("{side}{finger}Proximal")),
                            "Distal" => Some(format!("{side}{finger}Intermediate")),
                            _ => None,
                        };
                    }
                    return None;
                }
            };
            return Some(format!("{side}{part_parent}"));
        }
    };
    Some(parent.into())
}

fn validate_bone_hierarchy(
    bones: &BTreeMap<String, usize>,
    parents: &[Option<usize>],
    version: VrmVersion,
) -> Result<(), String> {
    let reverse: BTreeMap<_, _> = bones
        .iter()
        .map(|(name, index)| (*index, name.as_str()))
        .collect();
    for (bone, index) in bones {
        let mut expected = semantic_parent(bone, version);
        while expected
            .as_ref()
            .is_some_and(|name| !bones.contains_key(name))
        {
            expected = expected
                .as_deref()
                .and_then(|name| semantic_parent(name, version));
        }
        let mut ancestor = parents[*index];
        while ancestor.is_some_and(|index| !reverse.contains_key(&index)) {
            ancestor = ancestor.and_then(|index| parents[index]);
        }
        let actual = ancestor.and_then(|index| reverse.get(&index).copied());
        if actual != expected.as_deref() {
            return Err(format!(
                "VRM bone {bone} has incompatible parent {:?}; expected {:?}",
                actual, expected
            ));
        }
        if bone == "upperChest" && !bones.contains_key("chest") {
            return Err("VRM upperChest requires chest".into());
        }
        if (bone.ends_with("Intermediate")
            || bone.ends_with("Distal")
            || (version == VrmVersion::Vrm1 && bone.ends_with("ThumbProximal")))
            && semantic_parent(bone, version).is_some_and(|parent| !bones.contains_key(&parent))
        {
            return Err(format!(
                "VRM finger bone {bone} requires its preceding segment"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
