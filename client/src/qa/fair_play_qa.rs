//! One explicitly selected presentation fixture, not admission/gameplay proof.
//! OMOBA_FAIR_PLAY_QA_OUTPUT plus OMOBA_FAIR_PLAY_QA_SCENE; no live server.
use crate::{
    career::CareerClient,
    frontend::{AppScreen, ScreenDriverPaused},
    net::{ClientSession, GameStateSnapshot},
    ui::{Pressable, TestId},
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use shared::{
    HeroClass,
    map::Team,
    match_vote::{Active, Eligibility, Kind, View},
    party::{PartyInfo, PartyView},
    prematch::{DraftPlayer, PrematchPhase, PrematchSnapshot, Role},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    Taken,
    WideDraft,
    WideLobby,
    Compatibility,
    PartyRestart,
    SearchRestart,
    RemakeMenu,
    RemakePrompt,
    SurrenderPrompt,
    RemadeResult,
    SurrenderResult,
}
impl Scene {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "taken" => Self::Taken,
            "wide-draft" => Self::WideDraft,
            "wide-lobby" => Self::WideLobby,
            "compatibility" => Self::Compatibility,
            "party-restart" => Self::PartyRestart,
            "search-restart" => Self::SearchRestart,
            "remake-menu" => Self::RemakeMenu,
            "remake-prompt" => Self::RemakePrompt,
            "surrender-prompt" => Self::SurrenderPrompt,
            "remade-result" => Self::RemadeResult,
            "surrender-result" => Self::SurrenderResult,
            _ => return None,
        })
    }
    fn screen(self) -> AppScreen {
        match self {
            Self::Taken | Self::WideDraft => AppScreen::Draft,
            Self::WideLobby => AppScreen::Lobby,
            Self::Compatibility | Self::PartyRestart => AppScreen::Home,
            Self::SearchRestart => AppScreen::Searching,
            Self::RemadeResult | Self::SurrenderResult => AppScreen::PostMatch,
            _ => AppScreen::InMatch,
        }
    }
    fn expected(self) -> String {
        use crate::i18n::{data, tr, trf};
        match self {
            Self::Taken | Self::WideDraft => trf(
                "draft.class_taken",
                &[("hero", &data::hero_name(HeroClass::Warrior))],
            ),
            Self::WideLobby => tr("lobby.title").into(),
            Self::Compatibility => tr("net.compatibility.unavailable").into(),
            Self::PartyRestart => tr("party.notice.lobby_restarted").into(),
            Self::SearchRestart => tr("party.notice.search_restarted").into(),
            Self::RemakeMenu => tr("pause.vote.remake").into(),
            Self::RemadeResult => tr("postmatch.outcome.remade").into(),
            Self::SurrenderResult => trf("postmatch.summary.surrender", &[("minutes", &12)]),
            _ => tr("pause.vote.no").into(),
        }
    }
    fn controls(self) -> &'static [&'static str] {
        match self {
            Self::Taken | Self::WideDraft => &["DraftClass-warrior", "DraftLock"],
            Self::WideLobby => &["LobbyBack"],
            Self::Compatibility => &["HomeOfflinePractice"],
            Self::RemakeMenu => &["MatchVoteRemake"],
            Self::RemakePrompt | Self::SurrenderPrompt => {
                &["MatchVotePromptYes", "MatchVotePromptNo"]
            }
            _ => &[],
        }
    }
    fn restart(self) -> bool {
        matches!(self, Self::PartyRestart | Self::SearchRestart)
    }
    fn root(self) -> &'static str {
        match self {
            Self::Taken | Self::WideDraft => "DraftScreen",
            Self::WideLobby => "LobbyScreen",
            Self::Compatibility | Self::PartyRestart => "HomeScreen",
            Self::SearchRestart => "SearchingScreen",
            Self::RemakeMenu => "PauseMenuPanel",
            Self::RemakePrompt | Self::SurrenderPrompt => "MatchVotePrompt",
            Self::RemadeResult | Self::SurrenderResult => "PostMatchScreen",
        }
    }
}

