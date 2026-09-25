//! Shared pre-match choices. The server owns accepted selections and lock state.
use bevy::{
    input::{
        mouse::MouseWheel,
        touch::{TouchInput, TouchPhase},
    },
    prelude::*,
    window::PrimaryWindow,
};
use shared::{
    HeroClass,
    prematch::{DraftPlayer, PrematchAction, PrematchPhase, PrematchRequest, Role},
};

use super::{AppScreen, widgets};
use crate::{
    net::{ClientSession, GameStateSnapshot, NetworkCommand, SessionUiCommand},
    passport::{AvatarCatalogueSource, TicketPoll},
    persistence::ClientSessionId,
    team::{AvatarThumbnails, CharacterChoice},
    ui::{
        Activated, TestId, UiAction, UiActionAppExt, UiSet,
        action::UiActionT,
        theme::{self, ButtonKind},
        widgets::ButtonStyle,
    },
};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum DraftSet {
    Sync,
    Input,
    Send,
    Draw,
}

pub struct DraftScreenPlugin;
impl Plugin for DraftScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DraftClient>()
            .init_resource::<DraftScrollMemory>()
            .add_ui_action::<DraftAction>()
            .configure_sets(
                Update,
                (
                    DraftSet::Sync,
                    DraftSet::Input,
                    DraftSet::Send,
                    DraftSet::Draw,
                )
                    .chain()
                    .after(crate::net::ClientNetPipeline::ApplySnapshot),
            )
            // Draft and loading handle their typed presses in `Input`.
            .configure_sets(Update, DraftSet::Input.after(UiSet::Dispatch))
            .add_systems(Update, sync_draft.in_set(DraftSet::Sync))
            .add_systems(
                Update,
                draft_actions
                    .in_set(DraftSet::Input)
                    .run_if(in_state(AppScreen::Draft)),
            )
            .add_systems(Update, send_requests.in_set(DraftSet::Send))
            .add_systems(
                Update,
                render_draft
                    .in_set(DraftSet::Draw)
                    .run_if(in_state(AppScreen::Draft)),
            )
            .add_systems(Update, scroll_panels.after(DraftSet::Draw));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice {
    character: CharacterChoice,
    hero_class: HeroClass,
    avatar: Option<String>,
    sprite_character: Option<String>,
    role: Role,
}
impl Choice {
    /// Proposes the kit's duty unless the player already chose a different
    /// role on purpose.
    fn pick_class(&mut self, class: HeroClass) {
        if self.role == self.hero_class.primary_role() {
            self.role = class.primary_role();
        }
        self.hero_class = class;
    }
}
impl From<&DraftPlayer> for Choice {
    fn from(player: &DraftPlayer) -> Self {
        Self {
            character: player.character,
            hero_class: player.hero_class,
            avatar: player.avatar.clone(),
            sprite_character: player.sprite_character.clone(),
            role: player.role,
        }
    }
}

#[derive(Clone)]
struct PendingRequest {
    request: PrematchRequest,
    last_sent: f64,
}

#[derive(Resource, Default)]
pub(super) struct DraftClient {
    namespace: Option<(u64, u64, u64)>,
    next_request_id: u64,
    choice: Option<Choice>,
    pending: Option<PendingRequest>,
    desired_lock: Option<bool>,
    wants_loaded: bool,
    loaded_sent: bool,
    pub(super) notice: Option<String>,
    pub(super) local_assets: String,
}
impl DraftClient {
    pub(super) fn request_loaded(&mut self) {
        if !self.loaded_sent {
            self.wants_loaded = true;
        }
    }
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
    fn begin(&mut self, action: PrematchAction, now: f64) -> Option<PrematchRequest> {
        let (server_epoch, match_id, generation) = self.namespace?;
        self.next_request_id = self.next_request_id.saturating_add(1).max(1);
        let request = PrematchRequest {
            server_epoch,
            match_id,
            generation,
            request_id: self.next_request_id,
            action,
        };
        self.pending = Some(PendingRequest {
            request: request.clone(),
            last_sent: now,
        });
        Some(request)
    }
}

