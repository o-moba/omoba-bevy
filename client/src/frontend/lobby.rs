//! Party lobby: the party's avatars on stage, invites in and out, and the
//! leader's "play together" buttons.
//!
//! Everything shown comes from the server's [`shared::party::PartyView`]
//! (`crate::party::PartyClient`); buttons only send `PartyCommand`s. The
//! launch itself moves every member to hero select (`crate::party`).
//!
//! Text comes from the `lobby` dictionary; the render key includes the
//! locale generation, so a language change rebuilds the screen.
// i18n-strict

use bevy::prelude::*;
use shared::match_service::MatchPreference;
use shared::party::{MAX_PARTY_SIZE, OnlinePlayer, PartyCommand, PartyView};

use super::party_stage::{PartyStage, STAGE_HEIGHT, STAGE_WIDTH, StageMember, StageSurface};
use super::{AppScreen, automation_bypass, widgets};
use crate::i18n::{Locale, tr, trf};
use crate::net::{ClientSession, NetworkCommand};
use crate::party::PartyClient;
use crate::ui::living_background::{self, LivingBands, LivingScene};
use crate::ui::theme::TextStyle;
use crate::ui::theme::{self, ButtonKind};
use crate::ui::tokens::{Metric, TextRole};
use crate::ui::widgets as kit;
use crate::ui::{Activated, UiActionAppExt, UiSet};

pub struct LobbyScreenPlugin;

impl Plugin for LobbyScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_ui_action::<LobbyAction>()
            .add_systems(OnEnter(AppScreen::Lobby), (spawn_backdrop, spawn_lobby))
            .add_systems(
                PostUpdate,
                sync_scrollbar.after(bevy::ui::UiSystems::Layout),
            )
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

/// Keep player identity separate from party leadership. The viewer always
/// occupies the foreground slot, with the other members in stable server order.
fn ordered_members(view: &PartyView) -> Vec<&shared::party::PartyMember> {
    let mut members: Vec<_> = view.party.iter().flat_map(|p| &p.members).collect();
    members.sort_by_key(|member| member.player_id != view.you);
    members.truncate(MAX_PARTY_SIZE);
    members
}

pub(crate) fn stage_members(
    view: &PartyView,
    own_avatar: Option<String>,
    character: crate::team::CharacterChoice,
) -> Vec<StageMember> {
    if view.party.is_none() {
        return vec![StageMember {
            hero_class: Default::default(),
            handheld: shared::handheld::HandheldSelection::Unequipped,
            avatar: own_avatar,
            character,
            leader: true,
            revealed: true,
        }];
    }
    ordered_members(view)
        .iter()
        .map(|member| StageMember {
            hero_class: Default::default(),
            handheld: shared::handheld::HandheldSelection::Unequipped,
            avatar: member.avatar.clone().or_else(|| {
                (member.player_id == view.you)
                    .then(|| own_avatar.clone())
                    .flatten()
            }),
            character: if member.player_id == view.you {
                character
            } else {
                default()
            },
            leader: member.leader,
            revealed: true,
        })
        .collect()
}

