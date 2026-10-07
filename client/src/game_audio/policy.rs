use std::{collections::BTreeMap, time::Duration};

use bevy::prelude::*;
use serde::Deserialize;
use shared::{
    HeroClass,
    combat::{CombatEntity, CombatEntityKind, CombatEvent, ProjectileStyle},
    loadout::LoadoutState,
};

use crate::{
    net::GameState,
    skill_presentation::{
        SkillPresentation, SoundCue,
        cast::{CastKey, SkillCastObserved},
        stage::{EndKind, StageChange, StageEvent, Transition},
        vocab::{AudioBase, AudioSlice},
    },
    team::Team,
};

pub(super) const MAX_VOICES: usize = 12;
pub(super) const MAX_FRAME_CUES: usize = 4;
pub(super) const EFFECT_MAX_AGE: f64 = 4.0;
pub(super) const PENDING_MAX_AGE: f64 = 0.3;
/// Sample speeds are whole steps of 0.05; this many play the sample as recorded.
pub(super) const UNIT_STEP: u8 = 20;
const SPEED_STEP: f32 = 0.05;
/// Later notes one voice may carry.
pub(super) const MAX_NOTES: usize = 2;
/// A voice of a row is played up to this much faster or slower than its row says, so a
/// skill that hits many times does not repeat one identical sound.
pub(super) const MAX_DETUNE: f32 = 0.03;

/// Stable presentation IDs. Requests do not modify gameplay or confer authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AudioCue {
    Melee,
    Arrow,
    Arcane,
    Holy,
    Caster,
    Tower,
    Hit,
    Kill,
    Death,
    Respawn,
    LevelUp,
    MatchStart,
    Victory,
    Defeat,
    UiClick,
    UiConfirm,
    Butterfly,
    TrapTrigger,
    Bluff,
    VitalBreak,
}

