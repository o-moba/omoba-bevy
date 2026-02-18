use bevy::{
    app::AppExit,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use crate::player::Player;
use crate::team::{TeamSelectRoot, TeamSelection, spawn_team_select_ui};
use crate::world::{
    DEFAULT_AMBIENT_BRIGHTNESS, DEFAULT_LIGHT_ILLUMINANCE, DEFAULT_LIGHT_PITCH_DEG,
    DEFAULT_LIGHT_YAW_DEG, DEFAULT_MODEL_TARGET_HEIGHT, LightingSettings, MAX_AMBIENT_BRIGHTNESS,
    MAX_LIGHT_ILLUMINANCE, MAX_LIGHT_PITCH_DEG, MAX_LIGHT_YAW_DEG, MAX_MODEL_TARGET_HEIGHT,
    MIN_AMBIENT_BRIGHTNESS, MIN_LIGHT_ILLUMINANCE, MIN_LIGHT_PITCH_DEG, MIN_LIGHT_YAW_DEG,
    MIN_MODEL_TARGET_HEIGHT, ModelScaleSettings,
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
const BUTTON_COLOR: Color = Color::srgb(0.25, 0.25, 0.25);
const BUTTON_HOVER_COLOR: Color = Color::srgb(0.35, 0.35, 0.35);

pub struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenuState>()
            .init_resource::<RestartRequest>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    toggle_pause_menu,
                    handle_settings_navigation_buttons,
                    sync_pause_menu_visibility,
                    sync_pause_menu_sections,
                    handle_model_scale_buttons,
                    handle_lighting_buttons,
                    update_model_scale_label,
                    update_lighting_labels,
                    handle_restart_button_request,
                    handle_exit_button,
                ),
            )
            .add_systems(Last, process_restart_request);
    }
}

#[derive(Resource, Default)]
struct PauseMenuState {
    open: bool,
    in_settings: bool,
}

#[derive(Resource, Default)]
struct RestartRequest {
    pending: bool,
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
struct ExitButton;

#[derive(Component)]
struct RestartButton;

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
                    BackgroundColor(Color::srgb(0.12, 0.12, 0.12)),
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
                                Text::new("Pause"),
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
                                "Restart",
                                RestartButton,
                                "PauseMenuRestartButton",
                            );
                            spawn_menu_button(main, "Exit", ExitButton, "PauseMenuExitButton");
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

fn toggle_pause_menu(
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

fn handle_settings_navigation_buttons(
    mut menu_state: ResMut<PauseMenuState>,
    mut button_query: Query<
        (
            &Interaction,
            Option<&SettingsOpenButton>,
            Option<&SettingsBackButton>,
            &mut BackgroundColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
) {
    for (interaction, open_button, back_button, mut color) in &mut button_query {
        if open_button.is_none() && back_button.is_none() {
            continue;
        }

        match *interaction {
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
            Option<&ScaleDecreaseButton>,
            Option<&ScaleIncreaseButton>,
            &mut BackgroundColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
) {
    for (interaction, is_down, is_up, mut color) in &mut button_query {
        if is_down.is_none() && is_up.is_none() {
            continue;
        }

        match *interaction {
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
        (Changed<Interaction>, With<Button>),
    >,
) {
    for (
        interaction,
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

        match *interaction {
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

fn handle_exit_button(
    mut commands: Commands,
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<ExitButton>),
    >,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    window_query: Query<Entity, With<PrimaryWindow>>,
    mut app_exit_writer: MessageWriter<AppExit>,
) {
    for (interaction, mut color) in &mut interaction_query {
        match *interaction {
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

fn handle_restart_button_request(
    mut restart_request: ResMut<RestartRequest>,
    mut restart_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<RestartButton>),
    >,
) {
    for (interaction, mut color) in &mut restart_query {
        match *interaction {
            Interaction::Pressed => {
                info!("Restart selected from pause menu.");
                restart_request.pending = true;
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

fn process_restart_request(
    mut commands: Commands,
    mut menu_state: ResMut<PauseMenuState>,
    mut team_selection: ResMut<TeamSelection>,
    mut restart_request: ResMut<RestartRequest>,
    local_players: Query<Entity, With<Player>>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if !restart_request.pending {
        return;
    }
    restart_request.pending = false;

    menu_state.open = false;
    menu_state.in_settings = false;
    team_selection.team = None;

    if let Ok(mut cursor) = cursor_query.single_mut() {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }

    for entity in &local_players {
        commands
            .entity(entity)
            .despawn_related::<Children>()
            .despawn();
    }

    if overlay_query.single().is_err() {
        spawn_team_select_ui(&mut commands, team_selection.character);
    }
}
