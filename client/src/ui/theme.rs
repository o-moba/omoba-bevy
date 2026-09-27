//! The Verdant Crown theme: the legacy palette names mapped onto the design
//! tokens, the packaged fonts and text roles, the metrics every overlay
//! shares and the `ButtonKind` state colours.
//!
//! Every colour and size here comes from [`super::tokens`] (generated from
//! `client/ui/tokens/verdant-crown.json`). Documented exceptions that stay
//! literal: the developer-only `OMOBA_DEBUG_UI` toggle colours, the profile
//! card accent swatches (player-chosen content, not theme roles) and the
//! legacy responsive layout policy in [`metric`] (panel widths and phone text
//! clamps of screens that are not redesigned yet; they go as the screen steps
//! migrate).
// i18n-strict
use std::collections::HashSet;

use bevy::prelude::*;

use super::tokens::{self, FontFamily, TextCase, TextRole, color, radius, space};

// --- Legacy palette names → tokens (`omoba-ui/handoff/tokens.md` mapping). ---
// Kept as aliases until every screen is migrated to token names.

/// Opaque menu backdrop (`color.surface.0`). The gameplay world keeps
/// rendering underneath, so the backdrop must not be translucent.
pub const BACKDROP: Color = color::SURFACE_0;
/// Translucent overlay panel (`color.surface.1`).
pub const PANEL: Color = color::SURFACE_1;
/// Opaque panel used by the front-end screens (`color.surface.1.opaque`).
pub const PANEL_OPAQUE: Color = color::SURFACE_1_OPAQUE;
/// Tile, card and row fill (`color.surface.2`).
pub const TILE: Color = color::SURFACE_2;
/// Pointer hover tint (`color.surface.hover`).
pub const HOVER: Color = color::SURFACE_HOVER;
/// Plain panel and control border (`color.border.subtle`).
pub const EDGE: Color = color::BORDER_SUBTLE;
pub const PANEL_EDGE: Color = EDGE;
/// Selected tile, row or tab (`color.surface.selected`).
pub const TILE_SELECTED: Color = color::SURFACE_SELECTED;
/// Primary slab fills: idle, hover, pressed (`color.emerald.600/500/700`).
pub const PRIMARY: Color = color::EMERALD_600;
pub const PRIMARY_HOVER: Color = color::EMERALD_500;
pub const PRIMARY_PRESSED: Color = color::EMERALD_700;
/// Danger slab fills (`color.state.danger`, `.hover`).
pub const DANGER: Color = color::STATE_DANGER;
pub const DANGER_HOVER: Color = color::STATE_DANGER_HOVER;
/// Text colours by emphasis (`color.text.gold/primary/muted/accent`).
pub const GOLD: Color = color::TEXT_GOLD;
pub const IVORY: Color = color::TEXT_PRIMARY;
pub const MUTED: Color = color::TEXT_MUTED;
pub const JADE: Color = color::TEXT_ACCENT;
/// Dim scrim behind a modal overlay (`color.scrim`).
pub const SCRIM: Color = color::SCRIM;
/// `ButtonKind::Link` is the tertiary button: a text action without a slab
/// (`components/button.md`), so its fill is transparent.
pub const LINK: Color = Color::NONE;
pub const LINK_HOVER: Color = Color::NONE;
/// Team lock-in fills (`color.team.*.dim`, the bright tone under the pointer);
/// the team colour itself is the 4 px bar on the primary slab.
pub const TEAM_GREEN: Color = color::TEAM_GREEN_DIM;
pub const TEAM_GREEN_HOVER: Color = color::TEAM_GREEN;
pub const TEAM_BLUE: Color = color::TEAM_BLUE_DIM;
pub const TEAM_BLUE_HOVER: Color = color::TEAM_BLUE;
/// The ability "+" badge (`color.emerald.400`, `.300` under the pointer).
pub const SKILL_UPGRADE: Color = color::EMERALD_400;
pub const SKILL_UPGRADE_HOVER: Color = color::EMERALD_300;
/// A shop item card the hero already owns (`color.surface.selected`).
pub const SHOP_OWNED: Color = color::SURFACE_SELECTED;
/// `OMOBA_DEBUG_UI` toggles: off, god mode on, speed boost on (and hovers).
/// Developer UI, deliberately outside the Verdant tokens.
pub const DEBUG_OFF: Color = Color::srgba(0.18, 0.18, 0.20, 0.92);
pub const DEBUG_OFF_HOVER: Color = Color::srgba(0.26, 0.26, 0.28, 0.95);
pub const DEBUG_GOD: Color = Color::srgba(0.78, 0.20, 0.22, 0.96);
pub const DEBUG_GOD_HOVER: Color = Color::srgba(0.88, 0.28, 0.30, 0.98);
pub const DEBUG_SPEED: Color = Color::srgba(0.20, 0.44, 0.80, 0.96);
pub const DEBUG_SPEED_HOVER: Color = Color::srgba(0.28, 0.52, 0.90, 0.98);

