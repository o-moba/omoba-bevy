//! Matchmaking screen: what the server says about the queue, and a way out.

use bevy::prelude::*;
use shared::career::QueueView;

use super::AppScreen;
use super::widgets::{self, ButtonKind};
use crate::career::CareerClient;
use crate::net::{ClientSession, GameState, GameStateSnapshot, SessionUiCommand};
use crate::team::TeamSelection;

pub struct SearchingScreenPlugin;

impl Plugin for SearchingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppScreen::Searching), spawn_searching)
            .add_systems(
                Update,
                (searching_actions, refresh_status)
                    .chain()
                    .run_if(in_state(AppScreen::Searching)),
            );
    }
}

#[derive(Component)]
struct SearchingStatus;

#[derive(Component)]
struct SearchingCancel;

/// One status line for the screen. The career queue is authoritative when the
/// server runs ranked matchmaking; otherwise the match formation counters are.
pub fn status_text(queue: &QueueView, game: &GameState, session: &ClientSession) -> String {
    if session.state() != crate::net::ClientConnectionState::Connected {
        return super::home::connection_line(session).0;
    }
    if let Some(rejection) = session.join_rejection() {
        return rejection.message().to_owned();
    }
    if let Some(text) = crate::career::queue_text(queue) {
        return text;
    }
    match game {
        GameState::Lobby => "Waiting for players…".to_owned(),
        GameState::Forming { ready, needed } => {
            format!(
                "Waiting for players · {ready}/{needed}\nThe match starts as soon as the roster is full."
            )
        }
        GameState::Starting { countdown_ms } => format!(
            "Match found!\nStarting in {}…",
            countdown_ms.div_ceil(1000).max(1)
        ),
        GameState::Running | GameState::Victory { .. } => "Joining the match…".to_owned(),
    }
}

fn spawn_searching(mut commands: Commands, selection: Res<TeamSelection>) {
    let hero = format!(
        "{} · {}",
        selection.hero_class.display_name(),
        selection
            .avatar
            .as_deref()
            .and_then(omoba_passport::avatars::avatar_definition)
            .map_or("default avatar", |avatar| avatar.display_name.as_str())
    );
    commands
        .spawn(widgets::screen_root(
            AppScreen::Searching,
            "SearchingScreen",
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(620.0),
                    max_width: Val::Percent(92.0),
                    margin: UiRect::all(Val::Auto),
                    padding: UiRect::all(Val::Px(40.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(16.0),
                    ..default()
                },
                BackgroundColor(widgets::PANEL),
                BorderColor::all(widgets::PANEL_EDGE),
                Name::new("SearchingBody"),
            ))
            .with_children(|body| {
                body.spawn(widgets::label("MATCHMAKING", 12.0, widgets::GOLD));
                body.spawn((
                    Node {
                        width: Val::Px(48.0),
                        height: Val::Px(3.0),
                        margin: UiRect::bottom(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(widgets::GOLD),
                ));
                body.spawn(widgets::heading("Finding a match", 34.0));
                body.spawn((
                    widgets::label("Contacting the server…", 16.0, widgets::IVORY),
                    SearchingStatus,
                    Name::new("SearchingStatus"),
                ));
                body.spawn(widgets::label(&hero, 14.0, widgets::MUTED));
                widgets::button(
                    body,
                    "Cancel",
                    ButtonKind::Secondary,
                    SearchingCancel,
                    "SearchingCancel",
                );
            });
        });
}

fn searching_actions(
    mut session_ui: MessageWriter<SessionUiCommand>,
    buttons: Query<&Interaction, (Changed<Interaction>, With<SearchingCancel>)>,
) {
    for interaction in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // One path for every server mode: the server drops the queue entry or
        // the seat, the client drops the join, and the shell goes home. A
        // signed career `CancelQueue` alone did nothing on a practice or dev
        // server and left the join retrying behind the menus.
        session_ui.write(SessionUiCommand::LeaveMatch);
    }
}

fn refresh_status(
    career: Res<CareerClient>,
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    mut status: Query<&mut Text, With<SearchingStatus>>,
) {
    let text = if session.state() != crate::net::ClientConnectionState::Connected {
        super::home::connection_line(&session).0
    } else if let Some(error) = career
        .view
        .error
        .as_ref()
        .filter(|_| career.view.match_service.is_some())
    {
        error.clone()
    } else if let Some(view) = &career.view.match_service {
        crate::match_service::status_text(view)
    } else {
        status_text(&career.view.queue, &game.state, &session)
    };
    for mut label in &mut status {
        if label.0 != text {
            label.0.clone_from(&text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formation_counters_are_shown_when_the_career_queue_is_idle() {
        let mut session = ClientSession::default();
        session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        let text = status_text(
            &QueueView::Idle,
            &GameState::Forming {
                ready: 3,
                needed: 10,
            },
            &session,
        );
        assert!(text.contains("3/10"), "got: {text}");
    }

    #[test]
    fn a_rejection_replaces_the_queue_copy() {
        let mut session = ClientSession::default();
        session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        session.reject_for_test(shared::protocol::JoinRejection::MatchFull);
        let text = status_text(&QueueView::Idle, &GameState::Lobby, &session);
        assert_eq!(text, shared::protocol::JoinRejection::MatchFull.message());
    }
}
