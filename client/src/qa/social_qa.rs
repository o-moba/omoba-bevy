//! Opt-in native captures driven by real Join/chat/reaction commands and UDP echoes.
use crate::{
    camera::MainCamera,
    combat::CombatStats,
    mobile_controls::MobileControls,
    net::{ClientSession, GameStateSnapshot, NetworkCommand, NetworkPlayerId},
    player::Player,
    social::{SocialBubble, SocialClient},
    sprite::PlayerVisualMode,
    team::{Team, TeamSelection},
};
use bevy::{
    ecs::system::NonSendMarker,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        touch::{TouchInput, TouchPhase},
    },
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::{PrimaryWindow, WindowFocused},
    winit::{WINIT_WINDOWS, WinitSettings},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
const FILES: [&str; 4] = [
    "01-chat.png",
    "02-reaction-wheel.png",
    "03-confirmed-reaction.png",
    "04-skill-description.png",
];
const TOUCH_ID: u64 = 908143;
pub(crate) struct SocialQaPlugin;
impl Plugin for SocialQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_SOCIAL_QA_OUTPUT")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        // Keep this short-lived diagnostic run updating when macOS launches it
        // in the background. Production input still requires actual OS focus.
        app.insert_resource(WinitSettings::continuous())
            .insert_resource(Qa {
                directory,
                started: Instant::now(),
                step: 0,
                settled: 0,
                readbacks: Vec::new(),
                records: Vec::new(),
                in_flight: None,
                join_sent: false,
                chat_sent: false,
                gesture_started: false,
                gesture_released: false,
                skill_help: std::env::var("OMOBA_SOCIAL_QA_SKILL_HELP").as_deref() == Ok("1"),
                skill_started: None,
                skill_released: false,
                skill_release_frames: 0,
                skill_cast_commands: 0,
                focus_requested: false,
                focus_request_count: 0,
                last_focus_request: None,
                canceled_gestures: 0,
                focus_history: Vec::new(),
                focus_confirmed: false,
                native_focused: None,
                diagnostic_at: None,
                diagnostic: serde_json::Value::Null,
                done: false,
            })
            .add_systems(PreUpdate, resize_window)
            .add_systems(
                Update,
                drive.after(crate::input_context::InputContextSet::Actions),
            )
            .add_systems(
                PostUpdate,
                capture
                    .after(monitor_skill_casts)
                    .after(bevy::ui::UiSystems::PostLayout)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            )
            .add_systems(PostUpdate, monitor_skill_casts);
    }
}
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    started: Instant,
    step: usize,
    settled: u32,
    readbacks: Vec<usize>,
    records: Vec<serde_json::Value>,
    in_flight: Option<usize>,
    join_sent: bool,
    chat_sent: bool,
    gesture_started: bool,
    gesture_released: bool,
    skill_help: bool,
    skill_started: Option<Instant>,
    skill_released: bool,
    skill_release_frames: u32,
    skill_cast_commands: usize,
    focus_requested: bool,
    focus_request_count: u8,
    last_focus_request: Option<Instant>,
    canceled_gestures: u32,
    focus_history: Vec<serde_json::Value>,
    focus_confirmed: bool,
    native_focused: Option<bool>,
    diagnostic_at: Option<Instant>,
    diagnostic: serde_json::Value,
    done: bool,
}
#[derive(Component)]
struct Shot(usize);
#[derive(bevy::ecs::system::SystemParam)]
struct QaWorld<'w, 's> {
    session: Res<'w, ClientSession>,
    snapshot: Res<'w, GameStateSnapshot>,
    mobile: Res<'w, MobileControls>,
    mode: Res<'w, PlayerVisualMode>,
    local: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static CombatStats,
            &'static NetworkPlayerId,
        ),
        With<Player>,
    >,
    camera: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    window: Query<'w, 's, (Entity, &'static Window), With<PrimaryWindow>>,
}
fn drive(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    world: QaWorld,
    mut social: ResMut<SocialClient>,
    mut out: MessageWriter<NetworkCommand>,
    mut touches: MessageWriter<TouchInput>,
    mut keys: MessageWriter<KeyboardInput>,
    mut help: ResMut<crate::help_overlay::HelpOverlayVisible>,
    mut career: ResMut<crate::career::CareerClient>,
    mut selection: ResMut<TeamSelection>,
    picker: Query<Entity, With<crate::team::TeamSelectRoot>>,
) {
    if qa.done {
        return;
    }
    help.0 = false;
    career.modal = crate::career::CareerModal::Closed;
    if !qa.join_sent && world.session.is_connected() {
        out.write(prepare_join(&mut selection, *world.mode));
        for entity in &picker {
            commands.entity(entity).despawn();
        }
        qa.join_sent = true;
    }
    if !world.session.join_confirmed() || world.snapshot.match_mode != "practice" {
        return;
    }
    let Ok((position, stats, _)) = world.local.single() else {
        return;
    };
    if !stats.is_alive() {
        return;
    }
    let Ok((window_id, window)) = world.window.single() else {
        return;
    };
    if !focus_ready(window.focused, qa.focus_confirmed, qa.native_focused) {
        if canceled_gesture_may_retry(qa.step, qa.gesture_started, qa.gesture_released) {
            qa.gesture_started = false;
            qa.canceled_gestures += 1;
            let entry = serde_json::json!({"kind":"gesture_canceled_before_send","elapsed_secs":qa.started.elapsed().as_secs_f64(),"step":qa.step});
            qa.focus_history.push(entry);
            if world.mobile.enabled {
                touches.write(TouchInput {
                    phase: TouchPhase::Canceled,
                    position: Vec2::ZERO,
                    window: window_id,
                    force: None,
                    id: TOUCH_ID,
                });
            } else {
                keys.write(key_event(
                    window_id,
                    KeyCode::KeyT,
                    "t",
                    ButtonState::Released,
                ));
            }
        }
        return;
    }
    if qa.step == 0 && !qa.chat_sent {
        social.qa_send_chat(&mut out);
        qa.chat_sent = true;
    }
    if qa.step == 1 && !qa.gesture_started {
        let Some(point) = world.camera.single().ok().and_then(|(camera, transform)| {
            crate::social::hero_screen(camera, transform, *world.mode, position.translation, 1.1)
        }) else {
            return;
        };
        if world.mobile.enabled {
            touches.write(TouchInput {
                phase: TouchPhase::Started,
                position: point,
                window: window_id,
                force: None,
                id: TOUCH_ID,
            });
        } else {
            keys.write(key_event(
                window_id,
                KeyCode::KeyT,
                "t",
                ButtonState::Pressed,
            ));
        }
        qa.gesture_started = true;
    }
    if qa.step == 2 && !qa.gesture_released {
        let Some(center) = social.wheel_center() else {
            return;
        };
        if world.mobile.enabled {
            let point = center + crate::social::choice_offset(0, world.mobile.scale());
            touches.write(TouchInput {
                phase: TouchPhase::Moved,
                position: point,
                window: window_id,
                force: None,
                id: TOUCH_ID,
            });
            touches.write(TouchInput {
                phase: TouchPhase::Ended,
                position: point,
                window: window_id,
                force: None,
                id: TOUCH_ID,
            });
        } else {
            keys.write(key_event(
                window_id,
                KeyCode::KeyT,
                "t",
                ButtonState::Released,
            ));
            keys.write(key_event(
                window_id,
                KeyCode::Digit1,
                "1",
                ButtonState::Pressed,
            ));
        }
        qa.gesture_released = true;
    }
    if qa.skill_help && world.mobile.enabled {
        let point = world.mobile.layout().ability_centers[0];
        if qa.step == 3 && qa.skill_started.is_none() {
            touches.write(TouchInput {
                phase: TouchPhase::Started,
                position: point,
                window: window_id,
                force: None,
                id: TOUCH_ID + 1,
            });
            qa.skill_started = Some(Instant::now());
        } else if qa.step == 4 && !qa.skill_released {
            touches.write(TouchInput {
                phase: TouchPhase::Ended,
                position: point,
                window: window_id,
                force: None,
                id: TOUCH_ID + 1,
            });
            qa.skill_released = true;
        }
    }
}
fn monitor_skill_casts(mut qa: ResMut<Qa>, mut commands: MessageReader<NetworkCommand>) {
    let casts = commands
        .read()
        .filter(|command| matches!(command, NetworkCommand::Cast { .. }))
        .count();
    if qa.skill_started.is_some() && !qa.done {
        qa.skill_cast_commands += casts;
        if qa.skill_released {
            qa.skill_release_frames += 1;
        }
    }
}
fn finish(qa: &mut Qa, exit: &mut MessageWriter<AppExit>) {
    let result = serde_json::json!({
        "status":"passed", "version":env!("CARGO_PKG_VERSION"), "scenario":"native_social",
        "fixture":false, "scripted_join_and_input":true, "manual_input_verified":false,
        "physical_phone_verified":false, "captures":qa.records,
        "skill_help_requested":qa.skill_help,
        "skill_hold_release_without_cast":qa.skill_help && qa.skill_released && qa.skill_release_frames >= 3 && qa.skill_cast_commands == 0,
        "skill_cast_commands":qa.skill_cast_commands,
    });
    if std::fs::write(
        qa.directory.join("qa-summary.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .is_err()
    {
        fail(qa, "cannot write summary", exit);
        return;
    }
    qa.done = true;
    exit.write(AppExit::Success);
}
fn fail(qa: &mut Qa, reason: &str, exit: &mut MessageWriter<AppExit>) {
    let _ = std::fs::create_dir_all(&qa.directory);
    let _ = std::fs::write(
        qa.directory.join("qa-failure.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"status":"failed","step":qa.step,"reason":reason,"diagnostic":qa.diagnostic}),
        )
        .unwrap(),
    );
    qa.done = true;
    exit.write(AppExit::error());
}
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    world: QaWorld,
    mut social: ResMut<SocialClient>,
    nodes: Query<(
        &Name,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
    )>,
    bubbles: Query<&SocialBubble>,
    pending: Res<crate::combat::PendingCast>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.done {
        return;
    }
    qa.diagnostic = serde_json::json!({
        "elapsed_secs": qa.started.elapsed().as_secs_f64(), "step": qa.step,
        "window_focused": world.window.single().ok().map(|(_, w)| w.focused),
        "native_focused": qa.native_focused,
        "focus_requested": qa.focus_requested, "focus_event_confirmed": qa.focus_confirmed,
        "focus_request_count": qa.focus_request_count, "canceled_gestures": qa.canceled_gestures,
        "focus_history": qa.focus_history,
        "connected": world.session.is_connected(), "joined": world.session.join_confirmed(),
        "connection_state": format!("{:?}", world.session.state()),
        "match_mode": world.snapshot.match_mode,
        "game_state": format!("{:?}", world.snapshot.state),
        "server_epoch": world.snapshot.meta.server_epoch, "match_id": world.snapshot.meta.match_id,
        "local_query_count": world.local.iter().count(),
        "local_alive": world.local.single().ok().map(|(_, stats, _)| stats.is_alive()),
        "join_sent": qa.join_sent, "chat_sent": qa.chat_sent,
        "gesture_started": qa.gesture_started, "gesture_released": qa.gesture_released,
        "skill_help": qa.skill_help, "skill_started": qa.skill_started.is_some(),
        "skill_released": qa.skill_released, "skill_cast_commands": qa.skill_cast_commands,
        "social": social.qa_diagnostics(),
    });
    if qa
        .diagnostic_at
        .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
    {
        let _ = std::fs::create_dir_all(&qa.directory);
        let _ = std::fs::write(
            qa.directory.join("qa-diagnostics.json"),
            serde_json::to_vec_pretty(&qa.diagnostic).unwrap(),
        );
        qa.diagnostic_at = Some(Instant::now());
    }
    if qa.started.elapsed() > Duration::from_secs(120) {
        fail(
            &mut qa,
            "native social QA timed out waiting for actual server/UI state",
            &mut exit,
        );
        return;
    }
    if let Some(stage) = qa.in_flight {
        if qa.readbacks.contains(&stage)
            && qa
                .directory
                .join(FILES[stage])
                .metadata()
                .is_ok_and(|m| m.len() > 100)
        {
            qa.in_flight = None;
            qa.step += 1;
            qa.settled = 0;
            if qa.step == 1 {
                social.qa_close();
            }
            if qa.step == 3 && !qa.skill_help {
                finish(&mut qa, &mut exit);
            }
        }
        return;
    }
    let Ok((_, window)) = world.window.single() else {
        return;
    };
    if !focus_ready(window.focused, qa.focus_confirmed, qa.native_focused) {
        qa.settled = 0;
        return;
    }
    if qa.skill_help && !world.mobile.enabled {
        fail(
            &mut qa,
            "skill help capture requires the mobile UI profile",
            &mut exit,
        );
        return;
    }
    let description_visible = nodes.iter().any(|(name, node, _, visible)| {
        name.as_str() == "MobileSkillDescription"
            && visible.is_none_or(|v| v.get())
            && node.size().min_element() > 0.0
    });
    if qa.step == 4 {
        if qa.skill_released && qa.skill_release_frames >= 3 {
            if qa.skill_cast_commands > 0 || pending.is_pending() || description_visible {
                fail(
                    &mut qa,
                    "skill inspection release cast, queued a cast, or left its tooltip visible",
                    &mut exit,
                );
            } else {
                finish(&mut qa, &mut exit);
            }
        }
        return;
    }
    let viewport = Vec2::new(window.width(), window.height());
    let confirmed_chat=social.events.iter().any(|event|matches!(&event.kind,shared::social::SocialEventKind::Chat{text,..}if text=="QA: Привет 小明 — ready for practice!"));
    let reaction=social.reactions.iter().find(|(event,_)|matches!(&event.kind,shared::social::SocialEventKind::Reaction{reaction_id}if reaction_id=="thumbs_up"));
    let visible_reaction =
        reaction.is_some_and(|(event, _)| bubbles.iter().any(|bubble| bubble.event_id == event.id));
    let ready = match qa.step {
        0 => confirmed_chat && social.chat_open,
        1 => social.wheel_center().is_some(),
        2 => visible_reaction,
        3 => {
            description_visible
                && qa
                    .skill_started
                    .is_some_and(|started| started.elapsed() >= Duration::from_millis(500))
        }
        _ => false,
    };
    if !ready {
        qa.settled = 0;
        return;
    }
    qa.settled += 1;
    if qa.settled < if qa.step == 2 { 2 } else { 24 } {
        return;
    }
    let measured:Vec<_>=nodes.iter().filter(|(name,_,_,_)|matches!(name.as_str(),"MobileSkillDescription"|"MobileAbility-0"|"MobileAbility-1"|"MobileAbility-2"|"MobileAbility-3"|"SocialChatPanel"|"SocialChatLog"|"SocialSend"|"SocialClose"|"SocialWheelRoot"|"SocialWheelChoice0"|"SocialWheelChoice1"|"SocialWheelChoice2"|"SocialWheelChoice3"|"SocialReactionBubble"|"SocialEntry"|"SocialStatus"|"MinimapRoot"|"MatchHudColumn"|"SkillBarRoot"|"EquipmentHud"|"MatchObjectiveRoot")).map(|(name,node,transform,visible)|{
        let size=node.size()*node.inverse_scale_factor();let min=transform.translation*node.inverse_scale_factor()-size*0.5;
        serde_json::json!({"name":name.as_str(),"min":min.to_array(),"size":size.to_array(),"visible":visible.is_none_or(|v|v.get()),"fits":min.x>=-1.0&&min.y>=-1.0&&(min+size).x<=viewport.x+1.0&&(min+size).y<=viewport.y+1.0&&size.x>0.0&&size.y>0.0})
    }).collect();
    let required: Vec<&str> = match qa.step {
        0 => vec!["SocialChatPanel", "SocialSend", "SocialClose"],
        1 => vec![
            "SocialWheelChoice0",
            "SocialWheelChoice1",
            "SocialWheelChoice2",
            "SocialWheelChoice3",
        ],
        2 => vec!["SocialReactionBubble", "SocialEntry", "SocialStatus"],
        _ => vec![
            "MobileSkillDescription",
            "MobileAbility-0",
            "MobileAbility-1",
            "MobileAbility-2",
            "MobileAbility-3",
        ],
    };
    if !required.iter().all(|name| {
        measured
            .iter()
            .any(|node| node["name"] == *name && node["visible"] == true && node["fits"] == true)
    }) {
        qa.diagnostic["measured_nodes"] = serde_json::json!(measured);
        fail(
            &mut qa,
            "required social UI is absent or clipped",
            &mut exit,
        );
        return;
    }
    let visible_rects: Vec<_> = nodes
        .iter()
        .filter(|(_, node, _, visible)| {
            visible.is_none_or(|v| v.get()) && node.size().x > 0.0 && node.size().y > 0.0
        })
        .map(|(name, node, transform, _)| {
            (
                name.as_str().to_owned(),
                Rect::from_center_size(
                    transform.translation * node.inverse_scale_factor(),
                    node.size() * node.inverse_scale_factor(),
                ),
            )
        })
        .collect();
    let overlaps = desktop_hud_overlaps(&visible_rects);
    if !world.mobile.enabled && !overlaps.is_empty() {
        qa.diagnostic["measured_nodes"] = serde_json::json!(measured);
        qa.diagnostic["hud_overlaps"] = serde_json::json!(overlaps);
        fail(
            &mut qa,
            "social controls obscure the desktop gameplay HUD",
            &mut exit,
        );
        return;
    }
    let record = serde_json::json!({"file":FILES[qa.step],"ui_profile":if world.mobile.enabled{"Mobile"}else{"Desktop"},"pixels":[window.resolution.physical_width(),window.resolution.physical_height()],"epoch":world.snapshot.meta.server_epoch,"match_id":world.snapshot.meta.match_id,"match_mode":world.snapshot.match_mode,"confirmed_chat":confirmed_chat,"chat_events":social.events,"reaction_event":reaction.map(|(event,_)|event),"visible_reaction":visible_reaction,"nodes":measured,"desktop_hud_overlap_checked":!world.mobile.enabled,"desktop_hud_overlaps":if world.mobile.enabled {Vec::<(String,String)>::new()} else {overlaps},"diagnostic":qa.diagnostic});
    qa.records.push(record);
    if std::fs::create_dir_all(&qa.directory).is_err() {
        fail(&mut qa, "cannot create output", &mut exit);
        return;
    }
    let stage = qa.step;
    commands
        .spawn((Screenshot::primary_window(), Shot(stage)))
        .observe(save_to_disk(qa.directory.join(FILES[stage])))
        .observe(readback);
    qa.in_flight = Some(stage);
}
fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<Qa>) {
    if let Ok(shot) = shots.get(captured.entity)
        && captured.image.width() > 0
        && captured.image.height() > 0
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}

