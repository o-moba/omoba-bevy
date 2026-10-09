//! The controls guide (`omoba-ui/handoff/screens/hud-help.md`): illustrated
//! cards over the running match or a menu. Does not despawn gameplay entities.
//!
//! Desktop: the keyboard and mouse guide as six cards (icon disc, title, body
//! and an input legend of keycaps and mouse chips), a field/camera strip, the
//! dismiss button and the reopen hint in a framed panel. Phone: the touch
//! guide as ten cards in a vertical scroll list under a title row that holds
//! the dismiss button. Every text is a `Localized` key (`help.card.*`,
//! `help.phone.card.*`), so the guide follows a language change while it is
//! open. The overlay is a modal (`ModalId::Help`) while it is shown, so a
//! controller focuses its dismiss button and nothing under it reacts.
// i18n-strict

use bevy::{prelude::*, window::PrimaryWindow};

use crate::i18n::Localized;
use crate::input_bindings::{
    HELP_TOGGLE_KEY, SKILL_SLOT_KEY_LABELS, help_key_display, skill_keys_display,
    upgrade_key_display,
};
use crate::net::{ClientSession, GameState, GameStateSnapshot};
use crate::ui::kit_assets::Icon;
use crate::ui::theme::{self, ButtonKind, Form, TextStyle};
use crate::ui::tokens::{TextRole, border, color, motion, radius, size, space};
use crate::ui::widgets::surfaces::{self, BadgeKind, InfoCard};
use crate::ui::widgets::{ButtonSize, button_node, spawn_button};
use crate::ui::{Activated, ModalId, ModalRoot, ScrollArea, TestId, UiActionAppExt};

pub struct HelpOverlayPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HelpOverlaySet {
    Input,
}

impl Plugin for HelpOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HelpOverlayVisible>()
            .init_resource::<HelpOverlayShown>()
            .init_resource::<HelpAutoShowState>()
            .add_ui_action::<HelpAction>()
            .add_systems(Startup, setup_help_overlay)
            .add_systems(
                Update,
                (
                    auto_show_help_on_first_match_start,
                    toggle_help_overlay,
                    dismiss_help_button,
                    sync_help_overlay_visibility,
                )
                    .chain()
                    .in_set(HelpOverlaySet::Input)
                    .after(crate::ui::UiSet::Dispatch)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                PostUpdate,
                (layout_help_overlay, sync_help_scroll_thumb)
                    .chain()
                    .before(bevy::ui::UiSystems::Layout),
            );
    }
}

/// One-time prompt when the match first enters Running (session scope).
#[derive(Resource)]
struct HelpAutoShowState {
    pending: bool,
    was_running: bool,
}

impl Default for HelpAutoShowState {
    fn default() -> Self {
        Self {
            pending: true,
            was_running: false,
        }
    }
}

fn auto_show_help_on_first_match_start(
    snapshot: Res<GameStateSnapshot>,
    session: Option<Res<ClientSession>>,
    mut state: ResMut<HelpAutoShowState>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    let running = matches!(snapshot.state, GameState::Running)
        && session
            .as_ref()
            .is_none_or(|session| session.join_confirmed());
    if running && !state.was_running && state.pending && !crate::sandbox::requested() {
        visible.0 = true;
        state.pending = false;
    }
    state.was_running = running;
}

/// The player asked for the guide (first match, F1, a Help button).
#[derive(Resource, Default)]
pub struct HelpOverlayVisible(pub bool);

/// The guide is on screen: requested, and over a running match or a menu.
/// Drives its `ModalId::Help` entry in the modal stack.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(crate) struct HelpOverlayShown(pub bool);

#[derive(Component)]
struct HelpOverlayRoot;

/// The guide's panel (framed on desktop, plain on a phone).
#[derive(Component)]
struct HelpOverlayPanel;

#[derive(Component)]
struct HelpDismissButton;

/// The phone card list (a touch scroll area) and its scroll thumb.
#[derive(Component)]
struct HelpCardList;
#[derive(Component)]
struct HelpScrollThumb;

/// Seconds since the guide last opened (the `motion.duration.panel_open`
/// scale-in and scrim fade).
#[derive(Component, Default)]
struct HelpOpening {
    elapsed: f32,
}

/// The overlay's one button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelpAction {
    Dismiss,
}

/// The layout every size in this module is authored at (`hud-help.md`).
const DESKTOP_PANEL: Vec2 = Vec2::new(1088.0, 672.0);
const DESKTOP_CARD: Vec2 = Vec2::new(336.0, 192.0);
/// Row gap of the desktop card grid (cards at y 124 and 340).
const DESKTOP_CARD_ROW_GAP: f32 = space::S24;
/// Header heights: eyebrow 16, title 40 (y 48 / 68, grid at 124).
const DESKTOP_EYEBROW_H: f32 = 16.0;
const DESKTOP_TITLE_H: f32 = 40.0;
/// The field/camera strip (y 548, 52 high); its columns are one card and
/// two cards wide.
const DESKTOP_STRIP_H: f32 = 52.0;
const DESKTOP_DISMISS_W: f32 = 360.0;
/// The reopen hint beside the dismiss button (x 836, y 625, 324 × 34).
const DESKTOP_HINT: Rect = Rect {
    min: Vec2::new(716.0, 13.0),
    max: Vec2::new(1040.0, 47.0),
};
/// Phone: the panel sits `space.screen_margin.phone` inside the safe area,
/// 12 from the top and the safe bottom; title row 44 with a 240-wide
/// dismiss; cards 96 high in two columns; the closing line 34 high.
const PHONE_PANEL_INSET_Y: f32 = space::S12;
const PHONE_DISMISS_W: f32 = 240.0;
const PHONE_CARD_H: f32 = 96.0;
const PHONE_FOOTER_H: f32 = 34.0;
/// Scroll thumb: 4 wide in the panel's right padding, `color.gold.600`.
const PHONE_THUMB_W: f32 = space::S4;

/// Gap between legend entries (the redline's 6 px).
const LEGEND_GAP: f32 = 6.0;

