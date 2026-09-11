//! Explicit 720p UI readbacks. Entry/help/gameplay use real admission and
//! production button handlers. The final result snapshot is synthetic. An explicit
//! OMOBA_BETA_UI_SKILL_UPGRADES=1 additionally enables a client-only progression
//! fixture in gameplay captures. Both are labeled and reported, never match proof.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use bevy::{
    app::AppExit,
    asset::RecursiveDependencyLoadState,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    scene::{SceneInstance, SceneSpawner},
    window::PrimaryWindow,
};

use crate::{
    help_overlay::HelpOverlayVisible,
    net::{ClientSession, GameState, GameStateSnapshot},
    team::{Team, TeamSelectRoot},
    verdant3d::VerdantEnvironment,
};

const FILES: [&str; 7] = [
    "01-entry-720p.png",
    "02-help-720p.png",
    "03-gameplay-720p.png",
    "04-shop-720p.png",
    "05-purchase-720p.png",
    "06-shop-closed-720p.png",
    "07-result-fixture-720p.png",
];
const SETTLE_FRAMES: u32 = 45;
fn capture_file(qa: &BetaUiQa, index: usize) -> String {
    FILES[index].replace("720p", &format!("{}p", qa.height))
}

pub(crate) struct BetaUiQaPlugin;
impl Plugin for BetaUiQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_VISUAL_QA_DIR")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let seconds = std::env::var("OMOBA_VISUAL_QA_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(240)
            .clamp(30, 600);
        app.insert_resource(BetaUiQa {
            directory,
            started: Instant::now(),
            timeout: Duration::from_secs(seconds),
            stage: 0,
            frames: 0,
            in_flight: false,
            readbacks: Vec::new(),
            captures: Vec::new(),
            fixture_label: false,
            skill_upgrades: std::env::var("OMOBA_BETA_UI_SKILL_UPGRADES")
                .is_ok_and(|value| value == "1"),
            width: std::env::var("OMOBA_QA_WIDTH")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1280),
            height: std::env::var("OMOBA_QA_HEIGHT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(720),
        })
        .add_systems(
            PreUpdate,
            prepare_controls.after(bevy::ui::UiSystems::Focus),
        )
        .add_systems(
            Update,
            prepare_result_fixture
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::game_state::GameStateUiSet)
                .before(crate::input_context::InputContextSet::Modal),
        )
        .add_systems(PostUpdate, capture.after(bevy::ui::UiSystems::Layout));
        if app.world().resource::<BetaUiQa>().skill_upgrades {
            app.init_resource::<SkillUpgradeFixtureState>().add_systems(
                Update,
                prepare_skill_upgrade_fixture
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(crate::mobile_controls::MobileControlsSet::Input)
                    .before(crate::mobile_controls::MobileControlsSet::Visuals)
                    .before(crate::match_hud::MatchHudVisuals),
            );
        }
    }
}

#[derive(Resource)]
struct BetaUiQa {
    directory: PathBuf,
    started: Instant,
    timeout: Duration,
    stage: usize,
    frames: u32,
    in_flight: bool,
    readbacks: Vec<usize>,
    captures: Vec<serde_json::Value>,
    fixture_label: bool,
    skill_upgrades: bool,
    width: u32,
    height: u32,
}
#[derive(Component)]
struct BetaUiShot(usize);