impl AudioCue {
    pub(super) const ALL: [Self; 20] = [
        Self::Melee,
        Self::Arrow,
        Self::Arcane,
        Self::Holy,
        Self::Caster,
        Self::Tower,
        Self::Hit,
        Self::Kill,
        Self::Death,
        Self::Respawn,
        Self::LevelUp,
        Self::MatchStart,
        Self::Victory,
        Self::Defeat,
        Self::UiClick,
        Self::UiConfirm,
        Self::Butterfly,
        Self::TrapTrigger,
        Self::Bluff,
        Self::VitalBreak,
    ];

    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Melee => "melee",
            Self::Arrow => "arrow",
            Self::Arcane => "arcane",
            Self::Holy => "holy",
            Self::Caster => "caster",
            Self::Tower => "tower",
            Self::Hit => "hit",
            Self::Kill => "kill",
            Self::Death => "death",
            Self::Respawn => "respawn",
            Self::LevelUp => "level_up",
            Self::MatchStart => "match_start",
            Self::Victory => "victory",
            Self::Defeat => "defeat",
            Self::UiClick => "ui_click",
            Self::UiConfirm => "ui_confirm",
            Self::Butterfly => "butterfly",
            Self::TrapTrigger => "trap_trigger",
            Self::Bluff => "bluff",
            Self::VitalBreak => "vital_break",
        }
    }

    pub(super) const fn is_ui(self) -> bool {
        matches!(self, Self::UiClick | Self::UiConfirm)
    }

    pub(super) const fn priority(self) -> u8 {
        match self {
            Self::Victory | Self::Defeat | Self::Death => 0,
            Self::Respawn
            | Self::LevelUp
            | Self::Kill
            | Self::MatchStart
            | Self::Butterfly
            | Self::TrapTrigger
            | Self::VitalBreak => 1,
            Self::UiClick | Self::UiConfirm | Self::Hit => 2,
            _ => 3,
        }
    }

    const fn cooldown(self) -> f64 {
        match self {
            Self::Victory | Self::Defeat | Self::Death | Self::Respawn | Self::MatchStart => 0.8,
            Self::Kill | Self::LevelUp => 0.45,
            Self::Caster => 0.2,
            Self::UiClick | Self::UiConfirm => 0.08,
            _ => 0.12,
        }
    }

    /// The sample a voice of a skill row is built from.
    const fn for_base(base: AudioBase) -> Self {
        match base {
            AudioBase::Melee => Self::Melee,
            AudioBase::Arrow => Self::Arrow,
            AudioBase::Arcane => Self::Arcane,
            AudioBase::Holy => Self::Holy,
            AudioBase::Caster => Self::Caster,
            AudioBase::Tower => Self::Tower,
            AudioBase::Bluff => Self::Bluff,
        }
    }

    fn for_style(style: ProjectileStyle) -> Self {
        match style {
            ProjectileStyle::Arrow | ProjectileStyle::Bullet => Self::Arrow,
            ProjectileStyle::Arcane => Self::Arcane,
            ProjectileStyle::Holy => Self::Holy,
            ProjectileStyle::CasterBolt => Self::Caster,
            ProjectileStyle::TowerBolt | ProjectileStyle::Rocket => Self::Tower,
            ProjectileStyle::Standard | ProjectileStyle::Crescent | ProjectileStyle::Claw => {
                Self::Melee
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CueAsset {
    pub path: String,
    #[serde(default = "unit_gain")]
    pub gain: f32,
}

fn unit_gain() -> f32 {
    1.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CueCatalog {
    version: u32,
    pub music: CueAsset,
    pub cues: BTreeMap<String, CueAsset>,
}

impl Default for CueCatalog {
    fn default() -> Self {
        Self {
            version: 1,
            music: CueAsset {
                path: "audio/music/arena.ogg".into(),
                gain: 0.8,
            },
            cues: AudioCue::ALL
                .into_iter()
                .map(|cue| {
                    (
                        cue.id().into(),
                        CueAsset {
                            path: format!("audio/sfx/{}.ogg", cue.id()),
                            gain: 0.55,
                        },
                    )
                })
                .collect(),
        }
    }
}

impl CueCatalog {
    pub fn parse(json: &str) -> Result<Self, String> {
        let catalog: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        if catalog.version != 1 {
            return Err("audio manifest version must be 1".into());
        }
        validate_asset(&catalog.music, "audio/music/")?;
        if catalog.cues.len() != AudioCue::ALL.len() {
            return Err("audio manifest must contain exactly the supported cue IDs".into());
        }
        for cue in AudioCue::ALL {
            let asset = catalog
                .cues
                .get(cue.id())
                .ok_or_else(|| format!("missing audio cue {}", cue.id()))?;
            validate_asset(asset, "audio/sfx/")?;
        }
        Ok(catalog)
    }
}

fn validate_asset(asset: &CueAsset, prefix: &str) -> Result<(), String> {
    let valid = asset
        .path
        .strip_prefix(prefix)
        .and_then(|name| name.strip_suffix(".ogg"))
        .is_some_and(|stem| {
            !stem.is_empty()
                && stem.len() <= 80
                && stem
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        });
    if !valid || !asset.gain.is_finite() || !(0.0..=1.0).contains(&asset.gain) {
        return Err(format!(
            "invalid packaged audio asset {} (expected {prefix}<name>.ogg, gain 0..1)",
            asset.path
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct LocalState {
    pub id: u64,
    pub position: Vec3,
    pub alive: bool,
    pub level: u32,
    pub team: Team,
}

/// One way to play a sample: its speed and the part of it that sounds. Every variant has a
/// cooldown of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Variant {
    pub cue: AudioCue,
    /// Playback speed in steps of 0.05.
    pub step: u8,
    pub slice: AudioSlice,
}

impl From<AudioCue> for Variant {
    /// The sample as recorded.
    fn from(cue: AudioCue) -> Self {
        Self {
            cue,
            step: UNIT_STEP,
            slice: AudioSlice::Full,
        }
    }
}

impl Variant {
    pub fn speed(self) -> f32 {
        f32::from(self.step) * SPEED_STEP
    }

    /// The part of the sample that sounds: where it starts, and how long it lasts when it
    /// does not run to the end. Both are times of the sample, whatever its playback speed.
    pub fn span(self) -> (Option<Duration>, Option<Duration>) {
        let ms = Duration::from_millis;
        match self.slice {
            AudioSlice::Full => (None, None),
            AudioSlice::Tick => (None, Some(ms(120))),
            AudioSlice::Body => (Some(ms(60)), Some(ms(340))),
            AudioSlice::Tail => (Some(ms(200)), None),
        }
    }
}

/// The moments of an action that a row of the skill registry gives a voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Moment {
    Cast,
    Recast,
    /// A telegraph fired.
    Release,
    /// A confirmed hit.
    Impact,
    /// An accepted basic attack of an enemy, in the voice of its hit.
    Attack,
}

impl Moment {
    #[cfg(feature = "qa")]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Cast => "cast",
            Self::Recast => "recast",
            Self::Release => "release",
            Self::Impact => "impact",
            Self::Attack => "attack",
        }
    }
}

/// Where a candidate comes from, as far as the skill registry is concerned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Origin {
    /// Game state, the interface or an explicit request. No row answers for it.
    Other,
    /// The wire style of an accepted damage receipt, until the row of the action that
    /// dealt it gives the hit a voice of its own.
    Receipt {
        id: u64,
        source: CombatEntity,
        slot: Option<u8>,
    },
    /// The class style of an accepted attack of an enemy, until a row answers for it.
    Attack { actor: u64, sequence: u64 },
    /// A row gave the voice: the id of the row, the hero that acted (0 when the client
    /// does not know it) and the action sequence, receipt or effect that set it off.
    Row {
        moment: Moment,
        row: &'static str,
        actor: u64,
        id: u64,
    },
}

/// A later note of a voice: the same sample and slice again, after a delay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Note {
    pub delay_secs: f64,
    /// Playback speed in steps of 0.05.
    pub step: u8,
    pub gain: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Candidate {
    pub cue: AudioCue,
    pub gain: f32,
    /// Playback speed in steps of 0.05.
    pub step: u8,
    pub slice: AudioSlice,
    /// Later notes. Only a voice of the local hero's own action carries any.
    pub notes: [Option<Note>; MAX_NOTES],
    pub origin: Origin,
}

impl Candidate {
    /// The sample as recorded.
    const fn plain(cue: AudioCue, gain: f32, origin: Origin) -> Self {
        Self {
            cue,
            gain,
            step: UNIT_STEP,
            slice: AudioSlice::Full,
            notes: [None; MAX_NOTES],
            origin,
        }
    }

    pub fn local(cue: AudioCue) -> Self {
        Self::plain(cue, 1.0, Origin::Other)
    }

    /// The voice a row gives one moment, heard at `gain`. The later notes belong to the
    /// local hero's own action; everyone else is heard with the first note alone.
    fn voiced(cue: &SoundCue, gain: f32, own: bool, origin: Origin) -> Self {
        let step = |speed: f32| (speed / SPEED_STEP).round() as u8;
        let mut notes = [None; MAX_NOTES];
        if own {
            for (slot, note) in notes.iter_mut().zip(&cue.notes) {
                *slot = Some(Note {
                    delay_secs: f64::from(note.delay_ms) / 1000.0,
                    step: step(note.speed),
                    gain: gain * note.gain,
                });
            }
        }
        Self {
            cue: AudioCue::for_base(cue.base),
            gain: gain * cue.gain,
            step: step(cue.speed),
            slice: cue.slice,
            notes,
            origin,
        }
    }

    pub fn variant(&self) -> Variant {
        Variant {
            cue: self.cue,
            step: self.step,
            slice: self.slice,
        }
    }

    /// What the rate budget has to admit before the first note may sound.
    pub fn admission(&self) -> Admission {
        Admission {
            variant: self.variant(),
            notes: 1 + self.notes.iter().flatten().count(),
        }
    }

    /// The speed factor of this play: a voice of a row is detuned by a fixed amount that
    /// follows from the action, receipt or effect behind it. Everything else is exact.
    pub fn detune(&self) -> f32 {
        match self.origin {
            Origin::Row { id, .. } => detune(id),
            Origin::Other | Origin::Receipt { .. } | Origin::Attack { .. } => 1.0,
        }
    }
}

/// A speed factor within `MAX_DETUNE` of 1, the same for the same seed.
pub(super) fn detune(seed: u64) -> f32 {
    1.0 + MAX_DETUNE * crate::game_vfx::jitter(seed, 0, 0xA0D1)
}

#[derive(Default)]
pub(super) struct EventCursor {
    identity: Option<(u64, u64, u64)>,
    round: Option<(u64, u64)>,
    phase: Option<GameState>,
    high_water: u64,
    alive: bool,
    level: u32,
}

impl EventCursor {
    pub fn accept(
        &mut self,
        round: (u64, u64),
        phase: &GameState,
        local: Option<LocalState>,
        events: &[CombatEvent],
    ) -> (bool, Vec<Candidate>) {
        let previous_round = self.round.replace(round);
        let previous_phase = self.phase.replace(phase.clone());
        let latest = events.iter().map(|event| event.id).max().unwrap_or(0);
        let Some(local) = local else {
            let changed = self.identity.take().is_some();
            self.high_water = latest;
            return (changed, Vec::new());
        };
        let identity = (round.0, round.1, local.id);
        if self.identity != Some(identity) {
            let previous_identity = self.identity.replace(identity);
            self.high_water = latest;
            self.alive = local.alive;
            self.level = local.level;
            // Announce an observed start, but never replay an already-running match
            // merely because a client connected or a local entity was recreated.
            let observed_start = matches!(phase, GameState::Running)
                && previous_round.is_some_and(|old| old.0 == round.0)
                && (matches!(
                    previous_phase,
                    Some(GameState::Lobby | GameState::Forming { .. } | GameState::Starting { .. })
                ) || previous_identity.is_some_and(|old| (old.0, old.1) != round));
            return (
                true,
                if observed_start {
                    vec![Candidate::local(AudioCue::MatchStart)]
                } else {
                    Vec::new()
                },
            );
        }

        let mut cues = Vec::new();
        if !matches!(previous_phase, Some(GameState::Running))
            && matches!(phase, GameState::Running)
        {
            cues.push(Candidate::local(AudioCue::MatchStart));
        }
        if let GameState::Victory { winner } = phase
            && !matches!(previous_phase, Some(GameState::Victory { .. }))
        {
            cues.push(Candidate::local(if *winner == local.team {
                AudioCue::Victory
            } else {
                AudioCue::Defeat
            }));
        }
        let mut death = self.alive && !local.alive;
        if !self.alive && local.alive {
            cues.push(Candidate::local(AudioCue::Respawn));
        }
        if local.level > self.level {
            cues.push(Candidate::local(AudioCue::LevelUp));
        }
        self.alive = local.alive;
        self.level = local.level;

        let previous = self.high_water;
        self.high_water = self.high_water.max(latest);
        let mut seen = Vec::new();
        for event in events.iter().take(96) {
            if event.id <= previous || seen.contains(&event.id) {
                continue;
            }
            seen.push(event.id);
            if !event.amount.is_finite()
                || event.amount < 0.0
                || !Vec3::new(event.x, event.y, event.z).is_finite()
                || event.target.kind == CombatEntityKind::Unknown
            {
                continue;
            }
            let incoming =
                event.target.kind == CombatEntityKind::Player && event.target.id == local.id;
            let outgoing =
                event.source.kind == CombatEntityKind::Player && event.source.id == local.id;
            let gain = distance_gain(local.position, Vec3::new(event.x, event.y, event.z));
            if event.trap_triggered && (outgoing || incoming || gain > 0.0) {
                cues.push(Candidate::plain(
                    AudioCue::TrapTrigger,
                    if outgoing || incoming { 1.0 } else { gain },
                    Origin::Other,
                ));
            }
            // A shield can absorb the damage while the trap still activates.
            // Its explicit receipt is audible without inventing a damage hit.
            if event.amount == 0.0 {
                continue;
            }
            if event.near_lethal
                && !event.killed
                && event.target.kind == CombatEntityKind::Player
                && (outgoing || incoming || gain > 0.0)
            {
                cues.push(Candidate::plain(
                    AudioCue::VitalBreak,
                    if outgoing || incoming { 1.0 } else { gain },
                    Origin::Other,
                ));
            }
            if incoming {
                if event.killed {
                    death = true;
                } else {
                    cues.push(Candidate::local(AudioCue::Hit));
                }
            }
            if outgoing && event.killed && event.target.kind == CombatEntityKind::Player {
                cues.push(Candidate::local(AudioCue::Kill));
            }
            if gain > 0.0 {
                cues.push(Candidate::plain(
                    AudioCue::for_style(event.style),
                    gain,
                    Origin::Receipt {
                        id: event.id,
                        source: event.source,
                        slot: event.action_slot,
                    },
                ));
            }
        }
        if death {
            cues.push(Candidate::local(AudioCue::Death));
        }
        (false, cues)
    }
}

/// Accepted attack actions are audible even when a shield or god mode absorbs
/// every hit. This cursor consumes snapshots while muted and never replays old
/// attacks when an enemy appears from fog, reconnects or a round changes.
#[derive(Default)]
pub(super) struct AttackCursor {
    identity: Option<(u64, u64, u64)>,
    seen: BTreeMap<u64, u64>,
}
#[derive(Clone, Copy)]
pub(super) struct AttackObservation {
    pub id: u64,
    pub sequence: u64,
    pub attacking: bool,
    pub visible: bool,
    pub alive: bool,
    pub team: Team,
    pub position: Vec3,
    pub style: ProjectileStyle,
}
impl AttackCursor {
    pub fn accept(
        &mut self,
        round: (u64, u64),
        running: bool,
        local: Option<LocalState>,
        actors: impl IntoIterator<Item = AttackObservation>,
    ) -> Vec<Candidate> {
        let identity = local.map(|p| (round.0, round.1, p.id));
        if self.identity != identity {
            self.identity = identity;
            self.seen.clear();
        }
        let mut live = BTreeMap::new();
        let mut cues = Vec::new();
        for actor in actors {
            let previous = self.seen.get(&actor.id).copied();
            live.insert(actor.id, previous.unwrap_or(0).max(actor.sequence));
            if let Some(local) = local
                && running
                && actor.alive
                && actor.visible
                && actor.attacking
                && actor.team != local.team
                && previous.is_some_and(|seq| actor.sequence > seq)
            {
                let gain = distance_gain(local.position, actor.position);
                if gain > 0.0 {
                    cues.push(Candidate::plain(
                        AudioCue::for_style(actor.style),
                        gain,
                        Origin::Attack {
                            actor: actor.id,
                            sequence: actor.sequence,
                        },
                    ));
                }
            }
        }
        self.seen = live;
        cues
    }
}

/// A hero as the frame holds it, for the rows its actions resolve to.
#[derive(Clone, Copy)]
pub(super) struct HeroHeard<'a> {
    pub id: u64,
    /// The entity is drawn. Only a hero the client sees lends its rows to a receipt.
    pub visible: bool,
    pub class: HeroClass,
    pub loadout: Option<&'a LoadoutState>,
    /// The slot of the hero's latest accepted action.
    pub slot: u8,
}

/// What one frame observed that the skill registry may give a voice.
#[derive(Clone, Copy)]
pub(super) struct Heard<'a> {
    pub registry: &'a SkillPresentation,
    pub listener: LocalState,
    pub heroes: &'a [HeroHeard<'a>],
    /// The accepted actions the cast observer and the stage tracker reported.
    pub casts: &'a [SkillCastObserved],
    pub stages: &'a [StageEvent],
}