fn sync_draft(game: Res<GameStateSnapshot>, mut state: ResMut<DraftClient>) {
    let Some(draft) = &game.prematch else {
        state.reset();
        return;
    };
    let namespace = (game.meta.server_epoch, game.meta.match_id, draft.generation);
    let local = draft.players.iter().find(|p| p.player_id == game.your_id);
    if state.namespace != Some(namespace) {
        state.reset();
        state.namespace = Some(namespace);
        state.next_request_id = draft.last_request_id;
        state.choice = local.map(Choice::from);
        state.notice = draft.error.clone();
    }
    let acknowledged = state
        .pending
        .as_ref()
        .is_some_and(|pending| draft.last_request_id >= pending.request.request_id);
    if acknowledged {
        state.pending = None;
        if let Some(error) = &draft.error {
            state.choice = local.map(Choice::from);
            state.desired_lock = None;
            state.notice = Some(error.clone());
        }
    }
    state.next_request_id = state.next_request_id.max(draft.last_request_id);
    if state.choice.is_none()
        || draft.phase != PrematchPhase::Draft
        || (local.is_some_and(|player| player.locked) && state.pending.is_none())
    {
        state.choice = local.map(Choice::from);
    }
    if local.is_some_and(|local| Some(local.locked) == state.desired_lock) {
        state.desired_lock = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DraftAction {
    Class(HeroClass),
    Role(Role),
    Avatar(String),
    Lock,
    Leave,
    Refresh,
    Connect,
}

fn draft_actions(
    mut activated: MessageReader<Activated<DraftAction>>,
    game: Res<GameStateSnapshot>,
    mut state: ResMut<DraftClient>,
    mut session: MessageWriter<SessionUiCommand>,
) {
    let pressed: Vec<DraftAction> = activated.read().map(|a| a.action.clone()).collect();
    // Leaving remains available when transport teardown removed the roster.
    if pressed.contains(&DraftAction::Leave) {
        state.reset();
        session.write(SessionUiCommand::LeaveMatch);
        return;
    }
    let Some(draft) = &game.prematch else {
        return;
    };
    let Some(local) = draft.players.iter().find(|p| p.player_id == game.your_id) else {
        return;
    };
    for action in &pressed {
        match action {
            DraftAction::Leave => {
                state.reset();
                session.write(SessionUiCommand::LeaveMatch);
            }
            DraftAction::Refresh => crate::passport::refresh_avatar_catalogue(),
            DraftAction::Connect => crate::passport::connect_account(),
            DraftAction::Lock if draft.phase == PrematchPhase::Draft => {
                state.notice = None;
                state.desired_lock = Some(!local.locked);
            }
            _ if draft.phase != PrematchPhase::Draft || local.locked => {
                state.notice = Some("Unlock your choice before making changes.".into());
            }
            DraftAction::Avatar(slug) => {
                if omoba_passport::avatars::avatar_definition(slug)
                    .is_none_or(|avatar| !crate::passport::can_select(avatar))
                {
                    state.notice = Some("This avatar is not available in your library.".into());
                    continue;
                }
                if let Some(choice) = &mut state.choice {
                    choice.avatar = Some(slug.clone());
                }
                state.notice = None;
            }
            DraftAction::Class(class) => {
                if let Some(choice) = &mut state.choice {
                    choice.pick_class(*class);
                }
                state.notice = None;
            }
            DraftAction::Role(role) => {
                if let Some(choice) = &mut state.choice {
                    choice.role = *role;
                }
                state.notice = None;
            }
            DraftAction::Lock => {}
        }
    }
}

fn send_requests(
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    identity: Res<ClientSessionId>,
    time: Res<Time<Real>>,
    mut state: ResMut<DraftClient>,
    mut writer: MessageWriter<NetworkCommand>,
) {
    let Some(draft) = &game.prematch else {
        return;
    };
    if !session.is_connected() || !session.join_confirmed() {
        return;
    }
    let Some(local) = draft.players.iter().find(|p| p.player_id == game.your_id) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if let Some(pending) = &mut state.pending {
        // Packet loss repeats the exact request identity, never a new purchase/select.
        if now - pending.last_sent >= 0.6 {
            pending.last_sent = now;
            writer.write(NetworkCommand::Prematch(pending.request.clone()));
        }
        return;
    }
    let action = if draft.phase == PrematchPhase::Loading && state.wants_loaded && !local.loaded {
        state.wants_loaded = false;
        state.loaded_sent = true;
        Some(PrematchAction::Loaded)
    } else if draft.phase == PrematchPhase::Draft {
        let desired = state.choice.clone();
        if !local.locked
            && desired
                .as_ref()
                .is_some_and(|choice| *choice != Choice::from(local))
        {
            let choice = desired.unwrap();
            let ticket = match if choice.avatar == local.avatar {
                TicketPoll::Free
            } else {
                crate::passport::ticket_for_slug(choice.avatar.as_deref(), &identity.0)
            } {
                TicketPoll::Free => None,
                TicketPoll::Ready(ticket) => Some(ticket),
                TicketPoll::Pending => {
                    state.notice = Some("Verifying your avatar…".into());
                    return;
                }
                TicketPoll::Denied(_) => {
                    state.choice = Some(Choice::from(local));
                    state.desired_lock = None;
                    state.notice = Some(
                        "Avatar verification failed. Your accepted choice is unchanged.".into(),
                    );
                    return;
                }
            };
            state.notice = None;
            Some(PrematchAction::Select {
                character: choice.character,
                hero_class: choice.hero_class,
                avatar: choice.avatar,
                sprite_character: choice.sprite_character,
                role: choice.role,
                passport_ticket: ticket,
            })
        } else {
            state
                .desired_lock
                .filter(|locked| *locked != local.locked)
                .map(|locked| PrematchAction::Lock { locked })
        }
    } else {
        None
    };
    if let Some(action) = action
        && let Some(request) = state.begin(action, now)
    {
        writer.write(NetworkCommand::Prematch(request));
    }
}

pub(super) fn composition_warning(players: &[DraftPlayer], local_id: u64) -> String {
    let Some(local) = players.iter().find(|p| p.player_id == local_id) else {
        return String::new();
    };
    let team: Vec<_> = players.iter().filter(|p| p.team == local.team).collect();
    let duplicate = local.avatar.as_ref().is_some_and(|avatar| {
        team.iter()
            .any(|p| p.player_id != local_id && p.avatar.as_ref() == Some(avatar))
    });
    if duplicate {
        return "A teammate chose the same avatar · coordinate before locking.".into();
    }
    let same_role = team.iter().filter(|p| p.role == local.role).count();
    if same_role > 1 {
        return format!(
            "{same_role} players intend {} · agree your lanes together.",
            local.role.label()
        );
    }
    if team.len() >= 3 && team.iter().all(|p| p.hero_class == local.hero_class) {
        return "One class across the team · consider a broader mix of kits.".into();
    }
    "Roles describe your plan; class determines your abilities.".into()
}

#[derive(Component)]
struct DraftRoot;
#[derive(Component, Clone, Copy)]
pub(super) struct DraftScroll(pub u8);
#[derive(Resource, Default)]
pub(super) struct DraftScrollMemory(pub std::collections::HashMap<u8, f32>);

/// A fixed-width 44 px draft/loading button: a selected tile, the primary
/// lock-in or a secondary command, with a gold edge on the first two.
pub(super) fn action_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: T,
    name: &str,
    width: f32,
    primary: bool,
    selected: bool,
) {
    let style = if selected {
        ButtonStyle {
            kind: ButtonKind::Tile,
            selected: true,
        }
    } else {
        ButtonStyle::new(if primary {
            ButtonKind::Primary
        } else {
            ButtonKind::Secondary
        })
    };
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(width),
                min_width: Val::Px(width),
                height: Val::Px(44.0),
                min_height: Val::Px(44.0),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(5.0), Val::Px(4.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(style.idle_color()),
            BorderColor::all(if primary || selected {
                theme::GOLD
            } else {
                theme::PANEL_EDGE
            }),
            style,
            UiAction(action),
            TestId::new(name.to_owned()),
        ))
        .with_children(|button| {
            button.spawn(widgets::label(label, 13.0, theme::IVORY));
        });
}