pub(crate) struct FairPlayQaPlugin;
impl Plugin for FairPlayQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(output) = std::env::var_os("OMOBA_FAIR_PLAY_QA_OUTPUT").filter(|v| !v.is_empty())
        else {
            return;
        };
        let scene = Scene::parse(&std::env::var("OMOBA_FAIR_PLAY_QA_SCENE").unwrap_or_default())
            .expect("OMOBA_FAIR_PLAY_QA_SCENE must name one documented fixture");
        let dimension = |key, default| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(default)
                .clamp(200, 3840)
        };
        let target = app
            .world()
            .get_resource::<super::offscreen::Surface>()
            .map(|surface| surface.image.clone());
        app.insert_resource(Capture {
            directory: PathBuf::from(output),
            scene,
            started: Instant::now(),
            pixels: UVec2::new(
                dimension("OMOBA_QA_WIDTH", 852),
                dimension("OMOBA_QA_HEIGHT", 393),
            ),
            seeded: false,
            restart_sent: false,
            frames: 0,
            in_flight: false,
            readback: false,
            color_pixels: false,
            report: serde_json::Value::Null,
            finished: false,
            target,
        })
        .insert_resource(ScreenDriverPaused(true))
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Startup, watermark)
        .add_systems(
            Update,
            prepare
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::frontend::FrontendSet),
        )
        .add_systems(
            PostUpdate,
            capture
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}
#[derive(Resource)]
struct Capture {
    directory: PathBuf,
    scene: Scene,
    started: Instant,
    pixels: UVec2,
    seeded: bool,
    restart_sent: bool,
    frames: u32,
    in_flight: bool,
    readback: bool,
    color_pixels: bool,
    report: serde_json::Value,
    finished: bool,
    target: Option<Handle<Image>>,
}
fn watermark(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(2.),
            right: Val::Px(4.),
            ..default()
        },
        Text::new("QA FIXTURE · synthetic state · no live match"),
        TextFont {
            font_size: 9.0.into(),
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        GlobalZIndex(5000),
        Pickable::IGNORE,
    ));
}
const WIDE_NAME: &str = "WWWWWWWWWWWWWWWWWWWW#1234";

fn draft(wide: bool) -> PrematchSnapshot {
    let avatars = ["agnes", "pirate-bot", "good-knight", "lady-koi", "crowley"];
    PrematchSnapshot {
        join_remaining_ms: None,
        generation: 1,
        phase: PrematchPhase::Draft,
        remaining_ms: 23_000,
        needed: 10,
        last_request_id: 0,
        error: None,
        players: (0..10)
            .map(|i| DraftPlayer {
                handheld: Default::default(),
                player_id: i + 1,
                nickname: if wide {
                    WIDE_NAME.into()
                } else {
                    format!("Player {}", i + 1)
                },
                team: if i < 5 { Team::Green } else { Team::Blue },
                character: crate::team::CharacterChoice::Ipfs,
                hero_class: if i == 0 {
                    HeroClass::Warrior
                } else {
                    HeroClass::Mage
                },
                avatar: Some(avatars[i as usize % 5].into()),
                sprite_character: None,
                role: Role::ALL[i as usize % 5],
                is_bot: false,
                locked: i == 0,
                loaded: false,
            })
            .collect(),
    }
}

