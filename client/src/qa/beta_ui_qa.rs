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
    team::TeamSelectRoot,
    verdant3d::VerdantEnvironment,
};

#[path = "edge_hud_qa.rs"]
mod edge;

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
    let file = if index < FILES.len() {
        FILES[index]
    } else {
        edge::FILES[index - FILES.len()]
    };
    file.replace("720p", &format!("{}p", qa.height))
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
            next_readiness_report: Duration::ZERO,
            hero_class: std::env::var("OMOBA_BETA_UI_CLASS").ok().map(|name| {
                shared::HeroClass::from_id(&name)
                    .expect("OMOBA_BETA_UI_CLASS must be warrior, mage, ranger or cleric")
            }),
            edge: std::env::var("OMOBA_BETA_UI_EDGE").is_ok_and(|value| value == "1"),
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
        .insert_resource(bevy::winit::WinitSettings::continuous())
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
        if app.world().resource::<BetaUiQa>().edge {
            edge::configure(app);
        }
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
    next_readiness_report: Duration,
    hero_class: Option<shared::HeroClass>,
    edge: bool,
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
impl BetaUiQa {
    fn total(&self, phone: bool) -> usize {
        FILES.len()
            + if self.edge {
                if phone {
                    edge::FILES.len()
                } else {
                    edge::FILES.len() - 1
                }
            } else {
                0
            }
    }
}

#[derive(Component)]
struct BetaUiShot(usize);

fn prepare_controls(
    qa: Res<BetaUiQa>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    mut buttons: crate::qa::TestIdPresses,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    shop: Res<crate::shop::ShopState>,
    equipment: Query<&crate::net::PlayerEquipment, With<crate::player::Player>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut focus_requested: Local<bool>,
    screen: Res<State<crate::frontend::AppScreen>>,
    mut next: ResMut<NextState<crate::frontend::AppScreen>>,
    selection: Res<crate::team::TeamSelection>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut network: MessageWriter<crate::net::NetworkCommand>,
) {
    // World harnesses bypass the session-driven frontend. The picker is now
    // created only on HeroSelect, so this legacy HUD sequence owns those two
    // presentation states while admission still uses the real lock-in button.
    use crate::frontend::AppScreen;
    // This legacy HUD fixture bypasses the shell; normal Find match always
    // negotiates the coordinated draft, covered by frontend_flow_qa.
    if qa.stage == 1 && session.is_connected() && !session.has_committed_join() {
        network.write(crate::net::NetworkCommand::Join {
            team: if std::env::var("OMOBA_QA_TEAM").as_deref() == Ok("blue") {
                crate::team::Team::Blue
            } else {
                crate::team::Team::Green
            },
            character: selection.character,
            hero_class: selection.hero_class,
            avatar: selection.avatar.clone(),
            sprite_character: Some(selection.sprite_character.clone()),
        });
    }
    let wanted = if qa.stage == 0 {
        Some(AppScreen::HeroSelect)
    } else if session.join_confirmed() {
        Some(AppScreen::InMatch)
    } else {
        None
    };
    if let Some(wanted) = wanted.filter(|wanted| *wanted != *screen.get()) {
        next.set(wanted);
    }
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
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    if qa.stage == 5 && shop.open && !phone {
        keys.press(KeyCode::Escape);
    } else {
        keys.release(KeyCode::Escape);
    }
    buttons.press_where(|name| {
        let class_press = qa.stage == 0
            && qa.hero_class.is_some_and(|class| {
                selection.hero_class != class && name == format!("ClassButton-{}", class.id())
            });
        class_press
            || (qa.stage == 2 && help.0 && name == "HelpDismissButton")
            || (qa.stage == 3 && !shop.open && name == "GoldShopButton")
            || (qa.stage == 5 && shop.open && phone && name == "ShopCloseButton")
            || (qa.stage == 4
                && !qa.edge
                && shop.open
                && !shop.purchase_pending()
                && name == "ShopBuy-EB"
                && equipment.single().is_ok_and(|equipment| {
                    !equipment
                        .inventory
                        .contains(&shared::shop::ItemId::EmberBlade)
                }))
    });
}