/// A control legend entry at the bottom of a desktop card.
#[derive(Clone, Copy)]
enum Legend {
    /// A mouse input (`help.input.*`), drawn as a muted badge.
    Mouse(&'static str),
    /// A key glyph, drawn as a keycap.
    Key(&'static str),
    /// A small gap between key groups.
    Gap,
}

/// One desktop card: id (for `Name`), icon, title and body keys, legend.
struct DesktopCard {
    id: &'static str,
    icon: Icon,
    title: &'static str,
    body: &'static str,
    legend: &'static [Legend],
}

const DESKTOP_CARDS: [DesktopCard; 6] = [
    DesktopCard {
        id: "move",
        icon: Icon::HudDash,
        title: "help.card.move.title",
        body: "help.card.move.body",
        legend: &[Legend::Mouse("help.input.right_click")],
    },
    DesktopCard {
        id: "attack",
        icon: Icon::HudAttack,
        title: "help.card.attack.title",
        body: "help.card.attack.body",
        legend: &[Legend::Mouse("help.input.right_click")],
    },
    DesktopCard {
        id: "target",
        icon: Icon::NavSearch,
        title: "help.card.target.title",
        body: "help.card.target.body",
        legend: &[
            Legend::Mouse("help.input.left_click"),
            Legend::Key("Tab"),       // i18n-allow: key names
            Legend::Key("Backspace"), // i18n-allow: key names
            Legend::Key("S"),
        ],
    },
    DesktopCard {
        id: "abilities",
        icon: Icon::HudLevelUp,
        title: "help.card.abilities.title",
        body: "help.card.abilities.body",
        // Filled from the bindings (`SKILL_SLOT_KEY_LABELS`, upgrade key).
        legend: &[],
    },
    DesktopCard {
        id: "shop",
        icon: Icon::NavShoppingBag,
        title: "help.card.shop.title",
        body: "help.card.shop.body",
        legend: &[Legend::Key("P")],
    },
    DesktopCard {
        id: "objective",
        icon: Icon::HudTower,
        title: "help.card.objective.title",
        body: "help.card.objective.body",
        legend: &[],
    },
];

/// The phone cards: id, icon, title and body keys (one dictionary line each).
const PHONE_CARDS: [(&str, Icon, &str, &str); 10] = [
    (
        "move",
        Icon::HudDash,
        "help.phone.card.move.title",
        "help.phone.card.move.body",
    ),
    (
        "attack",
        Icon::HudAttack,
        "help.phone.card.attack.title",
        "help.phone.card.attack.body",
    ),
    (
        "target",
        Icon::NavSearch,
        "help.phone.card.target.title",
        "help.phone.card.target.body",
    ),
    (
        "farm",
        Icon::HudMinion,
        "help.phone.card.farm.title",
        "help.phone.card.farm.body",
    ),
    (
        "utility",
        Icon::HudHaste,
        "help.phone.card.utility.title",
        "help.phone.card.utility.body",
    ),
    (
        "skills",
        Icon::HudLevelUp,
        "help.phone.card.skills.title",
        "help.phone.card.skills.body",
    ),
    (
        "grow",
        Icon::NavPlus,
        "help.phone.card.grow.title",
        "help.phone.card.grow.body",
    ),
    (
        "win",
        Icon::HudTower,
        "help.phone.card.win.title",
        "help.phone.card.win.body",
    ),
    (
        "recover",
        Icon::HudRecall,
        "help.phone.card.recover.title",
        "help.phone.card.recover.body",
    ),
    (
        "look",
        Icon::NavEye,
        "help.phone.card.look.title",
        "help.phone.card.look.body",
    ),
];

/// A card body in the active language, with the build's bindings filled in.
fn card_body(key: &'static str) -> Localized {
    match key {
        "help.card.abilities.body" => Localized::with_args(
            "help.card.abilities.body",
            [
                ("skills", &skill_keys_display()),
                ("upgrade", &upgrade_key_display()),
            ],
        ),
        key => Localized::new(key),
    }
}

/// The reopen hint with the help key filled in.
fn reopen_hint() -> Localized {
    Localized::with_args("help.reopen", [("help_key", &help_key_display())])
}

fn setup_help_overlay(mut commands: Commands, platform: Option<Res<crate::ui::UiPlatform>>) {
    let phone = platform.is_some_and(|platform| platform.is_mobile());
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                display: Display::None,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            ZIndex(ModalId::Help.layer()),
            BackgroundColor(theme::perceptual(color::SCRIM)),
            ModalRoot(ModalId::Help),
            HelpOverlayRoot,
            Name::new("HelpOverlayRoot"),
        ))
        .with_children(|root| {
            if phone {
                spawn_phone_guide(root);
            } else {
                spawn_desktop_guide(root);
            }
        });
}

/// The dismiss button (`HelpDismissButton`, label `HelpDismissLabel`).
fn spawn_dismiss(parent: &mut ChildSpawnerCommands, form: Form) {
    let (node, label, style) = match form {
        Form::Desktop => (
            Node {
                width: Val::Px(DESKTOP_DISMISS_W),
                ..button_node(ButtonSize::Large, ButtonKind::Primary, form)
            },
            Localized::new("help.dismiss.button"),
            TextStyle::new(TextRole::ButtonLg),
        ),
        Form::Phone => (
            Node {
                width: Val::Px(PHONE_DISMISS_W),
                ..button_node(ButtonSize::Regular, ButtonKind::Primary, form)
            },
            Localized::new("help.phone.dismiss"),
            TextStyle::new(TextRole::Button),
        ),
    };
    let button = spawn_button(
        parent,
        node,
        label,
        style,
        ButtonKind::Primary,
        None,
        HelpAction::Dismiss,
        TestId::new("HelpDismissButton"),
        Name::new("HelpDismissLabel"),
    );
    parent.commands().entity(button).insert(HelpDismissButton);
}

