//! Party lobby: the party's avatars on stage, invites in and out, and the
//! leader's "play together" buttons.
//!
//! Everything shown comes from the server's [`shared::party::PartyView`]
//! (`crate::party::PartyClient`); buttons only send `PartyCommand`s. The
//! launch itself moves every member to hero select (`crate::party`).

use bevy::prelude::*;
use shared::match_service::MatchPreference;
use shared::party::{MAX_PARTY_SIZE, OnlinePlayer, PartyCommand, PartyView};

use super::party_stage::{PartyStage, STAGE_HEIGHT, STAGE_WIDTH, StageMember};
use super::{AppScreen, automation_bypass, widgets};
use crate::net::{ClientSession, NetworkCommand};
use crate::party::PartyClient;
use crate::ui::theme::{self, ButtonKind};
use crate::ui::widgets::screen_button;
use crate::ui::{Activated, UiActionAppExt, UiSet};

pub struct LobbyScreenPlugin;

impl Plugin for LobbyScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_ui_action::<LobbyAction>()
            .add_systems(OnEnter(AppScreen::Lobby), spawn_lobby)
            .add_systems(
                Update,
                (lobby_actions, refresh_lobby)
                    .chain()
                    .after(UiSet::Dispatch)
                    .run_if(in_state(AppScreen::Lobby)),
            );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LobbyAction {
    Back,
    /// The career friends modal (add by code, requests); needs storage.
    Friends,
    Play(MatchPreference),
    Leave,
    Invite(u64),
    Accept(u64),
    Decline(u64),
    Kick(u64),
}

#[derive(Component)]
struct LobbyRoot;

/// What the invite column offers for one online player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InviteOffer {
    Invite,
    Invited,
    /// Already in the viewer's party.
    Member,
    /// Only the leader invites; or the party is full.
    Unavailable,
}

pub(crate) fn invite_offer(view: &PartyView, player: &OnlinePlayer) -> InviteOffer {
    let members = view.party.as_ref().map_or(&[][..], |p| &p.members[..]);
    if members.iter().any(|m| m.player_id == player.player_id) {
        InviteOffer::Member
    } else if player.invited {
        InviteOffer::Invited
    } else if !view.can_launch() || members.len() >= MAX_PARTY_SIZE {
        InviteOffer::Unavailable
    } else {
        InviteOffer::Invite
    }
}

/// Who stands on stage: the party (leader first), or the viewer alone.
pub(crate) fn stage_members(view: &PartyView, own_avatar: Option<String>) -> Vec<StageMember> {
    match &view.party {
        Some(party) => party
            .members
            .iter()
            .map(|m| StageMember {
                avatar: m.avatar.clone(),
                leader: m.leader,
            })
            .collect(),
        None => vec![StageMember {
            avatar: own_avatar,
            leader: true,
        }],
    }
}

fn member_status(member: &shared::party::PartyMember) -> (&'static str, Color) {
    if member.away {
        ("Away", theme::MUTED)
    } else if member.in_match {
        ("In match", theme::GOLD)
    } else {
        ("Ready", theme::JADE)
    }
}

#[derive(PartialEq, Clone)]
struct LobbySignature {
    view: PartyView,
    live: bool,
    public: bool,
    friends: bool,
    server: String,
    field: super::server_field::ServerField,
}