fn member_status(member: &shared::party::PartyMember) -> (&'static str, Color) {
    if member.away {
        (tr("lobby.status.away"), theme::MUTED)
    } else if member.in_match {
        (tr("lobby.status.in_match"), theme::GOLD)
    } else {
        (tr("lobby.status.in_lobby"), theme::JADE)
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
    /// The locale generation: a language change rebuilds the screen.
    locale: u32,
    viewport: (u32, u32),
    scale: u32,
    avatar: Option<String>,
    character: crate::team::CharacterChoice,
}

#[allow(clippy::too_many_arguments)]
fn signature(
    party: &PartyClient,
    career: &crate::career::CareerClient,
    session: &ClientSession,
    field: &super::server_field::ServerField,
    locale: Option<&Locale>,
    viewport: Vec2,
    scale: f32,
    avatar: Option<String>,
    character: crate::team::CharacterChoice,
) -> LobbySignature {
    LobbySignature {
        view: party.view.clone(),
        live: party.is_live(std::time::Instant::now()),
        public: career.view.match_service.is_some(),
        friends: career.view.storage_enabled,
        server: session.server_addr().to_owned(),
        field: field.clone(),
        locale: locale.map_or(0, Locale::generation),
        viewport: (viewport.x.to_bits(), viewport.y.to_bits()),
        scale: scale.to_bits(),
        avatar,
        character,
    }
}

// The painted background survives presence updates, preventing fade-in flicker.
fn spawn_backdrop(mut commands: Commands, platform: Res<crate::ui::UiPlatform>) {
    commands
        .spawn(widgets::screen_root(AppScreen::Lobby, "LobbyBackdrop"))
        .insert(ZIndex(theme::SCREEN_Z - 1))
        .with_children(|root| {
            living_background::spawn(
                root,
                LivingScene::Arena,
                LivingBands::default(),
                theme::Form::of(platform.is_mobile()),
            );
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    ..default()
                },
                BackgroundGradient::from(LinearGradient::to_bottom(vec![
                    ColorStop::percent(Color::srgba(0.02, 0.07, 0.065, 0.88), 0.0),
                    ColorStop::percent(Color::srgba(0.02, 0.07, 0.065, 0.28), 40.0),
                    ColorStop::percent(Color::srgba(0.02, 0.07, 0.065, 0.94), 100.0),
                ])),
                Pickable::IGNORE,
            ));
        });
}

fn lobby_canvas(mobile: bool, viewport: Vec2, scale: f32) -> (bool, Node, UiTransform) {
    let phone = mobile && viewport.y < 600.0;
    let reference = if phone {
        Vec2::new(844.0, 390.0)
    } else {
        Vec2::new(1280.0, 720.0)
    };
    let scale = scale.max(0.1);
    let fit = (viewport.x / reference.x).min(viewport.y / reference.y);
    (
        phone,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px((viewport.x / scale - reference.x) * 0.5),
            top: Val::Px((viewport.y / scale - reference.y) * 0.5),
            width: Val::Px(reference.x),
            height: Val::Px(reference.y),
            padding: if phone {
                UiRect {
                    left: Val::Px(32.0),
                    right: Val::Px(32.0),
                    top: Val::Px(18.0),
                    bottom: Val::Px(20.0),
                }
            } else {
                UiRect::all(Val::Px(40.0))
            },
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(if phone { 10.0 } else { 20.0 }),
            ..default()
        },
        UiTransform::from_scale(Vec2::splat(fit / scale)),
    )
}

fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        theme::styled_text(TextStyle::keep_case(TextRole::Body).sized(Metric::new(size, size))),
        TextColor(color),
        Pickable::IGNORE,
    )
}

