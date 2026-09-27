//! The Verdant Crown kit gallery: every kit component in every state, per
//! layout family (desktop/phone) and language, without a match.
//!
//! - `OMOBA_UI_GALLERY=1` opens it over the front-end shell. F2 switches the
//!   profile, F3 the language, PageUp/PageDown (or the page tabs) the page.
//!   The first ("idle") column of each row is live: hover, press and gamepad
//!   focus work on it; the other columns are pinned with `PreviewState`.
//! - `OMOBA_UI_GALLERY_OUTPUT=<dir>` captures every page × profile ×
//!   language at 1280×720, plus the buttons page at 1920×1080 and 1024×640,
//!   into `<dir>` (`gallery-<page>-<profile>-<lang>-<w>x<h>.png`,
//!   `gallery-summary.json`) and exits.
use std::path::PathBuf;

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};

use crate::i18n::{Locale, LocaleId, Localized};
use crate::ui::{
    Pressable, ScrollArea, UiActionAppExt,
    kit_assets::{Background, Icon, KitImage},
    theme::{self, ButtonKind, ButtonState, Form, UiForm},
    tokens::{TextRole, color, size, space},
    widgets::{
        self, ButtonSize, ButtonStyle, PreviewState,
        controls::{self, TabPlacement},
        game::{
            self, AbilityView, AvatarSource, BarKind, BarValue, RingSize, ScoreRow, ShopCard,
            TimerRing,
        },
        surfaces::{self, BadgeKind, RowLeading, ToastKind},
    },
};

const PAGES: [(&str, &str); 10] = [
    ("buttons", "kit.gallery.page.buttons"),
    ("controls", "kit.gallery.page.controls"),
    ("inputs", "kit.gallery.page.inputs"),
    ("panels", "kit.gallery.page.surfaces"),
    ("feedback", "kit.gallery.page.feedback"),
    ("abilities", "kit.gallery.page.abilities"),
    ("heroes", "kit.gallery.page.heroes"),
    ("hud", "kit.gallery.page.hud"),
    ("type", "kit.gallery.page.type"),
    ("backgrounds", "kit.gallery.page.backgrounds"),
];

/// The six interaction states, left to right (`components/*.png`).
const STATES: [(Option<ButtonState>, bool, &str); 6] = [
    (None, false, "kit.gallery.state.idle"),
    (Some(ButtonState::Hover), false, "kit.gallery.state.hover"),
    (
        Some(ButtonState::Pressed),
        false,
        "kit.gallery.state.pressed",
    ),
    (Some(ButtonState::Idle), true, "kit.gallery.state.focused"),
    (
        Some(ButtonState::Disabled),
        false,
        "kit.gallery.state.disabled",
    ),
    (Some(ButtonState::Idle), false, "kit.gallery.state.selected"),
];

/// Row label column and state column widths.
const LABEL_W: f32 = 120.0;
const CELL_W: f32 = 160.0;
/// A slider row: label, 280 track, value.
const SLIDER_CELL_W: f32 = 480.0;
/// Z above the shell, under the focus ring (5000) and preview rings.
const GALLERY_Z: i32 = 4000;