fn prepare_controls(
    qa: Res<BetaUiQa>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mut buttons: Query<(&Name, &mut Interaction), With<Button>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    shop: Res<crate::shop::ShopState>,
    equipment: Query<&crate::net::PlayerEquipment, With<crate::player::Player>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut focus_requested: Local<bool>,
) {
    if let Ok(mut window) = windows.single_mut() {
        // The capture command owns this short-lived window. Request real OS
        // focus once; do not bypass the production focus-loss input gate.
        if !window.focused && !*focus_requested {
            window.focused = true;
            *focus_requested = true;
        }
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != qa.width || window.physical_height() != qa.height {
            window
                .resolution
                .set_physical_resolution(qa.width, qa.height);
        }
    }
    if qa.stage == 5 && shop.open {
        keys.press(KeyCode::Escape);
    } else {
        keys.release(KeyCode::Escape);
    }
    for (name, mut interaction) in &mut buttons {
        let press = (qa.stage == 1
            && session.is_connected()
            && !session.join_confirmed()
            && name.as_str()
                == if std::env::var("OMOBA_QA_TEAM").as_deref() == Ok("blue") {
                    "TeamBlueButton"
                } else {
                    "TeamGreenButton"
                })
            || (qa.stage == 2 && help.0 && name.as_str() == "HelpDismissButton")
            || (qa.stage == 3 && !shop.open && name.as_str() == "ShopOpenButton")
            || (qa.stage == 4
                && shop.open
                && !shop.purchase_pending()
                && name.as_str() == "ShopBuy-EB"
                && equipment.single().is_ok_and(|equipment| {
                    !equipment
                        .inventory
                        .contains(&shared::shop::ItemId::EmberBlade)
                }));
        if press {
            *interaction = Interaction::Pressed;
        }
    }
}

fn prepare_result_fixture(
    mut commands: Commands,
    mut qa: ResMut<BetaUiQa>,
    mut game: ResMut<GameStateSnapshot>,
) {
    if qa.stage != 6 {
        return;
    }
    // Applied after each real network snapshot, exclusively in this opt-in
    // renderer scenario. The authoritative server and actors remain unchanged.
    game.state = GameState::Victory {
        winner: Team::Green,
    };
    game.rematch_in_secs = Some(10);
    if !qa.fixture_label {
        qa.fixture_label = true;
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(16.0),
                top: Val::Px(16.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            Text::new("QA FIXTURE: result layout only"),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.8, 0.3)),
            BackgroundColor(Color::srgb(0.05, 0.05, 0.05)),
            ZIndex(200),
        ));
    }
}

#[derive(Resource, Default)]
struct SkillUpgradeFixtureState {
    saved: Option<(Entity, crate::net::PlayerProgression)>,
    applied: bool,
    label_spawned: bool,
}

#[derive(Component)]
struct SkillUpgradeFixtureLabel;

fn has_fixture_progression(progression: &crate::net::PlayerProgression) -> bool {
    progression.level == 6 && progression.skill_points == 4 && progression.ranks == [1; 4]
}

fn prepare_skill_upgrade_fixture(
    mut commands: Commands,
    qa: Res<BetaUiQa>,
    mut fixture: ResMut<SkillUpgradeFixtureState>,
    mut player: Query<(Entity, &mut crate::net::PlayerProgression), With<crate::player::Player>>,
    mut labels: Query<&mut Node, With<SkillUpgradeFixtureLabel>>,
) {
    let active = qa.skill_upgrades && matches!(qa.stage, 2 | 5);
    if let Ok((entity, mut progression)) = player.single_mut() {
        if active {
            // Refresh the saved value whenever a real snapshot replaced our local
            // fixture. Never send a rank-up command or mutate authoritative actors.
            if fixture.saved.is_none_or(|(saved, _)| saved != entity)
                || !has_fixture_progression(&progression)
            {
                fixture.saved = Some((entity, *progression));
            }
            progression.level = 6;
            progression.skill_points = 4;
            progression.ranks = [1; 4];
            fixture.applied = true;
        } else {
            // Do not let the fixture leak into the shop/result stages when a
            // network snapshot has not arrived on the exact transition frame.
            if fixture.applied
                && has_fixture_progression(&progression)
                && let Some((saved_entity, saved)) = fixture.saved
                && saved_entity == entity
            {
                *progression = saved;
            }
            fixture.saved = None;
            fixture.applied = false;
        }
    } else {
        fixture.saved = None;
        fixture.applied = false;
    }
    for mut label in &mut labels {
        label.display = if fixture.applied {
            Display::Flex
        } else {
            Display::None
        };
    }
    if fixture.applied && !fixture.label_spawned {
        fixture.label_spawned = true;
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    bottom: Val::Px(4.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                ZIndex(200),
                SkillUpgradeFixtureLabel,
                Name::new("QaSkillUpgradeFixtureLabel"),
            ))
            .with_child((
                Text::new("QA: skill-upgrade layout fixture"),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.8, 0.3)),
                BackgroundColor(Color::srgba(0.02, 0.02, 0.02, 0.92)),
            ));
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct UiScene<'w, 's> {
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
    environment: Query<'w, 's, Entity, With<VerdantEnvironment>>,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    join: Query<'w, 's, Entity, With<TeamSelectRoot>>,
    nodes: Query<
        'w,
        's,
        (
            &'static Name,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static InheritedVisibility>,
        ),
    >,
}