/// Accent colours a player can put on their profile card. The name is only the
/// swatch `TestId` suffix (`CardAccent-<name>`); the card shows the colour.
/// Player content, not theme roles, so they are not tokens.
pub const ACCENTS: [(&str, Color); 6] = [
    ("Verdant", Color::srgb(0.24, 0.79, 0.58)),  // i18n-allow
    ("Ember", Color::srgb(0.90, 0.46, 0.22)),    // i18n-allow
    ("Amethyst", Color::srgb(0.58, 0.42, 0.88)), // i18n-allow
    ("Tide", Color::srgb(0.24, 0.58, 0.88)),     // i18n-allow
    ("Gold", Color::srgb(0.94, 0.77, 0.43)),     // i18n-allow
    ("Rose", Color::srgb(0.88, 0.36, 0.52)),     // i18n-allow
];

pub fn accent_color(index: usize) -> Color {
    ACCENTS[index.min(ACCENTS.len() - 1)].1
}

/// Front-end screens sit above every gameplay overlay (the highest one in the
/// gameplay UI is the legacy select root at 15).
pub const SCREEN_Z: i32 = 60;

/// Layout metrics shared by the kit widgets.
pub mod metric {
    use super::tokens::size;

    /// Smallest touch target on a phone (`size.touch_min`).
    pub const TOUCH_MIN: f32 = size::TOUCH_MIN;
    /// Menu button height (`size.button.height.desktop`).
    pub const BUTTON_H: f32 = size::BUTTON_HEIGHT.desktop;
    /// Square `-`/`+` stepper button: 44 on both profiles (`components/stepper.md`).
    pub const ADJUST_BTN: f32 = size::TOUCH_MIN;
    /// Menu button width (legacy pause/sandbox column).
    pub const MENU_W: f32 = 320.0;

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

    /// The reference resolution every Verdant layout is authored at.
    pub const REFERENCE: (f32, f32) = (1280.0, 720.0);
    /// Desktop `UiScale` bounds (DECISIONS R2.3: `clamp(min(w/1280, h/720),
    /// 0.8, 2.0)`, "adjust the floor if text < ~11 px"). A 1024×640 capture at
    /// 0.8 put the screens' legacy 11–12 px labels at 9–10 px, so the floor
    /// is 1.0 until the screens use text roles (which keep
    /// [`DESKTOP_TEXT_FLOOR`] at any scale); screen steps lower it to 0.8.
    /// Details in `docs/ui-kit.md` (UI scale).
    pub const DESKTOP_SCALE_MIN: f32 = 1.0;
    pub const DESKTOP_SCALE_MAX: f32 = 2.0;
    /// Smallest rendered (physical-looking) size of kit role text on
    /// desktop; a role below it at a small `UiScale` is raised.
    pub const DESKTOP_TEXT_FLOOR: f32 = 11.0;

    /// Desktop `UiScale` for a logical window size:
    /// `clamp(min(w / 1280, h / 720), DESKTOP_SCALE_MIN, 2.0)` (1920×1080 →
    /// 1.5, 1280×720 and smaller → 1.0). Phones keep `frontend::menu_scale`.
    pub fn desktop_ui_scale(width: f32, height: f32) -> f32 {
        if width <= 0.0 || height <= 0.0 {
            return 1.0;
        }
        (width / REFERENCE.0)
            .min(height / REFERENCE.1)
            .clamp(DESKTOP_SCALE_MIN, DESKTOP_SCALE_MAX)
    }

    /// Smallest readable front-end font on a phone, in physical-looking px
    /// (divided by `UiScale` by [`menu_font`]); `type.heading.size.phone` and
    /// `type.caption.size.phone`.
    pub const PHONE_HEADING_MIN: f32 = super::TextRole::Heading.style().size.phone;
    pub const PHONE_LABEL_MIN: f32 = super::TextRole::Caption.style().size.phone;

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

pub use metric::Form;

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
    /// The tertiary button: a low-emphasis text action without a slab
    /// (external targets add the `nav/link` icon).
    Link,
    /// A team's lock-in button: the primary slab with a team-colour bar.
    Team(crate::domain::Team),
    /// A desktop combat skill-bar slot; darkens while held.
    Skill,
    /// The upgrade arrow above a skill slot (shown only when a point can be
    /// spent, so it is always the "ready" green).
    SkillUpgrade,
    /// A shop item card; `selected` means owned and wins over the pointer.
    ShopItem,
    /// An `OMOBA_DEBUG_UI` toggle; `selected` means on.
    Debug(DebugToggle),
}

/// Which debug toggle a [`ButtonKind::Debug`] button shows when on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugToggle {
    GodMode,
    SpeedBoost,
}

/// The painted slab family of a kind (`assets/frames/button-*`), if it has one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum SlabFamily {
    Primary,
    Secondary,
    Danger,
}

impl ButtonKind {
    /// Primary, Secondary, Danger and Team buttons are drawn with a 9-slice
    /// slab per state; every other kind is a native fill.
    pub fn slab(self) -> Option<SlabFamily> {
        match self {
            ButtonKind::Primary | ButtonKind::Team(_) => Some(SlabFamily::Primary),
            ButtonKind::Secondary => Some(SlabFamily::Secondary),
            ButtonKind::Danger => Some(SlabFamily::Danger),
            _ => None,
        }
    }
}

/// Interaction state of a kit control as the painter draws it. Touch has no
/// hover: a resting finger is `Hover` only through `Pressable::effective`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ButtonState {
    Idle,
    Hover,
    Pressed,
    Disabled,
}

