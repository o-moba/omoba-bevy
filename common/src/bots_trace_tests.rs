//! Elite trace equivalence (docs/plans/bot-personas.md, S1).
//!
//! Two human-less 5v5 practice rounds run on an explicit 50 ms clock:
//!
//! - `roster`: the standard practice fill (legacy kits, three lanes and a
//!   jungler);
//! - `mid`: ten advanced kits on one lane, so point casts, aim points and the
//!   mobility withholding near towers are exercised as well.
//!
//! Every tick records each bot's commands in issue order (`command_trace`),
//! then its position, facing, health, level, gold, target and retreat state.
//! The trace is folded into one FNV-1a digest per simulated second.
//!
//! - Each round runs twice in one process. Each `GameWorld` gets its own
//!   `HashMap` keys, so equal traces show the scenario does not depend on
//!   iteration order.
//! - The digests must equal the baseline recorded from the pre-refactor brain
//!   (`origin/dev` `ce04e5f` plus this test only), so the Elite profile is
//!   today's behaviour exactly. Neither round has a dagger class, the only
//!   gameplay code that draws randomness.
//!
//! Positions come from `sin`, `cos`, `atan2` and `hypot`, which the platform
//! `libm` computes, and those last bits differ between C libraries and some
//! glibc releases. A baseline is therefore keyed by a fingerprint of those
//! functions. Without a matching baseline the comparison is skipped with a
//! notice, unless `OMOBA_REQUIRE_BOT_TRACE=1`. The determinism check runs
//! everywhere.
//!
//! Recording a baseline for a new `libm` is only valid on the pre-refactor
//! brain: check out the baseline commit, then run
//! `OMOBA_BOT_TRACE_OUT=<file> OMOBA_BOT_TRACE_LABEL=<label> cargo test -p common elite_command_trace`
//! and append the file to the fixture.
use super::*;
use crate::{balance, offline::PracticeSession};

/// Five simulated minutes: the first wave meets mid, towers are pressed and
/// heroes trade, retreat and die.
const TICKS: usize = 6_000;
const DT: f32 = 0.05;
const TICKS_PER_DIGEST: usize = 20;
const FIXTURE: &str = include_str!("../tests/fixtures/elite_bot_trace.txt");

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
fn fnv(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// The `libm` results the simulation depends on, over the coordinate and
/// angle ranges of the arena.
fn libm_fingerprint() -> u64 {
    let mut hash = FNV_OFFSET;
    let mut state = 0x2545_f491_u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    for _ in 0..65_536 {
        let a = next() * 240.0 - 120.0;
        let b = next() * 240.0 - 120.0;
        let angle = next() * 16.0 - 8.0;
        for value in [
            angle.sin(),
            angle.cos(),
            b.atan2(a),
            a.hypot(b),
            (a * 0.01).hypot(b * 0.01),
        ] {
            fnv(&mut hash, &value.to_bits().to_le_bytes());
        }
    }
    hash
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Roster,
    Mid,
}

impl Scenario {
    const ALL: [Self; 2] = [Self::Roster, Self::Mid];

    fn name(self) -> &'static str {
        match self {
            Self::Roster => "roster",
            Self::Mid => "mid",
        }
    }
}

/// Advanced kits for the one-lane round, chosen to cover every aim branch
/// (self-aimed leaps and guards, clamped point casts) and most mobility
/// techniques. Explicit classes take the mid lane without jungling.
const MID_KITS: [(Team, HeroClass); 10] = [
    (Team::Green, HeroClass::Edgeweaver),
    (Team::Green, HeroClass::Stormfist),
    (Team::Green, HeroClass::Riftshot),
    (Team::Green, HeroClass::Wildspark),
    (Team::Green, HeroClass::Orbitwright),
    (Team::Blue, HeroClass::Cinderforge),
    (Team::Blue, HeroClass::Veilstalker),
    (Team::Blue, HeroClass::Chainkeeper),
    (Team::Blue, HeroClass::Frostguard),
    (Team::Blue, HeroClass::Emberveil),
];

/// The world setup `Join` performs for offline practice (Running, the first
/// wave timer, the boss schedule and the ledger), then the bots.
fn scripted_match(scenario: Scenario) -> PracticeSession {
    let start = Instant::now();
    let mut practice = PracticeSession::new(start);
    practice.world.game_state = GameState::Running;
    practice.world.last_wave_spawn_at =
        start - (balance::MINION_WAVE_INTERVAL - balance::FIRST_MINION_WAVE_DELAY);
    crate::neutrals::schedule_boss_spawns(&mut practice.world.neutrals, start);
    practice
        .combat_log
        .ledger
        .begin(Vec::new())
        .expect("fresh practice ledger");
    match scenario {
        Scenario::Roster => practice.host().fill_practice_bots(start),
        Scenario::Mid => {
            for (team, class) in MID_KITS {
                practice
                    .host()
                    .spawn_bot(team, Some(class), BotKind::Lane, start)
                    .expect("a free bot slot");
            }
        }
    }
    practice
}

