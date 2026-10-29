//! Headless difficulty simulator (docs/plans/bot-personas.md, S3).
//!
//! Full human-less 5v5 practice rounds between bot levels, in process, on
//! `PracticeSession` with the round start a `Join` performs
//! (`PracticeSession::bots_only`). A round ends at `GameState::Victory` or at
//! a time cap. Bot-only rounds rarely reach Victory soon (two Elite teams
//! still stand after 40 minutes), so a capped round is adjudicated: the side
//! that removed more of the enemy's structure health (a destroyed structure
//! counts 1, a damaged one its lost fraction) wins, and a lead under
//! `DRAW_MARGIN` is a draw.
//!
//! - **Seeds.** The standard roster has no dagger class, so a round draws no
//!   randomness and replays exactly. What varies between rounds is the step
//!   length: the host's step jitters around `dt` the way a server tick or a
//!   client frame does, from a fixed seed per round. Rounds alternate sides,
//!   so the subject level plays Green in even rounds and Blue in odd ones.
//! - **Metrics** per level side: wins, draws and round length; kills and
//!   deaths per minute; health retreats with the health at their start;
//!   outnumbered and threat retreats; threat-response answers; tower shots
//!   taken and tower-dive incidents; time fighting a hero, and how much of it
//!   outnumbered; route plans, and the plans that found no route. Each level
//!   must stay within `ROUTE_BUDGET` plans per bot-minute (LR-10).
//! - **Statistics.** Win rates are over decisive rounds, with 95% Wilson
//!   intervals. Adjacent levels are compared with a one-sided two-proportion
//!   z-test.
//!
//! The quick tests below run in CI on a small sample and assert only
//! intervals. The full matrix is `#[ignore]`d; run it on a many-core machine
//! (not the dev VPS) or with the `Bot difficulty matrix` workflow. The
//! acceptance matrix (docs/plans/bot-personas.md, S3 notes) is:
//!
//! ```sh
//! OMOBA_BOT_SIM_MATCHES=200 OMOBA_BOT_SIM_SEED=1 OMOBA_BOT_SIM_REPORT=report.json \
//!   cargo test --release --locked -p common --features test-support \
//!   --test bot_difficulty_sim -- --ignored --exact --nocapture full_matrix > /dev/null
//! ```
//!
//! The thread count defaults to the available parallelism and does not
//! change the result.
//!
//! Optional settings: `OMOBA_BOT_SIM_SEED` (base seed), `OMOBA_BOT_SIM_DT`
//! (nominal step, s), `OMOBA_BOT_SIM_JITTER` (relative step jitter),
//! `OMOBA_BOT_SIM_CAP_MIN` (time cap, game minutes), `OMOBA_BOT_SIM_PAIRINGS`
//! (`recruit:elite,regular:veteran@100`, subject first; `@N` sets that
//! pairing's rounds) and
//! `OMOBA_BOT_SIM_TUNE` (`recruit.retreat_hp=0.45;regular.trade_margin=off`),
//! which overrides profile fields for a calibration sweep.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::bots::sim_probe::{self, Event, GateCause, ThreatAnswer};
use common::bots::{BotProfile, JungleGank, TargetPriority};
use common::offline::PracticeSession;
use common::vision;
use shared::bot_profile::BotDifficulty;
use shared::combat::CombatEntityKind;
use shared::hero_balance::BOT_ENGAGE_RANGE as VISION;
use shared::map::Team;
use shared::wire::{GameState, TargetKind};

const TEAMS: [Team; 2] = [Team::Green, Team::Blue];
/// How often the fight sample runs: the bots' think interval.
const SAMPLE_SECS: f32 = 0.25;
/// Tower shots on one hero this close together are one dive.
const DIVE_GAP: Duration = Duration::from_secs(3);
/// 95% two-sided normal quantile.
const Z95: f64 = 1.959_964;
/// A structure lead below this (in structures) at the cap is a draw.
const DRAW_MARGIN: f64 = 0.05;
/// Route plans per bot-minute any level may spend (LR-10).
const ROUTE_BUDGET: f64 = 80.0;

fn side_of(team: Team) -> usize {
    match team {
        Team::Green => 0,
        Team::Blue => 1,
    }
}

/// SplitMix64: a fixed, portable sequence per seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1_u64 << 24) as f32
    }
}

fn fnv(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Clone, Copy, Debug)]
struct Config {
    /// Nominal step (s).
    dt: f32,
    /// Relative step jitter: each step is `dt * (1 ± jitter)`.
    jitter: f32,
    /// Game time after which a round is a draw (s).
    cap_secs: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dt: 0.05,
            jitter: 0.2,
            cap_secs: 20.0 * 60.0,
        }
    }
}

/// One team's behaviour in one round, or summed over rounds.
#[derive(Clone, Debug, Default)]
struct Side {
    rounds: u32,
    /// Bot-seconds on the field (five seats for the round's length).
    seat_secs: f64,
    kills: u32,
    deaths: u32,
    /// Health at the start of each health retreat.
    health_retreats: Vec<f32>,
    /// Health triggers during a retreat, which only extend it.
    health_retriggers: u32,
    outnumbered_retreats: u32,
    threat_retreats: u32,
    threat_retaliations: u32,
    /// Threat retreats by cause: beyond `VISION`, refused by a gate, or
    /// unreachable past a danger disc.
    threat_out_of_range: u32,
    threat_refused: u32,
    /// Refusals in which the trade gate, or the outnumbered hold, said no.
    threat_refused_trade: u32,
    threat_refused_outnumbered: u32,
    threat_unreachable: u32,
    tower_hits: u32,
    dives: u32,
    /// Routes planned, and the plans among them that found no route.
    routes: u32,
    route_failures: u32,
    hero_fight_secs: f64,
    outnumbered_fight_secs: f64,
    structures_destroyed: u32,
    /// Enemy structure health removed, in structures.
    structure_score: f64,
}