/// Background when the pointer is away. Selected tiles keep their colour.
pub fn button_idle_color(kind: ButtonKind, selected: bool) -> Color {
    match (kind, selected) {
        (ButtonKind::ShopItem, true) => SHOP_OWNED,
        (ButtonKind::Debug(DebugToggle::GodMode), true) => DEBUG_GOD,
        (ButtonKind::Debug(DebugToggle::SpeedBoost), true) => DEBUG_SPEED,
        (ButtonKind::Link, _) => LINK,
        (_, true) => TILE_SELECTED,
        (ButtonKind::Primary, false) => PRIMARY,
        (ButtonKind::Secondary | ButtonKind::Tile | ButtonKind::ShopItem, false) => TILE,
        (ButtonKind::Danger, false) => DANGER,
        (ButtonKind::Team(crate::domain::Team::Green), false) => TEAM_GREEN,
        (ButtonKind::Team(crate::domain::Team::Blue), false) => TEAM_BLUE,
        (ButtonKind::Skill, false) => PANEL,
        (ButtonKind::SkillUpgrade, false) => SKILL_UPGRADE,
        (ButtonKind::Debug(_), false) => DEBUG_OFF,
    }
}

/// Background while hovered (pointer only).
pub fn button_hover_color(kind: ButtonKind, selected: bool) -> Color {
    match (kind, selected) {
        (ButtonKind::ShopItem, true) => SHOP_OWNED,
        (ButtonKind::Debug(DebugToggle::GodMode), true) => DEBUG_GOD_HOVER,
        (ButtonKind::Debug(DebugToggle::SpeedBoost), true) => DEBUG_SPEED_HOVER,
        (ButtonKind::Link, _) => LINK_HOVER,
        (_, true) => TILE_SELECTED,
        (ButtonKind::Primary, false) => PRIMARY_HOVER,
        (ButtonKind::Secondary | ButtonKind::Tile | ButtonKind::ShopItem, false) => HOVER,
        (ButtonKind::Danger, false) => DANGER_HOVER,
        (ButtonKind::Team(crate::domain::Team::Green), false) => TEAM_GREEN_HOVER,
        (ButtonKind::Team(crate::domain::Team::Blue), false) => TEAM_BLUE_HOVER,
        (ButtonKind::Skill, false) => HOVER,
        (ButtonKind::SkillUpgrade, false) => SKILL_UPGRADE_HOVER,
        (ButtonKind::Debug(_), false) => DEBUG_OFF_HOVER,
    }
}

/// Background while held: the slabs' pressed tone (`color.emerald.700`,
/// `color.surface.0`), the skill slots and unowned shop cards darken to
/// `TILE` as they always did, the rest keep their hover colour.
pub fn button_pressed_color(kind: ButtonKind, selected: bool) -> Color {
    match (kind, selected) {
        (ButtonKind::Skill, _) | (ButtonKind::ShopItem, false) => TILE,
        (ButtonKind::Primary, false) => PRIMARY_PRESSED,
        (ButtonKind::Secondary | ButtonKind::Tile, false) => color::SURFACE_0,
        _ => button_hover_color(kind, selected),
    }
}

/// Label colour of a kit button in a state (`components/button.md`,
/// `tab.md`): on-primary on emerald, gold when a secondary is selected, the
/// tertiary brightens to gold under the pointer, disabled is always
/// `color.text.disabled`.
pub fn button_label_color(kind: ButtonKind, selected: bool, state: ButtonState) -> Color {
    if state == ButtonState::Disabled {
        return color::TEXT_DISABLED;
    }
    match kind {
        ButtonKind::Primary | ButtonKind::Team(_) | ButtonKind::SkillUpgrade => {
            color::TEXT_ON_PRIMARY
        }
        ButtonKind::Secondary if selected => color::TEXT_GOLD,
        ButtonKind::Link => match state {
            ButtonState::Hover => color::TEXT_GOLD,
            ButtonState::Pressed => color::GOLD_600,
            _ if selected => color::TEXT_PRIMARY,
            _ => color::TEXT_SECONDARY,
        },
        ButtonKind::Tile if selected => color::TEXT_GOLD,
        ButtonKind::Tile => match state {
            ButtonState::Hover | ButtonState::Pressed => color::TEXT_PRIMARY,
            _ => color::TEXT_SECONDARY,
        },
        _ => color::TEXT_PRIMARY,
    }
}

/// Border of a native (slab-less) kit control: hairline `border.subtle`,
/// `gold.600` under the pointer, `gold.500` when selected,
/// `border.disabled` when disabled.
pub fn native_border_color(selected: bool, state: ButtonState) -> Color {
    match (state, selected) {
        (ButtonState::Disabled, _) => color::BORDER_DISABLED,
        (_, true) => color::GOLD_500,
        (ButtonState::Hover, false) => color::GOLD_600,
        _ => color::BORDER_SUBTLE,
    }
}