fn target_code(target: Option<TargetId>) -> String {
    target.map_or_else(
        || "-".into(),
        |target| {
            let kind = match target.kind {
                TargetKind::Player => 'P',
                TargetKind::Minion => 'M',
                TargetKind::Structure => 'S',
                TargetKind::Neutral => 'N',
            };
            format!("{kind}{}", target.id)
        },
    )
}

#[derive(Default, PartialEq, Debug)]
struct Summary {
    casts: u32,
    unit_casts: u32,
    sustains: u32,
    attacks: u32,
    retreat_starts: u32,
    deaths: u32,
    hero_targets: u32,
    minion_targets: u32,
    structure_targets: u32,
    neutral_targets: u32,
    max_level: u32,
    structures_down: u32,
}

impl Summary {
    fn line(&self) -> String {
        format!(
            "summary casts={} unit_casts={} sustains={} attacks={} retreat_starts={} deaths={} hero_targets={} minion_targets={} structure_targets={} neutral_targets={} max_level={} structures_down={}",
            self.casts,
            self.unit_casts,
            self.sustains,
            self.attacks,
            self.retreat_starts,
            self.deaths,
            self.hero_targets,
            self.minion_targets,
            self.structure_targets,
            self.neutral_targets,
            self.max_level,
            self.structures_down,
        )
    }
}

struct Trace {
    /// One text block per tick: the commands, then one state line per bot.
    ticks: Vec<String>,
    digests: Vec<u64>,
    summary: Summary,
}

fn run_trace(scenario: Scenario) -> Trace {
    let mut practice = scripted_match(scenario);
    let bots = practice.bots.controllers.len();
    assert_eq!(bots, 10, "{scenario:?} is a full 5v5 roster");
    let structures = practice.world.structures.len();
    let mut ticks = Vec::with_capacity(TICKS);
    let mut summary = Summary::default();
    let mut previous: HashMap<SocketAddr, (bool, bool, Option<TargetId>)> = HashMap::new();
    command_trace::start();
    for _ in 0..TICKS {
        practice.advance(DT);
        let now = practice.now;
        let mut block = String::new();
        for command in command_trace::drain() {
            match command.split(' ').nth(1) {
                Some("cast") => summary.casts += 1,
                Some("unit") => summary.unit_casts += 1,
                Some("sustain") => summary.sustains += 1,
                Some("attack") => summary.attacks += 1,
                other => panic!("unknown command kind {other:?}"),
            }
            block.push_str(&command);
            block.push('\n');
        }
        let mut addresses: Vec<_> = practice.bots.controllers.keys().copied().collect();
        addresses.sort_unstable();
        for addr in addresses {
            let controller = &practice.bots.controllers[&addr];
            let player = &practice.world.players[&addr];
            let hero = &player.hero;
            let retreating = controller.retreat_until.is_some_and(|until| now < until);
            let alive = hero.hp > 0.0;
            let (was_retreating, was_alive, previous_target) =
                previous.get(&addr).copied().unwrap_or((false, true, None));
            summary.retreat_starts += u32::from(retreating && !was_retreating);
            summary.deaths += u32::from(was_alive && !alive);
            if controller.target != previous_target
                && let Some(target) = controller.target
            {
                *match target.kind {
                    TargetKind::Player => &mut summary.hero_targets,
                    TargetKind::Minion => &mut summary.minion_targets,
                    TargetKind::Structure => &mut summary.structure_targets,
                    TargetKind::Neutral => &mut summary.neutral_targets,
                } += 1;
            }
            summary.max_level = summary.max_level.max(hero.progress.level);
            previous.insert(addr, (retreating, alive, controller.target));
            block.push_str(&format!(
                "{} x={:?} z={:?} yaw={:?} hp={:?} lvl={} gold={} target={} retreat={}\n",
                addr.port(),
                hero.x,
                hero.z,
                hero.yaw,
                hero.hp,
                hero.progress.level,
                player.economy.gold,
                target_code(controller.target),
                u8::from(retreating),
            ));
        }
        ticks.push(block);
    }
    command_trace::stop();
    summary.structures_down = practice
        .world
        .structures
        .values()
        .filter(|s| s.state.hp <= 0.0)
        .count() as u32
        + (structures - practice.world.structures.len()) as u32;
    let digests = ticks
        .chunks(TICKS_PER_DIGEST)
        .map(|chunk| {
            let mut hash = FNV_OFFSET;
            for block in chunk {
                fnv(&mut hash, block.as_bytes());
            }
            hash
        })
        .collect();
    Trace {
        ticks,
        digests,
        summary,
    }
}

/// The ticks of the first differing digest window, for a failure message.
fn window(trace: &Trace, second: usize) -> String {
    trace
        .ticks
        .iter()
        .enumerate()
        .skip(second * TICKS_PER_DIGEST)
        .take(TICKS_PER_DIGEST)
        .map(|(tick, block)| format!("tick {tick}\n{block}"))
        .collect()
}

