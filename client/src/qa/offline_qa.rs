//! Opt-in native smoke. Synthetic actions use production UI/network handlers;
//! screenshots come from the real renderer, without starting any server.
use crate::{
    frontend::AppScreen,
    net::{ClientSession, NetworkCommand, NetworkPlayerId, SessionUiCommand, TargetId, TargetKind},
    pause_menu::PauseMenuState,
    player::{MovementTarget, Player},
};
use bevy::{
    app::AppExit,
    ecs::system::RunSystemOnce,
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};
pub(crate) struct OfflineQaPlugin;
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    stage: u8,
    started: Instant,
    since: Instant,
    origin: Vec3,
    moved: f32,
    hit: bool,
    restored: bool,
    expected_server: String,
    target_id: u64,
    kit_feedback: bool,
    reentered: bool,
    quick_buy_requested: bool,
    quick_buy_confirmed: bool,
    chat_sent: bool,
    chat_confirmed: bool,
    chat_readable: bool,
    chat_layout: serde_json::Value,
    debug_layout: serde_json::Value,
    chat_captured: bool,
    portrait_selected: bool,
    recall_confirmed: bool,
    recall_visible_since: Option<Instant>,
    recall_visual_submitted: bool,
    recall_readback_active: bool,
    recall_diagnostic: serde_json::Value,
    reaction_sent: bool,
    reaction_confirmed: bool,
    proof_step: u8,
    completed_captures: HashSet<String>,
    shop_frames: Vec<serde_json::Value>,
    shop_drag: Option<(Vec2, Vec2)>,
    shop_scrolls: u8,
}
impl Plugin for OfflineQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_OFFLINE_SMOKE_DIR").map(PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&directory).expect("QA directory");
        app.insert_resource(Qa {
            directory,
            stage: 0,
            started: Instant::now(),
            since: Instant::now(),
            origin: Vec3::ZERO,
            moved: 0.0,
            hit: false,
            restored: false,
            target_id: 0,
            kit_feedback: false,
            reentered: false,
            quick_buy_requested: false,
            quick_buy_confirmed: false,
            chat_sent: false,
            chat_confirmed: false,
            chat_readable: false,
            chat_layout: serde_json::Value::Null,
            debug_layout: serde_json::Value::Null,
            chat_captured: false,
            portrait_selected: false,
            recall_confirmed: false,
            recall_visible_since: None,
            recall_visual_submitted: false,
            recall_readback_active: false,
            recall_diagnostic: serde_json::Value::Null,
            reaction_sent: false,
            reaction_confirmed: false,
            proof_step: 0,
            completed_captures: HashSet::new(),
            shop_frames: Vec::new(),
            shop_drag: None,
            shop_scrolls: 0,
            expected_server: std::env::var("GAME_SERVER_ADDR").unwrap_or_default(),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(PreUpdate, (focus, drive.after(bevy::ui::UiSystems::Focus)))
        .add_systems(
            Update,
            fixture_window.before(crate::mobile_controls::MobileControlsSet::Layout),
        )
        .add_systems(Last, (fixture_pacing, capture_recall));
    }
}
fn focus(windows: Query<Entity, With<PrimaryWindow>>, _main: bevy::ecs::system::NonSendMarker) {
    let Ok(entity) = windows.single() else {
        return;
    };
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(w) = windows.get_window(entity)
            && !w.has_focus()
        {
            w.focus_window();
        }
    });
}
fn synthetic_focus_enabled() -> bool {
    std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref() == Ok("1")
}

// Explicit QA fixture input only. This does not claim native OS focus or alter
// the production foreground policy; the normal layout/context systems run next.
fn fixture_window(
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut mobile: ResMut<crate::mobile_controls::MobileControls>,
) {
    if synthetic_focus_enabled() {
        for mut window in &mut windows {
            window.focused = true;
        }
        mobile.focused = true;
    }
}

fn fixture_pacing(mut pacing: ResMut<bevy::winit::WinitSettings>) {
    if synthetic_focus_enabled() {
        // Native background focus can still select Winit's unfocused mode.
        // Keep this opt-in input fixture moving; its cadence is not FPS proof.
        *pacing = bevy::winit::WinitSettings::continuous();
    }
}