fn button<T: crate::ui::action::UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    width: f32,
    kind: ButtonKind,
    action: T,
    id: impl Into<crate::ui::TestId>,
) -> Entity {
    kit::spawn_button(
        parent,
        Node {
            width: Val::Px(width),
            height: Val::Px(48.0),
            min_height: Val::Px(48.0),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(Val::Px(10.0)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        text,
        TextStyle::keep_case(TextRole::Button).sized(Metric::new(14.0, 13.0)),
        kind,
        None,
        action,
        id.into(),
        (),
    )
}

#[derive(Component)]
struct LobbySocialScroll;
#[derive(Component)]
struct LobbyScrollTrack;
#[derive(Component)]
struct LobbyScrollThumb;

fn social_scroll() -> crate::ui::ScrollArea {
    crate::ui::ScrollArea::wheel(48.0)
        .hover_only()
        .page_keys(180.0)
        .touch_drag(8.0)
        .keyed(0x5041525459)
}

fn sync_scrollbar(
    panels: Query<(&ComputedNode, &ScrollPosition), With<LobbySocialScroll>>,
    mut tracks: Query<&mut Visibility, With<LobbyScrollTrack>>,
    mut thumbs: Query<&mut Node, With<LobbyScrollThumb>>,
) {
    let Ok((node, scroll)) = panels.single() else {
        return;
    };
    let height = node.size().y * node.inverse_scale_factor();
    let content = node.content_size().y * node.inverse_scale_factor();
    let overflow = (content - height).max(0.0);
    for mut visibility in &mut tracks {
        *visibility = if overflow > 1.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    let ratio = (height / content.max(1.0)).clamp(0.08, 1.0);
    for mut node in &mut thumbs {
        node.height = Val::Percent(ratio * 100.0);
        node.top =
            Val::Percent((scroll.y / overflow.max(1.0)).clamp(0.0, 1.0) * (1.0 - ratio) * 100.0);
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
    locale: Option<Res<Locale>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    scale: Res<UiScale>,
) {
    if automation_bypass() {
        return;
    }
    let viewport = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let (phone, canvas, transform) = lobby_canvas(platform.is_mobile(), viewport, scale.0);
    let sig = signature(
        &party,
        &career,
        &session,
        &field,
        locale.as_deref(),
        viewport,
        scale.0,
        crate::party::presence_avatar(&card, &selection),
        selection.character,
    );
    let view = &sig.view;
    stage.members = stage_members(view, sig.avatar.clone(), sig.character);
    let own_name = career
        .view
        .profile
        .as_ref()
        .map_or_else(|| career.nickname.clone(), |p| p.nickname.clone());
    commands
        .spawn((
            canvas,
            transform,
            LobbyRoot,
            ZIndex(theme::SCREEN_Z),
            bevy::state::state_scoped::DespawnOnExit(AppScreen::Lobby),
            Name::new("LobbyScreen"),
        ))
        .with_children(|root| {
            root.spawn(Node {
                height: Val::Px(if phone { 44.0 } else { 56.0 }),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            })
            .with_children(|header| {
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(4.0),
                        ..default()
                    })
                    .with_children(|title| {
                        title.spawn((
                            Text::new(tr("lobby.title")),
                            theme::styled_text(
                                TextStyle::new(TextRole::Title).sized(Metric::new(32.0, 24.0)),
                            ),
                            TextColor(theme::IVORY),
                        ));
                        if !phone {
                            title.spawn(label(tr("lobby.subtitle"), 14.0, theme::MUTED));
                        }
                    });
                header
                    .spawn(Node {
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|actions| {
                        if view.party.is_some() {
                            button(
                                actions,
                                tr("lobby.button.leave"),
                                if phone { 112.0 } else { 136.0 },
                                ButtonKind::Link,
                                LobbyAction::Leave,
                                "LobbyLeave",
                            );
                        }
                        button(
                            actions,
                            tr("common.back"),
                            96.0,
                            ButtonKind::Secondary,
                            LobbyAction::Back,
                            "LobbyBack",
                        );
                    });
            });
            root.spawn(Node {
                flex_grow: 1.0,
                min_height: Val::Px(0.0),
                column_gap: Val::Px(if phone { 16.0 } else { 28.0 }),
                ..default()
            })
            .with_children(|body| {
                body.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        min_width: Val::Px(0.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    Name::new("LobbyStage"),
                ))
                .with_children(|column| {
                    column
                        .spawn(Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(if phone { 18.0 } else { 28.0 }),
                            flex_shrink: 0.0,
                            justify_content: JustifyContent::SpaceBetween,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn(label(
                                trf(
                                    "lobby.stage.count",
                                    &[("count", &view.member_count()), ("max", &MAX_PARTY_SIZE)],
                                ),
                                if phone { 11.0 } else { 13.0 },
                                theme::GOLD,
                            ));
                            row.spawn((
                                label(
                                    tr("lobby.stage.rotate"),
                                    if phone { 11.0 } else { 13.0 },
                                    theme::IVORY,
                                ),
                                Node {
                                    padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.02, 0.06, 0.05, 0.82)),
                            ));
                        });
                    column
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_grow: 1.0,
                                min_height: Val::Px(0.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            Name::new("LobbyStageViewport"),
                        ))
                        .with_children(|surface| {
                            surface.spawn((
                                ImageNode::new(stage.image.clone()),
                                Node {
                                    width: Val::Px(if phone { 366.0 } else { 760.0 }),
                                    height: Val::Px(
                                        if phone { 366.0 } else { 760.0 } * STAGE_HEIGHT as f32
                                            / STAGE_WIDTH as f32,
                                    ),
                                    flex_shrink: 0.0,
                                    ..default()
                                },
                                StageSurface,
                                Name::new("LobbyStageImage"),
                            ));
                        });
                    spawn_nameplates(column, view, &own_name, phone);
                    column
                        .spawn(Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(if phone { 3.0 } else { 10.0 }),
                            margin: UiRect::top(Val::Px(if phone { 6.0 } else { 16.0 })),
                            flex_shrink: 0.0,
                            ..default()
                        })
                        .with_children(|footer| {
                            footer
                                .spawn(Node {
                                    column_gap: Val::Px(10.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    height: Val::Px(48.0),
                                    ..default()
                                })
                                .with_children(|actions| {
                                    if view.can_launch() {
                                        button(
                                            actions,
                                            tr("lobby.button.play_bots"),
                                            if phone { 212.0 } else { 272.0 },
                                            ButtonKind::Primary,
                                            LobbyAction::Play(MatchPreference::BotPractice),
                                            "LobbyPlayBots",
                                        );
                                        if sig.public {
                                            button(
                                                actions,
                                                tr("lobby.button.quick_match"),
                                                if phone { 160.0 } else { 208.0 },
                                                ButtonKind::Secondary,
                                                LobbyAction::Play(MatchPreference::Quick),
                                                "LobbyQuickMatch",
                                            );
                                        }
                                    } else {
                                        let waiting = view
                                            .party
                                            .as_ref()
                                            .and_then(|p| p.members.iter().find(|m| m.leader))
                                            .map_or_else(
                                                || tr("lobby.waiting_for_leader").to_owned(),
                                                |m| {
                                                    trf(
                                                        "lobby.waiting_for",
                                                        &[("leader", &m.nickname)],
                                                    )
                                                },
                                            );
                                        actions.spawn((
                                            label(
                                                waiting,
                                                if phone { 12.0 } else { 16.0 },
                                                theme::GOLD,
                                            ),
                                            Node {
                                                max_width: Val::Px(if phone {
                                                    490.0
                                                } else {
                                                    780.0
                                                }),
                                                ..default()
                                            },
                                            TextLayout::new(Justify::Center, LineBreak::NoWrap),
                                        ));
                                    }
                                });
                            if !phone {
                                footer.spawn(label(
                                    tr(if view.party.is_some() {
                                        "lobby.hint.party"
                                    } else {
                                        "lobby.hint.solo"
                                    }),
                                    13.0,
                                    theme::MUTED,
                                ));
                            }
                        });
                });
                spawn_social(body, &sig, phone);
            });
        });
}