fn resize_window(
    mut qa: ResMut<Qa>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut focus_events: MessageReader<WindowFocused>,
    _main_thread: NonSendMarker,
) {
    let dimension = |name: &str, fallback: u32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(fallback)
            .clamp(320, 3840)
    };
    let width = dimension("OMOBA_QA_WIDTH", 1280);
    let height = dimension("OMOBA_QA_HEIGHT", 720);
    if let Ok((entity, mut window)) = windows.single_mut() {
        for event in focus_events.read().filter(|event| event.window == entity) {
            qa.focus_confirmed = event.focused;
            if qa.focus_history.len() < 32 {
                let entry = serde_json::json!({"kind":"native_focus_event","focused":event.focused,"elapsed_secs":qa.started.elapsed().as_secs_f64(),"step":qa.step});
                qa.focus_history.push(entry);
            }
        }
        qa.native_focused = WINIT_WINDOWS
            .with_borrow(|windows| windows.get_window(entity).map(|window| window.has_focus()));
        if focus_retry_due(
            qa.native_focused,
            qa.focus_request_count,
            qa.last_focus_request.map(|at| at.elapsed()),
        ) {
            // Ask the OS to activate this short-lived QA window, at most three
            // times. Do not write the observed Window.focused value ourselves.
            WINIT_WINDOWS.with_borrow(|windows| {
                if let Some(window) = windows.get_window(entity) {
                    window.focus_window();
                }
            });
            qa.focus_requested = true;
            qa.focus_request_count += 1;
            qa.last_focus_request = Some(Instant::now());
            let entry = serde_json::json!({"kind":"native_focus_request","attempt":qa.focus_request_count,"elapsed_secs":qa.started.elapsed().as_secs_f64(),"step":qa.step});
            qa.focus_history.push(entry);
        }
        if window.resolution.physical_width() != width
            || window.resolution.physical_height() != height
            || window.resolution.scale_factor() != 1.0
        {
            window.resolution.set_scale_factor_override(Some(1.0));
            window.resolution.set_physical_resolution(width, height);
        }
    }
}

