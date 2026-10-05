//! Opt-in frame recorder for demo videos. Disabled unless OMOBA_RECORD_DIR is
//! set. It reads the primary window back at OMOBA_RECORD_FPS (default 30) and
//! writes numbered PNGs plus `frames.jsonl` with each frame's wall-clock time,
//! so an encoder can rebuild real-time playback even when readbacks drop
//! frames. PNG encoding runs on the async compute pool, off the main thread.
//!
//! `OMOBA_RECORD_STEP=1` switches to frame-stepped capture for footage that
//! real-time readbacks cannot keep up with (Full HD and above at 60 fps): game
//! time advances exactly one frame interval per rendered frame, every frame is
//! captured and none are skipped, so frame `n` is at `n / fps` seconds however
//! long it took to render. Use it with the in-process offline practice, which
//! is driven by the same clock. Frames are held to a steady wall-clock cadence
//! (`OMOBA_RECORD_PACE` frames per second, default 15) because remote heroes
//! are interpolated on wall-clock receipt times. The capture must not depend
//! on the window having focus (the game idles an unfocused window at one
//! update per second), so this mode runs on a timer, turns vsync off and keeps
//! the window above the others; `OMOBA_RECORD_WINDOW_AT=x,y` (pixels) can park
//! it with only a corner on screen while the capture runs.
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    tasks::AsyncComputeTaskPool,
};

/// Frames still being encoded before new readbacks are skipped (real time)
/// or the next step waits (frame-stepped).
const MAX_IN_FLIGHT: usize = 12;
/// Longest a step waits for the encoders; readbacks need frames to complete.
const STEP_BACKLOG_WAIT: Duration = Duration::from_secs(2);

pub(crate) struct RecordQaPlugin;

impl Plugin for RecordQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_RECORD_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        std::fs::create_dir_all(&directory).expect("OMOBA_RECORD_DIR must be writable");
        let fps = std::env::var("OMOBA_RECORD_FPS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(30)
            .clamp(1, 60);
        let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
        let pace = std::env::var("OMOBA_RECORD_STEP")
            .is_ok_and(|value| value == "1")
            .then(|| {
                let wall_fps = std::env::var("OMOBA_RECORD_PACE")
                    .ok()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(15)
                    .clamp(1, 60);
                Duration::from_secs_f64(1.0 / f64::from(wall_fps))
            });
        if pace.is_some() {
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(interval))
                .add_systems(Last, keep_presenting.before(record));
        }
        app.insert_resource(Recorder {
            directory,
            interval,
            fps,
            pace,
            last_step: None,
            started: Instant::now(),
            next: Duration::ZERO,
            index: 0,
            skipped: 0,
            stopped: false,
            in_flight: Arc::new(AtomicUsize::new(0)),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Last, record);
    }
}

#[derive(Resource)]
pub(crate) struct Recorder {
    directory: PathBuf,
    interval: Duration,
    fps: u32,
    /// Frame-stepped capture: the wall-clock time each step is held to.
    pace: Option<Duration>,
    last_step: Option<Instant>,
    started: Instant,
    next: Duration,
    index: u64,
    skipped: u64,
    stopped: bool,
    in_flight: Arc<AtomicUsize>,
}

impl Recorder {
    /// Stops new readbacks; directors then wait for [`Recorder::idle`] across
    /// frames before exiting (pending readbacks need frames to complete).
    pub(crate) fn stop(&mut self) {
        self.stopped = true;
    }

    pub(crate) fn idle(&self) -> bool {
        self.in_flight.load(Ordering::Relaxed) == 0
    }

    /// Seconds since recording started on the video clock: wall-clock time,
    /// or stepped game time in frame-stepped capture. Demo directors pace
    /// their acts and stamp their caption events with the same clock.
    pub(crate) fn elapsed_seconds(&self) -> f64 {
        if self.pace.is_some() {
            step_seconds(self.index, self.fps)
        } else {
            self.started.elapsed().as_secs_f64()
        }
    }
}

/// Video time of frame `index` in frame-stepped capture.
fn step_seconds(index: u64, fps: u32) -> f64 {
    index as f64 / f64::from(fps)
}