/// A translucent token as Bevy should draw it to look like the handoff.
///
/// The handoff sheets are rendered by a browser, which blends in sRGB; Bevy
/// blends in linear light, so a translucent colour reads lighter over dark
/// content (a 65 % black cooldown veil darkens by only ~38 %) and a light
/// accent reads stronger (the 25 % focus halo looks like ~50 %). The alpha
/// is remapped with the display gamma (2.2) for the colour's side: dark
/// colours `1 − (1 − a)^2.2` (exact for black over any content), light ones
/// `a^2.2` (exact over black, close over the kit's dark surfaces). Opaque
/// colours are unchanged. The token data stays as designed.
pub fn perceptual(color: Color) -> Color {
    const GAMMA: f32 = 2.2;
    let srgba = color.to_srgba();
    let alpha = srgba.alpha;
    if alpha <= 0.0 || alpha >= 1.0 {
        return color;
    }
    let luminance = 0.2126 * srgba.red + 0.7152 * srgba.green + 0.0722 * srgba.blue;
    let alpha = if luminance < 0.5 {
        1.0 - (1.0 - alpha).powf(GAMMA)
    } else {
        alpha.powf(GAMMA)
    };
    color.with_alpha(alpha)
}

/// The packaged fonts: the legacy default (Inter variable, for text without
/// a role), the CJK fallback and one handle per `font.family.*` token.
#[derive(Resource)]
pub(crate) struct UiTheme {
    pub font: Handle<Font>,
    pub cjk_font: Handle<Font>,
    pub families: Vec<(FontFamily, Handle<Font>)>,
}

impl UiTheme {
    pub(crate) fn family(&self, family: FontFamily) -> Handle<Font> {
        self.families
            .iter()
            .find(|(candidate, _)| *candidate == family)
            .map_or_else(|| self.font.clone(), |(_, handle)| handle.clone())
    }
}

pub(super) fn load_theme(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(UiTheme {
        font: assets.load("ui/Inter.ttf"),
        cjk_font: assets.load(FontFamily::CjkBody.asset_path()),
        families: FontFamily::ALL
            .into_iter()
            .map(|family| (family, assets.load(family.asset_path())))
            .collect(),
    });
}

pub(crate) fn needs_cjk_font(text: &str) -> bool {
    text.chars().any(is_cjk)
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32,
    0x1100..=0x11ff | 0x2e80..=0xa4cf | 0xa960..=0xa97f |
    0xac00..=0xd7ff | 0xf900..=0xfaff | 0xfe30..=0xfe4f |
    0xff00..=0xffef | 0x20000..=0x323af)
}

/// Picks the packaged Latin/Cyrillic font or the CJK fallback for every UI
/// text, span and world-space `Text2d` whose text changed and which has no
/// kit [`TextStyle`] (those are finished by [`apply_text_styles`]). Runs in
/// `PostUpdate` in `I18nSystems::Font`, after every `Update` text writer and
/// the `Localized` relabel, before UI and `Text2d` layout, so a text never
/// renders a frame in the wrong font.
pub(crate) fn apply_theme_font(
    theme: Res<UiTheme>,
    mut text: Query<
        (
            &mut TextFont,
            Option<&Text>,
            Option<&TextSpan>,
            Option<&Text2d>,
        ),
        (
            Without<TextStyle>,
            Or<(
                Added<TextFont>,
                Changed<Text>,
                Changed<TextSpan>,
                Changed<Text2d>,
            )>,
        ),
    >,
) {
    for (mut font, root, span, world) in &mut text {
        let cjk = root.is_some_and(|text| needs_cjk_font(&text.0))
            || span.is_some_and(|text| needs_cjk_font(&text.0))
            || world.is_some_and(|text| needs_cjk_font(&text.0));
        let next = if cjk { &theme.cjk_font } else { &theme.font };
        if font.font != *next {
            font.font = next.clone();
        }
    }
}

// --- Text roles ---

/// The kit's type role for a UI text: family, size per layout family, line
/// height and case from `type.<role>.*`, the CJK pair when the text needs
/// it. [`apply_text_styles`] owns the text's `TextFont` and `LineHeight`.
///
/// `keep_case` leaves the text as written: set on labels a module rewrites
/// itself (the kit never fights that writer; Cinzel draws lower case as small
/// capitals, so such labels still read as display type).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) struct TextStyle {
    pub role: TextRole,
    pub keep_case: bool,
    /// A size a component spec sets for this role (tertiary buttons use
    /// `type.button` at 14 px, tooltips `type.body` at 13 px).
    pub size: Option<tokens::Metric>,
}

impl TextStyle {
    pub(crate) const fn new(role: TextRole) -> Self {
        Self {
            role,
            keep_case: false,
            size: None,
        }
    }

    pub(crate) const fn keep_case(role: TextRole) -> Self {
        Self {
            role,
            keep_case: true,
            size: None,
        }
    }

    pub(crate) const fn sized(mut self, size: tokens::Metric) -> Self {
        self.size = Some(size);
        self
    }
}

/// The layout family kit roles and widget sizes resolve for. Starts from
/// [`super::UiPlatform`]; the kit gallery switches it to preview the other
/// profile.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct UiForm(pub Form);

/// Characters a face has (its `cmap`), read once the font loaded. Display
/// faces (Cinzel, the Serif SC subset) are small; a text with a character
/// they lack is set in the fallback face instead (tokens.md, data strings).
#[derive(Resource, Default)]
pub(crate) struct FontCoverage {
    faces: Vec<(FontFamily, HashSet<char>)>,
}

