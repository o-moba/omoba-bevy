//! Anatomical palm frames derived once from immutable rest-pose metadata.
use bevy::prelude::*;
use omoba_passport::humanoid::HumanoidRig;

pub(super) fn grip_frame(rig: &HumanoidRig, hand: &str) -> Option<Transform> {
    let mut rest = vec![GlobalTransform::IDENTITY; rig.nodes.len()];
    for &i in &rig.topological_order {
        let n = &rig.nodes[i];
        let t = Transform {
            translation: Vec3::from_array(n.translation),
            rotation: Quat::from_array(n.rotation),
            scale: Vec3::from_array(n.scale),
        };
        rest[i] = n
            .parent
            .map_or_else(|| GlobalTransform::from(t), |p| rest[p].mul_transform(t));
    }
    let h = *rig.bones.get(hand)?;
    let prefix = hand.strip_suffix("Hand")?;
    let point = |name: &str| rig.bones.get(name).map(|&n| rest[n].translation());
    let wrist = rest[h].translation();
    let elbow = point(&format!("{prefix}LowerArm"))?;
    let middle = point(&format!("{prefix}MiddleProximal"));
    let fingers = (middle.unwrap_or(wrist + (wrist - elbow) * 0.3) - wrist).try_normalize()?;
    let up = (point("head")? - point("hips")?).try_normalize()?;
    let across = point("leftHand")? - point("rightHand")?;
    let forward = across.cross(up).try_normalize()?;
    // +Y: shaft toward index/thumb side; +Z: out toward fingertips.
    let shaft = match (
        point(&format!("{prefix}IndexProximal")),
        point(&format!("{prefix}LittleProximal")),
    ) {
        (Some(index), Some(little)) => index - little,
        _ => forward,
    };
    let y = (shaft - fingers * shaft.dot(fingers)).try_normalize()?;
    let x = y.cross(fingers).try_normalize()?;
    let z = x.cross(y).normalize();
    let rotation = Quat::from_mat3(&Mat3::from_cols(x, y, z));
    let palm = middle.map_or(wrist + fingers * (wrist - elbow).length() * 0.12, |p| {
        wrist.lerp(p, 0.55)
    });
    let (scale, hand_rotation, _) = rest[h].to_scale_rotation_translation();
    if !scale.x.is_finite() || scale.x <= 0. {
        return None;
    }
    Some(Transform {
        translation: rest[h].affine().inverse().transform_point3(palm),
        rotation: hand_rotation.inverse() * rotation,
        // Props authored in metres for a .25 m forearm. Cancels source units;
        // model normalization is inherited naturally through the bone hierarchy.
        scale: Vec3::splat((wrist - elbow).length() / 0.25 / scale.x),
    })
}
