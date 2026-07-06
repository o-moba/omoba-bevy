//! Fill bots: joins N dummy players to a running server over real UDP so one
//! developer can form a full 5v5 match locally and walk through the whole
//! matchmaking flow from the client's point of view.
//!
//! The bots are deliberately dumb: they join (round-robin classes/avatars),
//! then keep the connection alive with pings and a slow wander around their
//! spawn so they look alive in-game. They exercise matchmaking, match start,
//! and basic playability - not combat AI.
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

use harness::{Bot, Character, GameState, HeroClass, Team};

const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(100);
const STATUS_INTERVAL: Duration = Duration::from_secs(2);
/// Wander ellipse radius in world units, small enough to stay on the pad.
const WANDER_RADIUS: f32 = 2.0;
const WANDER_SPEED: f32 = 0.4;

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

fn main() {
    let args = parse_args();
    println!(
        "Connecting {} bot(s) to {} (Ctrl+C or `make stop` to disconnect)",
        args.count, args.server
    );

    let mut bots: Vec<Bot> = Vec::with_capacity(args.count);
    let mut loadouts: Vec<(Team, HeroClass, Option<&str>)> = Vec::with_capacity(args.count);
    for index in 0..args.count {
        let bot = Bot::connect(args.server);
        let team = if index % 2 == 0 { Team::Blue } else { Team::Green };
        let class = CLASSES[index % CLASSES.len()];
        let avatar = AVATARS[index % AVATARS.len()];
        bot.join_with_loadout(team, Character::Ipfs, class, avatar);
        println!(
            "bot#{index}: join sent ({:?}, avatar {})",
            class,
            avatar.unwrap_or("default")
        );
        loadouts.push((team, class, avatar));
        bots.push(bot);
    }

    // Learn spawn anchors from the first snapshots so the wander stays local.
    let mut anchors: Vec<Option<(f32, f32)>> = vec![None; bots.len()];
    // UDP joins can be lost while the server is still starting up: resend
    // until the bot sees itself in a snapshot.
    let mut last_join_sent: Vec<Instant> = vec![Instant::now(); bots.len()];
    let started = Instant::now();
    let mut last_status = Instant::now() - STATUS_INTERVAL;

    loop {
        let elapsed = started.elapsed().as_secs_f32();
        for (index, bot) in bots.iter_mut().enumerate() {
            // Drain all pending snapshots (they arrive faster than this loop
            // ticks); keep only the freshest one for anchor/status handling.
            let mut snapshot = None;
            while let Some(next) = bot.recv_snapshot(Instant::now() + Duration::from_millis(2)) {
                snapshot = Some(next);
            }
            if let Some(snapshot) = &snapshot {
                let my_id = snapshot.your_id();
                if anchors[index].is_none()
                    && let Some(me) = snapshot.player(my_id)
                {
                    anchors[index] = Some((me.x, me.z));
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
                        GameState::Running => println!("match running"),
                        GameState::Victory { winner } => {
                            println!("match over, winner {winner:?}")
                        }
                        GameState::Lobby => println!("lobby"),
                    }
                }
            }
            bot.ping();
            if anchors[index].is_none() && last_join_sent[index].elapsed() > Duration::from_secs(1)
            {
                let (team, class, avatar) = loadouts[index];
                bot.join_with_loadout(team, Character::Ipfs, class, avatar);
                last_join_sent[index] = Instant::now();
            }
            if let Some((ax, az)) = anchors[index] {
                // Slow per-bot wander circle; phase-shifted so bots spread out.
                let phase = elapsed * WANDER_SPEED + index as f32 * 0.7;
                let x = ax + phase.cos() * WANDER_RADIUS;
                let z = az + phase.sin() * WANDER_RADIUS;
                let yaw = (-phase.sin()).atan2(-phase.cos());
                bot.send_transform(x, 0.5, z, yaw);
            }
        }
        std::thread::sleep(KEEPALIVE_INTERVAL);
    }
}
