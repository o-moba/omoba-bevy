//! Desktop "which server" field for the party lobby.
//!
//! A phone has the SERVER keypad (`mobile_ui::ServerEntry`); a desktop build
//! only had `GAME_SERVER_ADDR`, so a friend who double-clicked the game could
//! never reach the host. This field types a `host:port`, validates it with
//! the same rule as the preferences file and reconnects through
//! `SessionUiCommand::ConnectTo`, which also remembers it for the next start.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

use super::{AppScreen, widgets};
use crate::net::SessionUiCommand;
use crate::ui::theme::{self, ButtonKind};
use crate::ui::widgets::screen_button;
use crate::ui::{Activated, UiActionAppExt, UiSet};

/// Longest address the field accepts (a DNS name plus a port).
const MAX_ADDRESS_CHARS: usize = 64;

#[derive(Resource, Default, Clone, PartialEq, Eq, Debug)]
pub struct ServerField {
    pub editing: bool,
    pub text: String,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServerFieldAction {
    Edit,
    Connect,
    Cancel,
}

pub struct ServerFieldPlugin;

impl Plugin for ServerFieldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerField>()
            .add_ui_action::<ServerFieldAction>()
            .add_systems(OnExit(AppScreen::Lobby), stop_editing)
            .add_systems(
                Update,
                type_address
                    .run_if(in_state(AppScreen::Lobby))
                    // Escape cancels the edit before it can open the pause menu.
                    .before(crate::pause_menu::PauseMenuSet::Close),
            )
            .add_systems(
                Update,
                field_actions
                    .after(UiSet::Dispatch)
                    .run_if(in_state(AppScreen::Lobby)),
            );
    }
}

/// Keeps the characters an address can contain, up to the length limit.
pub(crate) fn append_address(text: &mut String, typed: &str) {
    for ch in typed.chars() {
        if text.chars().count() >= MAX_ADDRESS_CHARS {
            break;
        }
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | '-' | '[' | ']') {
            text.push(ch);
        }
    }
}

/// `Ok(address)` to connect to, or the message shown under the field.
pub(crate) fn submit(text: &str) -> Result<String, String> {
    crate::persistence::validate_game_server_addr(text.trim())
        .ok_or_else(|| "Use host:port, for example 192.168.1.20:4000".to_owned())
}

fn stop_editing(mut field: ResMut<ServerField>) {
    field.editing = false;
    field.error = None;
}

fn connect(field: &mut ServerField, commands: &mut MessageWriter<SessionUiCommand>) {
    match submit(&field.text) {
        Ok(address) => {
            commands.write(SessionUiCommand::ConnectTo(address));
            field.editing = false;
            field.error = None;
        }
        Err(message) => field.error = Some(message),
    }
}

fn type_address(
    mut field: ResMut<ServerField>,
    mut typed: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut commands: MessageWriter<SessionUiCommand>,
) {
    if !field.editing {
        typed.clear();
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        keys.clear_just_pressed(KeyCode::Escape);
        field.editing = false;
        field.error = None;
        typed.clear();
        return;
    }
    for event in typed.read() {
        if !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => {
                field.text.pop();
            }
            Key::Enter => connect(&mut field, &mut commands),
            Key::Escape => {}
            _ => {
                if let Some(text) = event.text.as_deref() {
                    append_address(&mut field.text, text);
                }
            }
        }
    }
}

fn field_actions(
    mut activated: MessageReader<Activated<ServerFieldAction>>,
    mut field: ResMut<ServerField>,
    session: Res<crate::net::ClientSession>,
    mut commands: MessageWriter<SessionUiCommand>,
) {
    for Activated { action, .. } in activated.read() {
        match action {
            ServerFieldAction::Edit => {
                field.editing = true;
                field.error = None;
                field.text = session.server_addr().to_owned();
            }
            ServerFieldAction::Connect => connect(&mut field, &mut commands),
            ServerFieldAction::Cancel => {
                field.editing = false;
                field.error = None;
            }
        }
    }
}

/// The SERVER block of the lobby's social column.
pub(crate) fn spawn_server_field(
    parent: &mut ChildSpawnerCommands,
    field: &ServerField,
    current: &str,
) {
    parent.spawn(widgets::label("SERVER", 12.0, theme::GOLD));
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(theme::TILE),
            BorderColor::all(if field.editing {
                theme::GOLD
            } else {
                theme::PANEL_EDGE
            }),
            Name::new("LobbyServerField"),
        ))
        .with_children(|block| {
            if field.editing {
                block.spawn(widgets::label(
                    &format!("{}▏", field.text),
                    16.0,
                    theme::IVORY,
                ));
                block.spawn(widgets::label(
                    "Type the host's address · Enter connects · Esc cancels",
                    11.0,
                    theme::MUTED,
                ));
                if let Some(error) = &field.error {
                    block.spawn(widgets::label(error, 12.0, theme::DANGER_HOVER));
                }
                block
                    .spawn(Node {
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        screen_button(
                            row,
                            "Connect",
                            ButtonKind::Secondary,
                            ServerFieldAction::Connect,
                            "LobbyServerConnect",
                        );
                        screen_button(
                            row,
                            "Cancel",
                            ButtonKind::Secondary,
                            ServerFieldAction::Cancel,
                            "LobbyServerCancel",
                        );
                    });
            } else {
                block
                    .spawn(Node {
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(widgets::label(current, 15.0, theme::IVORY));
                        screen_button(
                            row,
                            "Change",
                            ButtonKind::Secondary,
                            ServerFieldAction::Edit,
                            "LobbyServerChange",
                        );
                    });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_address_characters_are_typed_and_the_length_is_bounded() {
        let mut text = String::new();
        append_address(&mut text, "192.168.1.20:4000 ;rm");
        assert_eq!(text, "192.168.1.20:4000rm");
        let mut long = String::new();
        append_address(&mut long, &"a".repeat(200));
        assert_eq!(long.chars().count(), MAX_ADDRESS_CHARS);
    }

    #[test]
    fn submit_accepts_host_port_and_explains_anything_else() {
        assert_eq!(
            submit(" 192.168.1.20:4000 "),
            Ok("192.168.1.20:4000".into())
        );
        assert!(submit("192.168.1.20").is_err());
        assert!(submit("").is_err());
    }
}