#[derive(Component)]
struct ResultFixtureLabel;

fn prepare_result_fixture(
    mut commands: Commands,
    mut qa: ResMut<BetaUiQa>,
    mut game: ResMut<GameStateSnapshot>,
    mut labels: Query<&mut Node, With<ResultFixtureLabel>>,
) {
    for mut node in &mut labels {
        node.display = if qa.stage == 6 {
            Display::Flex
        } else {
            Display::None
        };
    }
    if qa.stage != 6 {
        return;
    }
    // Applied after each real network snapshot, exclusively in this opt-in
    // renderer scenario. The authoritative server and actors remain unchanged.
    game.state = GameState::Victory {
        winner: shared::map::Team::Green,
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
            ResultFixtureLabel,
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
    screen: Res<'w, State<crate::frontend::AppScreen>>,
    selection: Res<'w, crate::team::TeamSelection>,
    hero_classes: Query<'w, 's, &'static crate::net::NetworkHeroClass, With<crate::player::Player>>,
    windows: Query<'w, 's, Entity, With<PrimaryWindow>>,
    edge: Option<Res<'w, edge::EdgeQa>>,
    scoreboard: Option<Res<'w, crate::edge_hud::ScoreboardState>>,
    texts: Query<'w, 's, (&'static Name, &'static Text)>,
    utility: Query<'w, 's, &'static crate::net::PlayerUtility, With<crate::player::Player>>,
    environment: Query<'w, 's, Entity, With<VerdantEnvironment>>,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    join: Query<'w, 's, Entity, With<TeamSelectRoot>>,
    nodes: Query<
        'w,
        's,
        (
            crate::qa::QaName,
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
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    if qa.stage >= qa.total(phone) {
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
            if qa.stage == qa.total(phone) {
                let summary = serde_json::json!({"version":env!("CARGO_PKG_VERSION"), "scenario":"beta-ui", "pixels":[qa.width,qa.height],
                    "method":"Bevy Screenshot::primary_window + save_to_disk", "captures":qa.captures,
                    "elapsed_seconds":qa.started.elapsed().as_secs_f64(), "manual_interaction_verified":false,
                    "button_handler_interactions":"scripted production Join/Help/Gold/Shop/purchase/Close buttons; edge mode also drives scoreboard and mobile touch utility controls",
                    "edge_scenario":qa.edge, "edge_fixture_limits":"target actors and populated scoreboard are client presentation fixtures; utility acknowledgments and purchase receipts are real server snapshots",
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
    let class_ready = qa.hero_class.is_none_or(|class| {
        if qa.stage == 0 {
            scene.selection.hero_class == class
        } else {
            scene
                .hero_classes
                .single()
                .is_ok_and(|actual| actual.0 == class)
        }
    });
    let ready = scenes_ready
        && class_ready
        && match qa.stage {
            0 => {
                session.is_connected()
                    && !session.join_confirmed()
                    && !scene.join.is_empty()
                    && *scene.screen.get() == crate::frontend::AppScreen::HeroSelect
            }
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
                    && if qa.edge {
                        scene.edge.as_deref().is_some_and(|edge| {
                            edge::purchase_verified(edge, equipment.single().ok())
                        })
                    } else {
                        equipment.single().is_ok_and(|equipment| {
                            equipment
                                .inventory
                                .contains(&shared::shop::ItemId::EmberBlade)
                                && equipment
                                    .last_purchase
                                    .as_ref()
                                    .is_some_and(|receipt| receipt.error.is_none())
                        })
                    }
            }
            5 => !shop.open && !pause.open && context.gameplay_allowed(),
            6 => session.join_confirmed() && matches!(game.state, GameState::Victory { .. }),
            7..=13 => {
                qa.edge
                    && edge::ready(
                        stage_for_edge(&qa),
                        &game,
                        &context,
                        scene.scoreboard.as_deref(),
                        scene.edge.as_deref(),
                        scene.utility.single().ok(),
                    )
            }
            _ => false,
        };
    if !ready && qa.started.elapsed() >= qa.next_readiness_report {
        qa.next_readiness_report = qa.started.elapsed() + Duration::from_secs(5);
        let diagnostic = serde_json::json!({
            "stage": qa.stage,
            "screen": format!("{:?}", scene.screen.get()),
            "connected": session.is_connected(), "admitted": session.join_confirmed(),
            "picker_roots": scene.join.iter().count(), "scenes_ready": scenes_ready, "class_ready":class_ready,
            "requested_class":qa.hero_class.map(|class|class.id()),
            "environment_roots": scene.environment.iter().count(),
            "scenes": scene.scenes.iter().map(|(root, instance)| serde_json::json!({
                "instance_ready": instance.is_some_and(|instance| spawner.instance_is_ready(**instance)),
                "dependencies": format!("{:?}", assets.recursive_dependency_load_state(root.0.id())),
            })).collect::<Vec<_>>(),
            "help_open": help.0, "shop_open": shop.open, "gameplay_allowed": context.gameplay_allowed(),
            "elapsed_seconds": qa.started.elapsed().as_secs_f64(),
        });
        let _ = std::fs::create_dir_all(&qa.directory);
        let _ = std::fs::write(
            qa.directory.join("qa-readiness.json"),
            serde_json::to_vec_pretty(&diagnostic).unwrap(),
        );
    }
    qa.frames = if ready { qa.frames + 1 } else { 0 };
    if qa.frames < SETTLE_FRAMES {
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        error!("BETA_UI_QA failed: cannot create output directory");
        exit.write(AppExit::error());
        return;
    }
    let primary_nodes: Vec<_> = scene.nodes.iter().filter(|(name, _, _, _)| edge::tracked(name.as_str()) || matches!(name.as_str(),
        "FindMatchButton" | "AvatarGrid" | "HelpDismissButton" | "HelpOverlayRoot" | "GameStateLabel" | "ConnectionStatusPanel" | "MinimapRoot" | "MatchObjectivePanel" | "MatchHudColumn" | "SkillBarRoot" | "SkillSlot-Q" | "SkillSlot-R" | "EquipmentHud" | "ShopOpenButton" | "ShopPanel" | "ShopCloseButton" | "ShopBuy-EB" | "ShopBuy-GC" | "ShopSummary" | "ShopFeedback" | "MobileJoystick" | "MobileAttack" | "MobileAbility-0" | "MobileAbility-1" | "MobileAbility-2" | "MobileAbility-3" | "MobileUpgrade-0" | "MobileUpgrade-1" | "MobileUpgrade-2" | "MobileUpgrade-3" | "PhoneMenuBar" | "QaSkillUpgradeFixtureLabel" | "SocialEntry" | "SocialStatus" | "CareerEntryActions" | "HudProgressionText" | "HudXpText" | "MatchStatusText" | "MatchBuffText" | "EquipmentGold")
            || name.as_str().starts_with("ShopBuy-") || name.as_str().starts_with("ShopDescription-") || name.as_str().starts_with("ShopDetails-") || name.as_str().starts_with("SkillName-") || name.as_str().starts_with("SkillRank-") || name.as_str().starts_with("SkillSlot-") || name.as_str().starts_with("SkillIcon-"))
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
    let shop_close_clear = !matches!(stage, 3 | 4)
        || primary_nodes
            .iter()
            .find(|node| node["name"] == "ShopCloseButton" && node["visible"] == true)
            .and_then(measured_rect)
            .is_some_and(|close| {
                primary_nodes
                    .iter()
                    .find(|node| node["name"] == "PhoneMenuBar" && node["visible"] == true)
                    .and_then(measured_rect)
                    .is_none_or(|bar| {
                        let overlap = close.intersect(bar);
                        overlap.width() <= 0.5 || overlap.height() <= 0.5
                    })
            });
    if !shop_close_clear {
        error!("BETA_UI_QA failed: shop Close is missing or obscured by phone utility controls");
        exit.write(AppExit::error());
        return;
    }
    let resting = edge::resting(stage);
    let minimap_top_left = !resting
        || primary_nodes.iter().any(|node| {
            if node["name"] != "MinimapRoot" || node["visible"] != true {
                return false;
            }
            let Some(rect) = measured_logical_rect(node) else {
                return false;
            };
            let expected = if phone {
                if qa.height <= 340 {
                    96.0
                } else {
                    116.0 * mobile.as_ref().unwrap().scale()
                }
            } else {
                144.0
            };
            let inset = if phone {
                mobile.as_ref().unwrap().safe.left
            } else {
                16.0
            };
            let top = if phone {
                mobile.as_ref().unwrap().safe.top
            } else {
                16.0
            };
            (rect.width() - expected).abs() <= 1.0
                && (rect.height() - expected).abs() <= 1.0
                && (rect.min.x - inset).abs() <= 1.0
                && (rect.min.y - top).abs() <= 1.0
        });
    let playfield_clear = !resting
        || primary_nodes
            .iter()
            .filter(|node| {
                node["visible"] == true && node["name"].as_str().is_some_and(persistent_hud_panel)
            })
            .all(|node| {
                measured_logical_rect(node).is_none_or(|rect| {
                    clears_playfield(rect, Vec2::new(qa.width as f32, qa.height as f32), phone)
                })
            });
    let radial_geometry_valid = !resting
        || !phone
        || mobile
            .as_deref()
            .is_some_and(|mobile| edge::mobile_geometry_valid(&primary_nodes, mobile));
    if !minimap_top_left || !playfield_clear || !radial_geometry_valid {
        error!(
            "BETA_UI_QA failed: edge placement/top-left map, open playfield or radial geometry: map={minimap_top_left} playfield={playfield_clear} radial={radial_geometry_valid} {primary_nodes:?}"
        );
        exit.write(AppExit::error());
        return;
    }
    let hud_text_fits = !resting
        || primary_nodes.iter().all(|text| {
            let Some(name) = text["name"].as_str() else {
                return true;
            };
            let parent = match name {
                "HudProgressionText" | "HudXpText" => Some("MatchHudColumn".to_owned()),
                "MatchStatusText" | "MatchBuffText" => Some("MatchObjectivePanel".to_owned()),
                "EquipmentGold" => Some("EquipmentHud".to_owned()),
                "TargetHealthName" | "TargetHealthValue" => Some("TargetHealthRoot".to_owned()),
                "QuickBuyPrice-0" => Some("QuickBuy-0".to_owned()),
                "QuickBuyPrice-1" => Some("QuickBuy-1".to_owned()),
                _ => name
                    .strip_prefix("SkillName-")
                    .or_else(|| name.strip_prefix("SkillRank-"))
                    .map(|slot| format!("SkillSlot-{slot}")),
            };
            if text["visible"] != true {
                return true;
            }
            let Some(parent) = parent else {
                return true;
            };
            let Some(rect) = measured_rect(text) else {
                return true;
            };
            primary_nodes
                .iter()
                .find(|node| node["name"] == parent)
                .and_then(measured_rect)
                .is_some_and(|parent| {
                    parent.inflate(1.0).contains(rect.min) && parent.inflate(1.0).contains(rect.max)
                })
        });
    if !hud_text_fits {
        error!("BETA_UI_QA failed: meaningful HUD text leaves its panel: {primary_nodes:?}");
        exit.write(AppExit::error());
        return;
    }
    // Key badges intentionally sit on artwork; full ability names and live
    // rank/status text must remain in the distinct lower card area, including
    // the longer Ranger and Cleric names.
    let skill_art_clear = !resting
        || mobile.as_ref().is_some_and(|mobile| mobile.enabled)
        || primary_nodes.iter().all(|text| {
            let Some(slot) = text["name"].as_str().and_then(|name| {
                name.strip_prefix("SkillName-")
                    .or_else(|| name.strip_prefix("SkillRank-"))
            }) else {
                return true;
            };
            if text["visible"] != true {
                return true;
            }
            let Some(text_rect) = measured_rect(text) else {
                return true;
            };
            primary_nodes
                .iter()
                .find(|node| node["name"] == format!("SkillIcon-{slot}"))
                .filter(|node| node["visible"] == true)
                .and_then(measured_rect)
                .is_none_or(|icon| {
                    let overlap = icon.intersect(text_rect);
                    overlap.width() <= 0.5 || overlap.height() <= 0.5
                })
        });
    if !skill_art_clear {
        error!("BETA_UI_QA failed: skill labels overlap their artwork: {primary_nodes:?}");
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
        0 => &["FindMatchButton", "AvatarGrid"],
        1 => &["HelpDismissButton", "HelpOverlayRoot"],
        3 | 4 => &[
            "ShopPanel",
            "ShopCloseButton",
            "ShopBuy-EB",
            "ShopBuy-GC",
            "ShopSummary",
        ],
        6 => &["GameStateLabel"],
        11 => &[
            "ScoreboardPanel",
            "ScoreboardCloseButton",
            "ScoreboardGreenRows",
            "ScoreboardBlueRows",
        ],
        _ if resting && phone => &[
            "MinimapRoot",
            "MatchHudColumn",
            "QuickBuyHud",
            "GoldShopButton",
            "QuickBuy-0",
            "QuickBuy-1",
            "MatchScoreButton",
            "MatchMenuButton",
            "MobileJoystick",
            "MobileAttack",
            "MobileAbility-0",
            "MobileAbility-1",
            "MobileAbility-2",
            "MobileAbility-3",
            "MobileMinionAttack",
            "MobileTowerAttack",
            "MobileDash",
            "MobileHaste",
        ],
        _ if resting => &[
            "MinimapRoot",
            "MatchHudColumn",
            "QuickBuyHud",
            "GoldShopButton",
            "QuickBuy-0",
            "QuickBuy-1",
            "MatchScoreButton",
            "MatchMenuButton",
            "SkillBarRoot",
            "SkillSlot-Q",
            "SkillSlot-R",
        ],
        _ => &[],
    };
    let synthetic_progression = progression_fixture
        .as_ref()
        .is_some_and(|fixture| fixture.applied);
    let upgrade_required: &[&str] =
        if synthetic_progression && mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
            &[
                "MobileRankMode",
                "MobileRankRing-0",
                "MobileRankRing-1",
                "MobileRankRing-2",
                "MobileRankRing-3",
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
        "QuickBuyHud",
        "MatchScoreStrip",
        "MatchMenuButton",
        "TargetHealthRoot",
        "MobileJoystick",
        "MobileAttack",
        "MobileAbility-0",
        "MobileAbility-1",
        "MobileAbility-2",
        "MobileAbility-3",
        "MobileMinionAttack",
        "MobileTowerAttack",
        "MobileDash",
        "MobileHaste",
        "MobileRankMode",
        "PhoneMenuBar",
        "SocialEntry",
        "SocialStatus",
        "CareerEntryActions",
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
    let dock_clear = !resting
        || dock.iter().enumerate().all(|(i, a)| {
            dock.iter().skip(i + 1).all(|b| {
                let delta_x =
                    (a["center"][0].as_f64().unwrap() - b["center"][0].as_f64().unwrap()).abs();
                let delta_y =
                    (a["center"][1].as_f64().unwrap() - b["center"][1].as_f64().unwrap()).abs();
                // Raw mobile touch controls are circles. Their bounding-square
                // corners may overlap without overlapping visible/hit regions.
                let circle =
                    |node: &serde_json::Value| node["name"].as_str().is_some_and(edge::circle);
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
        "mobile_controls":mobile.as_ref().is_some_and(|mobile| mobile.enabled), "admitted":session.join_confirmed(),
        "requested_class":qa.hero_class.map(|class|class.id()), "selected_class":scene.selection.hero_class.id(),
        "authoritative_class":scene.hero_classes.single().ok().map(|class|class.0.id()), "skill_art_clear":skill_art_clear, "shop_close_clear":shop_close_clear, "server_epoch":game.meta.server_epoch, "snapshot_tick":game.meta.snapshot_tick,
        "synthetic_result":stage == 6, "synthetic_progression":synthetic_progression,
        "progression_fixture":synthetic_progression.then(||serde_json::json!({"level":6,"skill_points":4,"ranks":[1,1,1,1],"server_unchanged":true})),
        "minimap":minimap.diagnostics(), "minimap_top_left":minimap_top_left, "playfield_clear":playfield_clear, "radial_geometry_valid":radial_geometry_valid, "hud_text_fits":hud_text_fits, "shop_modal":shop.open, "gameplay_allowed":context.gameplay_allowed(), "pause_open":pause.open,
        "edge":edge::record(stage, scene.edge.as_deref(), &game, scene.utility.single().ok(), &scene.texts),
        "equipment":equipment.single().ok().map(|e|serde_json::json!({"gold":e.gold,"inventory":e.inventory,"bonuses":e.item_bonuses,"receipt":e.last_purchase})), "primary_controls_fit":controls_fit, "shop_text_fits":shop_text_fits, "primary_nodes":primary_nodes});
    if qa.edge
        && !edge::validate(
            stage,
            &primary_nodes,
            &scene.texts,
            scene.edge.as_deref(),
            &context,
            scene.utility.single().ok(),
        )
    {
        error!("BETA_UI_QA failed: edge target/score/utility state assertions: {record}");
        exit.write(AppExit::error());
        return;
    }
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

/// Check screen-space panels only; world labels and transient feedback remain independent.
fn persistent_hud_panel(name: &str) -> bool {
    matches!(
        name,
        "MinimapRoot"
            | "MatchObjectivePanel"
            | "MatchHudColumn"
            | "SkillBarRoot"
            | "EquipmentHud"
            | "PhoneMenuBar"
            | "SocialEntry"
            | "SocialStatus"
            | "CareerEntryActions"
            | "ConnectionStatusPanel"
            | "QuickBuyHud"
            | "MatchScoreStrip"
            | "MatchMenuButton"
            | "TargetHealthRoot"
    )
}

fn measured_rect(node: &serde_json::Value) -> Option<Rect> {
    let center = Vec2::new(
        node["center"][0].as_f64()? as f32,
        node["center"][1].as_f64()? as f32,
    );
    let size = Vec2::new(
        node["size"][0].as_f64()? as f32,
        node["size"][1].as_f64()? as f32,
    );
    (center.is_finite() && size.is_finite() && size.min_element() > 0.0)
        .then(|| Rect::from_center_size(center, size))
}

fn measured_logical_rect(node: &serde_json::Value) -> Option<Rect> {
    let min = Vec2::new(
        node["logical_min"][0].as_f64()? as f32,
        node["logical_min"][1].as_f64()? as f32,
    );
    let size = Vec2::new(
        node["logical_size"][0].as_f64()? as f32,
        node["logical_size"][1].as_f64()? as f32,
    );
    (min.is_finite() && size.is_finite() && size.min_element() > 0.0)
        .then(|| Rect::from_corners(min, min + size))
}

fn stage_for_edge(qa: &BetaUiQa) -> usize {
    qa.stage
}

fn clears_playfield(panel: Rect, viewport: Vec2, phone: bool) -> bool {
    // Target health intentionally occupies upper-center; protect the actual hero
    // neighborhood and mobile lower-center opening from resting opaque panels.
    let regions = if phone {
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.viewport = viewport;
        let layout = mobile.layout();
        let gutter = 16.0 * mobile.combat_scale();
        [
            // Protect the central hero neighborhood. The complete radial group
            // now occupies the right third, including its outside rank control.
            Rect::from_corners(
                Vec2::new(0.35, 0.30) * viewport,
                Vec2::new(0.53, 0.60) * viewport,
            ),
            // Preserve the actual passage between both thumb groups, including
            // the joystick's full hit area and a gutter before utility buttons.
            Rect::from_corners(
                Vec2::new(
                    layout.joystick_center.x + layout.joystick_radius * 1.3 + gutter,
                    viewport.y * 0.62,
                ),
                Vec2::new(
                    layout.utility_centers[0].x - layout.auxiliary_radius - gutter,
                    viewport.y - mobile.safe.bottom - 3.0,
                ),
            ),
        ]
    } else {
        [Rect::from_corners(
            Vec2::new(0.30, 0.20) * viewport,
            Vec2::new(0.70, 0.65) * viewport,
        ); 2]
    };
    regions.into_iter().all(|region| {
        let overlap = panel.intersect(region);
        overlap.width() <= 0.5 || overlap.height() <= 0.5
    })
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    #[test]
    fn edge_guard_accepts_target_strip_but_rejects_old_phone_bottom_banner() {
        let viewport = Vec2::new(844.0, 390.0);
        assert!(clears_playfield(
            Rect::from_corners(Vec2::new(330.0, 10.0), Vec2::new(514.0, 40.0)),
            viewport,
            true
        ));
        assert!(!clears_playfield(
            Rect::from_corners(Vec2::new(186.0, 327.0), Vec2::new(530.0, 370.0)),
            viewport,
            true
        ));
        assert!(!clears_playfield(
            Rect::from_center_size(viewport * 0.5, Vec2::splat(40.0)),
            viewport,
            true
        ));
        assert!(clears_playfield(
            Rect::from_corners(Vec2::new(12.0, 182.0), Vec2::new(152.0, 214.0)),
            viewport,
            true
        ));
    }
    #[test]
    fn desktop_cards_and_target_strip_preserve_the_playfield_across_sizes() {
        for viewport in [
            Vec2::new(1024.0, 640.0),
            Vec2::new(1280.0, 720.0),
            Vec2::new(1920.0, 1080.0),
        ] {
            assert!(clears_playfield(
                Rect::from_center_size(Vec2::new(viewport.x * 0.5, 32.0), Vec2::new(224.0, 32.0)),
                viewport,
                false
            ));
            assert!(clears_playfield(
                Rect::from_center_size(
                    Vec2::new(viewport.x * 0.5, viewport.y - 68.0),
                    Vec2::new(360.0, 112.0)
                ),
                viewport,
                false
            ));
        }
    }

    #[test]
    fn radial_controls_leave_hero_and_lower_center_passage_open() {
        for viewport in [
            Vec2::new(693.0, 320.0),
            Vec2::new(844.0, 390.0),
            Vec2::new(932.0, 430.0),
        ] {
            let mut mobile = crate::mobile_controls::MobileControls::default();
            mobile.viewport = viewport;
            let layout = mobile.layout();
            for (center, radius) in layout
                .ability_centers
                .into_iter()
                .zip(
                    layout
                        .ability_radii
                        .map(|radius| radius + 3.0 * mobile.combat_scale()),
                )
                .chain(
                    layout
                        .utility_centers
                        .into_iter()
                        .map(|center| (center, layout.auxiliary_radius)),
                )
                .chain([
                    (layout.upgrade_center, layout.upgrade_radius),
                    (layout.cancel_center, layout.cancel_radius),
                ])
            {
                assert!(
                    clears_playfield(
                        Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
                        viewport,
                        true
                    ),
                    "control at {center:?} blocks {viewport:?}"
                );
            }
            assert!(!clears_playfield(
                Rect::from_center_size(viewport * 0.5, Vec2::splat(40.0)),
                viewport,
                true
            ));
            assert!(!clears_playfield(
                Rect::from_center_size(
                    Vec2::new(viewport.x * 0.4, viewport.y - 44.0),
                    Vec2::new(300.0, 48.0)
                ),
                viewport,
                true
            ));
        }
    }
}
