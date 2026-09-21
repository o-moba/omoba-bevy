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

/// One line about the player's own last match, for the home screen.
pub fn last_match_line(result: &shared::career::MatchResult, profile_id: Option<&str>) -> String {
    let mine = profile_id.and_then(|id| {
        result
            .participants
            .iter()
            .find(|entry| entry.profile_id.as_deref() == Some(id))
    });
    let outcome = match (result.winner, mine.map(|entry| entry.team)) {
        (Some(winner), Some(team)) if winner == team => "Victory",
        (Some(_), Some(_)) => "Defeat",
        _ => "Match complete",
    };
    let minutes = result.duration_ms / 60_000;
    match mine {
        Some(entry) => format!(
            "{outcome} · {}/{}/{} · {} · {minutes} min",
            entry.stats.kills,
            entry.stats.deaths,
            entry.stats.assists,
            entry.hero_class.display_name(),
        ),
        None => format!("{outcome} · {minutes} min"),
    }
}

/// Shared status line: the home header and the picker header both use it.
pub(crate) fn connection_line(session: &ClientSession) -> (String, Color) {
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
    mut preview: ResMut<super::preview::AvatarPreview>,
) {
    if automation_bypass() {
        return;
    }
    // The home screen shows the card's hero in 3D.
    if let Some(slug) = card.showcase_avatar.as_deref() {
        preview.show_portrait(slug);
    }
    let preview_image = preview.image.clone();
    let phone = crate::platform::ui_profile() == crate::platform::UiProfile::Mobile;
    let (status, status_color) = connection_line(&session);
    let profile = career.view.profile.clone();
    let last_match = career
        .view
        .last_result
        .as_ref()
        .map(|result| last_match_line(result, career.public_profile_id.as_deref()));
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
                        title.spawn(widgets::label(
                            &format!("Verdant Arena · 5v5 · {}", env!("CARGO_PKG_VERSION")),
                            14.0,
                            widgets::MUTED,
                        ));
                    });
                header.spawn((
                    widgets::label(&status, 15.0, status_color),
                    Node {
                        // The phone bar (?, MENU, SERVER) owns the corner.
                        margin: UiRect::right(Val::Px(if phone { 330.0 } else { 0.0 })),
                        ..default()
                    },
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
                    if let Some(last) = last_match.as_deref() {
                        column
                            .spawn((widgets::panel_row(), Name::new("HomeLastMatch")))
                            .with_children(|panel| {
                                panel.spawn(widgets::label("Last match", 12.0, widgets::MUTED));
                                panel.spawn(widgets::label(last, 14.0, widgets::IVORY));
                            });
                    }
                });

                // Live showcase of the card's hero, between the identity and
                // the actions.
                body.spawn((
                    Node {
                        flex_grow: 1.0,
                        // Zero basis: the showcase takes what is left after the
                        // card and the action rail, never pushing them out.
                        flex_basis: Val::Px(0.0),
                        min_width: Val::Px(0.0),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Name::new("HomeShowcase"),
                ))
                .with_children(|column| {
                    column.spawn((
                        ImageNode::new(preview_image),
                        Node {
                            // Sized from the window height so the showcase
                            // scales with the window and never overflows it.
                            width: Val::Vh(40.0),
                            max_width: Val::Px(360.0),
                            min_width: Val::Px(0.0),
                            aspect_ratio: Some(0.742),
                            border_radius: BorderRadius::all(Val::Px(14.0)),
                            ..default()
                        },
                        Name::new("HomeShowcaseImage"),
                    ));
                });

                body.spawn((
                    Node {
                        width: Val::Px(300.0),
                        flex_direction: FlexDirection::Column,
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
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Stretch,
                            width: Val::Percent(100.0),
                            row_gap: Val::Px(8.0),
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
                if phone {
                    "MENU opens settings · SERVER sets the address"
                } else {
                    "Escape opens settings"
                },
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
    preview: ResMut<super::preview::AvatarPreview>,
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
    spawn_home(commands, career, session, card, thumbnails, preview);
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