/// Sample player names and roster art (data, not UI copy).
const NAMES: [&str; 3] = ["Lunaria", "Dmitry#0097", "QA Player 6"]; // i18n-allow
const ART: [&str; 4] = [
    "avatars/agnes.jpg",
    "avatars/crowley.jpg",
    "avatars/lady-koi.jpg",
    "avatars/cool-tiger.jpg",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GalleryAction {
    Page(usize),
    Profile(bool),
    Language(usize),
    Noop,
}

#[derive(Resource)]
struct Gallery {
    page: usize,
    form: Form,
    locale: LocaleId,
    dirty: bool,
    capture: Option<Capture>,
}

struct Capture {
    directory: PathBuf,
    shots: Vec<Shot>,
    index: usize,
    settled: u32,
    in_flight: bool,
    done: Vec<serde_json::Value>,
}

#[derive(Clone)]
struct Shot {
    page: usize,
    form: Form,
    locale: LocaleId,
    pixels: UVec2,
    file: String,
}

#[derive(Component)]
struct GalleryRoot;

#[derive(Component)]
struct ShotMarker;

pub(crate) struct UiGalleryPlugin;

impl Plugin for UiGalleryPlugin {
    fn build(&self, app: &mut App) {
        let output = std::env::var_os("OMOBA_UI_GALLERY_OUTPUT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let open = std::env::var("OMOBA_UI_GALLERY").is_ok_and(|value| value == "1");
        if output.is_none() && !open {
            return;
        }
        let capture = output.map(|directory| Capture {
            directory,
            shots: shots(),
            index: 0,
            settled: 0,
            in_flight: false,
            done: Vec::new(),
        });
        app.insert_resource(Gallery {
            page: 0,
            form: Form::Desktop,
            locale: LocaleId::ENGLISH,
            dirty: true,
            capture,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .insert_resource(crate::frontend::ScreenDriverPaused(true))
        .add_ui_action::<GalleryAction>()
        .add_systems(Update, (keys, actions).after(crate::ui::UiSet::Dispatch))
        .add_systems(Update, drive_capture.before(rebuild))
        .add_systems(Update, rebuild.after(actions))
        .add_systems(
            PostUpdate,
            observe_capture.after(bevy::ui::UiSystems::Layout),
        );
    }
}

fn shots() -> Vec<Shot> {
    let zh = LocaleId::parse("zh-Hans").unwrap_or(LocaleId::ENGLISH);
    let mut shots = Vec::new();
    let reference = UVec2::new(1280, 720);
    for (form, profile) in [(Form::Desktop, "desktop"), (Form::Phone, "phone")] {
        for (locale, lang) in [(LocaleId::ENGLISH, "en"), (zh, "zh")] {
            for (page, (name, _)) in PAGES.iter().enumerate() {
                shots.push(Shot {
                    page,
                    form,
                    locale,
                    pixels: reference,
                    file: format!("gallery-{name}-{profile}-{lang}-1280x720.png"),
                });
            }
        }
    }
    for pixels in [UVec2::new(1920, 1080), UVec2::new(1024, 640)] {
        for (page, name) in [(0, "buttons"), (8, "type")] {
            shots.push(Shot {
                page,
                form: Form::Desktop,
                locale: LocaleId::ENGLISH,
                pixels,
                file: format!("gallery-{name}-desktop-en-{}x{}.png", pixels.x, pixels.y),
            });
        }
    }
    shots
}

fn keys(keys: Res<ButtonInput<KeyCode>>, mut gallery: ResMut<Gallery>) {
    if gallery.capture.is_some() {
        return;
    }
    if keys.just_pressed(KeyCode::F2) {
        gallery.form = if gallery.form == Form::Desktop {
            Form::Phone
        } else {
            Form::Desktop
        };
        gallery.dirty = true;
    }
    if keys.just_pressed(KeyCode::F3) {
        gallery.locale = gallery.locale.next();
        gallery.dirty = true;
    }
    if keys.just_pressed(KeyCode::PageDown) {
        gallery.page = (gallery.page + 1) % PAGES.len();
        gallery.dirty = true;
    }
    if keys.just_pressed(KeyCode::PageUp) {
        gallery.page = (gallery.page + PAGES.len() - 1) % PAGES.len();
        gallery.dirty = true;
    }
}

fn actions(
    mut activated: MessageReader<crate::ui::Activated<GalleryAction>>,
    mut gallery: ResMut<Gallery>,
) {
    for crate::ui::Activated { action, .. } in activated.read() {
        match *action {
            GalleryAction::Page(page) => gallery.page = page,
            GalleryAction::Profile(phone) => gallery.form = Form::of(phone),
            GalleryAction::Language(index) => {
                if let Some(locale) = crate::i18n::available_locales().nth(index) {
                    gallery.locale = locale;
                }
            }
            GalleryAction::Noop => continue,
        }
        gallery.dirty = true;
    }
}

fn drive_capture(
    mut gallery: ResMut<Gallery>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Some(capture) = gallery.capture.as_ref() else {
        return;
    };
    let Some(shot) = capture.shots.get(capture.index).cloned() else {
        return;
    };
    if let Ok(mut window) = windows.single_mut()
        && (window.resolution.physical_width() != shot.pixels.x
            || window.resolution.physical_height() != shot.pixels.y)
    {
        window.resolution.set_scale_factor_override(Some(1.0));
        window
            .resolution
            .set_physical_resolution(shot.pixels.x, shot.pixels.y);
    }
    if gallery.page != shot.page || gallery.form != shot.form || gallery.locale != shot.locale {
        gallery.page = shot.page;
        gallery.form = shot.form;
        gallery.locale = shot.locale;
        gallery.dirty = true;
    }
}

fn rebuild(
    mut commands: Commands,
    mut gallery: ResMut<Gallery>,
    roots: Query<Entity, With<GalleryRoot>>,
    mut form: ResMut<UiForm>,
    locale: Option<ResMut<Locale>>,
) {
    if !gallery.dirty {
        return;
    }
    gallery.dirty = false;
    if form.0 != gallery.form {
        form.0 = gallery.form;
    }
    if let Some(mut locale) = locale
        && locale.id() != gallery.locale
    {
        locale.set(gallery.locale);
    }
    if let Some(capture) = gallery.capture.as_mut() {
        capture.settled = 0;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let page = gallery.page;
    let form = gallery.form;
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(space::S24), Val::Px(space::S16)),
                row_gap: Val::Px(space::S12),
                ..default()
            },
            BackgroundColor(color::BG_BASE),
            GlobalZIndex(GALLERY_Z),
            GalleryRoot,
            Name::new("UiGalleryRoot"),
        ))
        .with_children(|root| {
            header(root, page, form);
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S12),
                    flex_grow: 1.0,
                    min_height: Val::Px(0.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ScrollArea::wheel(48.0),
                Name::new("UiGalleryPage"),
            ))
            .with_children(|body| spawn_page(body, page, form));
        });
}

fn spawn_page(body: &mut ChildSpawnerCommands, page: usize, form: Form) {
    match page {
        0 => buttons_page(body, form),
        1 => controls_page(body, form),
        2 => inputs_page(body, form),
        3 => panels_page(body, form),
        4 => feedback_page(body, form),
        5 => abilities_page(body, form),
        6 => heroes_page(body, form),
        7 => hud_page(body, form),
        8 => type_page(body),
        _ => backgrounds_page(body, form),
    }
}