impl Side {
    fn add(&mut self, other: &Side) {
        self.rounds += other.rounds;
        self.seat_secs += other.seat_secs;
        self.kills += other.kills;
        self.deaths += other.deaths;
        self.health_retreats
            .extend_from_slice(&other.health_retreats);
        self.health_retriggers += other.health_retriggers;
        self.outnumbered_retreats += other.outnumbered_retreats;
        self.threat_retreats += other.threat_retreats;
        self.threat_retaliations += other.threat_retaliations;
        self.threat_out_of_range += other.threat_out_of_range;
        self.threat_refused += other.threat_refused;
        self.threat_refused_trade += other.threat_refused_trade;
        self.threat_refused_outnumbered += other.threat_refused_outnumbered;
        self.threat_unreachable += other.threat_unreachable;
        self.tower_hits += other.tower_hits;
        self.dives += other.dives;
        self.routes += other.routes;
        self.route_failures += other.route_failures;
        self.hero_fight_secs += other.hero_fight_secs;
        self.outnumbered_fight_secs += other.outnumbered_fight_secs;
        self.structures_destroyed += other.structures_destroyed;
        self.structure_score += other.structure_score;
    }

    /// Per bot-minute on the field.
    fn rate(&self, count: f64) -> f64 {
        if self.seat_secs > 0.0 {
            count * 60.0 / self.seat_secs
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
struct Outcome {
    /// Winning side (0 Green, 1 Blue); `None` for a draw at the cap.
    winner: Option<usize>,
    /// The round reached `GameState::Victory` before the cap.
    victory: bool,
    secs: f32,
    sides: [Side; 2],
    /// Combat receipts the event window dropped before they were read.
    lost_events: u64,
}

/// One round: Green plays `profiles[0]`, Blue `profiles[1]`.
fn play(
    seed: u64,
    levels: [BotDifficulty; 2],
    profiles: [BotProfile; 2],
    config: Config,
) -> Outcome {
    let start = Instant::now();
    let mut practice = PracticeSession::bots_only(start, levels);
    for (team, profile) in TEAMS.into_iter().zip(profiles) {
        practice
            .bots
            .set_team_profile(team, profile, &practice.world.players);
    }
    let side_by_addr: HashMap<SocketAddr, usize> = practice
        .world
        .players
        .iter()
        .map(|(addr, p)| (*addr, side_of(p.hero.identity.team)))
        .collect();
    let side_by_id: HashMap<u64, usize> = practice
        .world
        .players
        .values()
        .map(|p| (p.hero.identity.id, side_of(p.hero.identity.team)))
        .collect();
    assert_eq!(side_by_addr.len(), 10, "a full 5v5 bot roster");
    let structures: Vec<(u64, usize, f32)> = practice
        .world
        .structures
        .values()
        .map(|s| (s.state.id, side_of(s.state.team), s.state.max_hp))
        .collect();
    let mut rng = Rng(seed);
    let mut sides = [Side::default(), Side::default()];
    let mut last_event = 0_u64;
    let mut lost_events = 0_u64;
    let mut last_tower_hit: HashMap<u64, Instant> = HashMap::new();
    let mut next_sample = SAMPLE_SECS;
    sim_probe::start();
    while matches!(practice.world.game_state, GameState::Running)
        && practice.world.match_elapsed_secs < config.cap_secs
    {
        let dt = config.dt * (1.0 + config.jitter * (2.0 * rng.unit() - 1.0));
        practice.advance(dt);
        let now = practice.now;
        for event in sim_probe::drain() {
            match event {
                Event::HealthRetreat { bot, hp, continued } => {
                    let side = &mut sides[side_by_addr[&bot]];
                    if continued {
                        side.health_retriggers += 1;
                    } else {
                        side.health_retreats.push(hp);
                    }
                }
                Event::GateRetreat { bot, cause } => match cause {
                    GateCause::Outnumbered => sides[side_by_addr[&bot]].outnumbered_retreats += 1,
                    GateCause::Threat => sides[side_by_addr[&bot]].threat_retreats += 1,
                },
                Event::Route { bot, found } => {
                    let side = &mut sides[side_by_addr[&bot]];
                    side.routes += 1;
                    side.route_failures += u32::from(!found);
                }
                Event::ThreatAnswer { bot, answer } => {
                    let side = &mut sides[side_by_addr[&bot]];
                    match answer {
                        ThreatAnswer::Retaliated => side.threat_retaliations += 1,
                        ThreatAnswer::OutOfRange => side.threat_out_of_range += 1,
                        ThreatAnswer::Refused { trade, outnumbered } => {
                            side.threat_refused += 1;
                            side.threat_refused_trade += u32::from(trade);
                            side.threat_refused_outnumbered += u32::from(outnumbered);
                        }
                        ThreatAnswer::Unreachable => side.threat_unreachable += 1,
                    }
                }
            }
        }
        for event in practice.combat_log.snapshot(now) {
            if event.id <= last_event {
                continue;
            }
            lost_events += event.id - last_event - 1;
            last_event = event.id;
            if event.source.kind == CombatEntityKind::Structure
                && event.target.kind == CombatEntityKind::Player
                && event.amount > 0.0
                && let Some(&side) = side_by_id.get(&event.target.id)
            {
                sides[side].tower_hits += 1;
                let previous = last_tower_hit.insert(event.target.id, now);
                if previous.is_none_or(|at| now.saturating_duration_since(at) > DIVE_GAP) {
                    sides[side].dives += 1;
                }
            }
        }
        if practice.world.match_elapsed_secs >= next_sample {
            next_sample += SAMPLE_SECS;
            sample_fights(&practice, &mut sides, now);
        }
    }
    sim_probe::stop();
    let secs = practice.world.match_elapsed_secs;
    for &(id, owner, max_hp) in &structures {
        let left = practice
            .world
            .structures
            .get(&id)
            .filter(|s| s.state.hp > 0.0 && max_hp > 0.0)
            .map_or(0.0, |s| f64::from(s.state.hp / max_hp));
        sides[1 - owner].structure_score += 1.0 - left;
    }
    for participant in practice.combat_log.ledger.snapshot() {
        let side = &mut sides[side_of(participant.team)];
        side.kills += participant.stats.kills;
        side.deaths += participant.stats.deaths;
        side.structures_destroyed += participant.stats.structures_destroyed;
    }
    for side in &mut sides {
        side.rounds = 1;
        side.seat_secs = 5.0 * f64::from(secs);
    }
    let (winner, victory) = match practice.world.game_state {
        GameState::Victory { winner } => (Some(side_of(winner)), true),
        _ => {
            let lead = sides[0].structure_score - sides[1].structure_score;
            let winner = if lead >= DRAW_MARGIN {
                Some(0)
            } else if lead <= -DRAW_MARGIN {
                Some(1)
            } else {
                None
            };
            (winner, false)
        }
    };
    Outcome {
        winner,
        victory,
        secs,
        sides,
        lost_events,
    }
}

/// Time each living bot spends on a hero target, and how much of it with
/// more visible enemy heroes than allies within `VISION`.
fn sample_fights(practice: &PracticeSession, sides: &mut [Side; 2], now: Instant) {
    let world = &practice.world;
    let sight = TEAMS.map(|team| vision::sources(team, world));
    for (addr, bot) in &world.players {
        if !bot.joined || !bot.hero.identity.is_bot || bot.hero.hp <= 0.0 {
            continue;
        }
        if practice
            .bots
            .target(*addr)
            .is_none_or(|target| target.kind != TargetKind::Player)
        {
            continue;
        }
        let team = bot.hero.identity.team;
        let side = side_of(team);
        let mut outnumbered = 0_i32;
        for other in world.players.values().filter(|o| {
            o.joined
                && o.hero.hp > 0.0
                && (o.hero.x - bot.hero.x).hypot(o.hero.z - bot.hero.z) <= VISION
        }) {
            if other.hero.identity.team == team {
                outnumbered -= 1;
            } else if vision::player_visible(&sight[side], other, now) {
                outnumbered += 1;
            }
        }
        let secs = f64::from(SAMPLE_SECS);
        sides[side].hero_fight_secs += secs;
        if outnumbered >= 1 {
            sides[side].outnumbered_fight_secs += secs;
        }
    }
}

// ---------------------------------------------------------------------------
// Statistics

/// 95% Wilson score interval for `k` successes in `n` trials.
fn wilson(k: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let n = f64::from(n);
    let p = f64::from(k) / n;
    let z2 = Z95 * Z95;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let half = Z95 * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    ((center - half).max(0.0), (center + half).min(1.0))
}

/// Complementary error function (Numerical Recipes `erfcc`, |error| < 1.2e-7).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let r = t * poly.exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

/// One-sided p-value that the second proportion exceeds the first (pooled
/// two-proportion z-test).
fn p_greater(k1: u32, n1: u32, k2: u32, n2: u32) -> f64 {
    if n1 == 0 || n2 == 0 {
        return 1.0;
    }
    let (n1f, n2f) = (f64::from(n1), f64::from(n2));
    let (p1, p2) = (f64::from(k1) / n1f, f64::from(k2) / n2f);
    let pooled = f64::from(k1 + k2) / (n1f + n2f);
    let se = (pooled * (1.0 - pooled) * (1.0 / n1f + 1.0 / n2f)).sqrt();
    if se == 0.0 {
        return if p2 > p1 { 0.0 } else { 1.0 };
    }
    0.5 * erfc((p2 - p1) / se / std::f64::consts::SQRT_2)
}

fn sorted(values: &[f32]) -> Vec<f32> {
    let mut values = values.to_vec();
    values.sort_by(f32::total_cmp);
    values
}

fn quantile(sorted: &[f32], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * q).round() as usize;
    f64::from(sorted[index])
}

/// A distribution-free 95% interval for the median: the order statistics
/// `x(l)` and `x(u)` around n/2 at ±1.96·√n/2 ranks.
fn median_interval(sorted: &[f32]) -> (f64, f64) {
    let n = sorted.len();
    if n == 0 {
        return (f64::NAN, f64::NAN);
    }
    let half = Z95 * (n as f64).sqrt() / 2.0;
    let low = ((n as f64 / 2.0 - half).floor().max(0.0) as usize).min(n - 1);
    let high = ((n as f64 / 2.0 + half).ceil() as usize).min(n - 1);
    (f64::from(sorted[low]), f64::from(sorted[high]))
}

// ---------------------------------------------------------------------------
// Pairings

/// The subject level against an opponent level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pairing {
    subject: BotDifficulty,
    opponent: BotDifficulty,
}

impl Pairing {
    fn name(self) -> String {
        format!("{}:{}", self.subject.name(), self.opponent.name())
    }

