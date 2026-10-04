//! Floating connection status panel and its Retry button.
// i18n-strict

use bevy::prelude::*;

use crate::i18n::{Localized, data, tr, trf};
use crate::session_config::{T_RETRY, T_WAIT_MAX};
use crate::ui::{Activated, TestId, UiAction};

use super::session::{ClientConnectionState, ClientSession, MAX_JOIN_ATTEMPTS, SessionUiCommand};

#[derive(Component)]
pub(in crate::net) struct ConnectionStatusRoot;

#[derive(Component)]
pub(in crate::net) struct ConnectionStatusLabel;

#[derive(Component)]
pub(in crate::net) struct ConnectionRetryButton;

/// The Retry press. The button keeps its fixed green (no `ButtonStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::net) struct RetryPressed;

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
                    UiAction(RetryPressed),
                    TestId::new("ConnectionRetryButton"),
                ))
                .with_children(|button| {
                    button.spawn((
                        Localized::new("net.retry").into_text(),
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
    mut activated: MessageReader<Activated<RetryPressed>>,
    mut writer: MessageWriter<SessionUiCommand>,
) {
    for _ in activated.read() {
        writer.write(SessionUiCommand::Retry);
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
    // header, search screen, the loading body), so the floating panel belongs
    // to the match only. The result screen prints a lost connection in its
    // status line (post-match.md).
    let owned_by_screen = screen.as_ref().is_some_and(|screen| {
        screen.get().is_menu() || *screen.get() == crate::frontend::AppScreen::PostMatch
    });
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
    // Written every frame, so a language change shows at once.
    let status = if let Some(issue) = client_session.compatibility_issue {
        compatibility_message(issue).to_owned()
    } else if let Some(reason) = client_session.join_error {
        data::join_rejection(reason).to_owned()
    } else if client_session.join_exhausted {
        tr("net.status.join_unconfirmed").to_owned()
    } else {
        connection_line(&client_session)
    };
    if text.0 != status {
        text.0 = status;
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

/// The connection as a screen reports it (`loading-shell.md` status
/// priority, rows 1–6): the same inputs as the floating panel, without its
/// developer lines (server address, `GAME_SERVER_ADDR`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkStatus {
    /// The server refused the join (after a reconnect, or a protocol mismatch).
    Rejected(shared::protocol::JoinRejection),
    Compatibility(shared::compatibility::CompatibilityIssue),
    /// The server never confirmed the join.
    Unconfirmed,
    /// The transport dropped; the auto-reconnect is on its `attempt`.
    Reconnecting {
        attempt: u32,
    },
    /// The transport dropped and nothing retries by itself.
    Disconnected,
    /// Connecting or waiting for the server (after a Retry).
    Connecting,
    /// Connected with a committed join the server has not admitted yet.
    Joining {
        attempt: u32,
        max: u32,
    },
    /// Connected (admitted, or nothing to admit).
    Connected,
}

impl LinkStatus {
    /// A Retry press would do something nothing else is already doing.
    pub(crate) fn can_retry(self) -> bool {
        matches!(
            self,
            LinkStatus::Compatibility(_)
                | LinkStatus::Rejected(_)
                | LinkStatus::Unconfirmed
                | LinkStatus::Disconnected
        )
    }

    /// The line a screen prints for it (`None` = nothing to say).
    pub(crate) fn detail(self) -> Option<String> {
        match self {
            LinkStatus::Compatibility(issue) => Some(compatibility_message(issue).to_owned()),
            LinkStatus::Rejected(reason) => Some(data::join_rejection(reason).to_owned()),
            LinkStatus::Unconfirmed => Some(tr("net.status.join_unconfirmed").to_owned()),
            LinkStatus::Reconnecting { attempt } => {
                Some(trf("net.status.reconnecting", &[("attempt", &attempt)]))
            }
            LinkStatus::Disconnected => Some(tr("net.status.disconnected").to_owned()),
            LinkStatus::Joining { attempt, max } => Some(trf(
                "net.status.joining",
                &[("attempt", &attempt), ("max", &max)],
            )),
            LinkStatus::Connecting | LinkStatus::Connected => None,
        }
    }
}

/// Where the session stands, first match wins (`loading-shell.md`).
pub(crate) fn link_status(session: &ClientSession) -> LinkStatus {
    if let Some(issue) = session.compatibility_issue {
        return LinkStatus::Compatibility(issue);
    }
    if let Some(reason) = session.join_error {
        return LinkStatus::Rejected(reason);
    }
    if session.join_exhausted {
        return LinkStatus::Unconfirmed;
    }
    match session.state {
        ClientConnectionState::Disconnected if session.reconnect.active => {
            LinkStatus::Reconnecting {
                attempt: session.reconnect.attempts.max(1),
            }
        }
        ClientConnectionState::Disconnected => LinkStatus::Disconnected,
        ClientConnectionState::Connecting | ClientConnectionState::WaitingForServer => {
            LinkStatus::Connecting
        }
        ClientConnectionState::Connected
            if session.last_join.is_some() && !session.join_confirmed() =>
        {
            LinkStatus::Joining {
                attempt: session.join_attempts,
                max: MAX_JOIN_ATTEMPTS,
            }
        }
        ClientConnectionState::Connected => LinkStatus::Connected,
    }
}

fn connection_line(client_session: &ClientSession) -> String {
    let addr = &client_session.server_addr_display;
    match client_session.state {
        ClientConnectionState::Connecting => trf("net.status.connecting", &[("addr", addr)]),
        ClientConnectionState::WaitingForServer => trf(
            "net.status.waiting",
            &[
                ("addr", addr),
                ("retry", &T_RETRY.as_secs()),
                ("max_wait", &T_WAIT_MAX.as_secs()),
            ],
        ),
        ClientConnectionState::Connected if client_session.join_confirmed() => {
            tr("net.status.joined").to_owned()
        }
        ClientConnectionState::Connected if client_session.last_join.is_some() => trf(
            "net.status.joining",
            &[
                ("attempt", &client_session.join_attempts),
                ("max", &MAX_JOIN_ATTEMPTS),
            ],
        ),
        ClientConnectionState::Connected => tr("net.status.connected").to_owned(),
        ClientConnectionState::Disconnected if client_session.reconnect.active => trf(
            "net.status.reconnecting",
            &[("attempt", &client_session.reconnect.attempts.max(1))],
        ),
        ClientConnectionState::Disconnected => tr("net.status.disconnected").to_owned(),
    }
}

fn compatibility_message(issue: shared::compatibility::CompatibilityIssue) -> &'static str {
    use shared::compatibility::CompatibilityIssue as Issue;
    tr(match issue {
        Issue::Handshake => "net.compatibility.handshake",
        Issue::Protocol => "net.compatibility.protocol",
        Issue::Catalog => "net.compatibility.catalog",
        Issue::Geometry => "net.compatibility.geometry",
        Issue::Gameplay => "net.compatibility.gameplay",
        Issue::Unavailable => "net.compatibility.unavailable",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status line and the Retry label follow a language change.
    #[test]
    fn connection_status_follows_the_language() {
        if crate::i18n::testing::isolated(
            "net::status_ui::tests::connection_status_follows_the_language",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let mut app = App::new();
        app.add_plugins(I18nPlugin::default())
            .init_resource::<ClientSession>()
            .add_systems(Startup, setup_connection_status_ui)
            .add_systems(Update, sync_connection_status_ui);
        app.world_mut().resource_mut::<ClientSession>().state = ClientConnectionState::Disconnected;
        let texts = |app: &mut App| {
            let mut label = app
                .world_mut()
                .query_filtered::<&Text, With<ConnectionStatusLabel>>();
            let label = label.single(app.world()).unwrap().0.clone();
            let mut retry = app.world_mut().query::<(&crate::i18n::Localized, &Text)>();
            let (_, retry) = retry.single(app.world()).unwrap();
            (label, retry.0.clone())
        };
        app.update();
        let (label, retry) = texts(&mut app);
        assert!(label.starts_with("Disconnected - connection lost"));
        assert_eq!(retry, "Retry");
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("zh-Hans").unwrap());
        app.update();
        let (label, retry) = texts(&mut app);
        assert!(label.starts_with("已断开"), "{label}");
        assert_eq!(retry, "重试");
        app.world_mut()
            .resource_mut::<ClientSession>()
            .join_exhausted = true;
        app.update();
        assert_eq!(
            texts(&mut app).0,
            "服务器未确认你的加入请求。请点击“重试”再试一次。"
        );
    }

    #[test]
    fn admission_hides_status_and_disconnection_exposes_working_retry_without_empty_layout() {
        let mut app = App::new();
        use crate::ui::UiActionAppExt;
        app.init_resource::<ClientSession>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<RetryPressed>()
            .add_systems(Startup, setup_connection_status_ui)
            .add_systems(
                Update,
                (handle_connection_retry_button, sync_connection_status_ui)
                    .chain()
                    .after(crate::ui::UiSet::Dispatch),
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
