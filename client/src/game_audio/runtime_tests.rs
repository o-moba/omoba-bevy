//! Headless tests exercise scheduling/ownership without pretending to open an
//! audio device. Native QA separately verifies Bevy's real decoder and sinks.
use super::*;
use std::time::Duration;

fn setup(loaded: bool) -> App {
    let mut app = App::new();
    app.init_resource::<Time<Real>>()
        .init_resource::<GameAudioRuntime>()
        .init_resource::<GameAudioDiagnostics>()
        .init_resource::<Assets<AudioSource>>()
        .init_resource::<GameStateSnapshot>()
        .init_resource::<ClientSession>()
        .init_resource::<AudioSettings>()
        .add_message::<AudioCueRequest>()
        .add_systems(Update, update_audio);
    app.world_mut().spawn((
        Window {
            focused: true,
            ..default()
        },
        PrimaryWindow,
    ));
    let handle = Handle::<AudioSource>::default();
    if loaded {
        app.world_mut()
            .resource_mut::<Assets<AudioSource>>()
            .insert(
                handle.id(),
                AudioSource {
                    bytes: Vec::new().into(),
                },
            )
            .unwrap();
    }
    let mut runtime = app.world_mut().resource_mut::<GameAudioRuntime>();
    runtime.cues.insert(
        AudioCue::UiConfirm,
        LoadedCue {
            source: handle,
            gain: 0.5,
        },
    );
    app
}

fn request(app: &mut App) {
    app.world_mut()
        .write_message(AudioCueRequest(AudioCue::UiConfirm));
}

fn advance(app: &mut App, seconds: f64) {
    app.world_mut()
        .resource_mut::<Time<Real>>()
        .advance_by(Duration::from_secs_f64(seconds));
    app.update();
}

fn voices(app: &mut App) -> usize {
    app.world_mut()
        .query::<&AudioVoice>()
        .iter(app.world())
        .count()
}

#[test]
fn a_late_loaded_asset_does_not_play_an_old_request() {
    let mut app = setup(false);
    request(&mut app);
    advance(&mut app, 0.01);
    assert_eq!(voices(&mut app), 0);
    assert_eq!(
        app.world()
            .resource::<GameAudioDiagnostics>()
            .dropped_missing,
        1
    );
    app.world_mut()
        .resource_mut::<Assets<AudioSource>>()
        .insert(
            Handle::<AudioSource>::default().id(),
            AudioSource {
                bytes: Vec::new().into(),
            },
        )
        .unwrap();
    advance(&mut app, 0.01);
    assert_eq!(voices(&mut app), 0);
    request(&mut app);
    advance(&mut app, 0.1);
    assert_eq!(voices(&mut app), 1);
}

#[test]
fn no_audio_device_does_not_accumulate_pending_effect_entities() {
    let mut app = setup(true);
    for _ in 0..80 {
        request(&mut app);
        advance(&mut app, 0.1);
        assert!(voices(&mut app) <= 4);
    }
    advance(&mut app, 0.31);
    assert_eq!(voices(&mut app), 0);
    let diagnostics = app.world().resource::<GameAudioDiagnostics>();
    assert!(diagnostics.played > 0);
    assert_eq!(diagnostics.observed_effect_sinks, 0);
}

#[test]
fn mute_and_lost_focus_discard_effects_and_do_not_resume_backlog() {
    let mut app = setup(true);
    request(&mut app);
    advance(&mut app, 0.1);
    assert_eq!(voices(&mut app), 1);
    app.world_mut().resource_mut::<AudioSettings>().muted = true;
    request(&mut app);
    advance(&mut app, 0.1);
    assert_eq!(voices(&mut app), 0);
    app.world_mut().resource_mut::<AudioSettings>().muted = false;
    advance(&mut app, 0.1);
    assert_eq!(voices(&mut app), 0);
    request(&mut app);
    advance(&mut app, 0.1);
    assert_eq!(voices(&mut app), 1);
    for mut window in app
        .world_mut()
        .query::<&mut Window>()
        .iter_mut(app.world_mut())
    {
        window.focused = false;
    }
    request(&mut app);
    advance(&mut app, 0.1);
    assert_eq!(voices(&mut app), 0);
}

#[test]
fn zero_ui_bus_discards_requests_and_live_volume_updates_pending_playback() {
    let mut app = setup(true);
    app.world_mut().resource_mut::<AudioSettings>().ui = 0.0;
    request(&mut app);
    advance(&mut app, 0.1);
    assert_eq!(voices(&mut app), 0);
    app.world_mut().resource_mut::<AudioSettings>().ui = 0.5;
    request(&mut app);
    advance(&mut app, 0.1);
    let mut playback = app.world_mut().query::<&PlaybackSettings>();
    assert!((playback.single(app.world()).unwrap().volume.to_linear() - 0.2).abs() < 0.001);
    app.world_mut().resource_mut::<AudioSettings>().master = 0.4;
    advance(&mut app, 0.05);
    assert!((playback.single(app.world()).unwrap().volume.to_linear() - 0.1).abs() < 0.001);
}

#[test]
fn music_has_one_pending_loop_without_device_and_updates_pause_and_gain_in_place() {
    let mut app = setup(true);
    app.world_mut().resource_mut::<GameAudioRuntime>().music = Some(LoadedCue {
        source: Handle::<AudioSource>::default(),
        gain: 0.8,
    });
    for _ in 0..120 {
        advance(&mut app, 0.1);
        assert_eq!(voices(&mut app), 1);
    }
    let mut playback = app.world_mut().query::<(Entity, &PlaybackSettings)>();
    let (music_entity, initial) = playback.single(app.world()).unwrap();
    assert!(!initial.paused);
    // Lobby gain is intentionally restrained; this is a pending component,
    // not a claim that the headless machine has an audible output sink.
    assert!((initial.volume.to_linear() - 0.8 * 0.25 * 0.8 * 0.45).abs() < 0.001);
    app.world_mut().resource_mut::<AudioSettings>().muted = true;
    advance(&mut app, 0.1);
    let (entity, muted) = playback.single(app.world()).unwrap();
    assert_eq!(entity, music_entity);
    assert!(muted.paused);
    assert_eq!(muted.volume.to_linear(), 0.0);
    app.world_mut().resource_mut::<AudioSettings>().muted = false;
    advance(&mut app, 0.1);
    let (entity, resumed) = playback.single(app.world()).unwrap();
    assert_eq!(entity, music_entity);
    assert!(!resumed.paused);
    assert!(resumed.volume.to_linear() > 0.0);
    assert!(resumed.volume.to_linear() < 0.8 * 0.25 * 0.8 * 0.45);
    assert!(!app.world().resource::<GameAudioDiagnostics>().music_sink);
}