    /// Round `i`'s seed: independent of the other pairings and of the order
    /// in which they run.
    fn seed(self, base: u64, round: u32) -> u64 {
        fnv(&format!("{}#{round}", self.name())) ^ base
    }
}

/// The default matrix: every level against Elite, and each adjacent pair.
fn default_pairings() -> Vec<Pairing> {
    use BotDifficulty::*;
    [
        (Elite, Elite),
        (Veteran, Elite),
        (Regular, Elite),
        (Recruit, Elite),
        (Regular, Veteran),
        (Recruit, Regular),
    ]
    .map(|(subject, opponent)| Pairing { subject, opponent })
    .into()
}

#[derive(Clone, Debug, Default)]
struct Tally {
    rounds: u32,
    subject_wins: u32,
    opponent_wins: u32,
    draws: u32,
    /// Rounds that reached Victory before the cap.
    victories: u32,
    /// Rounds won by Green, for the side bias.
    green_wins: u32,
    secs: Vec<f32>,
    /// Per round: the subject's structure score minus the opponent's.
    leads: Vec<f64>,
    subject: Side,
    opponent: Side,
    lost_events: u64,
}

impl Tally {
    fn decisive(&self) -> u32 {
        self.subject_wins + self.opponent_wins
    }

    fn add(&mut self, outcome: &Outcome, subject_side: usize) {
        self.rounds += 1;
        match outcome.winner {
            None => self.draws += 1,
            Some(side) if side == subject_side => self.subject_wins += 1,
            Some(_) => self.opponent_wins += 1,
        }
        self.victories += u32::from(outcome.victory);
        self.green_wins += u32::from(outcome.winner == Some(0));
        self.secs.push(outcome.secs);
        self.leads.push(
            outcome.sides[subject_side].structure_score
                - outcome.sides[1 - subject_side].structure_score,
        );
        self.subject.add(&outcome.sides[subject_side]);
        self.opponent.add(&outcome.sides[1 - subject_side]);
        self.lost_events += outcome.lost_events;
    }
}

/// Per-level profiles, after `OMOBA_BOT_SIM_TUNE`.
#[derive(Clone, Copy, Debug)]
struct Profiles([BotProfile; 4]);

impl Profiles {
    fn standard() -> Self {
        Self(BotDifficulty::ALL.map(BotProfile::for_level))
    }

