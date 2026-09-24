//! Shared look and behaviour for the front-end screens.
//!
//! Screens describe *what* they show; this module owns the palette, the
//! reusable nodes and the hover/press feedback so every screen reacts the same
//! way. A screen only has to read [`Interaction::Pressed`] on its own marker.
//!
//! The palette and [`MenuButton`] live in `crate::ui::theme`; they are
//! re-exported here so the screens did not have to change.

use bevy::prelude::*;

use super::AppScreen;

pub use crate::ui::theme::{
    ACCENTS, BACKDROP, ButtonKind, DANGER, DANGER_HOVER, GOLD, IVORY, MUTED, MenuButton,
    PANEL_EDGE, PANEL_OPAQUE as PANEL, PRIMARY, PRIMARY_HOVER, SCREEN_Z, TILE, TILE_HOVER,
    TILE_SELECTED, accent_color,
};

pub struct FrontendWidgetsPlugin;

impl Plugin for FrontendWidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (paint_buttons, repaint_changed_buttons))
            .add_systems(
                PostUpdate,
                adapt_phone_menu_readability.before(bevy::ui::UiSystems::Layout),
            );
    }
}

fn paint_buttons(
    mut buttons: Query<(&Interaction, &MenuButton, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (interaction, button, mut color) in &mut buttons {
        *color = BackgroundColor(match interaction {
            Interaction::Hovered | Interaction::Pressed => button.hover_color(),
            Interaction::None => button.idle_color(),
        });
    }
}

/// Selection changes come from the screen, not the pointer, so a tile that
/// just became (un)selected has to be repainted even without a hover event.
fn repaint_changed_buttons(
    mut buttons: Query<(&MenuButton, &mut BackgroundColor), Changed<MenuButton>>,
) {
    for (button, mut color) in &mut buttons {
        *color = BackgroundColor(button.idle_color());
    }
}

/// Frontend metrics are retained separately so global menu fitting never
/// turns phone controls into sub-44px touch targets or tiny labels.
#[derive(Component)]
struct MenuTypography {
    size: f32,
    heading: bool,
}
#[derive(Component)]
struct MenuControl {
    height: f32,
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
            metric.height.max(44.0 / scale)
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
        BackgroundColor(PANEL),
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
    (
        Text::new(text.to_owned()),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
        MenuTypography {
            size,
            heading: false,
        },
    )
}

/// Spawns a labelled button carrying the screen's own action marker.
pub fn button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    kind: ButtonKind,
    marker: M,
    name: &str,
) -> Entity {
    spawn_button(parent, text, MenuButton::new(kind), marker, name, false)
}

/// A grid tile whose selected state is owned by the screen.
pub fn tile<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    selected: bool,
    marker: M,
    name: &str,
) -> Entity {
    spawn_button(parent, text, MenuButton::tile(selected), marker, name, false)
}

/// A grid tile that shrinks on a phone (the avatar clip strip).
pub fn compact_tile<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    selected: bool,
    marker: M,
    name: &str,
    compact: bool,
) -> Entity {
    spawn_button(parent, text, MenuButton::tile(selected), marker, name, compact)
}

fn spawn_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    menu: MenuButton,
    marker: M,
    name: &str,
    compact_clip: bool,
) -> Entity {
    let (width, height, font) = match menu.kind {
        ButtonKind::Primary => (Val::Px(240.0), Val::Px(60.0), 22.0),
        _ => (Val::Auto, Val::Px(44.0), 15.0),
    };
    parent
        .spawn((
            Button,
            Node {
                width,
                height,
                min_width: Val::Px(if compact_clip { 86.0 } else { 120.0 }),
                padding: UiRect::axes(
                    Val::Px(if compact_clip { 10.0 } else { 18.0 }),
                    Val::Px(8.0),
                ),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(if menu.kind == ButtonKind::Primary {
                GOLD
            } else {
                PANEL_EDGE
            }),
            BackgroundColor(menu.idle_color()),
            MenuControl {
                height: if menu.kind == ButtonKind::Primary {
                    60.0
                } else {
                    44.0
                },
            },
            menu,
            marker,
            Name::new(name.to_owned()),
        ))
        .with_children(|button| {
            button.spawn(label(text, font, IVORY));
        })
        .id()
}
