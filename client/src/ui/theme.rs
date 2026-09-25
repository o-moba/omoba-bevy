//! One Verdant palette, the packaged fonts and the metrics every overlay shares.
//!
//! Every overlay and front-end screen reads its colours from here; the old
//! `ui_theme` and `frontend::widgets` re-export shims are gone.
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
/// Ekza wallet/account connection buttons on hero select.
pub const LINK: Color = Color::srgb(0.22, 0.30, 0.55);
pub const LINK_HOVER: Color = Color::srgb(0.30, 0.40, 0.70);
/// Team-coloured lock-in buttons on hero select.
pub const TEAM_GREEN: Color = Color::srgba(0.12, 0.40, 0.28, 0.98);
pub const TEAM_GREEN_HOVER: Color = Color::srgba(0.18, 0.65, 0.28, 0.98);
pub const TEAM_BLUE: Color = Color::srgba(0.16, 0.28, 0.48, 0.98);
pub const TEAM_BLUE_HOVER: Color = Color::srgba(0.22, 0.45, 0.85, 0.98);

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

    // The responsive policy: every phone/desktop size decision the overlays
    // make is answered here, so `adapt_phone_menu_readability`,
    // `adapt_phone_layout`, `sync_phone_ui` and `size_desktop_pause_panel`
    // only apply the numbers.

    /// The layout family a size is resolved for.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Form {
        Desktop,
        Phone,
    }

    impl Form {
        pub fn of(phone: bool) -> Self {
            if phone { Self::Phone } else { Self::Desktop }
        }

        /// `Phone` when the phone HUD is enabled (`MobileControls.enabled`).
        pub fn from_mobile(mobile: Option<&crate::mobile_controls::MobileControls>) -> Self {
            Self::of(mobile.is_some_and(|mobile| mobile.enabled))
        }
    }

    /// `UiScale` as the policy divides by it: never below 0.1.
    pub fn ui_scale(scale: f32) -> f32 {
        scale.max(0.1)
    }

    /// Smallest readable front-end font on a phone, in physical-looking px
    /// (divided by `UiScale` by [`menu_font`]).
    pub const PHONE_HEADING_MIN: f32 = 20.0;
    pub const PHONE_LABEL_MIN: f32 = 12.0;

    /// A front-end label designed at `size`: unchanged on desktop, raised to
    /// the phone minimum otherwise.
    pub fn menu_font(form: Form, size: f32, heading: bool, scale: f32) -> f32 {
        match form {
            Form::Desktop => size,
            Form::Phone => {
                let minimum = if heading {
                    PHONE_HEADING_MIN
                } else {
                    PHONE_LABEL_MIN
                };
                size.max(minimum / ui_scale(scale))
            }
        }
    }

    /// A front-end control designed `height` tall: unchanged on desktop,
    /// raised to [`TOUCH_MIN`] on a phone.
    pub fn menu_control_height(form: Form, height: f32, scale: f32) -> f32 {
        match form {
            Form::Desktop => height,
            Form::Phone => height.max(TOUCH_MIN / ui_scale(scale)),
        }
    }

    /// The pause panel as spawned (desktop), `(width, settings height)`.
    pub const PAUSE_PANEL: (f32, f32) = (480.0, 560.0);
    /// Desktop pause panel height on the main page.
    pub const PAUSE_MAIN_H: f32 = 380.0;
    /// Widest phone pause panel, and its main-page height cap.
    pub const PHONE_PAUSE_W: f32 = 650.0;
    pub const PHONE_PAUSE_MAIN_H: f32 = 360.0;

    /// Pause panel height. On a phone `available` is the safe-area height;
    /// the desktop ignores it.
    pub fn pause_panel_height(form: Form, in_settings: bool, available: f32) -> f32 {
        match (form, in_settings) {
            (Form::Desktop, true) => PAUSE_PANEL.1,
            (Form::Desktop, false) => PAUSE_MAIN_H,
            (Form::Phone, true) => available,
            (Form::Phone, false) => available.min(PHONE_PAUSE_MAIN_H),
        }
    }

    /// Widest phone help panel, server-entry panel and match result card.
    pub const PHONE_HELP_W: f32 = 740.0;
    pub const PHONE_SERVER_W: f32 = 860.0;
    pub const PHONE_RESULT_W: f32 = 640.0;

    /// Phone hero-select class column for a safe-area `width`.
    pub fn phone_class_column(width: f32) -> f32 {
        (width * 0.26).clamp(150.0, 210.0)
    }

    /// Phone shop card, `(width, height)`, three to a row.
    pub fn phone_shop_card(width: f32) -> (f32, f32) {
        ((width - 36.0) / 3.0, 103.0)
    }

    /// Phone bar button (`?`, MENU, SERVER) width before `UiScale`; the bar
    /// keeps them at least 48 wide and [`TOUCH_MIN`] tall.
    pub const PHONE_BAR_HELP_W: f32 = 48.0;
    pub const PHONE_BAR_MENU_W: f32 = 64.0;
    pub const PHONE_BAR_SERVER_W: f32 = 88.0;
    pub const PHONE_BAR_MIN_W: f32 = 48.0;

    /// Text families the phone layout rescales, by the panel they sit in.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum PhoneText {
        /// The phone bar: fixed physical size through `UiScale`.
        Bar,
        /// Hero select (`TeamSelectOverlay`).
        Entry,
        ShopCard,
        Shop,
        ShopSummary,
        Result,
        Help,
        Pause,
    }

    /// Font size of a phone label designed at `original`, inside a safe area
    /// `width` wide.
    pub fn phone_font(family: PhoneText, original: f32, width: f32, scale: f32) -> f32 {
        match family {
            PhoneText::Bar => original / ui_scale(scale),
            PhoneText::Entry => original.clamp(11.0, 16.0),
            PhoneText::ShopCard if width < 650.0 => {
                if original >= 18.0 {
                    15.0
                } else {
                    12.0
                }
            }
            PhoneText::ShopCard | PhoneText::Shop => original.clamp(12.0, 18.0),
            PhoneText::ShopSummary => 14.0,
            PhoneText::Result => 20.0,
            PhoneText::Help => 15.0,
            PhoneText::Pause => original.clamp(14.0, 22.0),
        }
    }
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
    /// External connections (Ekza wallet/account, studio refresh) on hero
    /// select; the blue the picker always used for them.
    Link,
    /// A team's lock-in button, in the team's colour.
    Team(crate::domain::Team),
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
        ButtonKind::Link => LINK,
        ButtonKind::Team(crate::domain::Team::Green) => TEAM_GREEN,
        ButtonKind::Team(crate::domain::Team::Blue) => TEAM_BLUE,
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
        ButtonKind::Link => LINK_HOVER,
        ButtonKind::Team(crate::domain::Team::Green) => TEAM_GREEN_HOVER,
        ButtonKind::Team(crate::domain::Team::Blue) => TEAM_BLUE_HOVER,
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
        assert_eq!(button_idle_color(ButtonKind::Tile, true), TILE_SELECTED);
        assert_eq!(button_hover_color(ButtonKind::Tile, true), TILE_SELECTED);
        assert_ne!(
            button_idle_color(ButtonKind::Tile, false),
            button_hover_color(ButtonKind::Tile, false)
        );
    }

    #[test]
    fn hero_select_kinds_keep_the_picker_colours() {
        use crate::domain::Team;
        assert_eq!(button_idle_color(ButtonKind::Link, false), LINK);
        assert_eq!(button_hover_color(ButtonKind::Link, false), LINK_HOVER);
        assert_eq!(
            button_idle_color(ButtonKind::Team(Team::Green), false),
            TEAM_GREEN
        );
        assert_eq!(
            button_hover_color(ButtonKind::Team(Team::Blue), false),
            TEAM_BLUE_HOVER
        );
    }

    /// Pins the responsive policy to the pixel values the phone passes used
    /// before they moved here.
    #[test]
    fn metric_policy_keeps_the_phone_and_desktop_sizes() {
        use metric::{Form, PhoneText};
        assert_eq!(Form::of(true), Form::Phone);
        assert_eq!(Form::from_mobile(None), Form::Desktop);
        // Front-end readability: desktop untouched, phone minimums / UiScale.
        assert_eq!(metric::menu_font(Form::Desktop, 9.0, true, 0.5), 9.0);
        assert_eq!(metric::menu_font(Form::Phone, 9.0, false, 1.0), 12.0);
        assert_eq!(metric::menu_font(Form::Phone, 9.0, true, 1.0), 20.0);
        assert_eq!(metric::menu_font(Form::Phone, 16.0, true, 0.5), 40.0);
        assert_eq!(metric::menu_font(Form::Phone, 30.0, false, 1.0), 30.0);
        assert_eq!(metric::menu_font(Form::Phone, 1.0, false, 0.0), 120.0);
        assert_eq!(metric::menu_control_height(Form::Desktop, 30.0, 0.5), 30.0);
        assert_eq!(metric::menu_control_height(Form::Phone, 30.0, 1.0), 44.0);
        assert_eq!(metric::menu_control_height(Form::Phone, 30.0, 0.5), 88.0);
        assert_eq!(metric::menu_control_height(Form::Phone, 60.0, 1.0), 60.0);
        // Pause panel.
        assert_eq!(metric::pause_panel_height(Form::Desktop, true, 0.0), 560.0);
        assert_eq!(metric::pause_panel_height(Form::Desktop, false, 0.0), 380.0);
        assert_eq!(metric::pause_panel_height(Form::Phone, true, 350.0), 350.0);
        assert_eq!(metric::pause_panel_height(Form::Phone, false, 390.0), 360.0);
        assert_eq!(metric::pause_panel_height(Form::Phone, false, 300.0), 300.0);
        assert_eq!(metric::PAUSE_PANEL, (480.0, 560.0));
        // Phone hero select and shop.
        assert_eq!(metric::phone_class_column(400.0), 150.0);
        assert_eq!(metric::phone_class_column(700.0), 182.0);
        assert_eq!(metric::phone_class_column(1000.0), 210.0);
        assert_eq!(metric::phone_shop_card(846.0), (270.0, 103.0));
        // Phone text families.
        assert_eq!(metric::phone_font(PhoneText::Bar, 16.0, 800.0, 0.5), 32.0);
        assert_eq!(metric::phone_font(PhoneText::Entry, 30.0, 800.0, 1.0), 16.0);
        assert_eq!(metric::phone_font(PhoneText::Entry, 8.0, 800.0, 1.0), 11.0);
        assert_eq!(
            metric::phone_font(PhoneText::ShopCard, 20.0, 600.0, 1.0),
            15.0
        );
        assert_eq!(
            metric::phone_font(PhoneText::ShopCard, 14.0, 600.0, 1.0),
            12.0
        );
        assert_eq!(
            metric::phone_font(PhoneText::ShopCard, 20.0, 700.0, 1.0),
            18.0
        );
        assert_eq!(metric::phone_font(PhoneText::Shop, 10.0, 700.0, 1.0), 12.0);
        assert_eq!(
            metric::phone_font(PhoneText::ShopSummary, 30.0, 700.0, 1.0),
            14.0
        );
        assert_eq!(
            metric::phone_font(PhoneText::Result, 30.0, 700.0, 1.0),
            20.0
        );
        assert_eq!(metric::phone_font(PhoneText::Help, 30.0, 700.0, 1.0), 15.0);
        assert_eq!(metric::phone_font(PhoneText::Pause, 30.0, 700.0, 1.0), 22.0);
        assert_eq!(metric::phone_font(PhoneText::Pause, 10.0, 700.0, 1.0), 14.0);
    }

    #[test]
    fn accents_are_addressable_and_clamped() {
        assert_eq!(accent_color(0), ACCENTS[0].1);
        assert_eq!(accent_color(99), ACCENTS[ACCENTS.len() - 1].1);
    }
}