fn spawn_nameplates(
    parent: &mut ChildSpawnerCommands,
    view: &PartyView,
    own_name: &str,
    phone: bool,
) {
    let ordered = ordered_members(view);
    let count = ordered.len().max(1);
    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Px(if phone { 35.0 } else { 56.0 }),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|row| {
            for index in 0..count {
                let member = ordered.get(index);
                let you = member.is_none_or(|m| m.player_id == view.you);
                let name = member.map_or(own_name, |m| m.nickname.as_str());
                let (status, color) =
                    member.map_or((tr("lobby.solo_hint"), theme::MUTED), |m| member_status(m));
                let status = if member.is_some_and(|m| m.leader) {
                    tr("lobby.stage.leader")
                } else if you {
                    tr("lobby.stage.you")
                } else {
                    status
                };
                let width = if phone {
                    if count > 3 { 76.0 } else { 148.0 }
                } else if count > 3 {
                    112.0
                } else {
                    220.0
                };
                let x = super::party_stage::slot_anchor(index, count).x;
                row.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(x * 100.0),
                        width: Val::Px(width),
                        height: Val::Percent(100.0),
                        margin: UiRect::left(Val::Px(-width * 0.5)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        padding: UiRect::top(Val::Px(if phone { 2.0 } else { 6.0 })),
                        border: UiRect::top(Val::Px(if you { 2.0 } else { 1.0 })),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BorderColor::all(if you { theme::GOLD } else { theme::PANEL_EDGE }),
                    BackgroundColor(Color::srgba(0.02, 0.06, 0.05, 0.82)),
                    Name::new(format!("LobbyMemberPlate{index}")),
                ))
                .with_children(|plate| {
                    plate.spawn((
                        label(
                            plate_name(name, width, if phone { 12.0 } else { 16.0 }),
                            if phone { 12.0 } else { 16.0 },
                            theme::IVORY,
                        ),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                    ));
                    plate.spawn((
                        label(
                            status,
                            if phone { 10.0 } else { 12.0 },
                            if you { theme::GOLD } else { color },
                        ),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                    ));
                });
            }
        });
}