fn signature(
    party: &PartyClient,
    career: &crate::career::CareerClient,
    session: &ClientSession,
    field: &super::server_field::ServerField,
) -> LobbySignature {
    LobbySignature {
        view: party.view.clone(),
        live: party.is_live(std::time::Instant::now()),
        public: career.view.match_service.is_some(),
        friends: career.view.storage_enabled,
        server: session.server_addr().to_owned(),
        field: field.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_lobby(
    mut commands: Commands,
    party: Res<PartyClient>,
    career: Res<crate::career::CareerClient>,
    session: Res<ClientSession>,
    card: Res<super::card::ProfileCard>,
    selection: Res<crate::team::TeamSelection>,
    mut stage: ResMut<PartyStage>,
    platform: Res<crate::ui::UiPlatform>,
    field: Res<super::server_field::ServerField>,
) {
    if automation_bypass() {
        return;
    }
    let phone = platform.is_mobile();
    let sig = signature(&party, &career, &session, &field);
    let view = &sig.view;
    stage.members = stage_members(view, crate::party::presence_avatar(&card, &selection));
    let image = stage.image.clone();
    let own_name = career
        .view
        .profile
        .as_ref()
        .map_or_else(|| career.nickname.clone(), |p| p.nickname.clone());
    commands
        .spawn((
            widgets::screen_root(AppScreen::Lobby, "LobbyScreen"),
            LobbyRoot,
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
                        title.spawn(widgets::heading("PARTY", 32.0));
                        title.spawn(widgets::label(
                            "Group up · same team · play together",
                            14.0,
                            theme::MUTED,
                        ));
                    });
                header
                    .spawn(Node {
                        column_gap: Val::Px(10.0),
                        margin: UiRect::right(Val::Px(if phone { 400.0 } else { 0.0 })),
                        ..default()
                    })
                    .with_children(|actions| {
                        if sig.friends {
                            screen_button(
                                actions,
                                "Friends list",
                                ButtonKind::Secondary,
                                LobbyAction::Friends,
                                "LobbyFriends",
                            );
                        }
                        if view.party.is_some() {
                            screen_button(
                                actions,
                                "Leave party",
                                ButtonKind::Danger,
                                LobbyAction::Leave,
                                "LobbyLeave",
                            );
                        }
                        screen_button(
                            actions,
                            "Back",
                            ButtonKind::Secondary,
                            LobbyAction::Back,
                            "LobbyBack",
                        );
                    });
            });

            root.spawn((
                Node {
                    flex_grow: 1.0,
                    column_gap: Val::Px(20.0),
                    min_height: Val::Px(0.0),
                    padding: UiRect::all(Val::Px(if phone { 12.0 } else { 20.0 })),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(theme::PANEL_OPAQUE),
                BorderColor::all(theme::PANEL_EDGE),
            ))
            .with_children(|body| {
                // Stage and member plates.
                body.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        min_width: Val::Px(0.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(12.0),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Name::new("LobbyStage"),
                ))
                .with_children(|column| {
                    column.spawn(widgets::label(
                        &format!(
                            "YOUR PARTY · {}/{MAX_PARTY_SIZE}",
                            view.member_count()
                        ),
                        12.0,
                        theme::GOLD,
                    ));
                    column.spawn((
                        ImageNode::new(image),
                        Node {
                            width: Val::Percent(100.0),
                            aspect_ratio: Some(STAGE_WIDTH as f32 / STAGE_HEIGHT as f32),
                            border_radius: BorderRadius::all(Val::Px(14.0)),
                            ..default()
                        },
                        Name::new("LobbyStageImage"),
                    ));
                    column
                        .spawn(Node {
                            column_gap: Val::Px(10.0),
                            justify_content: JustifyContent::Center,
                            flex_wrap: FlexWrap::Wrap,
                            width: Val::Percent(100.0),
                            ..default()
                        })
                        .with_children(|plates| match &view.party {
                            Some(party) => {
                                for member in &party.members {
                                    spawn_plate(plates, view, member);
                                }
                            }
                            None => {
                                plates
                                    .spawn((widgets::panel_row(), Name::new("LobbyPlateSelf")))
                                    .with_children(|plate| {
                                        plate.spawn(widgets::label(&own_name, 16.0, theme::IVORY));
                                        plate.spawn(widgets::label(
                                            "Solo · invite a friend to team up",
                                            12.0,
                                            theme::MUTED,
                                        ));
                                    });
                            }
                        });
                    // Play buttons under the line-up.
                    column
                        .spawn(Node {
                            column_gap: Val::Px(12.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            if view.can_launch() {
                                screen_button(
                                    row,
                                    "PLAY VS BOTS",
                                    ButtonKind::Primary,
                                    LobbyAction::Play(MatchPreference::BotPractice),
                                    "LobbyPlayBots",
                                );
                                if sig.public {
                                    screen_button(
                                        row,
                                        "Quick match",
                                        ButtonKind::Secondary,
                                        LobbyAction::Play(MatchPreference::Quick),
                                        "LobbyQuickMatch",
                                    );
                                }
                            } else {
                                let leader = view
                                    .party
                                    .as_ref()
                                    .and_then(|p| p.members.iter().find(|m| m.leader))
                                    .map_or("the leader", |m| m.nickname.as_str());
                                row.spawn(widgets::label(
                                    &format!("Waiting for {leader} to start the match…"),
                                    15.0,
                                    theme::GOLD,
                                ));
                            }
                        });
                    column.spawn(widgets::label(
                        if view.party.is_some() {
                            "The whole party joins one team. Bots take every empty seat."
                        } else {
                            "Bots take every empty seat. Invite a friend to share your team."
                        },
                        12.0,
                        theme::MUTED,
                    ));
                });

                // Invites and online players.
                body.spawn((
                    Node {
                        width: Val::Px(if phone { 280.0 } else { 340.0 }),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(10.0),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Name::new("LobbySocial"),
                ))
                .with_children(|column| {
                    // A phone reaches the server through its SERVER keypad.
                    if !phone {
                        super::server_field::spawn_server_field(column, &sig.field, &sig.server);
                    }
                    if !view.invites.is_empty() {
                        column.spawn(widgets::label("INVITES", 12.0, theme::GOLD));
                        for invite in &view.invites {
                            column
                                .spawn((widgets::panel_row(), Name::new("LobbyInvite")))
                                .with_children(|panel| {
                                    panel.spawn(widgets::label(
                                        &format!("{} invites you", invite.from_nickname),
                                        15.0,
                                        theme::IVORY,
                                    ));
                                    panel
                                        .spawn(Node {
                                            column_gap: Val::Px(8.0),
                                            margin: UiRect::top(Val::Px(6.0)),
                                            ..default()
                                        })
                                        .with_children(|row| {
                                            screen_button(
                                                row,
                                                "Accept",
                                                ButtonKind::Secondary,
                                                LobbyAction::Accept(invite.party_id),
                                                format!("LobbyAccept{}", invite.party_id),
                                            );
                                            screen_button(
                                                row,
                                                "Decline",
                                                ButtonKind::Secondary,
                                                LobbyAction::Decline(invite.party_id),
                                                format!("LobbyDecline{}", invite.party_id),
                                            );
                                        });
                                });
                        }
                    }
                    column.spawn(widgets::label("ONLINE ON THIS SERVER", 12.0, theme::GOLD));
                    if !sig.live {
                        column.spawn(widgets::label(
                            "This server does not host parties (or is not answering). Update the server, or reconnect.",
                            13.0,
                            theme::MUTED,
                        ));
                    } else if view.online.is_empty() {
                        column.spawn(widgets::label(
                            &format!(
                                "Nobody else is here yet. Ask your friend to connect to\n{}",
                                sig.server
                            ),
                            13.0,
                            theme::MUTED,
                        ));
                    }
                    for player in &view.online {
                        spawn_online_row(column, view, player);
                    }
                });
            });
        });
}

