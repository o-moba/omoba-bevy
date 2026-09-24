use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::Deserialize;
use shared::combat::{CombatEntityKind, CombatEvent, ProjectileStyle};

use crate::{net::GameState, team::Team};

pub(super) const MAX_VOICES: usize = 12;
pub(super) const MAX_FRAME_CUES: usize = 4;
pub(super) const EFFECT_MAX_AGE: f64 = 4.0;
pub(super) const PENDING_MAX_AGE: f64 = 0.3;

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
}

impl AudioCue {
    pub(super) const ALL: [Self; 16] = [
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
        }
    }

    pub(super) const fn is_ui(self) -> bool {
        matches!(self, Self::UiClick | Self::UiConfirm)
    }

    pub(super) const fn priority(self) -> u8 {
        match self {
            Self::Victory | Self::Defeat | Self::Death => 0,
            Self::Respawn | Self::LevelUp | Self::Kill | Self::MatchStart => 1,
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

    fn for_style(style: ProjectileStyle) -> Self {
        match style {
            ProjectileStyle::Arrow => Self::Arrow,
            ProjectileStyle::Arcane => Self::Arcane,
            ProjectileStyle::Holy => Self::Holy,
            ProjectileStyle::CasterBolt => Self::Caster,
            ProjectileStyle::TowerBolt => Self::Tower,
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
            return Err("audio manifest must contain exactly the 16 supported cue IDs".into());
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

#[derive(Clone, Copy)]
pub(super) struct Candidate {
    pub cue: AudioCue,
    pub gain: f32,
}

impl Candidate {
    pub fn local(cue: AudioCue) -> Self {
        Self { cue, gain: 1.0 }
    }
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
                || event.amount <= 0.0
                || !Vec3::new(event.x, event.y, event.z).is_finite()
                || event.target.kind == CombatEntityKind::Unknown
            {
                continue;
            }
            let incoming =
                event.target.kind == CombatEntityKind::Player && event.target.id == local.id;
            let outgoing =
                event.source.kind == CombatEntityKind::Player && event.source.id == local.id;
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
            let gain = distance_gain(local.position, Vec3::new(event.x, event.y, event.z));
            if gain > 0.0 {
                cues.push(Candidate {
                    cue: AudioCue::for_style(event.style),
                    gain,
                });
            }
        }
        if death {
            cues.push(Candidate::local(AudioCue::Death));
        }
        (false, cues)
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

pub(super) struct RateBudget {
    tokens: f64,
    updated: f64,
    last: BTreeMap<AudioCue, f64>,
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
    pub fn allow(&mut self, cue: AudioCue, now: f64, active: usize, frame: usize) -> bool {
        self.tokens = (self.tokens + (now - self.updated).max(0.0) * 8.0).min(4.0);
        self.updated = now;
        if active >= MAX_VOICES
            || frame >= MAX_FRAME_CUES
            || self.tokens < 1.0
            || self
                .last
                .get(&cue)
                .is_some_and(|last| now - *last < cue.cooldown())
        {
            return false;
        }
        self.tokens -= 1.0;
        self.last.insert(cue, now);
        true
    }
}

#[cfg(test)]
mod tests;
