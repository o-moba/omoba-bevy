//! Debug toggles (TASK04): a bottom-left God Mode button (server skips damage) and
//! a Speed Boost button next to it (server raises the movement clamp; the client
//! moves faster locally). Both are authoritative and re-asserted on (re)connect.

use bevy::prelude::*;

use crate::net::{ClientSession, NetworkCommand};
use crate::player::DebugSpeedBoost;

const BUTTON_LEFT: f32 = 20.0;
/// Sits on the same bottom line as the skill bar (which is anchored bottom-right).
const BUTTON_BOTTOM: f32 = 20.0;
const BUTTON_WIDTH: f32 = 150.0;
const BUTTON_HEIGHT: f32 = 64.0;
const BUTTON_GAP: f32 = 10.0;
const OFF_COLOR: Color = Color::srgba(0.18, 0.18, 0.20, 0.92);
const OFF_HOVER_COLOR: Color = Color::srgba(0.26, 0.26, 0.28, 0.95);
const GOD_ON_COLOR: Color = Color::srgba(0.78, 0.20, 0.22, 0.96);
const GOD_ON_HOVER_COLOR: Color = Color::srgba(0.88, 0.28, 0.30, 0.98);
const SPEED_ON_COLOR: Color = Color::srgba(0.20, 0.44, 0.80, 0.96);
const SPEED_ON_HOVER_COLOR: Color = Color::srgba(0.28, 0.52, 0.90, 0.98);

pub struct GodModePlugin;

impl Plugin for GodModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugToggleState>()
            .add_systems(Startup, setup_debug_buttons)
            .add_systems(
                Update,
                (
                    keyboard_debug_toggles,
                    handle_god_mode_button,
                    handle_speed_boost_button,
                    periodically_assert_debug_toggles,
                    sync_debug_button_labels,
                )
                    .chain(),
            );
    }
}

#[derive(Resource, Default)]
struct DebugToggleState {
    god_mode: bool,
}

#[derive(Component)]
struct GodModeButton;

#[derive(Component)]
struct GodModeButtonLabel;

#[derive(Component)]
struct SpeedBoostButton;

#[derive(Component)]
struct SpeedBoostButtonLabel;

fn setup_debug_buttons(mut commands: Commands) {
    spawn_toggle_button(
        &mut commands,
        BUTTON_LEFT,
        "God Mode: OFF",
        OFF_COLOR,
        GodModeButton,
        GodModeButtonLabel,
        "GodModeButton",
    );
    spawn_toggle_button(
        &mut commands,
        BUTTON_LEFT + BUTTON_WIDTH + BUTTON_GAP,
        "Speed: OFF",
        OFF_COLOR,
        SpeedBoostButton,
        SpeedBoostButtonLabel,
        "SpeedBoostButton",
    );
}

fn spawn_toggle_button<B: Component, L: Component>(
    commands: &mut Commands,
    left: f32,
    text: &str,
    color: Color,
    button_marker: B,
    label_marker: L,
    name: &str,
) {
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(left),
                bottom: Val::Px(BUTTON_BOTTOM),
                width: Val::Px(BUTTON_WIDTH),
                height: Val::Px(BUTTON_HEIGHT),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(color),
            ZIndex(20),
            button_marker,
            Name::new(name.to_owned()),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(text),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                label_marker,
            ));
        });
}

/// Keyboard fallback (F2 god mode, F3 speed) so the debug toggles work even if the
/// on-screen buttons don't receive clicks.
fn keyboard_debug_toggles(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<DebugToggleState>,
    mut speed: ResMut<DebugSpeedBoost>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    if keyboard.just_pressed(KeyCode::F2) {
        state.god_mode = !state.god_mode;
        info!("[debug] god_mode -> {}", state.god_mode);
        if client_session.is_connected() {
            command_writer.write(NetworkCommand::SetGodMode {
                enabled: state.god_mode,
            });
        }
    }
    if keyboard.just_pressed(KeyCode::F3) {
        speed.0 = !speed.0;
        info!("[debug] speed_boost -> {}", speed.0);
        if client_session.is_connected() {
            command_writer.write(NetworkCommand::SetSpeedBoost { enabled: speed.0 });
        }
    }
}

fn handle_god_mode_button(
    mut state: ResMut<DebugToggleState>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut button_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<GodModeButton>),
    >,
) {
    for (interaction, mut color) in &mut button_query {
        if matches!(*interaction, Interaction::Pressed) {
            state.god_mode = !state.god_mode;
            info!("[debug] god_mode button -> {}", state.god_mode);
            if client_session.is_connected() {
                command_writer.write(NetworkCommand::SetGodMode {
                    enabled: state.god_mode,
                });
            }
        }
        *color = god_color(state.god_mode, *interaction).into();
    }
}

fn handle_speed_boost_button(
    mut speed: ResMut<DebugSpeedBoost>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut button_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<SpeedBoostButton>),
    >,
) {
    for (interaction, mut color) in &mut button_query {
        if matches!(*interaction, Interaction::Pressed) {
            speed.0 = !speed.0;
            if client_session.is_connected() {
                command_writer.write(NetworkCommand::SetSpeedBoost { enabled: speed.0 });
            }
        }
        *color = speed_color(speed.0, *interaction).into();
    }
}

/// Continuously re-assert the current debug toggle state to the server (~2x/sec).
/// A single edge-triggered send can be dropped (UDP, connection races, or a fresh
/// server session resetting the flags); periodic idempotent re-assertion guarantees
/// the server's `god_mode`/`speed_mult` eventually match the local toggles.
fn periodically_assert_debug_toggles(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    state: Res<DebugToggleState>,
    speed: Res<DebugSpeedBoost>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    if !client_session.is_connected() {
        return;
    }
    *elapsed += time.delta_secs();
    if *elapsed < 0.5 {
        return;
    }
    *elapsed = 0.0;
    command_writer.write(NetworkCommand::SetGodMode {
        enabled: state.god_mode,
    });
    command_writer.write(NetworkCommand::SetSpeedBoost { enabled: speed.0 });
}

fn sync_debug_button_labels(
    state: Res<DebugToggleState>,
    speed: Res<DebugSpeedBoost>,
    mut god_label: Query<&mut Text, (With<GodModeButtonLabel>, Without<SpeedBoostButtonLabel>)>,
    mut speed_label: Query<&mut Text, (With<SpeedBoostButtonLabel>, Without<GodModeButtonLabel>)>,
) {
    if state.is_changed() {
        let next = if state.god_mode {
            "God Mode: ON"
        } else {
            "God Mode: OFF"
        };
        for mut text in &mut god_label {
            if text.0 != next {
                text.0 = next.to_string();
            }
        }
    }
    if speed.is_changed() {
        let next = if speed.0 { "Speed: ON" } else { "Speed: OFF" };
        for mut text in &mut speed_label {
            if text.0 != next {
                text.0 = next.to_string();
            }
        }
    }
}

fn god_color(enabled: bool, interaction: Interaction) -> Color {
    let hot = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
    match (enabled, hot) {
        (true, true) => GOD_ON_HOVER_COLOR,
        (true, false) => GOD_ON_COLOR,
        (false, true) => OFF_HOVER_COLOR,
        (false, false) => OFF_COLOR,
    }
}

fn speed_color(enabled: bool, interaction: Interaction) -> Color {
    let hot = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
    match (enabled, hot) {
        (true, true) => SPEED_ON_HOVER_COLOR,
        (true, false) => SPEED_ON_COLOR,
        (false, true) => OFF_HOVER_COLOR,
        (false, false) => OFF_COLOR,
    }
}