fn capture(world: &mut World, qa: &Qa, name: &str) {
    // Focused modes render exactly their requested evidence state.
    if (std::env::var("OMOBA_OFFLINE_CHAT_QA_ONLY").as_deref() == Ok("1")
        && name != "03-local-chat.png")
        || (debug_only() && name != "01-debug-tools-bottom.png")
    {
        return;
    }
    // Reuse verified menus, but never reuse a failed gameplay proof frame.
    let gameplay = matches!(
        name,
        "03-practice.png"
            | "03-local-chat.png"
            | "04-target-recall.png"
            | "09-reentered-wildspark.png"
            | "01-shop-owned-component.png"
            | "02-shop-scrolled-upgrades.png"
    );
    if !gameplay && qa.directory.join(name).exists() {
        return;
    }
    let capture_name = name.to_owned();
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.directory.join(name)))
        .observe(
            move |_: On<ScreenshotCaptured>,
                  mut qa: ResMut<Qa>,
                  proof: Res<crate::recall::RecallVisualProof>,
                  utilities: Query<&crate::net::PlayerUtility, With<Player>>| {
                qa.completed_captures.insert(capture_name.clone());
                if capture_name == "04-target-recall.png" {
                    qa.recall_readback_active = proof.drawn_heroes > 0
                        && proof.max_remaining_secs > 0.0
                        && utilities
                            .iter()
                            .any(|utility| utility.state.recall_remaining_secs > 0.0);
                    qa.recall_diagnostic["active_on_readback"] = qa.recall_readback_active.into();
                    qa.recall_diagnostic["remaining_on_readback"] = proof.max_remaining_secs.into();
                }
            },
        );
}

// Last runs after the production PostUpdate recall renderer. A positive timer
// from the previous frame alone cannot qualify this screenshot.
fn capture_recall(world: &mut World) {
    if world.get_resource::<Qa>().is_none_or(|qa| qa.stage != 8) {
        return;
    }
    let mut qa = world.remove_resource::<Qa>().unwrap();
    let proof = world.resource::<crate::recall::RecallVisualProof>();
    let drawn = proof.drawn_heroes;
    let remaining = proof.max_remaining_secs;
    let active = world
        .query_filtered::<&crate::net::PlayerUtility, With<Player>>()
        .iter(world)
        .any(|utility| utility.state.recall_remaining_secs > 0.0);
    qa.recall_diagnostic = serde_json::json!({"drawn_heroes":drawn,"max_remaining_secs":remaining,"local_channel_active":active});
    if drawn > 0 && remaining > 0.0 && active {
        let since = qa.recall_visible_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= Duration::from_millis(500)
            && ui_ready(world, &format!("EnemyPortrait-{}", qa.target_id))
            && ui_ready(world, "SocialReactionBubble")
        {
            qa.recall_confirmed = true;
            qa.recall_visual_submitted = true;
            capture(world, &qa, "04-target-recall.png");
            qa.stage = 9;
            qa.since = Instant::now();
            info!("Offline QA captured stable rendered recall channel");
        }
    } else {
        qa.recall_visible_since = None;
    }
    world.insert_resource(qa);
}

fn ui_ready(world: &mut World, id: &str) -> bool {
    world
        .query::<(
            Option<&crate::ui::TestId>,
            Option<&Name>,
            &ComputedNode,
            Option<&InheritedVisibility>,
        )>()
        .iter(world)
        .any(|(test_id, name, node, visible)| {
            (test_id.is_some_and(|test_id| test_id.as_str() == id)
                || name.is_some_and(|name| name.as_str() == id))
                && node.size().min_element() > 0.0
                && visible.is_none_or(|visible| visible.get())
        })
}

fn gameplay_ready(world: &mut World) -> bool {
    let mobile = world.resource::<crate::mobile_controls::MobileControls>();
    let session = world.resource::<ClientSession>();
    let game = world.resource::<crate::net::GameStateSnapshot>();
    mobile.enabled
        && mobile.landscape
        && mobile.focused
        && session.is_offline()
        && session.join_confirmed()
        && matches!(game.state, crate::net::GameState::Running)
        && world
            .query_filtered::<&crate::combat::CombatStats, With<Player>>()
            .single(world)
            .is_ok_and(|stats| stats.is_alive())
        && world
            .query_filtered::<&Window, With<PrimaryWindow>>()
            .single(world)
            .is_ok_and(|window| window.focused)
}

fn readable_chat_layout(world: &mut World) -> Option<serde_json::Value> {
    let nodes: Vec<_> = world
        .query::<(
            crate::qa::QaName,
            &ComputedNode,
            &UiGlobalTransform,
            Option<&bevy::ui::CalculatedClip>,
            Option<&InheritedVisibility>,
        )>()
        .iter(world)
        .filter(|(_, node, _, _, visible)| {
            node.size().min_element() > 0.0 && visible.is_none_or(|visible| visible.get())
        })
        .map(|(name, node, transform, clip, _)| {
            let dpi = 1.0 / node.inverse_scale_factor();
            (
                name.as_str().to_owned(),
                crate::ui::gesture::logical_ui_rect(node, transform, None, dpi),
                crate::ui::gesture::logical_ui_rect(node, transform, clip, dpi),
            )
        })
        .collect();
    let (_, _, log) = nodes.iter().find(|(name, _, _)| name == "SocialChatLog")?;
    let (_, message, visible) = nodes
        .iter()
        .find(|(name, _, _)| name == "SocialChatMessage")?;
    if log.height() < 72.0
        || message.height() < 14.0
        || message.min.distance(visible.min) > 0.5
        || message.max.distance(visible.max) > 0.5
        || !log.contains(message.min)
        || !log.contains(message.max)
        || ["SocialSend", "SocialClose"].iter().any(|id| {
            !nodes
                .iter()
                .any(|(name, _, visible)| name == id && visible.height() >= 44.0)
        })
    {
        return None;
    }
    Some(serde_json::json!({
        "history_height": log.height(),
        "message_bounds": [message.min.x, message.min.y, message.max.x, message.max.y],
        "visible_message_bounds": [visible.min.x, visible.min.y, visible.max.x, visible.max.y],
    }))
}

