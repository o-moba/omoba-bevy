//! Local audio presentation. Accepted server receipts select effects; audio never
//! predicts damage, modifies gameplay, or retains a queue of unheard hits.
mod policy;

use std::collections::BTreeMap;

use bevy::{
    audio::{AudioSinkPlayback, Volume},
    ecs::system::SystemParam,
    prelude::*,
    window::PrimaryWindow,
};
use serde::Serialize;

use crate::{
    audio_settings::AudioSettings,
    combat::CombatStats,
    input_context::{GameplayInputContext, InputContextSet},
    mobile_controls::MobileControls,
    net::{ClientSession, GameState, GameStateSnapshot, NetworkPlayerId, PlayerProgression},
    player::Player,
    team::Team,
};
pub(crate) use policy::AudioCue;
use policy::{Candidate, CueCatalog, EventCursor, LocalState, RateBudget, expired};

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameAudioRuntime>()
            .init_resource::<GameAudioDiagnostics>()
            .add_message::<AudioCueRequest>()
            .add_systems(Startup, load_audio)
            .add_systems(
                Update,
                update_audio
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .after(crate::net::ClientNetPipeline::SessionLifecycle)
                    .after(InputContextSet::Resolve),
            );
    }
}

/// Explicit presentation cues for UI confirmation and opt-in audio QA. These
/// obey the same mixer, loading, focus and rate policy as normal playback.
#[derive(Message)]
pub(crate) struct AudioCueRequest(pub(crate) AudioCue);

#[derive(Resource, Default, Serialize)]
pub(crate) struct GameAudioDiagnostics {
    pub(crate) assets_ready: usize,
    pub(crate) assets_total: usize,
    pub(crate) active_effects: usize,
    pub(crate) active_sinks: usize,
    pub(crate) observed_effect_sinks: u64,
    pub(crate) music_sink: bool,
    pub(crate) music_position_secs: f32,
    pub(crate) music_volume: f32,
    pub(crate) music_paused: bool,
    pub(crate) focused: bool,
    pub(crate) unlocked: bool,
    pub(crate) muted: bool,
    pub(crate) played: u64,
    pub(crate) dropped_missing: u64,
    pub(crate) dropped_rate: u64,
    pub(crate) last_cue: String,
    pub(crate) cue_plays: BTreeMap<String, u64>,
}

struct LoadedCue {
    source: Handle<AudioSource>,
    gain: f32,
}

#[derive(Resource, Default)]
struct GameAudioRuntime {
    music: Option<LoadedCue>,
    cues: BTreeMap<AudioCue, LoadedCue>,
    cursor: EventCursor,
    budget: RateBudget,
    unlocked: bool,
    music_gain: f32,
}

#[derive(Component)]
struct AudioVoice {
    // None denotes the sole persistent music voice.
    cue: Option<AudioCue>,
    gain: f32,
    born: f64,
    observed_sink: bool,
}

fn load_audio(mut runtime: ResMut<GameAudioRuntime>, assets: Res<AssetServer>) {
    let catalog =
        CueCatalog::parse(include_str!("../assets/audio/manifest.json")).unwrap_or_else(|error| {
            warn!("Audio manifest rejected: {error}; using bundled defaults");
            CueCatalog::default()
        });
    runtime.music = Some(LoadedCue {
        source: assets.load(catalog.music.path),
        gain: catalog.music.gain,
    });
    runtime.cues = AudioCue::ALL
        .into_iter()
        .map(|cue| {
            let asset = &catalog.cues[cue.id()];
            (
                cue,
                LoadedCue {
                    source: assets.load(asset.path.clone()),
                    gain: asset.gain,
                },
            )
        })
        .collect();
}

#[derive(SystemParam)]
struct AudioInput<'w, 's> {
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    mouse: Option<Res<'w, ButtonInput<MouseButton>>>,
    keyboard: Option<Res<'w, ButtonInput<KeyCode>>>,
    touches: Option<Res<'w, Touches>>,
    mobile: Option<Res<'w, MobileControls>>,
    context: Option<Res<'w, GameplayInputContext>>,
    buttons: Query<'w, 's, &'static Interaction, (With<Button>, Changed<Interaction>)>,
}

