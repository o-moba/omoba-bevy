//! Fill bots: joins N dummy players to a running server over real UDP so one
//! developer can form a full 5v5 match locally and walk through the whole
//! matchmaking flow from the client's point of view.
//!
//! Before the match starts the bots idle near their spawn; once the match is
//! `Running` each bot plays its lane (TASK-23, `harness::bot_ai`): it walks
//! Mid/Top/Bot toward the enemy base, fights enemy players and minions it
//! meets with Q, sieges towers in reach, and rejoins its lane after a
//! respawn. The server stays authoritative for movement speed, cast range,
//! cooldowns, and damage — the bots are ordinary UDP clients.
//!
//! Usage:
//!   cargo run -p harness --bin bots -- [--count N] [--server ADDR]
//!
//! Defaults: `--count 9` (one human + nine bots = 5v5), `--server
//! 127.0.0.1:4000`. In release mode the server assigns teams, so the team
//! sent here is only a preference. Runs until killed (Ctrl+C / `make stop`).

use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use harness::{
    Bot, Character, GameState, HeroClass, Team,
    bot_ai::{
        BotBrain, Lane, WorldView, can_cast_slot, choose_self_sustain, choose_skill_upgrade,
        step_toward,
    },
};

const TICK_INTERVAL: Duration = Duration::from_millis(100);
const STATUS_INTERVAL: Duration = Duration::from_secs(2);
/// Bound upgrade requests while waiting for the authoritative rank update.
const UPGRADE_INTERVAL: Duration = Duration::from_millis(250);

const CLASSES: [HeroClass; 4] = [
    HeroClass::Warrior,
    HeroClass::Mage,
    HeroClass::Ranger,
    HeroClass::Cleric,
];
const AVATARS: [Option<&str>; 6] = [
    Some("agnes"),
    Some("crowley"),
    Some("pirate-bot"),
    Some("stitch-witch"),
    Some("good-knight"),
    None,
];

struct CliArgs {
    count: usize,
    server: SocketAddr,
}

fn parse_args() -> CliArgs {
    let mut count = 9usize;
    let mut server: SocketAddr = "127.0.0.1:4000".parse().expect("default addr parses");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--count" | "-n" => {
                let value = args.next().unwrap_or_default();
                count = value.parse().unwrap_or_else(|_| {
                    eprintln!("Invalid --count '{value}', using 9");
                    9
                });
            }
            "--server" | "-s" => {
                let value = args.next().unwrap_or_default();
                server = value.parse().unwrap_or_else(|_| {
                    eprintln!("Invalid --server '{value}', using 127.0.0.1:4000");
                    "127.0.0.1:4000".parse().expect("default addr parses")
                });
            }
            "--help" | "-h" => {
                println!("Usage: bots [--count N] [--server HOST:PORT]");
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument '{other}' (see --help)");
                std::process::exit(2);
            }
        }
    }
    CliArgs { count, server }
}

/// Everything one bot tracks between ticks.
struct BotRunner {
    bot: Bot,
    class: HeroClass,
    lane: Lane,
    /// Learned from snapshots: server-assigned identity.
    my_id: Option<u64>,
    my_team: Option<Team>,
    brain: Option<BotBrain>,
    /// Last known own position/health from a snapshot.
    position: Option<(f32, f32)>,
    alive: bool,
    last_cast_at: [Option<Instant>; 4],
    last_upgrade_at: Instant,
    round_key: Option<(u64, u64)>,
    last_join_sent: Instant,
}