    fn index(level: BotDifficulty) -> usize {
        BotDifficulty::ALL
            .iter()
            .position(|l| *l == level.effective())
            .expect("a selectable level")
    }

    fn get(&self, level: BotDifficulty) -> BotProfile {
        self.0[Self::index(level)]
    }

    /// `level.field=value` pairs separated by `;`. `off` clears an optional
    /// gate.
    fn tuned(spec: &str) -> Result<Self, String> {
        let mut profiles = Self::standard();
        for item in spec.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (key, value) = item
                .split_once('=')
                .ok_or_else(|| format!("{item}: expected level.field=value"))?;
            let (level, field) = key
                .split_once('.')
                .ok_or_else(|| format!("{key}: expected level.field"))?;
            let level =
                BotDifficulty::parse(level).ok_or_else(|| format!("{level}: not a bot level"))?;
            set_field(
                &mut profiles.0[Self::index(level)],
                field.trim(),
                value.trim(),
            )
            .map_err(|e| format!("{item}: {e}"))?;
        }
        Ok(profiles)
    }
}

fn set_field(profile: &mut BotProfile, field: &str, value: &str) -> Result<(), String> {
    let number = || value.parse::<f32>().map_err(|e| e.to_string());
    let optional = || {
        if value == "off" {
            Ok(None)
        } else {
            number().map(Some)
        }
    };
    let count = || {
        if value == "off" {
            Ok(None)
        } else {
            value.parse::<u8>().map(Some).map_err(|e| e.to_string())
        }
    };
    match field {
        "retreat_hp" => profile.retreat_hp = number()?,
        "resume_hp" => profile.resume_hp = optional()?,
        "retreat_secs" => profile.retreat_secs = number()?,
        "retreat_cap_secs" => profile.retreat_cap_secs = optional()?,
        "rearm_secs" => profile.rearm_secs = number()?,
        "cover_minions" => profile.cover_minions = value.parse().map_err(|e| format!("{e}"))?,
        "tower_margin" => profile.tower_margin = number()?,
        "staging_margin" => profile.staging_margin = number()?,
        "siege_min_hp" => profile.siege_min_hp = optional()?,
        "hero_engage_range" => profile.hero_engage_range = number()?,
        "target_leash" => profile.target_leash = number()?,
        "target_priority" => {
            profile.target_priority = match value {
                "nearest" => TargetPriority::Nearest,
                "minion_first" => TargetPriority::MinionFirst,
                _ => return Err("nearest or minion_first".into()),
            }
        }
        "trade_margin" => profile.trade_margin = optional()?,
        "outnumbered_hold" => profile.outnumbered_hold = count()?,
        "outnumbered_retreat" => profile.outnumbered_retreat = count()?,
        "threat_window_secs" => profile.threat_window_secs = optional()?,
        "dash_hero_hp" => profile.dash_hero_hp = optional()?,
        "mobility_margin" => profile.mobility_margin = number()?,
        "jungle_gank" => {
            profile.jungle_gank = match value {
                "always" => JungleGank::Always,
                "if_not_outnumbered" => JungleGank::IfNotOutnumbered,
                "if_ahead" => JungleGank::IfAhead,
                "never" => JungleGank::Never,
                _ => return Err("always, if_not_outnumbered, if_ahead or never".into()),
            }
        }
        _ => return Err(format!("unknown profile field {field}")),
    }
    Ok(())
}

/// Play `rounds[i]` rounds of pairing `i` on `threads` threads. The result
/// does not depend on the thread count.
fn run_matrix(
    pairings: &[Pairing],
    rounds: &[u32],
    base_seed: u64,
    profiles: Profiles,
    config: Config,
    threads: usize,
) -> Vec<Tally> {
    let jobs: Vec<(usize, u32)> = (0..pairings.len())
        .flat_map(|pairing| (0..rounds[pairing]).map(move |round| (pairing, round)))
        .collect();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Outcome>>> = Mutex::new(vec![None; jobs.len()]);
    let done = AtomicUsize::new(0);
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                loop {
                    let job = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(pairing, round)) = jobs.get(job) else {
                        break;
                    };
                    let pairing = pairings[pairing];
                    let subject_side = (round % 2) as usize;
                    let mut levels = [pairing.opponent; 2];
                    levels[subject_side] = pairing.subject;
                    let outcome = play(
                        pairing.seed(base_seed, round),
                        levels,
                        levels.map(|level| profiles.get(level)),
                        config,
                    );
                    results.lock().unwrap()[job] = Some(outcome);
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if finished.is_multiple_of(20) || finished == jobs.len() {
                        eprintln!(
                            "bot sim: {finished}/{} rounds, {:.0} s",
                            jobs.len(),
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
            });
        }
    });
    let results = results.into_inner().unwrap();
    let mut tallies = vec![Tally::default(); pairings.len()];
    for (&(pairing, round), outcome) in jobs.iter().zip(results) {
        let outcome = outcome.expect("every round ran");
        tallies[pairing].add(&outcome, (round % 2) as usize);
    }
    tallies
}