fn row_id(key: CastKey) -> &'static str {
    match key {
        CastKey::Skill(key) => key.id(),
        CastKey::Basic(class) => class.id(),
    }
}

/// The voice of an accepted cast: `sound.recast` on a recast edge of a row that has one,
/// else `sound.cast`. A basic attack is voiced by its hit, not by its swing.
fn cast_cue<'a>(
    registry: &'a SkillPresentation,
    cast: &SkillCastObserved,
) -> Option<(Moment, &'a SoundCue)> {
    let CastKey::Skill(key) = cast.key else {
        return None;
    };
    let sound = registry.row(key.id())?.sound.as_ref()?;
    match sound.recast.as_ref().filter(|_| cast.recast) {
        Some(cue) => Some((Moment::Recast, cue)),
        None => sound.cast.as_ref().map(|cue| (Moment::Cast, cue)),
    }
}

/// The voice of a confirmed hit: `sound.impact` of the skill that dealt it, or the cue of
/// the basic attack.
fn hit_cue(registry: &SkillPresentation, key: CastKey) -> Option<&SoundCue> {
    match key {
        CastKey::Skill(key) => registry.row(key.id())?.sound.as_ref()?.impact.as_ref(),
        CastKey::Basic(class) => registry.basic(class)?.sound.as_ref(),
    }
}

