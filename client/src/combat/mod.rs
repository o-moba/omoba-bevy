mod bars;
mod cast;
mod cooldown;
mod feedback;
mod hotbar;
mod marker;
mod mobile;
mod round_reset;
mod selection;
pub(crate) mod targeting;

pub use crate::domain::{CombatStats, MAX_HP};
use crate::input_context::InputContextSet;
pub(crate) use crate::input_context::{CombatPointerInputSet, WorldMovementInputSet};
use crate::targeting::{BasicAttackState, TargetAimPreview};
use bevy::prelude::*;

#[cfg(feature = "qa")]
pub(crate) use bars::CombatBarAnchor;
pub(crate) use cast::PendingCast;
pub use cooldown::LocalCastCooldown;
pub(crate) use cooldown::effective_cast_duration;
pub(crate) use feedback::ActionFeedback;
pub use selection::TargetState;
pub(crate) use selection::{TargetCandidates, WorldPointerState};

use bars::{
    setup_combat_visual_assets, spawn_combat_bars_system, sync_combat_bar_transforms_system,
    update_combat_bars_system,
};
use cast::{cast_spell_system, resolve_pending_cast_system};
use cooldown::{sync_authoritative_cooldown_durations, tick_local_cast_cooldown};
use feedback::{adapt_mobile_combat_feedback, update_action_feedback};
use hotbar::{
    setup_combat_ui, skill_button_system, skill_upgrade_input_system, update_skill_bar_system,
};
use marker::update_target_marker_system;
use mobile::{mobile_cast_system, mobile_utility_system};
use round_reset::{CombatRoundIdentity, reset_round_input_state};
use selection::select_target_system;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TargetState>()
            .init_resource::<BasicAttackState>()
            .init_resource::<TargetAimPreview>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<WorldPointerState>()
            .init_resource::<PendingCast>()
            .init_resource::<ActionFeedback>()
            .init_resource::<CombatRoundIdentity>()
            .add_systems(
                Update,
                reset_round_input_state
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(InputContextSet::Modal),
            )
            .add_systems(Startup, setup_combat_visual_assets)
            .add_systems(
                Startup,
                (setup_combat_ui, crate::targeting::setup_targeting_ui),
            )
            .add_systems(
                Update,
                select_target_system
                    .in_set(CombatPointerInputSet)
                    .in_set(InputContextSet::Actions),
            )
            .add_systems(
                Update,
                (
                    tick_local_cast_cooldown,
                    crate::targeting::tick_basic_attack,
                    sync_authoritative_cooldown_durations,
                    update_action_feedback,
                    crate::targeting::clear_invalid_selection,
                    cast_spell_system,
                    skill_button_system,
                    mobile_cast_system,
                    mobile_utility_system,
                    crate::targeting::mobile_basic_attack,
                    crate::targeting::resolve_basic_attack,
                    crate::targeting::face_attack_target,
                    resolve_pending_cast_system,
                    skill_upgrade_input_system,
                    update_skill_bar_system,
                    adapt_mobile_combat_feedback,
                    crate::targeting::draw_targeting_ui,
                )
                    .chain()
                    .after(WorldMovementInputSet)
                    .in_set(InputContextSet::Actions),
            );
        configure_target_presentation(app);
        app.add_systems(
            PostUpdate,
            (
                spawn_combat_bars_system,
                update_combat_bars_system,
                sync_combat_bar_transforms_system,
            )
                .chain(),
        );
    }
}

/// UI layout precedes Bevy's global-transform propagation. Compute current world
/// poses from the hierarchy here, after all movement and terrain grounding, so
/// both the world ring and screen frame use this frame's target/camera positions.
fn configure_target_presentation(app: &mut App) {
    app.add_systems(
        PostUpdate,
        (
            update_target_marker_system,
            crate::targeting::draw_locked_target,
        )
            .after(crate::net::NetworkGroundingSet)
            .after(bevy::camera::CameraUpdateSystems)
            .before(bevy::ui::UiSystems::Prepare)
            .before(bevy::ui::UiSystems::Layout)
            .before(bevy::transform::TransformSystems::Propagate),
    );
}

#[cfg(test)]
mod target_presentation_tests;

#[cfg(test)]
mod tests;
