//! Floating connection status panel and its Retry button.

use bevy::prelude::*;

use crate::session_config::{T_RETRY, T_WAIT_MAX};

use super::session::{ClientConnectionState, ClientSession, MAX_JOIN_ATTEMPTS, SessionUiCommand};

#[derive(Component)]
pub(in crate::net) struct ConnectionStatusRoot;

#[derive(Component)]
pub(in crate::net) struct ConnectionStatusLabel;

#[derive(Component)]
pub(in crate::net) struct ConnectionRetryButton;

const CONNECTION_PANEL_BG: Color = Color::srgba(0.02, 0.02, 0.06, 0.72);

pub(in crate::net) fn setup_connection_status_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(16.0),
                width: Val::Px(340.0),
                max_width: Val::Percent(90.0),
                top: Val::Px(12.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(CONNECTION_PANEL_BG),
            ConnectionStatusRoot,
            Visibility::Visible,
            ZIndex(20),
            Name::new("ConnectionStatusPanel"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                ConnectionStatusLabel,
            ));
            parent
                .spawn((
                    Button,
                    Node {
                        display: Display::None,
                        width: Val::Px(120.0),
                        height: Val::Px(36.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.25, 0.42, 0.32)),
                    Visibility::Hidden,
                    ConnectionRetryButton,
                    Name::new("ConnectionRetryButton"),
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new("Retry"),
                        TextFont {
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });
        });
}

pub(in crate::net) fn handle_connection_retry_button(
    interaction: Query<&Interaction, (With<ConnectionRetryButton>, Changed<Interaction>)>,
    mut writer: MessageWriter<SessionUiCommand>,
) {
    for i in &interaction {
        if *i == Interaction::Pressed {
            writer.write(SessionUiCommand::Retry);
        }
    }
}

pub(in crate::net) fn sync_connection_status_ui(
    client_session: Res<ClientSession>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut label_q: Query<&mut Text, With<ConnectionStatusLabel>>,
    mut root: Query<
        (&mut Visibility, &mut Node),
        (With<ConnectionStatusRoot>, Without<ConnectionRetryButton>),
    >,
    mut retry: Query<
        (&mut Visibility, &mut Node),
        (With<ConnectionRetryButton>, Without<ConnectionStatusRoot>),
    >,
) {
    // Front-end screens print their own status line (home header, picker
    // header, search screen), so the floating panel belongs to the match only.
    let owned_by_screen = screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    let healthy_admission = owned_by_screen
        || (client_session.join_confirmed()
            && client_session.join_error.is_none()
            && !client_session.join_exhausted);
    if let Ok((mut visibility, mut node)) = root.single_mut() {
        *visibility = if healthy_admission {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        node.display = if healthy_admission {
            Display::None
        } else {
            Display::Flex
        };
    }
    let Ok(mut text) = label_q.single_mut() else {
        return;
    };
    match client_session.state {
        ClientConnectionState::Connecting => {
            text.0 = format!("Connecting… ({})", client_session.server_addr_display);
        }
        ClientConnectionState::WaitingForServer => {
            text.0 = format!(
                "Waiting for server at {}. Start the server or check GAME_SERVER_ADDR. Retries every {}s (max wait {}s).",
                client_session.server_addr_display,
                T_RETRY.as_secs(),
                T_WAIT_MAX.as_secs()
            );
        }
        ClientConnectionState::Connected => {
            text.0 = if client_session.join_confirmed() {
                "Joined - connected to match.".to_owned()
            } else if client_session.last_join.is_some() {
                format!(
                    "Joining match… attempt {}/{}. Waiting for server admission.",
                    client_session.join_attempts, MAX_JOIN_ATTEMPTS
                )
            } else {
                "Connected - choose a hero and team to join.".to_owned()
            };
        }
        ClientConnectionState::Disconnected if client_session.reconnect.active => {
            text.0 = format!(
                "Connection lost - reconnecting (attempt {})...",
                client_session.reconnect.attempts.max(1)
            );
        }
        ClientConnectionState::Disconnected => {
            text.0 = "Disconnected - connection lost or timed out. Use Retry when the server is back, then choose your team again."
                .to_string();
        }
    }

    if let Some(reason) = client_session.join_error {
        text.0 = reason.message().to_owned();
    } else if client_session.join_exhausted {
        text.0 = "The server did not confirm your Join. Use Retry to try again.".to_owned();
    }

    if let Ok((mut visibility, mut node)) = retry.single_mut() {
        let can_retry = client_session.state == ClientConnectionState::Disconnected
            || client_session.join_error.is_some()
            || client_session.join_exhausted;
        *visibility = if can_retry {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        node.display = if can_retry {
            Display::Flex
        } else {
            Display::None
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_hides_status_and_disconnection_exposes_working_retry_without_empty_layout() {
        let mut app = App::new();
        app.init_resource::<ClientSession>()
            .add_message::<SessionUiCommand>()
            .add_systems(Startup, setup_connection_status_ui)
            .add_systems(
                Update,
                (handle_connection_retry_button, sync_connection_status_ui).chain(),
            );
        app.update();
        let root = app
            .world_mut()
            .query_filtered::<Entity, With<ConnectionStatusRoot>>()
            .single(app.world())
            .unwrap();
        let retry = app
            .world_mut()
            .query_filtered::<Entity, With<ConnectionRetryButton>>()
            .single(app.world())
            .unwrap();
        assert_eq!(
            app.world().get::<Node>(retry).unwrap().display,
            Display::None
        );
        {
            let mut session = app.world_mut().resource_mut::<ClientSession>();
            session.state = ClientConnectionState::Connected;
            session.admitted = true;
        }
        app.update();
        assert_eq!(
            app.world().get::<Node>(root).unwrap().display,
            Display::None
        );
        assert_eq!(
            *app.world().get::<Visibility>(root).unwrap(),
            Visibility::Hidden
        );
        app.world_mut().resource_mut::<ClientSession>().state = ClientConnectionState::Disconnected;
        app.update();
        assert_eq!(
            app.world().get::<Node>(root).unwrap().display,
            Display::Flex
        );
        assert_eq!(
            app.world().get::<Node>(retry).unwrap().display,
            Display::Flex
        );
        assert_eq!(
            *app.world().get::<Visibility>(retry).unwrap(),
            Visibility::Visible
        );
        app.world_mut()
            .entity_mut(retry)
            .insert(Interaction::Pressed);
        app.update();
        assert_eq!(
            app.world().resource::<Messages<SessionUiCommand>>().len(),
            1
        );
    }
}