fn capture(
    mut commands: Commands,
    mut qa: ResMut<BetaUiQa>,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    help: Res<HelpOverlayVisible>,
    shop: Res<crate::shop::ShopState>,
    pause: Res<crate::pause_menu::PauseMenuState>,
    equipment: Query<&crate::net::PlayerEquipment, With<crate::player::Player>>,
    context: Res<crate::input_context::GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    assets: Res<AssetServer>,
    spawner: Res<SceneSpawner>,
    scene: UiScene,
    minimap: crate::minimap::MinimapQaScene,
    progression_fixture: Option<Res<SkillUpgradeFixtureState>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage >= FILES.len() {
        return;
    }
    if qa.started.elapsed() >= qa.timeout {
        error!(
            "BETA_UI_QA failed: readiness/readback timeout stage={}",
            qa.stage
        );
        exit.write(AppExit::error());
        return;
    }
    if qa.in_flight {
        if qa.readbacks.contains(&qa.stage)
            && qa
                .directory
                .join(capture_file(&qa, qa.stage))
                .metadata()
                .is_ok_and(|p| p.len() > 32)
        {
            qa.stage += 1;
            qa.frames = 0;
            qa.in_flight = false;
            if qa.stage == FILES.len() {
                let summary = serde_json::json!({"version":env!("CARGO_PKG_VERSION"), "scenario":"beta-ui", "pixels":[qa.width,qa.height],
                    "method":"Bevy Screenshot::primary_window + save_to_disk", "captures":qa.captures,
                    "elapsed_seconds":qa.started.elapsed().as_secs_f64(), "manual_interaction_verified":false,
                    "button_handler_interactions":"scripted Interaction::Pressed on production Join, Help, Shop and purchase buttons; Escape uses production keyboard modal closure",
                    "real_purchase_verified":true,
                    "full_match_proof":false, "result_snapshot":"synthetic presentation fixture only",
                    "synthetic_progression":qa.skill_upgrades,
                    "progression_fixture":"opt-in local level 6, four skill points and rank 1 abilities during stages 2/5 only; server unchanged"});
                let saved = std::fs::write(
                    qa.directory.join("qa-summary.json"),
                    serde_json::to_vec_pretty(&summary).unwrap(),
                );
                info!("BETA_UI_QA completed summary={summary}");
                // Match the real Exit button: close the native window before
                // asking the runner to leave, releasing its render surface.
                if let Ok(window) = scene.windows.single() {
                    commands.entity(window).despawn();
                }
                exit.write(if saved.is_ok() {
                    AppExit::Success
                } else {
                    AppExit::error()
                });
            }
        }
        return;
    }
    let scenes_ready = !scene.environment.is_empty()
        && !scene.scenes.is_empty()
        && scene.scenes.iter().all(|(root, instance)| {
            instance.is_some_and(|instance| spawner.instance_is_ready(**instance))
                && matches!(
                    assets.recursive_dependency_load_state(root.0.id()),
                    RecursiveDependencyLoadState::Loaded
                )
        });
    let ready = scenes_ready
        && match qa.stage {
            0 => session.is_connected() && !session.join_confirmed() && !scene.join.is_empty(),
            1 => {
                session.join_confirmed()
                    && matches!(game.state, GameState::Running)
                    && help.0
                    && scene.join.is_empty()
            }
            2 => {
                session.join_confirmed()
                    && matches!(game.state, GameState::Running)
                    && !help.0
                    && scene.join.is_empty()
            }
            3 => shop.open && !context.gameplay_allowed(),
            4 => {
                shop.open
                    && !context.gameplay_allowed()
                    && equipment.single().is_ok_and(|equipment| {
                        equipment
                            .inventory
                            .contains(&shared::shop::ItemId::EmberBlade)
                            && equipment
                                .last_purchase
                                .as_ref()
                                .is_some_and(|receipt| receipt.error.is_none())
                    })
            }
            5 => !shop.open && !pause.open && context.gameplay_allowed(),
            6 => session.join_confirmed() && matches!(game.state, GameState::Victory { .. }),
            _ => false,
        };
    qa.frames = if ready { qa.frames + 1 } else { 0 };
    if qa.frames < SETTLE_FRAMES {
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        error!("BETA_UI_QA failed: cannot create output directory");
        exit.write(AppExit::error());
        return;
    }
    let primary_nodes: Vec<_> = scene.nodes.iter().filter(|(name, _, _, _)| matches!(name.as_str(),
        "TeamGreenButton" | "TeamBlueButton" | "AvatarGrid" | "HelpDismissButton" | "HelpOverlayRoot" | "GameStateLabel" | "ConnectionStatusPanel" | "MinimapRoot" | "MatchObjectivePanel" | "MatchHudColumn" | "SkillBarRoot" | "SkillSlot-Q" | "SkillSlot-R" | "EquipmentHud" | "ShopOpenButton" | "ShopPanel" | "ShopCloseButton" | "ShopBuy-EB" | "ShopBuy-GC" | "ShopSummary" | "ShopFeedback" | "MobileJoystick" | "MobileAbility-0" | "MobileAbility-1" | "MobileAbility-2" | "MobileAbility-3" | "MobileUpgrade-0" | "MobileUpgrade-1" | "MobileUpgrade-2" | "MobileUpgrade-3" | "PhoneMenuBar" | "QaSkillUpgradeFixtureLabel")
            || name.as_str().starts_with("ShopBuy-") || name.as_str().starts_with("ShopDescription-") || name.as_str().starts_with("ShopDetails-"))
        .map(|(name, node, transform, visible)| {
            let center = transform.translation;
            let size = node.size() * transform.to_scale_angle_translation().0.abs();
            let logical_min = (center - size * 0.5) * node.inverse_scale_factor();
            let logical_size = size * node.inverse_scale_factor();
            serde_json::json!({"name":name.as_str(), "center":[center.x,center.y], "size":[size.x,size.y],
                "logical_min":[logical_min.x,logical_min.y], "logical_size":[logical_size.x,logical_size.y],
                "visible":visible.is_none_or(|visibility| visibility.get()),
                "fits_viewport": center.x-size.x/2.0 >= -1.0 && center.y-size.y/2.0 >= -1.0
                    && center.x+size.x/2.0 <= qa.width as f32 + 1.0 && center.y+size.y/2.0 <= qa.height as f32 + 1.0})
        }).collect();
    let stage = qa.stage;
    let desktop_minimap_upper_left = (matches!(stage, 2 | 5)
        && !mobile.as_ref().is_some_and(|mobile| mobile.enabled))
    .then(|| {
        primary_nodes.iter().any(|node| {
            node["name"] == "MinimapRoot"
                && node["visible"] == true
                && (0..2).all(|axis| {
                    node["logical_min"][axis].as_f64().is_some_and(|value| {
                        (value - crate::minimap::DESKTOP_MINIMAP_INSET as f64).abs() <= 1.0
                    }) && node["logical_size"][axis].as_f64().is_some_and(|value| {
                        (value - crate::minimap::MINIMAP_SIZE as f64).abs() <= 1.0
                    })
                })
        })
    });
    if desktop_minimap_upper_left == Some(false) {
        error!(
            "BETA_UI_QA failed: desktop minimap must be 252px at logical inset16: {primary_nodes:?}"
        );
        exit.write(AppExit::error());
        return;
    }
    // Measure the real laid-out description and affordability text, including
    // wrapping. A card fitting the viewport alone does not prove its text fits.
    let shop_text_fits = !matches!(stage, 3 | 4)
        || primary_nodes
            .iter()
            .filter(|node| {
                node["name"].as_str().is_some_and(|name| {
                    name.starts_with("ShopDescription-") || name.starts_with("ShopDetails-")
                })
            })
            .all(|text| {
                let name = text["name"].as_str().unwrap();
                let code = name.split_once('-').unwrap().1;
                primary_nodes
                    .iter()
                    .find(|node| node["name"] == format!("ShopBuy-{code}"))
                    .is_some_and(|card| {
                        (0..2).all(|axis| {
                            let center = text["center"][axis].as_f64().unwrap();
                            let half = text["size"][axis].as_f64().unwrap() * 0.5;
                            let card_center = card["center"][axis].as_f64().unwrap();
                            let card_half = card["size"][axis].as_f64().unwrap() * 0.5;
                            center - half >= card_center - card_half - 1.0
                                && center + half <= card_center + card_half + 1.0
                        })
                    })
            });
    if !shop_text_fits {
        error!("BETA_UI_QA failed: shop description/price text leaves its card: {primary_nodes:?}");
        exit.write(AppExit::error());
        return;
    }
    let required: &[&str] = match stage {
        0 => &["TeamGreenButton", "TeamBlueButton", "AvatarGrid"],
        1 => &["HelpDismissButton", "HelpOverlayRoot"],
        2 | 5 if mobile.as_ref().is_some_and(|mobile| mobile.enabled) => &[
            "MinimapRoot",
            "MatchObjectivePanel",
            "MatchHudColumn",
            "EquipmentHud",
            "ShopOpenButton",
            "MobileJoystick",
            "MobileAbility-0",
            "MobileAbility-1",
            "MobileAbility-2",
            "MobileAbility-3",
            "PhoneMenuBar",
        ],
        2 | 5 => &[
            "MinimapRoot",
            "MatchObjectivePanel",
            "MatchHudColumn",
            "SkillBarRoot",
            "SkillSlot-Q",
            "SkillSlot-R",
            "EquipmentHud",
            "ShopOpenButton",
        ],
        3 | 4 => &[
            "ShopPanel",
            "ShopCloseButton",
            "ShopBuy-EB",
            "ShopBuy-GC",
            "ShopSummary",
        ],
        6 => &["GameStateLabel"],
        _ => &[],
    };
    let synthetic_progression = progression_fixture
        .as_ref()
        .is_some_and(|fixture| fixture.applied);
    let upgrade_required: &[&str] =
        if synthetic_progression && mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
            &[
                "MobileUpgrade-0",
                "MobileUpgrade-1",
                "MobileUpgrade-2",
                "MobileUpgrade-3",
                "QaSkillUpgradeFixtureLabel",
            ]
        } else {
            &[]
        };
    let controls_fit = required.iter().chain(upgrade_required).all(|name| {
        primary_nodes.iter().any(|node| {
            node["name"] == *name
                && node["visible"] == true
                && node["fits_viewport"] == true
                && node["size"][0].as_f64().is_some_and(|size| size > 0.0)
                && node["size"][1].as_f64().is_some_and(|size| size > 0.0)
        })
    });
    let dock_names = [
        "MinimapRoot",
        "MatchObjectivePanel",
        "MatchHudColumn",
        "SkillBarRoot",
        "EquipmentHud",
        "MobileJoystick",
        "MobileAbility-0",
        "MobileAbility-1",
        "MobileAbility-2",
        "MobileAbility-3",
        "MobileUpgrade-0",
        "MobileUpgrade-1",
        "MobileUpgrade-2",
        "MobileUpgrade-3",
        "PhoneMenuBar",
    ];
    let dock: Vec<_> = primary_nodes
        .iter()
        .filter(|node| {
            dock_names.iter().any(|name| node["name"] == *name)
                && node["visible"] == true
                && node["size"][0].as_f64().is_some_and(|size| size > 0.0)
                && node["size"][1].as_f64().is_some_and(|size| size > 0.0)
        })
        .collect();
    let dock_clear = !matches!(stage, 2 | 5)
        || dock.iter().enumerate().all(|(i, a)| {
            dock.iter().skip(i + 1).all(|b| {
                let delta_x =
                    (a["center"][0].as_f64().unwrap() - b["center"][0].as_f64().unwrap()).abs();
                let delta_y =
                    (a["center"][1].as_f64().unwrap() - b["center"][1].as_f64().unwrap()).abs();
                // Raw mobile touch controls are circles. Their bounding-square
                // corners may overlap without overlapping visible/hit regions.
                let circle = |node: &serde_json::Value| {
                    node["name"].as_str().is_some_and(|name| {
                        name == "MobileJoystick"
                            || name.starts_with("MobileAbility-")
                            || name.starts_with("MobileUpgrade-")
                    })
                };
                if circle(a) && circle(b) {
                    return delta_x.hypot(delta_y)
                        >= (a["size"][0].as_f64().unwrap() + b["size"][0].as_f64().unwrap()) / 2.0;
                }
                delta_x >= (a["size"][0].as_f64().unwrap() + b["size"][0].as_f64().unwrap()) / 2.0
                    || delta_y
                        >= (a["size"][1].as_f64().unwrap() + b["size"][1].as_f64().unwrap()) / 2.0
            })
        });
    if !dock_clear {
        error!("BETA_UI_QA failed: HUD panels overlap: {dock:?}");
        exit.write(AppExit::error());
        return;
    }
    if !controls_fit {
        error!(
            "BETA_UI_QA failed: primary controls are missing, hidden or outside requested viewport; gameplay_allowed={} modal_open={} focused={:?}: {primary_nodes:?}",
            context.gameplay_allowed(),
            context.modal_open,
            mobile.as_ref().map(|m| m.focused)
        );
        exit.write(AppExit::error());
        return;
    }
    let record = serde_json::json!({"file":capture_file(&qa, stage), "stage":stage, "pixels":[qa.width,qa.height],
        "mobile_controls":mobile.as_ref().is_some_and(|mobile| mobile.enabled), "admitted":session.join_confirmed(), "server_epoch":game.meta.server_epoch, "snapshot_tick":game.meta.snapshot_tick,
        "synthetic_result":stage == 6, "synthetic_progression":synthetic_progression,
        "progression_fixture":synthetic_progression.then(||serde_json::json!({"level":6,"skill_points":4,"ranks":[1,1,1,1],"server_unchanged":true})),
        "minimap":minimap.diagnostics(), "desktop_minimap_upper_left":desktop_minimap_upper_left, "shop_modal":shop.open, "gameplay_allowed":context.gameplay_allowed(), "pause_open":pause.open,
        "equipment":equipment.single().ok().map(|e|serde_json::json!({"gold":e.gold,"inventory":e.inventory,"bonuses":e.item_bonuses,"receipt":e.last_purchase})), "primary_controls_fit":controls_fit, "shop_text_fits":shop_text_fits, "primary_nodes":primary_nodes});
    info!("BETA_UI_QA capture_request={record}");
    qa.captures.push(record);
    qa.in_flight = true;
    commands
        .spawn((Screenshot::primary_window(), BetaUiShot(stage)))
        .observe(save_to_disk(qa.directory.join(capture_file(&qa, stage))))
        .observe(record_readback);
}

fn record_readback(
    captured: On<ScreenshotCaptured>,
    shots: Query<&BetaUiShot>,
    mut qa: ResMut<BetaUiQa>,
) {
    if let Ok(shot) = shots.get(captured.entity) {
        if captured.image.width() != qa.width || captured.image.height() != qa.height {
            error!("BETA_UI_QA failed: readback dimensions must be 1280x720");
        }
        qa.readbacks.push(shot.0);
        info!(
            "BETA_UI_QA readback_complete stage={} dimensions={}x{}",
            shot.0,
            captured.image.width(),
            captured.image.height()
        );
    }
}
