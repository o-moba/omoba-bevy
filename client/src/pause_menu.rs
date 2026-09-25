//! In-match pause menu: main page, settings page and the debug tools page
//! (`crate::debug::tools_page`). Built on the UI kit (`crate::ui`): every control carries a
//! `PauseAction`, the kit recognizes clicks and taps and paints the buttons,
//! and the systems here only consume `Activated<PauseAction>`.
use bevy::{
    app::AppExit,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use crate::audio_settings::AudioSettings;
use crate::camera::{CAMERA_ZOOM_STEP, CameraSettings};
use crate::model_scale::{
    DEFAULT_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT, MIN_MODEL_TARGET_HEIGHT,
    ModelScaleSettings,
};
use crate::net::{ClientConnectionState, ClientSession};
use crate::persistence::{
    ClientPrefsSaveGate, ClientSessionId, ResolvedServerAddressForPrefs, reset_graphics_to_defaults,
};
use crate::session_config::DEFAULT_GAME_SERVER_ADDR;
use crate::team::TeamSelection;
use crate::ui::{
    Activated, GestureEpoch, ModalId, ModalRoot, ScrollArea, UiAction, UiActionAppExt, UiSet,
    theme::{self, ButtonKind, metric},
    widgets,
};
use crate::world::{
    DEFAULT_AMBIENT_BRIGHTNESS, DEFAULT_LIGHT_ILLUMINANCE, DEFAULT_LIGHT_PITCH_DEG,
    DEFAULT_LIGHT_YAW_DEG, LightingSettings, MAX_AMBIENT_BRIGHTNESS, MAX_LIGHT_ILLUMINANCE,
    MAX_LIGHT_PITCH_DEG, MAX_LIGHT_YAW_DEG, MIN_AMBIENT_BRIGHTNESS, MIN_LIGHT_ILLUMINANCE,
    MIN_LIGHT_PITCH_DEG, MIN_LIGHT_YAW_DEG,
};

const PANEL_WIDTH: f32 = 480.0;
const PANEL_HEIGHT: f32 = 560.0;
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
            .init_resource::<AudioSettings>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
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
                    apply_pause_settings,
                    apply_pause_audio,
                    apply_pause_session,
                    update_setting_labels.after(apply_pause_settings),
                    update_audio_labels.after(apply_pause_audio),
                    sync_pause_menu_visibility,
                    sync_pause_menu_sections,
                    reset_pause_scroll_on_navigation.after(apply_pause_navigation),
                    sync_practice_actions,
                    sync_settings_server_addr_label,
                )
                    .in_set(PauseMenuSet::Visuals),
            )
            .add_systems(
                PostUpdate,
                size_desktop_pause_panel.before(bevy::ui::UiSystems::Layout),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct PauseMenuState {
    pub(crate) open: bool,
    pub(crate) in_settings: bool,
}

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

#[derive(Component)]
struct PauseMenuPanel;

/// A stepped graphics setting on the settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Setting {
    CameraZoom,
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
    ResetGraphics,
    /// One step of a setting; the sign is the direction.
    Step(Setting, i8),
    Audio(AudioButton),
    OpenPractice,
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
    Adjust(AudioBus, f32),
    Mute,
}
#[derive(Component, Clone, Copy)]
enum AudioLabel {
    Bus(AudioBus),
    Mute,
}

fn size_desktop_pause_panel(
    menu: Res<PauseMenuState>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut panels: Query<&mut Node, With<PauseMenuPanel>>,
) {
    if mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
        return;
    }
    let height = if menu.in_settings {
        Val::Px(PANEL_HEIGHT)
    } else {
        Val::Px(380.0)
    };
    for mut panel in &mut panels {
        if panel.height != height {
            panel.height = height;
        }
    }
}

fn section_title(parent: &mut ChildSpawnerCommands, text: &str, name: &str) {
    parent.spawn((
        Text::new(text),
        theme::text(18.0),
        TextColor(theme::GOLD),
        Name::new(name.to_owned()),
    ));
}

fn setting_row(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: String,
    setting: Setting,
    id: &str,
) {
    widgets::adjust_row(
        parent,
        label,
        value,
        SettingLabel(setting),
        PauseAction::Step(setting, -1),
        PauseAction::Step(setting, 1),
        id,
    );
}