impl FontCoverage {
    /// `None` until the face loaded (then nothing is known to be missing).
    fn covers(&self, family: FontFamily, text: &str) -> Option<bool> {
        let (_, chars) = self.faces.iter().find(|(face, _)| *face == family)?;
        Some(
            text.chars()
                .filter(|character| !character.is_whitespace())
                .all(|character| chars.contains(&character)),
        )
    }
}

/// Faces whose coverage decides a fallback: Latin display (Cinzel) and the
/// zh-Hans display subset. Barlow, Inter and Noto Sans CJK are complete for
/// what they are used for.
const LIMITED_FACES: [FontFamily; 3] = [
    FontFamily::Display,
    FontFamily::DisplayBold,
    FontFamily::CjkDisplay,
];

pub(super) fn index_font_coverage(
    theme: Option<Res<UiTheme>>,
    fonts: Res<Assets<Font>>,
    mut events: MessageReader<AssetEvent<Font>>,
    mut coverage: ResMut<FontCoverage>,
    mut texts: Query<&mut TextStyle>,
) {
    let Some(theme) = theme else { return };
    let mut indexed = false;
    for event in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = event else {
            continue;
        };
        for family in LIMITED_FACES {
            if theme.family(family).id() != *id
                || coverage.faces.iter().any(|(face, _)| *face == family)
            {
                continue;
            }
            if let Some(font) = fonts.get(*id) {
                coverage
                    .faces
                    .push((family, super::font_cmap::characters(&font.data)));
                indexed = true;
            }
        }
    }
    if indexed {
        // Re-resolve every role text against the new coverage.
        for mut style in &mut texts {
            style.set_changed();
        }
    }
}

/// The face, size, line height and case a role text gets.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct ResolvedText {
    pub family: FontFamily,
    pub size: f32,
    pub line_height: f32,
    /// The text to show when the role transforms case.
    pub text: Option<String>,
}

/// Resolves `style` for `text` (pure; [`apply_text_styles`] applies it).
///
/// - CJK text uses the role's CJK face when that face has every character
///   (the Serif SC subset only holds the dictionaries' characters), else
///   Noto Sans CJK SC; number roles keep Barlow only for text without CJK.
/// - Latin text in a display role falls back to `type.name_lg`'s face at the
///   same size when Cinzel lacks a character (Cyrillic or accented tags).
/// - Upper case applies to the role's own Latin face only, never to CJK.
/// - Size: `size.phone` on a phone with the `menu_font` minimums; on desktop
///   never below [`metric::DESKTOP_TEXT_FLOOR`] rendered px at `UiScale`.
pub(crate) fn resolve_text(
    style: TextStyle,
    text: &str,
    form: Form,
    ui_scale: f32,
    coverage: &FontCoverage,
) -> ResolvedText {
    let role = style.role.style();
    let cjk = needs_cjk_font(text);
    let family = if cjk {
        let cjk_face = matches!(
            role.cjk_family,
            FontFamily::CjkDisplay | FontFamily::CjkBody
        );
        let cjk_text: String = text.chars().filter(|c| is_cjk(*c)).collect();
        if cjk_face && coverage.covers(role.cjk_family, &cjk_text) != Some(false) {
            role.cjk_family
        } else {
            FontFamily::CjkBody
        }
    } else if coverage.covers(role.family, text) == Some(false) {
        TextRole::NameLg.style().family
    } else {
        role.family
    };
    let base = style.size.unwrap_or(role.size);
    let size = match form {
        Form::Phone => metric::menu_font(
            Form::Phone,
            base.phone,
            matches!(
                style.role,
                TextRole::TitleXl | TextRole::Title | TextRole::Heading
            ),
            ui_scale,
        ),
        Form::Desktop => base
            .desktop
            .max(metric::DESKTOP_TEXT_FLOOR / metric::ui_scale(ui_scale)),
    };
    let upper = role.case == TextCase::Upper && !style.keep_case && family == role.family;
    let cased = upper
        .then(|| text.to_uppercase())
        .filter(|cased| cased != text);
    ResolvedText {
        family,
        size,
        line_height: role.line_height,
        text: cased,
    }
}

/// Applies [`TextStyle`] roles: face, size, `LineHeight` and case. Runs in
/// `I18nSystems::Font` like [`apply_theme_font`] (after the relabel and every
/// `Update` writer, before layout) on added/changed text and whenever the
/// layout family or `UiScale` changes. A size the phone layout owns
/// (`mobile_ui`'s `PhoneFontSize`, legacy panels) is left to it.
#[allow(clippy::type_complexity)]
pub(crate) fn apply_text_styles(
    theme: Res<UiTheme>,
    form: Res<UiForm>,
    scale: Option<Res<UiScale>>,
    coverage: Res<FontCoverage>,
    mut texts: Query<(
        Ref<TextStyle>,
        &mut Text,
        &mut TextFont,
        Option<&mut bevy::text::LineHeight>,
        Has<PhoneSized>,
        Entity,
    )>,
    mut commands: Commands,
) {
    let all = form.is_changed() || scale.as_ref().is_some_and(|scale| scale.is_changed());
    let ui_scale = scale.as_ref().map_or(1.0, |scale| scale.0);
    for (style, mut text, mut font, line_height, phone_sized, entity) in &mut texts {
        if !all && !style.is_changed() && !text.is_changed() {
            continue;
        }
        let resolved = resolve_text(*style, &text.0, form.0, ui_scale, &coverage);
        if let Some(cased) = resolved.text {
            text.0 = cased;
        }
        let handle = theme.family(resolved.family);
        if font.font != handle {
            font.font = handle;
        }
        if !phone_sized && font.font_size != resolved.size {
            font.font_size = resolved.size;
        }
        let next = bevy::text::LineHeight::RelativeToFont(resolved.line_height);
        match line_height {
            Some(mut current) if *current != next => *current = next,
            Some(_) => {}
            None => {
                commands.entity(entity).insert(next);
            }
        }
    }
}

