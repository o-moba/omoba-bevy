//! Local audio presentation. Accepted server receipts select effects; audio never
//! predicts damage, modifies gameplay, or retains a queue of unheard hits.
//!
//! A row of the skill registry gives its skill a voice: a sample, a speed, a slice of the
//! sample and, for the local hero, up to two later notes. Such a voice is set off by an
//! accepted action the client observed, by a telegraph that fired or by an accepted
//! receipt, and by nothing else. The later notes are admitted together with the first one
//! and wait at most 240 ms for their turn; a voice that was refused is never tried again.
mod policy;

use std::collections::BTreeMap;

use bevy::{
    audio::{AudioSinkPlayback, Volume},
    ecs::{message::MessageCursor, system::SystemParam},
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
    skill_presentation::{SkillPresentation, cast::SkillCastObserved, stage::StageEvent},
    team::Team,
};
pub(crate) use policy::AudioCue;
use policy::{
    AttackCursor, AttackObservation, Candidate, CueCatalog, EventCursor, Heard, HeroHeard,
    LocalState, RateBudget, Variant, expired, voice_rows,
};

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
                    .after(InputContextSet::Resolve)
                    // The casts and stage events of this frame are voiced in this frame.
                    .after(crate::skill_presentation::stage::track_effects),
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
    /// Later notes of admitted voices that wait for their turn.
    pub(crate) pending_notes: usize,
    /// The newest voices the skill registry asked for, oldest first, whether or not the
    /// mixer let them sound.
    #[cfg(feature = "qa")]
    pub(crate) row_voices: std::collections::VecDeque<RowVoice>,
}

/// Evidence of one voice a row of the skill registry gave an action.
#[cfg(feature = "qa")]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RowVoice {
    /// `cast`, `recast`, `release`, `impact` or `attack`.
    pub(crate) moment: &'static str,
    /// The id of the skill row, or the class of a basic-attack row.
    pub(crate) row: &'static str,
    /// The hero that acted; 0 when the client does not know it.
    pub(crate) actor: u64,
    /// The action sequence, receipt or effect that set the voice off.
    pub(crate) id: u64,
    pub(crate) base: &'static str,
    pub(crate) speed: f32,
    pub(crate) slice: &'static str,
    pub(crate) gain: f32,
    /// Notes after the first one.
    pub(crate) notes: usize,
}

/// Voices kept as evidence.
#[cfg(feature = "qa")]
const ROW_VOICE_LOG: usize = 64;

struct LoadedCue {
    source: Handle<AudioSource>,
    gain: f32,
}

#[derive(Resource, Default)]
struct GameAudioRuntime {
    music: Option<LoadedCue>,
    cues: BTreeMap<AudioCue, LoadedCue>,
    cursor: EventCursor,
    attacks: AttackCursor,
    budget: RateBudget,
    /// Later notes of voices the budget admitted, each with the moment it starts.
    pending: Vec<PendingNote>,
    unlocked: bool,
    music_gain: f32,
}

