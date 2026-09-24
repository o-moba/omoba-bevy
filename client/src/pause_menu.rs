use bevy::{
    app::AppExit,
    input::mouse::{MouseScrollUnit, MouseWheel},
    input::touch::{TouchInput, TouchPhase},
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
use crate::world::{
    DEFAULT_AMBIENT_BRIGHTNESS, DEFAULT_LIGHT_ILLUMINANCE, DEFAULT_LIGHT_PITCH_DEG,
    DEFAULT_LIGHT_YAW_DEG, LightingSettings, MAX_AMBIENT_BRIGHTNESS, MAX_LIGHT_ILLUMINANCE,
    MAX_LIGHT_PITCH_DEG, MAX_LIGHT_YAW_DEG, MIN_AMBIENT_BRIGHTNESS, MIN_LIGHT_ILLUMINANCE,
    MIN_LIGHT_PITCH_DEG, MIN_LIGHT_YAW_DEG,
};

const OVERLAY_ALPHA: f32 = 0.7;
const PANEL_WIDTH: f32 = 480.0;
const PANEL_HEIGHT: f32 = 560.0;
const BUTTON_WIDTH: f32 = 320.0;
const BUTTON_HEIGHT: f32 = 46.0;
const ADJUST_BUTTON_SIZE: f32 = 44.0;
const SCALE_STEP: f32 = 0.04;
const ILLUMINANCE_STEP: f32 = 2_000.0;
const AMBIENT_STEP: f32 = 50.0;
const ANGLE_STEP_DEG: f32 = 5.0;
pub(crate) const BUTTON_COLOR: Color = crate::ui_theme::TILE;
pub(crate) const BUTTON_HOVER_COLOR: Color = crate::ui_theme::HOVER;

pub struct PauseMenuPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PauseMenuSet {
    Close,
    /// Mobile tap collection; button handlers run after it.
    Taps,
    Visuals,
}

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenuState>()
            .init_resource::<AudioSettings>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (toggle_pause_menu, close_pause_menu_when_disconnected)
                    .chain()
                    .in_set(PauseMenuSet::Close)
                    .after(crate::help_overlay::HelpOverlaySet::Input)
                    .after(crate::shop::ShopModalSet)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                Update,
                collect_pause_button_taps
                    .after(close_pause_menu_when_disconnected)
                    .in_set(PauseMenuSet::Taps)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                Update,
                (
                    handle_settings_navigation_buttons,
                    sync_pause_menu_visibility,
                    sync_pause_menu_sections,
                    handle_model_scale_buttons,
                    handle_camera_zoom_buttons,
                    update_camera_zoom_label.after(handle_camera_zoom_buttons),
                    handle_lighting_buttons,
                    handle_audio_buttons,
                    update_audio_labels.after(handle_audio_buttons),
                    update_model_scale_label,
                    update_lighting_labels,
                    handle_resume_button,
                    reset_pause_scroll_on_navigation
                        .after(handle_settings_navigation_buttons)
                        .after(handle_resume_button),
                    handle_reset_graphics_defaults_button,
                    handle_exit_button,
                    handle_leave_practice_button,
                    handle_controls_button,
                    sync_practice_actions,
                    sync_settings_server_addr_label,
                )
                    .after(collect_pause_button_taps)
                    .in_set(PauseMenuSet::Visuals),
            )
            .add_systems(
                PostUpdate,
                (scroll_desktop_settings, size_desktop_pause_panel)
                    .before(bevy::ui::UiSystems::Layout),
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
struct CloseButton;

#[derive(Component)]
struct SettingsSection;
#[derive(Component)]
struct SettingsFooter;

#[derive(Component)]
struct SettingsOpenButton;

#[derive(Component)]
struct SettingsBackButton;

#[derive(Component)]
struct ResetGraphicsDefaultsButton;

#[derive(Component)]
struct ExitButton;

#[derive(Component)]
struct LeavePracticeButton;

#[derive(Component)]
struct ResumeButton;

#[derive(Component)]
struct CameraZoomDecreaseButton;

#[derive(Component)]
struct CameraZoomIncreaseButton;

#[derive(Component)]
struct CameraZoomValueLabel;

#[derive(Component)]
struct ScaleDecreaseButton;

#[derive(Component)]
struct ScaleIncreaseButton;

#[derive(Component)]
struct ScaleValueLabel;

#[derive(Component)]
struct LightDecreaseButton;

#[derive(Component)]
struct LightIncreaseButton;

#[derive(Component)]
struct AmbientDecreaseButton;

#[derive(Component)]
struct AmbientIncreaseButton;

#[derive(Component)]
struct PitchDecreaseButton;

#[derive(Component)]
struct PitchIncreaseButton;

#[derive(Component)]
struct YawDecreaseButton;

#[derive(Component)]
struct YawIncreaseButton;

#[derive(Component)]
struct LightValueLabel;

#[derive(Component)]
struct AmbientValueLabel;

#[derive(Component)]
struct PitchValueLabel;

#[derive(Component)]
struct YawValueLabel;

#[derive(Component)]
struct SettingsServerAddrLabel;

#[derive(Component)]
struct PauseMenuPanel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AudioBus {
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

#[derive(Component, Clone, Copy)]
enum AudioButton {
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
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, OVERLAY_ALPHA)),
            Visibility::Hidden,
            ZIndex(100),
            PauseMenuRoot,
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
                    BackgroundColor(crate::ui_theme::PANEL.with_alpha(1.0)),
                    BorderColor::all(crate::ui_theme::EDGE),
                    PauseMenuPanel,
                    Name::new("PauseMenuPanel"),
                ))
                .with_children(|panel| {
                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                min_height: Val::Px(46.0),
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
                                crate::ui_theme::text(28.0),
                                TextColor(crate::ui_theme::IVORY),
                                Name::new("PauseMenuTitle"),
                            ));
                            header
                                .spawn((
                                    Button,
                                    CloseButton,
                                    PauseButtonGesture::default(),
                                    Node {
                                        width: Val::Px(46.0),
                                        height: Val::Px(46.0),
                                        flex_shrink: 0.0,
                                        justify_content: JustifyContent::Center,
                                        align_items: AlignItems::Center,
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(6.0)),
                                        ..default()
                                    },
                                    BackgroundColor(BUTTON_COLOR),
                                    BorderColor::all(crate::ui_theme::EDGE),
                                    Name::new("PauseMenuCloseButton"),
                                ))
                                .with_children(|button| {
                                    button.spawn((
                                        Text::new("×"),
                                        crate::ui_theme::text(28.0),
                                        TextColor(crate::ui_theme::IVORY),
                                    ));
                                });
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
                            ScrollPosition::default(),
                            crate::mobile_ui::TouchScrollPanel,
                            MainMenuSection,
                            Name::new("PauseMenuMainSection"),
                        ))
                        .with_children(|main| {
                            main.spawn((
                                Text::new("Your match continues while this menu is open."),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(crate::ui_theme::MUTED),
                                Name::new("PauseMenuMainTitle"),
                            ));

                            spawn_menu_button(
                                main,
                                "Settings",
                                SettingsOpenButton,
                                "SettingsButton",
                            );
                            crate::practice_sandbox::spawn_practice_open_button(main);
                            spawn_menu_button(
                                main,
                                "Controls guide",
                                crate::edge_hud::MatchHelpButton,
                                "PauseMenuHelpButton",
                            );
                            spawn_menu_button(main, "Exit game", ExitButton, "PauseMenuExitButton");
                            spawn_menu_button(
                                main,
                                "Leave practice",
                                LeavePracticeButton,
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
                            ScrollPosition::default(),
                            crate::mobile_ui::TouchScrollPanel,
                            SettingsSection,
                            Name::new("PauseMenuSettingsSection"),
                        ))
                        .with_children(|settings| {
                            settings.spawn((
                                Text::new("Settings"),
                                TextFont {
                                    font_size: 22.0,
                                    ..default()
                                },
                                TextColor(crate::ui_theme::IVORY),
                                Name::new("PauseMenuSettingsTitle"),
                            ));

                            settings.spawn((
                                Text::new("Sound"),
                                crate::ui_theme::text(18.0),
                                TextColor(crate::ui_theme::GOLD),
                                Name::new("PauseMenuAudioTitle"),
                            ));
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
                                spawn_adjust_row(
                                    settings,
                                    label,
                                    format!("{:.0}%", bus.value(AudioSettings::default()) * 100.0),
                                    AudioButton::Adjust(bus, -0.05),
                                    AudioLabel::Bus(bus),
                                    AudioButton::Adjust(bus, 0.05),
                                    name,
                                );
                            }
                            settings
                                .spawn((
                                    Button,
                                    PauseButtonGesture::default(),
                                    AudioButton::Mute,
                                    Node {
                                        width: Val::Px(BUTTON_WIDTH),
                                        min_height: Val::Px(BUTTON_HEIGHT),
                                        flex_shrink: 0.0,
                                        justify_content: JustifyContent::Center,
                                        align_items: AlignItems::Center,
                                        ..default()
                                    },
                                    BorderColor::all(crate::ui_theme::EDGE),
                                    BackgroundColor(BUTTON_COLOR),
                                    Name::new("PauseMenuAudioMuteButton"),
                                ))
                                .with_children(|button| {
                                    button.spawn((
                                        Text::new("Mute sound"),
                                        crate::ui_theme::text(20.0),
                                        TextColor(crate::ui_theme::IVORY),
                                        AudioLabel::Mute,
                                        Name::new("PauseMenuAudioMuteLabel"),
                                    ));
                                });

                            settings.spawn((
                                Text::new(""),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(crate::ui_theme::MUTED),
                                SettingsServerAddrLabel,
                                Name::new("PauseMenuServerAddrHint"),
                            ));

                            settings.spawn((
                                Text::new("Lighting"),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor(crate::ui_theme::GOLD),
                                Name::new("PauseMenuLightingTitle"),
                            ));

                            spawn_adjust_row(
                                settings,
                                "Main Light",
                                format!("{:.0}", DEFAULT_LIGHT_ILLUMINANCE),
                                LightDecreaseButton,
                                LightValueLabel,
                                LightIncreaseButton,
                                "PauseMenuMainLightControls",
                            );

                            spawn_adjust_row(
                                settings,
                                "Ambient",
                                format!("{:.0}", DEFAULT_AMBIENT_BRIGHTNESS),
                                AmbientDecreaseButton,
                                AmbientValueLabel,
                                AmbientIncreaseButton,
                                "PauseMenuAmbientControls",
                            );

                            spawn_adjust_row(
                                settings,
                                "Pitch",
                                format!("{:.0}°", DEFAULT_LIGHT_PITCH_DEG),
                                PitchDecreaseButton,
                                PitchValueLabel,
                                PitchIncreaseButton,
                                "PauseMenuPitchControls",
                            );

                            spawn_adjust_row(
                                settings,
                                "Yaw",
                                format!("{:.0}°", DEFAULT_LIGHT_YAW_DEG),
                                YawDecreaseButton,
                                YawValueLabel,
                                YawIncreaseButton,
                                "PauseMenuYawControls",
                            );

                            settings.spawn((
                                Text::new("Camera"),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor(crate::ui_theme::GOLD),
                                Name::new("PauseMenuCameraTitle"),
                            ));

                            // 100% is the default follow view; lower values bring
                            // the camera closer so hero silhouettes read larger.
                            spawn_adjust_row(
                                settings,
                                "Distance",
                                CameraSettings::default().percent_label(),
                                CameraZoomDecreaseButton,
                                CameraZoomValueLabel,
                                CameraZoomIncreaseButton,
                                "PauseMenuCameraZoomControls",
                            );

                            settings.spawn((
                                Text::new("Model"),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor(crate::ui_theme::GOLD),
                                Name::new("PauseMenuModelTitle"),
                            ));

                            spawn_adjust_row(
                                settings,
                                "Scale",
                                format!("{:.2}", DEFAULT_MODEL_TARGET_HEIGHT),
                                ScaleDecreaseButton,
                                ScaleValueLabel,
                                ScaleIncreaseButton,
                                "PauseMenuScaleControls",
                            );

                            spawn_menu_button(
                                settings,
                                "Reset graphics",
                                ResetGraphicsDefaultsButton,
                                "PauseMenuResetGraphicsButton",
                            );
                        });
                    panel
                        .spawn((
                            Node {
                                flex_shrink: 0.0,
                                min_height: Val::Px(BUTTON_HEIGHT),
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            MainMenuFooter,
                            Name::new("PauseMenuMainFooter"),
                        ))
                        .with_children(|footer| {
                            spawn_menu_button(
                                footer,
                                "Return to game",
                                ResumeButton,
                                "PauseMenuResumeButton",
                            );
                        });
                    panel
                        .spawn((
                            Node {
                                display: Display::None,
                                flex_shrink: 0.0,
                                min_height: Val::Px(46.0),
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            Visibility::Hidden,
                            SettingsFooter,
                            Name::new("PauseMenuSettingsFooter"),
                        ))
                        .with_children(|footer| {
                            spawn_menu_button(footer, "Back", SettingsBackButton, "BackButton")
                        });
                    crate::practice_sandbox::spawn_practice_section(panel);
                });
        });
}

