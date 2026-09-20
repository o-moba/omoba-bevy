//! Welcome screen: who you are, what you can look at, and the one button that
//! starts a match.

use bevy::prelude::*;

use super::card::{ProfileCard, spawn_card};
use super::widgets::{self, ButtonKind};
use super::{AppScreen, automation_bypass};
use crate::career::CareerClient;
use crate::net::{ClientConnectionState, ClientSession, NetworkCommand};
use crate::team::AvatarThumbnails;

pub struct HomeScreenPlugin;

impl Plugin for HomeScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppScreen::Home), spawn_home)
            .add_systems(
                Update,
                (home_actions, refresh_home)
                    .chain()
                    .run_if(in_state(AppScreen::Home)),
            );
    }
}

#[derive(Component, Clone, Copy)]
enum HomeAction {
    Play,
    Card,
    Collection,
    History,
    Friends,
    Profile,
}

#[derive(Component)]
struct HomeRoot;

/// What the home screen renders from. When it changes, the screen is rebuilt.
#[derive(PartialEq, Clone)]
struct HomeSignature {
    nickname: String,
    rating: i32,
    matches: u32,
    wins: u32,
    losses: u32,
    connection: ClientConnectionState,
    card: ProfileCard,
    last_result: Option<String>,
}

fn signature(career: &CareerClient, session: &ClientSession, card: &ProfileCard) -> HomeSignature {
    let profile = career.view.profile.as_ref();
    HomeSignature {
        nickname: profile.map_or_else(
            || career.nickname.clone(),
            |profile| profile.nickname.clone(),
        ),
        rating: profile.map_or(0, |profile| profile.rating),
        matches: profile.map_or(0, |profile| profile.matches_played),
        wins: profile.map_or(0, |profile| profile.wins),
        losses: profile.map_or(0, |profile| profile.losses),
        connection: session.state,
        card: card.clone(),
        last_result: career
            .view
            .last_result
            .as_ref()
            .map(|result| result.result_id.clone()),
    }
}

fn connection_line(session: &ClientSession) -> (String, Color) {
    match session.state {
        ClientConnectionState::Connected => (
            format!("Online · {}", session.server_addr_display),
            widgets::PRIMARY,
        ),
        ClientConnectionState::Connecting | ClientConnectionState::WaitingForServer => (
            format!("Connecting to {}…", session.server_addr_display),
            widgets::GOLD,
        ),
        ClientConnectionState::Disconnected => (
            format!("Offline · retrying {}", session.server_addr_display),
            widgets::DANGER_HOVER,
        ),
    }
}

fn spawn_home(
    mut commands: Commands,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    card: Res<ProfileCard>,
    thumbnails: Res<AvatarThumbnails>,
) {
    if automation_bypass() {
        return;
    }
    let (status, status_color) = connection_line(&session);
    let profile = career.view.profile.clone();
    commands
        .spawn((
            widgets::screen_root(AppScreen::Home, "HomeScreen"),
            HomeRoot,
        ))
        .with_children(|root| {
            root.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(2.0),
                        ..default()
                    })
                    .with_children(|title| {
                        title.spawn(widgets::heading("OMOBA", 38.0));
                        title.spawn(widgets::label("Verdant Arena · 5v5", 14.0, widgets::MUTED));
                    });
                header.spawn((
                    widgets::label(&status, 15.0, status_color),
                    Name::new("HomeConnectionStatus"),
                ));
            });

            root.spawn(Node {
                flex_grow: 1.0,
                column_gap: Val::Px(28.0),
                ..default()
            })
            .with_children(|body| {
                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(14.0),
                        ..default()
                    },
                    Name::new("HomeIdentity"),
                ))
                .with_children(|column| {
                    spawn_card(
                        column,
                        &card,
                        profile.as_ref(),
                        &career.nickname,
                        &thumbnails,
                    );
                    column
                        .spawn(Node {
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            widgets::button(
                                row,
                                "Customize card",
                                ButtonKind::Secondary,
                                HomeAction::Card,
                                "HomeCustomizeCard",
                            );
                            widgets::button(
                                row,
                                "Account",
                                ButtonKind::Secondary,
                                HomeAction::Profile,
                                "HomeAccount",
                            );
                        });
                });

                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(18.0),
                        ..default()
                    },
                    Name::new("HomePlayColumn"),
                ))
                .with_children(|column| {
                    widgets::button(
                        column,
                        "PLAY",
                        ButtonKind::Primary,
                        HomeAction::Play,
                        "HomePlay",
                    );
                    column.spawn(widgets::label(
                        "Pick your hero, then the search starts.",
                        14.0,
                        widgets::MUTED,
                    ));
                    column
                        .spawn(Node {
                            column_gap: Val::Px(10.0),
                            ..default()
                        })
                        .with_children(|row| {
                            widgets::button(
                                row,
                                "Avatars",
                                ButtonKind::Secondary,
                                HomeAction::Collection,
                                "HomeCollection",
                            );
                            widgets::button(
                                row,
                                "Match history",
                                ButtonKind::Secondary,
                                HomeAction::History,
                                "HomeHistory",
                            );
                            widgets::button(
                                row,
                                "Friends",
                                ButtonKind::Secondary,
                                HomeAction::Friends,
                                "HomeFriends",
                            );
                        });
                });
            });

            root.spawn(widgets::label(
                "Escape opens settings · F1 shows the controls",
                12.0,
                widgets::MUTED,
            ));
        });
}

fn home_actions(
    mut next: ResMut<NextState<AppScreen>>,
    mut career: ResMut<CareerClient>,
    mut requests: MessageWriter<NetworkCommand>,
    buttons: Query<(&Interaction, &HomeAction), Changed<Interaction>>,
) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            HomeAction::Play => next.set(AppScreen::HeroSelect),
            HomeAction::Card => next.set(AppScreen::Card),
            HomeAction::Collection => next.set(AppScreen::Collection),
            HomeAction::History => crate::career::open_history_modal(&mut career, &mut requests),
            HomeAction::Friends => crate::career::open_friends_modal(&mut career, &mut requests),
            HomeAction::Profile => career.open_profile_modal(),
        }
    }
}

/// The career view arrives over the network after the screen is already up, so
/// the screen rebuilds itself when the facts it renders change.
fn refresh_home(
    mut commands: Commands,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    card: Res<ProfileCard>,
    thumbnails: Res<AvatarThumbnails>,
    roots: Query<Entity, With<HomeRoot>>,
    mut last: Local<Option<HomeSignature>>,
) {
    let current = signature(&career, &session, &card);
    if last.as_ref() == Some(&current) {
        return;
    }
    *last = Some(current);
    let Ok(root) = roots.single() else {
        return;
    };
    commands
        .entity(root)
        .despawn_related::<Children>()
        .despawn();
    spawn_home(commands, career, session, card, thumbnails);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_line_reports_every_session_state() {
        for state in [
            ClientConnectionState::Connected,
            ClientConnectionState::Connecting,
            ClientConnectionState::WaitingForServer,
            ClientConnectionState::Disconnected,
        ] {
            let mut session = ClientSession::default();
            session.state = state;
            let (line, _) = connection_line(&session);
            assert!(!line.is_empty(), "{state:?} must have a status line");
        }
    }
}