/// Marks a text whose size a phone layout pass owns (see
/// [`apply_text_styles`]); `mobile_ui` inserts it with its original size.
#[derive(Component)]
pub(crate) struct PhoneSized;

/// A `TextFont` at `size` in the default face (legacy helper; kit widgets use
/// [`TextStyle`] roles).
pub(crate) fn text(size: f32) -> TextFont {
    TextFont {
        font_size: size,
        ..default()
    }
}

/// `TextFont` + [`TextStyle`] for a role, sized for the desktop until
/// [`apply_text_styles`] resolves it.
pub(crate) fn role_text(role: TextRole) -> (TextFont, TextStyle) {
    styled_text(TextStyle::new(role))
}

/// `TextFont` + a [`TextStyle`] (with its own size or case rule).
pub(crate) fn styled_text(style: TextStyle) -> (TextFont, TextStyle) {
    let size = style.size.unwrap_or(style.role.style().size).desktop;
    (text(size), style)
}

/// The plain panel (`components/panel.md`): padding `space.16`, hairline
/// border, `radius.lg`. Colours are set by the caller (`PANEL`, `EDGE`).
pub(crate) fn panel_node() -> Node {
    Node {
        padding: UiRect::all(Val::Px(space::S16)),
        border: UiRect::all(Val::Px(tokens::border::HAIRLINE)),
        border_radius: BorderRadius::all(Val::Px(radius::LG)),
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
                    .join("assets")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn theme_with(fonts: &mut Assets<Font>) -> UiTheme {
        let font = fonts.add(packaged_font("ui/Inter.ttf"));
        let cjk_font = fonts.add(packaged_font("ui/NotoSansCJKsc-Regular.otf"));
        UiTheme {
            font,
            cjk_font,
            families: FontFamily::ALL
                .into_iter()
                .map(|family| (family, fonts.add(packaged_font(family.asset_path()))))
                .collect(),
        }
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
        let theme = theme_with(&mut fonts);
        let (font, cjk_font) = (theme.font.clone(), theme.cjk_font.clone());
        let mut app = App::new();
        app.insert_resource(theme)
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

    /// World-space labels (lane and boss plates, nameplates) switch fonts
    /// too, and a text written in `Update` gets its font in the same frame's
    /// `PostUpdate`, before layout.
    #[test]
    fn text2d_and_update_writers_get_the_cjk_font_in_the_same_frame() {
        let mut fonts = Assets::<Font>::default();
        let theme = theme_with(&mut fonts);
        let (font, cjk_font) = (theme.font.clone(), theme.cjk_font.clone());
        let mut app = App::new();
        app.insert_resource(theme)
            .init_resource::<crate::i18n::Locale>();
        crate::i18n::configure_text_sets(&mut app);
        app.add_systems(
            PostUpdate,
            (
                crate::i18n::relabel_localized.in_set(crate::i18n::I18nSystems::Relabel),
                apply_theme_font.in_set(crate::i18n::I18nSystems::Font),
            ),
        );
        let lane = app
            .world_mut()
            .spawn((
                crate::i18n::Localized::new("lane.top").text2d(),
                TextFont::default(),
            ))
            .id();
        let plate = app
            .world_mut()
            .spawn((Text2d::new("Wendigo"), TextFont::default()))
            .id();
        app.update();
        assert_eq!(app.world().get::<TextFont>(lane).unwrap().font, font);
        let zh = crate::i18n::LocaleId::parse("zh-Hans").unwrap();
        app.world_mut()
            .resource_mut::<crate::i18n::Locale>()
            .set(zh);
        app.world_mut().get_mut::<Text2d>(plate).unwrap().0 = "温迪戈".into();
        app.update();
        assert_eq!(app.world().get::<Text2d>(lane).unwrap().0, "上路");
        assert_eq!(app.world().get::<TextFont>(lane).unwrap().font, cjk_font);
        assert_eq!(app.world().get::<TextFont>(plate).unwrap().font, cjk_font);
    }

    fn coverage_of(families: &[FontFamily]) -> FontCoverage {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        FontCoverage {
            faces: families
                .iter()
                .map(|family| {
                    let bytes = std::fs::read(root.join(family.asset_path())).unwrap();
                    (*family, super::super::font_cmap::characters(&bytes))
                })
                .collect(),
        }
    }

    /// Roles pick the Latin face and case, the paired CJK face for Chinese
    /// (never upper-casing it), the body face for tags Cinzel cannot draw,
    /// and the profile's size with the readability floors.
    #[test]
    fn text_roles_pair_cjk_faces_and_resolve_sizes_per_profile() {
        let coverage = coverage_of(&LIMITED_FACES);
        let resolve = |role, text: &str, form, scale| {
            resolve_text(TextStyle::new(role), text, form, scale, &coverage)
        };
        let play = resolve(TextRole::ButtonLg, "Play again", Form::Desktop, 1.0);
        assert_eq!(play.family, FontFamily::DisplayBold);
        assert_eq!(play.text.as_deref(), Some("PLAY AGAIN"));
        assert_eq!(play.size, 22.0);
        assert_eq!(play.line_height, 1.0);
        let zh = resolve(TextRole::ButtonLg, "再来一局", Form::Desktop, 1.0);
        assert_eq!(zh.family, FontFamily::CjkDisplay);
        assert_eq!(zh.text, None, "no case transform on CJK");
        // A character outside the Serif SC subset (a player name) falls back.
        let name = resolve(TextRole::Heading, "龘", Form::Desktop, 1.0);
        assert_eq!(name.family, FontFamily::CjkBody);
        // Body text in Chinese uses Noto Sans CJK; numbers keep Barlow.
        assert_eq!(
            resolve(TextRole::Body, "选择英雄", Form::Desktop, 1.0).family,
            FontFamily::CjkBody
        );
        assert_eq!(
            resolve(TextRole::Number, "1 250", Form::Desktop, 1.0).family,
            FontFamily::Number
        );
        assert_eq!(
            resolve(TextRole::Number, "3 秒", Form::Desktop, 1.0).family,
            FontFamily::CjkBody
        );
        // A Cyrillic tag in a display role uses name_lg's face, as written.
        let tag = resolve(TextRole::Title, "Дмитрий", Form::Desktop, 1.0);
        assert_eq!(tag.family, FontFamily::BodySemibold);
        assert_eq!(tag.text, None);
        // keep_case leaves an owner-written label alone.
        let kept = resolve_text(
            TextStyle::keep_case(TextRole::Button),
            "Mute sound",
            Form::Desktop,
            1.0,
            &coverage,
        );
        assert_eq!(kept.text, None);
        assert_eq!(kept.family, FontFamily::Display);
        // Sizes: phone values with the menu minimums, desktop floor at 0.8.
        assert_eq!(resolve(TextRole::Title, "A", Form::Phone, 1.0).size, 24.0);
        assert_eq!(
            resolve(TextRole::Caption, "a", Form::Phone, 0.5).size,
            metric::PHONE_LABEL_MIN / 0.5
        );
        assert_eq!(resolve(TextRole::Label, "a", Form::Desktop, 1.0).size, 15.0);
        let small = resolve(TextRole::Caption, "a", Form::Desktop, 0.8).size;
        assert!((small * 0.8 - metric::DESKTOP_TEXT_FLOOR).abs() < 1e-4);
    }

    #[test]
    fn the_display_subset_covers_every_zh_hans_dictionary_character() {
        let coverage = coverage_of(&[FontFamily::CjkDisplay]);
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n/zh-Hans");
        let mut missing = std::collections::BTreeSet::new();
        for file in std::fs::read_dir(folder).unwrap() {
            let path = file.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let values: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&text).unwrap();
            // The kit gallery is a developer screen: its copy may use characters
            // outside the subset (they fall back to Noto Sans CJK SC, see
            // `text_roles_pair_cjk_faces_and_resolve_sizes_per_profile`).
            for value in values
                .iter()
                .filter(|(key, _)| !key.starts_with("kit.gallery."))
                .filter_map(|(_, value)| value.as_str())
            {
                for character in value.chars().filter(|c| is_cjk(*c)) {
                    if coverage.covers(FontFamily::CjkDisplay, &character.to_string()) != Some(true)
                    {
                        missing.insert(character);
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "NotoSerifSC-SemiBold-subset.ttf lacks {missing:?}: re-run the handoff's \
             tools/fonts.py build and scripts/sync_ui_assets.py"
        );
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

    /// R2.3: the desktop scale follows the smaller axis of the 1280×720
    /// reference, up to 2.0.
    #[test]
    fn desktop_ui_scale_follows_the_reference_resolution() {
        assert_eq!(metric::desktop_ui_scale(1280.0, 720.0), 1.0);
        assert_eq!(metric::desktop_ui_scale(1920.0, 1080.0), 1.5);
        // The floor (1.0 until the screens use text roles, see DESKTOP_SCALE_MIN).
        assert_eq!(
            metric::desktop_ui_scale(1024.0, 640.0),
            metric::DESKTOP_SCALE_MIN
        );
        assert_eq!(
            metric::desktop_ui_scale(800.0, 600.0),
            metric::DESKTOP_SCALE_MIN
        );
        const { assert!(metric::DESKTOP_SCALE_MIN >= 0.8) };
        assert_eq!(metric::desktop_ui_scale(3840.0, 2160.0), 2.0);
        // Ultra-wide: the height decides.
        assert_eq!(metric::desktop_ui_scale(2560.0, 1080.0), 1.5);
        assert_eq!(metric::desktop_ui_scale(0.0, 720.0), 1.0);
    }

    /// The skill bar, shop cards and debug toggles keep the relations their
    /// modules painted by hand before they moved onto `ButtonStyle`; the
    /// menu slabs press to their darker tone (`components/button.md`).
    #[test]
    fn hud_kinds_keep_their_hand_painted_colours() {
        use ButtonKind::*;
        let states = |kind, selected| {
            (
                button_idle_color(kind, selected),
                button_hover_color(kind, selected),
                button_pressed_color(kind, selected),
            )
        };
        assert_eq!(states(Skill, false), (PANEL, HOVER, TILE));
        assert_eq!(
            states(SkillUpgrade, false),
            (SKILL_UPGRADE, SKILL_UPGRADE_HOVER, SKILL_UPGRADE_HOVER)
        );
        assert_eq!(states(ShopItem, false), (TILE, HOVER, TILE));
        assert_eq!(states(ShopItem, true), (SHOP_OWNED, SHOP_OWNED, SHOP_OWNED));
        for toggle in [DebugToggle::GodMode, DebugToggle::SpeedBoost] {
            assert_eq!(
                states(Debug(toggle), false),
                (DEBUG_OFF, DEBUG_OFF_HOVER, DEBUG_OFF_HOVER)
            );
        }
        assert_eq!(
            states(Debug(DebugToggle::GodMode), true),
            (DEBUG_GOD, DEBUG_GOD_HOVER, DEBUG_GOD_HOVER)
        );
        assert_eq!(
            states(Debug(DebugToggle::SpeedBoost), true),
            (DEBUG_SPEED, DEBUG_SPEED_HOVER, DEBUG_SPEED_HOVER)
        );
        assert_eq!(button_pressed_color(Primary, false), PRIMARY_PRESSED);
        assert_eq!(button_pressed_color(Tile, true), TILE_SELECTED);
    }

    /// Kit state colours follow `components/*.md`.
    #[test]
    fn labels_and_borders_follow_the_component_states() {
        use ButtonKind::*;
        use ButtonState::*;
        assert_eq!(
            button_label_color(Primary, false, Idle),
            color::TEXT_ON_PRIMARY
        );
        assert_eq!(button_label_color(Secondary, true, Idle), color::TEXT_GOLD);
        assert_eq!(
            button_label_color(Secondary, false, Idle),
            color::TEXT_PRIMARY
        );
        assert_eq!(button_label_color(Link, false, Idle), color::TEXT_SECONDARY);
        assert_eq!(button_label_color(Link, false, Hover), color::TEXT_GOLD);
        assert_eq!(button_label_color(Tile, true, Idle), color::TEXT_GOLD);
        assert_eq!(button_label_color(Tile, false, Hover), color::TEXT_PRIMARY);
        for kind in [Primary, Secondary, Danger, Link, Tile] {
            assert_eq!(
                button_label_color(kind, false, Disabled),
                color::TEXT_DISABLED
            );
        }
        assert_eq!(native_border_color(true, Idle), color::GOLD_500);
        assert_eq!(native_border_color(false, Hover), color::GOLD_600);
        assert_eq!(native_border_color(false, Idle), color::BORDER_SUBTLE);
        assert_eq!(native_border_color(true, Disabled), color::BORDER_DISABLED);
        assert_eq!(Primary.slab(), Some(SlabFamily::Primary));
        assert_eq!(
            Team(crate::domain::Team::Blue).slab(),
            Some(SlabFamily::Primary)
        );
        assert_eq!(Tile.slab(), None);
        assert_eq!(Link.slab(), None);
    }

    #[test]
    fn perceptual_alpha_matches_srgb_compositing() {
        // Black veils: 65 % → ~90 %; light accents: 25 % → ~4.7 %.
        let veil = perceptual(color::COOLDOWN_OVERLAY).alpha();
        assert!((veil - (1.0 - 0.35_f32.powf(2.2))).abs() < 1e-3, "{veil}");
        let halo = perceptual(color::FOCUS_HALO).alpha();
        assert!(
            (halo - (0x40 as f32 / 255.0).powf(2.2)).abs() < 1e-3,
            "{halo}"
        );
        assert_eq!(perceptual(color::TEXT_GOLD), color::TEXT_GOLD);
        assert_eq!(perceptual(Color::NONE), Color::NONE);
        // Over black, the linear blend of the remapped light accent equals the
        // sRGB blend of the original (the browser's).
        let accent = color::FOCUS_HALO.to_srgba();
        let srgb_result = accent.red * accent.alpha;
        let linear = bevy::color::LinearRgba::from(color::FOCUS_HALO.with_alpha(1.0)).red
            * perceptual(color::FOCUS_HALO).alpha();
        let back = Color::linear_rgb(linear, 0.0, 0.0).to_srgba().red;
        assert!((back - srgb_result).abs() < 0.01, "{back} vs {srgb_result}");
    }

    #[test]
    fn accents_are_addressable_and_clamped() {
        assert_eq!(accent_color(0), ACCENTS[0].1);
        assert_eq!(accent_color(99), ACCENTS[ACCENTS.len() - 1].1);
    }
}