fn header(root: &mut ChildSpawnerCommands, page: usize, form: Form) {
    root.spawn(Node {
        column_gap: Val::Px(space::S16),
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|bar| {
        bar.spawn((
            Localized::new("kit.gallery.title").into_text(),
            theme::role_text(TextRole::Title),
            TextColor(color::TEXT_GOLD),
        ));
        bar.spawn((
            Localized::new("kit.gallery.hint").into_text(),
            theme::role_text(TextRole::Caption),
            TextColor(color::TEXT_MUTED),
            Node {
                flex_grow: 1.0,
                ..default()
            },
        ));
        for (phone, key) in [
            (false, "kit.gallery.profile.desktop"),
            (true, "kit.gallery.profile.phone"),
        ] {
            controls::tab(
                bar,
                Localized::new(key),
                Some(if phone {
                    Icon::NavMenu
                } else {
                    Icon::SettingsMonitor
                }),
                TabPlacement::Top,
                Form::Desktop,
                Form::of(phone) == form,
                GalleryAction::Profile(phone),
                format!("GalleryProfile-{phone}"),
            );
        }
        for (index, locale) in crate::i18n::available_locales().enumerate() {
            controls::tab(
                bar,
                locale.native_name(),
                None,
                TabPlacement::Top,
                Form::Desktop,
                locale == crate::i18n::active(),
                GalleryAction::Language(index),
                format!("GalleryLanguage-{}", locale.code()),
            );
        }
    });
    root.spawn(Node {
        column_gap: Val::Px(space::S4),
        flex_shrink: 0.0,
        border: UiRect::bottom(Val::Px(1.0)),
        ..default()
    })
    .insert(BorderColor::all(color::BORDER_HAIRLINE))
    .with_children(|tabs| {
        for (index, (_, key)) in PAGES.iter().enumerate() {
            controls::tab(
                tabs,
                Localized::new(key),
                None,
                TabPlacement::Top,
                Form::Desktop,
                index == page,
                GalleryAction::Page(index),
                format!("GalleryPage-{index}"),
            );
        }
    });
}

/// A labelled row of cells.
fn row(
    body: &mut ChildSpawnerCommands,
    label: &'static str,
    cells: impl FnOnce(&mut ChildSpawnerCommands),
) {
    body.spawn(Node {
        column_gap: Val::Px(space::S16),
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|line| {
        line.spawn((
            Localized::new(label).into_text(),
            theme::role_text(TextRole::Label),
            TextColor(color::TEXT_SECONDARY),
            Node {
                width: Val::Px(LABEL_W),
                flex_shrink: 0.0,
                ..default()
            },
        ));
        line.spawn(Node {
            column_gap: Val::Px(space::S16),
            row_gap: Val::Px(space::S12),
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            ..default()
        })
        .with_children(cells);
    });
}

/// A fixed-width cell holding one component, captioned with its state
/// (`label` is an i18n key; empty for none).
fn cell(
    line: &mut ChildSpawnerCommands,
    width: f32,
    label: &'static str,
    content: impl FnOnce(&mut ChildSpawnerCommands),
) {
    line.spawn(Node {
        width: Val::Px(width),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: Val::Px(space::S8),
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|cell| {
        if !label.is_empty() {
            cell.spawn((
                Localized::new(label).into_text(),
                theme::role_text(TextRole::Caption),
                TextColor(color::TEXT_MUTED),
            ));
        }
        cell.spawn(Node {
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(content);
    });
}

/// Pins `entity` to column `index` of [`STATES`].
fn pin(commands: &mut Commands, entity: Entity, index: usize) {
    let (state, focused, _) = STATES[index];
    match (index, state) {
        (4, _) => {
            commands.entity(entity).insert(Pressable {
                disabled: true,
                ..default()
            });
        }
        (5, _) => {
            commands.entity(entity).queue(|mut entity: EntityWorldMut| {
                if let Some(mut style) = entity.get_mut::<ButtonStyle>() {
                    style.selected = true;
                }
            });
        }
        (_, Some(state)) => {
            commands
                .entity(entity)
                .insert(PreviewState { state, focused });
        }
        _ => {}
    }
}

fn state_labels(count: usize) -> Vec<&'static str> {
    STATES
        .iter()
        .take(count)
        .map(|(_, _, label)| *label)
        .collect()
}

fn buttons_page(body: &mut ChildSpawnerCommands, form: Form) {
    let rows: [(&str, ButtonSize, ButtonKind, &str, usize); 7] = [
        (
            "kit.gallery.row.primary_lg",
            ButtonSize::Large,
            ButtonKind::Primary,
            "home.play.play",
            5,
        ),
        (
            "kit.gallery.row.primary",
            ButtonSize::Regular,
            ButtonKind::Primary,
            "draft.button.lock_in",
            5,
        ),
        (
            "kit.gallery.row.secondary",
            ButtonSize::Regular,
            ButtonKind::Secondary,
            "pause.button.settings",
            6,
        ),
        (
            "kit.gallery.row.tertiary",
            ButtonSize::Regular,
            ButtonKind::Link,
            "postmatch.button.back_to_menu",
            6,
        ),
        (
            "kit.gallery.row.danger",
            ButtonSize::Regular,
            ButtonKind::Danger,
            "lobby.button.leave",
            5,
        ),
        (
            "kit.gallery.row.team",
            ButtonSize::Regular,
            ButtonKind::Team(crate::domain::Team::Green),
            "draft.button.lock_in",
            5,
        ),
        (
            "kit.gallery.row.team",
            ButtonSize::Regular,
            ButtonKind::Team(crate::domain::Team::Blue),
            "draft.button.lock_in",
            5,
        ),
    ];
    for (label, size, kind, key, columns) in rows {
        row(body, label, |line| {
            for index in 0..columns {
                cell(
                    line,
                    if size == ButtonSize::Large {
                        size::BUTTON_LG_MIN_WIDTH.at(form)
                    } else {
                        CELL_W
                    },
                    STATES[index].2,
                    |cell| {
                        let entity = kit_button(cell, size, kind, key, None, form);
                        pin(&mut cell.commands(), entity, index);
                    },
                );
            }
        });
    }
    row(body, "kit.gallery.row.hero", |line| {
        cell(line, size::BUTTON_HERO_WIDTH.at(form), "", |cell| {
            kit_button(
                cell,
                ButtonSize::Hero,
                ButtonKind::Primary,
                "home.play.play",
                None,
                form,
            );
        });
        cell(line, CELL_W + space::S48, "", |cell| {
            kit_button(
                cell,
                ButtonSize::Regular,
                ButtonKind::Secondary,
                "home.play.party_lobby",
                Some(Icon::NavUsers),
                form,
            );
        });
        cell(line, CELL_W, "", |cell| {
            kit_button(
                cell,
                ButtonSize::Regular,
                ButtonKind::Link,
                "career.button.devices",
                Some(Icon::NavLink),
                form,
            );
        });
    });
}

fn kit_button(
    parent: &mut ChildSpawnerCommands,
    size: ButtonSize,
    kind: ButtonKind,
    key: &'static str,
    icon: Option<Icon>,
    form: Form,
) -> Entity {
    let style = match (size, kind) {
        (_, ButtonKind::Link) => {
            theme::TextStyle::new(TextRole::Button).sized(widgets::TERTIARY_LABEL)
        }
        (ButtonSize::Large | ButtonSize::Hero, _) => theme::TextStyle::new(TextRole::ButtonLg),
        _ => theme::TextStyle::new(TextRole::Button),
    };
    widgets::spawn_button(
        parent,
        widgets::button_node(size, kind, form),
        Localized::new(key),
        style,
        kind,
        icon,
        GalleryAction::Noop,
        crate::ui::TestId::new(format!("Gallery-{key}")),
        (),
    )
}

fn controls_page(body: &mut ChildSpawnerCommands, form: Form) {
    row(body, "kit.gallery.row.icon_button", |line| {
        for index in 0..6 {
            cell(line, CELL_W, STATES[index].2, |cell| {
                let entity = controls::sized_icon_button(
                    cell,
                    if index == 5 {
                        Icon::NavMessageCircle
                    } else {
                        Icon::NavSettings
                    },
                    form,
                    ButtonKind::Secondary,
                    GalleryAction::Noop,
                    "GalleryIcon",
                );
                pin(&mut cell.commands(), entity, index);
            });
        }
    });
    row(body, "kit.gallery.row.icon_button", |line| {
        // Desktop 40 and phone 44 side by side.
        controls::icon_button(
            line,
            Icon::NavHelpCircle,
            ButtonKind::Secondary,
            GalleryAction::Noop,
            "GalleryIcon40",
        );
        controls::sized_icon_button(
            line,
            Icon::NavMessageCircle,
            Form::Phone,
            ButtonKind::Secondary,
            GalleryAction::Noop,
            "GalleryIcon44",
        );
    });
    row(body, "kit.gallery.row.stepper", |line| {
        for index in 0..5 {
            cell(line, CELL_W, STATES[index].2, |cell| {
                cell.spawn(Node {
                    column_gap: Val::Px(space::S8),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|stepper| {
                    let minus = controls::stepper_button(
                        stepper,
                        Icon::NavMinus,
                        GalleryAction::Noop,
                        "GalleryStepDown",
                    );
                    pin(&mut stepper.commands(), minus, index);
                    widgets::value_label(stepper, "80%".into(), GalleryValue, "GalleryStepValue");
                    let plus = controls::stepper_button(
                        stepper,
                        Icon::NavPlus,
                        GalleryAction::Noop,
                        "GalleryStepUp",
                    );
                    if index == 4 {
                        pin(&mut stepper.commands(), plus, 4);
                    }
                });
            });
        }
    });
    for (label, placement, key, icon) in [
        (
            "kit.gallery.row.tab_top",
            TabPlacement::Top,
            "pause.settings.sound",
            Icon::SettingsVolume2,
        ),
        (
            "kit.gallery.row.tab_rail",
            TabPlacement::Rail,
            "pause.settings.language",
            Icon::SettingsLanguages,
        ),
    ] {
        row(body, label, |line| {
            for index in 0..6 {
                let width = if placement == TabPlacement::Rail {
                    size::TAB_RAIL_WIDTH.at(form)
                } else {
                    CELL_W
                };
                cell(line, width, STATES[index].2, |cell| {
                    let entity = controls::tab(
                        cell,
                        Localized::new(key),
                        Some(icon),
                        placement,
                        form,
                        false,
                        GalleryAction::Noop,
                        "GalleryTab",
                    );
                    pin(&mut cell.commands(), entity, index);
                });
            }
        });
    }
}

#[derive(Component)]
struct GalleryValue;

fn inputs_page(body: &mut ChildSpawnerCommands, form: Form) {
    let toggle_states = [
        (None, false, false, "kit.gallery.state.off"),
        (None, false, true, "kit.gallery.state.on"),
        (
            Some(ButtonState::Hover),
            false,
            false,
            "kit.gallery.state.hover",
        ),
        (
            Some(ButtonState::Idle),
            true,
            true,
            "kit.gallery.state.focused",
        ),
        (
            Some(ButtonState::Disabled),
            false,
            false,
            "kit.gallery.state.disabled",
        ),
    ];
    row(body, "kit.gallery.row.toggle", |line| {
        for (state, focused, on, label) in toggle_states {
            cell(line, CELL_W, label, |cell| {
                let entity = controls::toggle(
                    cell,
                    Localized::new("pause.audio.mute"),
                    on,
                    GalleryAction::Noop,
                    "GalleryToggle",
                );
                if let Some(state) = state {
                    let mut commands = cell.commands();
                    if state == ButtonState::Disabled {
                        commands.entity(entity).insert(Pressable {
                            disabled: true,
                            ..default()
                        });
                    } else {
                        commands
                            .entity(entity)
                            .insert(PreviewState { state, focused });
                    }
                }
            });
        }
    });
    for (label, spawn) in [
        ("kit.gallery.row.slider", 0),
        ("kit.gallery.row.cycle", 1),
        ("kit.gallery.row.input", 2),
    ] {
        row(body, label, |line| {
            for index in 0..5 {
                cell(
                    line,
                    if spawn == 0 {
                        SLIDER_CELL_W
                    } else {
                        CELL_W * 2.0 + space::S24
                    },
                    STATES[index].2,
                    |cell| {
                        let entity = match spawn {
                            0 => controls::slider(
                                cell,
                                Localized::new("pause.audio.master"),
                                0.7,
                                form,
                                "GallerySlider",
                            ),
                            1 => {
                                let row = controls::cycle_row(
                                    cell,
                                    Localized::new("pause.settings.language"),
                                    if index == 1 {
                                        "简体中文"
                                    } else {
                                        "English"
                                    }, // i18n-allow
                                    GalleryValue,
                                    GalleryAction::Noop,
                                    GalleryAction::Noop,
                                    form,
                                    "GalleryCycle",
                                );
                                // Pin the control (the row's second child).
                                cell.commands().queue(move |world: &mut World| {
                                    let control = world
                                        .get::<Children>(row)
                                        .and_then(|children| children.get(1).copied());
                                    if let Some(control) = control {
                                        pin_world(world, control, index);
                                    }
                                });
                                Entity::PLACEHOLDER
                            }
                            _ => controls::text_input(
                                cell,
                                Localized::new(if index == 3 {
                                    "kit.gallery.sample.server"
                                } else {
                                    "career.friends.find_prompt"
                                }),
                                (index != 3).then_some(Icon::NavSearch),
                                GalleryValue,
                                GalleryAction::Noop,
                                "GalleryInput",
                            ),
                        };
                        if entity != Entity::PLACEHOLDER {
                            pin(&mut cell.commands(), entity, index);
                        }
                    },
                );
            }
        });
    }
    // Text field extra states: editing and error.
    row(body, "kit.gallery.row.input", |line| {
        for (editing, error, label) in [
            (true, false, "kit.gallery.state.editing"),
            (false, true, "kit.gallery.state.error"),
        ] {
            cell(line, CELL_W * 2.0 + space::S24, "", |cell| {
                cell.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(100.0),
                    row_gap: Val::Px(space::S4),
                    ..default()
                })
                .with_children(|column| {
                    column.spawn((
                        Localized::new(label).into_text(),
                        theme::role_text(TextRole::Caption),
                        TextColor(color::TEXT_MUTED),
                    ));
                    let field = controls::text_input(
                        column,
                        Localized::new("career.friends.find_prompt"),
                        None,
                        GalleryValue,
                        GalleryAction::Noop,
                        "GalleryInputState",
                    );
                    let name = NAMES[if editing { 1 } else { 0 }];
                    column.commands().queue(move |world: &mut World| {
                        if let Some(mut style) = world.get_mut::<ButtonStyle>(field) {
                            style.selected = editing;
                        }
                        if let Some(mut flag) = world.get_mut::<controls::InputError>(field) {
                            flag.0 = error;
                        }
                        let label = world
                            .get::<widgets::KitParts>(field)
                            .and_then(|parts| parts.label);
                        if let Some(mut text) = label.and_then(|label| world.get_mut::<Text>(label))
                        {
                            text.0 = name.into();
                        }
                    });
                });
            });
        }
    });
}

fn pin_world(world: &mut World, entity: Entity, index: usize) {
    let (state, focused, _) = STATES[index];
    match (index, state) {
        (4, _) => {
            if let Some(mut pressable) = world.get_mut::<Pressable>(entity) {
                pressable.disabled = true;
            }
        }
        (5, _) => {}
        (_, Some(state)) => {
            world
                .entity_mut(entity)
                .insert(PreviewState { state, focused });
        }
        _ => {}
    }
}

fn panels_page(body: &mut ChildSpawnerCommands, form: Form) {
    row(body, "kit.gallery.row.panels", |line| {
        line.spawn(Node {
            width: Val::Px(260.0),
            height: Val::Px(180.0),
            ..default()
        })
        .with_child(surfaces::ornament_frame());
        line.spawn(surfaces::plain_panel())
            .insert(Node {
                width: Val::Px(200.0),
                height: Val::Px(180.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(space::S8),
                ..theme::panel_node()
            })
            .with_children(|panel| {
                panel.spawn((
                    Localized::new("career.friends.title").into_text(),
                    theme::role_text(TextRole::Heading),
                    TextColor(color::TEXT_GOLD),
                ));
                panel.spawn((
                    Localized::new("kit.gallery.sample.body").into_text(),
                    theme::role_text(TextRole::Body),
                    TextColor(color::TEXT_SECONDARY),
                ));
            });
        line.spawn(surfaces::framed_panel(form))
            .insert(Node {
                width: Val::Px(240.0),
                height: Val::Px(180.0),
                padding: UiRect::all(Val::Px(surfaces::FRAMED_PADDING.at(form))),
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|panel| {
                surfaces::panel_header(
                    panel,
                    Some(Localized::new("kit.gallery.sample.eyebrow")),
                    Localized::new("kit.gallery.sample.heading"),
                );
            });
        line.spawn(Node {
            width: Val::Px(300.0),
            height: Val::Px(180.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|frame| {
            let panel = surfaces::modal(
                frame,
                Localized::new("pause.title"),
                GalleryAction::Noop,
                form,
                "GalleryModal",
            );
            frame
                .commands()
                .entity(panel)
                .queue(|mut panel: EntityWorldMut| {
                    if let Some(mut node) = panel.get_mut::<Node>() {
                        node.width = Val::Px(260.0);
                        node.height = Val::Px(140.0);
                    }
                });
        });
    });
    row(body, "kit.gallery.row.badge", |line| {
        surfaces::badge(
            line,
            Localized::new("kit.gallery.sample.mvp"),
            BadgeKind::Gold,
            false,
        );
        surfaces::badge(
            line,
            Localized::new("career.presence.online"),
            BadgeKind::Green,
            false,
        );
        surfaces::badge(
            line,
            Localized::new("kit.gallery.sample.blue"),
            BadgeKind::Blue,
            false,
        );
        surfaces::badge(
            line,
            Localized::new("career.presence.offline"),
            BadgeKind::Danger,
            false,
        );
        surfaces::badge(
            line,
            Localized::new("kit.gallery.sample.bot"),
            BadgeKind::Muted,
            false,
        );
        surfaces::badge(line, "3", BadgeKind::Gold, true);
    });
    row(body, "kit.gallery.row.list_row", |line| {
        for index in 0..6 {
            cell(line, 360.0, STATES[index].2, |cell| {
                let entity = surfaces::list_row(
                    cell,
                    RowLeading::Portrait(ART[0].into()),
                    NAMES[0],
                    Localized::new("career.presence.playing"),
                    GalleryAction::Noop,
                    "GalleryRow",
                    |trailing| {
                        surfaces::badge(
                            trailing,
                            Localized::new("career.presence.online"),
                            BadgeKind::Green,
                            false,
                        );
                        controls::sized_icon_button(
                            trailing,
                            Icon::NavPlus,
                            Form::Desktop,
                            ButtonKind::Secondary,
                            GalleryAction::Noop,
                            "GalleryRowAdd",
                        );
                    },
                );
                pin(&mut cell.commands(), entity, index);
            });
        }
    });
    row(body, "kit.gallery.row.list_row", |line| {
        for leading in [RowLeading::Icon(Icon::NavHistory), RowLeading::None] {
            cell(line, 360.0, "", |cell| {
                surfaces::list_row(
                    cell,
                    leading,
                    Localized::new("career.history.title"),
                    Localized::new("career.button.last_match"),
                    GalleryAction::Noop,
                    "GalleryRowPlain",
                    |trailing| {
                        trailing.spawn((
                            Text::new("+24"),
                            theme::role_text(TextRole::Number),
                            TextColor(color::TEXT_GOLD),
                        ));
                    },
                );
            });
        }
    });
}

fn feedback_page(body: &mut ChildSpawnerCommands, form: Form) {
    row(body, "kit.gallery.row.tooltip", |line| {
        let ability = shared::ability_for_class_slot(shared::HeroClass::Mage, shared::SkillSlot::Q);
        surfaces::tooltip_panel(
            line,
            Some(format!("{} · Q", crate::i18n::data::ability_name(ability))),
            crate::i18n::data::ability_desc(ability),
        );
        surfaces::tooltip_panel(line, None::<&str>, Localized::new("social.entry.chat"));
    });
    row(body, "kit.gallery.row.toast", |line| {
        for (kind, text) in [
            (
                ToastKind::Info,
                crate::i18n::trf("kit.gallery.sample.toast_info", &[("name", &NAMES[0])]),
            ),
            (
                ToastKind::Success,
                crate::i18n::tr("kit.gallery.sample.toast_success").into(),
            ),
            (
                ToastKind::Warning,
                crate::i18n::tr("kit.gallery.sample.toast_warning").into(),
            ),
            (
                ToastKind::Danger,
                crate::i18n::tr("kit.gallery.sample.toast_danger").into(),
            ),
        ] {
            surfaces::toast_panel(line, kind, text);
        }
    });
    row(body, "kit.gallery.row.bars", |line| {
        for (kind, current, max, respawn, value) in [
            (BarKind::HpSelf, 130.0, 203.0, None, true),
            (BarKind::HpEnemy, 302.0, 720.0, None, true),
            (BarKind::Mana, 80.0, 100.0, None, true),
            (BarKind::Xp, 35.0, 100.0, None, false),
            (BarKind::Loading, 72.0, 100.0, None, false),
            (BarKind::HpSelf, 0.0, 203.0, Some(8), true),
        ] {
            cell(line, 260.0, "", |cell| {
                game::bar(
                    cell,
                    kind,
                    BarValue {
                        current,
                        max,
                        respawn,
                    },
                    Val::Px(260.0),
                    form,
                    value,
                );
            });
        }
    });
}

fn ability(ability: &'static str, key: &'static str) -> AbilityView {
    AbilityView {
        ability: Some(ability),
        icon: Icon::HudAttack,
        key: Some(key),
        cost: Some(22),
        rank: 2,
        cooldown: None,
        locked: false,
        no_mana: false,
        pips: true,
    }
}

fn abilities_page(body: &mut ChildSpawnerCommands, form: Form) {
    let labels = [
        "kit.gallery.state.idle",
        "kit.gallery.state.hover",
        "kit.gallery.state.pressed",
        "kit.gallery.state.focused",
        "kit.gallery.state.cooldown",
        "kit.gallery.state.no_mana",
        "kit.gallery.state.locked",
        "kit.gallery.state.level_up",
        "kit.gallery.state.aiming",
    ];
    row(body, "kit.gallery.row.ability", |line| {
        for (index, _) in labels.iter().enumerate() {
            cell(line, 96.0, labels[index], |cell| {
                cell.spawn(Node {
                    width: Val::Px(size::ABILITY.at(form)),
                    height: Val::Px(size::ABILITY.at(form) + space::S24),
                    ..default()
                })
                .with_children(|slot| {
                    let mut view = ability("arc_bolt", "Q"); // i18n-allow
                    match index {
                        4 => view.cooldown = Some((3.0, 8.0)),
                        5 => view.no_mana = true,
                        6 => view.locked = true,
                        _ => {}
                    }
                    let entity = game::ability_button(
                        slot,
                        view,
                        size::ABILITY.at(form),
                        GalleryAction::Noop,
                        "GalleryAbility",
                    );
                    let mut commands = slot.commands();
                    match index {
                        1 => {
                            commands.entity(entity).insert(PreviewState {
                                state: ButtonState::Hover,
                                focused: false,
                            });
                        }
                        2 => {
                            commands.entity(entity).insert(PreviewState {
                                state: ButtonState::Pressed,
                                focused: false,
                            });
                        }
                        3 => {
                            commands.entity(entity).insert(PreviewState {
                                state: ButtonState::Idle,
                                focused: true,
                            });
                        }
                        8 => {
                            commands.entity(entity).queue(|mut e: EntityWorldMut| {
                                if let Some(mut style) = e.get_mut::<ButtonStyle>() {
                                    style.selected = true;
                                }
                            });
                        }
                        _ => {}
                    }
                    if index == 7 {
                        game::ability_upgrade(
                            slot,
                            size::ABILITY.at(form),
                            GalleryAction::Noop,
                            "GalleryUpgrade",
                        );
                    }
                });
            });
        }
    });
    row(body, "kit.gallery.row.phone_controls", |line| {
        for (side, icon) in [
            (size::ABILITY_ATTACK_PHONE, Icon::HudAttack),
            (size::ABILITY.phone, Icon::HudAttack),
            (size::ABILITY_UTILITY_PHONE, Icon::HudDash),
        ] {
            let mut view = ability("arc_bolt", "Q"); // i18n-allow
            view.key = None;
            view.cost = None;
            view.pips = false;
            if side != size::ABILITY.phone {
                view.ability = None;
                view.icon = icon;
            }
            game::ability_button(line, view, side, GalleryAction::Noop, "GalleryPhoneControl");
        }
    });
    row(body, "kit.gallery.row.item_slot", |line| {
        for index in 0..6 {
            cell(line, 64.0, STATES[index].2, |cell| {
                let entity = game::item_slot_button(
                    cell,
                    Icon::ItemEmberBlade32,
                    form,
                    GalleryAction::Noop,
                    "GallerySlot",
                );
                pin(&mut cell.commands(), entity, index);
            });
        }
        game::item_slot(line, Some(Icon::ItemVitalityGem32), Some(2), form);
    });
    row(body, "kit.gallery.row.shop_card", |line| {
        for index in 0..6 {
            cell(line, game::SHOP_CARD_W.at(form), STATES[index].2, |cell| {
                let entity = game::shop_card(
                    cell,
                    ShopCard {
                        item: shared::shop::ItemId::EmberBlade,
                        price: 80,
                        owned: false,
                        recommended: index == 0,
                        missing: (index == 4).then_some(35),
                    },
                    form,
                    GalleryAction::Noop,
                    "GalleryShopCard",
                );
                if index != 4 {
                    pin(&mut cell.commands(), entity, index);
                }
            });
        }
    });
}

fn heroes_page(body: &mut ChildSpawnerCommands, form: Form) {
    let mut labels = state_labels(6);
    labels.push("kit.gallery.state.studio");
    row(body, "kit.gallery.row.hero_tile", |line| {
        for index in 0..7 {
            cell(
                line,
                size::HERO_TILE.at(form) + space::S16,
                labels[index],
                |cell| {
                    let studio = index == 6;
                    let entity = game::hero_tile(
                        cell,
                        Some(ART[if studio { 1 } else { 0 }].into()),
                        shared::HeroClass::Mage,
                        if studio { "Crowley" } else { "Agnes" }, // i18n-allow
                        if studio {
                            AvatarSource::Studio
                        } else {
                            AvatarSource::Included
                        },
                        form,
                        GalleryAction::Noop,
                        "GalleryHeroTile",
                    );
                    if !studio {
                        pin(&mut cell.commands(), entity, index);
                    }
                },
            );
        }
        game::hero_tile(
            line,
            Some(ART[2].into()),
            shared::HeroClass::Cleric,
            "Lady Koi", // i18n-allow
            AvatarSource::Supporter,
            form,
            GalleryAction::Noop,
            "GalleryHeroTileSupporter",
        );
        game::portrait(line, ART[0].into(), 6, 0.35, size::PORTRAIT_MD);
        game::hero_tile(
            line,
            None,
            shared::HeroClass::Warden,
            "BaoSamurai", // i18n-allow
            AvatarSource::Owned,
            form,
            GalleryAction::Noop,
            "GalleryHeroTileNoArt",
        );
    });
    row(body, "kit.gallery.row.scoreboard", |line| {
        line.spawn(Node {
            width: Val::Px(if form == Form::Desktop { 620.0 } else { 440.0 }),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(space::S4),
            ..default()
        })
        .with_children(|board| {
            for (index, (own, offline)) in [(false, false), (true, false), (false, true)]
                .into_iter()
                .enumerate()
            {
                game::scoreboard_row(
                    board,
                    ScoreRow {
                        team: if index == 2 {
                            crate::domain::Team::Blue
                        } else {
                            crate::domain::Team::Green
                        },
                        art: ART[index + 1].into(),
                        name: NAMES[index].into(),
                        class: crate::i18n::data::hero_name(shared::HeroClass::ALL[index]).into(),
                        level: 12 + index as u32,
                        kda: (12, 2, 18),
                        cs: 148,
                        gold: 7401,
                        items: vec![Some(Icon::ItemEmberBlade32), Some(Icon::ItemTrailBoots32)],
                        own,
                        disconnected: offline,
                    },
                    form,
                );
            }
        });
    });
}

fn hud_page(body: &mut ChildSpawnerCommands, form: Form) {
    row(body, "kit.gallery.row.timer", |line| {
        game::timer_ring(
            line,
            TimerRing {
                progress: 47.0 / 60.0,
                warning: false,
            },
            RingSize::Large,
            "0:47".into(),
            Some("4/10".into()),
        );
        game::timer_ring(
            line,
            TimerRing {
                progress: 24.0 / 30.0,
                warning: false,
            },
            RingSize::Medium,
            "24".into(),
            None,
        );
        game::timer_ring(
            line,
            TimerRing {
                progress: 4.0 / 30.0,
                warning: true,
            },
            RingSize::Medium,
            "4".into(),
            None,
        );
    });
    row(body, "kit.gallery.row.hud", |line| {
        game::minimap_frame(line, form);
        game::player_status(
            line,
            ART[0].into(),
            6,
            0.35,
            BarValue {
                current: 130.0,
                max: 203.0,
                respawn: None,
            },
            BarValue {
                current: 80.0,
                max: 100.0,
                respawn: None,
            },
            125,
            form,
        );
        game::score_strip(line, 12, 9, "18:04".into(), (7, 2, 11));
        game::target_frame(
            line,
            ART[3].into(),
            NAMES[2].into(),
            shared::HeroClass::Ranger,
            BarValue {
                current: 302.0,
                max: 720.0,
                respawn: None,
            },
            true,
            form,
        );
    });
}

fn type_page(body: &mut ChildSpawnerCommands) {
    let samples: [(TextRole, &str); 14] = [
        (TextRole::TitleXl, "postmatch.outcome.victory"),
        (TextRole::Title, "pause.settings.title"),
        (TextRole::Heading, "pause.title"),
        (TextRole::NameLg, "career.friends.title"),
        (TextRole::Eyebrow, "kit.gallery.sample.eyebrow"),
        (TextRole::ButtonLg, "home.play.quick_match"),
        (TextRole::Button, "pause.button.resume"),
        (TextRole::Label, "pause.audio.master"),
        (TextRole::Body, "career.friends.scope"),
        (TextRole::Caption, "hud.target.none"),
        (TextRole::NumberXl, ""),
        (TextRole::NumberLg, ""),
        (TextRole::Number, ""),
        (TextRole::NumberSm, ""),
    ];
    body.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(space::S32),
        ..default()
    })
    .with_children(|columns| {
        columns
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(space::S8),
                width: Val::Px(760.0),
                ..default()
            })
            .with_children(|list| {
                for (role, key) in samples {
                    list.spawn(Node {
                        column_gap: Val::Px(space::S16),
                        align_items: AlignItems::Baseline,
                        ..default()
                    })
                    .with_children(|line| {
                        line.spawn((
                            Text::new(role.token()),
                            theme::role_text(TextRole::Caption),
                            TextColor(color::TEXT_MUTED),
                            Node {
                                width: Val::Px(LABEL_W),
                                flex_shrink: 0.0,
                                ..default()
                            },
                        ));
                        if key.is_empty() {
                            line.spawn((
                                Text::new("0:47 · 7 401 · 12/2/18"),
                                theme::role_text(role),
                                TextColor(color::TEXT_PRIMARY),
                            ));
                        } else {
                            line.spawn((
                                Localized::new(key).into_text(),
                                theme::role_text(role),
                                TextColor(
                                    if matches!(
                                        role,
                                        TextRole::TitleXl | TextRole::Title | TextRole::Heading
                                    ) {
                                        color::TEXT_GOLD
                                    } else {
                                        color::TEXT_PRIMARY
                                    },
                                ),
                                Node {
                                    max_width: Val::Px(560.0),
                                    ..default()
                                },
                            ));
                        }
                    });
                }
            });
        columns
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(space::S8),
                ..default()
            })
            .with_children(|icons| {
                icons.spawn((
                    Localized::new("kit.gallery.row.icons").into_text(),
                    theme::role_text(TextRole::Label),
                    TextColor(color::TEXT_SECONDARY),
                ));
                icons
                    .spawn(Node {
                        width: Val::Px(360.0),
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(space::S12),
                        row_gap: Val::Px(space::S12),
                        ..default()
                    })
                    .with_children(|grid| {
                        for icon in Icon::ALL {
                            grid.spawn(widgets::icon_node(icon, size::ICON_LG, color::TEXT_GOLD));
                        }
                    });
            });
    });
}

fn backgrounds_page(body: &mut ChildSpawnerCommands, form: Form) {
    row(body, "kit.gallery.row.backgrounds", |line| {
        for background in Background::ALL {
            line.spawn((
                Node {
                    width: Val::Px(480.0),
                    height: Val::Px(270.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(space::S16),
                    ..default()
                },
                KitImage::background(background),
            ))
            .with_children(|scene| {
                let primary = kit_button(
                    scene,
                    ButtonSize::Large,
                    ButtonKind::Primary,
                    "home.play.play",
                    None,
                    form,
                );
                pin(&mut scene.commands(), primary, 3);
                let secondary = kit_button(
                    scene,
                    ButtonSize::Regular,
                    ButtonKind::Secondary,
                    "pause.button.settings",
                    None,
                    form,
                );
                pin(&mut scene.commands(), secondary, 3);
            });
        }
    });
}

fn observe_capture(
    mut commands: Commands,
    mut gallery: ResMut<Gallery>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    images: Query<&ImageNode>,
    assets: Res<AssetServer>,
    mut exit: MessageWriter<AppExit>,
) {
    let dirty = gallery.dirty;
    let Some(capture) = gallery.capture.as_mut() else {
        return;
    };
    let Some(shot) = capture.shots.get(capture.index).cloned() else {
        return;
    };
    let file = capture.directory.join(&shot.file);
    if capture.in_flight {
        if file.metadata().is_ok_and(|meta| meta.len() > 32) {
            capture.in_flight = false;
            capture.done.push(serde_json::json!({
                "file": shot.file,
                "page": PAGES[shot.page].0,
                "profile": if shot.form == Form::Desktop { "desktop" } else { "phone" },
                "language": shot.locale.code(),
                "pixels": [shot.pixels.x, shot.pixels.y],
            }));
            capture.index += 1;
            capture.settled = 0;
            if capture.index == capture.shots.len() {
                let summary = serde_json::json!({
                    "status": "passed",
                    "scenario": "ui-kit-gallery",
                    "version": env!("CARGO_PKG_VERSION"),
                    "method": "real Bevy primary_window ScreenshotCaptured + save_to_disk",
                    "captures": capture.done,
                });
                let saved = std::fs::write(
                    capture.directory.join("gallery-summary.json"),
                    serde_json::to_vec_pretty(&summary).unwrap_or_default(),
                );
                if let Ok((entity, _)) = windows.single() {
                    commands.entity(entity).despawn();
                }
                exit.write(if saved.is_ok() {
                    AppExit::Success
                } else {
                    AppExit::error()
                });
            }
        }
        return;
    }
    let Ok((_, window)) = windows.single() else {
        return;
    };
    if dirty
        || window.resolution.physical_width() != shot.pixels.x
        || window.resolution.physical_height() != shot.pixels.y
    {
        capture.settled = 0;
        return;
    }
    capture.settled += 1;
    let loaded = images.iter().all(|image| {
        assets.is_loaded_with_dependencies(&image.image) || image.image == Handle::default()
    });
    if capture.settled < 24 || (!loaded && capture.settled < 600) {
        return;
    }
    if std::fs::create_dir_all(&capture.directory).is_err() {
        error!("UI gallery: cannot create {}", capture.directory.display());
        exit.write(AppExit::error());
        return;
    }
    commands
        .spawn((Screenshot::primary_window(), ShotMarker))
        .observe(save_to_disk(file))
        .observe(|_: On<ScreenshotCaptured>| {});
    capture.in_flight = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every page spawns on both profiles (no duplicate components, no
    /// missing keys) with the kit painters running over it.
    #[test]
    fn every_page_spawns_on_both_profiles() {
        for form in [Form::Desktop, Form::Phone] {
            for page in 0..PAGES.len() {
                let mut app = App::new();
                app.add_plugins(bevy::time::TimePlugin)
                    .insert_resource(UiForm(form))
                    .init_resource::<crate::ui::kit_assets::UiDensity>()
                    .init_resource::<crate::ui::kit_assets::KitAtlasLayouts>()
                    .add_message::<crate::ui::focus::FocusAdjust>()
                    .add_message::<crate::ui::SyntheticPress>()
                    .add_systems(
                        Update,
                        (
                            widgets::paint_pressables,
                            widgets::paint_slabs,
                            widgets::paint_kit,
                            widgets::paint_preview_rings,
                        )
                            .chain(),
                    );
                let root = app.world_mut().spawn(Node::default()).id();
                app.world_mut()
                    .commands()
                    .entity(root)
                    .with_children(|body| {
                        header(body, page, form);
                        spawn_page(body, page, form);
                    });
                app.world_mut().flush();
                app.update();
                app.update();
                let nodes = app.world_mut().query::<&Node>().iter(app.world()).count();
                assert!(nodes > 20, "page {page} {form:?}: {nodes} nodes");
            }
        }
    }

    #[test]
    fn capture_plan_covers_every_page_profile_and_language_once() {
        let shots = shots();
        let mut files: Vec<&str> = shots.iter().map(|shot| shot.file.as_str()).collect();
        let total = files.len();
        files.sort_unstable();
        files.dedup();
        assert_eq!(files.len(), total, "unique files");
        assert_eq!(total, PAGES.len() * 2 * 2 + 4);
        for page in 0..PAGES.len() {
            assert_eq!(
                shots
                    .iter()
                    .filter(|shot| shot.page == page && shot.pixels == UVec2::new(1280, 720))
                    .count(),
                4
            );
        }
    }
}