fn focus_ready(
    window_focused: bool,
    focus_event_confirmed: bool,
    native_focused: Option<bool>,
) -> bool {
    window_focused && focus_event_confirmed && native_focused == Some(true)
}

fn focus_retry_due(native_focused: Option<bool>, count: u8, elapsed: Option<Duration>) -> bool {
    native_focused == Some(false)
        && count < 3
        && elapsed.is_none_or(|elapsed| elapsed >= Duration::from_secs(2))
}

fn canceled_gesture_may_retry(step: usize, started: bool, released: bool) -> bool {
    step == 1 && started && !released
}

fn desktop_hud_overlaps(rects: &[(String, Rect)]) -> Vec<(String, String)> {
    let mut overlaps = Vec::new();
    for (social_name, social_rect) in rects
        .iter()
        .filter(|(name, _)| matches!(name.as_str(), "SocialEntry" | "SocialStatus"))
    {
        for (hud_name, hud_rect) in rects.iter().filter(|(name, _)| {
            matches!(
                name.as_str(),
                "MinimapRoot"
                    | "MatchHudColumn"
                    | "SkillBarRoot"
                    | "EquipmentHud"
                    | "MatchObjectiveRoot"
            )
        }) {
            let intersection = social_rect.intersect(*hud_rect);
            if intersection.width() > 0.5 && intersection.height() > 0.5 {
                overlaps.push((social_name.clone(), hud_name.clone()));
            }
        }
    }
    overlaps
}