fn spawn_desktop_guide(root: &mut ChildSpawnerCommands) {
    let form = Form::Desktop;
    root.spawn((
        surfaces::framed_panel(form),
        HelpOverlayPanel,
        HelpOpening::default(),
        UiTransform::default(),
        Name::new("HelpPanel"),
    ))
    .insert(Node {
        width: Val::Px(DESKTOP_PANEL.x),
        height: Val::Px(DESKTOP_PANEL.y),
        flex_shrink: 0.0,
        padding: UiRect::all(Val::Px(surfaces::FRAMED_PADDING.at(form))),
        flex_direction: FlexDirection::Column,
        ..default()
    })
    .with_children(|panel| {
        // Bevy justifies text within its widest line, so a single line is
        // centred by its row.
        for (key, role, tint, height, gap, name) in [
            (
                "help.eyebrow",
                TextRole::Eyebrow,
                color::TEXT_MUTED,
                DESKTOP_EYEBROW_H,
                space::S4,
                "HelpEyebrow",
            ),
            (
                "help.title",
                TextRole::Title,
                color::TEXT_GOLD,
                DESKTOP_TITLE_H,
                space::S16,
                "HelpTitle",
            ),
        ] {
            panel
                .spawn((
                    Node {
                        height: Val::Px(height),
                        margin: UiRect::bottom(Val::Px(gap)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        flex_shrink: 0.0,
                        ..default()
                    },
                    Name::new(name),
                ))
                .with_child((
                    Localized::new(key).into_text(),
                    theme::role_text(role),
                    TextColor(tint),
                    TextLayout::new(Justify::Center, LineBreak::NoWrap),
                ));
        }
        panel
            .spawn((
                Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px(3, DESKTOP_CARD.x),
                    grid_auto_rows: vec![GridTrack::px(DESKTOP_CARD.y)],
                    column_gap: Val::Px(space::S16),
                    row_gap: Val::Px(DESKTOP_CARD_ROW_GAP),
                    flex_shrink: 0.0,
                    ..default()
                },
                Name::new("HelpBody"),
            ))
            .with_children(|grid| {
                for card in &DESKTOP_CARDS {
                    spawn_desktop_card(grid, card);
                }
            });
        panel
            .spawn((
                Node {
                    height: Val::Px(DESKTOP_STRIP_H),
                    margin: UiRect::vertical(Val::Px(space::S16)).with_bottom(Val::Px(space::S12)),
                    column_gap: Val::Px(space::S16),
                    flex_shrink: 0.0,
                    ..default()
                },
                Name::new("HelpFieldAndCamera"),
            ))
            .with_children(|strip| {
                strip_entry(
                    strip,
                    DESKTOP_CARD.x,
                    "help.card.field.title",
                    "help.card.field.body",
                    "HelpField",
                );
                strip_entry(
                    strip,
                    DESKTOP_CARD.x * 2.0 + space::S16,
                    "help.card.camera.title",
                    "help.card.camera.body",
                    "HelpCamera",
                );
            });
        panel
            .spawn((
                Node {
                    height: Val::Px(size::BUTTON_LG_HEIGHT.desktop),
                    justify_content: JustifyContent::Center,
                    flex_shrink: 0.0,
                    ..default()
                },
                Name::new("HelpFooter"),
            ))
            .with_children(|footer| {
                spawn_dismiss(footer, form);
                footer
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(DESKTOP_HINT.min.x),
                            top: Val::Px(DESKTOP_HINT.min.y),
                            width: Val::Px(DESKTOP_HINT.width()),
                            height: Val::Px(DESKTOP_HINT.height()),
                            justify_content: JustifyContent::FlexEnd,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        Name::new("HelpReopenHint"),
                    ))
                    .with_child((
                        reopen_hint().into_text(),
                        theme::role_text(TextRole::Caption),
                        TextColor(color::TEXT_MUTED),
                        TextLayout::justify(Justify::Right),
                    ));
            });
    });
}

fn spawn_desktop_card(grid: &mut ChildSpawnerCommands, card: &DesktopCard) {
    let form = Form::Desktop;
    let padding = surfaces::INFO_CARD_PADDING.at(form);
    let parts: InfoCard = surfaces::info_card(
        grid,
        Node::default(),
        card.icon,
        Localized::new(card.title),
        card_body(card.body),
        form,
    );
    let mut commands = grid.commands();
    commands
        .entity(parts.card)
        .insert(Name::new(format!("HelpCard-{}", card.id))); // i18n-allow: identity
    let abilities: Vec<Legend> = if card.id == "abilities" {
        SKILL_SLOT_KEY_LABELS
            .iter()
            .copied()
            .map(Legend::Key)
            .chain([Legend::Gap, Legend::Key(upgrade_key_display())])
            .collect()
    } else {
        Vec::new()
    };
    let legend: &[Legend] = if abilities.is_empty() {
        card.legend
    } else {
        &abilities
    };
    if legend.is_empty() {
        return;
    }
    commands.entity(parts.card).with_children(|card| {
        card.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(padding),
                bottom: Val::Px(padding),
                column_gap: Val::Px(LEGEND_GAP),
                align_items: AlignItems::Center,
                ..default()
            },
            Name::new("HelpCardInputs"),
        ))
        .with_children(|row| {
            for entry in legend {
                match *entry {
                    Legend::Mouse(key) => {
                        surfaces::badge(row, Localized::new(key), BadgeKind::Muted, false);
                    }
                    Legend::Key(key) => {
                        surfaces::keycap(row, key);
                    }
                    Legend::Gap => {
                        row.spawn(Node {
                            width: Val::Px(space::S8),
                            ..default()
                        });
                    }
                }
            }
        });
    });
}

