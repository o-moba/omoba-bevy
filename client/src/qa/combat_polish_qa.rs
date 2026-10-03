//! Focused, explicitly synthetic presentation fixtures; never multiplayer proof.
use crate::{
    combat::CombatStats,
    net::{GameState, GameStateSnapshot, PlayerProgression},
    player::Player,
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{path::PathBuf, time::Instant};

pub(crate) struct CombatPolishQaPlugin;
impl Plugin for CombatPolishQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_COMBAT_POLISH_QA_DIR").map(PathBuf::from)
        else {
            return;
        };
        app.insert_resource(Qa {
            directory,
            started: Instant::now(),
            frames: 0,
            stage: 0,
            pending: false,
            iphone: std::env::var("OMOBA_IPHONE_UX_QA").is_ok_and(|v| v == "1"),
            fixture_target: None,
            reports: Vec::new(),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(
            Update,
            fixture_window.before(crate::mobile_controls::MobileControlsSet::Layout),
        )
        .add_systems(
            Update,
            prepare
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::mobile_controls::MobileControlsSet::Input),
        )
        .add_systems(PostUpdate, capture.after(bevy::ui::UiSystems::PostLayout));
    }
}
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    started: Instant,
    frames: u32,
    stage: usize,
    pending: bool,
    iphone: bool,
    fixture_target: Option<Entity>,
    reports: Vec<serde_json::Value>,
}

// Presentation fixtures already inject actors and touch events. Under macOS
// background automation, explicitly simulate an active window as another
// fixture input. This is not native focus or physical-device evidence.
fn fixture_window(qa: Res<Qa>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if qa.iphone && std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref() == Ok("1") {
        for mut window in &mut windows {
            window.focused = true;
        }
    }
}
const FILES: [&str; 3] = [
    "01-vitals-upgrades-kill-feed.png",
    "02-concealed.png",
    "03-rocket.png",
];
const IPHONE_FILES: [&str; 7] = [
    "01-iphone-gameplay-layout.png",
    "02-iphone-empty-attack.png",
    "03-iphone-concealed.png",
    "04-iphone-dash-aim.png",
    "05-iphone-recall-channel.png",
    "06-iphone-fps-settings.png",
    "07-iphone-hud-settings.png",
];
const IPHONE_FIXTURE_ACTORS: [u64; 5] = [90001, 90002, 90003, 90004, 999];

fn fixture_actor_position(index: usize) -> Vec3 {
    // These model-less actors exercise top portraits and the selected-target
    // frame only. Keep their ordinary world-space overhead plates off camera;
    // stacking fake actors beside the hero made the fixture look like a HUD bug.
    Vec3::new(10_000.0 + index as f32 * 100.0, 0.0, -10.0)
}

impl Qa {
    fn files(&self) -> &'static [&'static str] {
        if self.iphone { &IPHONE_FILES } else { &FILES }
    }
    fn viewport(&self) -> UVec2 {
        if self.iphone {
            UVec2::new(852, 393)
        } else {
            UVec2::new(1180, 820)
        }
    }
    fn settings(&self) -> bool {
        self.iphone && self.stage >= 5
    }
}

