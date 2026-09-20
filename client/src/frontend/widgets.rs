//! Shared look and behaviour for the front-end screens.
//!
//! Screens describe *what* they show; this module owns the palette, the
//! reusable nodes and the hover/press feedback so every screen reacts the same
//! way. A screen only has to read [`Interaction::Pressed`] on its own marker.

use bevy::prelude::*;

use super::AppScreen;

/// Opaque menu backdrop. The gameplay world keeps rendering underneath, so the
/// backdrop must not be translucent.
pub const BACKDROP: Color = Color::srgb(0.020, 0.052, 0.058);
pub const PANEL: Color = Color::srgb(0.035, 0.085, 0.092);
pub const PANEL_EDGE: Color = Color::srgb(0.13, 0.27, 0.27);
pub const TILE: Color = Color::srgb(0.055, 0.135, 0.142);
pub const TILE_HOVER: Color = Color::srgb(0.095, 0.225, 0.225);
pub const TILE_SELECTED: Color = Color::srgb(0.16, 0.38, 0.35);
pub const PRIMARY: Color = Color::srgb(0.18, 0.62, 0.42);
pub const PRIMARY_HOVER: Color = Color::srgb(0.24, 0.76, 0.52);
pub const DANGER: Color = Color::srgb(0.46, 0.16, 0.16);
pub const DANGER_HOVER: Color = Color::srgb(0.60, 0.22, 0.22);
pub const IVORY: Color = Color::srgb(0.91, 0.94, 0.86);
pub const MUTED: Color = Color::srgb(0.55, 0.70, 0.66);
pub const GOLD: Color = Color::srgb(0.94, 0.77, 0.43);

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
        app.add_systems(Update, (paint_buttons, repaint_changed_buttons));
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
            padding: UiRect::all(Val::Px(24.0)),
            row_gap: Val::Px(16.0),
            ..default()
        },
        BackgroundColor(BACKDROP),
        ZIndex(SCREEN_Z),
        bevy::state::state_scoped::DespawnOnExit(screen),
        Name::new(name.to_owned()),
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
    let (width, height, font) = match menu.kind {
        ButtonKind::Primary => (Val::Px(260.0), Val::Px(62.0), 24.0),
        _ => (Val::Auto, Val::Px(42.0), 16.0),
    };
    parent
        .spawn((
            Button,
            Node {
                width,
                height,
                min_width: Val::Px(120.0),
                padding: UiRect::axes(Val::Px(18.0), Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(menu.idle_color()),
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