/// Presses the UI-kit button carrying `id` with a `SyntheticPress`.
fn press(world: &mut World, id: &str) -> bool {
    let entity = world
        .query_filtered::<(Entity, &crate::ui::TestId), With<crate::ui::Pressable>>()
        .iter(world)
        .find(|(_, test_id)| test_id.as_str() == id)
        .map(|(entity, _)| entity);
    entity
        .map(|entity| world.write_message(crate::ui::SyntheticPress(entity)))
        .is_some()
}
fn debug_only() -> bool {
    std::env::var("OMOBA_OFFLINE_DEBUG_QA_ONLY").as_deref() == Ok("1")
}

fn drive_debug_tools(world: &mut World, qa: &mut Qa, age: f32) {
    match qa.proof_step {
        0 if age > 1.0 => {
            world.resource_mut::<PauseMenuState>().open = true;
            if ui_ready(world, "PauseMenuPracticeButton") && press(world, "PauseMenuPracticeButton")
            {
                qa.proof_step = 1;
                qa.since = Instant::now();
            }
        }
        1 if age > 0.5 && ui_ready(world, "PauseMenuPracticeSection") => {
            for (name, mut scroll) in world
                .query::<(&Name, &mut ScrollPosition)>()
                .iter_mut(world)
            {
                if name.as_str() == "PauseMenuPracticeSection" {
                    scroll.y = 100_000.0;
                }
            }
            qa.proof_step = 2;
            qa.since = Instant::now();
        }
        2 if age > 0.5 => {
            let mut bottom_visible = false;
            let mut offset = 0.0;
            for (name, node, pose, clip, visible, scroll) in world
                .query::<(
                    crate::qa::QaName,
                    &ComputedNode,
                    &UiGlobalTransform,
                    Option<&bevy::ui::CalculatedClip>,
                    Option<&InheritedVisibility>,
                    Option<&ScrollPosition>,
                )>()
                .iter(world)
            {
                if name.as_str() == "PauseMenuPracticeSection" {
                    offset = scroll.map_or(0.0, |scroll| scroll.y);
                }
                if name.as_str() == "PauseMenuPracticeBackButton"
                    && visible.is_some_and(|visible| visible.get())
                {
                    let full = crate::ui::gesture::logical_ui_rect(node, pose, None, 1.0);
                    let shown = crate::ui::gesture::logical_ui_rect(node, pose, clip, 1.0);
                    bottom_visible = shown.height() >= 40.0
                        && shown.min.distance(full.min) < 1.0
                        && shown.max.distance(full.max) < 1.0;
                }
            }
            if bottom_visible && offset > 0.0 {
                qa.debug_layout = serde_json::json!({"scroll_offset":offset,"last_action_fully_visible":true,"scroll_method":"fixture offset; raw touch separately regression tested"});
                capture(world, qa, "01-debug-tools-bottom.png");
                qa.proof_step = 3;
            }
        }
        3 if qa.completed_captures.contains("01-debug-tools-bottom.png") => {
            std::fs::write(qa.directory.join("debug-result.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "pass":true,"viewport":[852,393],"language":"en","physical_device_verified":false,
                "geometry":qa.debug_layout,"file":"01-debug-tools-bottom.png"
            })).unwrap()).unwrap();
            qa.stage = 250;
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

fn shop_only() -> bool {
    std::env::var("OMOBA_OFFLINE_SHOP_QA_ONLY").as_deref() == Ok("1")
}

struct ShopLayout {
    rect: Rect,
    offset: f32,
    max_offset: f32,
    ready: bool,
    nodes: serde_json::Value,
}

fn shop_layout(world: &mut World) -> Option<ShopLayout> {
    let mut viewport = None;
    let mut close = false;
    let mut card = false;
    let mut nodes = Vec::new();
    for (key, node, pose, clip, visible, scroll, text) in world
        .query::<(
            crate::qa::QaName,
            &ComputedNode,
            &UiGlobalTransform,
            Option<&bevy::ui::CalculatedClip>,
            Option<&InheritedVisibility>,
            Option<&ScrollPosition>,
            Option<&Text>,
        )>()
        .iter(world)
    {
        let name = key.as_str();
        if !name.starts_with("Shop")
            || node.size().min_element() <= 0.0
            || visible.is_some_and(|v| !v.get())
        {
            continue;
        }
        let dpi = 1.0 / node.inverse_scale_factor();
        let rect = crate::ui::gesture::logical_ui_rect(node, pose, clip, dpi);
        if name == "ShopCards" {
            viewport = Some((
                rect,
                scroll.map_or(0.0, |s| s.0.y),
                crate::ui::scroll::max_offset(node),
            ));
        }
        if name == "ShopCloseButton" {
            close = rect.width() >= 44.0 && rect.height() >= 43.5;
        }
        if name.starts_with("ShopBuy-") && rect.width() >= 44.0 && rect.height() >= 44.0 {
            card = true;
        }
        nodes.push(serde_json::json!({"name":name,"visible_rect":[rect.min.x,rect.min.y,rect.max.x,rect.max.y],"text":text.map(|t|t.0.as_str())}));
    }
    let (rect, offset, max_offset) = viewport?;
    Some(ShopLayout {
        rect,
        offset,
        max_offset,
        ready: close && card && rect.height() >= 80.0,
        nodes: nodes.into(),
    })
}

fn shop_touch(world: &mut World, phase: TouchPhase, position: Vec2) {
    if let Ok(window) = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
    {
        world.write_message(TouchInput {
            window,
            id: 918,
            phase,
            position,
            force: None,
        });
    }
}

/// One real local purchase followed by two catalog frames; all scrolling goes
/// through the normal raw-touch recognizer. No menu/social/combat capture sweep.
fn drive_shop(world: &mut World, qa: &mut Qa, age: f32) {
    match qa.proof_step {
        0 if age > 0.5 && ui_ready(world, "QuickBuy-0") => {
            if press(world, "QuickBuy-0") {
                qa.quick_buy_requested = true;
                qa.proof_step = 1;
                qa.since = Instant::now();
            }
        }
        1 => {
            qa.quick_buy_confirmed = world
                .query_filtered::<&crate::net::PlayerEquipment, With<Player>>()
                .single(world)
                .is_ok_and(|equipment| {
                    !equipment.inventory.is_empty()
                        && equipment
                            .last_purchase
                            .as_ref()
                            .is_some_and(|receipt| receipt.error.is_none())
                });
            if qa.quick_buy_confirmed && press(world, "GoldShopButton") {
                qa.proof_step = 2;
                qa.since = Instant::now();
            }
        }
        2 if age > 0.4 => {
            if let Some(layout) =
                shop_layout(world).filter(|layout| layout.ready && layout.max_offset > 20.0)
            {
                let equipment = world
                    .query_filtered::<&crate::net::PlayerEquipment, With<Player>>()
                    .single(world)
                    .unwrap();
                let quotes: Vec<_> = shared::shop::items().iter().filter_map(|item| {
                    let quote = shared::shop::upgrade_quote(item.id,&equipment.inventory);
                    (!quote.consumed.is_empty()).then(||serde_json::json!({"item":item.id,"full_price":item.cost,"remaining_price":quote.cost,"components":quote.consumed}))
                }).collect();
                if quotes.is_empty() {
                    return;
                }
                qa.shop_frames.push(serde_json::json!({"file":"01-shop-owned-component.png","scroll_offset":layout.offset,"scroll_max":layout.max_offset,"inventory":equipment.inventory,"purchase_receipt":equipment.last_purchase,"component_credit_quotes":quotes,"nodes":layout.nodes}));
                capture(world, qa, "01-shop-owned-component.png");
                qa.proof_step = 3;
                qa.since = Instant::now();
            }
        }
        3 if qa
            .completed_captures
            .contains("01-shop-owned-component.png")
            && age > 0.2 =>
        {
            if let Some(layout) = shop_layout(world).filter(|layout| layout.ready) {
                if layout.offset >= layout.max_offset - 2.0 && qa.shop_scrolls > 0 {
                    qa.shop_frames.push(serde_json::json!({"file":"02-shop-scrolled-upgrades.png","scroll_offset":layout.offset,"scroll_max":layout.max_offset,"raw_touch_drags":qa.shop_scrolls,"nodes":layout.nodes}));
                    capture(world, qa, "02-shop-scrolled-upgrades.png");
                    qa.proof_step = 7;
                    qa.since = Instant::now();
                } else if qa.shop_scrolls < 12 {
                    let start = Vec2::new(layout.rect.center().x, layout.rect.max.y - 12.0);
                    let end = Vec2::new(start.x, layout.rect.min.y + 12.0);
                    qa.shop_drag = Some((start, end));
                    shop_touch(world, TouchPhase::Started, start);
                    qa.proof_step = 4;
                    qa.since = Instant::now();
                }
            }
        }
        4 if age > 0.12 => {
            if let Some((_, end)) = qa.shop_drag {
                shop_touch(world, TouchPhase::Moved, end);
                qa.proof_step = 5;
                qa.since = Instant::now();
            }
        }
        5 if age > 0.12 => {
            if let Some((_, end)) = qa.shop_drag {
                shop_touch(world, TouchPhase::Ended, end);
                qa.shop_scrolls += 1;
                qa.proof_step = 3;
                qa.since = Instant::now();
            }
        }
        7 if qa
            .completed_captures
            .contains("02-shop-scrolled-upgrades.png") =>
        {
            let pass = qa.quick_buy_confirmed && qa.shop_scrolls > 0 && qa.shop_frames.len() == 2;
            let viewport = world
                .query_filtered::<&Window, With<PrimaryWindow>>()
                .single(world)
                .map(|window| [window.width(), window.height()])
                .unwrap_or([0.0, 0.0]);
            std::fs::write(qa.directory.join("shop-result.json"),serde_json::to_vec_pretty(&serde_json::json!({
                "pass":pass,"offline_purchase_confirmed":qa.quick_buy_confirmed,"raw_touch_scrolls":qa.shop_scrolls,
                "frames":qa.shop_frames,"viewport":viewport,"language":"en","physical_device_verified":false,
                "synthetic_window_focus":synthetic_focus_enabled(),"method":"Native renderer, production purchase and raw-touch scrolling; local authority."
            })).unwrap()).unwrap();
            qa.stage = 250;
            world.write_message(if pass {
                AppExit::Success
            } else {
                AppExit::error()
            });
        }
        _ => {}
    }
}

fn drive(world: &mut World) {
    let Some(mut qa) = world.remove_resource::<Qa>() else {
        return;
    };
    if qa.started.elapsed() > Duration::from_secs(180) {
        std::fs::write(
            qa.directory.join("result.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "pass":false, "timeout_stage":qa.stage, "proof_step":qa.proof_step,
                "synthetic_window_focus":synthetic_focus_enabled(),
                "mobile_focused":world.resource::<crate::mobile_controls::MobileControls>().focused,
                "context":format!("{:?}",world.resource::<crate::input_context::GameplayInputContext>()),
                "social":world.resource::<crate::social::SocialClient>().qa_diagnostics(),
                "quick_buy_confirmed":qa.quick_buy_confirmed,"chat_confirmed":qa.chat_confirmed,
                "chat_readable":qa.chat_readable,"chat_layout":qa.chat_layout,
                "reaction_confirmed":qa.reaction_confirmed,"moved":qa.moved,
                "target_id":qa.target_id,"portrait_selected":qa.portrait_selected,
                "hit":qa.hit,"recall_confirmed":qa.recall_confirmed,
                "recall_visual_submitted":qa.recall_visual_submitted,
                "recall_readback_active":qa.recall_readback_active,"recall_diagnostic":qa.recall_diagnostic,
                "completed_captures":qa.completed_captures,
                "shop_frames":qa.shop_frames,"shop_scrolls":qa.shop_scrolls,
            })).unwrap(),
        )
        .unwrap();
        world.write_message(AppExit::error());
        return;
    }
    if qa.stage == 0 {
        if let Ok(mut window) = world
            .query_filtered::<&mut Window, With<PrimaryWindow>>()
            .single_mut(world)
        {
            let width = std::env::var("OMOBA_QA_WIDTH")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(if shop_only() || debug_only() {
                    852
                } else {
                    1180
                });
            let height = std::env::var("OMOBA_QA_HEIGHT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(if shop_only() || debug_only() {
                    393
                } else {
                    820
                });
            window.resolution.set_scale_factor_override(Some(1.0));
            window.resolution.set(width as f32, height as f32);
        }
    }
    let screen = *world.resource::<State<AppScreen>>().get();
    // First-run accounts show the normal controls guide after admission.
    // Dismiss it through its production button before attempting gameplay.
    if qa.stage == 5
        && world
            .resource::<crate::help_overlay::HelpOverlayVisible>()
            .0
    {
        press(world, "HelpDismissButton");
        qa.since = Instant::now();
        world.insert_resource(qa);
        return;
    }
    // Start the gameplay proof clock after loading, so a slow asset load cannot
    // collapse chat-open, capture, close and purchase into a single frame.
    if qa.stage == 5 && screen != AppScreen::InMatch {
        qa.since = Instant::now();
    }
    let age = qa.since.elapsed().as_secs_f32();
    if matches!(qa.stage, 5..=8 | 19) && screen == AppScreen::InMatch && !gameplay_ready(world) {
        qa.since = Instant::now();
        world.insert_resource(qa);
        return;
    }
    if debug_only() && qa.stage == 5 && screen == AppScreen::InMatch {
        drive_debug_tools(world, &mut qa, age);
        world.insert_resource(qa);
        return;
    }
    if shop_only() && qa.stage == 5 && screen == AppScreen::InMatch {
        drive_shop(world, &mut qa, age);
        world.insert_resource(qa);
        return;
    }
    if qa.stage == 5 && screen == AppScreen::InMatch {
        let actions_ready = world
            .resource::<crate::input_context::GameplayInputContext>()
            .gameplay_allowed();
        match qa.proof_step {
            0 if age > 1.0 && actions_ready && ui_ready(world, "QuickBuy-0") => {
                qa.quick_buy_requested = press(world, "QuickBuy-0");
                if qa.quick_buy_requested {
                    qa.proof_step = 1;
                    qa.since = Instant::now();
                }
            }
            1 => {
                qa.quick_buy_confirmed = world
                    .query_filtered::<&crate::net::PlayerEquipment, With<Player>>()
                    .iter(world)
                    .any(|equipment| {
                        !equipment.inventory.is_empty()
                            && equipment
                                .last_purchase
                                .as_ref()
                                .is_some_and(|receipt| receipt.error.is_none())
                    });
                if qa.quick_buy_confirmed && actions_ready && ui_ready(world, "SocialOpenChat") {
                    world
                        .run_system_once(
                            |mut social: ResMut<crate::social::SocialClient>,
                             mut out: MessageWriter<NetworkCommand>| {
                                social.qa_send_chat(&mut out);
                                // Successful send releases gameplay; reopen the transcript for this capture.
                                assert!(!social.chat_open);
                                social.qa_open_transcript();
                            },
                        )
                        .expect("offline QA chat system");
                    qa.chat_sent = true;
                    qa.proof_step = 2;
                    qa.since = Instant::now();
                }
            }
            2 if age > 0.25 && ui_ready(world, "SocialChatPanel") => {
                let social = world.resource::<crate::social::SocialClient>();
                let diagnostics = social.qa_diagnostics();
                qa.chat_confirmed = !social.events.is_empty()
                    && diagnostics["pending"].is_null()
                    && diagnostics["status"] == "";
                if qa.chat_confirmed
                    && let Some(layout) = readable_chat_layout(world)
                {
                    qa.chat_readable = true;
                    qa.chat_layout = layout;
                    capture(world, &qa, "03-local-chat.png");
                    qa.chat_captured = true;
                    qa.proof_step = 3;
                    qa.since = Instant::now();
                }
            }
            3 if qa.completed_captures.contains("03-local-chat.png") => {
                if std::env::var("OMOBA_OFFLINE_CHAT_QA_ONLY").as_deref() == Ok("1") {
                    std::fs::write(
                        qa.directory.join("chat-result.json"),
                        serde_json::to_vec_pretty(&serde_json::json!({
                            "pass":qa.chat_confirmed && qa.chat_readable,
                            "chat_readable":qa.chat_readable,"chat_layout":qa.chat_layout,
                            "local_chat_without_pending_ack":qa.chat_confirmed,
                            "synthetic_window_focus":synthetic_focus_enabled(),
                            "screenshot":"03-local-chat.png",
                            "physical_device_verified":false,
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                    world.write_message(AppExit::Success);
                    return;
                }
                world
                    .resource_mut::<crate::social::SocialClient>()
                    .qa_close();
                qa.proof_step = 4;
                qa.since = Instant::now();
            }
            4 if age > 0.25 && actions_ready && ui_ready(world, "MobileAbility-0") => {
                world
                    .run_system_once(
                        |mut social: ResMut<crate::social::SocialClient>,
                         mut out: MessageWriter<NetworkCommand>| {
                            social.qa_send_reaction(&mut out);
                        },
                    )
                    .expect("offline QA reaction system");
                qa.reaction_sent = true;
                qa.proof_step = 5;
                qa.since = Instant::now();
            }
            5 if age > 0.1 && ui_ready(world, "SocialReactionBubble") => {
                qa.reaction_confirmed = world
                    .resource::<crate::social::SocialClient>()
                    .reactions
                    .len()
                    == 1;
                if qa.reaction_confirmed {
                    capture(world, &qa, "03-practice.png");
                    qa.proof_step = 6;
                    qa.since = Instant::now();
                }
            }
            _ => {}
        }
    }
    let mut advance = false;
    match qa.stage {
        0 if age > 4.0 && screen == AppScreen::Home => {
            if !shop_only() {
                capture(world, &qa, "01-home.png");
            }
            advance = true;
        }
        1 if age > 0.5 => {
            advance = press(world, "HomeOfflinePractice");
        }
        2 if age > 4.0 && screen == AppScreen::HeroSelect => {
            press(world, "ClassButton-dawnweaver");
            advance = true;
        }
        3 if age > 2.0 => {
            if !shop_only() {
                capture(world, &qa, "02-hero-picker.png");
            }
            advance = true;
        }
        4 if age > 0.5 => {
            advance = press(world, "FindMatchButton");
        }
        5 if qa.proof_step == 6
            && qa.completed_captures.contains("03-practice.png")
            && world
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed() =>
        {
            if let Ok((entity, t)) = world
                .query_filtered::<(Entity, &Transform), With<Player>>()
                .single(world)
            {
                qa.origin = t.translation;
                let destination = t.translation + Vec3::new(4.0, 0.0, 3.0);
                world.entity_mut(entity).insert(MovementTarget {
                    target: destination,
                });
                world.write_message(NetworkCommand::Debug(
                    shared::debug::DebugCommand::Practice(
                        shared::practice::PracticeCommand::ClearBots,
                    ),
                ));
                world.write_message(NetworkCommand::Debug(
                    shared::debug::DebugCommand::Practice(
                        shared::practice::PracticeCommand::SpawnDummy,
                    ),
                ));
                advance = true;
            }
        }
        6 if age > 3.0 => {
            if let Ok(t) = world
                .query_filtered::<&Transform, With<Player>>()
                .single(world)
            {
                qa.moved = t.translation.distance(qa.origin);
            }
            let dummy = world
                .query::<(
                    Entity,
                    &NetworkPlayerId,
                    &Transform,
                    &crate::combat::CombatStats,
                )>()
                .iter(world)
                .find(|(_, id, _, stats)| id.0 != 1 && stats.max_hp >= shared::debug::DUMMY_MAX_HP)
                .map(|(_, id, t, _)| (id.0, t.translation));
            if let Some((id, position)) = dummy
                && qa.moved > 1.0
                && ui_ready(world, &format!("EnemyPortrait-{id}"))
            {
                qa.target_id = id;
                press(world, &format!("EnemyPortrait-{id}"));
                world.write_message(NetworkCommand::CastSkill {
                    slot: 0,
                    aim: Vec2::new(position.x, position.z),
                });
                advance = true;
            }
        }
        7 if age > 0.25 => {
            qa.hit = world
                .query::<(&NetworkPlayerId, &crate::combat::CombatStats)>()
                .iter(world)
                .any(|(id, c)| id.0 == qa.target_id && c.hp < c.max_hp);
            qa.kit_feedback |= world
                .query::<(
                    &NetworkPlayerId,
                    &crate::net::PlayerLoadout,
                    &crate::net::PlayerSkillCooldowns,
                )>()
                .iter(world)
                .any(|(id, kit, cooldowns)| {
                    id.0 == 1 && kit.0.is_some() && cooldowns.remaining_secs[0] > 0.0
                });
            qa.portrait_selected = world
                .resource::<crate::combat::TargetState>()
                .selected_target
                == Some(TargetId {
                    kind: TargetKind::Player,
                    id: qa.target_id,
                });
            if !qa.hit
                || !qa.portrait_selected
                || !qa.kit_feedback
                || !world
                    .resource::<crate::input_context::GameplayInputContext>()
                    .gameplay_allowed()
                || !ui_ready(world, "MobileRecall")
            {
                world.insert_resource(qa);
                return;
            }
            // A real touch stream goes through the mobile recall handler and the
            // local command authority; the ring follows its replicated timer.
            let position = world
                .resource::<crate::mobile_controls::MobileControls>()
                .layout()
                .recall_center;
            if let Ok(window) = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
            {
                for phase in [TouchPhase::Started, TouchPhase::Ended] {
                    world.write_message(TouchInput {
                        window,
                        phase,
                        position,
                        id: 91234,
                        force: None,
                    });
                }
            }
            world
                .run_system_once(
                    |mut social: ResMut<crate::social::SocialClient>,
                     mut out: MessageWriter<NetworkCommand>| {
                        social.qa_send_reaction(&mut out);
                    },
                )
                .expect("recall-stage reaction");
            advance = true;
        }
        9 if qa.completed_captures.contains("04-target-recall.png")
            && !world.resource::<PauseMenuState>().open =>
        {
            world.resource_mut::<PauseMenuState>().open = true;
        }
        9 if age > 1.0
            && qa.completed_captures.contains("04-target-recall.png")
            && world.resource::<PauseMenuState>().open
            && ui_ready(world, "PauseMenuPracticeButton") =>
        {
            capture(world, &qa, "05-game-menu.png");
            advance = true;
        }
        10 if age > 0.5 => {
            advance = press(world, "PauseMenuPracticeButton");
        }
        11 if age > 1.0 => {
            capture(world, &qa, "06-practice-controls.png");
            advance = true;
        }
        12 if age > 0.5 => {
            advance = press(world, "PauseMenuPracticeGodModeButton");
        }
        13 if age > 0.5 => {
            advance = true;
        }
        14 if age > 0.5 => {
            world.resource_mut::<PauseMenuState>().open = false;
            world.write_message(SessionUiCommand::LeaveMatch);
            advance = true;
        }
        15 if age > 2.0 && screen == AppScreen::Home => {
            qa.restored = world.resource::<ClientSession>().server_addr() == qa.expected_server
                && !world.resource::<ClientSession>().is_offline()
                && world
                    .query_filtered::<Entity, With<Player>>()
                    .iter(world)
                    .count()
                    == 0;
            capture(world, &qa, "08-return-home.png");
            advance = true;
        }
        16 if age > 0.5 => {
            advance = press(world, "HomeOfflinePractice");
        }
        17 if age > 2.0 && screen == AppScreen::HeroSelect => {
            press(world, "ClassButton-wildspark");
            advance = true;
        }
        18 if age > 1.0 => {
            advance = press(world, "FindMatchButton");
        }
        19 if age > 2.0
            && screen == AppScreen::InMatch
            && world
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed()
            && ui_ready(world, "MobileAbility-0") =>
        {
            qa.reentered = world.resource::<ClientSession>().is_offline()
                && world.resource::<ClientSession>().join_confirmed()
                && world
                    .query_filtered::<&crate::net::NetworkHeroClass, With<Player>>()
                    .iter(world)
                    .any(|class| class.0 == shared::HeroClass::Wildspark);
            capture(world, &qa, "09-reentered-wildspark.png");
            advance = true;
        }
        20 if age > 1.0 && qa.completed_captures.contains("09-reentered-wildspark.png") => {
            world.write_message(SessionUiCommand::LeaveMatch);
            advance = true;
        }
        21 if age > 2.0 && screen == AppScreen::Home => {
            let files = [
                "01-home.png",
                "02-hero-picker.png",
                "03-practice.png",
                "03-local-chat.png",
                "04-target-recall.png",
                "05-game-menu.png",
                "06-practice-controls.png",
                "09-reentered-wildspark.png",
                "08-return-home.png",
            ];
            let passed = qa.moved > 1.0
                && qa.hit
                && qa.restored
                && qa.kit_feedback
                && qa.reentered
                && qa.quick_buy_confirmed
                && qa.chat_confirmed
                && qa.chat_readable
                && qa.reaction_confirmed
                && qa.portrait_selected
                && qa.recall_confirmed
                && qa.recall_visual_submitted
                && qa.recall_readback_active
                && [
                    "03-practice.png",
                    "03-local-chat.png",
                    "04-target-recall.png",
                    "09-reentered-wildspark.png",
                ]
                .iter()
                .all(|name| qa.completed_captures.contains(*name))
                && files.iter().all(|f| qa.directory.join(f).is_file());
            let report = serde_json::json!({"pass":passed,"offline_quick_buy_confirmed":qa.quick_buy_confirmed,"portrait_tap_selected_exact_enemy":qa.portrait_selected,"recall_touch_authoritative_channel":qa.recall_confirmed,"local_chat_without_pending_ack":qa.chat_confirmed,"local_reaction_above_vitals":qa.reaction_confirmed,"standard_kit_cooldown_feedback":qa.kit_feedback,"reentered_wildspark":qa.reentered,"moved_metres":qa.moved,"target_damaged":qa.hit,"returned_home_and_restored_online":qa.restored,"screenshots":files,"expected_online_endpoint":qa.expected_server,"restored_online_endpoint":world.resource::<ClientSession>().server_addr(),"physical_ipad_verified":false,"synthetic_window_focus":synthetic_focus_enabled(),"scale_factor_override":1.0,"method":"Native renderer, production UI and local packet handlers, synthetic actions; one English 852x393 viewport."});
            let mut report = report;
            report["chat_readable"] = qa.chat_readable.into();
            report["chat_layout"] = qa.chat_layout;
            report["recall_visual_submitted"] = qa.recall_visual_submitted.into();
            report["recall_readback_active"] = qa.recall_readback_active.into();
            report["recall_diagnostic"] = qa.recall_diagnostic;
            std::fs::write(
                qa.directory.join("result.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            world.write_message(if passed {
                AppExit::Success
            } else {
                AppExit::error()
            });
            return;
        }
        _ => {}
    }
    if advance {
        info!("Offline QA completed stage {}", qa.stage);
        qa.stage += 1;
        qa.since = Instant::now();
    }
    world.insert_resource(qa);
}