fn prepare(
    mut qa: ResMut<Qa>,
    mut commands: Commands,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut game: ResMut<GameStateSnapshot>,
    mut hero: Query<
        (
            &mut Transform,
            &mut CombatStats,
            &mut PlayerProgression,
            &crate::team::Team,
        ),
        With<Player>,
    >,
    mut presses: crate::qa::TestIdPresses,
    mut touch: MessageWriter<bevy::input::touch::TouchInput>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    mut exit: MessageWriter<AppExit>,
    mut state: (
        ResMut<crate::combat::TargetState>,
        ResMut<crate::pause_menu::PauseMenuState>,
        ResMut<crate::pause_menu::SettingsTab>,
    ),
    mut utilities: Query<&mut crate::net::PlayerUtility, With<Player>>,
    _main_thread: bevy::ecs::system::NonSendMarker,
) {
    if qa.started.elapsed().as_secs() > 240 {
        let _ = std::fs::write(qa.directory.join("timeout.json"), serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"frames":qa.frames,"pending":qa.pending,"game_state":format!("{:?}",game.state),"heroes":hero.iter().count(),"focused":mobile.focused})).unwrap());
        exit.write(AppExit::error());
        return;
    }
    let Ok((window_id, mut window)) = windows.single_mut() else {
        return;
    };
    window.resolution.set_scale_factor_override(Some(1.0));
    let viewport = qa.viewport();
    window.resolution.set(viewport.x as f32, viewport.y as f32);
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(window) = windows.get_window(window_id)
            && !window.has_focus()
        {
            window.focus_window();
        }
    });
    presses.press("HelpDismissButton");
    let Ok((mut pose, mut stats, mut progression, team)) = hero.single_mut() else {
        return;
    };
    if !matches!(game.state, GameState::Running) {
        return;
    }
    qa.frames += 1;
    if qa.iphone && qa.fixture_target.is_none() {
        for (index, id) in IPHONE_FIXTURE_ACTORS.into_iter().enumerate() {
            let actor = commands
                .spawn((
                    Transform::from_translation(fixture_actor_position(index)),
                    CombatStats {
                        hp: 70.0,
                        max_hp: 100.0,
                        mana: 65.0,
                        max_mana: 100.0,
                        ..default()
                    },
                    crate::net::NetworkPlayerId(id),
                    crate::net::NetworkHeroClass(shared::HeroClass::Wildspark),
                    crate::net::NetworkAvatar(Some("agnes".into())),
                    if id == 999 {
                        if *team == crate::team::Team::Green {
                            crate::team::Team::Blue
                        } else {
                            crate::team::Team::Green
                        }
                    } else {
                        *team
                    },
                    Name::new(format!("SyntheticIphoneUxActor{id}")),
                ))
                .id();
            if id == 999 {
                qa.fixture_target = Some(actor);
            }
        }
        commands.spawn((
            Text::new("SYNTHETIC UX FIXTURE · 852×393"),
            crate::ui::theme::text(9.0),
            TextColor(Color::srgb(1.0, 0.85, 0.35)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(6.0),
                top: Val::Px(1.0),
                ..default()
            },
            GlobalZIndex(200),
            Pickable::IGNORE,
            Name::new("IphoneUxFixtureLabel"),
        ));
    }
    if qa.iphone {
        state.1.open = qa.settings();
        state.1.in_settings = qa.settings();
        if qa.settings() {
            *state.2 = if qa.stage == 5 {
                crate::pause_menu::SettingsTab::Graphics
            } else {
                crate::pause_menu::SettingsTab::Hud
            };
        }
        state.0.selected_entity = if qa.stage == 0 {
            qa.fixture_target
        } else {
            None
        };
        state.0.selected_target = (qa.stage == 0).then_some(crate::net::TargetId {
            kind: crate::net::TargetKind::Player,
            id: 999,
        });
        if let Ok(mut utility) = utilities.single_mut() {
            utility.state.dash_remaining_secs = 0.0;
            utility.state.recall_remaining_secs = if qa.stage == 4 { 3.5 } else { 0.0 };
        }
        let gesture = |id, phase, position| bevy::input::touch::TouchInput {
            window: window_id,
            phase,
            position,
            force: None,
            id,
        };
        use bevy::input::touch::TouchPhase;
        if qa.frames == 1 {
            touch.write(gesture(
                998,
                TouchPhase::Canceled,
                mobile.layout().attack_center,
            ));
            touch.write(gesture(
                997,
                TouchPhase::Canceled,
                mobile.layout().utility_centers[0],
            ));
        }
        if qa.stage == 1 {
            if qa.frames == 3 {
                touch.write(gesture(
                    998,
                    TouchPhase::Started,
                    mobile.layout().attack_center,
                ));
            }
            if qa.frames == 4 {
                touch.write(gesture(
                    998,
                    TouchPhase::Ended,
                    mobile.layout().attack_center,
                ));
            }
        }
        if qa.stage == 3 {
            if qa.frames == 3 {
                touch.write(gesture(
                    997,
                    TouchPhase::Started,
                    mobile.layout().utility_centers[0],
                ));
            }
            if qa.frames == 4 {
                touch.write(gesture(
                    997,
                    TouchPhase::Moved,
                    mobile.layout().utility_centers[0] + Vec2::new(55.0, -60.0),
                ));
            }
        }
    }
    pose.translation.x = -22.0;
    pose.translation.z = -10.0;
    stats.hp = 65.0;
    stats.max_hp = 100.0;
    stats.mana = 40.0;
    stats.max_mana = 100.0;
    progression.level = 10;
    // The fixture is a level-ten presentation. A live dev host may still
    // report its level-one sandbox unlock mask; do not mix those two states.
    progression.sandbox_unlocked = None;
    progression.skill_points = 4;
    progression.ranks = [1; 4];
    game.vision = Some(shared::vision::TeamVision {
        sources: vec![shared::vision::VisionSource {
            position: [-22.0, -10.0],
            radius: 32.0,
        }],
        local_brush: Some(1),
        local_hidden: if qa.iphone {
            qa.stage == 2
        } else {
            qa.stage == 1
        },
    });
    let player = |id, name: &str, team| shared::live_score::LiveScorePlayer {
        player_id: id,
        nickname: name.into(),
        avatar: Some("agnes".into()),
        team,
        hero_class: shared::HeroClass::Wildspark,
        kills: 0,
        deaths: 0,
        assists: 0,
        earned_gold: 0,
        level: 10,
        connected: true,
    };
    let mut players = vec![
        player(game.your_id, "Wildspark", (*team).into()),
        player(
            999,
            "Target dummy",
            if *team == crate::team::Team::Green {
                shared::map::Team::Blue
            } else {
                shared::map::Team::Green
            },
        ),
    ];
    if qa.iphone {
        for (id, name) in [
            (90001, "Fixture ally 1"),
            (90002, "Fixture ally 2"),
            (90003, "Fixture ally 3"),
            (90004, "Fixture ally 4"),
        ] {
            players.push(player(id, name, (*team).into()));
        }
    }
    game.scoreboard = Some(shared::live_score::LiveScoreboard {
        players,
        kills: if qa.frames >= 150 || (qa.iphone && qa.stage > 0) {
            vec![shared::live_score::KillNotice {
                event_id: (qa.frames / 90) as u64
                    + if qa.iphone { qa.stage as u64 * 100 } else { 0 },
                killer_id: game.your_id,
                victim_id: 999,
            }]
        } else {
            vec![]
        },
    });
    if !qa.iphone && qa.stage == 0 && qa.frames == 150 {
        touch.write(bevy::input::touch::TouchInput {
            window: window_id,
            phase: bevy::input::touch::TouchPhase::Started,
            position: mobile.layout().attack_center,
            force: None,
            id: 998,
        });
    }
    if !qa.iphone && qa.stage == 1 && qa.frames == 1 {
        touch.write(bevy::input::touch::TouchInput {
            window: window_id,
            phase: bevy::input::touch::TouchPhase::Canceled,
            position: mobile.layout().attack_center,
            force: None,
            id: 998,
        });
    }
    game.skill_effects = if !qa.iphone && qa.stage == 2 {
        vec![shared::loadout::SkillEffectState {
            id: 9999,
            owner_id: game.your_id,
            owner_team: shared::map::Team::Green,
            skill: shared::loadout::SkillId::WildRocket,
            kind: shared::loadout::EffectVisualKind::Rocket,
            position: [-16.0, -10.0],
            end: [-15.0, -10.0],
            radius: 2.5,
            remaining_secs: 2.0,
            armed: true,
            consumed_segments: 0,
        }]
    } else {
        vec![]
    };
}
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    roots: Query<(&SceneRoot, Option<&bevy::scene::SceneInstance>)>,
    assets: Res<AssetServer>,
    context: Res<crate::input_context::GameplayInputContext>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    nodes: Query<(
        Option<&Name>,
        Option<&crate::ui::TestId>,
        &Node,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
        Option<&Text>,
        Option<&bevy::ui::CalculatedClip>,
    )>,
) {
    if (!qa.settings() && !context.gameplay_allowed()) || !mobile.focused {
        if qa.frames.is_multiple_of(120) {
            info!(
                "COMBAT_POLISH_QA waiting for gameplay: context={context:?}, focused={}",
                mobile.focused
            );
        }
        return;
    }
    if qa.stage >= qa.files().len()
        || qa.pending
        || qa.frames < if qa.stage == 0 { 180 } else { 45 }
    {
        return;
    }
    if roots.iter().any(|(scene, instance)| {
        instance.is_none()
            || !matches!(
                assets.recursive_dependency_load_state(scene.0.id()),
                bevy::asset::RecursiveDependencyLoadState::Loaded
            )
    }) {
        if qa.frames.is_multiple_of(120) {
            let scenes: Vec<_> = roots.iter().map(|(scene, instance)| serde_json::json!({"asset":format!("{:?}",assets.get_path(scene.0.id())),"instance":instance.is_some(),"state":format!("{:?}",assets.recursive_dependency_load_state(scene.0.id()))})).collect();
            let _ = std::fs::write(
                qa.directory.join("scene-readiness.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"stage":qa.stage,"frames":qa.frames,"scenes":scenes}),
                )
                .unwrap(),
            );
        }
        return;
    }
    if qa.iphone {
        let measured: Vec<_> = nodes.iter().filter_map(|(name, id, node, computed, pose, inherited, text, clip)| {
            let name = crate::ui::test_id::node_key(name, id)?;
            let watched = name.starts_with("Mobile") || name.starts_with("AllyCamera-") || name.starts_with("PauseMenuHud")
                || matches!(name, "AlliedVitals" | "TargetHealthRoot" | "KillFeed" | "HeroOverheadPlate" | "HiddenOverheadIcon" | "MeasuredFps" | "MatchMenuButton" | "SocialOpenChat" | "SocialOpenWheel" | "PauseMenuRenderFpsControls" | "IphoneUxFixtureLabel" | "TargetAimLabel" | "LockedTargetLabel" | "ActionFeedback" | "BrushStatus" | "CareerEntryActions" | "CareerProfileButton" | "CareerHistoryButton" | "CareerFriendsButton" | "CareerQueueLeaveButton");
            watched.then(|| {
                let rect = Rect::from_center_size(pose.translation, computed.size() * pose.to_scale_angle_translation().0.abs());
                let viewport = Rect::from_corners(Vec2::ZERO, qa.viewport().as_vec2());
                let shown = rect.intersect(viewport).intersect(clip.map_or(viewport, |c| c.clip));
                serde_json::json!({"name":name, "visible":node.display != Display::None && shown.width() > 1.0 && shown.height() > 1.0 && computed.size().min_element() > 0.0 && inherited.is_none_or(|v| v.get()),
                    "rect":[rect.min.x,rect.min.y,rect.max.x,rect.max.y], "text":text.map(|t|t.0.as_str())})
            })
        }).collect();
        let visible = |name: &str| {
            measured
                .iter()
                .any(|n| n["name"] == name && n["visible"] == true)
        };
        let hidden = |name: &str| !visible(name);
        let mut errors = Vec::new();
        let visible_world_overhead_plates = measured
            .iter()
            .filter(|node| node["name"] == "HeroOverheadPlate" && node["visible"] == true)
            .count();
        if visible_world_overhead_plates > 1 {
            errors.push(format!(
                "{visible_world_overhead_plates} world overhead plates visible; only the real local hero may be on camera"
            ));
        }
        if !visible("IphoneUxFixtureLabel") {
            errors.push("fixture label not visible".to_owned());
        }
        if !visible("MeasuredFps") {
            errors.push("measured FPS readout not visible".to_owned());
        }
        if !qa.settings() {
            let layout = mobile.layout();
            let scale = mobile.combat_scale();
            let attack = Vec2::new(
                mobile.viewport.x - mobile.safe.right - 124.0 * scale,
                mobile.viewport.y - mobile.safe.bottom - 76.0 * scale,
            );
            for (slot, angle) in [162.0_f32, 204.0, 246.0, 288.0].into_iter().enumerate() {
                let name = format!("MobileAbility-{slot}");
                let expected = attack + Vec2::from_angle(angle.to_radians()) * 104.0 * scale;
                let node = measured.iter().find(|n| n["name"] == name);
                let matches = node.is_some_and(|n| {
                    let rect = &n["rect"];
                    let center = Vec2::new(
                        ((rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5) as f32,
                        ((rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5) as f32,
                    );
                    n["visible"] == true
                        && center.distance(expected) < 1.5
                        && layout.ability_centers[slot].distance(expected) < 0.01
                });
                if !matches {
                    errors.push(format!("skill {slot} did not preserve its default center"));
                }
            }
            for name in [
                "MobileAimHint",
                "TargetAimLabel",
                "LockedTargetLabel",
                "SocialOpenWheel",
            ] {
                if !hidden(name) {
                    errors.push(format!("unexpected {name}"));
                }
            }
        }
        if qa.settings() {
            for name in [
                "LockedTargetLabel",
                "BrushStatus",
                "CareerEntryActions",
                "CareerProfileButton",
                "CareerHistoryButton",
                "CareerFriendsButton",
                "CareerQueueLeaveButton",
            ] {
                if !hidden(name) {
                    errors.push(format!("unexpected {name} over settings"));
                }
            }
        }
        let expected: &[&str] = match qa.stage {
            0 => &[
                "AlliedVitals",
                "TargetHealthRoot",
                "KillFeed",
                "AllyCamera-90001",
                "AllyCamera-90002",
                "AllyCamera-90003",
                "AllyCamera-90004",
            ],
            1 => &["MobileAttack"],
            2 => &["HiddenOverheadIcon"],
            3 => &["MobileDashVector", "MobileDashThumb"],
            4 => &["MobileRecall"],
            5 => &["PauseMenuRenderFpsControls"],
            _ => &["PauseMenuHudJoystickX", "PauseMenuHudJoystickY"],
        };
        for name in expected {
            if !visible(name) {
                errors.push(format!("{name} not visible"));
            }
        }
        if qa.stage == 1 && !hidden("ActionFeedback") {
            errors.push("empty attack showed an action feedback box".to_owned());
        }
        if qa.stage == 3 && mobile.dash_aim().is_none() {
            errors.push("raw dash gesture did not produce aim".to_owned());
        }
        if !errors.is_empty() {
            if qa.frames.is_multiple_of(60) {
                warn!("IPHONE_UX_QA stage {} not ready: {:?}", qa.stage, errors);
                let _ = std::fs::write(
                    qa.directory.join("readiness.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"stage":qa.stage,"errors":errors,"nodes":measured}),
                    )
                    .unwrap(),
                );
            }
            return;
        }
        let file = qa.files()[qa.stage];
        qa.reports.push(serde_json::json!({
            "file":file,
            "readiness":"PASS",
            "visible_world_overhead_plates":visible_world_overhead_plates,
            "maximum_visible_world_overhead_plates":1,
            "nodes":measured
        }));
    }
    let path = qa.directory.join(qa.files()[qa.stage]);
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(readback);
}
fn readback(shot: On<ScreenshotCaptured>, mut qa: ResMut<Qa>, mut exit: MessageWriter<AppExit>) {
    let viewport = qa.viewport();
    if shot.image.width() != viewport.x || shot.image.height() != viewport.y {
        let _ = std::fs::write(qa.directory.join("size-error.json"), serde_json::to_vec_pretty(&serde_json::json!({"actual":[shot.image.width(),shot.image.height()],"expected":[viewport.x,viewport.y]})).unwrap());
        exit.write(AppExit::error());
        return;
    }
    qa.pending = false;
    qa.stage += 1;
    qa.frames = 0;
    if qa.stage == qa.files().len() {
        let fixture_actors = qa.iphone.then(|| {
            IPHONE_FIXTURE_ACTORS
                .into_iter()
                .enumerate()
                .map(|(index, id)| {
                    serde_json::json!({
                        "player_id":id,
                        "position":fixture_actor_position(index).to_array(),
                        "world_model":false,
                        "placement":"off_camera",
                        "purpose":"top_portraits_and_target_frame"
                    })
                })
                .collect::<Vec<_>>()
        });
        let report = serde_json::json!({"presentation_fixture":true,"synthetic_window_focus":std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref()==Ok("1"),"authoritative_multiplayer_verified":false,"physical_device_verified":false,"viewport":[viewport.x,viewport.y],"language":"en","fixture_actors":fixture_actors,"files":qa.files(),"stages":qa.reports});
        let _ = std::fs::write(
            qa.directory.join("capture.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        );
        exit.write(AppExit::Success);
    }
}
