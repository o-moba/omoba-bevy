//! Opt-in native QA harnesses, compiled only with the `qa` cargo feature
//! (on by default, so `cargo run -p client`, the packaging scripts and the
//! capture scripts see no difference). Every harness is dormant unless its
//! environment variable is set; `docs/ARCHITECTURE.md` lists them.
//!
//! [`QaPlugins`] is added after the production groups. Its plugins override
//! `WinitSettings` with `continuous()` and `FrontendQaPlugin`/`AvatarQaPlugin`
//! insert `ScreenDriverPaused(true)`, which is correct after `FrontendPlugin`'s
//! `init_resource` (the order they had before the groups).

use bevy::app::{PluginGroup, PluginGroupBuilder};

pub(crate) mod animation_qa;
mod audio_qa;
mod beta_ui_qa;
mod career_visual_qa;
mod combat_qa;
mod forest_pickup_qa;
mod frontend_flow_qa;
mod frontend_qa;
mod map_qa;
mod navigation_qa;
mod offline_qa;
mod social_qa;
mod supporter;
mod targeting_qa;
mod team_vision_qa;
mod visual_qa;

/// The env-triggered QA plugins, in the order `main` added them before the
/// groups. `VisualQaPlugin` nests `BetaUiQaPlugin` and `NavigationQaPlugin`,
/// `FrontendQaPlugin` nests `AvatarQaPlugin` and `FrontendFlowQaPlugin`.
pub(crate) struct QaPlugins;

impl PluginGroup for QaPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(frontend_qa::FrontendQaPlugin)
            .add(visual_qa::VisualQaPlugin)
            .add(social_qa::SocialQaPlugin)
            .add(supporter::SupporterQaPlugin)
            .add(team_vision_qa::TeamVisionQaPlugin)
            .add(audio_qa::AudioQaPlugin)
            .add(offline_qa::OfflineQaPlugin)
            .add(career_visual_qa::CareerVisualQaPlugin)
            .add(map_qa::MapQaPlugin)
            .add(combat_qa::CombatQaPlugin)
            .add(forest_pickup_qa::ForestPickupQaPlugin)
            .add(targeting_qa::TargetingQaPlugin)
    }
}
