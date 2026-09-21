//! Shared initial tuning and additive snapshot contract for utility actions.
use serde::{Deserialize, Serialize};

pub const DASH_DISTANCE: f32 = 5.0;
pub const DASH_COOLDOWN_SECS: f32 = 20.0;
pub const HASTE_SPEED_MULTIPLIER: f32 = 1.4;
pub const HASTE_DURATION_SECS: f32 = 3.0;
pub const HASTE_COOLDOWN_SECS: f32 = 25.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtilityAction {
    Dash,
    Haste,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UtilityState {
    pub dash_remaining_secs: f32,
    pub haste_remaining_secs: f32,
    pub haste_active_secs: f32,
    /// High-water request mark, including rejected requests in this round.
    pub last_request_id: u64,
    /// Advances only on an accepted dash, including fully blocked dashes.
    pub dash_sequence: u64,
}

impl UtilityState {
    pub fn movement_multiplier(self) -> f32 {
        if self.haste_active_secs > 0.0 {
            HASTE_SPEED_MULTIPLIER
        } else {
            1.0
        }
    }
}
