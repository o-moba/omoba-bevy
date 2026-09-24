//! One Verdant palette, the packaged fonts and the metrics every overlay shares.
//!
//! The old `ui_theme` and `frontend::widgets` constants live here; both keep
//! `pub use` shims so callers did not have to move in the same change.
use bevy::prelude::*;

/// Opaque menu backdrop. The gameplay world keeps rendering underneath, so the
/// backdrop must not be translucent.
pub const BACKDROP: Color = Color::srgb(0.018, 0.039, 0.045);
/// Translucent overlay panel (pause menu, help, career).
pub const PANEL: Color = Color::srgba(0.025, 0.060, 0.065, 0.96);
/// Opaque panel used by the front-end screens.
pub const PANEL_OPAQUE: Color = Color::srgb(0.025, 0.060, 0.065);
pub const TILE: Color = Color::srgb(0.050, 0.115, 0.125);
pub const HOVER: Color = Color::srgb(0.085, 0.205, 0.200);
pub const TILE_HOVER: Color = HOVER;
pub const EDGE: Color = Color::srgb(0.19, 0.32, 0.30);
pub const PANEL_EDGE: Color = EDGE;
pub const TILE_SELECTED: Color = Color::srgb(0.095, 0.27, 0.23);
pub const PRIMARY: Color = Color::srgb(0.12, 0.46, 0.34);
pub const PRIMARY_HOVER: Color = Color::srgb(0.18, 0.60, 0.43);
pub const DANGER: Color = Color::srgb(0.32, 0.12, 0.13);
pub const DANGER_HOVER: Color = Color::srgb(0.47, 0.18, 0.18);
pub const GOLD: Color = Color::srgb(0.86, 0.74, 0.49);
pub const IVORY: Color = Color::srgb(0.92, 0.94, 0.89);
pub const MUTED: Color = Color::srgb(0.61, 0.72, 0.70);
pub const JADE: Color = Color::srgb(0.24, 0.79, 0.58);
/// Dim scrim behind a modal overlay (the pause menu root).
pub const SCRIM: Color = Color::srgba(0.0, 0.0, 0.0, 0.7);

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

/// Layout metrics shared by the kit widgets.
pub mod metric {
    /// Smallest touch target on a phone.
    pub const TOUCH_MIN: f32 = 44.0;
    /// Menu button height.
    pub const BUTTON_H: f32 = 46.0;
    /// Square `-`/`+` stepper button.
    pub const ADJUST_BTN: f32 = 44.0;
    /// Menu button width.
    pub const MENU_W: f32 = 320.0;
    /// The one call to action on a front-end screen.
    pub const PRIMARY: (f32, f32) = (240.0, 60.0);
}

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

/// Background when the pointer is away. Selected tiles keep their colour.
pub fn button_idle_color(kind: ButtonKind, selected: bool) -> Color {
    if selected {
        return TILE_SELECTED;
    }
    match kind {
        ButtonKind::Primary => PRIMARY,
        ButtonKind::Secondary | ButtonKind::Tile => TILE,
        ButtonKind::Danger => DANGER,
    }
}

/// Background while hovered or held.
pub fn button_hover_color(kind: ButtonKind, selected: bool) -> Color {
    if selected {
        return TILE_SELECTED;
    }
    match kind {
        ButtonKind::Primary => PRIMARY_HOVER,
        ButtonKind::Secondary | ButtonKind::Tile => HOVER,
        ButtonKind::Danger => DANGER_HOVER,
    }
}

/// Makes a front-end button react to the pointer without every screen
/// repeating it. Painted by `frontend::widgets`; kit widgets use
/// [`crate::ui::widgets::ButtonStyle`] instead.
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
        button_idle_color(self.kind, self.selected)
    }

    pub fn hover_color(&self) -> Color {
        button_hover_color(self.kind, self.selected)
    }
}

#[derive(Resource)]
pub(crate) struct UiTheme {
    pub font: Handle<Font>,
    pub cjk_font: Handle<Font>,
}

pub(super) fn load_theme(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(UiTheme {
        font: assets.load("ui/Inter.ttf"),
        cjk_font: assets.load("ui/NotoSansCJKsc-Regular.otf"),
    });
}

fn needs_cjk_font(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character as u32,
        0x1100..=0x11ff | 0x2e80..=0xa4cf | 0xa960..=0xa97f |
        0xac00..=0xd7ff | 0xf900..=0xfaff | 0xfe30..=0xfe4f |
        0xff00..=0xffef | 0x20000..=0x323af)
    })
}

pub(super) fn apply_theme_font(
    theme: Res<UiTheme>,
    mut text: Query<
        (&mut TextFont, Option<&Text>, Option<&TextSpan>),
        Or<(Added<TextFont>, Changed<Text>, Changed<TextSpan>)>,
    >,
) {
    for (mut font, root, span) in &mut text {
        let cjk = root.is_some_and(|text| needs_cjk_font(&text.0))
            || span.is_some_and(|text| needs_cjk_font(&text.0));
        font.font = if cjk {
            theme.cjk_font.clone()
        } else {
            theme.font.clone()
        };
    }
}

pub(crate) fn text(size: f32) -> TextFont {
    TextFont {
        font_size: size,
        ..default()
    }
}

pub(crate) fn panel_node() -> Node {
    Node {
        padding: UiRect::all(Val::Px(12.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(8.0)),
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packaged_font(name: &str) -> Font {
        Font::try_from_bytes(
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/ui")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn latin_cyrillic_keep_inter_and_cjk_names_use_the_packaged_fallback() {
        for value in ["Victory", "QA Дмитрий", "Δημήτρης"] {
            assert!(!needs_cjk_font(value));
        }
        for value in ["QA 小明", "かなカナ", "한글", "中文漢字"] {
            assert!(needs_cjk_font(value));
        }
        let mut fonts = Assets::<Font>::default();
        let font = fonts.add(packaged_font("Inter.ttf"));
        let cjk_font = fonts.add(packaged_font("NotoSansCJKsc-Regular.otf"));
        let mut app = App::new();
        app.insert_resource(UiTheme {
            font: font.clone(),
            cjk_font: cjk_font.clone(),
        })
        .add_systems(Update, apply_theme_font);
        let entity = app
            .world_mut()
            .spawn((Text::new("QA Дмитрий"), TextFont::default()))
            .id();
        app.update();
        assert_eq!(app.world().get::<TextFont>(entity).unwrap().font, font);
        app.world_mut().get_mut::<Text>(entity).unwrap().0 = "QA 小明".into();
        app.update();
        assert_eq!(app.world().get::<TextFont>(entity).unwrap().font, cjk_font);
        app.world_mut().get_mut::<Text>(entity).unwrap().0 = "QA Дмитрий".into();
        app.update();
        assert_eq!(app.world().get::<TextFont>(entity).unwrap().font, font);
        let span = app
            .world_mut()
            .spawn((TextSpan::new("한글"), TextFont::default()))
            .id();
        app.update();
        assert_eq!(app.world().get::<TextFont>(span).unwrap().font, cjk_font);
    }

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