struct PendingNote {
    due: f64,
    variant: Variant,
    /// The factor of the voice it belongs to, so its notes keep their interval.
    detune: f32,
    gain: f32,
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
    actors: Query<
        'w,
        's,
        (
            &'static NetworkPlayerId,
            &'static Transform,
            &'static CombatStats,
            &'static Team,
            &'static crate::net::NetworkHeroClass,
            &'static crate::net::PlayerCosmeticAction,
            Option<&'static InheritedVisibility>,
            Option<&'static crate::net::PlayerLoadout>,
        ),
    >,
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

/// What the skill registry may give a voice in one frame. An app without the skill
/// presentation has none of it and keeps the cues of the wire style.
#[derive(SystemParam)]
struct SkillVoices<'w, 's> {
    registry: Option<Res<'w, SkillPresentation>>,
    casts: Option<Res<'w, Messages<SkillCastObserved>>>,
    cast_cursor: Local<'s, MessageCursor<SkillCastObserved>>,
    stages: Option<Res<'w, Messages<StageEvent>>>,
    stage_cursor: Local<'s, MessageCursor<StageEvent>>,
}

impl SkillVoices<'_, '_> {
    /// The accepted actions and the stage events since the previous frame. They are taken
    /// in every frame, so none is voiced late.
    fn take(&mut self) -> (Vec<SkillCastObserved>, Vec<StageEvent>) {
        (
            self.casts.as_deref().map_or_else(Vec::new, |casts| {
                self.cast_cursor.read(casts).cloned().collect()
            }),
            self.stages.as_deref().map_or_else(Vec::new, |stages| {
                self.stage_cursor.read(stages).cloned().collect()
            }),
        )
    }
}

/// One note of a voice, as it is handed to the mixer.
fn note_voice(
    source: Handle<AudioSource>,
    variant: Variant,
    detune: f32,
    gain: f32,
    level: f32,
    now: f64,
) -> impl Bundle {
    let (start, duration) = variant.span();
    let mut playback = PlaybackSettings::DESPAWN
        .with_volume(Volume::Linear(level * gain))
        .with_speed(variant.speed() * detune);
    if let Some(start) = start {
        playback = playback.with_start_position(start);
    }
    if let Some(duration) = duration {
        playback = playback.with_duration(duration);
    }
    (
        AudioPlayer::new(source),
        playback,
        AudioVoice {
            cue: Some(variant.cue),
            gain,
            born: now,
            observed_sink: false,
        },
    )
}

fn update_audio(
    mut commands: Commands,
    time: Res<Time<Real>>,
    input: AudioInput,
    world: AudioWorld,
    mut skills: SkillVoices,
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
    candidates.extend(
        runtime.attacks.accept(
            (
                world.snapshot.meta.server_epoch,
                world.snapshot.meta.match_id,
            ),
            matches!(world.snapshot.state, GameState::Running),
            local,
            world
                .actors
                .iter()
                .map(
                    |(id, pose, stats, team, class, action, visibility, _)| AttackObservation {
                        id: id.0,
                        sequence: action.sequence,
                        attacking: action.kind == shared::PlayerActionKind::Attack,
                        visible: visibility.is_none_or(|v| v.get()),
                        alive: stats.is_alive(),
                        team: *team,
                        position: pose.translation,
                        style: shared::combat::ProjectileStyle::for_class(class.0),
                    },
                ),
        ),
    );
    let (casts, stages) = skills.take();
    if let (Some(registry), Some(listener)) = (skills.registry.as_deref(), local) {
        let heroes: Vec<HeroHeard> = world
            .actors
            .iter()
            .map(
                |(id, _, _, _, class, action, visibility, loadout)| HeroHeard {
                    id: id.0,
                    visible: visibility.is_none_or(|v| v.get()),
                    class: class.0,
                    loadout: loadout.and_then(|loadout| loadout.0.as_ref()),
                    slot: action.slot,
                },
            )
            .collect();
        voice_rows(
            &Heard {
                registry,
                listener,
                heroes: &heroes,
                casts: &casts,
                stages: &stages,
            },
            &mut candidates,
        );
    }
    #[cfg(feature = "qa")]
    for candidate in &candidates {
        if let policy::Origin::Row {
            moment,
            row,
            actor,
            id,
        } = candidate.origin
        {
            if diagnostics.row_voices.len() == ROW_VOICE_LOG {
                diagnostics.row_voices.pop_front();
            }
            diagnostics.row_voices.push_back(RowVoice {
                moment: moment.id(),
                row,
                actor,
                id,
                base: candidate.cue.id(),
                speed: candidate.variant().speed(),
                slice: candidate.slice.id(),
                gain: candidate.gain,
                notes: candidate.admission().notes - 1,
            });
        }
    }
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
    // A note belongs to its round and to a mixer that lets it sound.
    if changed || !audible {
        runtime.pending.clear();
    }

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
        diagnostics.pending_notes = 0;
        return;
    }
    // Later notes that are due were admitted with their voice. They start in this frame
    // and count against what the frame may start besides them.
    let mut frame_voices = runtime
        .pending
        .iter()
        .filter(|note| note.due <= now)
        .count();
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
        let sample_gain = cue.gain;
        if sample_gain * candidate.gain <= 0.0 {
            continue;
        }
        // Every note of the voice is admitted now, or the voice is dropped whole.
        let admission = candidate.admission();
        let reserved = diagnostics.active_effects + runtime.pending.len();
        if !runtime.budget.allow(admission, now, reserved, frame_voices) {
            diagnostics.dropped_rate += 1;
            continue;
        }
        frame_voices += admission.notes;
        let detune = candidate.detune();
        runtime.pending.push(PendingNote {
            due: now,
            variant: admission.variant,
            detune,
            gain: sample_gain * candidate.gain,
        });
        runtime
            .pending
            .extend(candidate.notes.iter().flatten().map(|note| PendingNote {
                due: now + note.delay_secs,
                variant: Variant {
                    step: note.step,
                    ..admission.variant
                },
                detune,
                gain: sample_gain * note.gain,
            }));
    }
    let mut waiting = std::mem::take(&mut runtime.pending);
    waiting.retain(|note| {
        if note.due > now {
            return true;
        }
        let cue = note.variant.cue;
        let bus = if cue.is_ui() {
            settings.ui
        } else {
            settings.effects
        };
        // The bus may have been closed while the note waited.
        if let Some(loaded) = runtime
            .cues
            .get(&cue)
            .filter(|loaded| bus > 0.0 && sources.contains(&loaded.source))
        {
            commands.spawn(note_voice(
                loaded.source.clone(),
                note.variant,
                note.detune,
                note.gain,
                settings.master * bus,
                now,
            ));
            diagnostics.active_effects += 1;
            diagnostics.played += 1;
            diagnostics.last_cue = cue.id().into();
            *diagnostics.cue_plays.entry(cue.id().into()).or_default() += 1;
        }
        false
    });
    diagnostics.pending_notes = waiting.len();
    runtime.pending = waiting;
}

#[cfg(test)]
mod runtime_tests;
