//! Matchmaking screen: what the server says about the queue, and a way out.
//!
//! Text comes from the `searching` dictionary: the static labels are
//! `Localized`, the status and hero lines are rewritten every frame. A player
//! still seated in a running match gets the `rejoin` copy and a Resume button
//! instead of a handoff (LR-13). On the public lobby the screen names the
//! queue with its Rated/Unrated badge, and a long Ranked search offers to also
//! search Casual PvP (LR-08, `queue` dictionary).
// i18n-strict

use bevy::prelude::*;
use shared::career::QueueView;
use shared::match_service::MatchServiceView;

use super::AppScreen;
use super::widgets;
use crate::career::{CareerClient, server_error_text};
use crate::i18n::{Localized, data, tr, trf};
use crate::match_service::{CasualOffer, MatchServiceClient};
use crate::net::{ClientSession, GameState, GameStateSnapshot, SessionUiCommand};
use crate::team::TeamSelection;
use crate::ui::living_background::{self, LivingBands, LivingScene};
use crate::ui::theme::{self, ButtonKind};
use crate::ui::widgets::screen_button;
use crate::ui::{Activated, UiActionAppExt, UiSet};

pub struct SearchingScreenPlugin;

impl Plugin for SearchingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_ui_action::<SearchingAction>()
            .add_systems(OnEnter(AppScreen::Searching), spawn_searching)
            .add_systems(
                Update,
                (
                    searching_actions,
                    refresh_status,
                    refresh_resume,
                    refresh_queue,
                )
                    .chain()
                    .after(UiSet::Dispatch)
                    .run_if(in_state(AppScreen::Searching)),
            );
    }
}

#[derive(Component)]
struct SearchingStatus;

/// The "hero · avatar" line under the status.
#[derive(Component)]
struct SearchingHero;

/// Resume, shown only while the lobby reports the player in a running match.
#[derive(Component)]
struct SearchingResume;

/// "Ranked · Rated": the queue of this search and its rating (LR-08).
#[derive(Component)]
struct SearchingQueue;

/// The Casual offer's line on a long Ranked search (O-1).
#[derive(Component)]
struct SearchingOffer;

/// "Also search Casual PvP", shown while the offer stands.
#[derive(Component)]
struct SearchingAlsoCasual;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchingAction {
    Cancel,
    Resume,
    AlsoCasual,
}

/// One status line for the screen. The career queue is authoritative when the
/// server runs ranked matchmaking; otherwise the match formation counters are.
pub fn status_text(queue: &QueueView, game: &GameState, session: &ClientSession) -> String {
    if session.compatibility_issue.is_some() {
        return crate::net::link_status(session)
            .detail()
            .unwrap_or_default();
    }
    if session.state() != crate::net::ClientConnectionState::Connected {
        return super::home::connection_line(session).0;
    }
    if let Some(rejection) = session.join_rejection() {
        return data::join_rejection(rejection).to_owned();
    }
    if let Some(text) = crate::career::queue_text(queue) {
        return text;
    }
    match game {
        GameState::Lobby => tr("searching.status.waiting").to_owned(),
        GameState::Forming { ready, needed } => trf(
            "searching.status.forming",
            &[("ready", ready), ("needed", needed)],
        ),
        GameState::Starting { countdown_ms } => trf(
            "searching.status.starting",
            &[("seconds", &countdown_ms.div_ceil(1000).max(1))],
        ),
        GameState::Running | GameState::Victory { .. } => tr("searching.status.joining").to_owned(),
    }
}

/// "Hero · avatar" for the chosen hero, in the active language.
fn hero_line(selection: &TeamSelection) -> String {
    trf(
        "searching.hero",
        &[
            ("hero", &data::hero_name(selection.hero_class)),
            (
                "avatar",
                &selection
                    .avatar
                    .as_deref()
                    .and_then(omoba_passport::avatars::avatar_definition)
                    .map_or(tr("searching.default_avatar"), |avatar| {
                        avatar.display_name.as_str()
                    }),
            ),
        ],
    )
}