fn prepare_join(selection: &mut TeamSelection, mode: PlayerVisualMode) -> NetworkCommand {
    // Mirror the ordinary picker/autojoin commitment. The network snapshot
    // remains the only authority that creates the local Player entity.
    selection.team = Some(Team::Green);
    selection.hero_class = shared::HeroClass::Mage;
    selection.avatar = Some("agnes".into());
    NetworkCommand::Join {
        team: Team::Green,
        character: selection.character,
        hero_class: selection.hero_class,
        avatar: selection.avatar.clone(),
        sprite_character: (mode == PlayerVisualMode::Sprite2d)
            .then(|| selection.sprite_character.clone()),
    }
}

fn key_event(window: Entity, key_code: KeyCode, value: &str, state: ButtonState) -> KeyboardInput {
    KeyboardInput {
        window,
        key_code,
        logical_key: Key::Character(value.into()),
        text: None,
        state,
        repeat: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qa_focus_recovery_is_bounded_and_cannot_replay_a_sent_reaction() {
        assert!(focus_retry_due(Some(false), 0, None));
        assert!(!focus_retry_due(
            Some(false),
            1,
            Some(Duration::from_secs(1))
        ));
        assert!(focus_retry_due(
            Some(false),
            2,
            Some(Duration::from_secs(2))
        ));
        assert!(!focus_retry_due(
            Some(false),
            3,
            Some(Duration::from_secs(20))
        ));
        assert!(!focus_retry_due(Some(true), 0, None));
        assert!(!focus_retry_due(None, 0, None));
        assert!(canceled_gesture_may_retry(1, true, false));
        assert!(!canceled_gesture_may_retry(1, true, true));
        assert!(!canceled_gesture_may_retry(2, true, false));
        assert!(!canceled_gesture_may_retry(2, true, true));
    }
    #[test]
    fn native_hud_verifier_rejects_original_hp_and_qwer_occlusion() {
        let rect = |x, y, w, h| Rect::from_corners(Vec2::new(x, y), Vec2::new(x + w, y + h));
        let mut nodes = vec![
            ("MatchHudColumn".into(), rect(16.0, 554.0, 230.0, 150.0)),
            ("SkillBarRoot".into(), rect(264.0, 596.0, 422.0, 100.0)),
            ("MinimapRoot".into(), rect(16.0, 16.0, 252.0, 252.0)),
            ("SocialEntry".into(), rect(12.0, 664.0, 142.0, 44.0)),
            ("SocialStatus".into(), rect(448.0, 644.0, 60.0, 14.0)),
        ];
        assert_eq!(desktop_hud_overlaps(&nodes).len(), 2);
        nodes[3].1 = rect(16.0, 276.0, 142.0, 44.0);
        nodes[4].1 = rect(16.0, 328.0, 60.0, 14.0);
        assert!(desktop_hud_overlaps(&nodes).is_empty());
    }
    #[test]
    fn requested_focus_is_insufficient_without_os_event_and_native_readback() {
        assert!(!focus_ready(true, false, Some(false)));
        assert!(!focus_ready(true, false, Some(true)));
        assert!(!focus_ready(true, true, None));
        assert!(!focus_ready(false, true, Some(true)));
        assert!(focus_ready(true, true, Some(true)));
    }
    #[test]
    fn qa_join_commits_picker_before_authoritative_spawn_and_preserves_2d_choice() {
        let mut selection = TeamSelection::default();
        let command = prepare_join(&mut selection, PlayerVisualMode::Sprite2d);
        assert_eq!(selection.team, Some(Team::Green));
        assert_eq!(selection.hero_class, shared::HeroClass::Mage);
        assert!(matches!(command, NetworkCommand::Join {
            team: Team::Green, hero_class: shared::HeroClass::Mage,
            avatar: Some(ref avatar), sprite_character: Some(ref sprite), ..
        } if avatar == "agnes" && sprite == &selection.sprite_character));
    }
}
