//! Opt-in native audio/controls acceptance. Uses a real local Practice server.
//! Input and UI scrolling are scripted; no combat or server snapshots are fabricated.
use std::{path::PathBuf, time::Instant};

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

use crate::{
    audio_settings::AudioSettings,
    game_audio::{AudioCue, AudioCueRequest, GameAudioDiagnostics},
    mobile_controls::MobileControls,
    net::{ClientSession, GameStateSnapshot},
    pause_menu::PauseMenuState,
};

pub(crate) struct AudioQaPlugin;
impl Plugin for AudioQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os("OMOBA_AUDIO_QA_OUTPUT").filter(|s| !s.is_empty()) else {
            return;
        };
        app.insert_resource(WinitSettings::continuous())
            .insert_resource(Qa {
                output: path.into(),
                started: Instant::now(),
                ..default()
            })
            .add_systems(
                PreUpdate,
                (window_setup, drive.after(bevy::ui::UiSystems::Focus)),
            )
            .add_systems(PostUpdate, capture.after(bevy::ui::UiSystems::PostLayout));
    }
}

#[derive(Resource)]
struct Qa {
    output: PathBuf,
    started: Instant,
    stage: u8,
    action_at: f64,
    pending_touch: Option<Vec2>,
    native_focused: bool,
    focus_event: bool,
    focus_attempts: u8,
    focus_at: f64,
    capture_pending: Option<usize>,
    captured: Vec<usize>,
    records: Vec<serde_json::Value>,
    initial_music_volume: f32,
    resume_position: f32,
    sample_sent: bool,
    done: bool,
}
impl Default for Qa {
    fn default() -> Self {
        Self {
            output: PathBuf::new(),
            started: Instant::now(),
            stage: 0,
            action_at: -2.0,
            pending_touch: None,
            native_focused: false,
            focus_event: false,
            focus_attempts: 0,
            focus_at: -2.0,
            capture_pending: None,
            captured: Vec::new(),
            records: Vec::new(),
            initial_music_volume: 0.0,
            resume_position: 0.0,
            sample_sent: false,
            done: false,
        }
    }
}
#[derive(Component)]
struct AudioShot(usize);

