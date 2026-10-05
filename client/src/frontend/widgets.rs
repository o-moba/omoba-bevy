//! Layout nodes shared by the front-end screens: the screen root, panel
//! strips, headings and labels, and the phone readability pass.
//!
//! Buttons and tiles are kit widgets (`crate::ui::widgets::{screen_button,
//! screen_tile}`) carrying a typed `UiAction`; the palette is
//! `crate::ui::theme`.
// i18n-strict

use bevy::prelude::*;

use super::AppScreen;
use crate::ui::theme::{BACKDROP, IVORY, SCREEN_Z};
use crate::ui::widgets::{MenuControl, MenuTypography};

pub struct FrontendWidgetsPlugin;

impl Plugin for FrontendWidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            adapt_phone_menu_readability.before(bevy::ui::UiSystems::Layout),
        );
    }
}

/// Applies the `metric` policy to front-end labels and controls: phone
/// minimums for fonts and touch targets, the designed sizes on desktop.
fn adapt_phone_menu_readability(
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    scale: Res<UiScale>,
    mut labels: Query<(&MenuTypography, &mut TextFont)>,
    mut buttons: Query<(&MenuControl, &mut Node)>,
) {
    use crate::ui::theme::metric;
    let form = metric::Form::from_mobile(mobile.as_deref());
    for (typography, mut font) in &mut labels {
        let size = metric::menu_font(form, typography.size, typography.heading, scale.0);
        if font.font_size != size.into() {
            font.font_size = size.into();
        }
    }
    for (control, mut node) in &mut buttons {
        let height = metric::menu_control_height(form, control.height, scale.0);
        if node.height != Val::Px(height) {
            node.height = Val::Px(height);
        }
        if node.min_height != Val::Px(height) {
            node.min_height = Val::Px(height);
        }
        if node.flex_shrink != 0.0 {
            node.flex_shrink = 0.0;
        }
    }
}

/// Full-screen opaque root for a menu screen. Despawned automatically when the
/// screen is left.
pub fn screen_root(screen: AppScreen, name: &str) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(28.0)),
            row_gap: Val::Px(16.0),
            ..default()
        },
        BackgroundColor(BACKDROP),
        ZIndex(SCREEN_Z),
        bevy::state::state_scoped::DespawnOnExit(screen),
        Name::new(name.to_owned()),
    )
}

/// A front-end heading; `text` is a literal or a `crate::i18n::Localized` key.
pub fn heading(text: impl crate::i18n::UiLabel, size: f32) -> impl Bundle {
    (
        text.into_text(),
        TextFont {
            font_size: (size).into(),
            ..default()
        },
        TextColor(IVORY),
        MenuTypography {
            size,
            heading: true,
        },
    )
}

/// A front-end label; `text` is a literal or a `crate::i18n::Localized` key.
pub fn label(text: impl crate::i18n::UiLabel, size: f32, color: Color) -> impl Bundle {
    crate::ui::widgets::screen_label(text, size, color)
}
