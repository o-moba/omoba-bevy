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
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

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

/// A node's label in QA lookups and dumps: the `TestId` of a kit control,
/// otherwise its `Name` (layout nodes); see [`crate::ui::test_id::NodeKey`].
pub(crate) use crate::ui::test_id::NodeKey as QaName;

/// Harness presses by `TestId`. Every button a harness presses is a UI-kit
/// button, so the press is a `SyntheticPress`, which the recognizer turns
/// into exactly one activation in desktop and touch mode alike (and which a
/// modal in front blocks, like a real tap).
#[derive(SystemParam)]
pub(crate) struct TestIdPresses<'w, 's> {
    buttons: Query<'w, 's, (Entity, &'static crate::ui::TestId), With<crate::ui::Pressable>>,
    presses: MessageWriter<'w, crate::ui::SyntheticPress>,
}

impl TestIdPresses<'_, '_> {
    /// Presses every button whose id satisfies `wanted`; true if any did.
    pub(crate) fn press_where(&mut self, mut wanted: impl FnMut(&str) -> bool) -> bool {
        let mut pressed = false;
        for (entity, id) in &self.buttons {
            if wanted(id.as_str()) {
                pressed = true;
                self.presses.write(crate::ui::SyntheticPress(entity));
            }
        }
        pressed
    }

    /// Presses the button with id `wanted`, if it is on screen.
    pub(crate) fn press(&mut self, wanted: &str) -> bool {
        self.press_where(|id| id == wanted)
    }
}