#[derive(bevy::ecs::system::SystemParam)]
struct Input<'w, 's> {
    window: Query<'w, 's, (Entity, &'static Window), With<PrimaryWindow>>,
    buttons: Query<
        'w,
        's,
        (
            Entity,
            &'static Name,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static InheritedVisibility,
            Option<&'static bevy::ui::CalculatedClip>,
        ),
        With<Button>,
    >,
    panels: Query<
        'w,
        's,
        (
            &'static Name,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static mut ScrollPosition,
        ),
    >,
    keys: MessageWriter<'w, KeyboardInput>,
    touches: MessageWriter<'w, TouchInput>,
    presses: MessageWriter<'w, crate::ui::SyntheticPress>,
}
fn touch(window: Entity, position: Vec2, phase: TouchPhase) -> TouchInput {
    TouchInput {
        window,
        position,
        phase,
        id: 909144,
        force: None,
    }
}

fn drive(
    mut qa: ResMut<Qa>,
    mut input: Input,
    session: Res<ClientSession>,
    snapshot: Res<GameStateSnapshot>,
    mobile: Res<MobileControls>,
    menu: Res<PauseMenuState>,
    settings: Res<AudioSettings>,
    diagnostics: Res<GameAudioDiagnostics>,
    mut cues: MessageWriter<AudioCueRequest>,
    mut help: ResMut<crate::help_overlay::HelpOverlayVisible>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.done {
        return;
    }
    let elapsed = qa.started.elapsed().as_secs_f64();
    if elapsed > 75.0 {
        fail(
            &mut qa,
            &diagnostics,
            "native audio/settings acceptance timed out",
            &mut exit,
        );
        return;
    }
    help.0 = false;
    if !session.join_confirmed() || snapshot.match_mode != "practice" {
        return;
    }
    let Ok((window_id, window)) = input.window.single() else {
        return;
    };
    if !qa.native_focused || !qa.focus_event || !window.focused {
        if let Some(position) = qa.pending_touch.take() {
            input
                .touches
                .write(touch(window_id, position, TouchPhase::Canceled));
        }
        return;
    }
    if let Some(position) = qa.pending_touch.take() {
        input
            .touches
            .write(touch(window_id, position, TouchPhase::Ended));
        qa.action_at = elapsed;
        return;
    }
    if elapsed - qa.action_at < 0.25 {
        return;
    }
    if qa.stage == 0 {
        if !menu.open {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                input.keys.write(KeyboardInput {
                    key_code: KeyCode::Escape,
                    logical_key: Key::Escape,
                    state,
                    text: None,
                    repeat: false,
                    window: window_id,
                });
            }
            qa.action_at = elapsed;
            return;
        }
        if click(
            "SettingsButton",
            window_id,
            mobile.enabled,
            &mut input,
            &mut qa,
        ) {
            qa.stage = 1;
        }
    } else if qa.stage == 1 {
        if diagnostics.assets_ready != diagnostics.assets_total
            || diagnostics.assets_total != 17
            || !diagnostics.music_sink
            || diagnostics.music_position_secs < 0.3
            || diagnostics.music_volume <= 0.0
        {
            return;
        }
        qa.initial_music_volume = diagnostics.music_volume;
        if qa.records.is_empty() {
            qa.capture_pending = Some(0);
            return;
        }
        if click(
            "PauseMenuAudioMusicControls-Down",
            window_id,
            mobile.enabled,
            &mut input,
            &mut qa,
        ) {
            qa.stage = 2;
        }
    } else if qa.stage == 2 {
        if (settings.music - 0.2).abs() > 0.001
            || diagnostics.music_volume >= qa.initial_music_volume * 0.94
        {
            return;
        }
        if click(
            "PauseMenuAudioMuteButton",
            window_id,
            mobile.enabled,
            &mut input,
            &mut qa,
        ) {
            qa.stage = 3;
        }
    } else if qa.stage == 3 {
        if !settings.muted || !diagnostics.muted || !diagnostics.music_paused {
            return;
        }
        if qa.records.len() < 2 {
            qa.capture_pending = Some(1);
            return;
        }
        if click(
            "PauseMenuAudioMuteButton",
            window_id,
            mobile.enabled,
            &mut input,
            &mut qa,
        ) {
            qa.resume_position = diagnostics.music_position_secs;
            qa.stage = 4;
        }
    } else if qa.stage == 4 {
        if settings.muted || diagnostics.music_paused || diagnostics.music_volume <= 0.0 {
            return;
        }
        if !qa.sample_sent {
            // Explicit palette-preview request follows the real audio mixer gates.
            // It is not presented as an authoritative combat event.
            cues.write(AudioCueRequest(AudioCue::UiConfirm));
            qa.sample_sent = true;
            qa.action_at = elapsed;
            return;
        }
        if diagnostics.observed_effect_sinks == 0
            || diagnostics
                .cue_plays
                .get("ui_confirm")
                .copied()
                .unwrap_or(0)
                == 0
            || diagnostics.music_position_secs < qa.resume_position + 0.25
            || qa.captured.len() != 2
        {
            return;
        }
        let receipt = serde_json::json!({"status":"passed","version":env!("CARGO_PKG_VERSION"),
            "scenario":"native_audio","real_practice_server":true,"scripted_input":true,
            "scripted_ui_scroll":true,"scripted_palette_cue":true,"synthetic_combat_events":false,
            "physical_phone_verified":false,"manual_listening_verified":false,
            "initial_music_volume":qa.initial_music_volume,"final_settings":*settings,
            "diagnostics":*diagnostics,"captures":qa.records,
            "native_focus_confirmed":qa.native_focused && qa.focus_event,
            "match_id":snapshot.meta.match_id,"server_epoch":snapshot.meta.server_epoch});
        if std::fs::write(
            qa.output.join("qa-summary.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .is_ok()
        {
            qa.done = true;
            exit.write(AppExit::Success);
        }
    }
}

fn click(name: &str, window: Entity, mobile: bool, input: &mut Input, qa: &mut Qa) -> bool {
    let Some((entity, _, node, transform, visible, clip)) =
        input.buttons.iter().find(|(_, n, ..)| n.as_str() == name)
    else {
        return false;
    };
    if !visible.get() || node.size().min_element() <= 0.0 {
        return false;
    }
    let rect = Rect::from_center_size(
        transform.translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    );
    let visible_rect = clip.map(|c| {
        Rect::from_corners(
            c.clip.min * node.inverse_scale_factor(),
            c.clip.max * node.inverse_scale_factor(),
        )
    });
    if visible_rect.is_some_and(|clip| !clip.contains(rect.center()) || rect.max.y > clip.max.y) {
        for (n, panel, transform, mut scroll) in &mut input.panels {
            if n.as_str() == "PauseMenuPanel" {
                let panel_rect = Rect::from_center_size(
                    transform.translation * panel.inverse_scale_factor(),
                    panel.size() * panel.inverse_scale_factor(),
                );
                let maximum = ((panel.content_size().y - panel.size().y)
                    * panel.inverse_scale_factor())
                .max(0.0);
                scroll.y = (scroll.y + rect.max.y - panel_rect.max.y + 14.0).clamp(0.0, maximum);
            }
        }
        qa.action_at = qa.started.elapsed().as_secs_f64();
        return false;
    }
    if mobile {
        input
            .touches
            .write(touch(window, rect.center(), TouchPhase::Started));
        qa.pending_touch = Some(rect.center());
    } else {
        // A synthetic press activates the kit button on the next recognizer run.
        input.presses.write(crate::ui::SyntheticPress(entity));
    }
    qa.action_at = qa.started.elapsed().as_secs_f64();
    true
}

fn capture(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    settings: Res<AudioSettings>,
    diagnostics: Res<GameAudioDiagnostics>,
    nodes: Query<(
        &Name,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
    )>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let Some(index) = qa.capture_pending.take() else {
        return;
    };
    if qa.records.len() > index {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let rect = |node: &ComputedNode, transform: &UiGlobalTransform| {
        Rect::from_center_size(
            transform.translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    let Some(panel) = nodes.iter().find_map(|(name, node, transform, visible)| {
        (name.as_str() == "PauseMenuPanel" && visible.get()).then(|| rect(node, transform))
    }) else {
        return;
    };
    let controls: Vec<_> = nodes
        .iter()
        .filter(|(name, _, _, visible)| {
            visible.get()
                && name.as_str().starts_with("PauseMenuAudio")
                && (name.as_str().ends_with("-Down")
                    || name.as_str().ends_with("-Up")
                    || name.as_str() == "PauseMenuAudioMuteButton")
        })
        .collect();
    if controls.len() != 9
        || controls.iter().any(|(_, node, transform, _)| {
            let bounds = rect(node, transform);
            !panel.contains(bounds.min)
                || !panel.contains(bounds.max)
                || bounds.min.x < 0.0
                || bounds.min.y < 0.0
                || bounds.max.x > window.width()
                || bounds.max.y > window.height()
        })
    {
        return;
    }
    let file = if index == 0 {
        "01-audio-settings.png"
    } else {
        "02-muted-settings.png"
    };
    let measured: Vec<_> = nodes
        .iter()
        .filter(|(n, _, _, v)| n.as_str().starts_with("PauseMenuAudio") && v.get())
        .map(|(n, c, t, _)| {
            serde_json::json!({"name":n.as_str(),
            "center":(t.translation*c.inverse_scale_factor()).to_array(),
            "size":(c.size()*c.inverse_scale_factor()).to_array()})
        })
        .collect();
    qa.records.push(serde_json::json!({"file":file,"settings":*settings,"diagnostics":*diagnostics,"nodes":measured,"all_audio_controls_fit":true,
        "pixels":[window.resolution.physical_width(),window.resolution.physical_height()]}));
    if std::fs::create_dir_all(&qa.output).is_err() {
        return;
    }
    commands
        .spawn((Screenshot::primary_window(), AudioShot(index)))
        .observe(save_to_disk(qa.output.join(file)))
        .observe(readback);
}
fn readback(event: On<ScreenshotCaptured>, shots: Query<&AudioShot>, mut qa: ResMut<Qa>) {
    if let Ok(shot) = shots.get(event.entity)
        && event.image.width() > 0
        && event.image.height() > 0
        && !qa.captured.contains(&shot.0)
    {
        qa.captured.push(shot.0);
    }
}
fn fail(
    qa: &mut Qa,
    diagnostics: &GameAudioDiagnostics,
    reason: &str,
    exit: &mut MessageWriter<AppExit>,
) {
    let _ = std::fs::create_dir_all(&qa.output);
    let report = serde_json::json!({"status":"failed","reason":reason,"stage":qa.stage,
        "diagnostics":diagnostics,"native_focused":qa.native_focused,"focus_event":qa.focus_event});
    let _ = std::fs::write(
        qa.output.join("qa-failure.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    );
    qa.done = true;
    exit.write(AppExit::error());
}
fn window_setup(
    mut qa: ResMut<Qa>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut focus: MessageReader<WindowFocused>,
    _main_thread: NonSendMarker,
) {
    let Ok((id, mut window)) = windows.single_mut() else {
        return;
    };
    for event in focus.read().filter(|e| e.window == id) {
        qa.focus_event = event.focused;
    }
    qa.native_focused =
        WINIT_WINDOWS.with_borrow(|windows| windows.get_window(id).is_some_and(|w| w.has_focus()));
    let elapsed = qa.started.elapsed().as_secs_f64();
    if !qa.native_focused && qa.focus_attempts < 3 && elapsed - qa.focus_at > 2.0 {
        WINIT_WINDOWS.with_borrow(|windows| {
            if let Some(w) = windows.get_window(id) {
                w.focus_window();
            }
        });
        qa.focus_attempts += 1;
        qa.focus_at = elapsed;
    }
    let dimension = |key, default| {
        std::env::var(key)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(default)
            .clamp(320, 3840)
    };
    let (w, h) = (
        dimension("OMOBA_QA_WIDTH", 1280),
        dimension("OMOBA_QA_HEIGHT", 720),
    );
    if window.resolution.physical_width() != w
        || window.resolution.physical_height() != h
        || window.resolution.scale_factor() != 1.0
    {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(w, h);
    }
}