fn spawn_searching(
    mut commands: Commands,
    selection: Res<TeamSelection>,
    platform: Res<crate::ui::UiPlatform>,
) {
    let hero = hero_line(&selection);
    let form = theme::Form::of(platform.is_mobile());
    commands
        .spawn(widgets::screen_root(
            AppScreen::Searching,
            "SearchingScreen",
        ))
        .with_children(|root| {
            living_background::spawn(root, LivingScene::Stage, LivingBands::default(), form);
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
                BackgroundColor(theme::PANEL_OPAQUE),
                BorderColor::all(theme::PANEL_EDGE),
                Name::new("SearchingBody"),
            ))
            .with_children(|body| {
                body.spawn(widgets::label(
                    Localized::new("searching.title"),
                    12.0,
                    theme::GOLD,
                ));
                body.spawn((
                    Node {
                        width: Val::Px(48.0),
                        height: Val::Px(3.0),
                        margin: UiRect::bottom(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(theme::GOLD),
                ));
                body.spawn(widgets::heading(Localized::new("searching.heading"), 34.0));
                body.spawn((
                    widgets::label("", 14.0, theme::GOLD),
                    SearchingQueue,
                    Name::new("SearchingQueue"),
                ));
                body.spawn((
                    widgets::label(tr("searching.status.contacting"), 16.0, theme::IVORY),
                    SearchingStatus,
                    Name::new("SearchingStatus"),
                ));
                body.spawn((widgets::label(&hero, 14.0, theme::MUTED), SearchingHero));
                body.spawn((
                    widgets::label("", 13.0, theme::IVORY),
                    TextLayout::justify(Justify::Center),
                    Node {
                        max_width: Val::Percent(100.0),
                        display: Display::None,
                        ..default()
                    },
                    SearchingOffer,
                    Name::new("SearchingOffer"),
                ));
                let also_casual = screen_button(
                    body,
                    Localized::new("queue.offer.accept"),
                    ButtonKind::Secondary,
                    SearchingAction::AlsoCasual,
                    "SearchingAlsoCasual",
                );
                body.commands()
                    .entity(also_casual)
                    .insert(SearchingAlsoCasual);
                let resume = screen_button(
                    body,
                    Localized::new("rejoin.resume"),
                    ButtonKind::Primary,
                    SearchingAction::Resume,
                    "SearchingResume",
                );
                body.commands().entity(resume).insert(SearchingResume);
                screen_button(
                    body,
                    Localized::new("common.cancel"),
                    ButtonKind::Secondary,
                    SearchingAction::Cancel,
                    "SearchingCancel",
                );
            });
        });
}

fn searching_actions(
    mut session_ui: MessageWriter<SessionUiCommand>,
    mut activated: MessageReader<Activated<SearchingAction>>,
    mut flow: Option<ResMut<MatchServiceClient>>,
) {
    for Activated { action, .. } in activated.read() {
        if *action == SearchingAction::AlsoCasual {
            // Keeps the Ranked place; the next FindMatch carries it (O-1).
            if let Some(flow) = flow.as_deref_mut() {
                flow.take_casual_offer();
            }
            continue;
        }
        session_ui.write(match action {
            // One path for every server mode: the server drops the queue
            // entry or the seat, the client drops the join, and the shell
            // goes home. A signed career `CancelQueue` alone did nothing on a
            // practice or dev server and left the join retrying behind the
            // menus.
            SearchingAction::Cancel => SessionUiCommand::LeaveMatch,
            // Back to the running match's seat (`net::recovery::resume_actions`).
            SearchingAction::Resume => SessionUiCommand::ResumeMatch,
            SearchingAction::AlsoCasual => unreachable!("handled above"),
        });
    }
}

fn refresh_status(
    career: Res<CareerClient>,
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    flow: Option<Res<MatchServiceClient>>,
    selection: Option<Res<TeamSelection>>,
    mut status: Query<&mut Text, (With<SearchingStatus>, Without<SearchingHero>)>,
    mut hero: Query<&mut Text, (With<SearchingHero>, Without<SearchingStatus>)>,
) {
    if let Some(selection) = selection.as_deref() {
        let line = hero_line(selection);
        for mut label in &mut hero {
            if label.0 != line {
                label.0.clone_from(&line);
            }
        }
    }
    let text = if session.state() != crate::net::ClientConnectionState::Connected {
        super::home::connection_line(&session).0
    } else if let Some(error) = server_error_text(&career.view).filter(|_| {
        career.view.match_service.is_some()
            || career
                .view
                .error_code
                .as_deref()
                .is_some_and(crate::career::refuses_sign_in)
    }) {
        // A refused new profile (LR-16) can never reach the queue: say why.
        error
    } else if let Some(view) = &career.view.match_service {
        if flow
            .as_deref()
            .is_some_and(|flow| flow.closing_left_allocation(&career.view))
        {
            tr("rejoin.closing").to_owned()
        } else if let MatchServiceView::Failed { code } = view
            && let Some(text) = crate::moderation::refusal_text(code, &career.view.sanctions)
        {
            // A matchmaking or ranked ban (LR-15), with its end.
            text
        } else {
            crate::match_service::status_text(view)
        }
    } else {
        status_text(&career.view.queue, &game.state, &session)
    };
    for mut label in &mut status {
        if label.0 != text {
            label.0.clone_from(&text);
        }
    }
}