/// Conservative glyph budget keeps the beginning and a visible ellipsis.
/// Full names remain in the scrolling member list. CJK/wide glyphs use a full em.
fn plate_name(name: &str, width: f32, size: f32) -> String {
    let weight = |c: char| {
        if c as u32 >= 0x2e80 {
            1.05
        } else if c.is_whitespace() {
            0.35
        } else if c.is_uppercase() {
            0.72
        } else {
            0.63
        }
    };
    let budget = (width - 8.0) / size;
    if name.chars().map(weight).sum::<f32>() <= budget {
        return name.to_owned();
    }
    let mut used = 1.0; // reserve the ellipsis
    let mut result = String::new();
    for ch in name.chars() {
        used += weight(ch);
        if used > budget {
            break;
        }
        result.push(ch);
    }
    result.push('…');
    result
}

fn spawn_social(parent: &mut ChildSpawnerCommands, sig: &LobbySignature, phone: bool) {
    let view = &sig.view;
    parent
        .spawn((
            Node {
                width: Val::Px(if phone { 258.0 } else { 336.0 }),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(if phone { 12.0 } else { 20.0 })),
                row_gap: Val::Px(12.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.075, 0.067, 0.95)),
            BorderColor::all(theme::PANEL_EDGE),
            Name::new("LobbySocial"),
        ))
        .with_children(|panel| {
            panel
                .spawn(Node {
                    flex_shrink: 0.0,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|header| {
                    header.spawn(label(
                        tr("lobby.social.title"),
                        if phone { 16.0 } else { 20.0 },
                        theme::GOLD,
                    ));
                    if sig.friends {
                        button(
                            header,
                            tr("lobby.button.friends"),
                            if phone { 118.0 } else { 132.0 },
                            ButtonKind::Link,
                            LobbyAction::Friends,
                            "LobbyFriends",
                        );
                    }
                });
            panel
                .spawn(Node {
                    flex_grow: 1.0,
                    min_height: Val::Px(0.0),
                    ..default()
                })
                .with_children(|body| {
                    body.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            min_height: Val::Px(0.0),
                            padding: UiRect::right(Val::Px(10.0)),
                            row_gap: Val::Px(10.0),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                        social_scroll(),
                        LobbySocialScroll,
                        Name::new("LobbySocialScroll"),
                    ))
                    .with_children(|column| {
                        for invite in &view.invites {
                            column
                                .spawn((
                                    social_card(),
                                    BackgroundColor(theme::TILE),
                                    Name::new("LobbyInvite"),
                                ))
                                .with_children(|panel| {
                                    panel.spawn(label(
                                        trf(
                                            "lobby.invite_from",
                                            &[("name", &invite.from_nickname)],
                                        ),
                                        13.0,
                                        theme::IVORY,
                                    ));
                                    panel
                                        .spawn(Node {
                                            column_gap: Val::Px(6.0),
                                            ..default()
                                        })
                                        .with_children(|row| {
                                            button(
                                                row,
                                                tr("lobby.button.accept"),
                                                if phone { 96.0 } else { 118.0 },
                                                ButtonKind::Primary,
                                                LobbyAction::Accept(invite.party_id),
                                                format!("LobbyAccept{}", invite.party_id),
                                            );
                                            button(
                                                row,
                                                tr("lobby.button.decline"),
                                                if phone { 96.0 } else { 118.0 },
                                                ButtonKind::Link,
                                                LobbyAction::Decline(invite.party_id),
                                                format!("LobbyDecline{}", invite.party_id),
                                            );
                                        });
                                });
                        }
                        if let Some(party) = &view.party {
                            column.spawn(label(tr("lobby.social.members"), 11.0, theme::GOLD));
                            for member in &party.members {
                                column
                                    .spawn((social_card(), BackgroundColor(theme::TILE)))
                                    .with_children(|card| {
                                        card.spawn(Node {
                                            align_items: AlignItems::Center,
                                            justify_content: JustifyContent::SpaceBetween,
                                            column_gap: Val::Px(6.0),
                                            ..default()
                                        })
                                        .with_children(
                                            |row| {
                                                row.spawn((
                                                    label(&member.nickname, 13.0, theme::IVORY),
                                                    Node {
                                                        flex_grow: 1.0,
                                                        flex_basis: Val::Px(0.0),
                                                        min_width: Val::Px(0.0),
                                                        ..default()
                                                    },
                                                ));
                                                if view.is_leader() && member.player_id != view.you
                                                {
                                                    button(
                                                        row,
                                                        tr("lobby.button.kick"),
                                                        72.0,
                                                        ButtonKind::Link,
                                                        LobbyAction::Kick(member.player_id),
                                                        format!("LobbyKick{}", member.player_id),
                                                    );
                                                }
                                            },
                                        );
                                        let (status, color) = member_status(member);
                                        card.spawn(label(
                                            if member.leader {
                                                trf("lobby.plate.leader", &[("status", &status)])
                                            } else {
                                                status.into()
                                            },
                                            11.0,
                                            color,
                                        ));
                                    });
                            }
                            if party.members.len() == MAX_PARTY_SIZE {
                                column.spawn(label(tr("lobby.social.full"), 12.0, theme::GOLD));
                            }
                        }
                        column.spawn(label(tr("lobby.online.title"), 11.0, theme::GOLD));
                        if !sig.live {
                            column.spawn(label(tr("lobby.online.unavailable"), 13.0, theme::MUTED));
                        } else if view.online.is_empty() {
                            column.spawn(label(tr("lobby.online.empty_short"), 13.0, theme::MUTED));
                        }
                        for player in &view.online {
                            spawn_online_row(column, view, player);
                        }
                        // Connection details are secondary and scroll with the social list.
                        column
                            .spawn(Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                margin: UiRect::top(Val::Px(8.0)),
                                flex_shrink: 0.0,
                                ..default()
                            })
                            .with_children(|server| {
                                server.spawn(label(tr("lobby.server.title"), 11.0, theme::MUTED));
                                server.spawn(label(
                                    if sig.field.editing {
                                        format!("{}▏", sig.field.text)
                                    } else {
                                        sig.server.clone()
                                    },
                                    12.0,
                                    theme::MUTED,
                                ));
                                if sig.field.editing {
                                    server
                                        .spawn(Node {
                                            column_gap: Val::Px(6.0),
                                            flex_wrap: FlexWrap::Wrap,
                                            ..default()
                                        })
                                        .with_children(|row| {
                                            for (preset, key, id) in [
                                                (
                                                    crate::session_config::ServerPreset::Beta,
                                                    "phone.server.beta",
                                                    "LobbyServerBeta",
                                                ),
                                                (
                                                    crate::session_config::ServerPreset::Local,
                                                    "phone.server.local",
                                                    "LobbyServerLocal",
                                                ),
                                            ] {
                                                button(
                                                    row,
                                                    tr(key),
                                                    112.0,
                                                    ButtonKind::Secondary,
                                                    super::server_field::ServerFieldAction::Preset(
                                                        preset,
                                                    ),
                                                    id,
                                                );
                                            }
                                        });
                                    server.spawn(label(
                                        tr("lobby.server.hint"),
                                        11.0,
                                        theme::MUTED,
                                    ));
                                    if let Some(error) = sig.field.error {
                                        server.spawn(label(tr(error), 12.0, theme::DANGER_HOVER));
                                    }
                                    server
                                        .spawn(Node {
                                            column_gap: Val::Px(6.0),
                                            ..default()
                                        })
                                        .with_children(|row| {
                                            button(
                                                row,
                                                tr("lobby.server.connect"),
                                                100.0,
                                                ButtonKind::Secondary,
                                                super::server_field::ServerFieldAction::Connect,
                                                "LobbyServerConnect",
                                            );
                                            button(
                                                row,
                                                tr("common.cancel"),
                                                96.0,
                                                ButtonKind::Link,
                                                super::server_field::ServerFieldAction::Cancel,
                                                "LobbyServerCancel",
                                            );
                                        });
                                } else if !phone {
                                    button(
                                        server,
                                        tr("lobby.server.change"),
                                        112.0,
                                        ButtonKind::Link,
                                        super::server_field::ServerFieldAction::Edit,
                                        "LobbyServerChange",
                                    );
                                }
                            });
                    });
                    body.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(0.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            width: Val::Px(3.0),
                            ..default()
                        },
                        BackgroundColor(theme::PANEL_EDGE),
                        LobbyScrollTrack,
                        Pickable::IGNORE,
                    ))
                    .with_children(|track| {
                        track.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                width: Val::Percent(100.0),
                                height: Val::Percent(40.0),
                                ..default()
                            },
                            BackgroundColor(theme::GOLD),
                            LobbyScrollThumb,
                            Pickable::IGNORE,
                        ));
                    });
                });
        });
}

