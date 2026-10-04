//! In-match pause menu: main page, settings page and the debug tools page
//! (`crate::debug::tools_page`). Built on the UI kit (`crate::ui`): every control carries a
//! `PauseAction`, the kit recognizes clicks and taps and paints the buttons,
//! and the systems here only consume `Activated<PauseAction>`.
// i18n-strict
use bevy::{
    app::AppExit,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use crate::audio_settings::AudioSettings;
use crate::camera::{CAMERA_ZOOM_STEP, CameraSettings};
use crate::i18n::{Locale, Localized, tr, trf};
use crate::mobile_controls::HudPositionSettings;
use crate::model_scale::{
    DEFAULT_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT, MIN_MODEL_TARGET_HEIGHT,
    ModelScaleSettings,
};
use crate::net::{ClientConnectionState, ClientSession, GameState, GameStateSnapshot};
use crate::persistence::{
    ClientPrefsSaveGate, ClientSessionId, ResolvedServerAddressForPrefs, reset_graphics_to_defaults,
};
use crate::render_settings::RenderSettings;
use crate::session_config::DEFAULT_GAME_SERVER_ADDR;
use crate::team::TeamSelection;
use crate::ui::living_background::{
    self, LivingBackground, LivingBands, LivingScene, MotionSettings,
};
use crate::ui::widgets::controls::{Slider, SliderChanged};
use crate::ui::{
    Activated, GestureEpoch, ModalId, ModalRoot, ScrollArea, UiAction, UiActionAppExt, UiSet,
    kit_assets::Icon,
    test_id::NodeKey,
    theme::{self, ButtonKind, metric},
    tokens::{TextRole, color},
    widgets,
};
use crate::world::{
    LightingSettings, MAX_AMBIENT_BRIGHTNESS, MAX_LIGHT_ILLUMINANCE, MAX_LIGHT_PITCH_DEG,
    MAX_LIGHT_YAW_DEG, MIN_AMBIENT_BRIGHTNESS, MIN_LIGHT_ILLUMINANCE, MIN_LIGHT_PITCH_DEG,
    MIN_LIGHT_YAW_DEG,
};

const SCALE_STEP: f32 = 0.04;
const ILLUMINANCE_STEP: f32 = 2_000.0;
const AMBIENT_STEP: f32 = 50.0;
const ANGLE_STEP_DEG: f32 = 5.0;
const AUDIO_STEP: f32 = 0.05;
/// Wheel step (UI px per line) of the main and settings bodies on desktop;
/// on a phone they scroll by touch drag past the tap slop.
const MENU_WHEEL_STEP: f32 = 32.0;

pub struct PauseMenuPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PauseMenuSet {
    Close,
    /// Anchor after the kit dispatched this frame's presses; the handlers
    /// (`Visuals`) and the practice page run after it.
    Taps,
    Visuals,
}

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenuState>()
            .init_resource::<SettingsTab>()
            .init_resource::<AudioSettings>()
            .init_resource::<RenderSettings>()
            .init_resource::<HudPositionSettings>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
            .init_resource::<SettingsHelpReturn>()
            .add_ui_action::<PauseAction>()
            .add_systems(Startup, setup_pause_menu_ui)
            .configure_sets(
                Update,
                (
                    PauseMenuSet::Close
                        .after(UiSet::Gesture)
                        .after(crate::help_overlay::HelpOverlaySet::Input)
                        .after(crate::shop::ShopModalSet)
                        .in_set(crate::input_context::InputContextSet::Modal),
                    PauseMenuSet::Taps
                        .after(PauseMenuSet::Close)
                        .after(UiSet::Dispatch)
                        .in_set(crate::input_context::InputContextSet::Modal),
                    PauseMenuSet::Visuals.after(PauseMenuSet::Taps),
                ),
            )
            .add_systems(
                Update,
                (
                    toggle_pause_menu,
                    close_pause_menu_when_disconnected,
                    bump_gesture_epoch_on_navigation,
                )
                    .chain()
                    .in_set(PauseMenuSet::Close),
            )
            .add_systems(
                Update,
                (
                    apply_pause_navigation,
                    return_to_settings_after_help.after(apply_pause_navigation),
                    apply_pause_settings,
                    apply_render_and_hud_settings,
                    apply_pause_audio,
                    apply_pause_session,
                    apply_pause_language,
                    update_setting_labels
                        .after(apply_pause_settings)
                        .after(apply_render_and_hud_settings),
                    update_language_value.after(apply_pause_language),
                    sync_pause_menu_visibility,
                    sync_pause_menu_sections,
                    sync_settings_living_background,
                    reset_pause_scroll_on_navigation.after(apply_pause_navigation),
                    sync_practice_actions,
                    sync_settings_server_addr_label,
                    settings_tabs,
                )
                    .in_set(PauseMenuSet::Visuals),
            )
            .add_systems(
                PostUpdate,
                (size_desktop_pause_panel, layout_pause_contents)
                    .chain()
                    .before(bevy::ui::UiSystems::Layout),
            )
            .add_systems(
                Update,
                (apply_settings_sliders, sync_settings_sliders)
                    .chain()
                    .after(UiSet::Paint)
                    .after(PauseMenuSet::Visuals),
            )
            .add_systems(
                PostUpdate,
                paint_settings_scrollbar.after(bevy::ui::UiSystems::Layout),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct PauseMenuState {
    pub(crate) open: bool,
    pub(crate) in_settings: bool,
}

#[derive(Resource, Component, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    #[default]
    Sound,
    Graphics,
    Camera,
    Hud,
    Language,
}

#[derive(Component)]
struct SettingsRail;

#[derive(Component)]
struct SettingsRailItem;

#[derive(Component)]
struct SettingsScrollTrack;

#[derive(Component)]
struct SettingsScrollThumb;

#[derive(Component, Clone, Copy)]
enum SettingsSlider {
    Audio(AudioBus),
    Lighting(Setting),
}

fn settings_group(
    parent: &mut ChildSpawnerCommands,
    tab: SettingsTab,
    build: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                max_width: Val::Px(820.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(8.0),
                flex_shrink: 0.0,
                padding: UiRect::all(Val::Px(20.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(color::SURFACE_2),
            BorderColor::all(color::BORDER_SUBTLE),
            tab,
        ))
        .with_children(build);
}

fn settings_tabs(
    mut selected: ResMut<SettingsTab>,
    mut events: MessageReader<Activated<PauseAction>>,
    mut groups: Query<(&SettingsTab, &mut Node)>,
    mut buttons: Query<(&UiAction<PauseAction>, &mut widgets::ButtonStyle)>,
    mut scroll: Query<&mut ScrollPosition, With<SettingsSection>>,
    menu: Res<PauseMenuState>,
    mut titles: Query<(&Name, &mut Localized)>,
) {
    for event in events.read() {
        if let PauseAction::SettingsTab(tab) = event.action {
            *selected = tab;
            for mut position in &mut scroll {
                position.y = 0.0;
            }
        }
    }
    for (tab, mut node) in &mut groups {
        node.display = if *tab == *selected {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (action, mut style) in &mut buttons {
        if let PauseAction::SettingsTab(tab) = action.0 {
            widgets::ButtonStyle::set_selected(&mut style, tab == *selected);
        }
    }
    for (name, mut text) in &mut titles {
        if name.as_str() == "PauseMenuTitle" {
            let key = if menu.in_settings {
                "pause.settings.title"
            } else {
                "pause.title"
            };
            if text.key != key {
                text.key = key;
            }
        }
    }
}

/// The controls guide was opened from Settings (DECISIONS R6.5): closing it
/// reopens Settings where the player left it instead of returning to the game.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(crate) struct SettingsHelpReturn(pub(crate) bool);

#[derive(Component)]
struct PauseMenuRoot;

#[derive(Component)]
struct MainMenuSection;

#[derive(Component)]
struct MainMenuFooter;

#[derive(Component)]
struct SettingsSection;
#[derive(Component)]
struct SettingsFooter;

#[derive(Component)]
struct SettingsServerAddrLabel;

/// The Language row's value (the active language's native name).
#[derive(Component)]
struct LanguageValue;

#[derive(Component)]
struct PauseMenuPanel;

#[derive(Component)]
struct PauseLivingBackground;

/// A stepped graphics setting on the settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Setting {
    CameraZoom,
    RenderFps,
    JoystickX,
    JoystickY,
    CombatX,
    CombatY,
    ModelScale,
    Light,
    Ambient,
    Pitch,
    Yaw,
}

/// The value label next to a setting's stepper.
#[derive(Component, Clone, Copy)]
struct SettingLabel(Setting);

/// Everything a pause menu control can do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PauseAction {
    Resume,
    Close,
    OpenSettings,
    BackFromSettings,
    Help,
    Exit,
    LeavePractice,
    LeaveMatch,
    ResetGraphics,
    ResetHud,
    /// One step of a setting; the sign is the direction.
    Step(Setting, i8),
    Audio(AudioButton),
    OpenPractice,
    /// Switch to the next shipped language.
    CycleLanguage,
    ToggleReduceMotion,
    ToggleFpsReadout,
    TogglePhoneLayoutPreview,
    SettingsTab(SettingsTab),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioBus {
    Master,
    Music,
    Effects,
    Ui,
}

impl AudioBus {
    fn value(self, settings: AudioSettings) -> f32 {
        match self {
            Self::Master => settings.master,
            Self::Music => settings.music,
            Self::Effects => settings.effects,
            Self::Ui => settings.ui,
        }
    }
    fn adjust(self, settings: &mut AudioSettings, delta: f32) {
        *settings = settings.sanitized();
        let value = match self {
            Self::Master => &mut settings.master,
            Self::Music => &mut settings.music,
            Self::Effects => &mut settings.effects,
            Self::Ui => &mut settings.ui,
        };
        *value = ((*value * 100.0).round() + delta * 100.0).clamp(0.0, 100.0) / 100.0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AudioButton {
    Mute,
}
fn size_desktop_pause_panel(
    menu: Res<PauseMenuState>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut panels: Query<&mut Node, With<PauseMenuPanel>>,
) {
    // The phone panel is sized against the safe area by `adapt_phone_layout`.
    let form = metric::Form::from_mobile(mobile.as_deref());
    if form == metric::Form::Phone {
        return;
    }
    let height = if menu.in_settings {
        Val::Percent(92.0)
    } else {
        Val::Px(440.0)
    };
    for mut panel in &mut panels {
        panel.width = if menu.in_settings {
            Val::Percent(94.0)
        } else {
            Val::Px(560.0)
        };
        panel.max_width = Val::Px(if menu.in_settings { 1120.0 } else { 560.0 });
        if panel.height != height {
            panel.height = height;
        }
    }
}

/// Settings entered from the front end use the Arena painting; the same menu
/// opened during a live match deliberately keeps the world visible.
fn sync_settings_living_background(
    mut commands: Commands,
    menu: Res<PauseMenuState>,
    game: Option<Res<GameStateSnapshot>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    platform: Res<crate::ui::UiPlatform>,
    roots: Query<Entity, With<PauseMenuRoot>>,
    backgrounds: Query<Entity, (With<PauseLivingBackground>, With<LivingBackground>)>,
) {
    let in_match = game
        .as_ref()
        .is_some_and(|game| matches!(game.state, GameState::Running))
        && !screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    let should_show = menu.open && menu.in_settings && !in_match;
    if !should_show {
        for background in &backgrounds {
            commands.entity(background).try_despawn();
        }
        return;
    }
    if !backgrounds.is_empty() {
        return;
    }
    let Ok(root) = roots.single() else { return };
    let form = theme::Form::of(platform.is_mobile());
    commands.entity(root).with_children(|parent| {
        let background = living_background::spawn(
            parent,
            LivingScene::Arena,
            LivingBands {
                header: Some(if form == theme::Form::Phone {
                    56.0
                } else {
                    104.0
                }),
                footer: None,
            },
            form,
        );
        parent
            .commands()
            .entity(background)
            .insert(PauseLivingBackground);
    });
}

fn section_title(parent: &mut ChildSpawnerCommands, key: &'static str, name: &str) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(28.0),
                align_items: AlignItems::Center,
                column_gap: Val::Px(16.0),
                margin: UiRect::bottom(Val::Px(4.0)),
                flex_shrink: 0.0,
                ..default()
            },
            Name::new(format!("{name}Group")), // i18n-allow: ECS debug name, not player text.
        ))
        .with_children(|heading| {
            heading.spawn((
                Localized::new(key).into_text(),
                theme::role_text(TextRole::Heading),
                TextColor(theme::GOLD),
                Name::new(name.to_owned()),
            ));
            heading.spawn((
                Node {
                    height: Val::Px(1.0),
                    flex_grow: 1.0,
                    ..default()
                },
                BackgroundColor(color::BORDER_SUBTLE),
            ));
        });
}

/// Settings own their responsive columns; the slider kit owns dragging,
/// focus and the thumb. Fixed columns keep values aligned between sections.
fn settings_slider(
    parent: &mut ChildSpawnerCommands,
    label: Localized,
    source: SettingsSlider,
    form: theme::Form,
    id: &str,
) {
    let (value, step) = match source {
        SettingsSlider::Audio(bus) => (bus.value(AudioSettings::default()), AUDIO_STEP),
        SettingsSlider::Lighting(setting) => {
            let (min, max, step) = lighting_slider_range(setting);
            (
                (lighting_value(setting, &LightingSettings::default()) - min) / (max - min),
                step / (max - min),
            )
        }
    };
    let slider = widgets::controls::slider(parent, label, value, form, id);
    parent
        .commands()
        .entity(slider)
        .insert((source, Slider { value, step }));
}

fn lighting_slider_range(setting: Setting) -> (f32, f32, f32) {
    match setting {
        Setting::Light => (
            MIN_LIGHT_ILLUMINANCE,
            MAX_LIGHT_ILLUMINANCE,
            ILLUMINANCE_STEP,
        ),
        Setting::Ambient => (MIN_AMBIENT_BRIGHTNESS, MAX_AMBIENT_BRIGHTNESS, AMBIENT_STEP),
        Setting::Pitch => (MIN_LIGHT_PITCH_DEG, MAX_LIGHT_PITCH_DEG, ANGLE_STEP_DEG),
        Setting::Yaw => (MIN_LIGHT_YAW_DEG, MAX_LIGHT_YAW_DEG, ANGLE_STEP_DEG),
        _ => unreachable!("only lighting settings use a continuous slider"),
    }
}

fn lighting_value(setting: Setting, lighting: &LightingSettings) -> f32 {
    match setting {
        Setting::Light => lighting.illuminance,
        Setting::Ambient => lighting.ambient_brightness,
        Setting::Pitch => lighting.light_pitch_deg,
        Setting::Yaw => lighting.light_yaw_deg,
        _ => unreachable!("only lighting settings use a continuous slider"),
    }
}

fn apply_settings_sliders(
    mut events: MessageReader<SliderChanged>,
    sliders: Query<&SettingsSlider>,
    menu: Res<PauseMenuState>,
    career: Option<Res<crate::career::CareerClient>>,
    social: Option<Res<crate::social::SocialClient>>,
    mut audio: ResMut<AudioSettings>,
    mut lighting: ResMut<LightingSettings>,
) {
    let allowed = menu.open
        && menu.in_settings
        && !career.as_ref().is_some_and(|career| career.modal_open())
        && !social
            .as_ref()
            .is_some_and(|social| social.blocks_gameplay());
    for event in events.read() {
        if !allowed || !event.value.is_finite() {
            continue;
        }
        let Ok(source) = sliders.get(event.slider) else {
            continue;
        };
        let normalized = event.value.clamp(0.0, 1.0);
        match *source {
            SettingsSlider::Audio(bus) => {
                let delta = normalized - bus.value(*audio);
                bus.adjust(&mut audio, delta);
            }
            SettingsSlider::Lighting(setting) => {
                let (min, max, step) = lighting_slider_range(setting);
                let value =
                    (min + ((normalized * (max - min)) / step).round() * step).clamp(min, max);
                match setting {
                    Setting::Light => lighting.illuminance = value,
                    Setting::Ambient => lighting.ambient_brightness = value,
                    Setting::Pitch => lighting.light_pitch_deg = value,
                    Setting::Yaw => lighting.light_yaw_deg = value,
                    _ => unreachable!(),
                }
            }
        }
    }
}

fn sync_settings_sliders(
    audio: Res<AudioSettings>,
    lighting: Res<LightingSettings>,
    motion: Res<MotionSettings>,
    render: Res<RenderSettings>,
    mut sliders: Query<(&SettingsSlider, &mut Slider, &widgets::KitParts)>,
    mut values: Query<&mut Text>,
    mut toggles: Query<(&UiAction<PauseAction>, &mut widgets::ButtonStyle)>,
) {
    for (source, mut slider, parts) in &mut sliders {
        let (value, label) = match *source {
            SettingsSlider::Audio(bus) => {
                let value = bus.value(audio.sanitized());
                (value, format!("{:.0}%", value * 100.0))
            }
            SettingsSlider::Lighting(setting) => {
                let (min, max, _) = lighting_slider_range(setting);
                let value = lighting_value(setting, &lighting);
                (
                    (value - min) / (max - min),
                    match setting {
                        Setting::Pitch | Setting::Yaw => format!("{value:.0}°"),
                        _ => format!("{value:.0}"),
                    },
                )
            }
        };
        if (slider.value - value).abs() > f32::EPSILON {
            slider.value = value;
        }
        if let Some(entity) = parts.extra[0]
            && let Ok(mut text) = values.get_mut(entity)
            && text.0 != label
        {
            text.0 = label;
        }
    }
    for (action, mut style) in &mut toggles {
        let selected = match action.0 {
            PauseAction::ToggleReduceMotion => motion.reduce,
            PauseAction::ToggleFpsReadout => render.show_fps,
            PauseAction::Audio(AudioButton::Mute) => audio.muted,
            _ => continue,
        };
        widgets::ButtonStyle::set_selected(&mut style, selected);
    }
}

#[allow(clippy::type_complexity)]
fn layout_pause_contents(
    windows: Query<&Window, With<PrimaryWindow>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    menu: Res<PauseMenuState>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut nodes: Query<(
        NodeKey,
        Option<&SettingsRail>,
        Option<&SettingsRailItem>,
        Option<&SettingsTab>,
        Option<&SettingsScrollTrack>,
        &mut Node,
    )>,
    sliders: Query<(Entity, &widgets::KitParts), With<SettingsSlider>>,
    actions: Query<(&UiAction<PauseAction>, &widgets::KitParts)>,
) {
    let size = mobile
        .as_ref()
        .filter(|mobile| mobile.enabled)
        .map(|mobile| mobile.viewport)
        .or_else(|| {
            windows
                .single()
                .ok()
                .map(|window| Vec2::new(window.width(), window.height()))
        })
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let compact = size.y < 540.0 || size.x < 900.0;
    let inset = if compact { 16.0 } else { 28.0 };
    let rail_width = if compact { 148.0 } else { 196.0 };
    let content_left = inset + rail_width + if compact { 16.0 } else { 28.0 };
    let content_top = if compact { 64.0 } else { 100.0 };
    let content_bottom = if compact { 12.0 } else { 76.0 };
    let front_end = screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    for (name, rail, rail_item, group, track, mut node) in &mut nodes {
        if rail.is_some() {
            node.left = Val::Px(inset);
            node.top = Val::Px(content_top);
            node.bottom = Val::Px(inset);
            node.width = Val::Px(rail_width);
            node.row_gap = Val::Px(4.0);
        }
        if rail_item.is_some() {
            node.width = Val::Percent(100.0);
            node.min_height = Val::Px(44.0);
            node.padding = UiRect::horizontal(Val::Px(if compact { 8.0 } else { 16.0 }));
        }
        if group.is_some() {
            node.padding = UiRect::all(Val::Px(if compact { 10.0 } else { 20.0 }));
            node.row_gap = Val::Px(if compact { 4.0 } else { 8.0 });
        }
        if track.is_some() {
            node.top = Val::Px(content_top);
            node.bottom = Val::Px(content_bottom);
            node.right = Val::Px(inset);
        }
        match name.as_str() {
            "PauseMenuPanel" if !menu.in_settings && front_end => {
                node.height = Val::Px(if compact { 300.0 } else { 380.0 });
            }
            "PauseMenuTitle" => {
                node.margin.left = Val::Px(if compact && menu.in_settings {
                    60.0
                } else {
                    0.0
                });
            }
            "PauseMenuHeader" => node.padding.bottom = Val::Px(if compact { 0.0 } else { 8.0 }),
            "PauseMenuAudioTitleGroup" => {
                node.display = if compact {
                    Display::None
                } else {
                    Display::Flex
                }
            }
            "PauseMenuSettingsSection" => {
                node.left = Val::Px(content_left);
                node.top = Val::Px(content_top);
                node.right = Val::Px(inset + 12.0);
                node.bottom = Val::Px(content_bottom);
                node.align_items = AlignItems::Stretch;
                node.row_gap = Val::Px(if compact { 10.0 } else { 16.0 });
            }
            "PauseMenuSettingsFooter" => {
                node.left = Val::Px(if compact { inset } else { content_left });
                node.right = if compact {
                    Val::Auto
                } else {
                    Val::Px(inset + 12.0)
                };
                node.top = if compact { Val::Px(12.0) } else { Val::Auto };
                node.bottom = if compact { Val::Auto } else { Val::Px(16.0) };
                node.width = if compact { Val::Px(44.0) } else { Val::Auto };
                node.justify_content = JustifyContent::FlexEnd;
            }
            "BackButton" => {
                node.width = Val::Px(if compact { 44.0 } else { 180.0 });
                node.max_width = node.width;
                node.min_width = Val::Px(44.0);
                node.height = Val::Px(if compact { 44.0 } else { 46.0 });
                node.padding = UiRect::horizontal(Val::Px(if compact { 0.0 } else { 16.0 }));
            }
            _ => {}
        }
    }
    for (action, parts) in &actions {
        if action.0 == PauseAction::BackFromSettings
            && let Some(label) = parts.label
            && let Ok((_, _, _, _, _, mut node)) = nodes.get_mut(label)
        {
            node.display = if compact {
                Display::None
            } else {
                Display::Flex
            };
        }
    }
    for (entity, parts) in &sliders {
        if let Ok((_, _, _, _, _, mut row)) = nodes.get_mut(entity) {
            row.width = Val::Percent(100.0);
            row.column_gap = Val::Px(if compact { 10.0 } else { 20.0 });
            row.border = UiRect::bottom(Val::Px(1.0));
        }
        for (part, width, grow) in [
            (parts.label, if compact { 106.0 } else { 160.0 }, false),
            (parts.track, 0.0, true),
            (parts.extra[0], 60.0, false),
        ] {
            if let Some(part) = part
                && let Ok((_, _, _, _, _, mut node)) = nodes.get_mut(part)
            {
                node.width = if grow { Val::Auto } else { Val::Px(width) };
                node.max_width = if grow {
                    Val::Percent(100.0)
                } else {
                    Val::Px(width)
                };
                node.min_width = Val::Px(if grow { 64.0 } else { 0.0 });
                node.flex_grow = if grow { 1.0 } else { 0.0 };
                node.flex_basis = if grow { Val::Px(0.0) } else { Val::Auto };
                node.flex_shrink = if grow { 1.0 } else { 0.0 };
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn paint_settings_scrollbar(
    menu: Res<PauseMenuState>,
    bodies: Query<(&ComputedNode, &ScrollPosition), With<SettingsSection>>,
    mut bars: Query<
        (&mut Node, Has<SettingsScrollThumb>),
        Or<(With<SettingsScrollTrack>, With<SettingsScrollThumb>)>,
    >,
) {
    let Ok((computed, scroll)) = bodies.single() else {
        return;
    };
    let max = crate::ui::scroll::max_offset(computed);
    let ratio = (computed.size().y / computed.content_size().y.max(1.0)).clamp(0.08, 1.0);
    for (mut node, thumb) in &mut bars {
        node.display = if menu.open && menu.in_settings && max > 1.0 {
            Display::Flex
        } else {
            Display::None
        };
        if thumb {
            node.height = Val::Percent(ratio * 100.0);
            node.top =
                Val::Percent((scroll.y / max.max(1.0)).clamp(0.0, 1.0) * (1.0 - ratio) * 100.0);
        }
    }
}

fn setting_row(
    parent: &mut ChildSpawnerCommands,
    label: Localized,
    value: String,
    setting: Setting,
    id: &str,
) {
    let id = crate::ui::TestId::from(id);
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(44.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(16.0),
                flex_shrink: 0.0,
                ..default()
            },
            id.clone(),
        ))
        .with_children(|row| {
            row.spawn((
                label.into_text(),
                theme::role_text(crate::ui::tokens::TextRole::Label),
                TextColor(theme::IVORY),
                Node {
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_width: Val::Px(0.0),
                    ..default()
                },
            ));
            row.spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                flex_shrink: 0.0,
                ..default()
            })
            .with_children(|controls| {
                widgets::controls::stepper_button(
                    controls,
                    crate::ui::kit_assets::Icon::NavMinus,
                    PauseAction::Step(setting, -1),
                    id.child("-Down"),
                );
                widgets::value_label(controls, value, SettingLabel(setting), id.child("-Value"));
                widgets::controls::stepper_button(
                    controls,
                    crate::ui::kit_assets::Icon::NavPlus,
                    PauseAction::Step(setting, 1),
                    id.child("-Up"),
                );
            });
        });
}

fn menu_button(
    parent: &mut ChildSpawnerCommands,
    key: &'static str,
    kind: ButtonKind,
    icon: Icon,
    action: PauseAction,
    id: &str,
) -> Entity {
    let mut node = widgets::button_node(widgets::ButtonSize::Regular, kind, theme::Form::Desktop);
    node.width = Val::Percent(100.0);
    node.max_width = Val::Px(400.0);
    widgets::spawn_button(
        parent,
        node,
        Localized::new(key),
        theme::TextStyle::new(TextRole::Button),
        kind,
        Some(icon),
        action,
        id.into(),
        (),
    )
}

fn setup_pause_menu_ui(mut commands: Commands, platform: Option<Res<crate::ui::UiPlatform>>) {
    let form = theme::Form::of(mobile_pause(platform.as_deref()));
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
            BackgroundColor(theme::SCRIM),
            Visibility::Hidden,
            ZIndex(100),
            PauseMenuRoot,
            ModalRoot(ModalId::Pause),
            Name::new("PauseMenuRoot"),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Px(metric::PAUSE_PANEL.0),
                        height: Val::Px(metric::PAUSE_PANEL.1),
                        max_width: Val::Percent(95.0),
                        max_height: Val::Percent(95.0),
                        overflow: Overflow::clip(),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::FlexStart,
                        align_items: AlignItems::Stretch,
                        row_gap: Val::Px(16.0),
                        padding: UiRect::all(Val::Px(24.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(theme::PANEL.with_alpha(1.0)),
                    BorderColor::all(color::GOLD_700),
                    // The front-end Settings painting is inserted lazily as a
                    // sibling after this panel. Keep the controls in an
                    // explicit foreground layer instead of relying on child
                    // insertion order (which made the painting cover the
                    // whole settings UI).
                    ZIndex(1),
                    PauseMenuPanel,
                    Name::new("PauseMenuPanel"),
                ))
                .with_children(|panel| {
                    panel
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(20.0),
                                top: Val::Px(84.0),
                                width: Val::Px(168.0),
                                display: Display::None,
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(8.0),
                                flex_shrink: 0.0,
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            Visibility::Hidden,
                            SettingsRail,
                            ScrollArea::menu(MENU_WHEEL_STEP),
                        ))
                        .with_children(|rail| {
                            for (tab, key, icon, id) in [
                                (
                                    SettingsTab::Sound,
                                    "pause.settings.sound",
                                    Icon::SettingsVolume2,
                                    "SettingsTabSound",
                                ),
                                (
                                    SettingsTab::Graphics,
                                    "pause.settings.graphics",
                                    Icon::SettingsMonitor,
                                    "SettingsTabGraphics",
                                ),
                                (
                                    SettingsTab::Camera,
                                    "pause.settings.camera",
                                    Icon::SettingsCamera,
                                    "SettingsTabCamera",
                                ),
                                (
                                    SettingsTab::Hud,
                                    "pause.settings.hud",
                                    Icon::SettingsMonitor,
                                    "SettingsTabHud",
                                ),
                                (
                                    SettingsTab::Language,
                                    "pause.settings.language",
                                    Icon::SettingsLanguages,
                                    "SettingsTabLanguage",
                                ),
                            ] {
                                let button = widgets::controls::tab(
                                    rail,
                                    Localized::new(key),
                                    Some(icon),
                                    widgets::controls::TabPlacement::Rail,
                                    form,
                                    tab == SettingsTab::Sound,
                                    PauseAction::SettingsTab(tab),
                                    id,
                                );
                                rail.commands().entity(button).insert(SettingsRailItem);
                            }
                            rail.spawn((
                                Node {
                                    height: Val::Px(1.0),
                                    width: Val::Percent(100.0),
                                    margin: UiRect::vertical(Val::Px(8.0)),
                                    flex_shrink: 0.0,
                                    ..default()
                                },
                                BackgroundColor(color::BORDER_SUBTLE),
                            ));
                            let controls = widgets::controls::tab(
                                rail,
                                Localized::new("pause.settings.controls"),
                                Some(Icon::SettingsGamepad2),
                                widgets::controls::TabPlacement::Rail,
                                form,
                                false,
                                PauseAction::Help,
                                "PauseMenuSettingsControlsButton",
                            );
                            rail.commands().entity(controls).insert(SettingsRailItem);
                        });
                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                min_height: Val::Px(metric::BUTTON_H),
                                flex_shrink: 0.0,
                                justify_content: JustifyContent::SpaceBetween,
                                align_items: AlignItems::Center,
                                border: UiRect::bottom(Val::Px(1.0)),
                                padding: UiRect::bottom(Val::Px(8.0)),
                                ..default()
                            },
                            Name::new("PauseMenuHeader"),
                            BorderColor::all(color::BORDER_SUBTLE),
                        ))
                        .with_children(|header| {
                            header.spawn((
                                Localized::new("pause.title").into_text(),
                                theme::role_text(TextRole::Title),
                                TextColor(theme::GOLD),
                                Name::new("PauseMenuTitle"),
                            ));
                            widgets::controls::sized_icon_button(
                                header,
                                Icon::NavX,
                                theme::Form::Phone,
                                ButtonKind::Secondary,
                                PauseAction::Close,
                                "PauseMenuCloseButton",
                            );
                        });

                    panel
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(12.0),
                                display: Display::Flex,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::FlexStart,
                                flex_shrink: 1.0,
                                flex_grow: 1.0,
                                flex_basis: Val::Px(0.0),
                                min_height: Val::Px(0.0),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            ScrollArea::menu(MENU_WHEEL_STEP),
                            MainMenuSection,
                            Name::new("PauseMenuMainSection"),
                        ))
                        .with_children(|main| {
                            main.spawn((
                                Text::new(tr("pause.hint.online")),
                                theme::role_text(TextRole::Caption),
                                TextColor(theme::MUTED),
                                Name::new("PauseMenuMainTitle"),
                            ));
                            menu_button(
                                main,
                                "pause.button.settings",
                                ButtonKind::Secondary,
                                Icon::NavSettings,
                                PauseAction::OpenSettings,
                                "SettingsButton",
                            );
                            crate::debug::tools_page::spawn_practice_open_button(main);
                            menu_button(
                                main,
                                "pause.button.help",
                                ButtonKind::Secondary,
                                Icon::SettingsGamepad2,
                                PauseAction::Help,
                                "PauseMenuHelpButton",
                            );
                            menu_button(
                                main,
                                "pause.button.exit",
                                ButtonKind::Danger,
                                Icon::NavLogOut,
                                PauseAction::Exit,
                                "PauseMenuExitButton",
                            );
                            menu_button(
                                main,
                                "pause.button.leave_match",
                                ButtonKind::Danger,
                                Icon::NavLogOut,
                                PauseAction::LeaveMatch,
                                "PauseMenuLeaveMatchButton",
                            );
                            menu_button(
                                main,
                                "pause.button.leave_practice",
                                ButtonKind::Secondary,
                                Icon::NavLogOut,
                                PauseAction::LeavePractice,
                                "PauseMenuLeavePracticeButton",
                            );
                        });

                    panel
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(8.0),
                                position_type: PositionType::Absolute,
                                left: Val::Px(204.0),
                                right: Val::Px(20.0),
                                top: Val::Px(84.0),
                                bottom: Val::Px(72.0),
                                display: Display::None,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::FlexStart,
                                flex_shrink: 1.0,
                                flex_grow: 1.0,
                                flex_basis: Val::Px(0.0),
                                min_height: Val::Px(0.0),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            Visibility::Hidden,
                            ScrollArea::menu(MENU_WHEEL_STEP),
                            SettingsSection,
                            Name::new("PauseMenuSettingsSection"),
                        ))
                        .with_children(|settings| {
                            settings_group(settings, SettingsTab::Language, |settings| {
                                section_title(
                                    settings,
                                    "pause.settings.language",
                                    "PauseMenuLanguageTitle",
                                );
                                widgets::toggle_row(
                                    settings,
                                    Localized::new("pause.settings.language"),
                                    crate::i18n::active().native_name(),
                                    LanguageValue,
                                    PauseAction::CycleLanguage,
                                    "PauseMenuLanguage",
                                );
                            });
                            settings_group(settings, SettingsTab::Sound, |settings| {
                                section_title(
                                    settings,
                                    "pause.settings.sound",
                                    "PauseMenuAudioTitle",
                                );
                                for (bus, key, name) in [
                                    (
                                        AudioBus::Master,
                                        "pause.audio.master",
                                        "PauseMenuAudioMasterControls",
                                    ),
                                    (
                                        AudioBus::Music,
                                        "pause.audio.music",
                                        "PauseMenuAudioMusicControls",
                                    ),
                                    (
                                        AudioBus::Effects,
                                        "pause.audio.effects",
                                        "PauseMenuAudioEffectsControls",
                                    ),
                                    (AudioBus::Ui, "pause.audio.ui", "PauseMenuAudioUiControls"),
                                ] {
                                    settings_slider(
                                        settings,
                                        Localized::new(key),
                                        SettingsSlider::Audio(bus),
                                        form,
                                        name,
                                    );
                                }
                                widgets::controls::toggle(
                                    settings,
                                    Localized::new("pause.audio.mute"),
                                    false,
                                    PauseAction::Audio(AudioButton::Mute),
                                    "PauseMenuAudioMuteButton",
                                );
                            });
                            settings_group(settings, SettingsTab::Graphics, |settings| {
                                section_title(
                                    settings,
                                    "pause.settings.motion",
                                    "PauseMenuMotionTitle",
                                );
                                let motion_row = widgets::controls::toggle(
                                    settings,
                                    Localized::new("pause.motion.reduce"),
                                    false,
                                    PauseAction::ToggleReduceMotion,
                                    "PauseMenuReduceMotionButton",
                                );
                                settings.commands().entity(motion_row).insert(Node {
                                    width: Val::Percent(100.0),
                                    min_height: Val::Px(46.0),
                                    padding: UiRect::horizontal(Val::Px(16.0)),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::SpaceBetween,
                                    flex_shrink: 0.0,
                                    ..default()
                                });
                                settings.spawn((
                                    Localized::new("pause.motion.reduce_hint").into_text(),
                                    crate::ui::theme::role_text(
                                        crate::ui::tokens::TextRole::Caption,
                                    ),
                                    TextColor(theme::MUTED),
                                    Node {
                                        width: Val::Percent(100.0),
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                    Name::new("PauseMenuReduceMotionHint"),
                                ));
                                settings
                                    .spawn((
                                        Node {
                                            display: Display::None,
                                            width: Val::Percent(100.0),
                                            flex_direction: FlexDirection::Column,
                                            row_gap: Val::Px(8.0),
                                            margin: UiRect::top(Val::Px(12.0)),
                                            ..default()
                                        },
                                        crate::phone_layout_preview::PhonePreviewSettings,
                                        Name::new("PhonePreviewSettings"),
                                    ))
                                    .with_children(|preview| {
                                        widgets::controls::toggle(
                                            preview,
                                            Localized::new("pause.preview.iphone16"),
                                            false,
                                            PauseAction::TogglePhoneLayoutPreview,
                                            "PhonePreviewToggle",
                                        );
                                        preview.spawn((
                                            Localized::new("pause.preview.hint").into_text(),
                                            theme::role_text(TextRole::Caption),
                                            TextColor(theme::MUTED),
                                        ));
                                    });
                            });
                            settings_group(settings, SettingsTab::Language, |settings| {
                                settings.spawn((
                                    Text::new(""),
                                    theme::text(14.0),
                                    TextColor(theme::MUTED),
                                    // Text in a scrolling column keeps its height.
                                    Node {
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                    SettingsServerAddrLabel,
                                    Name::new("PauseMenuServerAddrHint"),
                                ));
                                // Third-party art credits (game-icons.net CC BY 3.0
                                // requires an in-game line).
                                settings.spawn((
                                    Localized::new("pause.credits.icons").into_text(),
                                    crate::ui::theme::role_text(
                                        crate::ui::tokens::TextRole::Caption,
                                    ),
                                    TextColor(theme::MUTED),
                                    Node {
                                        max_width: Val::Px(metric::MENU_W),
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                    Name::new("PauseMenuCredits"),
                                ));
                            });
                            settings_group(settings, SettingsTab::Graphics, |settings| {
                                section_title(
                                    settings,
                                    "pause.settings.render",
                                    "PauseMenuRenderTitle",
                                );
                                setting_row(
                                    settings,
                                    Localized::new("pause.render.limit"),
                                    "60".to_owned(),
                                    Setting::RenderFps,
                                    "PauseMenuRenderFpsControls",
                                );
                                widgets::controls::toggle(
                                    settings,
                                    Localized::new("pause.render.show_fps"),
                                    true,
                                    PauseAction::ToggleFpsReadout,
                                    "PauseMenuShowFpsToggle",
                                );
                                settings.spawn((
                                    Localized::new("pause.render.hint").into_text(),
                                    theme::role_text(TextRole::Caption),
                                    TextColor(theme::MUTED),
                                ));
                            });
                            settings_group(settings, SettingsTab::Hud, |settings| {
                                section_title(settings, "pause.settings.hud", "PauseMenuHudTitle");
                                settings.spawn((
                                    Localized::new("pause.hud.hint").into_text(),
                                    theme::role_text(TextRole::Caption),
                                    TextColor(theme::MUTED),
                                ));
                                for (key, setting, id) in [
                                    (
                                        "pause.hud.joystick_x",
                                        Setting::JoystickX,
                                        "PauseMenuHudJoystickX",
                                    ),
                                    (
                                        "pause.hud.joystick_y",
                                        Setting::JoystickY,
                                        "PauseMenuHudJoystickY",
                                    ),
                                    (
                                        "pause.hud.combat_x",
                                        Setting::CombatX,
                                        "PauseMenuHudCombatX",
                                    ),
                                    (
                                        "pause.hud.combat_y",
                                        Setting::CombatY,
                                        "PauseMenuHudCombatY",
                                    ),
                                ] {
                                    setting_row(
                                        settings,
                                        Localized::new(key),
                                        "0".to_owned(),
                                        setting,
                                        id,
                                    );
                                }
                                widgets::button(
                                    settings,
                                    Localized::new("pause.hud.reset"),
                                    ButtonKind::Secondary,
                                    PauseAction::ResetHud,
                                    "PauseMenuResetHudButton",
                                );
                            });
                            settings_group(settings, SettingsTab::Graphics, |settings| {
                                section_title(
                                    settings,
                                    "pause.settings.lighting",
                                    "PauseMenuLightingTitle",
                                );
                                settings_slider(
                                    settings,
                                    Localized::new("pause.light.main"),
                                    SettingsSlider::Lighting(Setting::Light),
                                    form,
                                    "PauseMenuMainLightControls",
                                );
                                settings_slider(
                                    settings,
                                    Localized::new("pause.light.ambient"),
                                    SettingsSlider::Lighting(Setting::Ambient),
                                    form,
                                    "PauseMenuAmbientControls",
                                );
                                settings_slider(
                                    settings,
                                    Localized::new("pause.light.pitch"),
                                    SettingsSlider::Lighting(Setting::Pitch),
                                    form,
                                    "PauseMenuPitchControls",
                                );
                                settings_slider(
                                    settings,
                                    Localized::new("pause.light.yaw"),
                                    SettingsSlider::Lighting(Setting::Yaw),
                                    form,
                                    "PauseMenuYawControls",
                                );
                            });
                            settings_group(settings, SettingsTab::Camera, |settings| {
                                section_title(
                                    settings,
                                    "pause.settings.camera",
                                    "PauseMenuCameraTitle",
                                );
                                // 100% is the default follow view; lower values bring
                                // the camera closer so hero silhouettes read larger.
                                setting_row(
                                    settings,
                                    Localized::new("pause.camera.distance"),
                                    CameraSettings::default().percent_label(),
                                    Setting::CameraZoom,
                                    "PauseMenuCameraZoomControls",
                                );
                            });
                            settings_group(settings, SettingsTab::Graphics, |settings| {
                                section_title(
                                    settings,
                                    "pause.settings.model",
                                    "PauseMenuModelTitle",
                                );
                                setting_row(
                                    settings,
                                    Localized::new("pause.model.scale"),
                                    format!("{:.2}", DEFAULT_MODEL_TARGET_HEIGHT),
                                    Setting::ModelScale,
                                    "PauseMenuScaleControls",
                                );

                                let reset = widgets::button(
                                    settings,
                                    Localized::new("pause.button.reset_graphics"),
                                    ButtonKind::Secondary,
                                    PauseAction::ResetGraphics,
                                    "PauseMenuResetGraphicsButton",
                                );
                                settings.commands().entity(reset).insert(Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(46.0),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    flex_shrink: 0.0,
                                    ..default()
                                });
                            });
                        });
                    panel
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                width: Val::Px(4.0),
                                border_radius: BorderRadius::all(Val::Px(2.0)),
                                display: Display::None,
                                ..default()
                            },
                            SettingsScrollTrack,
                            BackgroundColor(color::SURFACE_3),
                            Pickable::IGNORE,
                        ))
                        .with_children(|track| {
                            track.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    width: Val::Percent(100.0),
                                    border_radius: BorderRadius::all(Val::Px(2.0)),
                                    ..default()
                                },
                                SettingsScrollThumb,
                                BackgroundColor(color::GOLD_500),
                                Pickable::IGNORE,
                            ));
                        });
                    panel
                        .spawn((
                            Node {
                                flex_shrink: 0.0,
                                min_height: Val::Px(metric::BUTTON_H),
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            MainMenuFooter,
                            Name::new("PauseMenuMainFooter"),
                        ))
                        .with_children(|footer| {
                            menu_button(
                                footer,
                                "pause.button.resume",
                                ButtonKind::Primary,
                                Icon::NavPlay,
                                PauseAction::Resume,
                                "PauseMenuResumeButton",
                            );
                        });
                    panel
                        .spawn((
                            Node {
                                display: Display::None,
                                flex_shrink: 0.0,
                                min_height: Val::Px(metric::BUTTON_H),
                                justify_content: JustifyContent::Center,
                                position_type: PositionType::Absolute,
                                left: Val::Px(204.0),
                                right: Val::Px(20.0),
                                bottom: Val::Px(16.0),
                                ..default()
                            },
                            Visibility::Hidden,
                            SettingsFooter,
                            Name::new("PauseMenuSettingsFooter"),
                        ))
                        .with_children(|footer| {
                            menu_button(
                                footer,
                                "common.back",
                                ButtonKind::Secondary,
                                Icon::NavChevronLeft,
                                PauseAction::BackFromSettings,
                                "BackButton",
                            );
                        });
                    crate::debug::tools_page::spawn_practice_section(panel);
                });
        });
}

