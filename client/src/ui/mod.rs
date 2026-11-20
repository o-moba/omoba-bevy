//! Client UI kit: one theme, one tap recognizer, one scroll system, a modal
//! registry, typed button actions and the widgets built on them. `docs/ui-kit.md` describes the API and the
//! migration state; the pause menu and the practice sandbox are the pilot.
use bevy::{a11y::AccessibilityNode, prelude::*, ui::FocusPolicy};

mod accessibility;
pub(crate) mod action;
pub(crate) mod back;
pub(crate) mod focus;
pub(crate) mod font_cmap;
pub(crate) mod gesture;
pub(crate) mod kit_assets;
pub(crate) mod living_background;
pub(crate) mod modal;
pub(crate) mod scroll;
pub(crate) mod test_id;
pub(crate) mod theme;
pub(crate) mod tokens;
pub(crate) mod widgets;

pub(crate) use action::{Activated, UiAction, UiActionAppExt};
pub(crate) use back::{BackInput, BackPress};
pub(crate) use focus::{FocusEntry, FocusNav, UiFocus};
pub(crate) use gesture::{GestureEpoch, Pressable, SyntheticPress};
pub(crate) use modal::{ModalAppExt, ModalId, ModalRoot, ModalStack};
pub(crate) use scroll::ScrollArea;
pub(crate) use test_id::TestId;

use crate::platform::UiProfile;

/// OMOBA's legacy button marker. Bevy 0.20's deprecated compatibility alias
/// exposes a private type, so the UI kit owns the marker while it retains its
/// established `Interaction` behavior.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq, Eq)]
#[require(Node, FocusPolicy::Block, Interaction, AccessibilityNode = accessibility::button_node())]
pub(crate) struct Button;

/// Bounds of the visible portion of a physical-pixel rectangle. Focus rings and
/// QA use these axis-aligned bounds; pointer hit tests use the clip itself.
pub(crate) fn clipped_ui_rect(mut bounds: Rect, clip: Option<&CalculatedClip>) -> Rect {
    let Some(clip) = clip else { return bounds };
    let Some(rects) = clip.rects() else {
        return Rect::default();
    };
    for clip in rects {
        // Start from finite node bounds. A one-axis scroll clip has infinite
        // edges in its local space, which cannot be transformed into points.
        let local = transform_rect_bounds(bounds, clip.world_to_clip_local).intersect(clip.rect);
        if local.is_empty() {
            return Rect::default();
        }
        bounds = bounds.intersect(transform_rect_bounds(
            local,
            clip.world_to_clip_local.inverse(),
        ));
    }
    bounds
}

pub(super) fn transform_rect_bounds(rect: Rect, transform: bevy::math::Affine2) -> Rect {
    let mut bounds = Rect::from_center_size(transform.transform_point2(rect.min), Vec2::ZERO);
    for corner in [
        Vec2::new(rect.max.x, rect.min.y),
        rect.max,
        Vec2::new(rect.min.x, rect.max.y),
    ] {
        bounds = bounds.union_point(transform.transform_point2(corner));
    }
    bounds
}

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
        crate::i18n::configure_text_sets(app);
        let form = theme::Form::of(app.world().resource::<UiPlatform>().is_mobile());
        app.insert_resource(theme::UiForm(form))
            .init_resource::<theme::FontCoverage>()
            .init_resource::<kit_assets::UiDensity>()
            .init_resource::<kit_assets::KitAtlasLayouts>()
            .init_resource::<kit_assets::KitPreload>()
            .init_resource::<GestureEpoch>()
            .init_resource::<BackPress>()
            .init_resource::<UiFocus>()
            .add_message::<SyntheticPress>()
            .add_message::<FocusNav>()
            .add_message::<focus::FocusAdjust>()
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
            .add_systems(
                PostUpdate,
                (
                    theme::index_font_coverage,
                    theme::apply_theme_font,
                    theme::apply_text_styles,
                )
                    .chain()
                    .in_set(crate::i18n::I18nSystems::Font),
            )
            .add_systems(
                PostUpdate,
                kit_assets::fit_cover_images.after(bevy::ui::UiSystems::Layout),
            )
            .add_systems(
                PreUpdate,
                (
                    kit_assets::update_ui_density,
                    kit_assets::keep_preloaded.after(kit_assets::update_ui_density),
                ),
            );
        app.add_plugins(living_background::LivingBackgroundPlugin);
        accessibility::add_systems(app);
        widgets::controls::add_systems(app);
        widgets::surfaces::add_systems(app);
        widgets::game::add_systems(app);
        widgets::status::add_systems(app);
        app.add_systems(Update, gesture::recognize_presses.in_set(UiSet::Gesture))
            .add_systems(Update, scroll::scroll_areas.in_set(UiSet::Scroll))
            .add_systems(Update, focus::navigate_focus.in_set(UiSet::Focus))
            .add_systems(
                Update,
                (
                    widgets::paint_pressables,
                    widgets::paint_slabs,
                    widgets::paint_kit,
                    widgets::paint_focus_ring,
                    widgets::paint_preview_rings,
                    kit_assets::resolve_kit_images,
                )
                    .chain()
                    .in_set(UiSet::Paint),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_button_preserves_required_interaction_components() {
        let mut world = World::new();
        let button = world.spawn(Button).id();
        let entity = world.entity(button);

        assert!(entity.contains::<Node>());
        assert_eq!(entity.get::<FocusPolicy>(), Some(&FocusPolicy::Block));
        assert_eq!(entity.get::<Interaction>(), Some(&Interaction::None));
    }
}
