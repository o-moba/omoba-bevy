//! User-controlled mixer levels shared by playback, settings and persistence.
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Resource, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AudioSettings {
    pub(crate) master: f32,
    pub(crate) music: f32,
    pub(crate) effects: f32,
    pub(crate) ui: f32,
    pub(crate) muted: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            master: 0.8,
            music: 0.25,
            effects: 0.7,
            ui: 0.6,
            muted: false,
        }
    }
}

impl AudioSettings {
    pub(crate) fn sanitized(self) -> Self {
        let defaults = Self::default();
        let level = |value: f32, fallback| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                fallback
            }
        };
        Self {
            master: level(self.master, defaults.master),
            music: level(self.music, defaults.music),
            effects: level(self.effects, defaults.effects),
            ui: level(self.ui, defaults.ui),
            muted: self.muted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_preferences_use_independent_restrained_defaults() {
        assert_eq!(
            serde_json::from_str::<AudioSettings>("{}").unwrap(),
            AudioSettings::default()
        );
        let settings: AudioSettings =
            serde_json::from_str(r#"{"music":0.05,"muted":true}"#).unwrap();
        assert_eq!(
            settings,
            AudioSettings {
                music: 0.05,
                muted: true,
                ..AudioSettings::default()
            }
        );
        assert_eq!(
            serde_json::from_slice::<AudioSettings>(&serde_json::to_vec(&settings).unwrap())
                .unwrap(),
            settings
        );
    }

    #[test]
    fn invalid_levels_cannot_escape_the_mixer_range_or_poison_other_buses() {
        let settings = AudioSettings {
            master: f32::NAN,
            music: f32::INFINITY,
            effects: -2.0,
            ui: 4.0,
            muted: true,
        }
        .sanitized();
        assert_eq!(
            settings,
            AudioSettings {
                effects: 0.0,
                ui: 1.0,
                muted: true,
                ..AudioSettings::default()
            }
        );
        assert_eq!(settings.sanitized(), settings);
        let negative_infinity = AudioSettings {
            ui: f32::NEG_INFINITY,
            ..AudioSettings::default()
        }
        .sanitized();
        assert_eq!(negative_infinity, AudioSettings::default());
        assert!(!serde_json::to_string(&settings).unwrap().contains("null"));
    }
}
