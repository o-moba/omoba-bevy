use super::{
    SharedHumanoidMotion, humanoid_target_id,
    motion::{MotionClip, STATES},
};
use bevy::{animation::animated_field, math::curve::UnevenSampleAutoCurve, prelude::*};
use omoba_passport::humanoid::{HumanoidRig, VrmVersion};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct WorldRest {
    rotation: Quat,
    translation: Vec3,
    scale: f32,
}

struct RetargetBasis {
    rest: Vec<WorldRest>,
    alignment: Quat,
    hips_scale: f32,
    streams: BTreeMap<usize, String>,
    hips: usize,
}

impl RetargetBasis {
    fn new(rig: &HumanoidRig, motion: &SharedHumanoidMotion) -> Result<Self, String> {
        let mut rest = vec![
            WorldRest {
                rotation: Quat::IDENTITY,
                translation: Vec3::ZERO,
                scale: 1.0,
            };
            rig.nodes.len()
        ];
        for &index in &rig.topological_order {
            let node = &rig.nodes[index];
            let parent = node.parent.map(|parent| rest[parent]).unwrap_or(WorldRest {
                rotation: Quat::IDENTITY,
                translation: Vec3::ZERO,
                scale: 1.0,
            });
            rest[index] = WorldRest {
                rotation: (parent.rotation * Quat::from_array(node.rotation)).normalize(),
                translation: parent.translation
                    + parent.rotation * (Vec3::from_array(node.translation) * parent.scale),
                scale: parent.scale * node.scale[0],
            };
        }
        let point = |bone: &str| -> Result<Vec3, String> {
            rig.bones
                .get(bone)
                .map(|index| rest[*index].translation)
                .ok_or_else(|| format!("Humanoid lacks required motion bone {bone}"))
        };
        let hips = *rig.bones.get("hips").ok_or("Humanoid lacks hips")?;
        let up = (point("head")? - point("hips")?)
            .try_normalize()
            .ok_or("Humanoid head and hips cannot occupy the same rest position")?;
        let across = point("leftHand")? - point("rightHand")?;
        let side = (across - up * across.dot(up))
            .try_normalize()
            .ok_or("Humanoid hands cannot establish a rest facing")?;
        let forward = side.cross(up).normalize();
        let target_basis = Quat::from_mat3(&Mat3::from_cols(side, up, forward));
        let source_forward = Vec3::from_array(motion.source_reference_facing).normalize();
        let source_side = Vec3::Y
            .cross(source_forward)
            .try_normalize()
            .ok_or("Shared motion facing cannot be vertical")?;
        let source_up = source_forward.cross(source_side).normalize();
        let source_basis =
            Quat::from_mat3(&Mat3::from_cols(source_side, source_up, source_forward));
        let alignment = (target_basis * source_basis.inverse()).normalize();
        let feet = (point("leftFoot")? + point("rightFoot")?) * 0.5;
        let hips_height = (point("hips")? - feet).dot(up);
        if !hips_height.is_finite() || hips_height <= 0.0001 {
            return Err("Humanoid hips must stand above the feet in its rest pose".into());
        }
        let mut streams = BTreeMap::new();
        for (semantic, &node) in &rig.bones {
            let stream = source_semantic(rig, semantic);
            if motion.bones.contains(&stream) {
                streams.insert(node, stream);
            }
        }
        for required in [
            "hips",
            "leftUpperLeg",
            "leftLowerLeg",
            "leftFoot",
            "rightUpperLeg",
            "rightLowerLeg",
            "rightFoot",
        ] {
            if !rig
                .bones
                .get(required)
                .is_some_and(|node| streams.contains_key(node))
            {
                return Err(format!("Humanoid shared motion cannot drive {required}"));
            }
        }
        Ok(Self {
            rest,
            alignment,
            hips_scale: hips_height / motion.source_hips_to_feet_distance,
            streams,
            hips,
        })
    }

