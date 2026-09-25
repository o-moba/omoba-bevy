//! Debug toggles (TASK04): a bottom-left God Mode button (server skips damage) and
//! a Speed Boost button next to it (server raises the movement clamp; the client
//! moves faster locally), plus the F2/F3 hotkeys. Both are authoritative and
//! re-asserted by [`super::resend_debug_toggles`]. The state is [`DebugToggles`].
//!
//! These are the `OMOBA_DEBUG_UI` extras: they exist only with the env var
//! and outside Combat Test, and they show and react only while
//! [`ClientDebugAccess::toggles`] allows the toggles (step 11e). Without the
//! env var the same toggles live on the pause menu's tools page.

use bevy::prelude::*;
use shared::debug::DebugCommand;

use super::{ClientDebugAccess, DebugAccessSet, DebugConsole, DebugToggles};
use crate::net::{ClientSession, NetworkCommand};

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
        app.init_resource::<DebugConsole>()
            .init_resource::<DebugToggles>()
            .add_systems(Startup, setup_debug_buttons)
            .add_systems(
                Update,
                (
                    (
                        keyboard_debug_toggles,
                        handle_god_mode_button,
                        handle_speed_boost_button,
                    )
                        .run_if(debug_controls_enabled),
                    sync_debug_button_visibility,
                    sync_debug_button_labels,
                )
                    .chain()
                    .after(DebugAccessSet),
            );
    }
}

/// Both HUD buttons; shown only while the toggles are allowed.
#[derive(Component)]
struct DebugHudButton;

#[derive(Component)]
struct GodModeButton;

#[derive(Component)]
struct GodModeButtonLabel;

#[derive(Component)]
struct SpeedBoostButton;

#[derive(Component)]
struct SpeedBoostButtonLabel;

/// `OMOBA_DEBUG_UI` and the toggles allowed here (never in Combat Test).
fn debug_controls_enabled(
    console: Res<DebugConsole>,
    access: Option<Res<ClientDebugAccess>>,
) -> bool {
    console.ui_enabled && access.is_some_and(|access| access.toggles())
}

fn sync_debug_button_visibility(
    console: Res<DebugConsole>,
    access: Option<Res<ClientDebugAccess>>,
    mut buttons: Query<(&mut Node, &mut Visibility), With<DebugHudButton>>,
) {
    let show = console.ui_enabled && access.is_some_and(|access| access.toggles());
    let display = if show { Display::Flex } else { Display::None };
    for (mut node, mut visibility) in &mut buttons {
        if node.display != display {
            node.display = display;
            *visibility = if show {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
}

fn setup_debug_buttons(mut commands: Commands, console: Res<DebugConsole>) {
    if !console.ui_enabled || crate::sandbox::requested() {
        return;
    }
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
                // Hidden until `sync_debug_button_visibility` sees access.
                display: Display::None,
                ..default()
            },
            Visibility::Hidden,
            BackgroundColor(color),
            ZIndex(20),
            DebugHudButton,
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
    mut toggles: ResMut<DebugToggles>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    if keyboard.just_pressed(KeyCode::F2) {
        toggles.god_mode = !toggles.god_mode;
        info!("[debug] god_mode -> {}", toggles.god_mode);
        if client_session.is_connected() {
            command_writer.write(NetworkCommand::Debug(DebugCommand::GodMode(
                toggles.god_mode,
            )));
        }
    }
    if keyboard.just_pressed(KeyCode::F3) {
        toggles.speed_boost = !toggles.speed_boost;
        info!("[debug] speed_boost -> {}", toggles.speed_boost);
        if client_session.is_connected() {
            command_writer.write(NetworkCommand::Debug(DebugCommand::SpeedBoost(
                toggles.speed_boost,
            )));
        }
    }
}

fn handle_god_mode_button(
    mut toggles: ResMut<DebugToggles>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut button_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<GodModeButton>),
    >,
) {
    for (interaction, mut color) in &mut button_query {
        if matches!(*interaction, Interaction::Pressed) {
            toggles.god_mode = !toggles.god_mode;
            info!("[debug] god_mode button -> {}", toggles.god_mode);
            if client_session.is_connected() {
                command_writer.write(NetworkCommand::Debug(DebugCommand::GodMode(
                    toggles.god_mode,
                )));
            }
        }
        *color = god_color(toggles.god_mode, *interaction).into();
    }
}

fn handle_speed_boost_button(
    mut toggles: ResMut<DebugToggles>,
    client_session: Res<ClientSession>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut button_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<SpeedBoostButton>),
    >,
) {
    for (interaction, mut color) in &mut button_query {
        if matches!(*interaction, Interaction::Pressed) {
            toggles.speed_boost = !toggles.speed_boost;
            if client_session.is_connected() {
                command_writer.write(NetworkCommand::Debug(DebugCommand::SpeedBoost(
                    toggles.speed_boost,
                )));
            }
        }
        *color = speed_color(toggles.speed_boost, *interaction).into();
    }
}