// ---------------------------------------------------------------------------
// Report (JSON, in the style of `OMOBA_BALANCE_REPORT`)

fn json_f(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.4}")
    } else {
        "null".into()
    }
}

fn json_opt(value: Option<f32>) -> String {
    value.map_or_else(|| "null".into(), |v| json_f(f64::from(v)))
}

fn profile_json(profile: &BotProfile) -> String {
    format!(
        "{{\"retreat_hp\":{},\"resume_hp\":{},\"retreat_secs\":{},\"retreat_cap_secs\":{},\"rearm_secs\":{},\"cover_minions\":{},\"tower_margin\":{},\"staging_margin\":{},\"siege_min_hp\":{},\"hero_engage_range\":{},\"target_leash\":{},\"target_priority\":\"{:?}\",\"trade_margin\":{},\"outnumbered_hold\":{},\"outnumbered_retreat\":{},\"threat_window_secs\":{},\"dash_hero_hp\":{},\"mobility_margin\":{},\"jungle_gank\":\"{:?}\"}}",
        json_f(f64::from(profile.retreat_hp)),
        json_opt(profile.resume_hp),
        json_f(f64::from(profile.retreat_secs)),
        json_opt(profile.retreat_cap_secs),
        json_f(f64::from(profile.rearm_secs)),
        profile.cover_minions,
        json_f(f64::from(profile.tower_margin)),
        json_f(f64::from(profile.staging_margin)),
        json_opt(profile.siege_min_hp),
        json_f(f64::from(profile.hero_engage_range)),
        json_f(f64::from(profile.target_leash)),
        profile.target_priority,
        json_opt(profile.trade_margin),
        profile
            .outnumbered_hold
            .map_or_else(|| "null".into(), |v| v.to_string()),
        profile
            .outnumbered_retreat
            .map_or_else(|| "null".into(), |v| v.to_string()),
        json_opt(profile.threat_window_secs),
        json_opt(profile.dash_hero_hp),
        json_f(f64::from(profile.mobility_margin)),
        profile.jungle_gank,
    )
}

fn retreat_json(side: &Side) -> String {
    let hp = sorted(&side.health_retreats);
    let (low, high) = median_interval(&hp);
    format!(
        "{{\"starts\":{},\"retriggers\":{},\"min\":{},\"p10\":{},\"median\":{},\"median_ci95\":[{},{}],\"max\":{}}}",
        hp.len(),
        side.health_retriggers,
        json_f(quantile(&hp, 0.0)),
        json_f(quantile(&hp, 0.1)),
        json_f(quantile(&hp, 0.5)),
        json_f(low),
        json_f(high),
        json_f(quantile(&hp, 1.0)),
    )
}

fn side_json(side: &Side) -> String {
    format!(
        "{{\"kills_per_bot_min\":{},\"deaths_per_bot_min\":{},\"health_retreat_hp\":{},\"outnumbered_retreats_per_bot_min\":{},\"threat_retreats_per_bot_min\":{},\"threat_retaliations_per_bot_min\":{},\"threat_retreat_causes\":{{\"out_of_range\":{},\"refused\":{},\"refused_by_trade\":{},\"refused_by_outnumbered\":{},\"unreachable\":{}}},\"tower_hits_per_bot_min\":{},\"dives_per_bot_min\":{},\"routes_per_bot_min\":{},\"route_failures_per_bot_min\":{},\"hero_fight_share\":{},\"outnumbered_fight_share\":{},\"structures_per_round\":{},\"structure_score_per_round\":{}}}",
        json_f(side.rate(f64::from(side.kills))),
        json_f(side.rate(f64::from(side.deaths))),
        retreat_json(side),
        json_f(side.rate(f64::from(side.outnumbered_retreats))),
        json_f(side.rate(f64::from(side.threat_retreats))),
        json_f(side.rate(f64::from(side.threat_retaliations))),
        side.threat_out_of_range,
        side.threat_refused,
        side.threat_refused_trade,
        side.threat_refused_outnumbered,
        side.threat_unreachable,
        json_f(side.rate(f64::from(side.tower_hits))),
        json_f(side.rate(f64::from(side.dives))),
        json_f(side.rate(f64::from(side.routes))),
        json_f(side.rate(f64::from(side.route_failures))),
        json_f(if side.seat_secs > 0.0 {
            side.hero_fight_secs / side.seat_secs
        } else {
            0.0
        }),
        json_f(if side.hero_fight_secs > 0.0 {
            side.outnumbered_fight_secs / side.hero_fight_secs
        } else {
            0.0
        }),
        json_f(f64::from(side.structures_destroyed) / f64::from(side.rounds.max(1))),
        json_f(side.structure_score / f64::from(side.rounds.max(1))),
    )
}

/// Mean and normal 95% half-width.
fn mean_interval(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    if values.len() < 2 {
        return (values.first().copied().unwrap_or(f64::NAN), f64::NAN);
    }
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, Z95 * (variance / n).sqrt())
}

