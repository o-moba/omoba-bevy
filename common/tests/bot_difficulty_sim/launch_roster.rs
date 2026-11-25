//! Compare launch candidates in the same duty with four mirrored teammates.
use super::{
    BotDifficulty, BotProfile, Config, Instant, Outcome, PracticeSession, env, fnv, json_f,
    play_practice, wilson,
};
use shared::{HeroClass, prematch::Role};

const CANDIDATES: [[HeroClass; 2]; 5] = {
    use HeroClass::*;
    [
        [Warrior, Cinderforge],
        [Warden, Stormfist],
        [Mage, Orbitwright],
        [Ranger, Wildspark],
        [Chainkeeper, Frostguard],
    ]
};

// Each adjacent pair uses complementary teammates. The first 16 trials
// cover every combination of the other four duties.
const BACKGROUNDS: [usize; 16] = [0, 15, 5, 10, 3, 12, 6, 9, 1, 14, 4, 11, 2, 13, 7, 8];

fn teams(duty: usize, pair: usize) -> [[HeroClass; 5]; 2] {
    let background = BACKGROUNDS[pair % BACKGROUNDS.len()];
    std::array::from_fn(|candidate| {
        std::array::from_fn(|index| {
            let choice = if index == duty {
                candidate
            } else {
                let bit = index - usize::from(index > duty);
                background >> bit & 1
            };
            CANDIDATES[index][choice]
        })
    })
}

fn play(rosters: [[HeroClass; 5]; 2], seed: u64, config: Config) -> Outcome {
    let practice =
        PracticeSession::bots_with_rosters(Instant::now(), [BotDifficulty::Elite; 2], rosters);
    play_practice(practice, seed, [BotProfile::ELITE; 2], config)
}

fn game_json(outcome: &Outcome) -> String {
    let winner = match outcome.winner {
        Some(0) => "\"green\"",
        Some(1) => "\"blue\"",
        None => "null",
        Some(_) => unreachable!("a 5v5 has only two sides"),
    };
    format!(
        "{{\"winner\":{winner},\"base_destroyed\":{},\"seconds\":{},\"structure_score\":[{},{}],\"kills\":[{},{}],\"lost_events\":{}}}",
        outcome.victory,
        json_f(outcome.secs.into()),
        json_f(outcome.sides[0].structure_score),
        json_f(outcome.sides[1].structure_score),
        outcome.sides[0].kills,
        outcome.sides[1].kills,
        outcome.lost_events,
    )
}

fn compare(duty: usize, pairs: usize, base_seed: u64, config: Config) -> String {
    let role = Role::ALL[duty];
    let [first, second] = CANDIDATES[duty];
    let mut game_wins = [0; 2];
    let mut pair_wins = [0; 2];
    let mut base_wins = [0; 2];
    let mut base_pair_wins = [0; 2];
    let mut margin = 0.0;
    let mut samples = Vec::with_capacity(pairs);
    for pair in 0..pairs {
        let seed = fnv(&format!("{}#{pair}", role.label())) ^ base_seed;
        let [first_team, second_team] = teams(duty, pair);
        let games = [
            play([first_team, second_team], seed, config),
            play([second_team, first_team], seed, config),
        ];
        let mut pair_result = [0; 2];
        let mut base_pair_result = [0; 2];
        for (first_side, game) in games.iter().enumerate() {
            margin +=
                game.sides[first_side].structure_score - game.sides[1 - first_side].structure_score;
            if let Some(winner) = game.winner {
                let candidate = usize::from(winner != first_side);
                let base_win = u32::from(game.victory);
                game_wins[candidate] += 1;
                pair_result[candidate] += 1;
                base_wins[candidate] += base_win;
                base_pair_result[candidate] += base_win;
            }
        }
        if pair_result[0] != pair_result[1] {
            pair_wins[usize::from(pair_result[1] > pair_result[0])] += 1;
        }
        if base_pair_result[0] != base_pair_result[1] {
            base_pair_wins[usize::from(base_pair_result[1] > base_pair_result[0])] += 1;
        }
        samples.push(format!(
            "{{\"seed\":\"{seed:016x}\",\"first_game_lineups\":{:?},\"first_candidate_green\":{},\"first_candidate_blue\":{}}}",
            [first_team, second_team].map(|team| team.map(HeroClass::id)),
            game_json(&games[0]),
            game_json(&games[1]),
        ));
    }
    let decisive = pair_wins.iter().sum::<u32>();
    let (low, high) = wilson(pair_wins[0], decisive);
    let base_decisive = base_pair_wins.iter().sum::<u32>();
    let (base_low, base_high) = wilson(base_pair_wins[0], base_decisive);
    eprintln!("{}: finished {pairs} side-swapped pairs", role.label());
    format!(
        concat!(
            "{{\"duty\":{:?},\"candidates\":{:?},\"game_wins\":{:?},\"base_wins\":{:?},",
            "\"pair_wins\":{:?},\"tied_pairs\":{},\"first_candidate_decisive_pair_band_95\":[{},{}],",
            "\"base_pair_wins\":{:?},\"first_candidate_decisive_base_pair_band_95\":[{},{}],",
            "\"first_candidate_mean_structure_margin\":{},\"samples\":[{}]}}"
        ),
        role.label(),
        [first.id(), second.id()],
        game_wins,
        base_wins,
        pair_wins,
        pairs - decisive as usize,
        json_f(low),
        json_f(high),
        base_pair_wins,
        json_f(base_low),
        json_f(base_high),
        json_f(margin / (2 * pairs) as f64),
        samples.join(","),
    )
}

