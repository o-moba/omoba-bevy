//! Shared look and behaviour for the front-end screens.
//!
//! Screens describe *what* they show; this module owns the palette, the
//! reusable nodes and the hover/press feedback so every screen reacts the same
//! way. A screen only has to read [`Interaction::Pressed`] on its own marker.

use bevy::prelude::*;

use super::AppScreen;

/// Opaque menu backdrop. The gameplay world keeps rendering underneath, so the
/// backdrop must not be translucent.
pub const BACKDROP: Color = Color::srgb(0.018, 0.039, 0.045);
pub const PANEL: Color = Color::srgb(0.025, 0.060, 0.065);
pub const PANEL_EDGE: Color = crate::ui_theme::EDGE;
pub const TILE: Color = crate::ui_theme::TILE;
pub const TILE_HOVER: Color = crate::ui_theme::HOVER;
pub const TILE_SELECTED: Color = Color::srgb(0.095, 0.27, 0.23);
pub const PRIMARY: Color = Color::srgb(0.12, 0.46, 0.34);
pub const PRIMARY_HOVER: Color = Color::srgb(0.18, 0.60, 0.43);
pub const DANGER: Color = Color::srgb(0.32, 0.12, 0.13);
pub const DANGER_HOVER: Color = Color::srgb(0.47, 0.18, 0.18);
pub const IVORY: Color = crate::ui_theme::IVORY;
pub const MUTED: Color = crate::ui_theme::MUTED;
pub const GOLD: Color = crate::ui_theme::GOLD;

/// Accent colours a player can put on their profile card.
pub const ACCENTS: [(&str, Color); 6] = [
    ("Verdant", Color::srgb(0.24, 0.79, 0.58)),
    ("Ember", Color::srgb(0.90, 0.46, 0.22)),
    ("Amethyst", Color::srgb(0.58, 0.42, 0.88)),
    ("Tide", Color::srgb(0.24, 0.58, 0.88)),
    ("Gold", Color::srgb(0.94, 0.77, 0.43)),
    ("Rose", Color::srgb(0.88, 0.36, 0.52)),
];

pub fn accent_color(index: usize) -> Color {
    ACCENTS[index.min(ACCENTS.len() - 1)].1
}

/// Front-end screens sit above every gameplay overlay (the highest one in the
/// gameplay UI is the legacy select root at 15).
pub const SCREEN_Z: i32 = 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonKind {
    /// The one call to action on a screen.
    Primary,
    /// Navigation and secondary commands.
    Secondary,
    /// A selectable tile in a grid; selection is owned by the screen.
    Tile,
    /// Destructive or "back out" commands.
    Danger,
}

/// Makes a button react to the pointer without every screen repeating it.
#[derive(Component, Clone, Copy)]
pub struct MenuButton {
    pub kind: ButtonKind,
    /// Screens set this for grid tiles; selected tiles ignore hover colours.
    pub selected: bool,
}

impl MenuButton {
    pub fn new(kind: ButtonKind) -> Self {
        Self {
            kind,
            selected: false,
        }
    }

    pub fn tile(selected: bool) -> Self {
        Self {
            kind: ButtonKind::Tile,
            selected,
        }
    }

    pub fn idle_color(&self) -> Color {
        if self.selected {
            return TILE_SELECTED;
        }
        match self.kind {
            ButtonKind::Primary => PRIMARY,
            ButtonKind::Secondary => TILE,
            ButtonKind::Tile => TILE,
            ButtonKind::Danger => DANGER,
        }
    }

    pub fn hover_color(&self) -> Color {
        if self.selected {
            return TILE_SELECTED;
        }
        match self.kind {
            ButtonKind::Primary => PRIMARY_HOVER,
            ButtonKind::Secondary | ButtonKind::Tile => TILE_HOVER,
            ButtonKind::Danger => DANGER_HOVER,
        }
    }
}

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
    spawn_button(parent, text, MenuButton::new(kind), marker, name)
}

/// A grid tile whose selected state is owned by the screen.
pub fn tile<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    selected: bool,
    marker: M,
    name: &str,
) -> Entity {
    spawn_button(parent, text, MenuButton::tile(selected), marker, name)
}

fn spawn_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    menu: MenuButton,
    marker: M,
    name: &str,
) -> Entity {
    let compact_clip = crate::platform::ui_profile() == crate::platform::UiProfile::Mobile
        && (name.starts_with("AvatarClip-") || name == "AvatarAutoSpin");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_tiles_keep_their_colour_under_the_pointer() {
        let tile = MenuButton::tile(true);
        assert_eq!(tile.idle_color(), TILE_SELECTED);
        assert_eq!(tile.hover_color(), TILE_SELECTED);
        let plain = MenuButton::tile(false);
        assert_ne!(plain.idle_color(), plain.hover_color());
    }

    #[test]
    fn accents_are_addressable_and_clamped() {
        assert_eq!(accent_color(0), ACCENTS[0].1);
        assert_eq!(accent_color(99), ACCENTS[ACCENTS.len() - 1].1);
    }
}