/// Gives the frame the voices of its rows. A receipt whose source is a hero the client
/// sees takes the hit voice of the row of that action; a receipt no row answers for keeps
/// the cue of its wire style. An enemy's accepted attack takes the voice of its basic
/// attack, or leaves the word to the cast voice of its skill. Every observed cast and
/// every telegraph that fired adds the voice of its row, if the row has one.
pub(super) fn voice_rows(heard: &Heard, candidates: &mut Vec<Candidate>) {
    let Heard {
        registry,
        listener,
        heroes,
        casts,
        stages,
    } = *heard;
    let hero = |id: u64| heroes.iter().find(|hero| hero.id == id);
    candidates.retain_mut(|candidate| match candidate.origin {
        Origin::Receipt { id, source, slot } => {
            let dealt = (source.kind == CombatEntityKind::Player)
                .then(|| hero(source.id))
                .flatten()
                .filter(|hero| hero.visible)
                .and_then(|hero| Some((hero, CastKey::of(hero.class, hero.loadout, slot?)?)));
            if let Some((hero, key)) = dealt
                && let Some(cue) = hit_cue(registry, key)
            {
                *candidate = Candidate::voiced(
                    cue,
                    candidate.gain,
                    hero.id == listener.id,
                    Origin::Row {
                        moment: Moment::Impact,
                        row: row_id(key),
                        actor: hero.id,
                        id,
                    },
                );
            }
            true
        }
        Origin::Attack { actor, sequence } => {
            let Some(hero) = hero(actor) else {
                return true;
            };
            // The cast voice of the same action stands for the attack.
            if casts.iter().any(|cast| {
                (cast.actor_id, cast.sequence, cast.slot) == (actor, sequence, hero.slot)
                    && cast_cue(registry, cast).is_some()
            }) {
                return false;
            }
            if let Some(key @ CastKey::Basic(_)) = CastKey::of(hero.class, hero.loadout, hero.slot)
                && let Some(cue) = hit_cue(registry, key)
            {
                *candidate = Candidate::voiced(
                    cue,
                    candidate.gain,
                    false,
                    Origin::Row {
                        moment: Moment::Attack,
                        row: row_id(key),
                        actor,
                        id: sequence,
                    },
                );
            }
            true
        }
        Origin::Other | Origin::Row { .. } => true,
    });

    let heard_at = |own: bool, at: Vec3| {
        if own {
            1.0
        } else {
            distance_gain(listener.position, at)
        }
    };
    // The local hero's own casts ask the rate budget first.
    for own in [true, false] {
        for cast in casts.iter().filter(|cast| cast.local == own) {
            let Some((moment, cue)) = cast_cue(registry, cast) else {
                continue;
            };
            let gain = heard_at(own, cast.position);
            if gain > 0.0 {
                candidates.push(Candidate::voiced(
                    cue,
                    gain,
                    own,
                    Origin::Row {
                        moment,
                        row: row_id(cast.key),
                        actor: cast.actor_id,
                        id: cast.sequence,
                    },
                ));
            }
        }
    }
    for event in stages {
        // A warning became the beam or the bolt it announced, or a fuse burned down.
        if !matches!(
            event.change,
            StageChange::Transition(Transition::KindFlipped)
                | StageChange::Ended(EndKind::Released)
        ) {
            continue;
        }
        let effect = &event.effect;
        let Some(cue) = registry
            .profile(effect.skill)
            .and_then(|profile| profile.sound.as_ref()?.release.as_ref())
        else {
            continue;
        };
        let own = event.owner.is_some_and(|owner| owner.local);
        // The release sounds where the effect was received, never where a hero stands.
        let gain = heard_at(own, Vec3::new(effect.position[0], 0.0, effect.position[1]));
        if gain > 0.0 {
            candidates.push(Candidate::voiced(
                cue,
                gain,
                own,
                Origin::Row {
                    moment: Moment::Release,
                    row: effect.skill.id(),
                    actor: effect.owner_id,
                    id: effect.id,
                },
            ));
        }
    }
}