impl AudioInput<'_, '_> {
    fn touch_pressed(&self) -> bool {
        self.touches
            .as_ref()
            .is_some_and(|touches| touches.any_just_pressed())
    }
    fn physical_input(&self) -> bool {
        self.touch_pressed()
            || self
                .mouse
                .as_ref()
                .is_some_and(|mouse| mouse.get_just_pressed().next().is_some())
            || self
                .keyboard
                .as_ref()
                .is_some_and(|keys| keys.get_just_pressed().next().is_some())
    }
    fn ui_pressed(&self) -> bool {
        // Native desktop uses Bevy button interaction; mobile touch may also
        // synthesize a later mouse press, which must not sound a second time.
        let real_press = if self.mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
            self.touch_pressed()
        } else {
            self.mouse
                .as_ref()
                .is_some_and(|mouse| mouse.just_pressed(MouseButton::Left))
                || self.keyboard.as_ref().is_some_and(|keys| {
                    keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space)
                })
        };
        real_press
            && self
                .buttons
                .iter()
                .any(|interaction| *interaction == Interaction::Pressed)
    }
}

#[derive(SystemParam)]
struct AudioWorld<'w, 's> {
    snapshot: Res<'w, GameStateSnapshot>,
    session: Res<'w, ClientSession>,
    local: Query<
        'w,
        's,
        (
            &'static NetworkPlayerId,
            &'static Transform,
            &'static CombatStats,
            &'static PlayerProgression,
            &'static Team,
        ),
        With<Player>,
    >,
}

