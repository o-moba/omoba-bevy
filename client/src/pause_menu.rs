use bevy::{
    app::AppExit,
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

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
const PANEL_WIDTH: f32 = 420.0;
const PANEL_HEIGHT: f32 = 560.0;
const BUTTON_WIDTH: f32 = 180.0;
const BUTTON_HEIGHT: f32 = 46.0;
const ADJUST_BUTTON_SIZE: f32 = 40.0;
const SCALE_STEP: f32 = 0.04;
const ILLUMINANCE_STEP: f32 = 2_000.0;
const AMBIENT_STEP: f32 = 50.0;
const ANGLE_STEP_DEG: f32 = 5.0;
const BUTTON_COLOR: Color = crate::ui_theme::TILE;
const BUTTON_HOVER_COLOR: Color = crate::ui_theme::HOVER;

pub struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenuState>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (toggle_pause_menu, close_pause_menu_when_disconnected)
                    .chain()
                    .after(crate::help_overlay::HelpOverlaySet::Input)
                    .after(crate::shop::ShopModalSet)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                Update,
                collect_pause_button_taps
                    .after(close_pause_menu_when_disconnected)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                Update,
                (
                    handle_settings_navigation_buttons,
                    sync_pause_menu_visibility,
                    sync_pause_menu_sections,
                    handle_model_scale_buttons,
                    handle_lighting_buttons,
                    update_model_scale_label,
                    update_lighting_labels,
                    handle_resume_button,
                    handle_reset_graphics_defaults_button,
                    handle_exit_button,
                    sync_settings_server_addr_label,
                )
                    .after(collect_pause_button_taps),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct PauseMenuState {
    pub(crate) open: bool,
    in_settings: bool,
}

#[derive(Component)]
struct PauseMenuRoot;

#[derive(Component)]
struct MainMenuSection;

#[derive(Component)]
struct SettingsSection;

#[derive(Component)]
struct SettingsOpenButton;

#[derive(Component)]
struct SettingsBackButton;

#[derive(Component)]
struct ResetGraphicsDefaultsButton;

#[derive(Component)]
struct ExitButton;

#[derive(Component)]
struct ResumeButton;

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
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::FlexStart,
                        align_items: AlignItems::Stretch,
                        row_gap: Val::Px(16.0),
                        padding: UiRect::all(Val::Px(20.0)),
                        ..default()
                    },
                    BackgroundColor(crate::ui_theme::PANEL),
                    Name::new("PauseMenuPanel"),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Menu"),
                        TextFont {
                            font_size: 32.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Name::new("PauseMenuTitle"),
                    ));

                    panel
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(14.0),
                                display: Display::Flex,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::FlexStart,
                                ..default()
                            },
                            MainMenuSection,
                            Name::new("PauseMenuMainSection"),
                        ))
                        .with_children(|main| {
                            main.spawn((
                                Text::new("The online match continues"),
                                TextFont {
                                    font_size: 20.0,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                                Name::new("PauseMenuMainTitle"),
                            ));

                            spawn_menu_button(
                                main,
                                "Settings",
                                SettingsOpenButton,
                                "SettingsButton",
                            );
                            spawn_menu_button(
                                main,
                                "Resume match",
                                ResumeButton,
                                "PauseMenuResumeButton",
                            );
                            spawn_menu_button(main, "Exit game", ExitButton, "PauseMenuExitButton");
                        });

                    panel
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(12.0),
                                display: Display::None,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::FlexStart,
                                ..default()
                            },
                            Visibility::Hidden,
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
                                TextColor(Color::WHITE),
                                Name::new("PauseMenuSettingsTitle"),
                            ));

                            settings.spawn((
                                Text::new(""),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.75, 0.78, 0.85)),
                                SettingsServerAddrLabel,
                                Name::new("PauseMenuServerAddrHint"),
                            ));

                            settings.spawn((
                                Text::new("Lighting"),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.88, 0.88, 0.88)),
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
                                Text::new("Model"),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.88, 0.88, 0.88)),
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
                                "Reset graphics to defaults",
                                ResetGraphicsDefaultsButton,
                                "PauseMenuResetGraphicsButton",
                            );

                            spawn_menu_button(settings, "Back", SettingsBackButton, "BackButton");
                        });
                });
        });
}

