use super::*;
use crate::ui::test_id::harness;
use shared::match_vote::{Active, Eligibility};

fn fixture() -> View {
    View {
        server_epoch: 11,
        match_id: 7,
        next_generation: Some(9),
        cooldown_secs: 0,
        remake: Eligibility {
            available: true,
            starts_in_secs: 0,
            code: "available".into(),
        },
        surrender: Eligibility {
            available: false,
            starts_in_secs: 510,
            code: "too_early".into(),
        },
        active: Some(Active {
            kind: Kind::Remake,
            generation: 8,
            initiator: 42,
            initiator_name: "Союзник".into(),
            yes: 1,
            no: 0,
            required: 3,
            eligible: 4,
            remaining_secs: 27,
            my_vote: None,
        }),
    }
}
fn app() -> App {
    let mut app = harness::kit_app();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .add_plugins(crate::i18n::I18nPlugin::fixed(
            crate::i18n::LocaleId::ENGLISH,
        ))
        .insert_state(crate::frontend::AppScreen::InMatch)
        .insert_resource(ClientSession::admitted_for_test())
        .init_resource::<CareerClient>()
        .init_resource::<super::super::PauseMenuState>()
        .add_message::<NetworkCommand>()
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Node::default()).with_children(spawn);
        });
    configure(&mut app);
    app.world_mut()
        .resource_mut::<CareerClient>()
        .view
        .match_vote = Some(fixture());
    app
}

#[test]
fn modal_replacement_removes_vote_controls_before_kit_decoration() {
    if crate::i18n::testing::isolated(
        "pause_menu::match_votes::tests::modal_replacement_removes_vote_controls_before_kit_decoration",
    ) {
        return;
    }
    let mut app = app();
    app.init_resource::<crate::ui::ModalStack>()
        .add_systems(Update, crate::ui::widgets::paint_slabs.in_set(UiSet::Paint));
    // Exercise the real decorator's deferred entity commands, which the
    // earlier prompt-only tests omitted. Repeated modal opening also changes
    // the prompt identity while a button's skin is due to update.
    for _ in 0..20 {
        app.update();
        let yes = harness::find(app.world_mut(), "MatchVotePromptYes").unwrap();
        assert!(
            app.world()
                .get::<crate::ui::kit_assets::KitImage>(yes)
                .is_some(),
            "a newly shown prompt must receive its skin in the same frame"
        );
        app.world_mut().entity_mut(yes).insert(Interaction::Hovered);
        app.world_mut()
            .resource_mut::<crate::ui::ModalStack>()
            .push(crate::ui::ModalId::Help);
        app.update();
        assert!(harness::find(app.world_mut(), "MatchVotePromptYes").is_none());
        assert!(app.world().get_entity(yes).is_err());
        app.world_mut()
            .resource_mut::<crate::ui::ModalStack>()
            .pop(crate::ui::ModalId::Help);
    }
    app.update();
    let yes = harness::find(app.world_mut(), "MatchVotePromptYes").unwrap();
    harness::press(app.world_mut(), "MatchVotePromptYes");
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .any(|c| matches!(
                c,
                NetworkCommand::MatchVote {
                    server_epoch: 11,
                    match_id: 7,
                    generation: 8,
                    action: Action::Yes,
                    ..
                }
            ))
    );
    assert_eq!(
        harness::find(app.world_mut(), "MatchVotePromptYes"),
        Some(yes)
    );
}
#[test]
fn active_vote_reaches_closed_menu_and_real_press_keeps_original_round_and_generation() {
    if crate::i18n::testing::isolated(
        "pause_menu::match_votes::tests::active_vote_reaches_closed_menu_and_real_press_keeps_original_round_and_generation",
    ) {
        return;
    }
    let mut app = app();
    app.insert_resource(UiScale(0.7));
    app.update();
    assert!(harness::find(app.world_mut(), "MatchVotePrompt").is_some());
    let yes = harness::find(app.world_mut(), "MatchVotePromptYes").unwrap();
    let Val::Px(height) = app.world().get::<Node>(yes).unwrap().min_height else {
        panic!()
    };
    assert!(
        height * 0.7 >= 43.99,
        "scaled phone controls must retain a 44-pixel touch target"
    );
    let active = app
        .world_mut()
        .resource_mut::<CareerClient>()
        .into_inner()
        .view
        .match_vote
        .as_mut()
        .unwrap()
        .active
        .as_mut()
        .unwrap();
    active.remaining_secs = 26;
    active.yes = 2;
    app.update();
    assert_eq!(
        harness::find(app.world_mut(), "MatchVotePromptYes"),
        Some(yes),
        "timer/teammate votes must not destroy the control under a held touch"
    );
    let mut captions = app.world_mut().query::<(&Caption, &Text)>();
    assert!(
        captions
            .iter(app.world())
            .any(|(kind, text)| matches!(kind, Caption::Active) && text.0.contains("26"))
    );
    harness::press(app.world_mut(), "MatchVotePromptYes");
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .any(|command| matches!(
                command,
                NetworkCommand::MatchVote {
                    server_epoch: 11,
                    match_id: 7,
                    generation: 8,
                    kind: Kind::Remake,
                    action: Action::Yes
                }
            ))
    );
    app.world_mut()
        .resource_mut::<CareerClient>()
        .view
        .match_vote
        .as_mut()
        .unwrap()
        .active
        .as_mut()
        .unwrap()
        .my_vote = Some(true);
    app.update();
    assert!(harness::find(app.world_mut(), "MatchVotePromptYes").is_none());
    app.world_mut()
        .resource_mut::<CareerClient>()
        .view
        .match_vote = None;
    app.update();
    assert!(harness::find(app.world_mut(), "MatchVotePrompt").is_none());
}
#[test]
fn opening_pause_replaces_prompt_and_unavailable_surrender_has_no_start_button() {
    if crate::i18n::testing::isolated(
        "pause_menu::match_votes::tests::opening_pause_replaces_prompt_and_unavailable_surrender_has_no_start_button",
    ) {
        return;
    }
    let mut app = app();
    app.world_mut()
        .resource_mut::<super::super::PauseMenuState>()
        .open = true;
    app.update();
    assert!(harness::find(app.world_mut(), "MatchVotePrompt").is_none());
    harness::press(app.world_mut(), "MatchVoteNo");
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .any(|command| matches!(
                command,
                NetworkCommand::MatchVote {
                    server_epoch: 11,
                    match_id: 7,
                    generation: 8,
                    kind: Kind::Remake,
                    action: Action::No
                }
            ))
    );
    app.world_mut()
        .resource_mut::<CareerClient>()
        .view
        .match_vote
        .as_mut()
        .unwrap()
        .active = None;
    app.update();
    assert!(harness::find(app.world_mut(), "MatchVoteSurrender").is_none());
    harness::press(app.world_mut(), "MatchVoteRemake");
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .any(|command| matches!(
                command,
                NetworkCommand::MatchVote {
                    server_epoch: 11,
                    match_id: 7,
                    generation: 9,
                    kind: Kind::Remake,
                    action: Action::Start
                }
            ))
    );
    app.insert_resource(ClientSession::reconnecting_for_test());
    app.update();
    assert!(harness::find(app.world_mut(), "MatchVoteRemake").is_none());
}