fn wide_party() -> PartyView {
    let avatars = ["agnes", "pirate-bot", "good-knight", "lady-koi", "crowley"];
    PartyView {
        you: 1,
        party: Some(PartyInfo {
            party_id: 77,
            leader: 1,
            members: (0..5)
                .map(|i| shared::party::PartyMember {
                    player_id: i + 1,
                    nickname: WIDE_NAME.into(),
                    avatar: Some(avatars[i as usize].into()),
                    leader: i == 0,
                    in_match: false,
                    away: false,
                })
                .collect(),
            launch: None,
        }),
        ..default()
    }
}
fn vote(scene: Scene) -> View {
    let eligibility = |available, seconds, code: &str| Eligibility {
        available,
        starts_in_secs: seconds,
        code: code.into(),
    };
    View {
        server_epoch: 11,
        match_id: 7,
        next_generation: Some(9),
        cooldown_secs: 0,
        remake: eligibility(scene != Scene::SurrenderPrompt, 0, "available"),
        surrender: eligibility(
            scene == Scene::SurrenderPrompt,
            if scene == Scene::SurrenderPrompt {
                0
            } else {
                510
            },
            if scene == Scene::SurrenderPrompt {
                "available"
            } else {
                "too_early"
            },
        ),
        active: (scene != Scene::RemakeMenu).then_some(Active {
            kind: if scene == Scene::SurrenderPrompt {
                Kind::Surrender
            } else {
                Kind::Remake
            },
            generation: 8,
            initiator: 42,
            initiator_name: "Союзник".into(),
            yes: 1,
            no: 0,
            required: if scene == Scene::SurrenderPrompt {
                4
            } else {
                3
            },
            eligible: if scene == Scene::SurrenderPrompt {
                5
            } else {
                4
            },
            remaining_secs: 27,
            my_vote: None,
        }),
    }
}
#[allow(clippy::too_many_arguments)]
fn prepare(
    mut commands: Commands,
    mut qa: ResMut<Capture>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut game: ResMut<GameStateSnapshot>,
    mut session: ResMut<ClientSession>,
    mut career: ResMut<CareerClient>,
    mut menu: ResMut<crate::pause_menu::PauseMenuState>,
    mut entry: Option<ResMut<crate::mobile_ui::ServerEntry>>,
    mut flow: ResMut<crate::match_service::MatchServiceClient>,
    mut party: ResMut<crate::party::PartyClient>,
    mut players: Query<&mut crate::team::Team, With<crate::player::Player>>,
    mut buttons: super::TestIdPresses,
) {
    if qa.finished {
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.));
        window
            .resolution
            .set_physical_resolution(qa.pixels.x, qa.pixels.y);
    }
    session.hold_presentation_fixture(&mut commands);
    if qa.scene == Scene::Compatibility {
        session.compatibility_issue = Some(shared::compatibility::CompatibilityIssue::Unavailable);
    }
    menu.open = qa.scene == Scene::RemakeMenu;
    menu.in_settings = false;
    if let Some(entry) = entry.as_mut() {
        entry.open = false;
    }
    if !qa.seeded {
        // Re-enter even an initial Home/InMatch state so its OnEnter view and
        // result latch run, rather than accepting an empty backdrop.
        if *screen.get() == qa.scene.screen() {
            next.set(if qa.scene.screen() == AppScreen::Home {
                AppScreen::Card
            } else {
                AppScreen::Home
            });
            return;
        }
        if matches!(qa.scene, Scene::RemadeResult | Scene::SurrenderResult) {
            if players.is_empty() {
                commands.spawn((
                    crate::player::Player,
                    crate::team::Team::Green,
                    crate::net::NetworkAvatar(Some("agnes".into())),
                    Name::new("FairPlayQaPlayer"),
                ));
                return;
            }
            let Ok(mut team) = players.single_mut() else {
                return;
            };
            super::result_qa::fair_play_fixture(
                qa.scene == Scene::RemadeResult,
                &mut game,
                &mut career,
                &mut team,
            );
        } else {
            game.meta = shared::protocol::SnapshotMeta::new(11, 7, 1);
            game.your_id = 2;
            if matches!(qa.scene, Scene::Taken | Scene::WideDraft) {
                game.prematch = Some(draft(qa.scene == Scene::WideDraft));
            }
            if qa.scene == Scene::WideLobby {
                game.state = crate::net::GameState::Lobby;
                party.apply_view(11, 1, wide_party(), Instant::now());
            }
            if matches!(
                qa.scene,
                Scene::RemakeMenu | Scene::RemakePrompt | Scene::SurrenderPrompt
            ) {
                game.state = crate::net::GameState::Running;
                career.view.match_vote = Some(vote(qa.scene));
            }
        }
        flow.active = qa.scene == Scene::SearchRestart;
        next.set(qa.scene.screen());
        qa.seeded = true;
    }
    if *screen.get() != qa.scene.screen() {
        return;
    }
    qa.frames += 1;
    if matches!(qa.scene, Scene::Taken | Scene::WideDraft) {
        buttons.press("DraftTabClasses");
    }
    // Start the real epoch-change reaction only after the underlying screen
    // has laid out; otherwise a slow renderer can miss a correctly timed toast.
    if qa.scene.restart() && qa.frames >= 8 && !qa.restart_sent {
        let now = Instant::now();
        party.apply_view(
            11,
            1,
            PartyView {
                you: 2,
                party: (qa.scene == Scene::PartyRestart).then_some(PartyInfo {
                    party_id: 77,
                    leader: 2,
                    members: Vec::new(),
                    launch: None,
                }),
                ..default()
            },
            now,
        );
        party.apply_view(
            12,
            1,
            PartyView {
                you: 2,
                ..default()
            },
            now,
        );
        qa.restart_sent = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Capture>,
    screen: Res<State<AppScreen>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    nodes: Query<(
        &TestId,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
        Option<&Pressable>,
    )>,
    texts: Query<(&Text, &ComputedNode, &InheritedVisibility)>,
    toasts: Query<&Node, With<crate::ui::widgets::surfaces::ToastAge>>,
    roots: Query<(super::QaName, &ComputedNode, &InheritedVisibility)>,
    modals: Option<Res<crate::ui::ModalStack>>,
    help: Res<crate::help_overlay::HelpOverlayVisible>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(120) {
        let diagnostic = serde_json::json!({
            "status":"failed", "reason":"state/visible-text/readback deadline",
            "scene":format!("{:?}",qa.scene), "screen":format!("{:?}",screen.get()),
            "frames":qa.frames, "expected":qa.scene.expected(), "in_flight":qa.in_flight,
            "visible_labels":texts.iter().filter(|(_,n,v)| n.size().min_element()>0. && v.get()).take(128).map(|(t,..)| t.0.as_str()).collect::<Vec<_>>(),
            "present_controls":nodes.iter().map(|(id,..)| id.as_str()).take(128).collect::<Vec<_>>(),
        });
        let _ = std::fs::create_dir_all(&qa.directory);
        let _ = std::fs::write(
            qa.directory.join("qa-failure.json"),
            serde_json::to_vec_pretty(&diagnostic).unwrap(),
        );
        error!("FAIR_PLAY_QA failed: state/visible-text/readback deadline");
        qa.finished = true;
        exit.write(AppExit::error());
        return;
    }
    if qa.in_flight {
        if !qa.readback || !qa.directory.join("fair-play.png").is_file() {
            return;
        }
        qa.report["color_pixels"] = qa.color_pixels.into();
        qa.report["status"] = if qa.color_pixels { "passed" } else { "failed" }.into();
        let saved = std::fs::write(
            qa.directory.join("qa-summary.json"),
            serde_json::to_vec_pretty(&qa.report).unwrap(),
        )
        .is_ok();
        qa.finished = true;
        if let Ok((window, _)) = windows.single() {
            commands.entity(window).despawn();
        }
        exit.write(if qa.color_pixels && saved {
            AppExit::Success
        } else {
            AppExit::error()
        });
        return;
    }
    let minimum_frames = if matches!(
        qa.scene,
        Scene::WideDraft | Scene::WideLobby | Scene::Compatibility
    ) {
        // Creator Loop publishes label geometry every 250 ms. Leave enough
        // settled frames for its final state to describe this exact screen.
        30
    } else if qa.scene.restart() {
        8
    } else {
        12
    };
    if !qa.seeded || *screen.get() != qa.scene.screen() || qa.frames < minimum_frames {
        return;
    }
    let allowed_modal = (qa.scene == Scene::RemakeMenu).then_some(crate::ui::ModalId::Pause);
    if help.0
        || modals.as_ref().and_then(|m| m.top()) != allowed_modal
        || !roots.iter().any(|(name, node, visible)| {
            name.as_str() == qa.scene.root() && node.size().min_element() > 0. && visible.get()
        })
    {
        return;
    }
    let Ok((_, window)) = windows.single() else {
        return;
    };
    let expected = qa.scene.expected();
    let visible = texts.iter().any(|(t, n, v)| {
        t.0.to_lowercase() == expected.to_lowercase() && n.size().min_element() > 0. && v.get()
    });
    if !visible || (qa.scene.restart() && !toasts.iter().any(|n| n.top == Val::Px(0.))) {
        return;
    }
    let mut controls = Vec::new();
    for wanted in qa.scene.controls() {
        let Some((_, node, transform, visibility, pressable)) =
            nodes.iter().find(|(id, ..)| id.as_str() == *wanted)
        else {
            return;
        };
        let size =
            node.size() * transform.to_scale_angle_translation().0.abs() / window.scale_factor();
        let center = transform.translation / window.scale_factor();
        let min = center - size * 0.5;
        let max = center + size * 0.5;
        let fit = visibility.get()
            && size.min_element() > 0.
            && min.cmpge(Vec2::splat(-1.)).all()
            && max.cmple(qa.pixels.as_vec2() + Vec2::ONE).all();
        let disabled = pressable.is_some_and(|p| p.disabled);
        if !fit
            || (matches!(qa.scene, Scene::Taken | Scene::WideDraft)
                && *wanted == "DraftClass-warrior"
                && !disabled)
        {
            return;
        }
        if matches!(
            qa.scene,
            Scene::RemakeMenu | Scene::RemakePrompt | Scene::SurrenderPrompt
        ) && (size.min_element() < 43.9 || disabled)
        {
            return;
        }
        controls.push(serde_json::json!({"id":wanted,"size":size.to_array(),"min":min.to_array(),"fits":fit,"disabled":disabled}));
    }
    qa.report = serde_json::json!({"scene":format!("{:?}",qa.scene),"screen":format!("{:?}",screen.get()),"viewport":qa.pixels.to_array(),"locale":crate::i18n::active().code(),"ui_profile":format!("{:?}",crate::platform::ui_profile()),"debug_assertions":cfg!(debug_assertions),"expected_visible_text":expected,"controls":controls,"synthetic_state":true,"real_admission_or_gameplay":false,"physical_device":false,"method":"production renderer, real UI-kit class-tab press and party epoch-change reaction; other server views are fixtures"});
    qa.report["render_surface"] = if qa.target.is_some() {
        "offscreen image with virtual window metrics"
    } else {
        "OS window"
    }
    .into();
    if std::fs::create_dir_all(&qa.directory).is_err() {
        qa.finished = true;
        exit.write(AppExit::error());
        return;
    }
    commands
        .spawn(super::offscreen::screenshot(qa.target.as_ref()))
        .observe(save_to_disk(qa.directory.join("fair-play.png")))
        .observe(readback);
    qa.in_flight = true;
}
fn readback(event: On<ScreenshotCaptured>, mut qa: ResMut<Capture>) {
    qa.readback = true;
    qa.color_pixels = event.image.data.as_ref().is_some_and(|data| {
        data.chunks_exact(4)
            .any(|p| p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 12)
    });
}