struct Baseline {
    libm: u64,
    label: String,
    /// (scenario, summary line, digests)
    scenarios: Vec<(String, String, Vec<u64>)>,
}

fn last_scenario(found: &mut [Baseline]) -> &mut (String, String, Vec<u64>) {
    found
        .last_mut()
        .and_then(|baseline| baseline.scenarios.last_mut())
        .expect("a scenario line first")
}

fn baselines() -> Vec<Baseline> {
    let mut found: Vec<Baseline> = Vec::new();
    for line in FIXTURE.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "baseline" => {
                let (libm, label) = rest.split_once(' ').unwrap_or((rest, ""));
                found.push(Baseline {
                    libm: u64::from_str_radix(libm, 16).expect("libm fingerprint"),
                    label: label.into(),
                    scenarios: Vec::new(),
                });
            }
            "scenario" => found
                .last_mut()
                .expect("a baseline line first")
                .scenarios
                .push((rest.into(), String::new(), Vec::new())),
            "summary" => last_scenario(&mut found).1 = line.into(),
            "digest" => last_scenario(&mut found)
                .2
                .push(u64::from_str_radix(rest, 16).expect("digest")),
            other => panic!("unknown fixture line {other:?}"),
        }
    }
    found
}

fn baseline_text(traces: &[(Scenario, Trace)], libm: u64, label: &str) -> String {
    let mut text = format!("baseline {libm:016x} {label}\n");
    for (scenario, trace) in traces {
        text.push_str(&format!(
            "scenario {}\n{}\n",
            scenario.name(),
            trace.summary.line()
        ));
        for digest in &trace.digests {
            text.push_str(&format!("digest {digest:016x}\n"));
        }
    }
    text
}

/// Both runs of one scenario, proven equal.
fn deterministic_trace(scenario: Scenario) -> Trace {
    let first = run_trace(scenario);
    let second = run_trace(scenario);
    if let Some(at) = (0..first.digests.len()).find(|i| first.digests[*i] != second.digests[*i]) {
        panic!(
            "{scenario:?} is not deterministic from second {at}:\n--- first run\n{}\n--- second run\n{}",
            window(&first, at),
            window(&second, at)
        );
    }
    assert_eq!(first.summary, second.summary, "{scenario:?}");
    // Each round must exercise the knobs the profile replaces.
    let summary = &first.summary;
    match scenario {
        Scenario::Roster => {
            assert!(summary.retreat_starts > 0, "{summary:?}");
            assert!(summary.deaths > 0, "{summary:?}");
            assert!(summary.hero_targets > 0, "{summary:?}");
            assert!(summary.structure_targets > 0, "{summary:?}");
            assert!(summary.neutral_targets > 0, "{summary:?}");
            assert!(summary.sustains > 0, "{summary:?}");
        }
        Scenario::Mid => {
            assert!(summary.casts > 0, "{summary:?}");
            assert!(summary.hero_targets > 0, "{summary:?}");
            assert!(summary.deaths > 0, "{summary:?}");
        }
    }
    first
}

#[test]
fn elite_command_trace_is_deterministic_and_matches_the_pre_refactor_baseline() {
    let traces: Vec<_> = Scenario::ALL
        .into_iter()
        .map(|scenario| (scenario, deterministic_trace(scenario)))
        .collect();
    let libm = libm_fingerprint();
    if let Ok(path) = std::env::var("OMOBA_BOT_TRACE_OUT") {
        let label = std::env::var("OMOBA_BOT_TRACE_LABEL").unwrap_or_else(|_| "unlabelled".into());
        std::fs::write(&path, baseline_text(&traces, libm, &label)).expect("write trace baseline");
    }
    let baselines = baselines();
    let Some(baseline) = baselines.iter().find(|b| b.libm == libm) else {
        assert!(
            std::env::var("OMOBA_REQUIRE_BOT_TRACE").as_deref() != Ok("1"),
            "no Elite trace baseline for libm fingerprint {libm:016x}"
        );
        eprintln!(
            "Elite trace: no baseline for libm fingerprint {libm:016x}; determinism checked, equivalence skipped"
        );
        return;
    };
    assert_eq!(baseline.scenarios.len(), traces.len(), "{}", baseline.label);
    for ((scenario, trace), (name, summary, digests)) in traces.iter().zip(&baseline.scenarios) {
        assert_eq!(name, scenario.name(), "{}", baseline.label);
        assert_eq!(
            summary,
            &trace.summary.line(),
            "Elite {name} summary differs from the {} baseline",
            baseline.label
        );
        assert_eq!(digests.len(), trace.digests.len(), "{name}");
        if let Some(at) = (0..digests.len()).find(|i| digests[*i] != trace.digests[*i]) {
            panic!(
                "Elite {name} behaviour differs from the {} baseline from second {at}:\n{}",
                baseline.label,
                window(trace, at)
            );
        }
    }
}