/// Shows Resume, and hides the hero line of the new pick, while the lobby
/// reports the player still seated in a running match: a resume reclaims the
/// hero already in that match.
fn refresh_resume(
    career: Res<CareerClient>,
    flow: Option<Res<MatchServiceClient>>,
    mut nodes: Query<
        (&mut Node, Has<SearchingResume>),
        Or<(With<SearchingResume>, With<SearchingHero>)>,
    >,
) {
    let in_match = flow
        .as_deref()
        .is_some_and(|flow| flow.in_match(&career.view).is_some());
    for (mut node, resume) in &mut nodes {
        let display = if in_match == resume {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

/// The queue badge and the Casual offer (LR-08): the badge names this
/// search's queue, or its match's once allocated; the offer line and button
/// follow the lobby's answer and this player's choice.
fn refresh_queue(
    career: Res<CareerClient>,
    flow: Option<Res<MatchServiceClient>>,
    party: Option<Res<crate::party::PartyClient>>,
    mut texts: Query<
        (&mut Text, &mut Node, Has<SearchingQueue>),
        Or<(With<SearchingQueue>, With<SearchingOffer>)>,
    >,
    mut buttons: Query<&mut Node, (With<SearchingAlsoCasual>, Without<Text>)>,
) {
    let badge = flow.as_deref().and_then(|flow| flow.badge(&career.view));
    let offer = flow
        .as_deref()
        .map_or(CasualOffer::Hidden, |flow| flow.casual_offer(&career.view));
    let in_party = party.as_deref().is_some_and(|party| party.in_party());
    let line = match offer {
        CasualOffer::Hidden => None,
        CasualOffer::Offered => Some(tr("queue.offer.hint")),
        CasualOffer::Pending if in_party => Some(tr("queue.offer.party_pending")),
        CasualOffer::Pending | CasualOffer::Searching => Some(tr("queue.also_casual")),
    };
    let show = |node: &mut Node, visible: bool| {
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    };
    for (mut text, mut node, is_badge) in &mut texts {
        let value = if is_badge { badge.as_deref() } else { line };
        show(&mut node, value.is_some());
        let value = value.unwrap_or_default();
        if text.0 != value {
            value.clone_into(&mut text.0);
        }
    }
    for mut node in &mut buttons {
        show(&mut node, offer == CasualOffer::Offered);
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

    /// LR-13: a player still seated in a running match sees the in-match copy
    /// in their language with Resume, which asks to resume that seat. Resume
    /// is hidden for every other answer, where the hero line is back.
    #[test]
    fn a_running_match_renders_a_localized_message_with_resume() {
        if crate::i18n::testing::isolated(
            "frontend::searching::tests::a_running_match_renders_a_localized_message_with_resume",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        use crate::ui::test_id::harness;
        use shared::match_service::{InMatch, MatchAllocation, MatchPreference, MatchServiceView};
        let mut app = harness::kit_app();
        app.add_plugins((bevy::state::app::StatesPlugin, I18nPlugin::default()))
            .insert_state(AppScreen::Searching)
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .init_resource::<TeamSelection>()
            .init_resource::<CareerClient>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<SessionUiCommand>()
            .add_plugins(SearchingScreenPlugin);
        let mut session = ClientSession::default();
        session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        app.insert_resource(session);
        // The lobby answered this search: the player's seat in a running match.
        let mut flow = MatchServiceClient::default();
        flow.active = true;
        app.insert_resource(flow);
        let allocation = MatchAllocation {
            allocation_id: "a".repeat(32),
            endpoint: "127.0.0.1:41000".into(),
            preference: MatchPreference::Casual,
            team: shared::map::Team::Green,
            human_count: 1,
            bot_count: 9,
            rated: false,
            join_deadline_ms: 0,
        };
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.view.match_service_request_id = Some(0);
            career.view.match_service = Some(MatchServiceView::Assigned {
                allocation: allocation.clone(),
                in_match: Some(InMatch {
                    ends_within_secs: Some(240),
                }),
            });
        }
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("ru").unwrap());
        app.update();
        app.update();
        let text_of = |app: &mut App, id: &str| -> String {
            let entity = harness::find(app.world_mut(), id).unwrap_or_else(|| panic!("no {id}"));
            let mut texts = vec![];
            if let Some(text) = app.world().get::<Text>(entity) {
                texts.push(text.0.clone());
            }
            let mut stack = vec![entity];
            while let Some(entity) = stack.pop() {
                if let Some(children) = app.world().get::<Children>(entity) {
                    for child in children.iter() {
                        if let Some(text) = app.world().get::<Text>(child) {
                            texts.push(text.0.clone());
                        }
                        stack.push(child);
                    }
                }
            }
            texts.join(" ")
        };
        let shown = |app: &mut App, id: &str| {
            let entity = harness::find(app.world_mut(), id).unwrap_or_else(|| panic!("no {id}"));
            app.world().get::<Node>(entity).unwrap().display != Display::None
        };
        let status = |app: &mut App| {
            app.world_mut()
                .query_filtered::<&Text, With<SearchingStatus>>()
                .single(app.world())
                .unwrap()
                .0
                .clone()
        };
        let hero_shown = |app: &mut App| {
            app.world_mut()
                .query_filtered::<&Node, With<SearchingHero>>()
                .single(app.world())
                .unwrap()
                .display
                != Display::None
        };
        assert_eq!(
            status(&mut app),
            "Вы всё ещё в матче · он закончится не позже чем через 4 мин.\nВернитесь в него или подождите здесь: поиск начнётся, когда матч закончится."
        );
        assert!(shown(&mut app, "SearchingResume"));
        assert!(text_of(&mut app, "SearchingResume").contains("Вернуться в матч"));
        assert!(
            !hero_shown(&mut app),
            "the new pick is not the hero in that match"
        );
        assert!(shown(&mut app, "SearchingCancel"));

        harness::press(app.world_mut(), "SearchingResume");
        app.update();
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<SessionUiCommand>>()
            .drain()
            .collect();
        assert!(
            matches!(sent[..], [SessionUiCommand::ResumeMatch]),
            "{sent:?}"
        );

        // Any other answer: no Resume, the hero line is back.
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .match_service = Some(MatchServiceView::Allocating);
        app.update();
        assert!(!shown(&mut app, "SearchingResume"));
        assert!(hero_shown(&mut app));
        assert_eq!(status(&mut app), "Матч найден · подготовка арены…");
    }

    /// LR-15: a matchmaking ban and a suspended account are refused on the
    /// search screen in the player's language (here Russian) with their end.
    #[test]
    fn a_sanction_refusal_renders_in_russian_with_its_end() {
        if crate::i18n::testing::isolated(
            "frontend::searching::tests::a_sanction_refusal_renders_in_russian_with_its_end",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        use crate::ui::test_id::harness;
        use shared::moderation::{Sanction, SanctionKind, code};
        let mut app = harness::kit_app();
        app.add_plugins((bevy::state::app::StatesPlugin, I18nPlugin::default()))
            .insert_state(AppScreen::Searching)
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .init_resource::<TeamSelection>()
            .init_resource::<CareerClient>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<SessionUiCommand>()
            .add_plugins(SearchingScreenPlugin);
        let mut session = ClientSession::default();
        session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        app.insert_resource(session);
        let mut flow = MatchServiceClient::default();
        flow.active = true;
        app.insert_resource(flow);
        let until = Some(1_893_456_000); // 2030-01-01 00:00 UTC
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.view.match_service = Some(MatchServiceView::Failed {
                code: code::MATCHMAKING_BAN.into(),
            });
            career.view.sanctions = vec![Sanction {
                kind: SanctionKind::MatchmakingBan,
                until,
            }];
        }
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("ru").unwrap());
        app.update();
        app.update();
        let status = |app: &mut App| {
            app.world_mut()
                .query_filtered::<&Text, With<SearchingStatus>>()
                .single(app.world())
                .unwrap()
                .0
                .clone()
        };
        assert_eq!(
            status(&mut app),
            "Поиск матчей недоступен вам до 2030-01-01 00:00 UTC."
        );
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.view = shared::career::CareerView {
                error: Some(shared::moderation::ACCOUNT_BANNED_TEXT.into()),
                error_code: Some(code::ACCOUNT_BANNED.into()),
                sanctions: vec![Sanction {
                    kind: SanctionKind::AccountBan,
                    until,
                }],
                ..Default::default()
            };
        }
        app.update();
        assert_eq!(
            status(&mut app),
            "Ваш аккаунт заблокирован до 2030-01-01 00:00 UTC."
        );
    }

    /// LR-08: a Ranked search names its queue and rating, shows how many
    /// search it, and after five minutes offers to also search Casual PvP;
    /// taking the offer keeps the search and its Ranked place.
    #[test]
    fn a_long_ranked_search_offers_casual_pvp() {
        if crate::i18n::testing::isolated(
            "frontend::searching::tests::a_long_ranked_search_offers_casual_pvp",
        ) {
            return;
        }
        use crate::i18n::I18nPlugin;
        use crate::ui::test_id::harness;
        use shared::match_service::{MatchPreference, MatchServiceView};
        let mut app = harness::kit_app();
        app.add_plugins((bevy::state::app::StatesPlugin, I18nPlugin::default()))
            .insert_state(AppScreen::Searching)
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .init_resource::<TeamSelection>()
            .init_resource::<CareerClient>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<SessionUiCommand>()
            .add_plugins(SearchingScreenPlugin);
        let mut session = ClientSession::default();
        session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        app.insert_resource(session);
        let mut flow = MatchServiceClient::default();
        flow.active = true;
        flow.preference = MatchPreference::Ranked;
        app.insert_resource(flow);
        let wait = |elapsed_secs, also_casual| MatchServiceView::Waiting {
            preference: MatchPreference::Ranked,
            humans: 3,
            needed: 10,
            elapsed_secs,
            bot_fill_after_secs: None,
            capacity_wait: false,
            searching: Some(4),
            casual_offer_after_secs: Some(300),
            also_casual,
        };
        let answer = |app: &mut App, view| {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.view.match_service_request_id = Some(0);
            career.view.match_service = Some(view);
        };
        let text = |app: &mut App, id: &str| -> Option<String> {
            let entity = harness::find(app.world_mut(), id)
                .or_else(|| {
                    app.world_mut()
                        .query::<(Entity, &Name)>()
                        .iter(app.world())
                        .find(|(_, name)| name.as_str() == id)
                        .map(|(entity, _)| entity)
                })
                .unwrap_or_else(|| panic!("no {id}"));
            (app.world().get::<Node>(entity).unwrap().display != Display::None)
                .then(|| app.world().get::<Text>(entity).map(|t| t.0.clone()))
                .map(Option::unwrap_or_default)
        };
        answer(&mut app, wait(120, false));
        app.update();
        app.update();
        assert_eq!(
            text(&mut app, "SearchingQueue").as_deref(),
            Some("Ranked · Rated")
        );
        let status = text(&mut app, "SearchingStatus").unwrap();
        assert!(
            status.contains("3/10") && status.contains("no bots"),
            "{status}"
        );
        assert!(status.ends_with("4 searching this queue"), "{status}");
        assert_eq!(text(&mut app, "SearchingOffer"), None);
        assert!(text(&mut app, "SearchingAlsoCasual").is_none());

        answer(&mut app, wait(300, false));
        app.update();
        assert!(
            text(&mut app, "SearchingOffer")
                .unwrap()
                .contains("keep your place in Ranked")
        );
        assert!(text(&mut app, "SearchingAlsoCasual").is_some());
        harness::press(app.world_mut(), "SearchingAlsoCasual");
        app.update();
        assert!(
            app.world()
                .resource::<Messages<SessionUiCommand>>()
                .is_empty(),
            "the search goes on"
        );
        assert!(text(&mut app, "SearchingAlsoCasual").is_none());
        answer(&mut app, wait(302, true));
        app.update();
        assert_eq!(
            text(&mut app, "SearchingOffer").as_deref(),
            Some("Also searching Casual PvP · your place in Ranked is kept")
        );
        assert_eq!(
            app.world().resource::<MatchServiceClient>().preference,
            MatchPreference::Ranked
        );
    }

    #[test]
    fn cancel_leaves_once_and_a_disabled_cancel_does_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.add_message::<SessionUiCommand>()
            .add_ui_action::<SearchingAction>()
            .add_systems(Update, searching_actions.after(UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |root| {
            screen_button(
                root,
                "Cancel",
                ButtonKind::Secondary,
                SearchingAction::Cancel,
                "SearchingCancel",
            );
        });
        app.update();
        let sent = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<SessionUiCommand>>()
                .drain()
                .collect::<Vec<_>>()
        };
        harness::press(app.world_mut(), "SearchingCancel");
        app.update();
        assert!(matches!(sent(&mut app)[..], [SessionUiCommand::LeaveMatch]));
        app.update();
        assert!(sent(&mut app).is_empty());
        harness::set_disabled(app.world_mut(), "SearchingCancel", true);
        harness::press(app.world_mut(), "SearchingCancel");
        app.update();
        assert!(sent(&mut app).is_empty());
    }
}
