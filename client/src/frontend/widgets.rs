//! Layout nodes shared by the front-end screens: the screen root, panel
//! strips, headings and labels, and the phone readability pass.
//!
//! Buttons and tiles are kit widgets (`crate::ui::widgets::{screen_button,
//! screen_tile}`) carrying a typed `UiAction`; the palette is
//! `crate::ui::theme`.

use bevy::prelude::*;

use super::AppScreen;
use crate::ui::theme::{BACKDROP, IVORY, PANEL_EDGE, PANEL_OPAQUE, SCREEN_Z};
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

fn adapt_phone_menu_readability(
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    scale: Res<UiScale>,
    mut labels: Query<(&MenuTypography, &mut TextFont)>,
    mut buttons: Query<(&MenuControl, &mut Node)>,
) {
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    let scale = scale.0.max(0.1);
    for (metric, mut font) in &mut labels {
        let size = if phone {
            metric
                .size
                .max(if metric.heading { 20.0 } else { 12.0 } / scale)
        } else {
            metric.size
        };
        if font.font_size != size {
            font.font_size = size;
        }
    }
    for (metric, mut node) in &mut buttons {
        let height = if phone {
            metric
                .height
                .max(crate::ui::theme::metric::TOUCH_MIN / scale)
        } else {
            metric.height
        };
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

/// A short labelled strip inside a column (used for "last match" on home).
pub fn panel_row() -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(PANEL_OPAQUE),
        BorderColor::all(PANEL_EDGE),
    )
}

pub fn heading(text: &str, size: f32) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(IVORY),
        MenuTypography {
            size,
            heading: true,
        },
    )
}

pub fn label(text: &str, size: f32, color: Color) -> impl Bundle {
    crate::ui::widgets::screen_label(text, size, color)
}