fn sync_debug_button_labels(
    toggles: Res<DebugToggles>,
    mut god_label: Query<&mut Text, (With<GodModeButtonLabel>, Without<SpeedBoostButtonLabel>)>,
    mut speed_label: Query<&mut Text, (With<SpeedBoostButtonLabel>, Without<GodModeButtonLabel>)>,
) {
    if !toggles.is_changed() {
        return;
    }
    let god = if toggles.god_mode {
        "God Mode: ON"
    } else {
        "God Mode: OFF"
    };
    let speed = if toggles.speed_boost {
        "Speed: ON"
    } else {
        "Speed: OFF"
    };
    for mut text in &mut god_label {
        if text.0 != god {
            text.0 = god.to_string();
        }
    }
    for mut text in &mut speed_label {
        if text.0 != speed {
            text.0 = speed.to_string();
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::debug::DebugAccess;

    fn press_f2_f3(app: &mut App) {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset_all();
        keys.press(KeyCode::F2);
        keys.press(KeyCode::F3);
    }

    fn hud_app(ui_enabled: bool, access: ClientDebugAccess) -> App {
        let mut app = App::new();
        let mut console = DebugConsole::default();
        console.ui_enabled = ui_enabled;
        app.insert_resource(console)
            .insert_resource(access)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<DebugToggles>()
            .init_resource::<ClientSession>()
            .add_message::<NetworkCommand>()
            .add_systems(Startup, setup_debug_buttons)
            .add_systems(
                Update,
                (
                    keyboard_debug_toggles.run_if(debug_controls_enabled),
                    sync_debug_button_visibility,
                )
                    .chain(),
            );
        app
    }

    fn visible_buttons(app: &mut App) -> usize {
        app.world_mut()
            .query::<(&Button, &Node)>()
            .iter(app.world())
            .filter(|(_, node)| node.display != Display::None)
            .count()
    }

    #[test]
    fn disabled_debug_ui_spawns_no_controls_and_ignores_admin_keys() {
        let dev = ClientDebugAccess {
            server: DebugAccess::for_match_mode("dev"),
            combat_test: false,
        };
        let mut app = hud_app(false, dev);
        press_f2_f3(&mut app);
        app.update();
        assert_eq!(
            app.world_mut().query::<&Button>().iter(app.world()).count(),
            0
        );
        assert_eq!(
            *app.world().resource::<DebugToggles>(),
            DebugToggles::default()
        );
        assert!(
            app.world()
                .resource::<Messages<NetworkCommand>>()
                .is_empty()
        );

        let mut enabled = hud_app(true, dev);
        enabled.update();
        assert_eq!(
            enabled
                .world_mut()
                .query::<&Button>()
                .iter(enabled.world())
                .count(),
            2
        );
        assert_eq!(visible_buttons(&mut enabled), 2);
    }

    /// Step 11e: with `OMOBA_DEBUG_UI` the HUD still follows access: hidden
    /// and deaf in release, shown and working in dev.
    #[test]
    fn debug_ui_controls_follow_access() {
        let mut app = hud_app(true, ClientDebugAccess::default());
        press_f2_f3(&mut app);
        app.update();
        assert_eq!(visible_buttons(&mut app), 0);
        assert_eq!(
            *app.world().resource::<DebugToggles>(),
            DebugToggles::default()
        );

        app.insert_resource(ClientDebugAccess {
            server: DebugAccess::for_match_mode("dev"),
            combat_test: false,
        });
        app.insert_resource(ClientSession::admitted_for_test());
        press_f2_f3(&mut app);
        app.update();
        assert_eq!(visible_buttons(&mut app), 2);
        assert_eq!(
            *app.world().resource::<DebugToggles>(),
            DebugToggles {
                god_mode: true,
                speed_boost: true
            }
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            2
        );
    }
}
