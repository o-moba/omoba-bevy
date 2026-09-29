//! Desktop "which server" field for the party lobby.
//!
//! A phone has the SERVER keypad (`mobile_ui::ServerEntry`); a desktop build
//! only had `GAME_SERVER_ADDR`, so a friend who double-clicked the game could
//! never reach the host. This field types a `host:port`, validates it with
//! the same rule as the preferences file and reconnects through
//! `SessionUiCommand::ConnectTo`, which also remembers it for the next start.
//!
//! Text comes from the `lobby` dictionary (`lobby.server.*`); the lobby
//! rebuilds it on a language change.
// i18n-strict

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

use super::AppScreen;
use crate::net::SessionUiCommand;
use crate::ui::{Activated, UiActionAppExt, UiSet};

/// Longest address the field accepts (a DNS name plus a port).
const MAX_ADDRESS_CHARS: usize = 64;

#[derive(Resource, Default, Clone, PartialEq, Eq, Debug)]
pub struct ServerField {
    pub editing: bool,
    pub text: String,
    /// Dictionary key of the message shown under the field.
    pub error: Option<&'static str>,
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

/// `Ok(address)` to connect to, or the dictionary key of the message shown
/// under the field.
pub(crate) fn submit(text: &str) -> Result<String, &'static str> {
    crate::persistence::validate_game_server_addr(text.trim()).ok_or("lobby.server.invalid")
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
    mut back: crate::ui::BackInput,
    mut commands: MessageWriter<SessionUiCommand>,
) {
    if !field.editing {
        typed.clear();
        return;
    }
    if back.just_pressed() {
        back.consume();
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
