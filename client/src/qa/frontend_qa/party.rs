//! Explicit synthetic roster evidence rendered by the production party/draft UI.
//! Raw touch messages exercise the production rotation and scrolling systems.
use super::*;
use crate::{
    frontend::party_stage::{PartyStage, StageCamera},
    net::GameStateSnapshot,
    party::PartyClient,
};
use bevy::input::touch::{TouchInput, TouchPhase};
use shared::{
    HeroClass,
    map::Team,
    party::{OnlinePlayer, PartyInfo, PartyInvite, PartyMember, PartyView},
    prematch::{DraftPlayer, PrematchPhase, PrematchSnapshot, Role},
    wire::CharacterChoice,
};

const FILES: [&str; 8] = [
    "01-party-solo.png",
    "02-party-three.png",
    "03-party-five.png",
    "04-party-rotated.png",
    "05-party-social-scroll.png",
    "06-party-draft.png",
    "07-party-countdown.png",
    "08-party-loading.png",
];
const AVATARS: [&str; 5] = ["agnes", "pirate-bot", "good-knight", "lady-koi", "crowley"];
const VIEWER: u64 = 2;

pub(super) struct PartyQaPlugin {
    pub directory: PathBuf,
}
impl Plugin for PartyQaPlugin {
    fn build(&self, app: &mut App) {
        let dimension = |name: &str, default| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        app.insert_resource(PartyQa {
            directory: self.directory.clone(),
            pixels: UVec2::new(
                dimension("OMOBA_QA_WIDTH", 1280),
                dimension("OMOBA_QA_HEIGHT", 720),
            ),
            started: Instant::now(),
            stage: if draft_only() { 5 } else { 0 },
            frames: 0,
            sequence: 0,
            in_flight: false,
            readbacks: Vec::new(),
            captures: Vec::new(),
            finished: false,
            drag: None,
            scroll_before: None,
        })
        .insert_resource(ScreenDriverPaused(true))
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, drive.after(bevy::ui::UiSystems::Focus))
        .add_systems(
            PostUpdate,
            capture
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}
#[derive(Resource)]
struct PartyQa {
    directory: PathBuf,
    pixels: UVec2,
    started: Instant,
    stage: usize,
    frames: u32,
    sequence: u64,
    in_flight: bool,
    readbacks: Vec<usize>,
    captures: Vec<serde_json::Value>,
    finished: bool,
    drag: Option<(Vec2, f32)>,
    scroll_before: Option<f32>,
}
#[derive(Component)]
struct PartyShot(usize);
/// The two changed states only: editable draft and its circular countdown.
fn draft_only() -> bool {
    std::env::var("OMOBA_PARTY_QA_DRAFT_ONLY").as_deref() == Ok("1")
}
fn wanted(stage: usize) -> AppScreen {
    match stage {
        0..=4 => AppScreen::Lobby,
        5 => AppScreen::Draft,
        _ => AppScreen::Loading,
    }
}
fn setup(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(2.0),
            right: Val::Px(8.0),
            ..default()
        },
        Text::new("QA · SYNTHETIC ROSTER · production renderer · scripted touch"),
        TextFont {
            font_size: 9.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        FocusPolicy::Pass,
        Pickable::IGNORE,
        ZIndex(300),
        Name::new("PartyQaFixtureLabel"),
    ));
}
fn nickname(index: usize, long: bool) -> String {
    let names = if long {
        [
            "Marina Dawnkeeper",
            "Wotori",
            "Александра-Хранительница",
            "星辰守望者与森林之心",
            "Ironwood Expedition Captain",
        ]
    } else {
        ["Marina", "Wotori", "Alexander", "Mei", "Ironwood"]
    };
    names[index % names.len()].to_owned()
}
fn party_fixture(stage: usize) -> PartyView {
    let count = match stage {
        0 => 1,
        1 => 3,
        _ => 5,
    };
    let leader = if stage == 1 { 1 } else { VIEWER };
    let mut members: Vec<_> = (0..count)
        .map(|index| PartyMember {
            player_id: index as u64 + 1,
            nickname: nickname(index, stage >= 2),
            avatar: Some(AVATARS[index].into()),
            leader: index as u64 + 1 == leader,
            in_match: false,
            away: false,
        })
        .collect();
    members.sort_by_key(|m| !m.leader);
    PartyView {
        you: VIEWER,
        party: (stage != 0).then_some(PartyInfo {
            party_id: 77,
            leader,
            members,
            launch: None,
        }),
        invites: if stage == 0 {
            vec![PartyInvite {
                party_id: 88,
                from_player_id: 12,
                from_nickname: "Marina".into(),
                expires_in_secs: 45,
            }]
        } else {
            Vec::new()
        },
        online: (10..26)
            .map(|id| OnlinePlayer {
                player_id: id,
                nickname: format!("{} {id}", nickname((id - 10) as usize, false)),
                avatar: Some(AVATARS[(id as usize) % 5].into()),
                friend: id % 2 == 0,
                in_party: id % 5 == 0,
                in_match: id % 7 == 0,
                invited: id == 14,
            })
            .collect(),
    }
}
fn prematch_fixture(stage: usize) -> PrematchSnapshot {
    let phase = match stage {
        5 => PrematchPhase::Draft,
        6 => PrematchPhase::Countdown,
        _ => PrematchPhase::Loading,
    };
    PrematchSnapshot {
        generation: 1,
        phase,
        remaining_ms: match stage {
            5 => 23_000,
            6 => 2_400,
            _ => 18_000,
        },
        needed: 10,
        last_request_id: 0,
        error: None,
        players: (0..10)
            .map(|index| DraftPlayer {
                handheld: Default::default(),
                player_id: index as u64 + 1,
                nickname: if index < 5 {
                    nickname(index, false)
                } else {
                    format!("Bot {}", index - 4)
                },
                team: if index < 5 { Team::Green } else { Team::Blue },
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::ALL[index % 5],
                avatar: Some(AVATARS[index % 5].into()),
                sprite_character: None,
                role: Role::ALL[index % 5],
                is_bot: index >= 5,
                locked: stage != 5 || index % 2 == 0,
                loaded: stage == 7 && !(3..5).contains(&index),
            })
            .collect(),
    }
}
#[allow(clippy::too_many_arguments)]
fn drive(
    mut qa: ResMut<PartyQa>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut party: ResMut<PartyClient>,
    mut game: ResMut<GameStateSnapshot>,
    mut career: ResMut<crate::career::CareerClient>,
    mut card: ResMut<crate::frontend::card::ProfileCard>,
    mut pause: ResMut<crate::pause_menu::PauseMenuState>,
    mut server_entry: Option<ResMut<crate::mobile_ui::ServerEntry>>,
    stage: Res<PartyStage>,
    nodes: Query<(
        crate::qa::QaName,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&ScrollPosition>,
    )>,
    mut touches: MessageWriter<TouchInput>,
) {
    if qa.finished {
        return;
    }
    // This labelled fixture owns only the underlying party/prematch pages.
    // The phone's one-time localhost prompt can otherwise obscure the stage
    // when this intentionally serverless capture starts.
    pause.open = false;
    pause.in_settings = false;
    if let Some(entry) = server_entry.as_mut() {
        entry.open = false;
    }
    let Ok((window_entity, mut window)) = windows.single_mut() else {
        return;
    };
    window.resolution.set_scale_factor_override(Some(1.0));
    if window.resolution.physical_width() != qa.pixels.x
        || window.resolution.physical_height() != qa.pixels.y
    {
        window
            .resolution
            .set_physical_resolution(qa.pixels.x, qa.pixels.y);
    }
    qa.sequence += 1;
    party.apply_view(
        660_929,
        qa.sequence,
        party_fixture(qa.stage),
        Instant::now(),
    );
    career.nickname = "Wotori".into();
    career.view.storage_enabled = true;
    career.view.match_service = Some(shared::match_service::MatchServiceView::Idle);
    card.bypass_change_detection().showcase_avatar = Some(AVATARS[1].into());
    game.your_id = VIEWER;
    game.meta.server_epoch = 660_929;
    game.meta.match_id = 1;
    game.prematch = (qa.stage >= 5).then(|| prematch_fixture(qa.stage));
    game.state = if qa.stage == 6 {
        crate::net::GameState::Starting {
            countdown_ms: 2_400,
        }
    } else {
        crate::net::GameState::Forming {
            ready: 10,
            needed: 10,
        }
    };
    if *screen.get() != wanted(qa.stage) {
        next.set(wanted(qa.stage));
        return;
    }
    if qa.in_flight || !window.focused {
        return;
    }
    qa.frames += 1;
    if qa.stage == 3 {
        let Some((_, _, transform, _)) = nodes
            .iter()
            .find(|(name, ..)| name.as_str() == "LobbyStageImage")
        else {
            return;
        };
        let center = transform.translation / window.scale_factor();
        let phase = match qa.frames {
            12 => Some(TouchPhase::Started),
            16 => Some(TouchPhase::Moved),
            20 => Some(TouchPhase::Ended),
            _ => None,
        };
        if qa.frames == 12 {
            qa.drag = Some((center, stage.yaws[0]));
        }
        if let Some(phase) = phase {
            let start = qa.drag.map_or(center, |v| v.0);
            touches.write(TouchInput {
                window: window_entity,
                id: 810,
                phase,
                position: start
                    + if phase == TouchPhase::Started {
                        Vec2::ZERO
                    } else {
                        Vec2::new(64.0, 0.0)
                    },
                force: None,
            });
        }
    }
    if qa.stage == 4 {
        let Some((_, node, transform, Some(scroll))) = nodes
            .iter()
            .find(|(name, ..)| name.as_str() == "LobbySocialScroll")
        else {
            return;
        };
        qa.scroll_before.get_or_insert(scroll.y);
        // Multiple real in-panel swipes reach the bottom without setting ScrollPosition.
        if (12..72).contains(&qa.frames) {
            let cycle = (qa.frames - 12) % 4;
            let size = node.size() * transform.to_scale_angle_translation().0.abs()
                / window.scale_factor();
            let center = transform.translation / window.scale_factor();
            let start = center + Vec2::new(0.0, size.y * 0.5 - 12.0);
            let end = center - Vec2::new(0.0, size.y * 0.5 - 12.0);
            let phase = match cycle {
                0 => Some(TouchPhase::Started),
                1 => Some(TouchPhase::Moved),
                2 => Some(TouchPhase::Ended),
                _ => None,
            };
            if let Some(phase) = phase {
                touches.write(TouchInput {
                    window: window_entity,
                    id: 811,
                    phase,
                    position: if phase == TouchPhase::Started {
                        start
                    } else {
                        end
                    },
                    force: None,
                });
            }
        }
    }
}
fn abort(
    qa: &mut PartyQa,
    reason: &str,
    nodes: &[serde_json::Value],
    exit: &mut MessageWriter<AppExit>,
) {
    qa.finished = true;
    let _ = std::fs::create_dir_all(&qa.directory);
    let _ = std::fs::write(qa.directory.join("qa-failure.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status":"failed", "reason":reason, "stage":qa.stage, "frames":qa.frames, "captures":qa.captures, "nodes":nodes,
    })).unwrap());
    error!("PARTY_QA failed: {reason}");
    exit.write(AppExit::error());
}
#[allow(clippy::too_many_arguments)]
fn capture(
    mut commands: Commands,
    mut qa: ResMut<PartyQa>,
    stage: Res<PartyStage>,
    screen: Res<State<AppScreen>>,
    nodes: Query<(
        crate::qa::QaName,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
        Option<&ScrollPosition>,
    )>,
    scenes: Query<(&Name, &SceneRoot)>,
    cameras: Query<&Camera, With<StageCamera>>,
    assets: Res<AssetServer>,
    modals: Res<crate::ui::ModalStack>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(180) {
        abort(
            &mut qa,
            "180-second capture deadline (window focus and model loading required)",
            &[],
            &mut exit,
        );
        return;
    }
    if qa.in_flight {
        if !qa.readbacks.contains(&qa.stage)
            || !qa
                .directory
                .join(FILES[qa.stage])
                .metadata()
                .is_ok_and(|f| f.len() > 32)
        {
            return;
        }
        qa.stage += 1;
        qa.frames = 0;
        qa.in_flight = false;
        qa.drag = None;
        qa.scroll_before = None;
        if qa.stage == FILES.len() || (draft_only() && qa.stage == 7) {
            let summary = serde_json::json!({ "status":"passed", "scenario":"party-stage", "version":env!("CARGO_PKG_VERSION"),
                "method":"real Bevy primary_window ScreenshotCaptured + save_to_disk", "synthetic_roster_fixture":true,
                "live_multiplayer_evidence":false, "raw_touch_input_injected":!draft_only(), "manual_or_physical_device_input":false,
                "viewport_simulation":true, "captures":qa.captures });
            let saved = std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            );
            qa.finished = true;
            if let Ok((entity, _)) = windows.single() {
                commands.entity(entity).despawn();
            }
            exit.write(if saved.is_ok() {
                AppExit::Success
            } else {
                AppExit::error()
            });
        }
        return;
    }
    if *screen.get() != wanted(qa.stage) || qa.frames < 180 {
        return;
    }
    if modals.is_open() {
        let reason = format!("Unexpected modal obscures the fixture: {:?}", modals.top());
        abort(&mut qa, &reason, &[], &mut exit);
        return;
    }
    let Ok((_, window)) = windows.single() else {
        return;
    };
    if window.resolution.physical_width() != qa.pixels.x
        || window.resolution.physical_height() != qa.pixels.y
    {
        return;
    }
    let expected = match qa.stage {
        0 => 1,
        1 => 3,
        _ => 5,
    };
    let model_paths: Vec<_> = scenes
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("PartyStageModel"))
        .map(|(name, scene)| {
            (
                name.as_str().to_owned(),
                assets.get_path(scene.0.id()).map(|p| p.to_string()),
                assets.is_loaded_with_dependencies(scene.0.id()),
            )
        })
        .collect();
    if stage.members.len() != expected
        || model_paths.len() != expected
        || model_paths.iter().any(|m| !m.2)
    {
        return;
    }
    let mut records = Vec::new();
    let mut scrolling = None;
    for (name, node, transform, visible, scroll) in &nodes {
        if !["Lobby", "Draft", "Loading", "Prematch"]
            .iter()
            .any(|prefix| name.as_str().starts_with(prefix))
        {
            continue;
        }
        let size = node.size() * transform.to_scale_angle_translation().0.abs();
        let min = transform.translation - size * 0.5;
        if name.as_str() == "LobbySocialScroll" {
            scrolling = scroll.map(|position| (position.y, crate::ui::scroll::max_offset(node)));
        }
        records.push(serde_json::json!({ "name":name.as_str(), "min":min.to_array(), "size":size.to_array(),
            "visible":visible.is_none_or(|v| v.get()), "fits_viewport":fits(min,size,qa.pixels.as_vec2()) }));
    }
    let mut required = match qa.stage {
        0 => vec![
            "LobbyScreen",
            "LobbyStageImage",
            "LobbyBack",
            "LobbyFriends",
            "LobbyPlayBots",
            "LobbyQuickMatch",
            "LobbyAccept88",
        ],
        1 => vec![
            "LobbyScreen",
            "LobbyStageImage",
            "LobbyBack",
            "LobbyLeave",
            "LobbyFriends",
        ],
        2..=4 => vec![
            "LobbyScreen",
            "LobbyStageImage",
            "LobbyBack",
            "LobbyLeave",
            "LobbyPlayBots",
            "LobbyQuickMatch",
            "LobbySocialScroll",
        ],
        5 => vec![
            "DraftScreen",
            "PrematchStageImage",
            "DraftSelectionClock",
            "DraftCancel",
            "DraftLock",
        ],
        6 => vec![
            "LoadingScreen",
            "PrematchStageImage",
            "LoadingCancel",
            "LoadingCountdownRing",
        ],
        _ => vec!["LoadingScreen", "PrematchStageImage", "LoadingCancel"],
    };
    let plates: Vec<_> = if qa.stage < 5 {
        (0..expected)
            .map(|i| format!("LobbyMemberPlate{i}"))
            .collect()
    } else {
        (1..=5)
            .map(|id| format!("PrematchStagePlayer-{id}"))
            .collect()
    };
    required.extend(plates.iter().map(String::as_str));
    if let Some(missing) = required.iter().find(|name| {
        !records
            .iter()
            .any(|r| r["name"] == **name && r["visible"] == true && r["fits_viewport"] == true)
    }) {
        let reason =
            format!("Essential control or identity is missing/outside viewport: {missing}");
        abort(&mut qa, &reason, &records, &mut exit);
        return;
    }
    if qa.stage == 3
        && !qa
            .drag
            .is_some_and(|(_, before)| (stage.yaws[0] - before - 64.0 * 0.012).abs() < 0.002)
    {
        abort(
            &mut qa,
            "Raw touch did not rotate the actual hero stage",
            &records,
            &mut exit,
        );
        return;
    }
    if qa.stage == 4
        && !scrolling
            .zip(qa.scroll_before)
            .is_some_and(|((after, max), before)| {
                max > 0.0 && after > before + 8.0 && after >= max - 2.0
            })
    {
        abort(
            &mut qa,
            "Raw touch swipes did not reach social list bottom",
            &records,
            &mut exit,
        );
        return;
    }
    if !cameras.iter().any(|camera| camera.is_active) {
        abort(
            &mut qa,
            "Production stage camera is inactive",
            &records,
            &mut exit,
        );
        return;
    }
    let index = qa.stage;
    let capture = serde_json::json!({ "file":FILES[index], "screen":format!("{:?}", screen.get()), "pixels":qa.pixels.to_array(),
        "synthetic_roster_fixture":true, "viewer":VIEWER, "party":party_fixture(index), "prematch":(index>=5).then(||prematch_fixture(index)),
        "stage_members":stage.members.iter().map(|m| serde_json::json!({"avatar":m.avatar,"leader":m.leader,"revealed":m.revealed,"character":format!("{:?}",m.character)})).collect::<Vec<_>>(),
        "yaw":stage.yaws[0], "yaw_before_drag":qa.drag.map(|v|v.1), "scroll_before":qa.scroll_before,
        "scroll_after":scrolling.map(|v|v.0), "scroll_max":scrolling.map(|v|v.1), "window_focused":window.focused,
        "modal_stack_open":modals.is_open(), "top_modal":format!("{:?}",modals.top()),
        "model_paths":model_paths, "required":required, "nodes":records });
    qa.captures.push(capture);
    if std::fs::create_dir_all(&qa.directory).is_err() {
        abort(&mut qa, "Cannot create output directory", &[], &mut exit);
        return;
    }
    commands
        .spawn((Screenshot::primary_window(), PartyShot(index)))
        .observe(save_to_disk(qa.directory.join(FILES[index])))
        .observe(
            |event: On<ScreenshotCaptured>, shots: Query<&PartyShot>, mut qa: ResMut<PartyQa>| {
                if let Ok(shot) = shots.get(event.entity)
                    && event.image.width() == qa.pixels.x
                    && event.image.height() == qa.pixels.y
                {
                    qa.readbacks.push(shot.0);
                }
            },
        );
    qa.in_flight = true;
}