/// Match-assuming UI must not stay open without an active session (TASK-14 P5 / AC6).
fn close_pause_menu_when_disconnected(
    client_session: Res<ClientSession>,
    mut menu_state: ResMut<PauseMenuState>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
) {
    // Shell settings must remain usable before a server connects. A lost
    // gameplay connection still closes the in-match menu as before.
    if client_session.state() != ClientConnectionState::Disconnected
        || screen.as_ref().is_some_and(|screen| screen.get().is_menu())
    {
        return;
    }
    if menu_state.open {
        menu_state.open = false;
        menu_state.in_settings = false;
    }
}

/// Last reader of the back chain: `Esc` toggles the menu. A back press from
/// another source (a gamepad's East) only closes it; opening it from a pad
/// is the Start button, which goes through the `≡` button's path.
pub(crate) fn toggle_pause_menu(
    back: crate::ui::BackInput,
    social: Option<Res<crate::social::SocialClient>>,
    mut menu_state: ResMut<PauseMenuState>,
) {
    if social
        .as_ref()
        .is_some_and(|social| social.blocks_gameplay())
    {
        return;
    }
    if back.just_pressed() && (menu_state.open || back.pressed_on_keyboard()) {
        if menu_state.open {
            menu_state.open = false;
            menu_state.in_settings = false;
        } else {
            menu_state.open = true;
            menu_state.in_settings = false;
        }
        info!(
            "Pause menu {}",
            if menu_state.open { "opened" } else { "closed" }
        );
    }
}