#[test]
#[ignore = "full 5v5 roster simulation; run explicitly"]
fn launch_roster_matrix() {
    let pairs = env("OMOBA_LAUNCH_SIM_PAIRS", 16_usize);
    let base_seed = env("OMOBA_LAUNCH_SIM_SEED", 1_u64);
    let cap_minutes = env("OMOBA_LAUNCH_SIM_CAP_MIN", 45.0_f32);
    assert!((1..=128).contains(&pairs));
    assert!((0.25..=45.0).contains(&cap_minutes));
    let config = Config {
        cap_secs: cap_minutes * 60.0,
        ..Config::default()
    };
    let results = std::thread::scope(|scope| {
        std::array::from_fn::<_, 5, _>(|duty| {
            scope.spawn(move || compare(duty, pairs, base_seed, config))
        })
        .map(|job| job.join().expect("roster simulation thread"))
    });
    let report = format!(
        concat!(
            "{{\"schema\":1,\"gameplay\":{:?},\"base_seed\":\"{:016x}\",",
            "\"pairs_per_duty\":{},\"cap_minutes\":{},\"nominal_tick_secs\":{},",
            "\"relative_tick_jitter\":{},\"structure_draw_margin\":{},\"duties\":[{}]}}\n"
        ),
        shared::compatibility::GAMEPLAY_REVISION,
        base_seed,
        pairs,
        json_f(cap_minutes.into()),
        json_f(config.dt.into()),
        json_f(config.jitter.into()),
        json_f(super::DRAW_MARGIN),
        results.join(","),
    );
    if let Some(path) = std::env::var_os("OMOBA_LAUNCH_SIM_REPORT") {
        std::fs::write(path, report).expect("write roster simulation report");
    } else {
        eprintln!("{report}");
    }
}

#[test]
fn launch_roster_changes_one_duty_and_covers_all_teammate_combinations() {
    for (duty, role) in Role::ALL.into_iter().enumerate() {
        let mut backgrounds = Vec::new();
        for pair in 0..BACKGROUNDS.len() {
            let [first, second] = teams(duty, pair);
            assert_eq!(first[duty].primary_role(), role);
            assert_eq!(second[duty].primary_role(), role);
            assert_ne!(first[duty], second[duty]);
            let allies: Vec<_> = (0..first.len())
                .filter(|&index| index != duty)
                .map(|index| {
                    assert_eq!(first[index], second[index]);
                    first[index]
                })
                .collect();
            assert!(!backgrounds.contains(&allies));
            backgrounds.push(allies);
        }
    }
}
