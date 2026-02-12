use bevy::{
    app::AppExit,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use crate::player::Player;
use crate::team::{spawn_team_select_ui, TeamSelectRoot, TeamSelection};
use crate::world::{
    ModelScaleSettings, DEFAULT_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT,
    MIN_MODEL_TARGET_HEIGHT,
};

const OVERLAY_ALPHA: f32 = 0.7;
const PANEL_WIDTH: f32 = 320.0;
const PANEL_HEIGHT: f32 = 320.0;
const BUTTON_WIDTH: f32 = 180.0;
const BUTTON_HEIGHT: f32 = 46.0;
const SCALE_STEP: f32 = 0.04;

pub struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenuState>()
            .add_systems(Startup, setup_pause_menu_ui)
            .add_systems(
                Update,
                (
                    toggle_pause_menu,
                    handle_model_scale_buttons,
                    update_model_scale_label,
                    handle_restart_button,
                    handle_exit_button,
                    sync_pause_menu_visibility,
                ),
            );
    }
}

#[derive(Resource, Default)]
struct PauseMenuState {
    open: bool,
}

#[derive(Component)]
struct PauseMenuRoot;

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

fn setup_pause_menu_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
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
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
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

                    panel.spawn((
                        Text::new("Model Scale"),
                        TextFont {
                            font_size: 20.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Name::new("PauseMenuScaleTitle"),
                    ));

                    panel
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Row,
                                column_gap: Val::Px(12.0),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            Name::new("PauseMenuScaleControls"),
                        ))
                        .with_children(|row| {
                            row.spawn((
                                Button,
                                Node {
                                    width: Val::Px(44.0),
                                    height: Val::Px(40.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.25, 0.25, 0.25)),
                                ScaleDecreaseButton,
                                Name::new("ScaleDownButton"),
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("-"),
                                    TextFont {
                                        font_size: 24.0,
                                        ..default()
                                    },
                                    TextColor(Color::WHITE),
                                ));
                            });

                            row.spawn((
                                Text::new(format!("{:.2}", DEFAULT_MODEL_TARGET_HEIGHT)),
                                TextFont {
                                    font_size: 20.0,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                                ScaleValueLabel,
                                Name::new("ScaleValueLabel"),
                            ));

                            row.spawn((
                                Button,
                                Node {
                                    width: Val::Px(44.0),
                                    height: Val::Px(40.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.25, 0.25, 0.25)),
                                ScaleIncreaseButton,
                                Name::new("ScaleUpButton"),
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("+"),
                                    TextFont {
                                        font_size: 24.0,
                                        ..default()
                                    },
                                    TextColor(Color::WHITE),
                                ));
                            });
                        });

                    panel
                        .spawn((
                            Button,
                            Node {
                                width: Val::Px(BUTTON_WIDTH),
                                height: Val::Px(BUTTON_HEIGHT),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.25, 0.25, 0.25)),
                            RestartButton,
                            Name::new("PauseMenuRestartButton"),
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Restart"),
                                TextFont {
                                    font_size: 24.0,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                                Name::new("PauseMenuRestartLabel"),
                            ));
                        });

                    panel
                        .spawn((
                            Button,
                            Node {
                                width: Val::Px(BUTTON_WIDTH),
                                height: Val::Px(BUTTON_HEIGHT),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.25, 0.25, 0.25)),
                            ExitButton,
                            Name::new("PauseMenuExitButton"),
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Exit"),
                                TextFont {
                                    font_size: 24.0,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                                Name::new("PauseMenuExitLabel"),
                            ));
                        });
                });
        });
}

fn toggle_pause_menu(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut menu_state: ResMut<PauseMenuState>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        menu_state.open = !menu_state.open;
        info!("Pause menu {}", if menu_state.open { "opened" } else { "closed" });
    }
}

fn sync_pause_menu_visibility(
    menu_state: Res<PauseMenuState>,
    mut query: Query<&mut Visibility, With<PauseMenuRoot>>,
) {
    if !menu_state.is_changed() {
        return;
    }

    if let Ok(mut visibility) = query.single_mut() {
        *visibility = if menu_state.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
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
                *color = BackgroundColor(Color::srgb(0.35, 0.35, 0.35));
            }
            Interaction::Hovered => {
                *color = BackgroundColor(Color::srgb(0.35, 0.35, 0.35));
            }
            Interaction::None => {
                *color = BackgroundColor(Color::srgb(0.25, 0.25, 0.25));
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
                *color = BackgroundColor(Color::srgb(0.35, 0.35, 0.35));
            }
            Interaction::None => {
                *color = BackgroundColor(Color::srgb(0.25, 0.25, 0.25));
            }
        }
    }
}

fn handle_restart_button(
    mut commands: Commands,
    mut menu_state: ResMut<PauseMenuState>,
    mut team_selection: ResMut<TeamSelection>,
    mut restart_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<RestartButton>),
    >,
    local_players: Query<Entity, With<Player>>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    for (interaction, mut color) in &mut restart_query {
        match *interaction {
            Interaction::Pressed => {
                info!("Restart selected from pause menu.");
                menu_state.open = false;
                team_selection.team = None;

                if let Ok(mut cursor) = cursor_query.single_mut() {
                    cursor.grab_mode = CursorGrabMode::None;
                    cursor.visible = true;
                }

                for entity in &local_players {
                    commands.entity(entity).despawn_related::<Children>().despawn();
                }

                if overlay_query.single().is_err() {
                    spawn_team_select_ui(&mut commands, team_selection.character);
                }
            }
            Interaction::Hovered => {
                *color = BackgroundColor(Color::srgb(0.35, 0.35, 0.35));
            }
            Interaction::None => {
                *color = BackgroundColor(Color::srgb(0.25, 0.25, 0.25));
            }
        }
    }
}