pub(super) fn distance_gain(listener: Vec3, hit: Vec3) -> f32 {
    if !listener.is_finite() || !hit.is_finite() {
        return 0.0;
    }
    let distance = Vec2::new(listener.x - hit.x, listener.z - hit.z).length();
    ((36.0 - distance) / 30.0).clamp(0.0, 1.0).powi(2)
}

pub(super) fn expired(age: f64, has_sink: bool) -> bool {
    age >= EFFECT_MAX_AGE || (!has_sink && age >= PENDING_MAX_AGE)
}

/// What the rate budget is asked to admit: one variant of a sample and how many notes of
/// it, the first one included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Admission {
    pub variant: Variant,
    pub notes: usize,
}

impl From<AudioCue> for Admission {
    /// One note of the sample as recorded.
    fn from(cue: AudioCue) -> Self {
        Self {
            variant: cue.into(),
            notes: 1,
        }
    }
}

pub(super) struct RateBudget {
    tokens: f64,
    updated: f64,
    last: BTreeMap<Variant, f64>,
}

impl Default for RateBudget {
    fn default() -> Self {
        Self {
            tokens: 4.0,
            updated: 0.0,
            last: BTreeMap::new(),
        }
    }
}

impl RateBudget {
    /// Admits a voice with every note it has, or refuses it whole: the voice limit, the
    /// frame limit and the tokens must cover all of its notes in the frame that asks. The
    /// cooldown is kept per variant, so two variants of one sample do not silence each
    /// other. `active` counts the voices that sound and the notes already admitted.
    pub fn allow(
        &mut self,
        voice: impl Into<Admission>,
        now: f64,
        active: usize,
        frame: usize,
    ) -> bool {
        let Admission { variant, notes } = voice.into();
        self.tokens = (self.tokens + (now - self.updated).max(0.0) * 8.0).min(4.0);
        self.updated = now;
        if active + notes > MAX_VOICES
            || frame + notes > MAX_FRAME_CUES
            || self.tokens < notes as f64
            || self
                .last
                .get(&variant)
                .is_some_and(|last| now - *last < variant.cue.cooldown())
        {
            return false;
        }
        self.tokens -= notes as f64;
        self.last.insert(variant, now);
        true
    }
}

#[cfg(test)]
mod tests;