/// One column of the field/camera strip: an inline gold `type.eyebrow` label
/// (at `type.caption` size) followed by the `type.caption` secondary text.
fn strip_entry(
    strip: &mut ChildSpawnerCommands,
    width: f32,
    title: &'static str,
    body: &'static str,
    name: &'static str,
) {
    let caption = TextRole::Caption.style();
    strip
        .spawn((
            Localized::new(title).into_text(),
            theme::styled_text(TextStyle::new(TextRole::Eyebrow).sized(caption.size)),
            TextColor(color::TEXT_GOLD),
            Node {
                width: Val::Px(width),
                flex_shrink: 0.0,
                ..default()
            },
            Name::new(name),
        ))
        .with_children(|text| {
            // The label and the text are one paragraph; a space separates them.
            text.spawn((
                TextSpan::new(" "),
                theme::text(caption.size.desktop),
                TextColor(color::TEXT_SECONDARY),
            ));
            let (span, localized) = {
                let localized = Localized::new(body);
                (TextSpan::new(localized.text()), localized)
            };
            text.spawn((
                span,
                localized,
                theme::text(caption.size.desktop),
                bevy::text::LineHeight::RelativeToFont(caption.line_height),
                TextColor(color::TEXT_SECONDARY),
            ));
        });
}

fn spawn_phone_guide(root: &mut ChildSpawnerCommands) {
    let form = Form::Phone;
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(space::S12),
            // `space.12` from the panel's outer edge, hairline included.
            padding: UiRect::all(Val::Px(space::S12 - border::HAIRLINE)),
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::LG)),
            ..default()
        },
        BackgroundColor(color::SURFACE_1),
        BorderColor::all(color::BORDER_SUBTLE),
        HelpOverlayPanel,
        HelpOpening::default(),
        UiTransform::default(),
        Name::new("HelpPanel"),
    ))
    .with_children(|panel| {
        panel
            .spawn((
                Node {
                    height: Val::Px(size::BUTTON_HEIGHT.phone),
                    column_gap: Val::Px(space::S12),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    flex_shrink: 0.0,
                    ..default()
                },
                Name::new("HelpTitleRow"),
            ))
            .with_children(|row| {
                row.spawn((
                    Localized::new("help.phone.title").into_text(),
                    theme::role_text(TextRole::Heading),
                    TextColor(color::TEXT_GOLD),
                    TextLayout::no_wrap(),
                    Node {
                        flex_grow: 1.0,
                        min_width: Val::Px(0.0),
                        ..default()
                    },
                    Name::new("HelpTitle"),
                ));
                spawn_dismiss(row, form);
            });
        panel
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_height: Val::Px(0.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ScrollArea::phone_panel(),
                HelpCardList,
                Name::new("HelpBody"),
            ))
            .with_children(|list| {
                list.spawn((
                    Node {
                        display: Display::Grid,
                        grid_template_columns: RepeatedGridTrack::flex(2, 1.0),
                        grid_auto_rows: vec![GridTrack::px(PHONE_CARD_H)],
                        column_gap: Val::Px(space::S8),
                        row_gap: Val::Px(space::S8),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    Name::new("HelpCards"),
                ))
                .with_children(|grid| {
                    for (id, icon, title, body) in PHONE_CARDS {
                        let parts = surfaces::info_card(
                            grid,
                            Node {
                                min_width: Val::Px(0.0),
                                ..default()
                            },
                            icon,
                            Localized::new(title),
                            Localized::new(body),
                            form,
                        );
                        grid.commands()
                            .entity(parts.card)
                            .insert(Name::new(format!("HelpCard-{id}"))); // i18n-allow: identity
                    }
                });
                list.spawn((
                    Localized::new("help.phone.footer").into_text(),
                    theme::role_text(TextRole::Caption),
                    TextColor(color::TEXT_MUTED),
                    Node {
                        height: Val::Px(PHONE_FOOTER_H),
                        margin: UiRect::top(Val::Px(space::S8)),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    Name::new("HelpFooter"),
                ));
            });
        panel.spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(space::S4),
                width: Val::Px(PHONE_THUMB_W),
                border_radius: BorderRadius::all(Val::Px(PHONE_THUMB_W / 2.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(color::GOLD_600),
            Pickable::IGNORE,
            HelpScrollThumb,
            Name::new("HelpScrollThumb"),
        ));
    });
}

fn toggle_help_overlay(
    mut back: crate::ui::BackInput,
    game: Res<GameStateSnapshot>,
    mut visible: ResMut<HelpOverlayVisible>,
    career: Option<Res<crate::career::CareerClient>>,
    social: Option<Res<crate::social::SocialClient>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    splash: Option<Res<crate::frontend::boot::BootSplash>>,
) {
    if social
        .as_ref()
        .is_some_and(|social| social.blocks_gameplay())
        || career.as_ref().is_some_and(|career| career.modal_open())
        || splash.is_some_and(|splash| splash.blocks_input())
    {
        return;
    }
    if visible.0
        && (matches!(game.state, GameState::Running)
            || screen.as_ref().is_some_and(|screen| screen.get().is_menu()))
        && back.just_pressed()
    {
        visible.0 = false;
        // Consume only this dismissal so the same key does not open Pause.
        back.consume();
    } else if back.keys().just_pressed(HELP_TOGGLE_KEY) {
        visible.0 = !visible.0;
    }
}

fn dismiss_help_button(
    mut activated: MessageReader<Activated<HelpAction>>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    for Activated { action, .. } in activated.read() {
        match action {
            HelpAction::Dismiss => visible.0 = false,
        }
    }
}