    fn frame(&self, rig: &HumanoidRig, clip: &MotionClip, sample: usize) -> (Vec<Quat>, Vec3) {
        let mut world_rotations = vec![Quat::IDENTITY; rig.nodes.len()];
        let mut local_rotations = vec![Quat::IDENTITY; rig.nodes.len()];
        for &index in &rig.topological_order {
            let node = &rig.nodes[index];
            let parent_rotation = node
                .parent
                .map(|parent| world_rotations[parent])
                .unwrap_or(Quat::IDENTITY);
            let local = if let Some(semantic) = self.streams.get(&index) {
                let delta = Quat::from_array(clip.world_rotation_deltas[semantic][sample]);
                let desired_world =
                    self.alignment * delta * self.alignment.inverse() * self.rest[index].rotation;
                (parent_rotation.inverse() * desired_world).normalize()
            } else {
                Quat::from_array(node.rotation)
            };
            local_rotations[index] = local;
            world_rotations[index] = (parent_rotation * local).normalize();
        }
        let node = &rig.nodes[self.hips];
        let parent = node
            .parent
            .map(|parent| self.rest[parent])
            .unwrap_or(WorldRest {
                rotation: Quat::IDENTITY,
                translation: Vec3::ZERO,
                scale: 1.0,
            });
        // Hips ancestors are outside the mapped humanoid chain. Preserve their
        // rest basis/scale; no horizontal motion ever reaches the player root.
        let delta =
            self.alignment * Vec3::from_array(clip.hips_world_deltas[sample]) * self.hips_scale;
        let translation =
            Vec3::from_array(node.translation) + parent.rotation.inverse() * delta / parent.scale;
        (local_rotations, translation)
    }
}

/// VRM1 renamed the thumb chain. A target without upperChest applies the
/// complete source chest-chain world delta to its sole chest joint.
fn source_semantic(rig: &HumanoidRig, semantic: &str) -> String {
    if semantic == "chest" && !rig.bones.contains_key("upperChest") {
        return "upperChest".into();
    }
    if rig.version == VrmVersion::Vrm1 {
        if semantic.ends_with("ThumbMetacarpal") {
            return semantic.replace("ThumbMetacarpal", "ThumbProximal");
        }
        if semantic.ends_with("ThumbProximal") {
            return semantic.replace("ThumbProximal", "ThumbIntermediate");
        }
    }
    semantic.to_owned()
}

pub(super) fn retarget_all(
    rig: &HumanoidRig,
    motion: &SharedHumanoidMotion,
) -> Result<(BTreeMap<String, AnimationClip>, Vec<usize>), String> {
    let basis = RetargetBasis::new(rig, motion)?;
    let mut clips = BTreeMap::new();
    for name in STATES {
        let source = &motion.clips[name];
        let frames: Vec<_> = (0..source.times.len())
            .map(|sample| basis.frame(rig, source, sample))
            .collect();
        let mut clip = AnimationClip::default();
        for &node in basis.streams.keys() {
            let mut last = Quat::IDENTITY;
            let samples = source
                .times
                .iter()
                .copied()
                .zip(frames.iter().map(|(rotations, _)| {
                    let mut rotation = rotations[node];
                    if last.dot(rotation) < 0.0 {
                        rotation = -rotation;
                    }
                    last = rotation;
                    rotation
                }));
            let curve = UnevenSampleAutoCurve::new(samples)
                .map_err(|error| format!("Cannot build {name} rotation curve: {error:?}"))?;
            clip.add_curve_to_target(
                humanoid_target_id(node),
                AnimatableCurve::new(animated_field!(Transform::rotation), curve),
            );
        }
        let curve = UnevenSampleAutoCurve::new(
            source
                .times
                .iter()
                .copied()
                .zip(frames.iter().map(|(_, translation)| *translation)),
        )
        .map_err(|error| format!("Cannot build {name} hips curve: {error:?}"))?;
        clip.add_curve_to_target(
            humanoid_target_id(basis.hips),
            AnimatableCurve::new(animated_field!(Transform::translation), curve),
        );
        clip.set_duration(source.duration);
        clips.insert(name.into(), clip);
    }
    Ok((clips, basis.streams.keys().copied().collect()))
}

#[cfg(test)]
pub(super) fn sample_frame(
    rig: &HumanoidRig,
    motion: &SharedHumanoidMotion,
    clip: &str,
    sample: usize,
) -> Result<(Vec<Quat>, Vec3), String> {
    Ok(RetargetBasis::new(rig, motion)?.frame(rig, &motion.clips[clip], sample))
}
