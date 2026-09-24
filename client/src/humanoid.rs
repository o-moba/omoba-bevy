//! Engine-owned humanoid motion, independent of a model's embedded clips.
//!
//! VRM metadata supplies semantic joints; skin assets identify instantiated
//! entities. Bone names are never used for binding. Runtime clips preserve each
//! rig's local translations and never animate the authoritative player root.
mod binding;
mod embedded;
mod motion;
mod retarget;

#[cfg(feature = "qa")]
pub(crate) use binding::RuntimeHumanoidBindingError;
pub(crate) use binding::{RuntimeHumanoidPlayer, RuntimeHumanoidRequest, bind_runtime_humanoids};
pub(crate) use motion::SharedHumanoidMotion;

use bevy::{animation::AnimationTargetId, gltf::Gltf, prelude::*};
use omoba_passport::humanoid::HumanoidRig;
use std::collections::HashMap;

/// All states use one target namespace, including models without any clips.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeHumanoidClips {
    pub idle: Handle<AnimationClip>,
    pub walk: Handle<AnimationClip>,
    pub run: Handle<AnimationClip>,
    pub attack: Handle<AnimationClip>,
    pub cast: Handle<AnimationClip>,
    pub death: Handle<AnimationClip>,
}

struct RuntimeRig {
    clips: RuntimeHumanoidClips,
    rig: HumanoidRig,
    animated_nodes: Vec<usize>,
}

/// Cache identity is the loaded model asset, never a mutable display name/slug.
#[derive(Resource)]
pub(crate) struct HumanoidRuntimeLibrary {
    motion: Result<SharedHumanoidMotion, String>,
    models: HashMap<AssetId<Gltf>, RuntimeRig>,
}

impl Default for HumanoidRuntimeLibrary {
    fn default() -> Self {
        Self {
            motion: SharedHumanoidMotion::parse(include_str!(
                "../assets/animations/humanoid-motion-v1.json"
            )),
            models: HashMap::new(),
        }
    }
}

impl HumanoidRuntimeLibrary {
    pub(crate) fn ensure(
        &mut self,
        model: &Handle<Gltf>,
        gltf: &Gltf,
        clips: &mut Assets<AnimationClip>,
    ) -> Result<RuntimeHumanoidClips, String> {
        if let Some(cached) = self.models.get(&model.id()) {
            return Ok(cached.clips.clone());
        }
        let source = gltf
            .source
            .as_ref()
            .ok_or("Humanoid metadata is unavailable; load GLTF with include_source")?;
        let document = serde_json::to_value(source.document.as_json())
            .map_err(|error| format!("Cannot read humanoid metadata: {error}"))?;
        let rig = HumanoidRig::from_document(&document)?;
        let motion = self.motion.as_ref().map_err(Clone::clone)?;
        let (retargeted, animated_nodes) = retarget::retarget_all(&rig, motion)?;
        let mut handles: HashMap<String, Handle<AnimationClip>> = retargeted
            .into_iter()
            .map(|(name, clip)| (name, clips.add(clip)))
            .collect();
        // retarget_all validates all six states before adding any engine asset.
        let mut take = |name: &str| handles.remove(name).expect("validated shared motion state");
        let result = RuntimeHumanoidClips {
            idle: take("idle"),
            walk: take("walk"),
            run: take("run"),
            attack: take("attack"),
            cast: take("cast"),
            death: take("death"),
        };
        self.models.insert(
            model.id(),
            RuntimeRig {
                clips: result.clone(),
                rig,
                animated_nodes,
            },
        );
        Ok(result)
    }

    /// Preserve an explicitly selected source GLTF clip when the scene is now
    /// driven by runtime target IDs. Unsupported channels fail as a whole.
    pub(crate) fn remap_embedded_clip(
        &self,
        model: &Handle<Gltf>,
        gltf: &Gltf,
        clip: &AnimationClip,
    ) -> Result<AnimationClip, String> {
        let cached = self
            .models
            .get(&model.id())
            .ok_or("Humanoid motion is not prepared")?;
        let source = gltf
            .source
            .as_ref()
            .ok_or("Embedded alias requires GLTF source metadata")?;
        let names: Vec<_> = source
            .nodes()
            .map(|node| {
                node.name()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("GltfNode{}", node.index()))
            })
            .collect();
        embedded::remap(&cached.rig, &cached.animated_nodes, &names, clip)
    }

    #[cfg(feature = "qa")]
    pub(crate) fn semantic_nodes(
        &self,
        model: &Handle<Gltf>,
    ) -> Option<&std::collections::BTreeMap<String, usize>> {
        self.models.get(&model.id()).map(|entry| &entry.rig.bones)
    }
}

/// Stable within a clip; AnimatedBy selects the individual scene instance.
pub(crate) fn humanoid_target_id(node: usize) -> AnimationTargetId {
    AnimationTargetId::from_name(&Name::new(format!("omoba/humanoid-v1/node/{node}")))
}

#[cfg(test)]
mod tests;
