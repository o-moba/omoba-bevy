//! Client UI kit: one theme, one tap recognizer, one scroll system, a modal
//! registry, typed button actions and the widgets built on them. `docs/ui-kit.md` describes the API and the
//! migration state; the pause menu and the practice sandbox are the pilot.
use bevy::prelude::*;

pub(crate) mod action;
pub(crate) mod back;
pub(crate) mod focus;
pub(crate) mod gesture;
pub(crate) mod modal;
pub(crate) mod scroll;
pub(crate) mod test_id;
pub(crate) mod theme;
pub(crate) mod widgets;

pub(crate) use action::{Activated, UiAction, UiActionAppExt};
pub(crate) use back::{BackInput, BackPress};
pub(crate) use focus::{FocusNav, UiFocus};
pub(crate) use gesture::{GestureEpoch, Pressable, SyntheticPress};
pub(crate) use modal::{ModalAppExt, ModalId, ModalRoot, ModalStack};
pub(crate) use scroll::ScrollArea;
pub(crate) use test_id::TestId;

use crate::platform::UiProfile;

/// The compile-target interface family, resolved once at startup. Systems
/// read this instead of calling `platform::ui_profile()`; `MobileControls
/// .enabled` stays the runtime copy the gameplay HUD already uses.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct UiPlatform(pub UiProfile);

impl UiPlatform {
    pub(crate) fn is_mobile(&self) -> bool {
        self.0 == UiProfile::Mobile
    }
}

/// Frame order of the kit inside `InputContextSet::Modal`: directional focus
/// turns a confirm into a synthetic press, the recognizer runs next, scroll
/// areas move, typed actions are dispatched, then the buttons repaint.
/// Modules consume `Activated<T>` after `UiSet::Dispatch`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum UiSet {
    Focus,
    Gesture,
    Scroll,
    Dispatch,
    Paint,
}

pub(crate) struct UiKitPlugin;

impl Plugin for UiKitPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<UiPlatform>() {
            app.insert_resource(UiPlatform(crate::platform::ui_profile()));
        }
        app.init_resource::<GestureEpoch>()
            .init_resource::<BackPress>()
            .init_resource::<UiFocus>()
            .add_message::<SyntheticPress>()
            .add_message::<FocusNav>()
            .add_systems(Last, back::clear_back_press)
            .configure_sets(
                Update,
                (
                    UiSet::Focus,
                    UiSet::Gesture,
                    UiSet::Scroll,
                    UiSet::Dispatch,
                    UiSet::Paint,
                )
                    .chain()
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(Startup, theme::load_theme)
            .add_systems(Update, theme::apply_theme_font)
            .add_systems(Update, gesture::recognize_presses.in_set(UiSet::Gesture))
            .add_systems(Update, scroll::scroll_areas.in_set(UiSet::Scroll))
            .add_systems(Update, focus::navigate_focus.in_set(UiSet::Focus))
            .add_systems(
                Update,
                (widgets::paint_pressables, widgets::paint_focus_ring).in_set(UiSet::Paint),
            );
    }
}
