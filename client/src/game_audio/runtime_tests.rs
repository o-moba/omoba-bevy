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

// Voices from the rows of the skill registry.

use crate::skill_presentation::{
    cast::CastKey,
    stage::{OwnerSeen, StageChange, TelegraphSeen, Transition},
};
use shared::{
    HeroClass,
    combat::{CombatEntity, CombatEntityKind, CombatEvent, ProjectileStyle},
    loadout::{EffectVisualKind, SkillEffectState, SkillId},
};

const ME: u64 = 7;

/// The headless app with the final rows, a joined local Warrior at the origin and a
/// running round whose first frame has been taken.
fn skill_setup(registry: SkillPresentation) -> App {
    let mut app = setup(true);
    app.insert_resource(registry)
        .insert_resource(ClientSession::admitted_for_test())
        .add_message::<SkillCastObserved>()
        .add_message::<StageEvent>()
        .add_message::<TelegraphSeen>();
    let mut runtime = app.world_mut().resource_mut::<GameAudioRuntime>();
    for cue in [
        AudioCue::Melee,
        AudioCue::Holy,
        AudioCue::Kill,
        AudioCue::LevelUp,
        AudioCue::RadiantCharge,
        AudioCue::RadiantBeam,
        AudioCue::RadiantCore,
        AudioCue::RadianceBurst,
    ] {
        runtime.cues.insert(
            cue,
            LoadedCue {
                source: Handle::default(),
                gain: 0.5,
            },
        );
    }
    let mut snapshot = app.world_mut().resource_mut::<GameStateSnapshot>();
    snapshot.state = GameState::Running;
    snapshot.meta = shared::protocol::SnapshotMeta::new(3, 1, 1);
    app.world_mut().spawn((
        NetworkPlayerId(ME),
        Transform::default(),
        CombatStats::default(),
        PlayerProgression::default(),
        Team::Green,
        Player,
        crate::net::NetworkHeroClass(HeroClass::Warrior),
        crate::net::PlayerCosmeticAction::default(),
    ));
    advance(&mut app, 0.1);
    app
}

/// Reports an accepted action of the Warrior `actor` on the slot of the row `id`.
fn report_cast(app: &mut App, actor: u64, id: &str, sequence: u64, at: Vec3) {
    let slot = (0..4)
        .find(|slot| {
            CastKey::of(HeroClass::Warrior, None, *slot)
                .and_then(CastKey::skill)
                .is_some_and(|key| key.id() == id)
        })
        .unwrap();
    app.world_mut().write_message(SkillCastObserved {
        actor_id: actor,
        key: CastKey::of(HeroClass::Warrior, None, slot).unwrap(),
        slot,
        sequence,
        recast: false,
        origin: at,
        position: at,
        yaw: None,
        forward: Vec3::NEG_Z,
        local: actor == ME,
    });
}

/// What the mixer was handed for each effect voice, oldest first: speed, start, length
/// and volume.
fn handed(app: &mut App) -> Vec<(f32, Option<Duration>, Option<Duration>, f32)> {
    let mut voices: Vec<_> = app
        .world_mut()
        .query::<(&AudioVoice, &PlaybackSettings)>()
        .iter(app.world())
        .filter(|(voice, _)| voice.cue.is_some())
        .map(|(voice, playback)| {
            (
                voice.born,
                (
                    playback.speed,
                    playback.start_position,
                    playback.duration,
                    playback.volume.to_linear(),
                ),
            )
        })
        .collect();
    voices.sort_by(|a, b| a.0.total_cmp(&b.0));
    voices.into_iter().map(|(_, voice)| voice).collect()
}