/// `x,y` in pixels, as in `OMOBA_RECORD_WINDOW_AT`.
fn parse_window_at(value: &str) -> Option<IVec2> {
    let (x, y) = value.split_once(',')?;
    Some(IVec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Frame-stepped capture does not wait for focus or the display: it replaces
/// the game's frame pacing (which idles an unfocused window) with a timer and
/// keeps the window uncovered so the system keeps taking its frames.
fn keep_presenting(
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mut pacing: ResMut<bevy::winit::WinitSettings>,
    mut parked: Local<bool>,
) {
    use bevy::window::{PresentMode, WindowLevel, WindowPosition};
    let timer = bevy::winit::UpdateMode::Reactive {
        wait: Duration::from_millis(1),
        react_to_device_events: false,
        react_to_user_events: false,
        react_to_window_events: false,
    };
    if pacing.focused_mode != timer || pacing.unfocused_mode != timer {
        pacing.focused_mode = timer;
        pacing.unfocused_mode = timer;
    }
    for mut window in &mut windows {
        if window.present_mode != PresentMode::AutoNoVsync {
            window.present_mode = PresentMode::AutoNoVsync;
        }
        if window.window_level != WindowLevel::AlwaysOnTop {
            window.window_level = WindowLevel::AlwaysOnTop;
        }
        if !*parked {
            *parked = true;
            if let Some(at) = std::env::var("OMOBA_RECORD_WINDOW_AT")
                .ok()
                .and_then(|value| parse_window_at(&value))
            {
                window.position = WindowPosition::At(at);
            }
        }
    }
}

fn record(mut commands: Commands, mut recorder: ResMut<Recorder>) {
    if recorder.stopped {
        return;
    }
    let seconds = if let Some(pace) = recorder.pace {
        // Every frame is one video frame: wait for the encoders instead of
        // skipping, then hold the wall-clock cadence.
        let waiting = Instant::now();
        while recorder.in_flight.load(Ordering::Relaxed) >= MAX_IN_FLIGHT
            && waiting.elapsed() < STEP_BACKLOG_WAIT
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Some(wait) = recorder
            .last_step
            .and_then(|last| (last + pace).checked_duration_since(Instant::now()))
        {
            std::thread::sleep(wait);
        }
        recorder.last_step = Some(Instant::now());
        step_seconds(recorder.index, recorder.fps)
    } else {
        let now = recorder.started.elapsed();
        if now < recorder.next {
            return;
        }
        // Keep the grid anchored to the start; after a stall resume from now.
        recorder.next = (recorder.next + recorder.interval).max(now);
        if recorder.in_flight.load(Ordering::Relaxed) >= MAX_IN_FLIGHT {
            recorder.skipped += 1;
            return;
        }
        now.as_secs_f64()
    };
    let index = recorder.index;
    recorder.index += 1;
    let path = recorder.directory.join(format!("frame-{index:06}.png"));
    let log = recorder.directory.join("frames.jsonl");
    let in_flight = recorder.in_flight.clone();
    in_flight.fetch_add(1, Ordering::Relaxed);
    let line = serde_json::json!({"index": index, "file": path.file_name().and_then(|n| n.to_str()),
        "seconds": seconds, "skipped_before": recorder.skipped});
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
    {
        let _ = writeln!(file, "{line}");
    }
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>| {
            let image = captured.image.clone();
            let path = path.clone();
            let in_flight = in_flight.clone();
            AsyncComputeTaskPool::get()
                .spawn(async move {
                    if let Ok(image) = image.try_into_dynamic()
                        && let Err(error) = image.to_rgb8().save(&path)
                    {
                        error!("RECORD_QA cannot save {}: {error}", path.display());
                    }
                    in_flight.fetch_sub(1, Ordering::Relaxed);
                })
                .detach();
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder(pace: Option<Duration>, index: u64) -> Recorder {
        Recorder {
            directory: PathBuf::new(),
            interval: Duration::from_secs_f64(1.0 / 60.0),
            fps: 60,
            pace,
            last_step: None,
            started: Instant::now(),
            next: Duration::ZERO,
            index,
            skipped: 0,
            stopped: false,
            in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[test]
    fn stepped_clock_counts_frames_not_wall_time() {
        let stepped = recorder(Some(Duration::from_millis(50)), 150);
        assert_eq!(stepped.elapsed_seconds(), 2.5);
        assert_eq!(step_seconds(0, 60), 0.0);
        // No drift from the nanosecond-rounded interval on long clips.
        assert_eq!(step_seconds(216_000, 60), 3600.0);
        // Real-time capture stays on the wall clock, whatever the frame count.
        assert!(recorder(None, 150).elapsed_seconds() < 1.0);
    }

    #[test]
    fn window_position_is_two_pixel_coordinates() {
        assert_eq!(parse_window_at("2860, 1800"), Some(IVec2::new(2860, 1800)));
        assert_eq!(parse_window_at("-40,0"), Some(IVec2::new(-40, 0)));
        assert_eq!(parse_window_at("2860"), None);
        assert_eq!(parse_window_at("a,b"), None);
    }
}