fn social_card() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        flex_shrink: 0.0,
        padding: UiRect::all(Val::Px(8.0)),
        row_gap: Val::Px(4.0),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}

fn spawn_online_row(parent: &mut ChildSpawnerCommands, view: &PartyView, player: &OnlinePlayer) {
    parent
        .spawn((
            social_card(),
            BackgroundColor(theme::TILE),
            Name::new("LobbyOnlinePlayer"),
        ))
        .with_children(|card| {
            card.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    label(&player.nickname, 13.0, theme::IVORY),
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        min_width: Val::Px(0.0),
                        ..default()
                    },
                ));
                match invite_offer(view, player) {
                    InviteOffer::Invite => {
                        button(
                            row,
                            tr("lobby.button.invite"),
                            84.0,
                            ButtonKind::Secondary,
                            LobbyAction::Invite(player.player_id),
                            format!("LobbyInvite{}", player.player_id),
                        );
                    }
                    InviteOffer::Invited => {
                        row.spawn(label(tr("lobby.invited"), 11.0, theme::GOLD));
                    }
                    InviteOffer::Member => {
                        row.spawn(label(tr("lobby.in_party"), 11.0, theme::JADE));
                    }
                    InviteOffer::Unavailable => {}
                }
            });
            let tags: Vec<_> = [
                (player.friend, "lobby.tag.friend"),
                (player.in_match, "lobby.status.in_match"),
                (player.in_party && !player.in_match, "lobby.tag.in_a_party"),
            ]
            .into_iter()
            .filter(|(show, _)| *show)
            .map(|(_, key)| tr(key))
            .collect();
            if !tags.is_empty() {
                card.spawn(label(tags.join(" · "), 11.0, theme::MUTED));
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
    locale: Option<Res<Locale>>,
    mut last: Local<Option<LobbySignature>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    scale: Res<UiScale>,
) {
    let viewport = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let mut current = signature(
        &party,
        &career,
        &session,
        &field,
        locale.as_deref(),
        viewport,
        scale.0,
        crate::party::presence_avatar(&card, &selection),
        selection.character,
    );
    // Invite countdowns tick every second; they are not worth a rebuild.
    for invite in &mut current.view.invites {
        invite.expires_in_secs = 0;
    }
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
    spawn_lobby(
        commands, party, career, session, card, selection, stage, platform, field, locale, windows,
        scale,
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
            stage_members(&solo, Some("agnes".into()), default()),
            vec![StageMember {
                hero_class: Default::default(),
                handheld: shared::handheld::HandheldSelection::Unequipped,
                avatar: Some("agnes".into()),
                leader: true,
                revealed: true,
                character: default(),
            }]
        );
        let view = party_view(2, 1, &[1, 2]);
        let staged = stage_members(&view, None, default());
        assert_eq!(staged.len(), 2);
        assert!(!staged[0].leader && staged[1].leader);
        assert_eq!(ordered_members(&view)[0].player_id, view.you);
    }

    #[test]
    fn member_status_reads_presence_without_inventing_ready_votes() {
        let mut m = member(1, false);
        assert_eq!(member_status(&m).0, "In lobby");
        m.in_match = true;
        assert_eq!(member_status(&m).0, "In match");
        m.away = true;
        assert_eq!(member_status(&m).0, "Away");
    }
    #[test]
    fn five_members_keep_identity_and_centre_even_when_someone_else_leads() {
        let mut view = party_view(4, 2, &[1, 2, 3, 4, 5]);
        for member in &mut view.party.as_mut().unwrap().members {
            member.avatar = Some(format!("avatar-{}", member.player_id));
        }
        let members = ordered_members(&view);
        assert_eq!(
            members.iter().map(|m| m.player_id).collect::<Vec<_>>(),
            [4, 1, 2, 3, 5]
        );
        let staged = stage_members(&view, None, default());
        assert_eq!(staged.len(), MAX_PARTY_SIZE);
        assert_eq!(staged[0].avatar.as_deref(), Some("avatar-4"));
        assert!(!staged[0].leader);
        assert!(staged[2].leader);
    }

    #[test]
    fn phone_and_tablet_canvases_fit_independently_of_global_ui_scale() {
        for (mobile, viewport) in [
            (false, Vec2::new(1280.0, 720.0)),
            (true, Vec2::new(844.0, 390.0)),
            (true, Vec2::new(1180.0, 820.0)),
            (true, Vec2::new(1024.0, 768.0)),
        ] {
            for scale in [0.61, 1.0, 1.25] {
                let (phone, node, transform) = lobby_canvas(mobile, viewport, scale);
                assert_eq!(phone, mobile && viewport.y < 600.0);
                let (Val::Px(w), Val::Px(h), Val::Px(x), Val::Px(y)) =
                    (node.width, node.height, node.left, node.top)
                else {
                    panic!()
                };
                let centre = (Vec2::new(x, y) + Vec2::new(w, h) * 0.5) * scale;
                let extent = Vec2::new(w, h) * transform.scale * scale;
                assert!(centre.abs_diff_eq(viewport * 0.5, 0.01));
                assert!(extent.cmple(viewport + Vec2::splat(0.01)).all());
            }
        }
    }

    #[test]
    fn online_invite_uses_the_real_player_and_full_party_has_no_invite_button() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.add_ui_action::<LobbyAction>();
        let player = online(42);
        harness::spawn_ui(app.world_mut(), |root| {
            spawn_online_row(root, &PartyView::default(), &player)
        });
        app.update();
        harness::press(app.world_mut(), "LobbyInvite42");
        app.update();
        assert_eq!(
            harness::drain_actions::<LobbyAction>(app.world_mut()),
            vec![LobbyAction::Invite(42)]
        );
        let mut full = harness::kit_app();
        harness::spawn_ui(full.world_mut(), |root| {
            spawn_online_row(root, &party_view(1, 1, &[1, 2, 3, 4, 5]), &player)
        });
        assert!(harness::find(full.world_mut(), "LobbyInvite42").is_none());
    }

    #[test]
    fn social_panel_scrolls_with_touch_and_hovered_wheel() {
        use crate::ui::scroll::harness;
        let mut app = App::new();
        let window = harness::install(&mut app, crate::platform::UiProfile::Desktop);
        let centre = Vec2::new(600.0, 300.0);
        let panel = app
            .world_mut()
            .spawn((social_scroll(), ScrollPosition::default()))
            .id();
        harness::measure(&mut app, panel, centre);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(centre));
        harness::wheel_lines(&mut app, window, -2.0);
        assert_eq!(harness::offset(&app, panel), 96.0);
        harness::drag(&mut app, window, 7, centre, 150.0);
        assert!(harness::offset(&app, panel) > 200.0);
        let previous = harness::offset(&app, panel);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::ZERO));
        harness::wheel_lines(&mut app, window, -2.0);
        assert_eq!(
            harness::offset(&app, panel),
            previous,
            "rotating heroes must not scroll the social pane"
        );
    }
}
