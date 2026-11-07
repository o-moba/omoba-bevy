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
    path::PathBuf,
    time::{Duration, Instant},
};

use harness::{
    Bot, Character, GameState, HeroClass, SnapshotView, StructureKind, Team,
    bot_ai::{
        BotBrain, Lane, WorldView, choose_offensive_skill, choose_rally_lane, choose_self_sustain,
        choose_shop_item, choose_skill_upgrade, class_for_bot, step_toward,
    },
};

const TICK_INTERVAL: Duration = Duration::from_millis(100);
const STATUS_INTERVAL: Duration = Duration::from_secs(2);
/// Bound upgrade requests while waiting for the authoritative rank update.
const UPGRADE_INTERVAL: Duration = Duration::from_millis(250);

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
    telemetry: bool,
    public: bool,
    exit_after_result: bool,
    ranked_eligibility_probe: bool,
    rush_objectives: bool,
    search_gate: Option<PathBuf>,
    preference: shared::match_service::MatchPreference,
}

fn parse_args() -> CliArgs {
    let mut count = 9usize;
    let mut telemetry = false;
    let mut public = false;
    let mut exit_after_result = false;
    let mut ranked_eligibility_probe = false;
    let mut rush_objectives = false;
    let mut search_gate = None;
    let mut preference = shared::match_service::MatchPreference::Ranked;
    let mut server: SocketAddr = "127.0.0.1:4000".parse().expect("default addr parses");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--count" | "-n" => {
                let value = args.next().unwrap_or_default();
                count = value
                    .parse()
                    .ok()
                    .filter(|count| (1..=10).contains(count))
                    .unwrap_or_else(|| {
                        eprintln!("Invalid --count '{value}': expected 1..10");
                        std::process::exit(2);
                    });
            }
            "--server" | "-s" => {
                let value = args.next().unwrap_or_default();
                server = value
                    .parse::<SocketAddr>()
                    .ok()
                    .filter(|address| address.port() != 0 && !address.ip().is_unspecified())
                    .unwrap_or_else(|| {
                        eprintln!(
                            "Invalid --server '{value}': expected IP:PORT with a nonzero port"
                        );
                        std::process::exit(2);
                    });
            }
            "--telemetry" => telemetry = true,
            "--public" => public = true,
            "--exit-after-result" => exit_after_result = true,
            "--ranked-eligibility-probe" => ranked_eligibility_probe = true,
            "--rush-objectives" => rush_objectives = true,
            "--search-gate" => {
                let value = args.next().filter(|s| !s.is_empty()).unwrap_or_else(|| {
                    eprintln!("--search-gate requires a file path");
                    std::process::exit(2);
                });
                search_gate = Some(PathBuf::from(value));
            }
            "--preference" => {
                preference = match args.next().as_deref() {
                    Some("humans_only") => shared::match_service::MatchPreference::Ranked,
                    Some("quick") => shared::match_service::MatchPreference::Casual,
                    Some("bot_practice") => shared::match_service::MatchPreference::BotPractice,
                    _ => {
                        eprintln!(
                            "Invalid --preference: expected humans_only, quick or bot_practice"
                        );
                        std::process::exit(2);
                    }
                };
            }
            "--help" | "-h" => {
                println!(
                    "Usage: bots [--count 1..10] [--server IP:PORT] [--telemetry] [--public] [--exit-after-result] [--rush-objectives] [--search-gate FILE] [--preference humans_only|quick|bot_practice]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument '{other}' (see --help)");
                std::process::exit(2);
            }
        }
    }
    if exit_after_result && !public {
        eprintln!("--exit-after-result requires --public");
        std::process::exit(2);
    }
    if search_gate.is_some() && !(public && server.ip().is_loopback()) {
        eprintln!("--search-gate requires public loopback admission");
        std::process::exit(2);
    }
    if rush_objectives && !server.ip().is_loopback() {
        eprintln!("--rush-objectives is a loopback rehearsal option");
        std::process::exit(2);
    }
    if ranked_eligibility_probe
        && !(public
            && exit_after_result
            && count == 1
            && server.ip().is_loopback()
            && preference == shared::match_service::MatchPreference::BotPractice)
    {
        eprintln!(
            "eligibility probe requires one public loopback vs-Bots client with --exit-after-result"
        );
        std::process::exit(2);
    }
    CliArgs {
        count,
        server,
        telemetry,
        public,
        exit_after_result,
        ranked_eligibility_probe,
        rush_objectives,
        search_gate,
        preference,
    }
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
    navigation: harness::navigation::BotNavigator,
    rally_lane: Option<Lane>,
    /// Last known own position/health from a snapshot.
    position: Option<(f32, f32)>,
    alive: bool,
    last_cast_at: [Option<Instant>; 4],
    last_upgrade_at: Instant,
    round_key: Option<(u64, u64)>,
    last_join_sent: Instant,
    last_snapshot_at: Option<Instant>,
    last_shop_at: Instant,
    pending_purchase: Option<(shared::shop::ItemId, u64)>,
}

