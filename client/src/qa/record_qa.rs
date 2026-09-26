//! Opt-in frame recorder for demo videos. Disabled unless OMOBA_RECORD_DIR is
//! set. It reads the primary window back at OMOBA_RECORD_FPS (default 30) and
//! writes numbered PNGs plus `frames.jsonl` with each frame's wall-clock time,
//! so an encoder can rebuild real-time playback even when readbacks drop
//! frames. PNG encoding runs on the async compute pool, off the main thread.
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

/// Frames still being encoded before new readbacks are skipped.
const MAX_IN_FLIGHT: usize = 12;

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
        app.insert_resource(Recorder {
            directory,
            interval: Duration::from_secs_f64(1.0 / f64::from(fps)),
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

    /// Wall-clock seconds since recording started; demo directors stamp their
    /// caption events with the same clock.
    pub(crate) fn elapsed_seconds(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

fn record(mut commands: Commands, mut recorder: ResMut<Recorder>) {
    let now = recorder.started.elapsed();
    if recorder.stopped || now < recorder.next {
        return;
    }
    // Keep the grid anchored to the start; after a stall resume from now.
    recorder.next = (recorder.next + recorder.interval).max(now);
    if recorder.in_flight.load(Ordering::Relaxed) >= MAX_IN_FLIGHT {
        recorder.skipped += 1;
        return;
    }
    let index = recorder.index;
    recorder.index += 1;
    let seconds = now.as_secs_f64();
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