fn tally_json(pairing: Pairing, tally: &Tally) -> String {
    let (low, high) = wilson(tally.subject_wins, tally.decisive());
    let (lead, lead_half) = mean_interval(&tally.leads);
    let minutes = sorted(&tally.secs);
    format!(
        "{{\"pairing\":\"{}\",\"rounds\":{},\"subject_wins\":{},\"opponent_wins\":{},\"draws\":{},\"victories\":{},\"subject_win_rate\":{},\"wilson95\":[{},{}],\"draw_rate\":{},\"green_win_rate\":{},\"structure_lead\":{},\"structure_lead_ci95\":{},\"median_minutes\":{},\"max_minutes\":{},\"lost_events\":{},\"subject\":{},\"opponent\":{}}}",
        pairing.name(),
        tally.rounds,
        tally.subject_wins,
        tally.opponent_wins,
        tally.draws,
        tally.victories,
        json_f(f64::from(tally.subject_wins) / f64::from(tally.decisive().max(1))),
        json_f(low),
        json_f(high),
        json_f(f64::from(tally.draws) / f64::from(tally.rounds.max(1))),
        json_f(f64::from(tally.green_wins) / f64::from(tally.decisive().max(1))),
        json_f(lead),
        json_f(lead_half),
        json_f(quantile(&minutes, 0.5) / 60.0),
        json_f(quantile(&minutes, 1.0) / 60.0),
        tally.lost_events,
        side_json(&tally.subject),
        side_json(&tally.opponent),
    )
}

// ---------------------------------------------------------------------------
// Acceptance (docs/plans/bot-personas.md, "Difficulty simulator (S3)")

#[derive(Debug)]
struct Check {
    name: String,
    passed: bool,
    /// Whether `full_matrix` fails on it. Only an infeasible comparison is
    /// reported without gating.
    gating: bool,
    detail: String,
}

fn find(
    pairings: &[Pairing],
    tallies: &[Tally],
    subject: BotDifficulty,
    opponent: BotDifficulty,
) -> Option<Tally> {
    pairings
        .iter()
        .position(|p| p.subject == subject && p.opponent == opponent)
        .map(|i| tallies[i].clone())
}

/// Everything `level` did, on either side of any pairing.
fn level_side(pairings: &[Pairing], tallies: &[Tally], level: BotDifficulty) -> Side {
    let mut side = Side::default();
    for (pairing, tally) in pairings.iter().zip(tallies) {
        if pairing.subject == level {
            side.add(&tally.subject);
        }
        if pairing.opponent == level {
            side.add(&tally.opponent);
        }
    }
    side
}

fn checks(pairings: &[Pairing], tallies: &[Tally], profiles: &Profiles) -> Vec<Check> {
    use BotDifficulty::*;
    let mut checks = Vec::new();
    if let Some(tally) = find(pairings, tallies, Elite, Elite) {
        let (low, high) = wilson(tally.subject_wins, tally.decisive());
        checks.push(Check {
            gating: true,
            name: "elite_vs_elite_contains_half".into(),
            passed: low <= 0.5 && 0.5 <= high,
            detail: format!("Wilson95 [{low:.3}, {high:.3}]"),
        });
    }
    // Win rates against Elite are ordered, adjacent levels separated.
    let ladder = [Recruit, Regular, Veteran, Elite];
    let against_elite: Vec<_> = ladder
        .iter()
        .map(|level| find(pairings, tallies, *level, Elite))
        .collect();
    for pair in 0..3 {
        let (Some(low), Some(high)) = (&against_elite[pair], &against_elite[pair + 1]) else {
            continue;
        };
        let p = p_greater(
            low.subject_wins,
            low.decisive(),
            high.subject_wins,
            high.decisive(),
        );
        let rate = |t: &Tally| f64::from(t.subject_wins) / f64::from(t.decisive().max(1));
        // Two levels that both lose nearly every round to Elite cannot be
        // ordered by that win rate; the lead and the direct pairing do it.
        let hopeless = |t: &Tally| wilson(t.subject_wins, t.decisive()).1 < 0.10;
        let feasible = !(hopeless(low) && hopeless(high));
        checks.push(Check {
            gating: feasible,
            name: format!(
                "vs_elite_{}_below_{}",
                ladder[pair].name(),
                ladder[pair + 1].name()
            ),
            passed: rate(low) < rate(high) && p < 0.05,
            detail: format!(
                "{:.3} < {:.3}, one-sided p = {p:.4}{}",
                rate(low),
                rate(high),
                if feasible {
                    ""
                } else {
                    "; both lose nearly every round to Elite (Wilson upper < 0.10), so this is not gating"
                }
            ),
        });
    }
    // The margin against Elite, which still orders levels that both lose
    // nearly every round: mean structure leads with disjoint 95% intervals.
    for pair in 0..3 {
        let (Some(low), Some(high)) = (&against_elite[pair], &against_elite[pair + 1]) else {
            continue;
        };
        let (a, a_half) = mean_interval(&low.leads);
        let (b, b_half) = mean_interval(&high.leads);
        checks.push(Check {
            gating: true,
            name: format!(
                "vs_elite_lead_{}_below_{}",
                ladder[pair].name(),
                ladder[pair + 1].name()
            ),
            passed: a + a_half < b - b_half,
            detail: format!("{a:.2} ± {a_half:.2} < {b:.2} ± {b_half:.2} structures"),
        });
    }
    // Direct adjacent pairings: the stronger level wins more than half.
    for pair in 0..3 {
        if let Some(tally) = find(pairings, tallies, ladder[pair], ladder[pair + 1]) {
            let (low, high) = wilson(tally.subject_wins, tally.decisive());
            checks.push(Check {
                gating: true,
                name: format!(
                    "{}_loses_to_{}",
                    ladder[pair].name(),
                    ladder[pair + 1].name()
                ),
                passed: high < 0.5,
                detail: format!("subject Wilson95 [{low:.3}, {high:.3}]"),
            });
        }
    }
    for (pairing, tally) in pairings.iter().zip(tallies) {
        let rate = f64::from(tally.draws) / f64::from(tally.rounds.max(1));
        checks.push(Check {
            gating: true,
            name: format!("draws_at_most_10pct_{}", pairing.name()),
            passed: rate <= 0.10,
            detail: format!("{} of {} rounds", tally.draws, tally.rounds),
        });
    }
    // Health retreats begin below the level's line, the median within 0.10.
    for level in ladder {
        let side = level_side(pairings, tallies, level);
        if side.health_retreats.is_empty() {
            continue;
        }
        let line = profiles.get(level).retreat_hp;
        let hp = sorted(&side.health_retreats);
        let median = quantile(&hp, 0.5);
        checks.push(Check {
            gating: true,
            name: format!("health_retreats_{}", level.name()),
            passed: hp.iter().all(|h| *h < line) && median >= f64::from(line) - 0.10,
            detail: format!(
                "{} starts, max {:.4} < {line}, median {median:.3} >= {:.2}",
                hp.len(),
                hp[hp.len() - 1],
                line - 0.10
            ),
        });
    }
    // The route-planning CPU budget (docs/plans/launch-readiness.md, LR-10).
    for level in ladder {
        let side = level_side(pairings, tallies, level);
        if side.seat_secs <= 0.0 {
            continue;
        }
        let rate = side.rate(f64::from(side.routes));
        checks.push(Check {
            gating: true,
            name: format!("routes_within_budget_{}", level.name()),
            passed: rate <= ROUTE_BUDGET,
            detail: format!(
                "{rate:.1} plans per bot-minute <= {ROUTE_BUDGET}, {:.2} found no route",
                side.rate(f64::from(side.route_failures))
            ),
        });
    }
    checks
}

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| {
            v.trim()
                .parse()
                .unwrap_or_else(|_| panic!("{name}: cannot parse {v:?}"))
        })
        .unwrap_or(default)
}