pub(super) fn avatar_name(slug: Option<&str>) -> String {
    crate::passport::avatar_display_name(slug)
}

pub(super) fn roster_row(
    parent: &mut ChildSpawnerCommands,
    player: &DraftPlayer,
    your_id: u64,
    thumbnails: &AvatarThumbnails,
    loading: bool,
    compact: bool,
) {
    let mine = player.player_id == your_id;
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(if compact { 60.0 } else { 72.0 }),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(if mine {
                theme::TILE_SELECTED
            } else {
                theme::PANEL_OPAQUE
            }),
            BorderColor::all(if mine { theme::GOLD } else { theme::PANEL_EDGE }),
            Name::new(format!("DraftPlayer-{}", player.player_id)),
        ))
        .with_children(|row| {
            if let Some(image) = player
                .avatar
                .as_ref()
                .and_then(|slug| thumbnails.0.get(slug))
            {
                row.spawn((
                    ImageNode::new(image.clone()),
                    Node {
                        width: Val::Px(40.0),
                        height: Val::Px(40.0),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
            }
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                min_width: Val::Px(0.0),
                overflow: Overflow::clip(),
                row_gap: Val::Px(2.0),
                ..default()
            })
            .with_children(|text| {
                let identity = format!(
                    "{}{}{}",
                    player.nickname,
                    if mine { " · YOU" } else { "" },
                    if player.is_bot { " · BOT" } else { "" }
                );
                text.spawn((
                    widgets::label(&identity, 13.0, theme::IVORY),
                    TextLayout::new_with_justify(Justify::Left).with_linebreak(LineBreak::NoWrap),
                ));
                text.spawn(widgets::label(
                    &format!(
                        "{} · {}",
                        player.hero_class.display_name(),
                        player.role.label()
                    ),
                    12.0,
                    theme::GOLD,
                ));
                text.spawn((
                    widgets::label(&avatar_name(player.avatar.as_deref()), 11.0, theme::MUTED),
                    TextLayout::new_with_justify(Justify::Left).with_linebreak(LineBreak::NoWrap),
                ));
            });
            row.spawn(widgets::label(
                if loading {
                    if player.loaded { "READY" } else { "LOADING" }
                } else if player.locked {
                    "LOCKED"
                } else {
                    "PICKING"
                },
                10.0,
                if player.loaded || (!loading && player.locked) {
                    crate::ui::theme::JADE
                } else {
                    theme::MUTED
                },
            ));
        });
}