/// Opening, closing or changing page drops the tap held across it, so a
/// release on the new page never activates a button of the old one.
fn bump_gesture_epoch_on_navigation(
    menu: Res<PauseMenuState>,
    practice: Option<Res<crate::debug::tools_page::PracticeSandboxState>>,
    mut epoch: ResMut<GestureEpoch>,
    mut previous: Local<Option<(bool, bool, bool)>>,
) {
    let current = (
        menu.open,
        menu.in_settings,
        practice.as_ref().is_some_and(|practice| practice.open),
    );
    if *previous != Some(current) {
        if previous.is_some() {
            epoch.bump();
        }
        *previous = Some(current);
    }
}

fn sync_pause_menu_visibility(
    menu_state: Res<PauseMenuState>,
    mut query: Query<(&mut Visibility, &mut Node), With<PauseMenuRoot>>,
) {
    if !menu_state.is_changed() {
        return;
    }

    if let Ok((mut visibility, mut node)) = query.single_mut() {
        *visibility = if menu_state.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        node.display = if menu_state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn sync_pause_menu_sections(
    menu_state: Res<PauseMenuState>,
    practice: Option<Res<crate::debug::tools_page::PracticeSandboxState>>,
    mut section_queries: ParamSet<(
        Query<(&mut Visibility, &mut Node), Or<(With<MainMenuSection>, With<MainMenuFooter>)>>,
        Query<
            (&mut Visibility, &mut Node),
            Or<(
                With<SettingsSection>,
                With<SettingsFooter>,
                With<SettingsRail>,
            )>,
        >,
    )>,
) {
    let practice_open = practice.as_ref().is_some_and(|p| p.open);
    if !menu_state.is_changed() && !practice.as_ref().is_some_and(|p| p.is_changed()) {
        return;
    }

    // The main page yields to whichever sub-page is open.
    let main_hidden = menu_state.in_settings || practice_open;
    for (mut main_visibility, mut main_node) in &mut section_queries.p0() {
        *main_visibility = if main_hidden {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        main_node.display = if main_hidden {
            Display::None
        } else {
            Display::Flex
        };
    }

    for (mut settings_visibility, mut settings_node) in &mut section_queries.p1() {
        *settings_visibility = if menu_state.in_settings {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        settings_node.display = if menu_state.in_settings {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Page navigation: resume/close, settings in and out, the controls guide.
/// The guide closes the menu; opened from Settings it remembers to come back
/// (R6.5, [`return_to_settings_after_help`]).
fn apply_pause_navigation(
    mut activated: MessageReader<Activated<PauseAction>>,
    mut menu: ResMut<PauseMenuState>,
    mut help: ResMut<crate::help_overlay::HelpOverlayVisible>,
    help_return: Option<ResMut<SettingsHelpReturn>>,
) {
    let mut help_return = help_return;
    for Activated { action, .. } in activated.read() {
        match action {
            PauseAction::Resume | PauseAction::Close => {
                menu.open = false;
                menu.in_settings = false;
            }
            PauseAction::OpenSettings => menu.in_settings = true,
            PauseAction::BackFromSettings => menu.in_settings = false,
            PauseAction::Help if menu.open => {
                if let Some(help_return) = help_return.as_mut() {
                    help_return.0 = menu.in_settings;
                }
                menu.open = false;
                menu.in_settings = false;
                help.0 = true;
            }
            _ => {}
        }
    }
}

/// R6.5: the guide opened from Settings closes back into Settings, on the
/// page the player left (the Esc/B that closed the guide was consumed by it,
/// so the menu stays open). From the Game menu it returns to the game.
fn return_to_settings_after_help(
    help: Res<crate::help_overlay::HelpOverlayVisible>,
    mut help_return: ResMut<SettingsHelpReturn>,
    mut menu: ResMut<PauseMenuState>,
) {
    if help_return.0 && !help.0 {
        help_return.0 = false;
        menu.open = true;
        menu.in_settings = true;
    }
}

/// Graphics steppers and the reset button.
fn apply_pause_settings(
    mut activated: MessageReader<Activated<PauseAction>>,
    mut lighting: ResMut<LightingSettings>,
    mut model: ResMut<ModelScaleSettings>,
    mut camera: ResMut<CameraSettings>,
    mut motion: ResMut<MotionSettings>,
    mut prefs_gate: ResMut<ClientPrefsSaveGate>,
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    client_session_id: Res<ClientSessionId>,
    team: Res<TeamSelection>,
    audio: Res<AudioSettings>,
    locale: Option<Res<Locale>>,
    saved_language: Option<Res<crate::persistence::SavedLanguage>>,
) {
    for Activated { action, .. } in activated.read() {
        match *action {
            PauseAction::Step(setting, direction) => {
                let sign = f32::from(direction.signum());
                match setting {
                    Setting::CameraZoom => camera.adjust(sign * CAMERA_ZOOM_STEP),
                    Setting::RenderFps
                    | Setting::JoystickX
                    | Setting::JoystickY
                    | Setting::CombatX
                    | Setting::CombatY => {}
                    Setting::ModelScale => {
                        model.target_height = (model.target_height + sign * SCALE_STEP)
                            .clamp(MIN_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT);
                    }
                    Setting::Light => {
                        lighting.illuminance = (lighting.illuminance + sign * ILLUMINANCE_STEP)
                            .clamp(MIN_LIGHT_ILLUMINANCE, MAX_LIGHT_ILLUMINANCE);
                    }
                    Setting::Ambient => {
                        lighting.ambient_brightness = (lighting.ambient_brightness
                            + sign * AMBIENT_STEP)
                            .clamp(MIN_AMBIENT_BRIGHTNESS, MAX_AMBIENT_BRIGHTNESS);
                    }
                    Setting::Pitch => {
                        lighting.light_pitch_deg = (lighting.light_pitch_deg
                            + sign * ANGLE_STEP_DEG)
                            .clamp(MIN_LIGHT_PITCH_DEG, MAX_LIGHT_PITCH_DEG);
                    }
                    Setting::Yaw => {
                        lighting.light_yaw_deg = (lighting.light_yaw_deg + sign * ANGLE_STEP_DEG)
                            .clamp(MIN_LIGHT_YAW_DEG, MAX_LIGHT_YAW_DEG);
                    }
                }
            }
            PauseAction::ResetGraphics => {
                let addr = server_addr_for_prefs(&resolved_addr);
                reset_graphics_to_defaults(
                    lighting.as_mut(),
                    model.as_mut(),
                    camera.as_mut(),
                    prefs_gate.as_mut(),
                    team.character,
                    addr,
                    client_session_id.0.as_str(),
                    audio.as_ref(),
                    motion.as_ref(),
                    crate::persistence::language_to_save(
                        locale.as_deref(),
                        saved_language.as_deref().unwrap_or(&Default::default()),
                    ),
                );
            }
            PauseAction::ToggleReduceMotion => motion.reduce = !motion.reduce,
            _ => {}
        }
    }
}

/// Group offsets preserve the four skill buttons' relative arrangement. The
/// regular settings modal owns all touches while editing, so they cannot move
/// the hero. The live layout applies its final safe-area clamp independently.
fn apply_render_and_hud_settings(
    mut activated: MessageReader<Activated<PauseAction>>,
    menu: Res<PauseMenuState>,
    career: Option<Res<crate::career::CareerClient>>,
    social: Option<Res<crate::social::SocialClient>>,
    mut render: ResMut<RenderSettings>,
    mut hud: ResMut<HudPositionSettings>,
) {
    let allowed = menu.open
        && menu.in_settings
        && !career.as_ref().is_some_and(|career| career.modal_open())
        && !social
            .as_ref()
            .is_some_and(|social| social.blocks_gameplay());
    for Activated { action, .. } in activated.read() {
        if !allowed {
            continue;
        }
        match *action {
            PauseAction::Step(Setting::RenderFps, direction) => {
                render.fps_limit = if direction > 0 { 120 } else { 60 };
            }
            PauseAction::ToggleFpsReadout => render.show_fps = !render.show_fps,
            PauseAction::Step(
                setting @ (Setting::JoystickX
                | Setting::JoystickY
                | Setting::CombatX
                | Setting::CombatY),
                direction,
            ) => {
                *hud = hud.sanitized();
                let delta = f32::from(direction.signum()) * 8.0;
                match setting {
                    Setting::JoystickX => hud.joystick_offset.x += delta,
                    Setting::JoystickY => hud.joystick_offset.y += delta,
                    Setting::CombatX => hud.combat_offset.x += delta,
                    Setting::CombatY => hud.combat_offset.y += delta,
                    _ => unreachable!(),
                }
                *hud = hud.sanitized();
            }
            PauseAction::ResetHud => *hud = HudPositionSettings::default(),
            _ => {}
        }
    }
}

/// Sound levels and mute; only while the settings page is the front-most modal.
fn apply_pause_audio(
    mut activated: MessageReader<Activated<PauseAction>>,
    menu: Res<PauseMenuState>,
    career: Option<Res<crate::career::CareerClient>>,
    social: Option<Res<crate::social::SocialClient>>,
    mut settings: ResMut<AudioSettings>,
) {
    let allowed = menu.open
        && menu.in_settings
        && !career.as_ref().is_some_and(|career| career.modal_open())
        && !social
            .as_ref()
            .is_some_and(|social| social.blocks_gameplay());
    for Activated { action, .. } in activated.read() {
        let PauseAction::Audio(AudioButton::Mute) = action else {
            continue;
        };
        if !allowed {
            continue;
        }
        settings.muted = !settings.muted;
    }
}

fn mobile_pause(platform: Option<&crate::ui::UiPlatform>) -> bool {
    platform.map_or_else(
        || crate::platform::ui_profile() == crate::platform::UiProfile::Mobile,
        crate::ui::UiPlatform::is_mobile,
    )
}

/// Leave a session on mobile; application termination is desktop-only.
fn apply_pause_session(
    platform: Option<Res<crate::ui::UiPlatform>>,
    mut activated: MessageReader<Activated<PauseAction>>,
    mut commands: Commands,
    session: Res<ClientSession>,
    mut menu: ResMut<PauseMenuState>,
    mut session_commands: MessageWriter<crate::net::SessionUiCommand>,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    window_query: Query<Entity, With<PrimaryWindow>>,
    mut app_exit_writer: MessageWriter<AppExit>,
) {
    for Activated { action, .. } in activated.read() {
        match action {
            PauseAction::Exit if !mobile_pause(platform.as_deref()) => {
                info!("Exit selected from pause menu.");
                if let Ok(mut cursor) = cursor_query.single_mut() {
                    cursor.grab_mode = CursorGrabMode::None;
                    cursor.visible = true;
                }
                if let Ok(primary_window) = window_query.single() {
                    commands.entity(primary_window).despawn();
                }
                app_exit_writer.write(AppExit::Success);
            }
            PauseAction::LeaveMatch
                if mobile_pause(platform.as_deref())
                    && !session.is_offline()
                    && session.has_committed_join() =>
            {
                session_commands.write(crate::net::SessionUiCommand::LeaveMatch);
                menu.open = false;
                menu.in_settings = false;
            }
            PauseAction::LeavePractice if session.is_offline() => {
                session_commands.write(crate::net::SessionUiCommand::LeaveMatch);
                menu.open = false;
                menu.in_settings = false;
            }
            _ => {}
        }
    }
}

/// The Language row cycles through the shipped languages. `Locale::set`
/// switches the process-wide language at once; `Localized` labels, the
/// per-frame writers and the saved preference follow the resource change.
fn apply_pause_language(
    mut activated: MessageReader<Activated<PauseAction>>,
    locale: Option<ResMut<Locale>>,
) {
    let mut cycles = 0;
    for Activated { action, .. } in activated.read() {
        if *action == PauseAction::CycleLanguage {
            cycles += 1;
        }
    }
    let Some(mut locale) = locale else {
        return;
    };
    if cycles == 0 {
        return;
    }
    let mut next = locale.id();
    for _ in 0..cycles {
        next = next.next();
    }
    if next != locale.id() {
        info!("Language: {} ({})", next.native_name(), next.code());
        locale.set(next);
    }
}

/// The Language row shows the active language in its own script.
fn update_language_value(
    locale: Option<Res<Locale>>,
    mut values: Query<&mut Text, With<LanguageValue>>,
) {
    let Some(locale) = locale else {
        return;
    };
    if !locale.is_changed() {
        return;
    }
    let name = locale.id().native_name();
    for mut value in &mut values {
        if value.0 != name {
            value.0 = name.to_owned();
        }
    }
}

fn reset_pause_scroll_on_navigation(
    menu: Res<PauseMenuState>,
    mut previous: Local<Option<(bool, bool)>>,
    mut panels: Query<&mut ScrollPosition, Or<(With<SettingsSection>, With<MainMenuSection>)>>,
) {
    let current = (menu.open, menu.in_settings);
    if *previous != Some(current) {
        for mut scroll in &mut panels {
            scroll.y = 0.0;
        }
        *previous = Some(current);
    }
}

/// Value labels follow their settings, including changes made elsewhere
/// (wheel zoom writes the camera setting back).
fn update_setting_labels(
    render: Option<Res<RenderSettings>>,
    hud: Option<Res<HudPositionSettings>>,
    camera: Res<CameraSettings>,
    model: Res<ModelScaleSettings>,
    lighting: Res<LightingSettings>,
    mut labels: Query<(&SettingLabel, &mut Text)>,
) {
    for (label, mut text) in &mut labels {
        let next = match label.0 {
            Setting::RenderFps => render
                .as_ref()
                .map(|s| s.sanitized().fps_limit)
                .unwrap_or(60)
                .to_string(),
            setting @ (Setting::JoystickX
            | Setting::JoystickY
            | Setting::CombatX
            | Setting::CombatY) => {
                let hud = hud.as_ref().map(|s| s.sanitized()).unwrap_or_default();
                let value = match setting {
                    Setting::JoystickX => hud.joystick_offset.x,
                    Setting::JoystickY => hud.joystick_offset.y,
                    Setting::CombatX => hud.combat_offset.x,
                    Setting::CombatY => hud.combat_offset.y,
                    _ => unreachable!(),
                };
                format!("{value:+.0}")
            }
            Setting::CameraZoom if camera.is_changed() => camera.percent_label(),
            Setting::ModelScale if model.is_changed() => format!("{:.2}", model.target_height),
            Setting::Light if lighting.is_changed() => format!("{:.0}", lighting.illuminance),
            Setting::Ambient if lighting.is_changed() => {
                format!("{:.0}", lighting.ambient_brightness)
            }
            Setting::Pitch if lighting.is_changed() => {
                format!("{:.0}°", lighting.light_pitch_deg)
            }
            Setting::Yaw if lighting.is_changed() => format!("{:.0}°", lighting.light_yaw_deg),
            _ => continue,
        };
        text.0 = next;
    }
}

fn server_addr_for_prefs(resolved: &ResolvedServerAddressForPrefs) -> &str {
    let s = resolved.0.as_str().trim();
    if s.is_empty() {
        DEFAULT_GAME_SERVER_ADDR
    } else {
        s
    }
}

fn sync_settings_server_addr_label(
    session: Option<Res<crate::net::ClientSession>>,
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    mut label_q: Query<&mut Text, With<SettingsServerAddrLabel>>,
) {
    let addr = server_addr_for_prefs(&resolved_addr);
    let mut next = trf("pause.settings.server_hint", &[("addr", &addr)]);
    if let Some(session) = session {
        next.push('\n');
        next.push_str(&session.compatibility_summary());
        if let Some(detail) = crate::net::link_status(&session)
            .detail()
            .filter(|_| session.compatibility_issue.is_some())
        {
            next.push('\n');
            next.push_str(&detail);
        }
    }
    if let Ok(mut text) = label_q.single_mut() {
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// Mobile platforms leave sessions without terminating the application.
#[allow(clippy::type_complexity)]
fn sync_practice_actions(
    platform: Option<Res<crate::ui::UiPlatform>>,
    session: Res<ClientSession>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut buttons: Query<(
        &mut Node,
        &UiAction<PauseAction>,
        Option<&widgets::KitParts>,
    )>,
    mut hints: Query<(&Name, &mut Text)>,
    mut labels: Query<&mut Localized>,
) {
    let offline = session.is_offline();
    let front_end = screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    for (mut node, action, parts) in &mut buttons {
        if action.0 == PauseAction::Resume
            && let Some(label) = parts.and_then(|parts| parts.label)
            && let Ok(mut label) = labels.get_mut(label)
        {
            let key = if front_end {
                "common.back"
            } else {
                "pause.button.resume"
            };
            if label.key != key {
                label.key = key;
            }
        }
        let visible = match action.0 {
            PauseAction::Exit => !offline && !mobile_pause(platform.as_deref()),
            PauseAction::LeaveMatch => {
                !offline && session.has_committed_join() && mobile_pause(platform.as_deref())
            }
            PauseAction::LeavePractice => offline,
            _ => continue,
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    for (name, mut text) in &mut hints {
        if name.as_str() == "PauseMenuMainTitle" {
            let label = if front_end {
                ""
            } else {
                tr(if offline {
                    "pause.hint.offline"
                } else {
                    "pause.hint.online"
                })
            };
            if text.0 != label {
                text.0 = label.into();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        ModalAppExt, Pressable, action::dispatch_actions, gesture::recognize_presses,
        scroll::scroll_areas, test_id::harness,
    };
    use bevy::input::{
        mouse::{MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    };

    // Real Bevy/Taffy layout, font measurement and clipping; no fabricated
    // ComputedNode rectangles. GPU/window event loop are not required.
    fn layout_app(size: Vec2, dpi: f32, mobile_enabled: bool) -> (App, Entity) {
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
        app.init_resource::<Assets<bevy::mesh::Mesh>>()
            .init_resource::<Assets<TextureAtlasLayout>>()
            .init_resource::<ClientSession>()
            .insert_resource(crate::ui::UiPlatform(if mobile_enabled {
                crate::platform::UiProfile::Mobile
            } else {
                crate::platform::UiProfile::Desktop
            }))
            .init_resource::<GestureEpoch>()
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: true,
            })
            .init_resource::<AudioSettings>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
            .add_message::<Activated<PauseAction>>()
            .add_message::<crate::ui::SyntheticPress>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    bump_gesture_epoch_on_navigation,
                    recognize_presses,
                    scroll_areas,
                    dispatch_actions::<PauseAction>,
                    apply_pause_navigation,
                    apply_pause_audio,
                    sync_pause_menu_visibility,
                    sync_pause_menu_sections,
                    reset_pause_scroll_on_navigation,
                    sync_practice_actions,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                (size_desktop_pause_panel, layout_pause_contents)
                    .chain()
                    .before(bevy::ui::UiSystems::Layout),
            );
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.enabled = mobile_enabled;
        mobile.focused = true;
        mobile.landscape = true;
        mobile.viewport = size;
        app.insert_resource(mobile);
        crate::mobile_ui::add_pause_layout_test_systems(&mut app);
        let mut window = Window::default();
        window.resolution.set_scale_factor_override(Some(dpi));
        window.resolution.set(size.x, size.y);
        let window = app.world_mut().spawn((window, PrimaryWindow)).id();
        app.world_mut().spawn((
            Camera2d,
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: (size * dpi).as_uvec2(),
                        scale_factor: dpi,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        app.finish();
        app.cleanup();
        for _ in 0..5 {
            app.update();
        }
        (app, window)
    }

    fn named(app: &mut App, name: &str) -> Entity {
        app.world_mut()
            .query::<(Entity, crate::ui::test_id::NodeKey)>()
            .iter(app.world())
            .find(|(_, n)| n.as_str() == name)
            .unwrap()
            .0
    }

    fn rect(app: &App, entity: Entity, dpi: f32) -> Rect {
        crate::ui::gesture::logical_ui_rect(
            app.world().get::<ComputedNode>(entity).unwrap(),
            app.world().get::<UiGlobalTransform>(entity).unwrap(),
            app.world().get::<bevy::ui::CalculatedClip>(entity),
            dpi,
        )
    }

    #[test]
    fn tablet_graphics_rows_share_label_slider_and_value_columns() {
        let (mut app, _) = layout_app(Vec2::new(1180.0, 820.0), 2.0, true);
        app.insert_resource(SettingsTab::Graphics)
            .add_systems(Update, settings_tabs);
        app.update();
        app.update();
        let ids = [
            "PauseMenuMainLightControls",
            "PauseMenuAmbientControls",
            "PauseMenuPitchControls",
            "PauseMenuYawControls",
        ];
        let mut label_x: Option<f32> = None;
        let mut button_x: Option<f32> = None;
        for id in ids {
            let row = named(&mut app, id);
            let label = app.world().get::<Children>(row).unwrap()[0];
            let label_rect = rect(&app, label, 2.0);
            let parts = *app.world().get::<widgets::KitParts>(row).unwrap();
            let track_rect = rect(&app, parts.track.unwrap(), 2.0);
            if let Some(x) = label_x {
                assert!((label_rect.min.x - x).abs() < 1.0);
            }
            if let Some(x) = button_x {
                assert!((track_rect.min.x - x).abs() < 1.0);
            }
            assert!(label_rect.max.x <= track_rect.min.x);
            assert!(track_rect.width() >= 160.0);
            label_x = Some(label_rect.min.x);
            button_x = Some(track_rect.min.x);
        }
    }

    #[test]
    fn mobile_hides_exit_and_leaves_online_session_without_app_exit() {
        for mobile in [false, true] {
            let mut app = App::new();
            app.insert_resource(crate::ui::UiPlatform(if mobile {
                crate::platform::UiProfile::Mobile
            } else {
                crate::platform::UiProfile::Desktop
            }))
            .insert_resource(ClientSession::admitted_for_test())
            .init_resource::<PauseMenuState>()
            .add_message::<Activated<PauseAction>>()
            .add_message::<crate::net::SessionUiCommand>()
            .add_message::<AppExit>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(Update, (apply_pause_session, sync_practice_actions).chain());
            app.update();
            let exit = named(&mut app, "PauseMenuExitButton");
            let leave = named(&mut app, "PauseMenuLeaveMatchButton");
            assert_eq!(
                app.world().get::<Node>(exit).unwrap().display,
                if mobile { Display::None } else { Display::Flex }
            );
            assert_eq!(
                app.world().get::<Node>(leave).unwrap().display,
                if mobile { Display::Flex } else { Display::None }
            );
            app.world_mut().write_message(Activated {
                source: leave,
                action: PauseAction::LeaveMatch,
            });
            app.update();
            assert_eq!(
                app.world()
                    .resource::<Messages<crate::net::SessionUiCommand>>()
                    .len(),
                usize::from(mobile)
            );
            assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        }
    }

    #[test]
    fn real_layout_keeps_close_and_footer_reachable_and_touch_scrolls_settings() {
        for (size, dpi) in [
            (Vec2::new(568.0, 320.0), 2.0),
            (Vec2::new(844.0, 390.0), 3.0),
            (Vec2::new(1024.0, 768.0), 2.0),
            (Vec2::new(1180.0, 820.0), 2.0),
        ] {
            let (mut app, window) = layout_app(size, dpi, true);
            app.insert_resource(ClientSession::admitted_for_test());
            app.update();
            app.update();
            let close = named(&mut app, "PauseMenuCloseButton");
            let footer = named(&mut app, "BackButton");
            let body = named(&mut app, "PauseMenuSettingsSection");
            let reset = named(&mut app, "PauseMenuResetGraphicsButton");
            let viewport = Rect::from_corners(Vec2::ZERO, size);
            for entity in [close, footer] {
                let r = rect(&app, entity, dpi);
                assert!(r.width() >= 44.0 && r.height() >= 44.0, "{size:?}: {r:?}");
                assert!(
                    viewport.contains(r.min) && viewport.contains(r.max),
                    "{size:?}: {r:?}"
                );
            }
            let close_before = rect(&app, close, dpi);
            let before = *app.world().resource::<AudioSettings>();
            let area = rect(&app, body, dpi);
            for id in 1..10 {
                for (phase, point) in [
                    (TouchPhase::Started, area.center()),
                    (TouchPhase::Moved, area.center() - Vec2::Y * 200.0),
                    (TouchPhase::Ended, area.center() - Vec2::Y * 200.0),
                ] {
                    app.world_mut().write_message(TouchInput {
                        phase,
                        position: point,
                        window,
                        id,
                        force: None,
                    });
                    app.update();
                }
            }
            let scroll = app.world().get::<ScrollPosition>(body).unwrap().y;
            let reset_rect = rect(&app, reset, dpi);
            println!(
                "viewport={size:?} dpi={dpi} scroll={scroll} body={area:?} reset={reset_rect:?} close={close_before:?}"
            );
            assert!(scroll > 0.0, "settings must have real scrollable content");
            assert!(
                reset_rect.height() >= 44.0,
                "last setting must be reachable: {reset_rect:?}"
            );
            assert_eq!(rect(&app, close, dpi), close_before);
            assert_eq!(*app.world().resource::<AudioSettings>(), before);
            for settings in [true, false] {
                {
                    let mut state = app.world_mut().resource_mut::<PauseMenuState>();
                    state.open = true;
                    state.in_settings = settings;
                }
                app.update();
                app.update();
                if !settings {
                    let main = named(&mut app, "PauseMenuMainSection");
                    let guide = named(&mut app, "PauseMenuHelpButton");
                    let start = rect(&app, guide, dpi).center();
                    for (phase, position) in [
                        (TouchPhase::Started, start),
                        (TouchPhase::Moved, start - Vec2::Y * 180.0),
                        (TouchPhase::Ended, start - Vec2::Y * 180.0),
                    ] {
                        app.world_mut().write_message(TouchInput {
                            phase,
                            position,
                            window,
                            id: 80,
                            force: None,
                        });
                        app.update();
                    }
                    assert!(
                        !app.world()
                            .resource::<crate::help_overlay::HelpOverlayVisible>()
                            .0
                    );
                    assert!(app.world().resource::<PauseMenuState>().open);
                    if size.y <= 320.0 {
                        assert!(app.world().get::<ScrollPosition>(main).unwrap().y > 0.0);
                    }
                    let leave = named(&mut app, "PauseMenuLeaveMatchButton");
                    assert!(rect(&app, leave, dpi).height() >= 44.0);
                }
                let r = rect(&app, close, dpi);
                assert!(viewport.contains(r.min) && viewport.contains(r.max));
                for phase in [TouchPhase::Started, TouchPhase::Ended] {
                    app.world_mut().write_message(TouchInput {
                        phase,
                        position: r.center(),
                        window,
                        id: 88,
                        force: None,
                    });
                    app.update();
                }
                assert!(
                    !app.world().resource::<PauseMenuState>().open,
                    "close must work from settings={settings}"
                );
            }
        }
    }

    #[test]
    fn tablet_sound_fits_without_scrolling_and_tabs_keep_their_controls_separate() {
        let (mut app, _) = layout_app(Vec2::new(1180.0, 820.0), 2.0, true);
        app.init_resource::<SettingsTab>()
            .add_systems(Update, settings_tabs.after(apply_pause_navigation));
        app.update();
        app.update();
        let body = named(&mut app, "PauseMenuSettingsSection");
        let bounds = rect(&app, body, 2.0);
        for id in [
            "PauseMenuAudioMasterControls",
            "PauseMenuAudioMusicControls",
            "PauseMenuAudioEffectsControls",
            "PauseMenuAudioUiControls",
        ] {
            let entity = named(&mut app, id);
            let row = rect(&app, entity, 2.0);
            assert!(
                bounds.contains(row.min) && bounds.contains(row.max),
                "{id}: {row:?} outside {bounds:?}"
            );
        }
        assert_eq!(app.world().get::<ScrollPosition>(body).unwrap().y, 0.0);
        app.world_mut().write_message(Activated {
            action: PauseAction::SettingsTab(SettingsTab::Language),
            source: Entity::PLACEHOLDER,
        });
        app.update();
        app.update();
        for (tab, node) in app
            .world_mut()
            .query::<(&SettingsTab, &Node)>()
            .iter(app.world())
        {
            assert_eq!(
                node.display,
                if *tab == SettingsTab::Language {
                    Display::Flex
                } else {
                    Display::None
                }
            );
        }
    }

    #[test]
    fn phone_sound_keeps_all_volume_controls_and_header_navigation_in_view() {
        let (mut app, _) = layout_app(Vec2::new(844.0, 390.0), 3.0, true);
        app.init_resource::<SettingsTab>()
            .add_systems(Update, settings_tabs.after(apply_pause_navigation));
        app.update();
        app.update();
        let body = named(&mut app, "PauseMenuSettingsSection");
        let bounds = rect(&app, body, 3.0);
        for id in [
            "PauseMenuAudioMasterControls",
            "PauseMenuAudioMusicControls",
            "PauseMenuAudioEffectsControls",
            "PauseMenuAudioUiControls",
            "PauseMenuAudioMuteButton",
        ] {
            let entity = named(&mut app, id);
            let control = rect(&app, entity, 3.0);
            assert!(
                bounds.contains(control.min) && bounds.contains(control.max),
                "{id}: {control:?} outside {bounds:?}"
            );
        }
        let back = named(&mut app, "BackButton");
        let close = named(&mut app, "PauseMenuCloseButton");
        for entity in [back, close] {
            let control = rect(&app, entity, 3.0);
            assert!(control.height() >= 44.0 && control.max.y <= bounds.min.y);
        }
    }

    #[test]
    fn real_short_desktop_layout_scrolls_both_bodies_and_keeps_fixed_actions() {
        for size in [Vec2::new(640.0, 280.0), Vec2::new(1280.0, 720.0)] {
            let (mut app, window) = layout_app(size, 1.0, false);
            let close = named(&mut app, "PauseMenuCloseButton");
            for settings in [false, true] {
                app.world_mut().resource_mut::<PauseMenuState>().in_settings = settings;
                app.update();
                app.update();
                let body = named(
                    &mut app,
                    if settings {
                        "PauseMenuSettingsSection"
                    } else {
                        "PauseMenuMainSection"
                    },
                );
                let footer = named(
                    &mut app,
                    if settings {
                        "BackButton"
                    } else {
                        "PauseMenuResumeButton"
                    },
                );
                let last = named(
                    &mut app,
                    if settings {
                        "PauseMenuResetGraphicsButton"
                    } else {
                        "PauseMenuExitButton"
                    },
                );
                let close_before = rect(&app, close, 1.0);
                let footer_before = rect(&app, footer, 1.0);
                app.world_mut().write_message(MouseWheel {
                    unit: MouseScrollUnit::Pixel,
                    x: 0.0,
                    y: -2000.0,
                    window,
                });
                app.update();
                let last_rect = rect(&app, last, 1.0);
                assert!(
                    last_rect.height() >= 44.0,
                    "{size:?}, settings={settings}: last action {last_rect:?}"
                );
                for r in [close_before, footer_before] {
                    assert!(r.min.y >= 0.0 && r.max.y <= size.y && r.height() >= 44.0);
                }
                assert_eq!(rect(&app, close, 1.0), close_before);
                assert_eq!(rect(&app, footer, 1.0), footer_before);
                println!(
                    "desktop={size:?} settings={settings} scroll={} last={last_rect:?}",
                    app.world().get::<ScrollPosition>(body).unwrap().y
                );
                // A press on the header close (synthetic: the UI focus pass
                // would clear a pointer-less `Interaction::Pressed`) closes the menu.
                harness::press(app.world_mut(), "PauseMenuCloseButton");
                app.update();
                assert!(!app.world().resource::<PauseMenuState>().open);
                app.world_mut().resource_mut::<PauseMenuState>().open = true;
                app.update();
            }
        }
    }

    #[test]
    fn offline_shell_settings_stay_open_but_disconnected_match_menu_closes() {
        use crate::frontend::AppScreen;
        for (screen, remains_open) in [
            (AppScreen::Home, true),
            (AppScreen::HeroSelect, true),
            (AppScreen::InMatch, false),
        ] {
            let mut app = App::new();
            let mut session = ClientSession::default();
            session.set_state_for_test(ClientConnectionState::Disconnected);
            app.insert_resource(session)
                .insert_resource(State::new(screen))
                .insert_resource(PauseMenuState {
                    open: true,
                    in_settings: true,
                })
                .add_systems(Update, close_pause_menu_when_disconnected);
            app.update();
            let menu = app.world().resource::<PauseMenuState>();
            assert_eq!(menu.open, remains_open, "{screen:?}");
            assert_eq!(menu.in_settings, remains_open, "{screen:?}");
        }
    }

    #[test]
    fn audio_menu_shows_saved_levels_and_only_changes_selected_bus_while_open() {
        let initial = AudioSettings {
            music: 0.2,
            muted: true,
            ..default()
        };
        let mut app = App::new();
        app.insert_resource(initial)
            .init_resource::<LightingSettings>()
            .init_resource::<MotionSettings>()
            .init_resource::<RenderSettings>()
            .insert_resource(PauseMenuState {
                open: false,
                in_settings: true,
            })
            .add_message::<Activated<PauseAction>>()
            .add_message::<SliderChanged>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    dispatch_actions::<PauseAction>,
                    apply_pause_audio,
                    apply_settings_sliders,
                    sync_settings_sliders,
                )
                    .chain(),
            );
        app.update();
        let music = named(&mut app, "PauseMenuAudioMusicControls");
        app.world_mut().write_message(SliderChanged {
            slider: music,
            value: 0.15,
        });
        app.update();
        assert_eq!(*app.world().resource::<AudioSettings>(), initial);
        app.world_mut().resource_mut::<PauseMenuState>().open = true;
        app.update();
        app.world_mut().write_message(SliderChanged {
            slider: music,
            value: 0.15,
        });
        app.update();
        let expected = AudioSettings {
            music: 0.15,
            ..initial
        };
        assert_eq!(*app.world().resource::<AudioSettings>(), expected);
        app.update();
        assert_eq!(*app.world().resource::<AudioSettings>(), expected);
        let parts = app.world().get::<widgets::KitParts>(music).unwrap();
        assert_eq!(
            app.world().get::<Text>(parts.extra[0].unwrap()).unwrap().0,
            "15%"
        );
        let mute = harness::find(app.world_mut(), "PauseMenuAudioMuteButton").unwrap();
        assert!(
            app.world()
                .get::<widgets::ButtonStyle>(mute)
                .unwrap()
                .selected
        );
        app.world_mut()
            .entity_mut(mute)
            .insert(Interaction::Pressed);
        app.update();
        assert_eq!(
            *app.world().resource::<AudioSettings>(),
            AudioSettings {
                muted: false,
                ..expected
            }
        );
        let mut bounded = AudioSettings::default();
        for _ in 0..25 {
            AudioBus::Master.adjust(&mut bounded, 0.05);
            AudioBus::Music.adjust(&mut bounded, -0.05);
        }
        assert_eq!(bounded.master, 1.0);
        assert_eq!(bounded.music, 0.0);
    }

    #[test]
    fn lighting_sliders_clamp_and_resync_after_external_reset() {
        let mut app = App::new();
        app.init_resource::<AudioSettings>()
            .init_resource::<LightingSettings>()
            .init_resource::<MotionSettings>()
            .init_resource::<RenderSettings>()
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: true,
            })
            .add_message::<SliderChanged>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (apply_settings_sliders, sync_settings_sliders).chain(),
            );
        app.update();
        let light = named(&mut app, "PauseMenuMainLightControls");
        let yaw = named(&mut app, "PauseMenuYawControls");
        app.world_mut().write_message(SliderChanged {
            slider: light,
            value: 2.0,
        });
        app.world_mut().write_message(SliderChanged {
            slider: yaw,
            value: -1.0,
        });
        app.update();
        assert_eq!(
            app.world().resource::<LightingSettings>().illuminance,
            MAX_LIGHT_ILLUMINANCE
        );
        assert_eq!(
            app.world().resource::<LightingSettings>().light_yaw_deg,
            MIN_LIGHT_YAW_DEG
        );
        app.world_mut().insert_resource(LightingSettings::default());
        app.update();
        let default = LightingSettings::default();
        let expected = (default.illuminance - MIN_LIGHT_ILLUMINANCE)
            / (MAX_LIGHT_ILLUMINANCE - MIN_LIGHT_ILLUMINANCE);
        assert!((app.world().get::<Slider>(light).unwrap().value - expected).abs() < 1e-5);
        let parts = app.world().get::<widgets::KitParts>(yaw).unwrap();
        assert_eq!(
            app.world().get::<Text>(parts.extra[0].unwrap()).unwrap().0,
            format!("{:.0}°", default.light_yaw_deg)
        );
    }

    #[test]
    fn mobile_mute_changes_on_short_release_but_never_on_scroll_or_closed_menu() {
        let mut app = App::new();
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.enabled = true;
        mobile.focused = true;
        mobile.landscape = true;
        app.insert_resource(mobile)
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Mobile))
            .init_resource::<AudioSettings>()
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: true,
            })
            .init_resource::<Touches>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<TouchInput>()
            .add_message::<Activated<PauseAction>>()
            .add_systems(
                Update,
                (
                    recognize_presses,
                    dispatch_actions::<PauseAction>,
                    apply_pause_audio,
                )
                    .chain(),
            );
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut().spawn((
            Button,
            Node::default(),
            UiAction(PauseAction::Audio(AudioButton::Mute)),
            Interaction::Pressed,
            BackgroundColor(theme::TILE),
            ComputedNode {
                size: Vec2::new(44.0, 44.0),
                inverse_scale_factor: 1.0,
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::new(300.0, 150.0)),
            InheritedVisibility::VISIBLE,
        ));
        let event = |id, phase, position| TouchInput {
            id,
            phase,
            position,
            window,
            force: None,
        };
        let center = Vec2::new(300.0, 150.0);
        app.world_mut()
            .write_message(event(1, TouchPhase::Started, center));
        app.update();
        assert!(!app.world().resource::<AudioSettings>().muted);
        app.world_mut()
            .write_message(event(1, TouchPhase::Moved, center + Vec2::Y * 30.0));
        app.world_mut()
            .write_message(event(1, TouchPhase::Ended, center));
        app.update();
        assert!(!app.world().resource::<AudioSettings>().muted);
        app.world_mut()
            .write_message(event(2, TouchPhase::Started, center));
        app.world_mut()
            .write_message(event(2, TouchPhase::Ended, center + Vec2::X * 3.0));
        app.update();
        assert!(app.world().resource::<AudioSettings>().muted);
        app.update();
        assert!(app.world().resource::<AudioSettings>().muted);
        app.world_mut()
            .write_message(event(3, TouchPhase::Started, center));
        app.update();
        app.world_mut().resource_mut::<PauseMenuState>().open = false;
        app.world_mut()
            .write_message(event(3, TouchPhase::Ended, center));
        app.update();
        assert!(app.world().resource::<AudioSettings>().muted);
    }

    #[test]
    fn settings_back_button_is_outside_the_scrolling_body() {
        let mut app = App::new();
        app.add_systems(Startup, setup_pause_menu_ui);
        app.update();
        let back = harness::find(app.world_mut(), "BackButton").unwrap();
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<SettingsSection>>()
            .single(app.world())
            .unwrap();
        assert!(app.world().get::<ScrollPosition>(body).is_some());
        let footer = app.world().get::<ChildOf>(back).unwrap().parent();
        assert!(app.world().get::<SettingsFooter>(footer).is_some());
        assert_ne!(footer, body);
        assert_eq!(app.world().get::<Node>(footer).unwrap().flex_shrink, 0.0);
        assert_eq!(app.world().get::<Node>(back).unwrap().height, Val::Px(46.0));
        assert_eq!(
            app.world().get::<UiAction<PauseAction>>(back).unwrap().0,
            PauseAction::BackFromSettings
        );
    }

    #[test]
    fn desktop_settings_scroll_is_clamped_and_back_navigation_resets_it() {
        let mut app = App::new();
        app.insert_resource(PauseMenuState {
            open: true,
            in_settings: true,
        })
        .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
        .add_message::<MouseWheel>()
        .add_systems(
            Update,
            (reset_pause_scroll_on_navigation, scroll_areas).chain(),
        );
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let panel = app
            .world_mut()
            .spawn((
                SettingsSection,
                ScrollArea::menu(MENU_WHEEL_STEP),
                ComputedNode {
                    size: Vec2::new(400.0, 200.0),
                    content_size: Vec2::new(400.0, 800.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
            ))
            .id();
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: -2.0,
            window,
        });
        app.update();
        assert_eq!(
            app.world().get::<ScrollPosition>(panel).unwrap().y,
            64.0,
            "32 px per line"
        );
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: -50.0,
            window,
        });
        app.update();
        assert_eq!(app.world().get::<ScrollPosition>(panel).unwrap().y, 600.0);
        app.world_mut().resource_mut::<PauseMenuState>().in_settings = false;
        app.update();
        assert_eq!(app.world().get::<ScrollPosition>(panel).unwrap().y, 0.0);
    }

    /// Modal registry: the phone server-address entry opened over the pause
    /// menu takes its taps and its scroll without touching `Pressable::disabled`.
    #[test]
    fn server_entry_over_the_pause_menu_blocks_its_buttons_until_it_closes() {
        let (mut app, window) = layout_app(Vec2::new(844.0, 390.0), 2.0, true);
        app.init_resource::<crate::mobile_ui::ServerEntry>()
            .register_modal::<PauseMenuState>(ModalId::Pause, |menu| menu.open)
            .register_modal::<crate::mobile_ui::ServerEntry>(ModalId::ServerEntry, |entry| {
                entry.open
            });
        app.world_mut().resource_mut::<PauseMenuState>().in_settings = false;
        app.update();
        app.update();
        let help = harness::find(app.world_mut(), "PauseMenuHelpButton").unwrap();
        let tap = |app: &mut App, id| {
            let center = rect(app, help, 2.0).center();
            for phase in [TouchPhase::Started, TouchPhase::Ended] {
                app.world_mut().write_message(TouchInput {
                    phase,
                    position: center,
                    window,
                    id,
                    force: None,
                });
            }
            app.update();
        };
        app.world_mut()
            .resource_mut::<crate::mobile_ui::ServerEntry>()
            .open = true;
        app.update();
        let pressable = *app.world().get::<Pressable>(help).unwrap();
        assert!(pressable.blocked && !pressable.disabled);
        tap(&mut app, 1);
        assert!(
            !app.world()
                .resource::<crate::help_overlay::HelpOverlayVisible>()
                .0
        );
        // A synthetic press is gated like a tap.
        harness::press(app.world_mut(), "PauseMenuHelpButton");
        app.update();
        assert!(
            !app.world()
                .resource::<crate::help_overlay::HelpOverlayVisible>()
                .0
        );
        assert!(app.world().resource::<PauseMenuState>().open);
        app.world_mut()
            .resource_mut::<crate::mobile_ui::ServerEntry>()
            .open = false;
        app.update();
        assert!(!app.world().get::<Pressable>(help).unwrap().blocked);
        tap(&mut app, 2);
        assert!(
            app.world()
                .resource::<crate::help_overlay::HelpOverlayVisible>()
                .0
        );
    }

    #[test]
    fn pause_tap_cancels_a_scroll_even_after_returning_to_the_button() {
        use crate::ui::gesture::TapTracker;
        let entity = Entity::PLACEHOLDER;
        let rect = Rect::from_center_size(Vec2::new(100.0, 100.0), Vec2::new(180.0, 46.0));
        let buttons = [(entity, rect)];
        let mut state = TapTracker::default();
        assert_eq!(
            state.event(1, TouchPhase::Started, rect.center(), &buttons),
            None
        );
        assert_eq!(
            state.event(2, TouchPhase::Ended, rect.center(), &buttons),
            None
        );
        assert_eq!(
            state.event(
                1,
                TouchPhase::Moved,
                rect.center() + Vec2::Y * 30.0,
                &buttons
            ),
            None
        );
        assert_eq!(
            state.event(1, TouchPhase::Ended, rect.center(), &buttons),
            None
        );
        state.event(3, TouchPhase::Started, rect.center(), &buttons);
        assert_eq!(
            state.event(3, TouchPhase::Canceled, rect.center(), &buttons),
            None
        );
        state.event(4, TouchPhase::Started, rect.center(), &buttons);
        assert_eq!(
            state.event(
                4,
                TouchPhase::Ended,
                rect.center() + Vec2::X * 4.0,
                &buttons
            ),
            Some(entity)
        );
        assert_eq!(
            state.event(4, TouchPhase::Ended, rect.center(), &buttons),
            None
        );
    }

    #[test]
    fn mobile_taps_cannot_terminate_the_application() {
        let mut app = App::new();
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.enabled = true;
        app.insert_resource(mobile)
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Mobile))
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: false,
            })
            .init_resource::<ClientSession>()
            .init_resource::<Touches>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<TouchInput>()
            .add_message::<AppExit>()
            .add_message::<crate::net::SessionUiCommand>()
            .add_message::<Activated<PauseAction>>()
            .add_systems(
                Update,
                (
                    recognize_presses,
                    dispatch_actions::<PauseAction>,
                    apply_pause_session,
                )
                    .chain(),
            );
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let button = app
            .world_mut()
            .spawn((
                Button,
                Node::default(),
                UiAction(PauseAction::Exit),
                Interaction::Pressed,
                BackgroundColor(theme::TILE),
                ComputedNode {
                    size: Vec2::new(180.0, 46.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
                UiGlobalTransform::from_translation(Vec2::new(300.0, 150.0)),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let event = |id, phase, position| TouchInput {
            id,
            phase,
            position,
            window,
            force: None,
        };
        let center = Vec2::new(300.0, 150.0);
        app.world_mut()
            .write_message(event(1, TouchPhase::Started, center));
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        app.world_mut()
            .write_message(event(1, TouchPhase::Moved, center + Vec2::Y * 40.0));
        app.world_mut()
            .write_message(event(1, TouchPhase::Ended, center));
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        app.world_mut()
            .write_message(event(2, TouchPhase::Started, center));
        app.update();
        app.world_mut()
            .resource_mut::<crate::mobile_controls::MobileControls>()
            .focused = false;
        app.update();
        app.world_mut()
            .resource_mut::<crate::mobile_controls::MobileControls>()
            .focused = true;
        app.world_mut()
            .write_message(event(2, TouchPhase::Ended, center));
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        // A clipped-away button cannot receive a new tap through the scroll panel.
        app.world_mut()
            .entity_mut(button)
            .insert(bevy::ui::CalculatedClip {
                clip: Rect::from_corners(Vec2::ZERO, Vec2::splat(10.0)),
            });
        app.world_mut()
            .write_message(event(3, TouchPhase::Started, center));
        app.world_mut()
            .write_message(event(3, TouchPhase::Ended, center));
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        app.world_mut()
            .entity_mut(button)
            .remove::<bevy::ui::CalculatedClip>();
        app.world_mut()
            .write_message(event(4, TouchPhase::Started, center));
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        app.world_mut()
            .write_message(event(4, TouchPhase::Ended, center + Vec2::X * 3.0));
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        assert!(app.world().get_entity(window).is_ok());
    }

    #[test]
    fn resume_button_keeps_authoritative_player_and_loadout_intact() {
        let mut app = App::new();
        app.insert_resource(PauseMenuState {
            open: true,
            in_settings: false,
        })
        .init_resource::<TeamSelection>()
        .init_resource::<crate::help_overlay::HelpOverlayVisible>()
        .add_message::<Activated<PauseAction>>()
        .add_systems(Startup, setup_pause_menu_ui)
        .add_systems(
            Update,
            (
                dispatch_actions::<PauseAction>,
                apply_pause_navigation,
                sync_pause_menu_visibility,
            )
                .chain(),
        );
        app.world_mut().resource_mut::<TeamSelection>().team = Some(crate::team::Team::Green);
        let player = app.world_mut().spawn(crate::player::Player).id();
        app.update();
        let button = harness::find(app.world_mut(), "PauseMenuResumeButton").unwrap();
        assert_eq!(
            app.world()
                .get::<crate::ui::widgets::ButtonStyle>(button)
                .unwrap()
                .kind,
            ButtonKind::Primary
        );
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        assert!(!app.world().resource::<PauseMenuState>().open);
        assert!(app.world().get_entity(player).is_ok());
        assert_eq!(
            app.world().resource::<TeamSelection>().team,
            Some(crate::team::Team::Green)
        );
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<crate::team::TeamSelectRoot>>()
                .iter(app.world())
                .count(),
            0
        );
        let mut root = app
            .world_mut()
            .query_filtered::<(&Node, &Visibility), With<PauseMenuRoot>>();
        let (node, visibility) = root.single(app.world()).unwrap();
        assert_eq!(node.display, Display::None);
        assert_eq!(*visibility, Visibility::Hidden);
    }

    #[test]
    fn exit_button_emits_clean_application_exit() {
        let mut app = App::new();
        app.insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .init_resource::<ClientSession>()
            .init_resource::<PauseMenuState>()
            .add_message::<AppExit>()
            .add_message::<crate::net::SessionUiCommand>()
            .add_message::<Activated<PauseAction>>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (dispatch_actions::<PauseAction>, apply_pause_session).chain(),
            );
        app.update();
        let button = harness::find(app.world_mut(), "PauseMenuExitButton").unwrap();
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        assert_eq!(app.world().resource::<Messages<AppExit>>().len(), 1);
        // The synthetic press reaches the same handler through the recognizer.
        app.add_message::<crate::ui::SyntheticPress>()
            .add_message::<TouchInput>()
            .add_systems(
                Update,
                recognize_presses.before(dispatch_actions::<PauseAction>),
            );
        app.world_mut().entity_mut(button).insert(Interaction::None);
        app.update();
        app.world_mut().resource_mut::<Messages<AppExit>>().clear();
        harness::press(app.world_mut(), "PauseMenuExitButton");
        app.update();
        assert_eq!(app.world().resource::<Messages<AppExit>>().len(), 1);
    }

    /// The Language row cycles the shipped languages; every pause-menu text
    /// follows without a respawn: `Localized` labels, the state-dependent
    /// mute caption and hints, and the row's own value. Names and TestIds
    /// never change. Isolated: it switches the process-wide language.
    #[test]
    fn language_row_switches_every_pause_menu_text_live() {
        if crate::i18n::testing::isolated(
            "pause_menu::tests::language_row_switches_every_pause_menu_text_live",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let mut app = App::new();
        app.add_plugins(I18nPlugin::default())
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: true,
            })
            .init_resource::<AudioSettings>()
            .init_resource::<ClientSession>()
            .init_resource::<ResolvedServerAddressForPrefs>()
            .add_message::<Activated<PauseAction>>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    dispatch_actions::<PauseAction>,
                    apply_pause_language,
                    update_language_value,
                    sync_practice_actions,
                    sync_settings_server_addr_label,
                )
                    .chain(),
            );
        app.update();
        let text = |app: &mut App, id: &str| {
            let entity = named(app, id);
            app.world().get::<Text>(entity).unwrap().0.clone()
        };
        let child_text = |app: &mut App, id: &str| {
            let entity = named(app, id);
            let child = app
                .world()
                .get::<widgets::KitParts>(entity)
                .unwrap()
                .label
                .unwrap();
            app.world().get::<Text>(child).unwrap().0.clone()
        };
        assert_eq!(text(&mut app, "PauseMenuLanguageValue"), "English");
        assert_eq!(text(&mut app, "PauseMenuTitle"), "Game menu");
        assert_eq!(child_text(&mut app, "SettingsButton"), "Settings");
        assert_eq!(
            child_text(&mut app, "PauseMenuAudioMuteButton"),
            "Mute sound"
        );
        let button = harness::find(app.world_mut(), "PauseMenuLanguageButton").unwrap();
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        app.world_mut().entity_mut(button).insert(Interaction::None);
        app.update();
        assert_eq!(
            app.world().resource::<Locale>().id(),
            LocaleId::parse("ru").unwrap()
        );
        assert_eq!(text(&mut app, "PauseMenuLanguageValue"), "Русский");
        assert_eq!(child_text(&mut app, "SettingsButton"), "Настройки");
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        app.world_mut().entity_mut(button).insert(Interaction::None);
        app.update();
        let zh = LocaleId::parse("zh-Hans").unwrap();
        assert_eq!(app.world().resource::<Locale>().id(), zh);
        assert_eq!(app.world().resource::<Locale>().generation(), 2);
        assert_eq!(text(&mut app, "PauseMenuLanguageValue"), "简体中文");
        assert_eq!(text(&mut app, "PauseMenuTitle"), "游戏菜单");
        assert_eq!(child_text(&mut app, "SettingsButton"), "设置");
        assert_eq!(child_text(&mut app, "BackButton"), "返回");
        assert_eq!(child_text(&mut app, "PauseMenuAudioMuteButton"), "静音");
        assert_eq!(text(&mut app, "PauseMenuAudioTitle"), "声音");
        assert_eq!(
            text(&mut app, "PauseMenuMainTitle"),
            "菜单打开期间，对局仍在继续。"
        );
        assert!(text(&mut app, "PauseMenuServerAddrHint").contains("设置会自动保存。"));
        let row = named(&mut app, "PauseMenuAudioMusicControls");
        let caption = app.world().get::<Children>(row).unwrap()[0];
        assert_eq!(app.world().get::<Text>(caption).unwrap().0, "音乐");
        // Cycling again wraps back to English.
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Locale>().id(), LocaleId::ENGLISH);
        assert_eq!(text(&mut app, "PauseMenuTitle"), "Game menu");
        assert_eq!(
            child_text(&mut app, "PauseMenuAudioMuteButton"),
            "Mute sound"
        );
    }

    #[test]
    fn language_row_sits_on_the_settings_page_with_stable_ids() {
        let mut app = App::new();
        app.add_systems(Startup, setup_pause_menu_ui);
        app.update();
        let button = harness::find(app.world_mut(), "PauseMenuLanguageButton").unwrap();
        let value = harness::find(app.world_mut(), "PauseMenuLanguageValue").unwrap();
        let motion = harness::find(app.world_mut(), "PauseMenuReduceMotionButton").unwrap();
        assert_eq!(
            app.world().get::<UiAction<PauseAction>>(button).unwrap().0,
            PauseAction::CycleLanguage
        );
        assert_eq!(app.world().get::<Text>(value).unwrap().0, "English");
        assert_eq!(
            app.world().get::<UiAction<PauseAction>>(motion).unwrap().0,
            PauseAction::ToggleReduceMotion
        );
        let section = app.world().get::<ChildOf>(button).unwrap().parent();
        assert_eq!(
            app.world().get::<SettingsTab>(section),
            Some(&SettingsTab::Language)
        );
        let body = app.world().get::<ChildOf>(section).unwrap().parent();
        assert!(app.world().get::<SettingsSection>(body).is_some());
    }

    #[test]
    fn reduce_motion_toggle_is_edge_triggered_and_keeps_graphics_reset_separate() {
        let mut app = App::new();
        app.init_resource::<LightingSettings>()
            .init_resource::<ModelScaleSettings>()
            .init_resource::<CameraSettings>()
            .init_resource::<MotionSettings>()
            .init_resource::<ClientPrefsSaveGate>()
            .init_resource::<ResolvedServerAddressForPrefs>()
            .init_resource::<ClientSessionId>()
            .init_resource::<TeamSelection>()
            .init_resource::<AudioSettings>()
            .add_message::<Activated<PauseAction>>()
            .add_systems(Update, apply_pause_settings);
        app.world_mut().write_message(Activated {
            action: PauseAction::ToggleReduceMotion,
            source: Entity::PLACEHOLDER,
        });
        app.update();
        assert!(app.world().resource::<MotionSettings>().reduce);
        app.update();
        assert!(app.world().resource::<MotionSettings>().reduce);
    }

    #[test]
    fn render_and_hud_controls_require_settings_and_clamp_and_reset_offsets() {
        let mut app = App::new();
        app.init_resource::<PauseMenuState>()
            .init_resource::<RenderSettings>()
            .init_resource::<HudPositionSettings>()
            .add_message::<Activated<PauseAction>>()
            .add_systems(Update, apply_render_and_hud_settings);
        let press = |app: &mut App, action| {
            app.world_mut().write_message(Activated {
                action,
                source: Entity::PLACEHOLDER,
            });
            app.update();
        };
        press(&mut app, PauseAction::ToggleFpsReadout);
        assert!(app.world().resource::<RenderSettings>().show_fps);
        press(&mut app, PauseAction::Step(Setting::RenderFps, 1));
        assert_eq!(app.world().resource::<RenderSettings>().fps_limit, 60);
        *app.world_mut().resource_mut::<PauseMenuState>() = PauseMenuState {
            open: true,
            in_settings: true,
        };
        press(&mut app, PauseAction::Step(Setting::RenderFps, 1));
        assert_eq!(app.world().resource::<RenderSettings>().fps_limit, 120);
        press(&mut app, PauseAction::ToggleFpsReadout);
        assert!(!app.world().resource::<RenderSettings>().show_fps);
        app.update();
        assert!(!app.world().resource::<RenderSettings>().show_fps);
        for _ in 0..20 {
            press(&mut app, PauseAction::Step(Setting::CombatX, -1));
        }
        assert_eq!(
            app.world()
                .resource::<HudPositionSettings>()
                .combat_offset
                .x,
            -60.0
        );
        press(&mut app, PauseAction::Step(Setting::JoystickY, -1));
        assert_eq!(
            app.world()
                .resource::<HudPositionSettings>()
                .joystick_offset
                .y,
            -8.0
        );
        press(&mut app, PauseAction::ResetHud);
        assert_eq!(
            *app.world().resource::<HudPositionSettings>(),
            HudPositionSettings::default()
        );
        assert_eq!(app.world().resource::<RenderSettings>().fps_limit, 120);
        press(&mut app, PauseAction::Step(Setting::RenderFps, -1));
        assert_eq!(app.world().resource::<RenderSettings>().fps_limit, 60);
    }

    /// R6.5: Controls opened from Settings closes back into Settings (the
    /// Esc that closed the guide does not also close the menu); the Game
    /// menu's Controls guide still returns to the game.
    #[test]
    fn controls_from_settings_return_to_settings_and_the_game_menu_guide_does_not() {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .insert_state(crate::frontend::AppScreen::Home)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<crate::net::GameStateSnapshot>()
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: true,
            })
            .init_resource::<SettingsHelpReturn>()
            .add_message::<Activated<PauseAction>>()
            .add_plugins(crate::help_overlay::HelpOverlayPlugin)
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    dispatch_actions::<PauseAction>
                        .before(crate::help_overlay::HelpOverlaySet::Input),
                    toggle_pause_menu.after(crate::help_overlay::HelpOverlaySet::Input),
                    (apply_pause_navigation, return_to_settings_after_help)
                        .chain()
                        .after(toggle_pause_menu),
                ),
            );
        app.update();
        let press = |app: &mut App, id: &str| {
            let button = harness::find(app.world_mut(), id).unwrap();
            app.world_mut()
                .entity_mut(button)
                .insert(Interaction::Pressed);
            app.update();
            app.world_mut().entity_mut(button).insert(Interaction::None);
            app.update();
        };
        let escape = |app: &mut App| {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
            app.update();
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::Escape);
            keys.clear();
            app.update();
        };
        let help = |app: &App| {
            app.world()
                .resource::<crate::help_overlay::HelpOverlayVisible>()
                .0
        };
        press(&mut app, "PauseMenuSettingsControlsButton");
        assert!(help(&app));
        assert!(!app.world().resource::<PauseMenuState>().open);
        escape(&mut app);
        assert!(!help(&app));
        let menu = app.world().resource::<PauseMenuState>();
        assert!(menu.open && menu.in_settings, "back on Settings");
        // The Game menu's guide closes to the game (today's behaviour).
        app.world_mut().resource_mut::<PauseMenuState>().in_settings = false;
        app.update();
        press(&mut app, "PauseMenuHelpButton");
        assert!(help(&app));
        escape(&mut app);
        assert!(!help(&app));
        assert!(!app.world().resource::<PauseMenuState>().open);
        assert!(!app.world().resource::<SettingsHelpReturn>().0);
    }

    #[test]
    fn escape_dismisses_help_before_opening_menu() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<PauseMenuState>()
            .insert_resource(crate::net::GameStateSnapshot {
                state: crate::net::GameState::Running,
                ..default()
            })
            .add_plugins(crate::help_overlay::HelpOverlayPlugin)
            .add_systems(
                Update,
                toggle_pause_menu
                    .after(crate::help_overlay::HelpOverlaySet::Input)
                    .after(crate::shop::ShopModalSet),
            );
        app.update();
        assert!(
            app.world()
                .resource::<crate::help_overlay::HelpOverlayVisible>()
                .0
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(
            !app.world()
                .resource::<crate::help_overlay::HelpOverlayVisible>()
                .0
        );
        assert!(!app.world().resource::<PauseMenuState>().open);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(app.world().resource::<PauseMenuState>().open);
    }
}