fn spawn_menu_button<M: Component>(
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
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BUTTON_COLOR),
            marker,
            Name::new(name.to_owned()),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(text),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

fn spawn_adjust_row<Dec: Component, ValueMarker: Component, Inc: Component>(
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
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));

            row.spawn((
                Button,
                PauseButtonGesture::default(),
                Node {
                    width: Val::Px(ADJUST_BUTTON_SIZE),
                    height: Val::Px(ADJUST_BUTTON_SIZE),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
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
                    TextColor(Color::WHITE),
                ));
            });

            row.spawn((
                Text::new(value),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                value_marker,
                Name::new(format!("{row_name}-Value")),
            ));

            row.spawn((
                Button,
                PauseButtonGesture::default(),
                Node {
                    width: Val::Px(ADJUST_BUTTON_SIZE),
                    height: Val::Px(ADJUST_BUTTON_SIZE),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
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
                    TextColor(Color::WHITE),
                ));
            });
        });
}

/// Match-assuming UI must not stay open without an active session (TASK-14 P5 / AC6).
fn close_pause_menu_when_disconnected(
    client_session: Res<ClientSession>,
    mut menu_state: ResMut<PauseMenuState>,
) {
    if client_session.state != ClientConnectionState::Disconnected {
        return;
    }
    if menu_state.open {
        menu_state.open = false;
        menu_state.in_settings = false;
    }
}

pub(crate) fn toggle_pause_menu(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut menu_state: ResMut<PauseMenuState>,
) {
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
    mut section_queries: ParamSet<(
        Query<(&mut Visibility, &mut Node), With<MainMenuSection>>,
        Query<(&mut Visibility, &mut Node), With<SettingsSection>>,
    )>,
) {
    if !menu_state.is_changed() {
        return;
    }

    if let Ok((mut main_visibility, mut main_node)) = section_queries.p0().single_mut() {
        *main_visibility = if menu_state.in_settings {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        main_node.display = if menu_state.in_settings {
            Display::None
        } else {
            Display::Flex
        };
    }

    if let Ok((mut settings_visibility, mut settings_node)) = section_queries.p1().single_mut() {
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
struct PauseButtonGesture {
    touch_mode: bool,
    activated: bool,
}

impl PauseButtonGesture {
    fn effective(&self, interaction: Interaction) -> Interaction {
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
struct PauseTapState {
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
    fn event(
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
            let factor = node.inverse_scale_factor();
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
    let next = format!(
        "Game server: {addr}\n\
         Saved with preferences when you change graphics or character. \
         Next launch: set GAME_SERVER_ADDR or edit client_preferences.json (see persistence docs)."
    );
    if let Ok(mut text) = label_q.single_mut() {
        if text.0 != next {
            text.0 = next;
        }
    }
}

fn handle_reset_graphics_defaults_button(
    mut lighting: ResMut<LightingSettings>,
    mut model: ResMut<ModelScaleSettings>,
    mut prefs_gate: ResMut<ClientPrefsSaveGate>,
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    client_session_id: Res<ClientSessionId>,
    team: Res<TeamSelection>,
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
                    prefs_gate.as_mut(),
                    team.character,
                    addr,
                    client_session_id.0.as_str(),
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
            With<ResumeButton>,
        ),
    >,
) {
    for (interaction, gesture, mut color) in &mut buttons {
        match gesture.effective(*interaction) {
            Interaction::Pressed => {
                menu_state.open = false;
                menu_state.in_settings = false;
            }
            Interaction::Hovered => *color = BUTTON_HOVER_COLOR.into(),
            Interaction::None => *color = BUTTON_COLOR.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