fn render_draft(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    state: Res<DraftClient>,
    windows: Query<&Window, With<PrimaryWindow>>,
    roots: Query<Entity, With<DraftRoot>>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    assets: Res<AssetServer>,
    scroll: Res<DraftScrollMemory>,
    mut last: Local<String>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(draft) = &game.prematch else {
        return;
    };
    let catalogue = crate::passport::avatar_catalogue();
    let key = format!(
        "{}:{:?}:{:?}:{:?}:{}:{}:{}",
        serde_json::to_string(draft).unwrap_or_default(),
        state.choice,
        state.desired_lock,
        state.notice,
        state.pending.is_some(),
        catalogue.revision,
        window.width()
    );
    let key = format!("{key}:{}", window.height());
    if *last == key && !roots.is_empty() {
        return;
    }
    *last = key;
    for root in &roots {
        commands.entity(root).despawn();
    }
    for entry in &catalogue.entries {
        if let Some(path) = crate::passport::thumbnail_asset_path(&entry.avatar) {
            thumbnails
                .0
                .insert(entry.avatar.slug.clone(), assets.load(path));
        }
    }
    let compact = window.height() < 500.0;
    let side = if compact { 32.0 } else { 28.0 };
    let available = window.width() - side * 2.0;
    let roster_width = if compact {
        (available * 0.31).max(185.0)
    } else {
        330.0
    };
    let picker_width = available - roster_width - 12.0;
    let local = draft.players.iter().find(|p| p.player_id == game.your_id);
    let choice = state.choice.as_ref();
    let locked = local.is_some_and(|p| p.locked);
    let team = local.map(|p| p.team);
    let message = state
        .notice
        .clone()
        .unwrap_or_else(|| composition_warning(&draft.players, game.your_id));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect {
                    left: Val::Px(side),
                    right: Val::Px(side),
                    top: Val::Px(if compact { 12.0 } else { 24.0 }),
                    bottom: Val::Px(if compact { 20.0 } else { 24.0 }),
                },
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(theme::BACKDROP),
            ZIndex(theme::SCREEN_Z),
            DespawnOnExit(AppScreen::Draft),
            DraftRoot,
            Name::new("DraftScreen"),
        ))
        .with_children(|root| {
            root.spawn(Node {
                height: Val::Px(44.0),
                min_height: Val::Px(44.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        ..default()
                    })
                    .with_children(|title| {
                        title.spawn(widgets::heading(
                            "Assemble your team",
                            if compact { 20.0 } else { 28.0 },
                        ));
                        title.spawn(widgets::label(
                            &format!(
                                "{} / {} players · your side is assigned automatically",
                                draft.players.len(),
                                draft.needed
                            ),
                            12.0,
                            theme::MUTED,
                        ));
                    });
                action_button(
                    header,
                    "Cancel",
                    DraftAction::Leave,
                    "DraftCancel",
                    86.0,
                    false,
                    false,
                );
            });
            root.spawn(Node {
                flex_grow: 1.0,
                min_height: Val::Px(0.0),
                column_gap: Val::Px(12.0),
                ..default()
            })
            .with_children(|body| {
                body.spawn(Node {
                    width: Val::Px(roster_width),
                    min_width: Val::Px(roster_width),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|roster| {
                    roster.spawn(widgets::label("YOUR TEAM", 12.0, theme::GOLD));
                    roster
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                min_height: Val::Px(0.0),
                                flex_direction: FlexDirection::Column,
                                overflow: Overflow::scroll_y(),
                                row_gap: Val::Px(5.0),
                                ..default()
                            },
                            ScrollPosition(Vec2::new(0.0, *scroll.0.get(&0).unwrap_or(&0.0))),
                            DraftScroll(0),
                            Name::new("DraftTeamRoster"),
                        ))
                        .with_children(|rows| {
                            for player in draft.players.iter().filter(|p| Some(p.team) == team) {
                                roster_row(rows, player, game.your_id, &thumbnails, false, compact);
                            }
                        });
                });
                body.spawn(Node {
                    flex_grow: 1.0,
                    min_width: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(5.0),
                    ..default()
                })
                .with_children(|picker| {
                    picker
                        .spawn(Node {
                            column_gap: Val::Px(4.0),
                            ..default()
                        })
                        .with_children(|classes| {
                            for class in HeroClass::ALL {
                                action_button(
                                    classes,
                                    class.display_name(),
                                    DraftAction::Class(class),
                                    &format!("DraftClass-{}", class.id()),
                                    (picker_width - 16.0) / 5.0,
                                    false,
                                    choice.is_some_and(|c| c.hero_class == class),
                                );
                            }
                        });
                    picker
                        .spawn(Node {
                            column_gap: Val::Px(4.0),
                            ..default()
                        })
                        .with_children(|roles| {
                            for role in Role::ALL {
                                action_button(
                                    roles,
                                    role.label(),
                                    DraftAction::Role(role),
                                    &format!("DraftRole-{}", role.label()),
                                    (picker_width - 16.0) / 5.0,
                                    false,
                                    choice.is_some_and(|c| c.role == role),
                                );
                            }
                        });
                    picker
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                min_height: Val::Px(0.0),
                                flex_direction: FlexDirection::Column,
                                overflow: Overflow::scroll_y(),
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            ScrollPosition(Vec2::new(0.0, *scroll.0.get(&1).unwrap_or(&0.0))),
                            DraftScroll(1),
                            Name::new("DraftAvatarCatalogue"),
                        ))
                        .with_children(|avatars| {
                            for defaults in [true, false] {
                                avatars.spawn((
                                    widgets::label(
                                        if defaults {
                                            "INCLUDED HEROES"
                                        } else {
                                            "EKZA STUDIO · LIBRARY"
                                        },
                                        12.0,
                                        theme::GOLD,
                                    ),
                                    Name::new(if defaults {
                                        "DraftIncludedHeading"
                                    } else {
                                        "DraftStudioHeading"
                                    }),
                                ));
                                if !defaults {
                                    avatars.spawn((
                                        widgets::label(
                                            catalogue.status.label(),
                                            12.0,
                                            theme::MUTED,
                                        ),
                                        Name::new("DraftStudioStatus"),
                                    ));
                                    avatars
                                        .spawn(Node {
                                            column_gap: Val::Px(6.0),
                                            ..default()
                                        })
                                        .with_children(|actions| {
                                            action_button(
                                                actions,
                                                "Refresh",
                                                DraftAction::Refresh,
                                                "DraftStudioRefresh",
                                                86.0,
                                                false,
                                                false,
                                            );
                                            if !crate::passport::account_connected() {
                                                action_button(
                                                    actions,
                                                    "Connect Ekza",
                                                    DraftAction::Connect,
                                                    "DraftStudioConnect",
                                                    116.0,
                                                    false,
                                                    false,
                                                );
                                            }
                                        });
                                }
                                avatars
                                    .spawn(Node {
                                        flex_wrap: FlexWrap::Wrap,
                                        column_gap: Val::Px(6.0),
                                        row_gap: Val::Px(6.0),
                                        ..default()
                                    })
                                    .with_children(|tiles| {
                                        for entry in catalogue.entries.iter().filter(|e| {
                                            (e.source == AvatarCatalogueSource::Default) == defaults
                                        }) {
                                            let selected = choice.is_some_and(|c| {
                                                c.avatar.as_deref() == Some(&entry.avatar.slug)
                                            });
                                            let style = ButtonStyle {
                                                kind: ButtonKind::Tile,
                                                selected,
                                            };
                                            tiles
                                                .spawn((
                                                    Button,
                                                    Node {
                                                        width: Val::Px(84.0),
                                                        height: Val::Px(78.0),
                                                        min_height: Val::Px(78.0),
                                                        flex_shrink: 0.0,
                                                        flex_direction: FlexDirection::Column,
                                                        align_items: AlignItems::Center,
                                                        justify_content: JustifyContent::Center,
                                                        border: UiRect::all(Val::Px(1.0)),
                                                        border_radius: BorderRadius::all(Val::Px(
                                                            6.0,
                                                        )),
                                                        padding: UiRect::all(Val::Px(4.0)),
                                                        overflow: Overflow::clip(),
                                                        ..default()
                                                    },
                                                    BackgroundColor(style.idle_color()),
                                                    BorderColor::all(if selected {
                                                        theme::GOLD
                                                    } else {
                                                        theme::PANEL_EDGE
                                                    }),
                                                    style,
                                                    UiAction(DraftAction::Avatar(
                                                        entry.avatar.slug.clone(),
                                                    )),
                                                    TestId::new(format!(
                                                        "DraftAvatar-{}",
                                                        entry.avatar.slug
                                                    )),
                                                ))
                                                .with_children(|tile| {
                                                    if let Some(image) =
                                                        thumbnails.0.get(&entry.avatar.slug)
                                                    {
                                                        tile.spawn((
                                                            ImageNode::new(image.clone()),
                                                            Node {
                                                                width: Val::Px(44.0),
                                                                height: Val::Px(44.0),
                                                                flex_shrink: 0.0,
                                                                ..default()
                                                            },
                                                        ));
                                                    }
                                                    tile.spawn((
                                                        widgets::label(
                                                            &entry.avatar.display_name,
                                                            11.0,
                                                            theme::IVORY,
                                                        ),
                                                        TextLayout::new_with_justify(
                                                            Justify::Center,
                                                        )
                                                        .with_linebreak(LineBreak::NoWrap),
                                                    ));
                                                });
                                        }
                                    });
                            }
                        });
                });
            });
            root.spawn(Node {
                min_height: Val::Px(44.0),
                height: Val::Px(44.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: Val::Px(12.0),
                ..default()
            })
            .with_children(|footer| {
                footer.spawn((
                    widgets::label(
                        &message,
                        12.0,
                        if state.notice.is_some() {
                            theme::GOLD
                        } else {
                            theme::MUTED
                        },
                    ),
                    Node {
                        flex_grow: 1.0,
                        min_width: Val::Px(0.0),
                        max_height: Val::Px(44.0),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Name::new("DraftNotice"),
                ));
                action_button(
                    footer,
                    if state.pending.is_some() {
                        "Syncing…"
                    } else if locked {
                        "Unlock choice"
                    } else {
                        "Lock in"
                    },
                    DraftAction::Lock,
                    "DraftLock",
                    132.0,
                    !locked,
                    false,
                );
            });
        });
}