/// `subject:opponent` items separated by commas; `subject:opponent@N`
/// gives that pairing its own number of rounds.
fn parse_pairings(spec: &str) -> Vec<(Pairing, Option<u32>)> {
    spec.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|item| {
            let (item, rounds) = match item.split_once('@') {
                Some((item, rounds)) => (
                    item,
                    Some(
                        rounds
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("{rounds}: not a round count")),
                    ),
                ),
                None => (item, None),
            };
            let (subject, opponent) = item
                .split_once(':')
                .unwrap_or_else(|| panic!("{item}: expected subject:opponent"));
            let level = |name: &str| {
                BotDifficulty::parse(name).unwrap_or_else(|| panic!("{name}: not a level"))
            };
            let pairing = Pairing {
                subject: level(subject.trim()),
                opponent: level(opponent.trim()),
            };
            (pairing, rounds)
        })
        .collect()
}

#[test]
#[ignore = "full matrix: minutes to hours; see the module docs"]
fn full_matrix() {
    let rounds: u32 = env("OMOBA_BOT_SIM_MATCHES", 200);
    let threads: usize = env(
        "OMOBA_BOT_SIM_THREADS",
        std::thread::available_parallelism().map_or(1, usize::from),
    );
    let base_seed: u64 = env("OMOBA_BOT_SIM_SEED", 0);
    let defaults = Config::default();
    let config = Config {
        dt: env("OMOBA_BOT_SIM_DT", defaults.dt),
        jitter: env("OMOBA_BOT_SIM_JITTER", defaults.jitter),
        cap_secs: env("OMOBA_BOT_SIM_CAP_MIN", defaults.cap_secs / 60.0) * 60.0,
    };
    let tune = std::env::var("OMOBA_BOT_SIM_TUNE").unwrap_or_default();
    let profiles = Profiles::tuned(&tune).unwrap_or_else(|e| panic!("OMOBA_BOT_SIM_TUNE: {e}"));
    let plan: Vec<(Pairing, u32)> = std::env::var("OMOBA_BOT_SIM_PAIRINGS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map_or_else(
            || default_pairings().into_iter().map(|p| (p, None)).collect(),
            |v| parse_pairings(&v),
        )
        .into_iter()
        .map(|(pairing, n)| (pairing, n.unwrap_or(rounds)))
        .collect();
    let pairings: Vec<Pairing> = plan.iter().map(|(p, _)| *p).collect();
    let counts: Vec<u32> = plan.iter().map(|(_, n)| *n).collect();
    let started = Instant::now();
    let tallies = run_matrix(&pairings, &counts, base_seed, profiles, config, threads);
    let checks = checks(&pairings, &tallies, &profiles);
    let report = format!(
        "{{\n\"version\":\"{}\",\n\"setup\":{{\"default_rounds_per_pairing\":{rounds},\"base_seed\":{base_seed},\"dt\":{},\"jitter\":{},\"cap_minutes\":{},\"capped_rounds\":\"adjudicated by enemy structure health removed; a lead under {DRAW_MARGIN} structures is a draw\",\"roster\":\"standard practice fill\",\"sides\":\"subject Green in even rounds, Blue in odd\",\"tune\":\"{}\",\"wall_secs\":{:.0}}},\n\"profiles\":{{{}}},\n\"levels\":{{{}}},\n\"pairings\":[\n{}\n],\n\"checks\":[\n{}\n]\n}}\n",
        env!("CARGO_PKG_VERSION"),
        json_f(f64::from(config.dt)),
        json_f(f64::from(config.jitter)),
        json_f(f64::from(config.cap_secs / 60.0)),
        tune.replace('"', "'"),
        started.elapsed().as_secs_f64(),
        BotDifficulty::ALL
            .iter()
            .map(|level| format!(
                "\"{}\":{}",
                level.name(),
                profile_json(&profiles.get(*level))
            ))
            .collect::<Vec<_>>()
            .join(","),
        // Each level pooled over every pairing it played, on either side.
        BotDifficulty::ALL
            .iter()
            .map(|level| format!(
                "\n\"{}\":{}",
                level.name(),
                side_json(&level_side(&pairings, &tallies, *level))
            ))
            .collect::<Vec<_>>()
            .join(","),
        pairings
            .iter()
            .zip(&tallies)
            .map(|(pairing, tally)| tally_json(*pairing, tally))
            .collect::<Vec<_>>()
            .join(",\n"),
        checks
            .iter()
            .map(|c| format!(
                "{{\"check\":\"{}\",\"passed\":{},\"gating\":{},\"detail\":\"{}\"}}",
                c.name, c.passed, c.gating, c.detail
            ))
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    // The game logs to stdout; the report goes to stderr and the file.
    eprintln!("{report}");
    if let Ok(path) = std::env::var("OMOBA_BOT_SIM_REPORT") {
        std::fs::write(&path, &report).expect("write the simulator report");
    }
    let failed: Vec<_> = checks.iter().filter(|c| c.gating && !c.passed).collect();
    if env("OMOBA_BOT_SIM_ASSERT", 1_u8) == 1 {
        assert!(failed.is_empty(), "acceptance checks failed: {failed:#?}");
    }
}

// ---------------------------------------------------------------------------
// CI quick checks: small samples, interval assertions only.

#[test]
fn statistics_helpers_match_reference_values() {
    let (low, high) = wilson(8, 10);
    assert!(
        (low - 0.4902).abs() < 1e-3 && (high - 0.9433).abs() < 1e-3,
        "{low} {high}"
    );
    let (low, high) = wilson(0, 10);
    assert!(low == 0.0 && (high - 0.2775).abs() < 1e-3, "{high}");
    assert!((erfc(0.0) - 1.0).abs() < 1e-7);
    assert!((erfc(1.0) - 0.157_299_2).abs() < 1e-6);
    // 30/100 against 45/100: z = 2.19, one-sided p = 0.0142.
    let p = p_greater(30, 100, 45, 100);
    assert!((p - 0.0142).abs() < 1e-3, "{p}");
    let values: Vec<f32> = (0..100).map(|i| i as f32).collect();
    let (low, high) = median_interval(&values);
    assert!(
        (39.0..=41.0).contains(&low) && (59.0..=61.0).contains(&high),
        "{low} {high}"
    );
}

#[test]
fn a_round_replays_exactly_from_its_seed() {
    let config = Config {
        cap_secs: 60.0,
        ..Config::default()
    };
    let levels = [BotDifficulty::Recruit, BotDifficulty::Elite];
    let profiles = levels.map(BotProfile::for_level);
    let first = play(7, levels, profiles, config);
    let second = play(7, levels, profiles, config);
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    let other = play(8, levels, profiles, config);
    assert_ne!(
        format!("{first:?}"),
        format!("{other:?}"),
        "another seed plays another round"
    );
}

/// A CI smoke sample: `rounds` fixed-seed rounds of each pairing with the
/// shipped profiles, capped at `cap_min` game minutes.
fn smoke(pairings: &[Pairing], rounds: u32, cap_min: f32) -> Vec<Tally> {
    let config = Config {
        cap_secs: cap_min * 60.0,
        ..Config::default()
    };
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    run_matrix(
        pairings,
        &vec![rounds; pairings.len()],
        0,
        Profiles::standard(),
        config,
        threads,
    )
}

#[test]
#[ignore = "CI smoke (the bot-sim job runs it in release): `-- --ignored smoke_`"]
fn smoke_health_retreats_start_below_each_levels_line() {
    // Recruit against Regular and Veteran against Elite: all four levels.
    let pairings: Vec<_> = parse_pairings("recruit:regular,veteran:elite")
        .into_iter()
        .map(|(pairing, _)| pairing)
        .collect();
    let tallies = smoke(&pairings, 2, 8.0);
    for level in BotDifficulty::ALL {
        let line = BotProfile::for_level(level).retreat_hp;
        let hp = sorted(&level_side(&pairings, &tallies, level).health_retreats);
        assert!(
            hp.len() >= 20,
            "{level:?}: only {} health retreats",
            hp.len()
        );
        let highest = hp[hp.len() - 1];
        assert!(
            highest < line,
            "{level:?}: a health retreat began at {highest} >= {line}"
        );
        // The 95% interval of the median sits within 0.10 of the line.
        let (low, high) = median_interval(&hp);
        eprintln!(
            "{level:?}: {} health retreats below {line}, median in [{low:.3}, {high:.3}], highest {highest:.3}",
            hp.len()
        );
        assert!(
            low >= f64::from(line) - 0.10,
            "{level:?}: median health at retreat in [{low:.3}, {high:.3}], line {line}"
        );
    }
}

#[test]
#[ignore = "CI smoke (the bot-sim job runs it in release): `-- --ignored smoke_`"]
fn smoke_elite_outplays_recruit() {
    let pairing = Pairing {
        subject: BotDifficulty::Recruit,
        opponent: BotDifficulty::Elite,
    };
    let tally = &smoke(&[pairing], 8, 12.0)[0];
    let (low, high) = wilson(tally.subject_wins, tally.decisive());
    eprintln!(
        "Recruit against Elite: {}-{}-{}, Wilson95 [{low:.3}, {high:.3}]",
        tally.subject_wins, tally.opponent_wins, tally.draws
    );
    assert!(tally.decisive() >= 6, "{tally:?}");
    assert!(
        high < 0.5,
        "Recruit won {} of {} decisive rounds against Elite, Wilson95 [{low:.3}, {high:.3}]",
        tally.subject_wins,
        tally.decisive()
    );
}