fn main() {
    let args = parse_args();
    println!(
        "Connecting {} bot(s) to {} (Ctrl+C or `make stop` to disconnect)",
        args.count, args.server
    );

    let mut runners: Vec<BotRunner> = Vec::with_capacity(args.count);
    for index in 0..args.count {
        let team = if index % 2 == 0 {
            Team::Blue
        } else {
            Team::Green
        };
        let class = class_for_bot(index);
        let avatar = if args.public {
            None
        } else {
            AVATARS[index % AVATARS.len()]
        };
        let bot = if args.public {
            Bot::connect_public(args.server, args.preference, class, format!("Fill-{index}"))
        } else {
            let bot = Bot::connect_framed(args.server);
            bot.join_with_loadout(team, Character::Ipfs, class, avatar);
            bot
        };
        if args.ranked_eligibility_probe {
            bot.enable_ranked_eligibility_probe(args.server);
        }
        if args.search_gate.is_some() {
            bot.set_public_search_enabled(false);
        }
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
            navigation: harness::navigation::BotNavigator::default(),
            rally_lane: None,
            position: None,
            alive: true,
            last_cast_at: [None; 4],
            last_upgrade_at: Instant::now() - UPGRADE_INTERVAL,
            round_key: None,
            last_join_sent: Instant::now(),
            last_snapshot_at: None,
            last_shop_at: Instant::now() - UPGRADE_INTERVAL,
            pending_purchase: None,
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
                class_for_bot(index),
                AVATARS[index % AVATARS.len()],
            )
        })
        .collect();

    let started = Instant::now();
    let mut last_status = Instant::now() - STATUS_INTERVAL;

    let mut last_telemetry = Instant::now() - Duration::from_secs(1);
    let mut last_phase = String::new();
    let mut last_auth_telemetry = Instant::now() - Duration::from_secs(1);
    let mut search_gate_open = args.search_gate.is_none();
    loop {
        if !search_gate_open
            && args.search_gate.as_ref().is_some_and(|path| {
                std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == 0)
            })
        {
            for runner in &runners {
                runner.bot.set_public_search_enabled(true);
            }
            search_gate_open = true;
        }
        let mut observed = None;
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
                if args.public && index == 0 && last_status.elapsed() >= STATUS_INTERVAL {
                    last_status = Instant::now();
                    println!(
                        "PUBLIC_BOT {}",
                        serde_json::to_string(&runner.bot.public_status())
                            .expect("public status serializes")
                    );
                }
                // UDP joins can be lost while the server is still starting:
                // resend until the bot sees itself in a snapshot.
                if !args.public
                    && runner.my_id.is_none()
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

            runner.last_snapshot_at = Some(Instant::now());
            if index == 0 {
                observed = Some(snapshot.clone());
            }
            let meta = snapshot.meta();
            let round_key = (meta.server_epoch, meta.match_id);
            if runner.round_key != Some(round_key) {
                runner.round_key = Some(round_key);
                runner.brain = None;
                runner.navigation.clear();
                runner.rally_lane = None;
                runner.last_cast_at = [None; 4];
                runner.position = None;
                runner.my_id = None;
                runner.my_team = None;
                runner.pending_purchase = None;
            }
            let my_id = snapshot.your_id();
            let me = snapshot.player(my_id).cloned();
            if let Some(me) = &me {
                runner.my_id = Some(my_id);
                runner.my_team = Some(me.team);
                runner.position = Some((me.x, me.z));

                // Death / respawn tracking: on revival, rejoin the lane from
                // the nearest waypoint (the server teleported us to base).
                let now_alive = me.hp > 0.0;
                if now_alive
                    && !runner.alive
                    && let Some(brain) = runner.brain.as_mut()
                {
                    brain.resync(me.x, me.z);
                    runner.navigation.clear();
                    runner.last_cast_at = [None; 4];
                }
                runner.alive = now_alive;
            } else {
                runner.my_id = None;
                runner.position = None;
                if !args.public && runner.last_join_sent.elapsed() > Duration::from_secs(1) {
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
                    if let Some((_, request)) = runner.pending_purchase
                        && me.last_purchase.as_ref().is_some_and(|receipt| {
                            receipt.match_id == meta.match_id && receipt.request_id >= request
                        })
                    {
                        runner.pending_purchase = None;
                    }
                    if runner.pending_purchase.is_none()
                        && let Some(item) = choose_shop_item(me)
                    {
                        let request = me
                            .last_purchase
                            .as_ref()
                            .map_or(1, |receipt| receipt.request_id.saturating_add(1));
                        runner.pending_purchase = Some((item, request));
                    }
                    if runner.last_shop_at.elapsed() >= UPGRADE_INTERVAL
                        && let Some((item, request)) = runner.pending_purchase
                    {
                        runner
                            .bot
                            .buy_item(item, request, meta.match_id, meta.server_epoch);
                        runner.last_shop_at = now;
                    }
                    if runner.last_upgrade_at.elapsed() >= UPGRADE_INTERVAL {
                        if let Some(slot) = choose_skill_upgrade(me) {
                            runner.bot.upgrade_skill(slot);
                            runner.last_upgrade_at = now;
                        }
                    }
                    if me.level >= 6 && runner.rally_lane.is_none() {
                        runner.rally_lane = Some(choose_rally_lane(&snapshot, team));
                        runner.brain = None;
                        runner.navigation.clear();
                    }
                    let brain = runner.brain.get_or_insert_with(|| {
                        let mut brain = BotBrain::new(
                            runner.rally_lane.unwrap_or(runner.lane),
                            team,
                            runner.class,
                        );
                        brain.resync(x, z);
                        brain
                    });
                    let view = WorldView::from_snapshot(&snapshot, my_id, team);
                    let mut decision = brain.decide(x, z, &view);
                    // A disposable custom-map rehearsal can focus objectives
                    // instead of waiting for the lane AI's midgame rally. All
                    // movement, range, cooldown and damage remain authoritative.
                    if args.rush_objectives
                        && let Some(objective) = view
                            .structures
                            .iter()
                            .filter(|s| !view.protected_structures.contains(&s.id))
                            .min_by(|a, b| {
                                let distance =
                                    |p: &harness::bot_ai::EnemyRef| (p.x - x).hypot(p.z - z);
                                distance(a).total_cmp(&distance(b))
                            })
                    {
                        let dx = x - objective.x;
                        let dz = z - objective.z;
                        let distance = dx.hypot(dz);
                        decision.move_target = (distance > 6.0).then(|| {
                            (
                                objective.x + dx / distance * 6.0,
                                objective.z + dz / distance * 6.0,
                            )
                        });
                        decision.cast = Some(harness::TargetId {
                            kind: objective.kind,
                            id: objective.id,
                        });
                    }
                    let structures: Vec<_> = snapshot
                        .structures()
                        .iter()
                        .filter(|s| s.hp > 0.0)
                        .map(|s| shared::navigation::Disc {
                            center: [s.x, s.z],
                            radius: if s.kind == StructureKind::BaseTower {
                                3.2
                            } else {
                                1.3
                            },
                        })
                        .collect();
                    let transform = decision
                        .move_target
                        .and_then(|target| {
                            runner
                                .navigation
                                .next([x, z], [target.0, target.1], &structures)
                        })
                        .map(|target| {
                            step_toward(
                                (x, z),
                                (target[0], target[1]),
                                TICK_INTERVAL.as_secs_f32() * me.item_bonuses.move_speed_multiplier,
                            )
                        })
                        .unwrap_or((x, z, me.yaw));
                    // This fresh alive controller deliberately holds movement/
                    // aim even while waiting for minion support or a route.
                    // Send the normal active-input packet like the native
                    // client's held control; Ping alone is not gameplay input.
                    runner
                        .bot
                        .send_transform(transform.0, 0.5, transform.1, transform.2);
                    runner.position = Some((transform.0, transform.1));
                    if let Some(slot) = choose_self_sustain(me, &runner.last_cast_at, now) {
                        runner.bot.cast_slot(harness::TargetId::player(my_id), slot);
                        runner.last_cast_at[slot as usize] = Some(now);
                    } else if let Some(target) = decision.cast
                        && let Some(slot) =
                            choose_offensive_skill(me, target, &view, &runner.last_cast_at, now)
                    {
                        runner.bot.cast_slot(target, slot);
                        runner.last_cast_at[slot as usize] = Some(now);
                    }
                }
                GameState::Running => {
                    // Dead: wait for the server-side respawn.
                    runner.navigation.clear();
                }
                GameState::Victory { .. } => {
                    // Server auto-rematches; brains restart from the base.
                    runner.brain = None;
                    runner.navigation.clear();
                }
                _ => {
                    // Queue warm-up: small wander near spawn so bots look alive.
                    let phase = elapsed * 0.4 + index as f32 * 0.7;
                    let wx = x + phase.cos() * 0.15;
                    let wz = z + phase.sin() * 0.15;
                    let yaw = shared::math::hero_yaw_towards(phase.sin(), phase.cos());
                    runner.bot.send_transform(wx, 0.5, wz, yaw);
                }
            }
        }
        if args.telemetry
            && let Some(snapshot) = observed
        {
            let phase = match snapshot.game_state() {
                GameState::Lobby => "lobby",
                GameState::Forming { .. } => "forming",
                GameState::Starting { .. } => "starting",
                GameState::Running => "running",
                GameState::Victory { .. } => "victory",
            };
            if last_telemetry.elapsed() >= Duration::from_secs(1) || phase != last_phase {
                last_telemetry = Instant::now();
                last_phase = phase.to_owned();
                let players: Vec<_> = snapshot.players().iter().map(|p| serde_json::json!({
                    "id":p.id, "team":format!("{:?}",p.team), "class":p.hero_class,
                    "is_bot":p.is_bot, "level":p.level, "xp":p.xp, "gold":p.gold, "ranks":p.ranks,
                    "inventory":p.inventory, "item_bonuses":p.item_bonuses, "shop_available":p.shop_available, "last_purchase":p.last_purchase,
                    "hp":p.hp, "max_hp":p.max_hp, "mana":p.mana, "max_mana":p.max_mana,
                    "x":p.x,"z":p.z,"action_sequence":p.action_sequence,"action_slot":p.action_slot
                })).collect();
                let structures: Vec<_> = snapshot.structures().iter().map(|s| serde_json::json!({
                    "id":s.id,"team":format!("{:?}",s.team),"hp":s.hp,"protected":s.protected
                })).collect();
                let peers: Vec<_> = runners.iter().map(|r| serde_json::json!({
                    "id":r.my_id,"snapshot_age_ms":r.last_snapshot_at.map(|at|at.elapsed().as_millis()),
                    "transport":r.bot.transport_stats(),
                    "snapshot_meta":r.bot.snapshot_meta(),
                    "public":r.bot.public_status().map(|s|serde_json::json!({"phase":s.phase,"profile_id":s.profile_id,"allocation":s.allocation,"error":s.error,"ranked_denied_counts":s.ranked_denied_counts,"ranked_accepted_count":s.ranked_accepted_count,"stale_career_packets":s.stale_career_packets,"authority_resets":s.authority_resets}))
                })).collect();
                println!(
                    "BOT_SAMPLE {}",
                    serde_json::json!({
                        "elapsed_secs":started.elapsed().as_secs_f64(),"meta":snapshot.meta(),
                        "phase":phase,"state":format!("{:?}",snapshot.game_state()),
                        "players":players,"structures":structures,"peers":peers,
                        "minions":snapshot.minions().len(),"buffs":snapshot.team_buffs().len(),
                        "join_error":format!("{:?}",snapshot.join_error())
                    })
                );
            }
        }
        if args.telemetry
            && args.public
            && !search_gate_open
            && last_auth_telemetry.elapsed() >= Duration::from_secs(1)
        {
            last_auth_telemetry = Instant::now();
            let peers: Vec<_> = runners
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "public": r.bot.public_status().map(|s| serde_json::json!({
                            "phase": s.phase, "profile_id": s.profile_id,
                            "auth_age_ms": s.auth_age_ms, "error": s.error,
                            "allocation": s.allocation,
                        }))
                    })
                })
                .collect();
            println!(
                "BOT_AUTH {}",
                serde_json::json!({
                    "elapsed_secs": started.elapsed().as_secs_f64(), "peers": peers,
                })
            );
        }
        if args.exit_after_result {
            let statuses: Vec<_> = runners
                .iter()
                .filter_map(|r| r.bot.public_status())
                .collect();
            if statuses.len() == args.count
                && statuses.iter().all(|s| {
                    let (Some(result), Some(profile), Some(initial), Some(history)) =
                        (&s.result, &s.profile, &s.initial_profile, &s.history)
                    else {
                        return false;
                    };
                    result.saved
                        && (!args.ranked_eligibility_probe
                            || (s.ranked_denied_counts == [0, 9]
                                && s.ranked_accepted_count == Some(10)))
                        && result.outcome == shared::career::MatchOutcome::Completed
                        && profile.matches_played == initial.matches_played + 1
                        && history.iter().any(|r| r.result_id == result.result_id)
                })
            {
                println!(
                    "BOT_RESULT {}",
                    serde_json::to_string(&statuses).expect("saved results serialize")
                );
                return;
            }
        }
        std::thread::sleep(TICK_INTERVAL);
    }
}