#[allow(clippy::too_many_arguments)]
fn update_audio(
    mut commands: Commands,
    time: Res<Time<Real>>,
    input: AudioInput,
    world: AudioWorld,
    settings: Option<Res<AudioSettings>>,
    sources: Res<Assets<AudioSource>>,
    mut requests: MessageReader<AudioCueRequest>,
    mut runtime: ResMut<GameAudioRuntime>,
    mut diagnostics: ResMut<GameAudioDiagnostics>,
    mut voices: Query<(
        Entity,
        &mut AudioVoice,
        &mut PlaybackSettings,
        Option<&mut AudioSink>,
    )>,
) {
    let now = time.elapsed_secs_f64();
    let settings = settings.as_deref().copied().unwrap_or_default().sanitized();
    let focused = input.windows.single().is_ok_and(|window| window.focused);
    let requires_gesture = cfg!(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios"
    )) || input.mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    runtime.unlocked |= !requires_gesture || (focused && input.physical_input());
    let audible = focused && runtime.unlocked && !settings.muted && settings.master > 0.0;

    let local = world
        .session
        .join_confirmed()
        .then(|| world.local.single().ok())
        .flatten()
        .map(|(id, transform, combat, progression, team)| LocalState {
            id: id.0,
            position: transform.translation,
            alive: combat.hp > 0.0,
            level: progression.level,
            team: *team,
        });
    let (changed, mut candidates) = runtime.cursor.accept(
        (
            world.snapshot.meta.server_epoch,
            world.snapshot.meta.match_id,
        ),
        &world.snapshot.state,
        local,
        &world.snapshot.combat_events,
    );
    if input.ui_pressed() {
        candidates.push(Candidate::local(AudioCue::UiClick));
    }
    candidates.extend(requests.read().map(|request| Candidate::local(request.0)));
    candidates.sort_by_key(|candidate| candidate.cue.priority());

    diagnostics.assets_total = runtime.cues.len() + usize::from(runtime.music.is_some());
    diagnostics.assets_ready = runtime
        .cues
        .values()
        .filter(|cue| sources.contains(&cue.source))
        .count()
        + usize::from(
            runtime
                .music
                .as_ref()
                .is_some_and(|cue| sources.contains(&cue.source)),
        );
    diagnostics.focused = focused;
    diagnostics.unlocked = runtime.unlocked;
    diagnostics.muted = settings.muted;
    diagnostics.active_effects = 0;
    diagnostics.active_sinks = 0;
    diagnostics.music_sink = false;
    diagnostics.music_position_secs = 0.0;
    diagnostics.music_volume = 0.0;
    diagnostics.music_paused = true;

    let state_gain = match world.snapshot.state {
        GameState::Running => 1.0,
        GameState::Victory { .. } => 0.65,
        _ => 0.45,
    };
    let modal_gain = if input
        .context
        .as_ref()
        .is_some_and(|context| context.modal_open)
    {
        0.65
    } else {
        1.0
    };
    let target_music_gain = if audible {
        settings.master * settings.music * state_gain * modal_gain
    } else {
        0.0
    };
    if !audible {
        runtime.music_gain = 0.0;
    } else {
        runtime.music_gain += (target_music_gain - runtime.music_gain)
            * (1.0 - (-time.delta_secs().min(0.25) * 4.0).exp());
    }

    let mut has_music = false;
    for (entity, mut voice, mut playback, sink) in &mut voices {
        if let Some(cue) = voice.cue {
            if !audible || changed || expired(now - voice.born, sink.is_some()) {
                if let Some(sink) = sink {
                    sink.stop();
                }
                commands.entity(entity).despawn();
                continue;
            }
            diagnostics.active_effects += 1;
            let bus = if cue.is_ui() {
                settings.ui
            } else {
                settings.effects
            };
            let volume = Volume::Linear(settings.master * bus * voice.gain);
            playback.volume = volume;
            if let Some(mut sink) = sink {
                if !voice.observed_sink {
                    diagnostics.observed_effect_sinks += 1;
                    voice.observed_sink = true;
                }
                diagnostics.active_sinks += 1;
                sink.set_volume(volume);
            }
        } else {
            has_music = true;
            let volume = Volume::Linear(runtime.music_gain * voice.gain);
            playback.volume = volume;
            playback.paused = !audible || settings.music == 0.0;
            if let Some(mut sink) = sink {
                sink.set_volume(volume);
                if playback.paused {
                    sink.pause();
                } else {
                    sink.play();
                }
                diagnostics.active_sinks += 1;
                diagnostics.music_sink = true;
                diagnostics.music_volume = sink.volume().to_linear();
                diagnostics.music_paused = sink.is_paused();
                diagnostics.music_position_secs = sink.position().as_secs_f32();
            }
        }
    }

    if !has_music
        && audible
        && settings.music > 0.0
        && let Some(music) = &runtime.music
        && sources.contains(&music.source)
    {
        // One pending loop is safe even on machines without an output device.
        // Effects have a short independent pending deadline and never accumulate.
        commands.spawn((
            AudioPlayer::new(music.source.clone()),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(runtime.music_gain * music.gain)),
            AudioVoice {
                cue: None,
                gain: music.gain,
                born: now,
                observed_sink: false,
            },
        ));
    }

    // Candidates are consumed even while muted, loading, unfocused or throttled.
    // There is deliberately no retry queue for stale one-shot effects.
    if !audible {
        return;
    }
    let mut frame_voices = 0;
    for candidate in candidates {
        let bus = if candidate.cue.is_ui() {
            settings.ui
        } else {
            settings.effects
        };
        if bus == 0.0 {
            continue;
        }
        let Some(cue) = runtime.cues.get(&candidate.cue) else {
            continue;
        };
        if !sources.contains(&cue.source) {
            diagnostics.dropped_missing += 1;
            continue;
        }
        let source = cue.source.clone();
        let gain = cue.gain * candidate.gain;
        if gain <= 0.0 {
            continue;
        }
        if !runtime
            .budget
            .allow(candidate.cue, now, diagnostics.active_effects, frame_voices)
        {
            diagnostics.dropped_rate += 1;
            continue;
        }
        commands.spawn((
            AudioPlayer::new(source),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(settings.master * bus * gain)),
            AudioVoice {
                cue: Some(candidate.cue),
                gain,
                born: now,
                observed_sink: false,
            },
        ));
        diagnostics.active_effects += 1;
        frame_voices += 1;
        diagnostics.played += 1;
        diagnostics.last_cue = candidate.cue.id().into();
        *diagnostics
            .cue_plays
            .entry(candidate.cue.id().into())
            .or_default() += 1;
    }
}

#[cfg(test)]
mod runtime_tests;
