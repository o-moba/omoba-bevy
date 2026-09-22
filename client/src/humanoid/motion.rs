use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) const STATES: [&str; 6] = ["idle", "walk", "run", "attack", "cast", "death"];

#[derive(Debug, Deserialize)]
pub(crate) struct SharedHumanoidMotion {
    pub schema_version: u32,
    pub source_hips_height: f32,
    pub source_hips_to_feet_distance: f32,
    pub source_reference_facing: [f32; 3],
    pub bones: Vec<String>,
    pub clips: BTreeMap<String, MotionClip>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MotionClip {
    pub source_clip: String,
    pub duration: f32,
    pub looping: bool,
    pub times: Vec<f32>,
    pub world_rotation_deltas: BTreeMap<String, Vec<[f32; 4]>>,
    pub hips_world_deltas: Vec<[f32; 3]>,
}

impl SharedHumanoidMotion {
    pub(crate) fn parse(json: &str) -> Result<Self, String> {
        let motion: Self = serde_json::from_str(json)
            .map_err(|error| format!("Invalid shared humanoid motion JSON: {error}"))?;
        motion.validate()?;
        Ok(motion)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("Unsupported shared humanoid motion version".into());
        }
        if !self.source_hips_height.is_finite()
            || self.source_hips_height <= 0.001
            || !self.source_hips_to_feet_distance.is_finite()
            || self.source_hips_to_feet_distance <= 0.001
        {
            return Err("Shared motion hips height must be positive".into());
        }
        let facing = bevy::math::Vec3::from_array(self.source_reference_facing);
        if !facing.is_finite() || facing.length_squared() < 0.5 {
            return Err("Shared motion reference facing is invalid".into());
        }
        if self.bones.is_empty() || self.bones.len() > 128 {
            return Err("Shared motion bone list is empty or too large".into());
        }
        for name in STATES {
            let clip = self
                .clips
                .get(name)
                .ok_or_else(|| format!("Shared humanoid motion lacks {name}"))?;
            let count = clip.times.len();
            if clip.source_clip.is_empty()
                || !clip.duration.is_finite()
                || clip.duration <= 0.0
                || !(2..=4096).contains(&count)
                || clip.times[0] != 0.0
                || (clip.times[count - 1] - clip.duration).abs() > 0.0001
                || clip.times.iter().any(|time| !time.is_finite())
                || clip.times.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(format!("Shared motion {name} has invalid sample times"));
            }
            if clip.hips_world_deltas.len() != count
                || clip
                    .hips_world_deltas
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite())
            {
                return Err(format!("Shared motion {name} has invalid hips samples"));
            }
            for bone in &self.bones {
                let samples = clip
                    .world_rotation_deltas
                    .get(bone)
                    .ok_or_else(|| format!("Shared motion {name} lacks {bone}"))?;
                if samples.len() != count
                    || samples.iter().any(|sample| {
                        let rotation = bevy::math::Quat::from_array(*sample);
                        !rotation.is_finite() || (rotation.length_squared() - 1.0).abs() > 0.001
                    })
                {
                    return Err(format!("Shared motion {name}/{bone} has invalid rotations"));
                }
            }
            if matches!(name, "idle" | "walk" | "run") && !clip.looping {
                return Err(format!("Shared locomotion {name} must loop"));
            }
        }
        Ok(())
    }
}