fn diagnostics(app: &App) -> &GameAudioDiagnostics {
    app.world().resource::<GameAudioDiagnostics>()
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn the_notes_of_a_local_cast_start_on_time_in_the_voice_of_the_row() {
    let ms = Duration::from_millis;
    let mut app = skill_setup(SkillPresentation::target());
    report_cast(&mut app, ME, "rampage", 4, Vec3::ZERO);
    advance(&mut app, 0.01);
    // The first note: `melee` 0.70 `body`, at the gain of the mixer and the sample.
    let detune = policy::detune(4);
    let level = 0.8 * 0.7 * 0.5;
    let first = handed(&mut app);
    assert_eq!(first.len(), 1);
    assert!(near(first[0].0, 0.70 * detune));
    assert_eq!((first[0].1, first[0].2), (Some(ms(60)), Some(ms(340))));
    assert!(near(first[0].3, level));
    assert_eq!(diagnostics(&app).pending_notes, 2);
    // The second note is due 90 ms after the first one and the third 180 ms after it.
    advance(&mut app, 0.08);
    assert_eq!(handed(&mut app).len(), 1);
    advance(&mut app, 0.01);
    let two = handed(&mut app);
    assert_eq!(two.len(), 2);
    assert!(near(two[1].0, 0.80 * detune));
    assert_eq!((two[1].1, two[1].2), (Some(ms(60)), Some(ms(340))));
    assert!(near(two[1].3, level * 0.8));
    assert_eq!(diagnostics(&app).pending_notes, 1);
    advance(&mut app, 0.08);
    assert_eq!(handed(&mut app).len(), 2);
    advance(&mut app, 0.01);
    let three = handed(&mut app);
    assert_eq!(three.len(), 3);
    assert!(near(three[2].0, 0.90 * detune));
    assert!(near(three[2].3, level * 0.9));
    let report = diagnostics(&app);
    assert_eq!(report.pending_notes, 0);
    assert_eq!((report.played, report.dropped_rate), (3, 0));
    assert_eq!(report.cue_plays.get("melee"), Some(&3));
    // The cast was taken once: nothing follows it.
    advance(&mut app, 0.25);
    assert_eq!(diagnostics(&app).played, 3);
}

#[test]
fn a_voice_whose_notes_do_not_fit_is_dropped_whole_and_never_tried_again() {
    let mut app = skill_setup(SkillPresentation::target());
    // Two other cues take two of the four tokens.
    for cue in [AudioCue::Kill, AudioCue::LevelUp] {
        app.world_mut().write_message(AudioCueRequest(cue));
    }
    advance(&mut app, 0.01);
    assert_eq!(handed(&mut app).len(), 2);
    report_cast(&mut app, ME, "rampage", 4, Vec3::ZERO);
    advance(&mut app, 0.01);
    assert_eq!(handed(&mut app).len(), 2);
    let report = diagnostics(&app);
    assert_eq!(
        (report.played, report.dropped_rate, report.pending_notes),
        (2, 1, 0)
    );
    // The tokens come back; the voice that was refused does not.
    advance(&mut app, 0.25);
    let report = diagnostics(&app);
    assert_eq!((report.played, report.pending_notes), (2, 0));
    assert!(!report.cue_plays.contains_key("melee"));
    // The next cast is heard whole.
    report_cast(&mut app, ME, "rampage", 5, Vec3::ZERO);
    advance(&mut app, 0.01);
    assert_eq!(diagnostics(&app).pending_notes, 2);
    advance(&mut app, 0.2);
    assert_eq!(diagnostics(&app).cue_plays.get("melee"), Some(&3));
}

#[test]
fn the_notes_that_are_due_count_against_what_a_frame_may_start() {
    let mut app = skill_setup(SkillPresentation::target());
    report_cast(&mut app, ME, "rampage", 4, Vec3::ZERO);
    advance(&mut app, 0.01);
    assert_eq!(diagnostics(&app).pending_notes, 2);
    // A long frame: both later notes are due in it, and the tokens are back. A second
    // Rampage would make five notes start in one frame, so it is dropped whole.
    report_cast(&mut app, ME, "rampage", 5, Vec3::ZERO);
    advance(&mut app, 0.5);
    let report = diagnostics(&app);
    assert_eq!(
        (report.played, report.dropped_rate, report.pending_notes),
        (3, 1, 0)
    );
    assert_eq!(handed(&mut app).len(), 2);
    // One note beside the two that are due fits.
    let mut app = skill_setup(SkillPresentation::target());
    report_cast(&mut app, ME, "rampage", 4, Vec3::ZERO);
    advance(&mut app, 0.01);
    report_cast(&mut app, 8, "rampage", 5, Vec3::ZERO);
    advance(&mut app, 0.5);
    let report = diagnostics(&app);
    assert_eq!((report.played, report.dropped_rate), (4, 0));
}

#[test]
fn a_new_round_and_a_closed_mixer_clear_the_notes_that_wait() {
    let mut app = skill_setup(SkillPresentation::target());
    report_cast(&mut app, ME, "rampage", 4, Vec3::ZERO);
    advance(&mut app, 0.01);
    assert_eq!(diagnostics(&app).pending_notes, 2);
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .meta
        .match_id += 1;
    advance(&mut app, 0.05);
    assert_eq!(diagnostics(&app).pending_notes, 0);
    assert_eq!(voices(&mut app), 0);
    advance(&mut app, 0.2);
    assert_eq!(diagnostics(&app).played, 1);

    report_cast(&mut app, ME, "rampage", 5, Vec3::ZERO);
    advance(&mut app, 0.01);
    assert_eq!(diagnostics(&app).pending_notes, 2);
    app.world_mut().resource_mut::<AudioSettings>().muted = true;
    advance(&mut app, 0.05);
    assert_eq!(diagnostics(&app).pending_notes, 0);
    app.world_mut().resource_mut::<AudioSettings>().muted = false;
    advance(&mut app, 0.2);
    assert_eq!(diagnostics(&app).played, 2);
    // A cast observed while the mixer is closed is not heard afterwards.
    app.world_mut().resource_mut::<AudioSettings>().muted = true;
    report_cast(&mut app, ME, "rampage", 6, Vec3::ZERO);
    advance(&mut app, 0.05);
    app.world_mut().resource_mut::<AudioSettings>().muted = false;
    advance(&mut app, 0.2);
    assert_eq!(diagnostics(&app).played, 2);
    #[cfg(feature = "qa")]
    {
        // The evidence still names every voice that was asked for.
        let asked: Vec<_> = diagnostics(&app)
            .row_voices
            .iter()
            .map(|voice| (voice.moment, voice.row, voice.id, voice.notes))
            .collect();
        assert_eq!(
            asked,
            [
                ("cast", "rampage", 4, 2),
                ("cast", "rampage", 5, 2),
                ("cast", "rampage", 6, 2)
            ]
        );
        let voice = &diagnostics(&app).row_voices[0];
        assert_eq!(
            (voice.base, voice.slice, voice.actor),
            ("melee", "body", ME)
        );
        assert!(near(voice.speed, 0.70) && near(voice.gain, 1.0));
    }
}

#[test]
fn another_hero_is_heard_with_one_note_by_its_distance() {
    let mut app = skill_setup(SkillPresentation::target());
    report_cast(&mut app, 8, "rampage", 4, Vec3::new(21.0, 0.0, 0.0));
    advance(&mut app, 0.01);
    let heard = handed(&mut app);
    assert_eq!(heard.len(), 1);
    assert!(near(heard[0].0, 0.70 * policy::detune(4)));
    assert!(near(heard[0].3, 0.8 * 0.7 * 0.5 * 0.25));
    assert_eq!(diagnostics(&app).pending_notes, 0);
    advance(&mut app, 0.25);
    assert_eq!(diagnostics(&app).played, 1);
    // Beyond earshot there is no voice at all.
    report_cast(&mut app, 8, "rampage", 5, Vec3::new(36.0, 0.0, 0.0));
    advance(&mut app, 0.01);
    assert_eq!(diagnostics(&app).played, 1);
}

/// A receipt of damage the local hero dealt to a minion with the action on `slot`.
fn receipt(id: u64, slot: u8) -> CombatEvent {
    CombatEvent {
        id,
        source: CombatEntity {
            kind: CombatEntityKind::Player,
            id: ME,
        },
        target: CombatEntity {
            kind: CombatEntityKind::Minion,
            id: 40,
        },
        amount: 10.0,
        style: ProjectileStyle::Crescent,
        action_slot: Some(slot),
        ..Default::default()
    }
}

/// A Horizon Ray of the local hero as `kind`, lying along +X from the origin.
fn ray(kind: EffectVisualKind, remaining_secs: f32) -> (SkillEffectState, Option<OwnerSeen>) {
    (
        SkillEffectState {
            id: 31,
            owner_id: ME,
            owner_team: shared::map::Team::Green,
            skill: SkillId::DawnRay,
            kind,
            position: [0.0, 0.0],
            end: [6.0, 0.0],
            radius: 0.6,
            remaining_secs,
            armed: false,
            consumed_segments: 0,
        },
        Some(OwnerSeen {
            visible: true,
            alive: true,
            parrying: None,
            position: Vec3::ZERO,
            slot: Some(3),
            local: true,
        }),
    )
}

/// The ray of the local hero firing.
fn fired_ray() -> StageEvent {
    let (effect, owner) = ray(EffectVisualKind::Beam, 0.15);
    StageEvent {
        effect,
        change: StageChange::Transition(Transition::KindFlipped),
        owner,
    }
}

#[test]
fn receipts_and_releases_are_voiced_by_their_rows_and_the_packaged_rows_add_nothing() {
    let ms = Duration::from_millis;
    let beam = fired_ray();

    let mut app = skill_setup(SkillPresentation::target());
    // Shield Bash is on the first slot of the Warrior: its hit is `melee` 0.75 `tail`.
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .combat_events
        .push(receipt(1, 0));
    advance(&mut app, 0.01);
    let hit = handed(&mut app);
    assert_eq!(hit.len(), 1);
    assert!(near(hit[0].0, 0.75 * policy::detune(1)));
    assert_eq!((hit[0].1, hit[0].2), (Some(ms(200)), None));
    assert!(near(hit[0].3, 0.8 * 0.7 * 0.5));
    // The beam of Horizon Ray: the whole `radiant_beam`, one note that holds the chord.
    advance(&mut app, 0.4);
    assert_eq!(handed(&mut app).len(), 0);
    app.world_mut().write_message(beam.clone());
    advance(&mut app, 0.01);
    let fired = handed(&mut app);
    assert_eq!(fired.len(), 1);
    assert!(near(fired[0].0, policy::detune(31)));
    assert_eq!((fired[0].1, fired[0].2), (None, None));
    assert!(near(fired[0].3, 0.8 * 0.7 * 0.5));
    assert_eq!(diagnostics(&app).pending_notes, 0);
    assert_eq!(diagnostics(&app).cue_plays.get("radiant_beam"), Some(&1));

    // A registry without the rows names no voice: the receipt keeps the cue of its wire
    // style, played as recorded, and neither the cast nor the beam is heard.
    let mut app = skill_setup(SkillPresentation::default());
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .combat_events
        .push(receipt(1, 0));
    report_cast(&mut app, ME, "rampage", 4, Vec3::ZERO);
    app.world_mut().write_message(beam);
    advance(&mut app, 0.01);
    let styled = handed(&mut app);
    assert_eq!(styled.len(), 1);
    assert_eq!(styled[0].0, 1.0);
    assert_eq!((styled[0].1, styled[0].2), (None, None));
    assert!(near(styled[0].3, 0.8 * 0.7 * 0.5));
    assert_eq!(diagnostics(&app).cue_plays.get("melee"), Some(&1));
    assert_eq!(diagnostics(&app).pending_notes, 0);
    #[cfg(feature = "qa")]
    assert!(diagnostics(&app).row_voices.is_empty());
}

#[test]
fn the_ray_is_cast_with_the_ignition_warned_with_the_swell_and_fired_into_it() {
    let ms = Duration::from_millis;
    let mut app = skill_setup(SkillPresentation::target());
    // The local hero casts the ray; its warning reaches the client in the same snapshot.
    let (warning, owner) = ray(EffectVisualKind::BeamWarning, 0.9);
    let key = CastKey::of(HeroClass::Dawnweaver, None, 3).unwrap();
    assert_eq!(key.skill().map(|skill| skill.id()), Some("dawn_ray"));
    app.world_mut().write_message(SkillCastObserved {
        actor_id: ME,
        key,
        slot: 3,
        sequence: 6,
        recast: false,
        origin: Vec3::ZERO,
        position: Vec3::ZERO,
        yaw: None,
        forward: Vec3::NEG_Z,
        local: true,
    });
    app.world_mut().write_message(TelegraphSeen {
        effect: warning,
        owner,
    });
    advance(&mut app, 0.01);
    // The cast is the first 0.12 s of the charge, the warning the rest of it from 0.20 s:
    // the two never sound the same part of the sample.
    let level = 0.8 * 0.7 * 0.5;
    let mut charge = handed(&mut app);
    charge.sort_by_key(|voice| voice.1);
    assert_eq!(charge.len(), 2);
    assert!(near(charge[0].0, policy::detune(6)));
    assert_eq!((charge[0].1, charge[0].2), (None, Some(ms(120))));
    assert!(near(charge[0].3, level * 0.8));
    assert!(near(charge[1].0, policy::detune(31)));
    assert_eq!((charge[1].1, charge[1].2), (Some(ms(200)), None));
    assert!(near(charge[1].3, level * 0.9));
    assert_eq!(diagnostics(&app).cue_plays.get("radiant_charge"), Some(&2));
    // The windup later the ray fires, in a voice of its own.
    advance(&mut app, 0.79);
    app.world_mut().write_message(fired_ray());
    advance(&mut app, 0.01);
    assert_eq!(diagnostics(&app).cue_plays.get("radiant_beam"), Some(&1));
    assert_eq!(diagnostics(&app).dropped_rate, 0);
}

#[test]
fn a_full_frame_keeps_the_release_and_drops_what_only_adds_to_a_hit() {
    let mut app = skill_setup(SkillPresentation::target());
    // The ray kills a hero inside its core and bursts a mark on a minion, all in the frame
    // it fires: a kill, a hit, the release, a burst and a core note are five voices.
    let kill = CombatEvent {
        target: CombatEntity {
            kind: CombatEntityKind::Player,
            id: 8,
        },
        killed: true,
        core_hit: true,
        ..receipt(1, 0)
    };
    let burst = CombatEvent {
        radiance_burst: true,
        ..receipt(2, 0)
    };
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .combat_events
        .extend([burst, kill]);
    app.world_mut().write_message(fired_ray());
    advance(&mut app, 0.01);
    let report = diagnostics(&app);
    assert_eq!((report.played, report.dropped_rate), (4, 1));
    for cue in ["kill", "melee", "radiant_beam", "radiance_burst"] {
        assert_eq!(report.cue_plays.get(cue), Some(&1), "{cue}");
    }
    assert!(!report.cue_plays.contains_key("radiant_core"));
}

/// The length in seconds of what the decoder yields for one slice of the packaged sample
/// of `cue`, cut as the mixer cuts it.
fn decoded_secs(cue: AudioCue, slice: crate::skill_presentation::vocab::AudioSlice) -> f64 {
    use bevy::audio::{Decodable, Source};
    let catalog = CueCatalog::parse(include_str!("../../assets/audio/manifest.json")).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(&catalog.cues[cue.id()].path);
    let sample = AudioSource {
        bytes: std::fs::read(path).unwrap().into(),
    };
    let decoder = sample.decoder();
    let per_second = f64::from(decoder.sample_rate().get()) * f64::from(decoder.channels().get());
    let (start, duration) = Variant {
        slice,
        ..cue.into()
    }
    .span();
    let samples = match (start, duration) {
        (Some(start), Some(duration)) => {
            decoder.skip_duration(start).take_duration(duration).count()
        }
        (Some(start), None) => decoder.skip_duration(start).count(),
        (None, Some(duration)) => decoder.take_duration(duration).count(),
        (None, None) => decoder.count(),
    };
    samples as f64 / per_second
}

#[test]
fn every_slice_of_every_base_sample_sounds() {
    use crate::skill_presentation::vocab::AudioSlice;
    for cue in [
        AudioCue::Melee,
        AudioCue::Arrow,
        AudioCue::Arcane,
        AudioCue::Holy,
        AudioCue::Caster,
        AudioCue::Tower,
        AudioCue::Bluff,
        AudioCue::RadiantCharge,
        AudioCue::RadiantBeam,
    ] {
        let about = |secs: f64, wanted: f64| (secs - wanted).abs() < 0.002;
        let full = decoded_secs(cue, AudioSlice::Full);
        // `body` ends 0.40 s into the sample and `tail` starts 0.20 s into it.
        assert!(full >= 0.41, "{} lasts {full:.3} s", cue.id());
        let tick = decoded_secs(cue, AudioSlice::Tick);
        assert!(about(tick, 0.12), "{} tick {tick:.3}", cue.id());
        let body = decoded_secs(cue, AudioSlice::Body);
        assert!(about(body, 0.34), "{} body {body:.3}", cue.id());
        let tail = decoded_secs(cue, AudioSlice::Tail);
        assert!(about(tail, full - 0.20), "{} tail {tail:.3}", cue.id());
    }
}

#[test]
fn the_charge_lasts_the_windup_past_the_tail_and_the_cues_of_receipts_sound() {
    use crate::skill_presentation::vocab::AudioSlice;
    // The warning starts at the tail of the charge and peaks when the ray fires, 0.80 s on,
    // with a short ring after it.
    let tail = decoded_secs(AudioCue::RadiantCharge, AudioSlice::Tail);
    assert!((0.85..=1.0).contains(&tail), "{tail:.3}");
    for cue in [AudioCue::RadiantCore, AudioCue::RadianceBurst] {
        let full = decoded_secs(cue, AudioSlice::Full);
        assert!(
            (0.3..=0.6).contains(&full),
            "{} lasts {full:.3} s",
            cue.id()
        );
    }
}