pub(crate) fn spawn_menu_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    marker: M,
    name: &str,
) {
    parent
        .spawn((
            Button,
            PauseButtonGesture::default(),
            Node {
                width: Val::Px(BUTTON_WIDTH),
                height: Val::Px(BUTTON_HEIGHT),
                max_width: Val::Percent(100.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(crate::ui_theme::EDGE),
            BackgroundColor(if name == "PauseMenuResumeButton" {
                crate::frontend::widgets::PRIMARY
            } else {
                BUTTON_COLOR
            }),
            marker,
            Name::new(name.to_owned()),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(text),
                TextFont {
                    font_size: 17.0,
                    ..default()
                },
                TextColor(crate::ui_theme::IVORY),
            ));
        });
}

pub(crate) fn spawn_adjust_row<Dec: Component, ValueMarker: Component, Inc: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: String,
    decrease_marker: Dec,
    value_marker: ValueMarker,
    increase_marker: Inc,
    row_name: &str,
) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_shrink: 0.0,
                column_gap: Val::Px(10.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            Name::new(row_name.to_owned()),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                Node {
                    width: Val::Px(110.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(crate::ui_theme::IVORY),
            ));

            row.spawn((
                Button,
                PauseButtonGesture::default(),
                Node {
                    width: Val::Px(ADJUST_BUTTON_SIZE),
                    height: Val::Px(ADJUST_BUTTON_SIZE),
                    flex_shrink: 0.0,
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BorderColor::all(crate::ui_theme::EDGE),
                BackgroundColor(BUTTON_COLOR),
                decrease_marker,
                Name::new(format!("{row_name}-Down")),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new("-"),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(crate::ui_theme::IVORY),
                ));
            });

            row.spawn((
                Text::new(value),
                Node {
                    width: Val::Px(62.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                TextLayout::new_with_justify(Justify::Center),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(crate::ui_theme::IVORY),
                value_marker,
                Name::new(format!("{row_name}-Value")),
            ));

            row.spawn((
                Button,
                PauseButtonGesture::default(),
                Node {
                    width: Val::Px(ADJUST_BUTTON_SIZE),
                    height: Val::Px(ADJUST_BUTTON_SIZE),
                    flex_shrink: 0.0,
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BorderColor::all(crate::ui_theme::EDGE),
                BackgroundColor(BUTTON_COLOR),
                increase_marker,
                Name::new(format!("{row_name}-Up")),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new("+"),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(crate::ui_theme::IVORY),
                ));
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
    if client_session.state != ClientConnectionState::Disconnected
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
    practice: Option<Res<crate::practice_sandbox::PracticeSandboxState>>,
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

/// Scrollable mobile menus activate only on a short release within the same
/// visible button. Desktop mouse Interaction behavior remains unchanged.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PauseButtonGesture {
    touch_mode: bool,
    activated: bool,
}

impl PauseButtonGesture {
    pub(crate) fn effective(&self, interaction: Interaction) -> Interaction {
        if !self.touch_mode {
            return interaction;
        }
        if self.activated {
            Interaction::Pressed
        } else if interaction == Interaction::Pressed {
            Interaction::Hovered
        } else {
            interaction
        }
    }
}

#[derive(Default)]
pub(crate) struct PauseTapState {
    held: Option<PauseTap>,
    menu: Option<(bool, bool, Vec2)>,
}

struct PauseTap {
    id: u64,
    button: Entity,
    start: Vec2,
    canceled: bool,
}

impl PauseTapState {
    pub(crate) fn event(
        &mut self,
        id: u64,
        phase: TouchPhase,
        point: Vec2,
        buttons: &[(Entity, Rect)],
    ) -> Option<Entity> {
        if !point.is_finite() {
            self.held = None;
            return None;
        }
        if phase == TouchPhase::Started {
            if self.held.is_none() {
                if let Some((button, _)) = buttons.iter().find(|(_, rect)| rect.contains(point)) {
                    self.held = Some(PauseTap {
                        id,
                        button: *button,
                        start: point,
                        canceled: false,
                    });
                }
            }
            return None;
        }
        let tap = self.held.as_mut().filter(|tap| tap.id == id)?;
        // Sticky cancellation: scrolling away then back cannot revive a tap.
        tap.canceled |= tap.start.distance(point) > 10.0;
        let candidate = tap.button;
        let released = phase == TouchPhase::Ended
            && !tap.canceled
            && buttons
                .iter()
                .any(|(entity, rect)| *entity == candidate && rect.contains(point));
        if matches!(phase, TouchPhase::Ended | TouchPhase::Canceled) {
            self.held = None;
        }
        released.then_some(candidate)
    }
}

#[allow(clippy::type_complexity)]
fn collect_pause_button_taps(
    mut state: Local<PauseTapState>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    menu: Res<PauseMenuState>,
    server: Option<Res<crate::mobile_ui::ServerEntry>>,
    touches: Res<Touches>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut events: MessageReader<TouchInput>,
    lifecycle: Option<Res<Messages<bevy::window::AppLifecycle>>>,
    mut lifecycle_cursor: Local<bevy::ecs::message::MessageCursor<bevy::window::AppLifecycle>>,
    mut buttons: Query<(
        Entity,
        &mut PauseButtonGesture,
        Option<&ComputedNode>,
        Option<&UiGlobalTransform>,
        Option<&InheritedVisibility>,
        Option<&bevy::ui::CalculatedClip>,
    )>,
) {
    let touch_mode = mobile.as_ref().is_some_and(|m| m.enabled);
    for (_, mut gesture, _, _, _, _) in &mut buttons {
        gesture.set_if_neq(PauseButtonGesture {
            touch_mode,
            activated: false,
        });
    }
    let Some(mobile) = mobile.filter(|m| m.enabled && m.landscape && m.focused) else {
        state.held = None;
        events.clear();
        return;
    };
    let interrupted = lifecycle.as_ref().is_some_and(|events| {
        lifecycle_cursor.read(events).any(|event| {
            matches!(
                event,
                bevy::window::AppLifecycle::WillSuspend
                    | bevy::window::AppLifecycle::Suspended
                    | bevy::window::AppLifecycle::WillResume
            )
        })
    });
    if interrupted {
        state.held = None;
        events.clear();
        return;
    }
    let current_menu = (menu.open, menu.in_settings, mobile.viewport);
    if state.menu != Some(current_menu) {
        state.held = None;
        state.menu = Some(current_menu);
    }
    if !menu.open || server.as_ref().is_some_and(|entry| entry.open) {
        state.held = None;
        events.clear();
        return;
    }
    let visible: Vec<_> = buttons
        .iter()
        .filter_map(|(entity, _, node, transform, visibility, clip)| {
            if visibility.is_some_and(|v| !v.get()) {
                return None;
            }
            let (Some(node), Some(transform)) = (node, transform) else {
                return None;
            };
            let factor = 1.0 / window.single().map_or(1.0, |(_, w)| w.scale_factor());
            let size = node.size() * transform.to_scale_angle_translation().0.abs() * factor;
            if size.min_element() <= 0.0 {
                return None;
            }
            let mut rect = Rect::from_center_size(transform.translation * factor, size);
            if let Some(clip) = clip {
                rect = rect.intersect(Rect::from_corners(
                    clip.clip.min * factor,
                    clip.clip.max * factor,
                ));
            }
            (rect.width() > 0.0 && rect.height() > 0.0).then_some((entity, rect))
        })
        .collect();
    let Ok((window_entity, window)) = window.single() else {
        state.held = None;
        events.clear();
        return;
    };
    let mut activated = Vec::new();
    for event in events.read() {
        if event.window == window_entity {
            if let Some(entity) = state.event(event.id, event.phase, event.position, &visible) {
                activated.push(entity);
            }
        }
    }
    // Mouse QA follows the same release rule. Native touch devices ignore
    // synthesized mouse events so a physical release cannot activate twice.
    if !cfg!(any(target_os = "android", target_os = "ios"))
        && touches.iter().next().is_none()
        && !touches.any_just_released()
        && !touches.any_just_canceled()
    {
        if let Some(point) = window.cursor_position() {
            let phase = if mouse.just_pressed(MouseButton::Left) {
                Some(TouchPhase::Started)
            } else if mouse.just_released(MouseButton::Left) {
                Some(TouchPhase::Ended)
            } else if mouse.pressed(MouseButton::Left) {
                Some(TouchPhase::Moved)
            } else {
                None
            };
            if let Some(phase) = phase {
                if let Some(entity) = state.event(u64::MAX, phase, point, &visible) {
                    activated.push(entity);
                }
            }
        }
    }
    for entity in activated {
        if let Ok((_, mut gesture, _, _, _, _)) = buttons.get_mut(entity) {
            gesture.activated = true;
        }
    }
}

fn handle_settings_navigation_buttons(
    mut menu_state: ResMut<PauseMenuState>,
    mut button_query: Query<
        (
            &Interaction,
            &PauseButtonGesture,
            Option<&SettingsOpenButton>,
            Option<&SettingsBackButton>,
            &mut BackgroundColor,
        ),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
        ),
    >,
) {
    for (interaction, gesture, open_button, back_button, mut color) in &mut button_query {
        if open_button.is_none() && back_button.is_none() {
            continue;
        }

        match gesture.effective(*interaction) {
            Interaction::Pressed => {
                if open_button.is_some() {
                    menu_state.in_settings = true;
                }
                if back_button.is_some() {
                    menu_state.in_settings = false;
                }
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::Hovered => {
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = BUTTON_COLOR.into();
            }
        }
    }
}

fn handle_model_scale_buttons(
    mut scale_settings: ResMut<ModelScaleSettings>,
    mut button_query: Query<
        (
            &Interaction,
            &PauseButtonGesture,
            Option<&ScaleDecreaseButton>,
            Option<&ScaleIncreaseButton>,
            &mut BackgroundColor,
        ),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
        ),
    >,
) {
    for (interaction, gesture, is_down, is_up, mut color) in &mut button_query {
        if is_down.is_none() && is_up.is_none() {
            continue;
        }

        match gesture.effective(*interaction) {
            Interaction::Pressed => {
                if is_down.is_some() {
                    scale_settings.target_height =
                        (scale_settings.target_height - SCALE_STEP).max(MIN_MODEL_TARGET_HEIGHT);
                } else if is_up.is_some() {
                    scale_settings.target_height =
                        (scale_settings.target_height + SCALE_STEP).min(MAX_MODEL_TARGET_HEIGHT);
                }
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::Hovered => {
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = BUTTON_COLOR.into();
            }
        }
    }
}

fn handle_camera_zoom_buttons(
    mut camera_settings: ResMut<CameraSettings>,
    mut button_query: Query<
        (
            &Interaction,
            &PauseButtonGesture,
            Option<&CameraZoomDecreaseButton>,
            Option<&CameraZoomIncreaseButton>,
            &mut BackgroundColor,
        ),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
        ),
    >,
) {
    for (interaction, gesture, is_closer, is_farther, mut color) in &mut button_query {
        if is_closer.is_none() && is_farther.is_none() {
            continue;
        }

        match gesture.effective(*interaction) {
            Interaction::Pressed => {
                if is_closer.is_some() {
                    camera_settings.adjust(-CAMERA_ZOOM_STEP);
                } else {
                    camera_settings.adjust(CAMERA_ZOOM_STEP);
                }
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::Hovered => {
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = BUTTON_COLOR.into();
            }
        }
    }
}

/// Also follows wheel zoom, since the camera writes it back into the setting.
fn update_camera_zoom_label(
    camera_settings: Res<CameraSettings>,
    mut label_query: Query<&mut Text, With<CameraZoomValueLabel>>,
) {
    if !camera_settings.is_changed() {
        return;
    }

    if let Ok(mut text) = label_query.single_mut() {
        text.0 = camera_settings.percent_label();
    }
}

fn handle_lighting_buttons(
    mut lighting_settings: ResMut<LightingSettings>,
    mut button_query: Query<
        (
            &Interaction,
            &PauseButtonGesture,
            Option<&LightDecreaseButton>,
            Option<&LightIncreaseButton>,
            Option<&AmbientDecreaseButton>,
            Option<&AmbientIncreaseButton>,
            Option<&PitchDecreaseButton>,
            Option<&PitchIncreaseButton>,
            Option<&YawDecreaseButton>,
            Option<&YawIncreaseButton>,
            &mut BackgroundColor,
        ),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
        ),
    >,
) {
    for (
        interaction,
        gesture,
        light_down,
        light_up,
        ambient_down,
        ambient_up,
        pitch_down,
        pitch_up,
        yaw_down,
        yaw_up,
        mut color,
    ) in &mut button_query
    {
        let is_lighting_button = light_down.is_some()
            || light_up.is_some()
            || ambient_down.is_some()
            || ambient_up.is_some()
            || pitch_down.is_some()
            || pitch_up.is_some()
            || yaw_down.is_some()
            || yaw_up.is_some();

        if !is_lighting_button {
            continue;
        }

        match gesture.effective(*interaction) {
            Interaction::Pressed => {
                if light_down.is_some() {
                    lighting_settings.illuminance = (lighting_settings.illuminance
                        - ILLUMINANCE_STEP)
                        .max(MIN_LIGHT_ILLUMINANCE);
                } else if light_up.is_some() {
                    lighting_settings.illuminance = (lighting_settings.illuminance
                        + ILLUMINANCE_STEP)
                        .min(MAX_LIGHT_ILLUMINANCE);
                } else if ambient_down.is_some() {
                    lighting_settings.ambient_brightness = (lighting_settings.ambient_brightness
                        - AMBIENT_STEP)
                        .max(MIN_AMBIENT_BRIGHTNESS);
                } else if ambient_up.is_some() {
                    lighting_settings.ambient_brightness = (lighting_settings.ambient_brightness
                        + AMBIENT_STEP)
                        .min(MAX_AMBIENT_BRIGHTNESS);
                } else if pitch_down.is_some() {
                    lighting_settings.light_pitch_deg = (lighting_settings.light_pitch_deg
                        - ANGLE_STEP_DEG)
                        .max(MIN_LIGHT_PITCH_DEG);
                } else if pitch_up.is_some() {
                    lighting_settings.light_pitch_deg = (lighting_settings.light_pitch_deg
                        + ANGLE_STEP_DEG)
                        .min(MAX_LIGHT_PITCH_DEG);
                } else if yaw_down.is_some() {
                    lighting_settings.light_yaw_deg =
                        (lighting_settings.light_yaw_deg - ANGLE_STEP_DEG).max(MIN_LIGHT_YAW_DEG);
                } else if yaw_up.is_some() {
                    lighting_settings.light_yaw_deg =
                        (lighting_settings.light_yaw_deg + ANGLE_STEP_DEG).min(MAX_LIGHT_YAW_DEG);
                }
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::Hovered => {
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = BUTTON_COLOR.into();
            }
        }
    }
}

fn handle_audio_buttons(
    menu: Res<PauseMenuState>,
    career: Option<Res<crate::career::CareerClient>>,
    social: Option<Res<crate::social::SocialClient>>,
    mut settings: ResMut<AudioSettings>,
    mut buttons: Query<
        (
            &Interaction,
            &PauseButtonGesture,
            &AudioButton,
            &mut BackgroundColor,
        ),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
        ),
    >,
) {
    if !menu.open
        || !menu.in_settings
        || career.as_ref().is_some_and(|career| career.modal_open())
        || social
            .as_ref()
            .is_some_and(|social| social.blocks_gameplay())
    {
        return;
    }
    for (interaction, gesture, action, mut color) in &mut buttons {
        match gesture.effective(*interaction) {
            Interaction::Pressed => {
                match *action {
                    AudioButton::Adjust(bus, delta) => bus.adjust(&mut settings, delta),
                    AudioButton::Mute => settings.muted = !settings.muted,
                }
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::Hovered => *color = BUTTON_HOVER_COLOR.into(),
            Interaction::None => *color = BUTTON_COLOR.into(),
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

fn scroll_desktop_settings(
    menu: Res<PauseMenuState>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut wheel: MessageReader<MouseWheel>,
    mut panels: Query<
        (&ComputedNode, &mut ScrollPosition, Has<SettingsSection>),
        Or<(With<SettingsSection>, With<MainMenuSection>)>,
    >,
) {
    let delta: f32 = wheel
        .read()
        .map(|event| {
            event.y
                * if event.unit == MouseScrollUnit::Line {
                    32.0
                } else {
                    1.0
                }
        })
        .sum();
    if !menu.open || mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
        return;
    }
    for (node, mut scroll, settings) in &mut panels {
        if settings != menu.in_settings {
            continue;
        }
        let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0);
        scroll.y = (scroll.y - delta).clamp(0.0, max);
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

fn update_model_scale_label(
    scale_settings: Res<ModelScaleSettings>,
    mut label_query: Query<&mut Text, With<ScaleValueLabel>>,
) {
    if !scale_settings.is_changed() {
        return;
    }

    if let Ok(mut text) = label_query.single_mut() {
        text.0 = format!("{:.2}", scale_settings.target_height);
    }
}

fn update_lighting_labels(
    lighting_settings: Res<LightingSettings>,
    mut label_queries: ParamSet<(
        Query<&mut Text, With<LightValueLabel>>,
        Query<&mut Text, With<AmbientValueLabel>>,
        Query<&mut Text, With<PitchValueLabel>>,
        Query<&mut Text, With<YawValueLabel>>,
    )>,
) {
    if !lighting_settings.is_changed() {
        return;
    }

    if let Ok(mut text) = label_queries.p0().single_mut() {
        text.0 = format!("{:.0}", lighting_settings.illuminance);
    }
    if let Ok(mut text) = label_queries.p1().single_mut() {
        text.0 = format!("{:.0}", lighting_settings.ambient_brightness);
    }
    if let Ok(mut text) = label_queries.p2().single_mut() {
        text.0 = format!("{:.0}°", lighting_settings.light_pitch_deg);
    }
    if let Ok(mut text) = label_queries.p3().single_mut() {
        text.0 = format!("{:.0}°", lighting_settings.light_yaw_deg);
    }
}

fn sync_settings_server_addr_label(
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    mut label_q: Query<&mut Text, With<SettingsServerAddrLabel>>,
) {
    let addr = {
        let s = resolved_addr.0.as_str().trim();
        if s.is_empty() {
            DEFAULT_GAME_SERVER_ADDR
        } else {
            s
        }
    };
    let next = format!("Server: {addr}\nSettings are saved automatically.");
    if let Ok(mut text) = label_q.single_mut() {
        if text.0 != next {
            text.0 = next;
        }
    }
}

fn handle_reset_graphics_defaults_button(
    mut lighting: ResMut<LightingSettings>,
    mut model: ResMut<ModelScaleSettings>,
    mut camera: ResMut<CameraSettings>,
    mut prefs_gate: ResMut<ClientPrefsSaveGate>,
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    client_session_id: Res<ClientSessionId>,
    team: Res<TeamSelection>,
    audio: Res<AudioSettings>,
    mut button_query: Query<
        (&Interaction, &PauseButtonGesture, &mut BackgroundColor),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
            With<ResetGraphicsDefaultsButton>,
        ),
    >,
) {
    let addr = {
        let s = resolved_addr.0.as_str().trim();
        if s.is_empty() {
            DEFAULT_GAME_SERVER_ADDR
        } else {
            s
        }
    };

    for (interaction, gesture, mut color) in &mut button_query {
        match gesture.effective(*interaction) {
            Interaction::Pressed => {
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
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::Hovered => {
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = BUTTON_COLOR.into();
            }
        }
    }
}

fn sync_practice_actions(
    session: Res<ClientSession>,
    mut buttons: Query<
        (&mut Node, Has<LeavePracticeButton>),
        Or<(With<ExitButton>, With<LeavePracticeButton>)>,
    >,
    mut hints: Query<(&Name, &mut Text)>,
) {
    let offline = session.is_offline();
    for (mut node, leave_practice) in &mut buttons {
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

fn handle_controls_button(
    mut menu: ResMut<PauseMenuState>,
    mut help: ResMut<crate::help_overlay::HelpOverlayVisible>,
    mut buttons: Query<
        (&Interaction, &PauseButtonGesture, &mut BackgroundColor),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<crate::edge_hud::MatchHelpButton>,
        ),
    >,
) {
    for (interaction, gesture, mut color) in &mut buttons {
        match gesture.effective(*interaction) {
            Interaction::Pressed if menu.open => {
                menu.open = false;
                menu.in_settings = false;
                help.0 = true;
            }
            Interaction::Hovered => *color = BUTTON_HOVER_COLOR.into(),
            _ => *color = BUTTON_COLOR.into(),
        }
    }
}

fn handle_leave_practice_button(
    session: Res<ClientSession>,
    mut menu: ResMut<PauseMenuState>,
    mut commands: MessageWriter<crate::net::SessionUiCommand>,
    mut buttons: Query<
        (&Interaction, &PauseButtonGesture, &mut BackgroundColor),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<LeavePracticeButton>,
        ),
    >,
) {
    for (interaction, gesture, mut color) in &mut buttons {
        match gesture.effective(*interaction) {
            Interaction::Pressed if session.is_offline() => {
                commands.write(crate::net::SessionUiCommand::LeaveMatch);
                menu.open = false;
                menu.in_settings = false;
            }
            Interaction::Hovered => *color = BUTTON_HOVER_COLOR.into(),
            _ => *color = BUTTON_COLOR.into(),
        }
    }
}

fn handle_exit_button(
    mut commands: Commands,
    mut interaction_query: Query<
        (&Interaction, &PauseButtonGesture, &mut BackgroundColor),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
            With<ExitButton>,
        ),
    >,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    window_query: Query<Entity, With<PrimaryWindow>>,
    mut app_exit_writer: MessageWriter<AppExit>,
) {
    for (interaction, gesture, mut color) in &mut interaction_query {
        match gesture.effective(*interaction) {
            Interaction::Pressed => {
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
            Interaction::Hovered => {
                *color = BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = BUTTON_COLOR.into();
            }
        }
    }
}

fn handle_resume_button(
    mut menu_state: ResMut<PauseMenuState>,
    mut buttons: Query<
        (&Interaction, &PauseButtonGesture, &mut BackgroundColor),
        (
            Or<(Changed<Interaction>, Changed<PauseButtonGesture>)>,
            With<Button>,
            Or<(With<ResumeButton>, With<CloseButton>)>,
        ),
    >,
) {
    for (interaction, gesture, mut color) in &mut buttons {
        match gesture.effective(*interaction) {
            Interaction::Pressed => {
                menu_state.open = false;
                menu_state.in_settings = false;
            }
            Interaction::Hovered => *color = crate::frontend::widgets::PRIMARY_HOVER.into(),
            Interaction::None => *color = crate::frontend::widgets::PRIMARY.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: true,
            })
            .init_resource::<AudioSettings>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    collect_pause_button_taps,
                    handle_audio_buttons,
                    handle_resume_button,
                    handle_controls_button,
                    sync_pause_menu_visibility,
                    sync_pause_menu_sections,
                    reset_pause_scroll_on_navigation,
                    sync_practice_actions,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                (size_desktop_pause_panel, scroll_desktop_settings)
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
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find(|(_, n)| n.as_str() == name)
            .unwrap()
            .0
    }

    fn rect(app: &App, entity: Entity, dpi: f32) -> Rect {
        crate::mobile_ui::logical_ui_rect(
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
                *app.world_mut().get_mut::<Interaction>(close).unwrap() = Interaction::Pressed;
                use bevy::ecs::system::RunSystemOnce;
                app.world_mut()
                    .run_system_once(handle_resume_button)
                    .unwrap();
                assert!(!app.world().resource::<PauseMenuState>().open);
                *app.world_mut().get_mut::<Interaction>(close).unwrap() = Interaction::None;
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
            session.state = ClientConnectionState::Disconnected;
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
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(Update, (handle_audio_buttons, update_audio_labels).chain());
        app.update();
        let music = app.world_mut().query::<(Entity, &AudioButton)>().iter(app.world())
            .find(|(_, button)| matches!(button, AudioButton::Adjust(AudioBus::Music, delta) if *delta < 0.0)).unwrap().0;
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
        let mute = app
            .world_mut()
            .query::<(Entity, &AudioButton)>()
            .iter(app.world())
            .find(|(_, action)| matches!(action, AudioButton::Mute))
            .unwrap()
            .0;
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
            .init_resource::<AudioSettings>()
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: true,
            })
            .init_resource::<Touches>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<TouchInput>()
            .add_systems(
                Update,
                (collect_pause_button_taps, handle_audio_buttons).chain(),
            );
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut().spawn((
            Button,
            Node::default(),
            AudioButton::Adjust(AudioBus::Music, 0.05),
            PauseButtonGesture::default(),
            Interaction::Pressed,
            BackgroundColor(BUTTON_COLOR),
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
        let back = app
            .world_mut()
            .query_filtered::<Entity, With<SettingsBackButton>>()
            .single(app.world())
            .unwrap();
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
    }

    #[test]
    fn desktop_settings_scroll_is_clamped_and_back_navigation_resets_it() {
        let mut app = App::new();
        app.insert_resource(PauseMenuState {
            open: true,
            in_settings: true,
        })
        .add_message::<MouseWheel>()
        .add_systems(
            Update,
            (reset_pause_scroll_on_navigation, scroll_desktop_settings).chain(),
        );
        let panel = app
            .world_mut()
            .spawn((
                SettingsSection,
                ScrollPosition::default(),
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
            y: -50.0,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        assert_eq!(app.world().get::<ScrollPosition>(panel).unwrap().y, 600.0);
        app.world_mut().resource_mut::<PauseMenuState>().in_settings = false;
        app.update();
        assert_eq!(app.world().get::<ScrollPosition>(panel).unwrap().y, 0.0);
    }

    #[test]
    fn pause_tap_cancels_a_scroll_even_after_returning_to_the_button() {
        let entity = Entity::PLACEHOLDER;
        let rect = Rect::from_center_size(Vec2::new(100.0, 100.0), Vec2::new(180.0, 46.0));
        let buttons = [(entity, rect)];
        let mut state = PauseTapState::default();
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
            .insert_resource(PauseMenuState {
                open: true,
                in_settings: false,
            })
            .init_resource::<Touches>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<TouchInput>()
            .add_message::<AppExit>()
            .add_systems(
                Update,
                (collect_pause_button_taps, handle_exit_button).chain(),
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
                ExitButton,
                PauseButtonGesture::default(),
                Interaction::Pressed,
                BackgroundColor(BUTTON_COLOR),
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
        .add_systems(Startup, setup_pause_menu_ui)
        .add_systems(
            Update,
            (handle_resume_button, sync_pause_menu_visibility).chain(),
        );
        app.world_mut().resource_mut::<TeamSelection>().team = Some(crate::team::Team::Green);
        let player = app.world_mut().spawn(crate::player::Player).id();
        app.update();
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<ResumeButton>>()
            .single(app.world())
            .unwrap();
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
        app.add_message::<AppExit>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(Update, handle_exit_button);
        app.update();
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<ExitButton>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
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