fn sync_help_overlay_visibility(
    visible: Res<HelpOverlayVisible>,
    snapshot: Res<GameStateSnapshot>,
    session: Option<Res<ClientSession>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut shown: ResMut<HelpOverlayShown>,
    mut root: Query<(&mut Visibility, &mut Node), With<HelpOverlayRoot>>,
    mut opening: Query<&mut HelpOpening>,
) {
    let in_running_match = matches!(snapshot.state, GameState::Running)
        && session
            .as_ref()
            .is_none_or(|session| session.join_confirmed());
    if !visible.is_changed()
        && !snapshot.is_changed()
        && session.as_ref().is_none_or(|session| !session.is_changed())
        && screen.as_ref().is_none_or(|screen| !screen.is_changed())
    {
        return;
    }
    let Ok((mut v, mut node)) = root.single_mut() else {
        return;
    };
    // Explicit Help requests are usable before admission too. Automatic
    // first-match onboarding remains gated separately by local admission.
    let shell = screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    let show_panel = visible.0 && (in_running_match || shell);
    if show_panel && !shown.0 {
        for mut opening in &mut opening {
            opening.elapsed = 0.0;
        }
    }
    shown.set_if_neq(HelpOverlayShown(show_panel));
    node.display = if show_panel {
        Display::Flex
    } else {
        Display::None
    };
    *v = if show_panel {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

/// Desktop: at most the authored size, scaled down to fit a window smaller
/// than the 1280×720 reference (in logical UI px, after `UiScale`). A menu
/// already follows R2.3 through `UiScale`; the desktop match stays at 1.0,
/// where this keeps the whole guide on a small window.
fn desktop_fit(viewport: Vec2) -> f32 {
    let (width, height) = theme::metric::REFERENCE;
    (viewport.x / width)
        .min(viewport.y / height)
        .clamp(0.1, 1.0)
}

/// The phone panel rectangle for a safe area (`hud-help.md`): safe
/// left/right + `space.screen_margin.phone`, 12 from the top and from the
/// safe bottom.
fn phone_panel_rect(viewport: Vec2, safe: &crate::mobile_controls::MobileSafeInsets) -> Rect {
    let margin = space::SCREEN_MARGIN.phone;
    Rect::new(
        safe.left + margin,
        safe.top + PHONE_PANEL_INSET_Y,
        viewport.x - safe.right - margin,
        viewport.y - safe.bottom - PHONE_PANEL_INSET_Y,
    )
}

/// Sizes and places the guide for the window, and plays the open animation
/// (scale from `motion.panel_open.scale_from`, scrim fade) over
/// `motion.duration.panel_open`.
#[allow(clippy::type_complexity)]
fn layout_help_overlay(
    time: Option<Res<Time>>,
    shown: Res<HelpOverlayShown>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut panels: Query<(&mut Node, &mut UiTransform, &mut HelpOpening), With<HelpOverlayPanel>>,
    mut roots: Query<&mut BackgroundColor, With<HelpOverlayRoot>>,
) {
    if !shown.0 {
        return;
    }
    let phone = mobile
        .as_ref()
        .filter(|mobile| mobile.enabled && mobile.landscape);
    let viewport = windows.single().ok().map(|window| {
        Vec2::new(window.width(), window.height())
            / theme::metric::ui_scale(ui_scale.as_ref().map_or(1.0, |scale| scale.0))
    });
    let duration = motion::DURATION_PANEL_OPEN.as_secs_f32();
    for (mut node, mut transform, mut opening) in &mut panels {
        let delta = time.as_ref().map_or(0.0, |time| time.delta_secs());
        opening.elapsed = (opening.elapsed + delta).min(duration);
        let t = motion::EASING_ENTER.ease((opening.elapsed / duration).clamp(0.0, 1.0));
        let from = motion::PANEL_OPEN_SCALE_FROM;
        let open = from + (1.0 - from) * t;
        let fit = match (phone, viewport) {
            (Some(mobile), _) => {
                let rect = phone_panel_rect(mobile.viewport, &mobile.safe);
                let place = [
                    (Val::Px(rect.min.x), node.left),
                    (Val::Px(rect.min.y), node.top),
                    (Val::Px(rect.width().max(0.0)), node.width),
                    (Val::Px(rect.height().max(0.0)), node.height),
                ];
                if place.iter().any(|(next, current)| next != current) {
                    node.left = place[0].0;
                    node.top = place[1].0;
                    node.width = place[2].0;
                    node.height = place[3].0;
                }
                1.0
            }
            (None, Some(viewport)) => desktop_fit(viewport),
            (None, None) => 1.0,
        };
        let scale = Vec2::splat(fit * open);
        if transform.scale != scale {
            transform.scale = scale;
        }
        for mut scrim in &mut roots {
            let next = theme::perceptual(color::SCRIM.with_alpha(color::SCRIM.alpha() * t));
            if scrim.0 != next {
                scrim.0 = next;
            }
        }
    }
}

/// The phone list's scroll thumb: shown while the cards overflow, its length
/// the visible share of the list, placed by the scroll offset beside it (in
/// the panel's right padding).
#[allow(clippy::type_complexity)]
fn sync_help_scroll_thumb(
    shown: Res<HelpOverlayShown>,
    lists: Query<(&ComputedNode, &UiGlobalTransform, &ScrollPosition), With<HelpCardList>>,
    panels: Query<(&ComputedNode, &UiGlobalTransform), With<HelpOverlayPanel>>,
    mut thumbs: Query<&mut Node, With<HelpScrollThumb>>,
) {
    if !shown.0 {
        return;
    }
    let (Ok((list, list_at, scroll)), Ok((panel, panel_at))) = (lists.single(), panels.single())
    else {
        return;
    };
    let Ok(mut thumb) = thumbs.single_mut() else {
        return;
    };
    let scale = list.inverse_scale_factor();
    let visible = list.size().y * scale;
    let content = list.content_size().y * scale;
    let (display, top, height) = if content > visible + 0.5 && visible > 0.0 {
        let height = (visible * visible / content).max(size::TOUCH_MIN / 2.0);
        let travel = (content - visible).max(1.0);
        // The list's top inside the panel's padding box.
        let list_top = (list_at.translation.y - list.size().y / 2.0)
            - (panel_at.translation.y - panel.size().y / 2.0);
        let list_top = list_top * scale - border::HAIRLINE;
        let top = list_top + (scroll.y / travel).clamp(0.0, 1.0) * (visible - height);
        (Display::Flex, top, height)
    } else {
        (Display::None, 0.0, 0.0)
    };
    if thumb.display != display {
        thumb.display = display;
    }
    if thumb.top != Val::Px(top) {
        thumb.top = Val::Px(top);
    }
    if thumb.height != Val::Px(height) {
        thumb.height = Val::Px(height);
    }
}

/// Every text the desktop guide shows, in the active language (tests).
#[cfg(test)]
fn desktop_guide_copy() -> String {
    let mut copy: Vec<String> = DESKTOP_CARDS
        .iter()
        .flat_map(|card| {
            [
                crate::i18n::tr(card.title).to_owned(),
                card_body(card.body).text(),
            ]
        })
        .collect();
    for key in [
        "help.card.field.title",
        "help.card.field.body",
        "help.card.camera.title",
        "help.card.camera.body",
    ] {
        copy.push(crate::i18n::tr(key).to_owned());
    }
    copy.push(reopen_hint().text());
    copy.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_help_opens_and_dismisses_before_admission_without_auto_onboarding() {
        for state in [GameState::Lobby, GameState::Running] {
            let mut app = App::new();
            app.init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ClientSession>()
                .insert_resource(State::new(crate::frontend::AppScreen::Home))
                .insert_resource(GameStateSnapshot { state, ..default() })
                .add_plugins(HelpOverlayPlugin);
            app.update();
            assert!(!app.world().resource::<HelpOverlayVisible>().0);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(HELP_TOGGLE_KEY);
            app.update();
            let mut roots = app
                .world_mut()
                .query_filtered::<(&Node, &Visibility), With<HelpOverlayRoot>>();
            let (node, visibility) = roots.single(app.world()).unwrap();
            assert_eq!(node.display, Display::Flex);
            assert_eq!(*visibility, Visibility::Visible);
            assert!(app.world().resource::<HelpAutoShowState>().pending);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
            app.update();
            assert!(!app.world().resource::<HelpOverlayVisible>().0);
            let (node, visibility) = roots.single(app.world()).unwrap();
            assert_eq!(node.display, Display::None);
            assert_eq!(*visibility, Visibility::Hidden);
            assert!(
                !app.world()
                    .resource::<ButtonInput<KeyCode>>()
                    .just_pressed(KeyCode::Escape)
            );
        }
    }

    /// Decision R2.1 / `hud-help.md`: the guide's copy is split into card
    /// keys; the same first-match actions are covered (was: one `help.body`).
    #[test]
    fn help_overlay_copy_covers_core_first_match_actions() {
        let body = desktop_guide_copy();
        assert!(body.contains("Move"));
        assert!(body.contains("Target"));
        assert!(body.contains("Right-click a hostile"));
        assert!(body.contains("on-screen buttons"));
        assert!(body.contains("Abilities"));
        assert!(body.contains("Objective"));
        assert!(body.contains("Y toggles hero follow"));
        assert!(body.contains(&skill_keys_display()));
    }

    /// The platform guide follows a language change while the overlay
    /// exists, with the key bindings still filled in. Isolated: it switches
    /// the process-wide language. (R2.1: cards and the short dismiss label.)
    #[test]
    fn help_copy_follows_the_language_on_desktop_and_phone() {
        if crate::i18n::testing::isolated(
            "help_overlay::tests::help_copy_follows_the_language_on_desktop_and_phone",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let zh = LocaleId::parse("zh-Hans").unwrap();
        for (profile, first_card, dismiss) in [
            (
                crate::platform::UiProfile::Desktop,
                ("Move", "右键点击地面即可前往。"),
                "进入竞技场",
            ),
            (
                crate::platform::UiProfile::Mobile,
                ("Move", "拖动左侧摇杆。"),
                "知道了，开始游戏",
            ),
        ] {
            let mut app = App::new();
            app.add_plugins(I18nPlugin::default())
                .insert_resource(crate::ui::UiPlatform(profile))
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<GameStateSnapshot>()
                .add_plugins(HelpOverlayPlugin);
            app.update();
            let named = |app: &mut App, wanted: &str| {
                app.world_mut()
                    .query::<(&Name, &Children)>()
                    .iter(app.world())
                    .find(|(name, _)| name.as_str() == wanted)
                    .map(|(_, children)| children.to_vec())
                    .unwrap()
            };
            let card_texts = |app: &mut App| -> Vec<String> {
                let card = named(app, "HelpCard-move");
                let mut texts = Vec::new();
                let mut stack = card;
                while let Some(entity) = stack.pop() {
                    if let Some(text) = app.world().get::<Text>(entity) {
                        texts.push(text.0.clone());
                    }
                    if let Some(children) = app.world().get::<Children>(entity) {
                        stack.extend(children.iter());
                    }
                }
                texts
            };
            let english = card_texts(&mut app);
            assert!(
                english.iter().any(|text| text == first_card.0),
                "{english:?}"
            );
            app.world_mut().resource_mut::<Locale>().set(zh);
            app.update();
            let chinese = card_texts(&mut app);
            assert!(
                chinese.iter().any(|text| text.starts_with(first_card.1)),
                "{chinese:?}"
            );
            if profile == crate::platform::UiProfile::Desktop {
                let copy = desktop_guide_copy();
                assert!(copy.contains(&skill_keys_display()));
                assert!(copy.contains("按 F1"));
            }
            let label = app
                .world_mut()
                .query::<(&Name, &Text)>()
                .iter(app.world())
                .find(|(name, _)| name.as_str() == "HelpDismissLabel")
                .map(|(_, text)| text.0.clone())
                .unwrap();
            assert_eq!(label, dismiss);
        }
    }

    #[test]
    fn help_overlay_copy_includes_toggle_hint() {
        let body = desktop_guide_copy();
        assert!(body.contains("press F1"));
    }
    #[test]
    fn first_match_help_button_dismisses_and_does_not_reopen_on_rematch() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_plugins(HelpOverlayPlugin);
        app.update();
        assert!(app.world().resource::<HelpOverlayVisible>().0);
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<HelpDismissButton>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
        let mut root = app
            .world_mut()
            .query_filtered::<(&Node, &Visibility), With<HelpOverlayRoot>>();
        let (node, visibility) = root.single(app.world()).unwrap();
        assert_eq!(node.display, Display::None);
        assert_eq!(*visibility, Visibility::Hidden);
        app.world_mut().entity_mut(button).insert(Interaction::None);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.update();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
    }

    #[test]
    fn running_server_does_not_show_first_match_help_before_local_admission() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ClientSession>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_plugins(HelpOverlayPlugin);
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(crate::net::ClientConnectionState::Connected);
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
        assert!(app.world().resource::<HelpAutoShowState>().pending);
    }

    /// A headless app with Bevy UI layout at `size` (1 px per UI px) and the
    /// guide open over a running match; the open animation is finished.
    fn layout_app(size: Vec2, profile: crate::platform::UiProfile) -> App {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            bevy::image::ImagePlugin::default(),
            bevy::text::TextPlugin,
            bevy::transform::TransformPlugin,
            bevy::input::InputPlugin,
            bevy::ui::UiPlugin,
            bevy::camera::visibility::VisibilityPlugin,
            bevy::picking::PickingPlugin,
            bevy::picking::InteractionPlugin,
        ));
        let phone = profile == crate::platform::UiProfile::Mobile;
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.enabled = phone;
        mobile.focused = true;
        mobile.landscape = true;
        mobile.viewport = size;
        mobile.safe = crate::mobile_controls::MobileSafeInsets {
            left: 47.0,
            right: 47.0,
            top: 0.0,
            bottom: 21.0,
        };
        app.init_resource::<Assets<bevy::mesh::Mesh>>()
            .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
            .init_resource::<Assets<TextureAtlasLayout>>()
            .insert_resource(crate::ui::UiPlatform(profile))
            .insert_resource(mobile)
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_plugins(HelpOverlayPlugin);
        let mut window = Window::default();
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set(size.x, size.y);
        app.world_mut().spawn((window, PrimaryWindow));
        app.world_mut().spawn((
            Camera2d,
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: size.as_uvec2(),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        app.finish();
        app.cleanup();
        app.update();
        for mut opening in app
            .world_mut()
            .query::<&mut HelpOpening>()
            .iter_mut(app.world_mut())
        {
            opening.elapsed = f32::MAX;
        }
        for _ in 0..4 {
            app.update();
        }
        app
    }

    fn named(app: &mut App, name: &str) -> Entity {
        app.world_mut()
            .query::<(Entity, crate::ui::test_id::NodeKey)>()
            .iter(app.world())
            .find(|(_, key)| key.as_str() == name)
            .map(|(entity, _)| entity)
            .unwrap_or_else(|| panic!("no node {name}"))
    }

    /// `(x, y, w, h)` of a node in window px, rounded.
    fn rect(app: &mut App, name: &str) -> [f32; 4] {
        let entity = named(app, name);
        let node = app.world().get::<ComputedNode>(entity).unwrap();
        let at = app.world().get::<UiGlobalTransform>(entity).unwrap();
        let r = crate::ui::focus::node_rect(node, at);
        [r.min.x, r.min.y, r.width(), r.height()].map(f32::round)
    }

    /// AC1: the desktop guide is the redline's six cards, strip, dismiss and
    /// hint at 1280×720 (`hud-help.md` desktop table), not a text block.
    #[test]
    fn desktop_guide_matches_the_redline_at_720p() {
        let mut app = layout_app(
            Vec2::new(1280.0, 720.0),
            crate::platform::UiProfile::Desktop,
        );
        assert!(app.world().resource::<HelpOverlayShown>().0);
        assert_eq!(rect(&mut app, "HelpOverlayRoot"), [0.0, 0.0, 1280.0, 720.0]);
        assert_eq!(rect(&mut app, "HelpPanel"), [96.0, 24.0, 1088.0, 672.0]);
        assert_eq!(rect(&mut app, "HelpEyebrow"), [120.0, 48.0, 1040.0, 16.0]);
        assert_eq!(rect(&mut app, "HelpTitle"), [120.0, 68.0, 1040.0, 40.0]);
        for (id, x, y) in [
            ("move", 120.0, 124.0),
            ("attack", 472.0, 124.0),
            ("target", 824.0, 124.0),
            ("abilities", 120.0, 340.0),
            ("shop", 472.0, 340.0),
            ("objective", 824.0, 340.0),
        ] {
            assert_eq!(
                rect(&mut app, &format!("HelpCard-{id}")),
                [x, y, 336.0, 192.0],
                "{id}"
            );
        }
        assert_eq!(rect(&mut app, "HelpField"), [120.0, 548.0, 336.0, 52.0]);
        assert_eq!(rect(&mut app, "HelpCamera"), [472.0, 548.0, 688.0, 52.0]);
        assert_eq!(
            rect(&mut app, "HelpDismissButton"),
            [460.0, 612.0, 360.0, 60.0]
        );
        assert_eq!(
            rect(&mut app, "HelpReopenHint"),
            [836.0, 625.0, 324.0, 34.0]
        );
        // No legacy text wall: no text node carries the old one-block body.
        let texts: Vec<String> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|text| text.0.clone())
            .collect();
        assert!(texts.iter().all(|text| !text.contains('\n')), "{texts:?}");
    }

    /// AC1: every desktop card carries the keyboard/mouse legend of its line:
    /// mouse chips from `help.input.*`, keycaps from the bindings.
    #[test]
    fn desktop_cards_carry_their_input_legends() {
        let mut app = layout_app(
            Vec2::new(1280.0, 720.0),
            crate::platform::UiProfile::Desktop,
        );
        let legend = |app: &mut App, id: &str| -> Vec<String> {
            let card = named(app, &format!("HelpCard-{id}"));
            let mut out = Vec::new();
            let mut stack = vec![card];
            while let Some(entity) = stack.pop() {
                if app
                    .world()
                    .get::<Name>(entity)
                    .is_some_and(|name| name.as_str() == "HelpCardInputs")
                {
                    let mut inner: Vec<Entity> =
                        app.world().get::<Children>(entity).unwrap().to_vec();
                    inner.reverse();
                    let mut walk = inner;
                    while let Some(entity) = walk.pop() {
                        if let Some(text) = app.world().get::<Text>(entity) {
                            out.push(text.0.clone());
                        }
                        if let Some(children) = app.world().get::<Children>(entity) {
                            let mut children: Vec<Entity> = children.to_vec();
                            children.reverse();
                            walk.extend(children);
                        }
                    }
                }
                if let Some(children) = app.world().get::<Children>(entity) {
                    stack.extend(children.iter());
                }
            }
            out
        };
        assert_eq!(legend(&mut app, "move"), ["Right-click"]);
        assert_eq!(legend(&mut app, "attack"), ["Right-click"]);
        assert_eq!(
            legend(&mut app, "target"),
            ["Left-click", "Tab", "Backspace", "S"]
        );
        let mut keys: Vec<String> = SKILL_SLOT_KEY_LABELS
            .iter()
            .map(|k| k.to_string())
            .collect();
        keys.push(upgrade_key_display().to_owned());
        assert_eq!(legend(&mut app, "abilities"), keys);
        assert_eq!(legend(&mut app, "shop"), ["P"]);
        assert!(legend(&mut app, "objective").is_empty());
    }

    /// AC1: the phone guide is a safe-area modal (`hud-help.md` phone table):
    /// title row with the dismiss button, ten cards in two columns in a
    /// scroll list, the closing line as its last row.
    #[test]
    fn phone_guide_fits_the_safe_area_and_scrolls_its_cards() {
        let mut app = layout_app(Vec2::new(844.0, 390.0), crate::platform::UiProfile::Mobile);
        assert_eq!(rect(&mut app, "HelpPanel"), [63.0, 12.0, 718.0, 345.0]);
        assert_eq!(
            rect(&mut app, "HelpDismissButton"),
            [529.0, 24.0, 240.0, 44.0]
        );
        assert_eq!(rect(&mut app, "HelpBody"), [75.0, 80.0, 694.0, 265.0]);
        let names = [
            "move", "attack", "target", "farm", "utility", "skills", "grow", "win", "recover",
            "look",
        ];
        for (index, id) in names.iter().enumerate() {
            let [x, _, w, h] = rect(&mut app, &format!("HelpCard-{id}"));
            assert_eq!(
                (x, w, h),
                (if index % 2 == 0 { 75.0 } else { 426.0 }, 343.0, 96.0),
                "{id}"
            );
        }
        let list = named(&mut app, "HelpBody");
        let node = app.world().get::<ComputedNode>(list).unwrap();
        assert_eq!(
            node.content_size().y.round(),
            554.0,
            "5 × 96 + 4 × 8 + 8 + 34"
        );
        assert!(app.world().get::<ScrollArea>(list).is_some());
        let thumb = named(&mut app, "HelpScrollThumb");
        assert_eq!(
            app.world().get::<Node>(thumb).unwrap().display,
            Display::Flex
        );
        // The touch dismiss target keeps the touch minimum.
        assert!(rect(&mut app, "HelpDismissButton")[3] >= size::TOUCH_MIN);
    }

    /// A window below the 1280×720 reference keeps the whole desktop guide
    /// on screen (scaled), a larger one keeps the authored size.
    #[test]
    fn desktop_guide_fits_small_windows() {
        assert_eq!(desktop_fit(Vec2::new(1280.0, 720.0)), 1.0);
        assert_eq!(desktop_fit(Vec2::new(1920.0, 1080.0)), 1.0);
        assert_eq!(desktop_fit(Vec2::new(1024.0, 640.0)), 0.8);
        let mut app = layout_app(
            Vec2::new(1024.0, 640.0),
            crate::platform::UiProfile::Desktop,
        );
        let [x, y, w, h] = rect(&mut app, "HelpPanel");
        assert!(
            x >= 0.0 && y >= 0.0 && x + w <= 1024.0 && y + h <= 640.0,
            "{x} {y} {w} {h}"
        );
    }

    /// AC2/AC4: the shown guide is the top modal, so a controller's focus
    /// starts on its dismiss button (not a HUD button above it), gamepad East
    /// closes it, and the HUD button under it is blocked meanwhile.
    #[test]
    fn controller_focus_starts_on_dismiss_and_east_closes() {
        use crate::ui::ModalAppExt;
        use crate::ui::focus::{FocusNav, UiFocus, navigate_focus};
        let mut app = layout_app(
            Vec2::new(1280.0, 720.0),
            crate::platform::UiProfile::Desktop,
        );
        app.init_resource::<UiFocus>()
            .init_resource::<crate::ui::BackPress>()
            .init_resource::<crate::ui::GestureEpoch>()
            .add_message::<FocusNav>()
            .add_message::<crate::ui::SyntheticPress>()
            .add_message::<crate::ui::focus::FocusAdjust>()
            .register_modal::<HelpOverlayShown>(ModalId::Help, |shown| shown.0)
            .add_systems(
                Update,
                navigate_focus
                    .after(crate::ui::modal::ModalSet::Early)
                    .before(HelpOverlaySet::Input),
            );
        let hud = app
            .world_mut()
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(8.0),
                    top: Val::Px(8.0),
                    width: Val::Px(40.0),
                    height: Val::Px(40.0),
                    ..default()
                },
                crate::ui::Pressable::default(),
                Interaction::default(),
            ))
            .id();
        app.world_mut().resource_mut::<UiFocus>().set_enabled(true);
        for _ in 0..3 {
            app.world_mut().resource_mut::<UiFocus>().set_enabled(true);
            app.update();
        }
        let dismiss = named(&mut app, "HelpDismissButton");
        assert_eq!(app.world().resource::<UiFocus>().focused(), Some(dismiss));
        assert_eq!(
            app.world().resource::<crate::ui::ModalStack>().top(),
            Some(ModalId::Help)
        );
        let mut gate =
            bevy::ecs::system::SystemState::<crate::ui::modal::ModalGate>::new(app.world_mut());
        assert!(
            !gate
                .get(app.world())
                .expect("modal test resources exist")
                .allows(hud),
            "the HUD waits under the guide"
        );
        // Gamepad East is a back press.
        app.world_mut()
            .resource_mut::<crate::ui::BackPress>()
            .press();
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
        app.update();
        assert!(!app.world().resource::<crate::ui::ModalStack>().is_open());
    }
}