fn setup_pause_menu_ui(mut commands: Commands) {
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
                        width: Val::Px(PANEL_WIDTH),
                        height: Val::Px(PANEL_HEIGHT),
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
                    BorderColor::all(theme::EDGE),
                    PauseMenuPanel,
                    Name::new("PauseMenuPanel"),
                ))
                .with_children(|panel| {
                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                min_height: Val::Px(metric::BUTTON_H),
                                flex_shrink: 0.0,
                                justify_content: JustifyContent::SpaceBetween,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            Name::new("PauseMenuHeader"),
                        ))
                        .with_children(|header| {
                            header.spawn((
                                Text::new("Game menu"),
                                theme::text(28.0),
                                TextColor(theme::IVORY),
                                Name::new("PauseMenuTitle"),
                            ));
                            widgets::icon_button(
                                header,
                                "×",
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
                                Text::new("Your match continues while this menu is open."),
                                theme::text(14.0),
                                TextColor(theme::MUTED),
                                Name::new("PauseMenuMainTitle"),
                            ));
                            widgets::button(
                                main,
                                "Settings",
                                ButtonKind::Secondary,
                                PauseAction::OpenSettings,
                                "SettingsButton",
                            );
                            crate::debug::tools_page::spawn_practice_open_button(main);
                            widgets::button(
                                main,
                                "Controls guide",
                                ButtonKind::Secondary,
                                PauseAction::Help,
                                "PauseMenuHelpButton",
                            );
                            widgets::button(
                                main,
                                "Exit game",
                                ButtonKind::Secondary,
                                PauseAction::Exit,
                                "PauseMenuExitButton",
                            );
                            widgets::button(
                                main,
                                "Leave practice",
                                ButtonKind::Secondary,
                                PauseAction::LeavePractice,
                                "PauseMenuLeavePracticeButton",
                            );
                        });

                    panel
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(8.0),
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
                            settings.spawn((
                                Text::new("Settings"),
                                theme::text(22.0),
                                TextColor(theme::IVORY),
                                Name::new("PauseMenuSettingsTitle"),
                            ));

                            section_title(settings, "Sound", "PauseMenuAudioTitle");
                            for (bus, label, name) in [
                                (AudioBus::Master, "Master", "PauseMenuAudioMasterControls"),
                                (AudioBus::Music, "Music", "PauseMenuAudioMusicControls"),
                                (
                                    AudioBus::Effects,
                                    "Effects",
                                    "PauseMenuAudioEffectsControls",
                                ),
                                (AudioBus::Ui, "Interface", "PauseMenuAudioUiControls"),
                            ] {
                                widgets::adjust_row(
                                    settings,
                                    label,
                                    format!("{:.0}%", bus.value(AudioSettings::default()) * 100.0),
                                    AudioLabel::Bus(bus),
                                    PauseAction::Audio(AudioButton::Adjust(bus, -AUDIO_STEP)),
                                    PauseAction::Audio(AudioButton::Adjust(bus, AUDIO_STEP)),
                                    name,
                                );
                            }
                            widgets::button_with_label(
                                settings,
                                "Mute sound",
                                ButtonKind::Secondary,
                                PauseAction::Audio(AudioButton::Mute),
                                "PauseMenuAudioMuteButton",
                                AudioLabel::Mute,
                                "PauseMenuAudioMuteLabel",
                            );

                            settings.spawn((
                                Text::new(""),
                                theme::text(14.0),
                                TextColor(theme::MUTED),
                                SettingsServerAddrLabel,
                                Name::new("PauseMenuServerAddrHint"),
                            ));

                            section_title(settings, "Lighting", "PauseMenuLightingTitle");
                            setting_row(
                                settings,
                                "Main Light",
                                format!("{:.0}", DEFAULT_LIGHT_ILLUMINANCE),
                                Setting::Light,
                                "PauseMenuMainLightControls",
                            );
                            setting_row(
                                settings,
                                "Ambient",
                                format!("{:.0}", DEFAULT_AMBIENT_BRIGHTNESS),
                                Setting::Ambient,
                                "PauseMenuAmbientControls",
                            );
                            setting_row(
                                settings,
                                "Pitch",
                                format!("{:.0}°", DEFAULT_LIGHT_PITCH_DEG),
                                Setting::Pitch,
                                "PauseMenuPitchControls",
                            );
                            setting_row(
                                settings,
                                "Yaw",
                                format!("{:.0}°", DEFAULT_LIGHT_YAW_DEG),
                                Setting::Yaw,
                                "PauseMenuYawControls",
                            );

                            section_title(settings, "Camera", "PauseMenuCameraTitle");
                            // 100% is the default follow view; lower values bring
                            // the camera closer so hero silhouettes read larger.
                            setting_row(
                                settings,
                                "Distance",
                                CameraSettings::default().percent_label(),
                                Setting::CameraZoom,
                                "PauseMenuCameraZoomControls",
                            );

                            section_title(settings, "Model", "PauseMenuModelTitle");
                            setting_row(
                                settings,
                                "Scale",
                                format!("{:.2}", DEFAULT_MODEL_TARGET_HEIGHT),
                                Setting::ModelScale,
                                "PauseMenuScaleControls",
                            );

                            widgets::button(
                                settings,
                                "Reset graphics",
                                ButtonKind::Secondary,
                                PauseAction::ResetGraphics,
                                "PauseMenuResetGraphicsButton",
                            );
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
                            widgets::button(
                                footer,
                                "Return to game",
                                ButtonKind::Primary,
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
                                ..default()
                            },
                            Visibility::Hidden,
                            SettingsFooter,
                            Name::new("PauseMenuSettingsFooter"),
                        ))
                        .with_children(|footer| {
                            widgets::button(
                                footer,
                                "Back",
                                ButtonKind::Secondary,
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

pub(crate) fn toggle_pause_menu(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    social: Option<Res<crate::social::SocialClient>>,
    mut menu_state: ResMut<PauseMenuState>,
) {
    if social
        .as_ref()
        .is_some_and(|social| social.blocks_gameplay())
    {
        return;
    }
    if keyboard_input.just_pressed(KeyCode::Escape) {
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
        Query<(&mut Visibility, &mut Node), Or<(With<SettingsSection>, With<SettingsFooter>)>>,
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
fn apply_pause_navigation(
    mut activated: MessageReader<Activated<PauseAction>>,
    mut menu: ResMut<PauseMenuState>,
    mut help: ResMut<crate::help_overlay::HelpOverlayVisible>,
) {
    for Activated { action, .. } in activated.read() {
        match action {
            PauseAction::Resume | PauseAction::Close => {
                menu.open = false;
                menu.in_settings = false;
            }
            PauseAction::OpenSettings => menu.in_settings = true,
            PauseAction::BackFromSettings => menu.in_settings = false,
            PauseAction::Help if menu.open => {
                menu.open = false;
                menu.in_settings = false;
                help.0 = true;
            }
            _ => {}
        }
    }
}

/// Graphics steppers and the reset button.
fn apply_pause_settings(
    mut activated: MessageReader<Activated<PauseAction>>,
    mut lighting: ResMut<LightingSettings>,
    mut model: ResMut<ModelScaleSettings>,
    mut camera: ResMut<CameraSettings>,
    mut prefs_gate: ResMut<ClientPrefsSaveGate>,
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    client_session_id: Res<ClientSessionId>,
    team: Res<TeamSelection>,
    audio: Res<AudioSettings>,
) {
    for Activated { action, .. } in activated.read() {
        match *action {
            PauseAction::Step(setting, direction) => {
                let sign = f32::from(direction.signum());
                match setting {
                    Setting::CameraZoom => camera.adjust(sign * CAMERA_ZOOM_STEP),
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
                );
            }
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
        let PauseAction::Audio(button) = action else {
            continue;
        };
        if !allowed {
            continue;
        }
        match *button {
            AudioButton::Adjust(bus, delta) => bus.adjust(&mut settings, delta),
            AudioButton::Mute => settings.muted = !settings.muted,
        }
    }
}

/// Leaving: quit the application or leave an offline practice match.
fn apply_pause_session(
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
            PauseAction::Exit => {
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
            PauseAction::LeavePractice if session.is_offline() => {
                session_commands.write(crate::net::SessionUiCommand::LeaveMatch);
                menu.open = false;
                menu.in_settings = false;
            }
            _ => {}
        }
    }
}

fn update_audio_labels(settings: Res<AudioSettings>, mut labels: Query<(&AudioLabel, &mut Text)>) {
    if !settings.is_changed() {
        return;
    }
    let settings = settings.sanitized();
    for (label, mut text) in &mut labels {
        text.0 = match *label {
            AudioLabel::Bus(bus) => format!("{:.0}%", bus.value(settings) * 100.0),
            AudioLabel::Mute => if settings.muted {
                "Unmute sound"
            } else {
                "Mute sound"
            }
            .into(),
        };
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
    camera: Res<CameraSettings>,
    model: Res<ModelScaleSettings>,
    lighting: Res<LightingSettings>,
    mut labels: Query<(&SettingLabel, &mut Text)>,
) {
    for (label, mut text) in &mut labels {
        let next = match label.0 {
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
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    mut label_q: Query<&mut Text, With<SettingsServerAddrLabel>>,
) {
    let addr = server_addr_for_prefs(&resolved_addr);
    let next = format!("Server: {addr}\nSettings are saved automatically.");
    if let Ok(mut text) = label_q.single_mut() {
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// "Exit game" belongs to online matches, "Leave practice" to offline ones.
fn sync_practice_actions(
    session: Res<ClientSession>,
    mut buttons: Query<(&mut Node, &UiAction<PauseAction>)>,
    mut hints: Query<(&Name, &mut Text)>,
) {
    let offline = session.is_offline();
    for (mut node, action) in &mut buttons {
        let leave_practice = match action.0 {
            PauseAction::Exit => false,
            PauseAction::LeavePractice => true,
            _ => continue,
        };
        let display = if offline == leave_practice {
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
            let label = if offline {
                "Offline practice · No rating or progression rewards."
            } else {
                "Your match continues while this menu is open."
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

    fn action_button(app: &mut App, wanted: impl Fn(&PauseAction) -> bool) -> Entity {
        app.world_mut()
            .query::<(Entity, &UiAction<PauseAction>)>()
            .iter(app.world())
            .find(|(_, action)| wanted(&action.0))
            .unwrap()
            .0
    }

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
                size_desktop_pause_panel.before(bevy::ui::UiSystems::Layout),
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
            .query::<(Entity, &Name)>()
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
    fn real_layout_keeps_close_and_footer_reachable_and_touch_scrolls_settings() {
        for (size, dpi) in [
            (Vec2::new(568.0, 320.0), 2.0),
            (Vec2::new(844.0, 390.0), 3.0),
            (Vec2::new(1024.0, 768.0), 2.0),
            (Vec2::new(1180.0, 820.0), 2.0),
        ] {
            let (mut app, window) = layout_app(size, dpi, true);
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
                    let exit = named(&mut app, "PauseMenuExitButton");
                    assert!(rect(&app, exit, dpi).height() >= 44.0);
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
            .insert_resource(PauseMenuState {
                open: false,
                in_settings: true,
            })
            .add_message::<Activated<PauseAction>>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    dispatch_actions::<PauseAction>,
                    apply_pause_audio,
                    update_audio_labels,
                )
                    .chain(),
            );
        app.update();
        let music = action_button(
            &mut app,
            |action| matches!(action, PauseAction::Audio(AudioButton::Adjust(AudioBus::Music, delta)) if *delta < 0.0),
        );
        app.world_mut()
            .entity_mut(music)
            .insert(Interaction::Pressed);
        app.update();
        assert_eq!(*app.world().resource::<AudioSettings>(), initial);
        app.world_mut().entity_mut(music).insert(Interaction::None);
        app.world_mut().resource_mut::<PauseMenuState>().open = true;
        app.update();
        app.world_mut()
            .entity_mut(music)
            .insert(Interaction::Pressed);
        app.update();
        let expected = AudioSettings {
            music: 0.15,
            ..initial
        };
        assert_eq!(*app.world().resource::<AudioSettings>(), expected);
        app.update();
        assert_eq!(*app.world().resource::<AudioSettings>(), expected);
        let labels: Vec<_> = app
            .world_mut()
            .query::<(&AudioLabel, &Text)>()
            .iter(app.world())
            .map(|(label, text)| (*label, text.0.clone()))
            .collect();
        assert!(
            labels.iter().any(
                |(label, text)| matches!(label, AudioLabel::Bus(AudioBus::Music)) && text == "15%"
            )
        );
        assert!(
            labels
                .iter()
                .any(|(label, text)| matches!(label, AudioLabel::Mute) && text == "Unmute sound")
        );
        let mute = harness::find(app.world_mut(), "PauseMenuAudioMuteButton").unwrap();
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
    fn mobile_audio_changes_on_short_release_but_never_on_scroll_or_closed_menu() {
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
            UiAction(PauseAction::Audio(AudioButton::Adjust(
                AudioBus::Music,
                0.05,
            ))),
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
        assert_eq!(app.world().resource::<AudioSettings>().music, 0.25);
        app.world_mut()
            .write_message(event(1, TouchPhase::Moved, center + Vec2::Y * 30.0));
        app.world_mut()
            .write_message(event(1, TouchPhase::Ended, center));
        app.update();
        assert_eq!(app.world().resource::<AudioSettings>().music, 0.25);
        app.world_mut()
            .write_message(event(2, TouchPhase::Started, center));
        app.world_mut()
            .write_message(event(2, TouchPhase::Ended, center + Vec2::X * 3.0));
        app.update();
        assert_eq!(app.world().resource::<AudioSettings>().music, 0.3);
        app.update();
        assert_eq!(app.world().resource::<AudioSettings>().music, 0.3);
        app.world_mut()
            .write_message(event(3, TouchPhase::Started, center));
        app.update();
        app.world_mut().resource_mut::<PauseMenuState>().open = false;
        app.world_mut()
            .write_message(event(3, TouchPhase::Ended, center));
        app.update();
        assert_eq!(app.world().resource::<AudioSettings>().music, 0.3);
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
    fn mobile_exit_requires_release_and_focus_loss_cancels_the_pending_tap() {
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
        assert_eq!(app.world().resource::<Messages<AppExit>>().len(), 1);
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
        app.init_resource::<ClientSession>()
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
