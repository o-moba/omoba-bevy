// i18n-strict
mod bars;
mod cast;
mod cooldown;
mod feedback;
mod hotbar;
pub(crate) mod inspection;
mod marker;
mod mobile;
mod round_reset;
mod selection;
pub(crate) mod skill_card;
pub(crate) mod standard;
mod tactical_hud;
pub(crate) mod targeting;

pub use crate::domain::{CombatStats, MAX_HP};
use crate::input_context::InputContextSet;
pub(crate) use crate::input_context::{CombatPointerInputSet, WorldMovementInputSet};
use crate::targeting::{BasicAttackState, TargetAimPreview};
use bevy::prelude::*;

#[cfg(feature = "qa")]
pub(crate) use bars::CombatBarAnchor;
pub(crate) use cast::{PendingCast, queue_cast_request};
pub use cooldown::LocalCastCooldown;
pub(crate) use cooldown::effective_cast_duration;
pub(crate) use feedback::ActionFeedback;
pub(crate) use hotbar::upgrade_eligible;
pub(crate) use mobile::mobile_assisted_target;
pub use selection::TargetState;
#[cfg(feature = "qa")]
pub(crate) use selection::nearest_enemy;
pub(crate) use selection::{TargetCandidates, WorldPointerState};

use bars::{
    setup_combat_visual_assets, spawn_combat_bars_system, sync_combat_bar_transforms_system,
    update_combat_bars_system,
};
use cast::{cast_spell_system, resolve_pending_cast_system};
use cooldown::{sync_authoritative_cooldown_durations, tick_local_cast_cooldown};
use feedback::update_action_feedback;
use hotbar::{
    setup_combat_ui, skill_button_system, skill_upgrade_input_system, sync_skill_key_labels,
    update_skill_bar_system, update_skill_tooltip,
};
use marker::update_target_marker_system;
use mobile::{mobile_cast_system, mobile_utility_system};
use round_reset::reset_round_input_state;
use selection::select_target_system;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        use crate::ui::UiActionAppExt;
        app.insert_gizmo_config(
            standard::SkillAimGizmos,
            bevy::gizmos::config::GizmoConfig {
                line: bevy::gizmos::config::GizmoLineConfig {
                    width: 5.0,
                    ..default()
                },
                depth_bias: -0.01,
                ..default()
            },
        )
        .insert_gizmo_config(
            standard::SkillEffectGizmos,
            bevy::gizmos::config::GizmoConfig {
                line: bevy::gizmos::config::GizmoLineConfig {
                    width: 3.5,
                    ..default()
                },
                depth_bias: -0.005,
                ..default()
            },
        )
        .init_resource::<standard::SkillAimVector>()
        .init_resource::<tactical_hud::TacticalHud>()
        .add_systems(
            Update,
            tactical_hud::update_tactical_hud
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .after(crate::mobile_controls::MobileControlsSet::Input),
        )
        .add_systems(
            PostUpdate,
            (standard::draw_minimap_aim, standard::draw_minimap_traps)
                .before(bevy::ui::UiSystems::Layout),
        )
        .add_ui_action::<hotbar::HotbarAction>()
        .init_resource::<TargetState>()
        .init_resource::<BasicAttackState>()
        .init_resource::<TargetAimPreview>()
        .init_resource::<LocalCastCooldown>()
        .init_resource::<WorldPointerState>()
        .init_resource::<PendingCast>()
        .init_resource::<ActionFeedback>()
        .init_resource::<inspection::SkillInspection>()
        .add_systems(
            Update,
            reset_round_input_state
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(InputContextSet::Modal),
        )
        .add_systems(Startup, (setup_combat_visual_assets, standard::setup))
        .add_systems(
            Update,
            (
                standard::interact,
                standard::draw_effects,
                standard::draw_aim.after(InputContextSet::Actions),
            ),
        )
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
                inspection::update_inspection,
                update_skill_tooltip,
                standard::update_status,
                sync_skill_key_labels,
            )
                .chain()
                .after(WorldMovementInputSet)
                .in_set(InputContextSet::Actions),
        );
        app.add_systems(
            Update,
            skill_card::paint_skill_card.in_set(crate::ui::UiSet::Paint),
        );
        configure_target_presentation(app);
        app.add_systems(
            PostUpdate,
            (
                spawn_combat_bars_system,
                update_combat_bars_system,
                sync_combat_bar_transforms_system,
                tactical_hud::update_overhead,
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
            crate::targeting::refresh_aim_projection,
            crate::targeting::draw_targeting_ui.after(crate::targeting::refresh_aim_projection),
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