fn spawn_plate(
    parent: &mut ChildSpawnerCommands,
    view: &PartyView,
    member: &shared::party::PartyMember,
) {
    let (status, color) = member_status(member);
    parent
        .spawn((widgets::panel_row(), Name::new("LobbyPlate")))
        .with_children(|plate| {
            let you = if member.player_id == view.you {
                " (you)"
            } else {
                ""
            };
            plate.spawn(widgets::label(
                &format!("{}{you}", member.nickname),
                16.0,
                theme::IVORY,
            ));
            plate.spawn(widgets::label(
                &if member.leader {
                    format!("★ Leader · {status}")
                } else {
                    status.to_owned()
                },
                12.0,
                if member.leader { theme::GOLD } else { color },
            ));
            if view.is_leader() && member.player_id != view.you {
                screen_button(
                    plate,
                    "Kick",
                    ButtonKind::Danger,
                    LobbyAction::Kick(member.player_id),
                    format!("LobbyKick{}", member.player_id),
                );
            }
        });
}

fn spawn_online_row(parent: &mut ChildSpawnerCommands, view: &PartyView, player: &OnlinePlayer) {
    parent
        .spawn((
            Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(theme::TILE),
            BorderColor::all(theme::PANEL_EDGE),
            Name::new("LobbyOnlinePlayer"),
        ))
        .with_children(|row| {
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                min_width: Val::Px(0.0),
                ..default()
            })
            .with_children(|name| {
                name.spawn(widgets::label(&player.nickname, 15.0, theme::IVORY));
                let mut tags = Vec::new();
                if player.friend {
                    tags.push("★ Friend");
                }
                if player.in_match {
                    tags.push("In match");
                } else if player.in_party {
                    tags.push("In a party");
                }
                if !tags.is_empty() {
                    name.spawn(widgets::label(&tags.join(" · "), 11.0, theme::MUTED));
                }
            });
            match invite_offer(view, player) {
                InviteOffer::Invite => {
                    screen_button(
                        row,
                        "Invite",
                        ButtonKind::Secondary,
                        LobbyAction::Invite(player.player_id),
                        format!("LobbyInvite{}", player.player_id),
                    );
                }
                InviteOffer::Invited => {
                    row.spawn(widgets::label("Invited", 13.0, theme::GOLD));
                }
                InviteOffer::Member => {
                    row.spawn(widgets::label("In party", 13.0, theme::JADE));
                }
                InviteOffer::Unavailable => {}
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn lobby_actions(
    mut activated: MessageReader<Activated<LobbyAction>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut requests: MessageWriter<NetworkCommand>,
    mut session_ui: MessageWriter<crate::net::SessionUiCommand>,
    mut matchmaking: ResMut<crate::match_service::MatchServiceClient>,
    session: Res<ClientSession>,
    party: Res<PartyClient>,
    mut career: ResMut<crate::career::CareerClient>,
) {
    for Activated { action, .. } in activated.read() {
        match *action {
            LobbyAction::Back => next.set(AppScreen::Home),
            LobbyAction::Friends => crate::career::open_friends_modal(&mut career, &mut requests),
            LobbyAction::Play(preference) => {
                if party.view.is_leader() {
                    // The launch reaches every member, this client included,
                    // and moves them all to hero select (`crate::party`).
                    requests.write(NetworkCommand::Party(PartyCommand::Launch { preference }));
                } else if party.view.party.is_none() {
                    if session.is_offline() {
                        session_ui.write(crate::net::SessionUiCommand::LeaveMatch);
                    }
                    matchmaking.preference = preference;
                    next.set(AppScreen::HeroSelect);
                }
            }
            LobbyAction::Leave => {
                requests.write(NetworkCommand::Party(PartyCommand::Leave));
            }
            LobbyAction::Invite(player_id) => {
                requests.write(NetworkCommand::Party(PartyCommand::Invite { player_id }));
            }
            LobbyAction::Accept(party_id) => {
                requests.write(NetworkCommand::Party(PartyCommand::Accept { party_id }));
            }
            LobbyAction::Decline(party_id) => {
                requests.write(NetworkCommand::Party(PartyCommand::Decline { party_id }));
            }
            LobbyAction::Kick(player_id) => {
                requests.write(NetworkCommand::Party(PartyCommand::Kick { player_id }));
            }
        }
    }
}

/// The view arrives every half second; rebuild only when what the screen
/// renders changed.
#[allow(clippy::too_many_arguments)]
fn refresh_lobby(
    mut commands: Commands,
    party: Res<PartyClient>,
    career: Res<crate::career::CareerClient>,
    session: Res<ClientSession>,
    card: Res<super::card::ProfileCard>,
    selection: Res<crate::team::TeamSelection>,
    stage: ResMut<PartyStage>,
    platform: Res<crate::ui::UiPlatform>,
    field: Res<super::server_field::ServerField>,
    roots: Query<Entity, With<LobbyRoot>>,
    mut last: Local<Option<LobbySignature>>,
) {
    let mut current = signature(&party, &career, &session, &field);
    // Invite countdowns tick every second; they are not worth a rebuild.
    for invite in &mut current.view.invites {
        invite.expires_in_secs = 0;
    }
    if last.as_ref() == Some(&current) {
        return;
    }
    let first = last.is_none();
    *last = Some(current);
    if first {
        return;
    }
    let Ok(root) = roots.single() else {
        return;
    };
    commands
        .entity(root)
        .despawn_related::<Children>()
        .despawn();
    spawn_lobby(
        commands, party, career, session, card, selection, stage, platform, field,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::party::{PartyInfo, PartyMember};

    fn member(id: u64, leader: bool) -> PartyMember {
        PartyMember {
            player_id: id,
            nickname: format!("P{id}"),
            avatar: None,
            leader,
            in_match: false,
            away: false,
        }
    }

    fn online(id: u64) -> OnlinePlayer {
        OnlinePlayer {
            player_id: id,
            nickname: format!("P{id}"),
            avatar: None,
            friend: false,
            in_party: false,
            in_match: false,
            invited: false,
        }
    }

    fn party_view(you: u64, leader: u64, ids: &[u64]) -> PartyView {
        PartyView {
            you,
            party: Some(PartyInfo {
                party_id: 1,
                leader,
                members: ids.iter().map(|&id| member(id, id == leader)).collect(),
                launch: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_solo_player_or_the_leader_invites_and_members_do_not() {
        let solo = PartyView {
            you: 1,
            ..Default::default()
        };
        assert_eq!(invite_offer(&solo, &online(2)), InviteOffer::Invite);
        let leader = party_view(1, 1, &[1, 2]);
        assert_eq!(invite_offer(&leader, &online(2)), InviteOffer::Member);
        assert_eq!(invite_offer(&leader, &online(3)), InviteOffer::Invite);
        let mut invited = online(3);
        invited.invited = true;
        assert_eq!(invite_offer(&leader, &invited), InviteOffer::Invited);
        let member = party_view(2, 1, &[1, 2]);
        assert_eq!(invite_offer(&member, &online(3)), InviteOffer::Unavailable);
        let full = party_view(1, 1, &[1, 2, 3, 4, 5]);
        assert_eq!(invite_offer(&full, &online(6)), InviteOffer::Unavailable);
    }

    #[test]
    fn the_stage_shows_the_party_or_the_player_alone() {
        let solo = PartyView::default();
        assert_eq!(
            stage_members(&solo, Some("agnes".into())),
            vec![StageMember {
                avatar: Some("agnes".into()),
                leader: true
            }]
        );
        let view = party_view(2, 1, &[1, 2]);
        let staged = stage_members(&view, None);
        assert_eq!(staged.len(), 2);
        assert!(staged[0].leader && !staged[1].leader);
    }

    #[test]
    fn member_status_reads_away_match_or_ready() {
        let mut m = member(1, false);
        assert_eq!(member_status(&m).0, "Ready");
        m.in_match = true;
        assert_eq!(member_status(&m).0, "In match");
        m.away = true;
        assert_eq!(member_status(&m).0, "Away");
    }
}