#[derive(Clone, Copy)]
struct ScrollGesture {
    finger: u64,
    pane: u8,
    screen: AppScreen,
    previous_y: f32,
    pending_delta: f32,
    ended: bool,
}

fn scroll_panels(
    screen: Res<State<AppScreen>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut wheel: MessageReader<MouseWheel>,
    mut touch: MessageReader<TouchInput>,
    mut panels: Query<(
        Entity,
        &DraftScroll,
        &ComputedNode,
        &UiGlobalTransform,
        &mut ScrollPosition,
    )>,
    mut memory: ResMut<DraftScrollMemory>,
    mut held: Local<Option<ScrollGesture>>,
) {
    if !matches!(screen.get(), AppScreen::Draft | AppScreen::Loading) {
        wheel.clear();
        touch.clear();
        *held = None;
        return;
    }
    if held.is_some_and(|gesture| gesture.screen != *screen.get()) {
        *held = None;
    }
    let pointer = windows.single().ok().and_then(Window::cursor_position);
    let mut delta = 0.0;
    for event in wheel.read() {
        delta -= event.y * 24.0;
    }
    for event in touch.read() {
        match event.phase {
            TouchPhase::Started if held.is_none() => {
                for (_, pane, node, transform, _) in &panels {
                    if node.size().min_element() <= 0.0 {
                        continue;
                    }
                    let rect = Rect::from_center_size(
                        transform.translation * node.inverse_scale_factor(),
                        node.size() * node.inverse_scale_factor(),
                    );
                    if rect.contains(event.position) {
                        *held = Some(ScrollGesture {
                            finger: event.id,
                            pane: pane.0,
                            screen: *screen.get(),
                            previous_y: event.position.y,
                            pending_delta: 0.0,
                            ended: false,
                        });
                        break;
                    }
                }
            }
            TouchPhase::Moved => {
                if let Some(gesture) = held.as_mut()
                    && gesture.finger == event.id
                {
                    gesture.pending_delta += gesture.previous_y - event.position.y;
                    gesture.previous_y = event.position.y;
                }
            }
            TouchPhase::Ended if held.is_some_and(|gesture| gesture.finger == event.id) => {
                held.as_mut().unwrap().ended = true;
            }
            TouchPhase::Canceled if held.is_some_and(|gesture| gesture.finger == event.id) => {
                *held = None;
            }
            _ => {}
        }
    }
    for (_, pane, node, transform, mut position) in &mut panels {
        // Rebuilt panels keep their stable pane ID but have no measured size
        // until layout runs. Preserve both the saved offset and pending motion.
        if node.size().min_element() <= 0.0 {
            continue;
        }
        let rect = Rect::from_center_size(
            transform.translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        );
        let max = (node.content_size().y - node.size().y).max(0.0) * node.inverse_scale_factor();
        if let Some(gesture) = held.as_mut()
            && gesture.pane == pane.0
        {
            position.0.y = (position.0.y + gesture.pending_delta).clamp(0.0, max);
            gesture.pending_delta = 0.0;
        }
        if pointer.is_some_and(|point| rect.contains(point)) {
            position.0.y = (position.0.y + delta).clamp(0.0, max);
        }
        memory.0.insert(pane.0, position.0.y.clamp(0.0, max));
    }
    if held.is_some_and(|gesture| gesture.ended && gesture.pending_delta == 0.0) {
        *held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_gesture_survives_panel_replacement_and_unmeasured_rebuild_frame() {
        let mut app = App::new();
        app.insert_resource(State::new(AppScreen::Loading))
            .init_resource::<DraftScrollMemory>()
            .add_message::<MouseWheel>()
            .add_message::<TouchInput>()
            .add_systems(Update, scroll_panels);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let measured = || ComputedNode {
            size: Vec2::new(200.0, 100.0),
            content_size: Vec2::new(200.0, 600.0),
            inverse_scale_factor: 1.0,
            ..default()
        };
        let old_panel = app
            .world_mut()
            .spawn((
                DraftScroll(2),
                measured(),
                UiGlobalTransform::default(),
                ScrollPosition::default(),
            ))
            .id();
        for (phase, y) in [(TouchPhase::Started, 20.0), (TouchPhase::Moved, -10.0)] {
            app.world_mut().write_message(TouchInput {
                id: 5,
                phase,
                position: Vec2::new(0.0, y),
                window,
                force: None,
            });
        }
        app.update();
        assert_eq!(
            app.world().get::<ScrollPosition>(old_panel).unwrap().0.y,
            30.0
        );
        app.world_mut().despawn(old_panel);
        let new_panel = app
            .world_mut()
            .spawn((
                DraftScroll(2),
                ComputedNode::default(),
                UiGlobalTransform::default(),
                ScrollPosition(Vec2::new(0.0, 30.0)),
            ))
            .id();
        app.world_mut().write_message(TouchInput {
            id: 5,
            phase: TouchPhase::Moved,
            position: Vec2::new(0.0, -30.0),
            window,
            force: None,
        });
        app.update();
        assert_eq!(app.world().resource::<DraftScrollMemory>().0[&2], 30.0);
        app.world_mut().entity_mut(new_panel).insert(measured());
        app.world_mut().write_message(TouchInput {
            id: 5,
            phase: TouchPhase::Moved,
            position: Vec2::new(0.0, -60.0),
            window,
            force: None,
        });
        app.update();
        assert_eq!(
            app.world().get::<ScrollPosition>(new_panel).unwrap().0.y,
            80.0
        );
        assert_eq!(app.world().resource::<DraftScrollMemory>().0[&2], 80.0);
        app.world_mut().write_message(TouchInput {
            id: 5,
            phase: TouchPhase::Ended,
            position: Vec2::new(0.0, -60.0),
            window,
            force: None,
        });
        app.update();
        app.world_mut().write_message(TouchInput {
            id: 6,
            phase: TouchPhase::Started,
            position: Vec2::new(0.0, 20.0),
            window,
            force: None,
        });
        app.world_mut().write_message(TouchInput {
            id: 6,
            phase: TouchPhase::Moved,
            position: Vec2::new(0.0, 10.0),
            window,
            force: None,
        });
        app.update();
        assert_eq!(
            app.world().get::<ScrollPosition>(new_panel).unwrap().0.y,
            90.0
        );
    }

    fn request_app() -> App {
        let mut app = App::new();
        let mut game = GameStateSnapshot {
            your_id: 7,
            ..default()
        };
        game.meta.server_epoch = 10;
        game.meta.match_id = 3;
        game.prematch = Some(shared::prematch::PrematchSnapshot {
            generation: 4,
            phase: PrematchPhase::Draft,
            remaining_ms: 0,
            needed: 1,
            last_request_id: 0,
            error: None,
            players: vec![DraftPlayer {
                player_id: 7,
                nickname: "Local".into(),
                team: shared::map::Team::Green,
                character: CharacterChoice::default(),
                hero_class: HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
                role: Role::Mid,
                is_bot: false,
                locked: false,
                loaded: false,
            }],
        });
        app.insert_resource(game)
            .insert_resource(ClientSession::admitted_for_test())
            .init_resource::<DraftClient>()
            .init_resource::<ClientSessionId>()
            .init_resource::<Time<Real>>()
            .add_message::<NetworkCommand>()
            .add_systems(Update, (sync_draft, send_requests).chain());
        app.update();
        app
    }

    fn drain_requests(app: &mut App) -> Vec<PrematchRequest> {
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .filter_map(|command| {
                if let NetworkCommand::Prematch(request) = command {
                    Some(request)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn picking_a_class_proposes_its_role_but_keeps_a_deliberate_one() {
        let mut choice = Choice {
            character: CharacterChoice::default(),
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            role: Role::Mid,
        };
        choice.pick_class(HeroClass::Warden);
        assert_eq!(choice.role, Role::Jungle);
        choice.role = Role::Support;
        choice.pick_class(HeroClass::Warrior);
        assert_eq!(
            (choice.hero_class, choice.role),
            (HeroClass::Warrior, Role::Support)
        );
    }

    #[test]
    fn choice_retries_keep_the_same_id_and_rejection_restores_accepted_selection() {
        let mut app = request_app();
        app.world_mut()
            .resource_mut::<DraftClient>()
            .choice
            .as_mut()
            .unwrap()
            .hero_class = HeroClass::Mage;
        app.update();
        let sent = drain_requests(&mut app);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            (sent[0].server_epoch, sent[0].match_id, sent[0].generation),
            (10, 3, 4)
        );
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(std::time::Duration::from_millis(650));
        app.update();
        let retried = drain_requests(&mut app);
        assert_eq!(retried.len(), 1);
        assert_eq!(
            serde_json::to_string(&sent[0]).unwrap(),
            serde_json::to_string(&retried[0]).unwrap()
        );
        {
            let mut game = app.world_mut().resource_mut::<GameStateSnapshot>();
            let draft = game.prematch.as_mut().unwrap();
            draft.last_request_id = sent[0].request_id;
            draft.error = Some("Selection unavailable".into());
        }
        app.update();
        let state = app.world().resource::<DraftClient>();
        assert_eq!(
            state.choice.as_ref().unwrap().hero_class,
            HeroClass::Warrior
        );
        assert_eq!(state.notice.as_deref(), Some("Selection unavailable"));
        assert!(state.pending.is_none());
        assert!(drain_requests(&mut app).is_empty());
    }

    #[test]
    fn namespace_change_cancels_inflight_choice_and_loading_ack_waits_for_loading_phase() {
        let mut app = request_app();
        app.world_mut()
            .resource_mut::<DraftClient>()
            .choice
            .as_mut()
            .unwrap()
            .role = Role::Jungle;
        app.update();
        assert_eq!(drain_requests(&mut app).len(), 1);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .prematch
            .as_mut()
            .unwrap()
            .generation += 1;
        app.update();
        assert!(app.world().resource::<DraftClient>().pending.is_none());
        assert_eq!(
            app.world()
                .resource::<DraftClient>()
                .choice
                .as_ref()
                .unwrap()
                .role,
            Role::Mid
        );
        assert!(drain_requests(&mut app).is_empty());
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .prematch
            .as_mut()
            .unwrap()
            .phase = PrematchPhase::Countdown;
        app.world_mut()
            .resource_mut::<DraftClient>()
            .request_loaded();
        app.update();
        assert!(drain_requests(&mut app).is_empty());
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .prematch
            .as_mut()
            .unwrap()
            .phase = PrematchPhase::Loading;
        app.update();
        let requests = drain_requests(&mut app);
        assert_eq!(requests.len(), 1);
        assert!(matches!(requests[0].action, PrematchAction::Loaded));
    }

    #[test]
    fn repeated_request_keeps_identity_and_new_generation_discards_old_work() {
        let mut state = DraftClient {
            namespace: Some((7, 3, 2)),
            next_request_id: 9,
            ..default()
        };
        let request = state
            .begin(PrematchAction::Lock { locked: true }, 1.0)
            .unwrap();
        assert_eq!(
            (
                request.server_epoch,
                request.match_id,
                request.generation,
                request.request_id
            ),
            (7, 3, 2, 10)
        );
        let retry = state.pending.as_ref().unwrap().request.clone();
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            serde_json::to_string(&retry).unwrap()
        );
        state.reset();
        assert!(state.pending.is_none() && state.namespace.is_none() && !state.wants_loaded);
    }
    #[test]
    fn composition_warnings_are_advisory_and_do_not_impose_class_uniqueness() {
        let player = |id| DraftPlayer {
            player_id: id,
            nickname: format!("P{id}"),
            team: shared::map::Team::Green,
            character: CharacterChoice::default(),
            hero_class: HeroClass::Warrior,
            avatar: Some("agnes".into()),
            sprite_character: None,
            role: Role::Jungle,
            is_bot: false,
            locked: false,
            loaded: false,
        };
        let mut players = vec![player(1), player(2)];
        assert!(composition_warning(&players, 1).contains("same avatar"));
        players[1].avatar = Some("another".into());
        assert!(composition_warning(&players, 1).contains("Jungle"));
        players[1].role = Role::Support;
        assert!(composition_warning(&players, 1).contains("class determines"));
    }

    #[test]
    fn draft_presses_dispatch_once_and_disabled_buttons_do_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.init_resource::<DraftClient>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<DraftAction>()
            .add_systems(Update, draft_actions.after(UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |root| {
            action_button(
                root,
                "Cancel",
                DraftAction::Leave,
                "DraftCancel",
                86.0,
                false,
                false,
            );
            action_button(
                root,
                "Lock in",
                DraftAction::Lock,
                "DraftLock",
                132.0,
                true,
                false,
            );
        });
        app.update();
        let lock = harness::find(app.world_mut(), "DraftLock").unwrap();
        assert_eq!(
            app.world().get::<ButtonStyle>(lock).unwrap().kind,
            ButtonKind::Primary
        );
        let leaves = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<SessionUiCommand>>()
                .drain()
                .filter(|command| matches!(command, SessionUiCommand::LeaveMatch))
                .count()
        };
        // Leaving works even without a roster (transport teardown).
        harness::press(app.world_mut(), "DraftCancel");
        app.update();
        assert_eq!(leaves(&mut app), 1);
        app.update();
        assert_eq!(leaves(&mut app), 0);
        harness::set_disabled(app.world_mut(), "DraftCancel", true);
        harness::press(app.world_mut(), "DraftCancel");
        app.update();
        assert_eq!(leaves(&mut app), 0);
        assert!(harness::drain_actions::<DraftAction>(app.world_mut()).is_empty());
        // Without a prematch roster the lock-in has nothing to toggle.
        harness::press(app.world_mut(), "DraftLock");
        app.update();
        assert_eq!(
            harness::drain_actions::<DraftAction>(app.world_mut()),
            [DraftAction::Lock]
        );
        assert_eq!(app.world().resource::<DraftClient>().desired_lock, None);
    }
}
