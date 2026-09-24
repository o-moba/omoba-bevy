mod animation;
mod input;
mod motion;
mod respawn_ui;

use crate::combat::{CombatPointerInputSet, WorldMovementInputSet};
pub(crate) use crate::domain::{MovementRoute, MovementTarget};
pub use crate::domain::{Player, PlayerBody, VerticalVelocity};
use crate::maps::MapLayout;
use crate::model_scale::NormalizeModelScale;
use crate::sprite::PlayerVisualMode;
use bevy::prelude::*;
pub use shared::hero_balance::{DEBUG_SPEED_MULTIPLIER, PLAYER_SPEED};

pub(crate) use animation::{PlayerAnimationBinding, register_hero_animation_systems};
pub(crate) use input::{mobile_screen_direction, viewport_to_simulation_world};

use animation::{PlayerAnimationLibrary, sync_jump_fallback_mode};
use input::{handle_player_input, move_player_mobile, plan_movement_routes};
use motion::{animate_jump, apply_gravity, move_player, resolve_player_structure_overlap};
use respawn_ui::{RespawnCountdown, respawn_countdown_system, setup_respawn_ui};

/// Debug speed-boost toggle, shared by the boost button and local movement.
#[derive(Resource, Default)]
pub struct DebugSpeedBoost(pub bool);
pub const PLAYER_SIZE: f32 = 1.0;
pub const JUMP_HEIGHT: f32 = 1.5;
pub const JUMP_DURATION: f32 = 0.6;
const GRAVITY: f32 = 20.0;
const GROUND_EPSILON: f32 = 0.001;
const RESPAWN_DELAY_SECONDS: f32 = shared::hero_balance::RESPAWN_DELAY_SECS as f32;

/// Entity-origin height that puts a character's feet on the walkable surface
/// at (x, z): terrain height plus the model's measured foot offset. Entities
/// without a measured model (primitive cube stand-in, or a GLB that is still
/// loading) keep the legacy half-cube offset.
pub(crate) fn ground_origin_y(
    layout: &MapLayout,
    mode: PlayerVisualMode,
    normalization: Option<&NormalizeModelScale>,
    x: f32,
    z: f32,
) -> f32 {
    let terrain = if mode == PlayerVisualMode::Models3d {
        layout.terrain_height_3d(x, z)
    } else {
        layout.terrain_height(x, z)
    };
    let offset = match normalization.and_then(NormalizeModelScale::foot_local_y) {
        Some(foot_local_y) => -foot_local_y,
        None => PLAYER_SIZE * 0.5,
    };
    terrain + offset
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                sync_jump_fallback_mode,
                handle_player_input.after(crate::input_context::InputContextSet::Resolve),
                move_player_mobile,
                plan_movement_routes,
                animate_jump,
                move_player,
            )
                .chain()
                .after(CombatPointerInputSet)
                .in_set(WorldMovementInputSet),
        )
        .add_systems(Update, resolve_player_structure_overlap.after(move_player))
        .add_systems(PostUpdate, apply_gravity)
        .init_resource::<RespawnCountdown>()
        .init_resource::<DebugSpeedBoost>()
        .init_resource::<PlayerAnimationLibrary>()
        .add_systems(Startup, setup_respawn_ui)
        .add_systems(Update, respawn_countdown_system);
        register_hero_animation_systems(app);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod animation_tests;