fn main() {
    let args = parse_args();
    println!(
        "Connecting {} bot(s) to {} (Ctrl+C or `make stop` to disconnect)",
        args.count, args.server
    );

    let mut runners: Vec<BotRunner> = Vec::with_capacity(args.count);
    for index in 0..args.count {
        let bot = Bot::connect_framed(args.server);
        let team = if index % 2 == 0 {
            Team::Blue
        } else {
            Team::Green
        };
        let class = CLASSES[index % CLASSES.len()];
        let avatar = AVATARS[index % AVATARS.len()];
        bot.join_with_loadout(team, Character::Ipfs, class, avatar);
        println!(
            "bot#{index}: join sent ({:?}, lane {:?}, avatar {})",
            class,
            Lane::ALL[index % Lane::ALL.len()],
            avatar.unwrap_or("default")
        );
        runners.push(BotRunner {
            bot,
            class,
            lane: Lane::ALL[index % Lane::ALL.len()],
            my_id: None,
            my_team: None,
            brain: None,
            position: None,
            alive: true,
            last_cast_at: [None; 4],
            last_upgrade_at: Instant::now() - UPGRADE_INTERVAL,
            round_key: None,
            last_join_sent: Instant::now(),
        });
    }

    let loadouts: Vec<(Team, HeroClass, Option<&str>)> = (0..args.count)
        .map(|index| {
            (
                if index % 2 == 0 {
                    Team::Blue
                } else {
                    Team::Green
                },
                CLASSES[index % CLASSES.len()],
                AVATARS[index % AVATARS.len()],
            )
        })
        .collect();

    let started = Instant::now();
    let mut last_status = Instant::now() - STATUS_INTERVAL;

    loop {
        let elapsed = started.elapsed().as_secs_f32();
        for (index, runner) in runners.iter_mut().enumerate() {
            // One call per tick: `recv_snapshot` blocks briefly for a
            // snapshot, then drains the buffered backlog internally and
            // returns the newest one. (Do NOT loop over it: with snapshots
            // arriving every 50 ms it would never return None, starving the
            // ping/move side until the server times the bot out.)
            let snapshot = runner
                .bot
                .recv_snapshot(Instant::now() + Duration::from_millis(20));
            runner.bot.ping();

            let Some(snapshot) = snapshot else {
                // UDP joins can be lost while the server is still starting:
                // resend until the bot sees itself in a snapshot.
                if runner.my_id.is_none()
                    && runner.last_join_sent.elapsed() > Duration::from_secs(1)
                {
                    let (team, class, avatar) = loadouts[index];
                    runner
                        .bot
                        .join_with_loadout(team, Character::Ipfs, class, avatar);
                    runner.last_join_sent = Instant::now();
                }
                continue;
            };

            let meta = snapshot.meta();
            let round_key = (meta.server_epoch, meta.match_id);
            if runner.round_key != Some(round_key) {
                runner.round_key = Some(round_key);
                runner.brain = None;
                runner.last_cast_at = [None; 4];
                runner.position = None;
                runner.my_id = None;
                runner.my_team = None;
            }
            let my_id = snapshot.your_id();
            let me = snapshot.player(my_id).cloned();
            if let Some(me) = &me {
                runner.my_id = Some(my_id);
                runner.my_team = me.team;
                runner.position = Some((me.x, me.z));

                // Death / respawn tracking: on revival, rejoin the lane from
                // the nearest waypoint (the server teleported us to base).
                let now_alive = me.hp > 0.0;
                if now_alive
                    && !runner.alive
                    && let Some(brain) = runner.brain.as_mut()
                {
                    brain.resync(me.x, me.z);
                    runner.last_cast_at = [None; 4];
                }
                runner.alive = now_alive;
            } else {
                runner.my_id = None;
                runner.position = None;
                if runner.last_join_sent.elapsed() > Duration::from_secs(1) {
                    let (team, class, avatar) = loadouts[index];
                    runner
                        .bot
                        .join_with_loadout(team, Character::Ipfs, class, avatar);
                    runner.last_join_sent = Instant::now();
                }
                continue;
            }

            if index == 0 && last_status.elapsed() >= STATUS_INTERVAL {
                last_status = Instant::now();
                match snapshot.game_state() {
                    GameState::Forming { ready, needed } => {
                        println!("matchmaking: waiting for players {ready}/{needed}")
                    }
                    GameState::Starting { countdown_ms } => {
                        println!("matchmaking: match found, starting in {countdown_ms}ms")
                    }
                    GameState::Running => println!("match running - bots pushing lanes"),
                    GameState::Victory { winner } => {
                        println!("match over, winner {winner:?} (waiting for rematch)")
                    }
                    GameState::Lobby => println!("lobby"),
                }
            }

            let (Some((x, z)), Some(team)) = (runner.position, runner.my_team) else {
                continue;
            };

            match snapshot.game_state() {
                GameState::Running if runner.alive => {
                    let me = me.as_ref().expect("admitted bot has its own state");
                    let now = Instant::now();
                    if runner.last_upgrade_at.elapsed() >= UPGRADE_INTERVAL {
                        if let Some(slot) = choose_skill_upgrade(me) {
                            runner.bot.upgrade_skill(slot);
                            runner.last_upgrade_at = now;
                        }
                    }
                    let brain = runner.brain.get_or_insert_with(|| {
                        let mut brain = BotBrain::new(runner.lane, team, runner.class);
                        brain.resync(x, z);
                        brain
                    });
                    let view = WorldView::from_snapshot(&snapshot, my_id, team);
                    let decision = brain.decide(x, z, &view);
                    if let Some(target) = decision.move_target {
                        let (nx, nz, yaw) =
                            step_toward((x, z), target, TICK_INTERVAL.as_secs_f32());
                        runner.bot.send_transform(nx, 0.5, nz, yaw);
                        runner.position = Some((nx, nz));
                    }
                    if let Some(slot) = choose_self_sustain(me, &runner.last_cast_at, now) {
                        runner.bot.cast_slot(harness::TargetId::player(my_id), slot);
                        runner.last_cast_at[slot as usize] = Some(now);
                    } else if let Some(target) = decision.cast
                        && can_cast_slot(me, 0, &runner.last_cast_at, now)
                    {
                        runner.bot.cast_slot(target, 0);
                        runner.last_cast_at[0] = Some(now);
                    }
                }
                GameState::Running => {
                    // Dead: wait for the server-side respawn.
                }
                GameState::Victory { .. } => {
                    // Server auto-rematches; brains restart from the base.
                    runner.brain = None;
                }
                _ => {
                    // Queue warm-up: small wander near spawn so bots look alive.
                    let phase = elapsed * 0.4 + index as f32 * 0.7;
                    let wx = x + phase.cos() * 0.15;
                    let wz = z + phase.sin() * 0.15;
                    let yaw = (-phase.sin()).atan2(-phase.cos());
                    runner.bot.send_transform(wx, 0.5, wz, yaw);
                }
            }
        }
        std::thread::sleep(TICK_INTERVAL);
    }
}
