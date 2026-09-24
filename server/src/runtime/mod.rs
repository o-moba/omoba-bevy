//! The authoritative UDP game server runtime: socket, sub-runtimes and the
//! process entry point.
use crate::*;

pub(crate) mod dispatch;
pub(crate) mod tick;

pub(crate) const DEFAULT_BIND_ADDR: &str = "0.0.0.0:4000";

pub(crate) const PLAYER_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) const SIMULATION_STEP_SLEEP: Duration = Duration::from_millis(10);

pub(crate) const NETWORK_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(crate) struct RateLimitedDiagnostic {
    pub(crate) last_emitted_at: Option<Instant>,
    pub(crate) suppressed: u64,
}

impl RateLimitedDiagnostic {
    /// Returns the number of suppressed events when this occurrence should be
    /// logged. Calls inside the interval are counted without producing output.
    pub(crate) fn record(&mut self, now: Instant) -> Option<u64> {
        if self
            .last_emitted_at
            .is_none_or(|last| now.duration_since(last) >= NETWORK_DIAGNOSTIC_INTERVAL)
        {
            self.last_emitted_at = Some(now);
            return Some(std::mem::take(&mut self.suppressed));
        }
        self.suppressed = self.suppressed.saturating_add(1);
        None
    }
}

pub(crate) struct ServerRuntime {
    pub(crate) sandbox: Option<sandbox::SandboxRuntime>,
    pub(crate) match_service: match_service::MatchService,
    pub(crate) public_transport: public_transport::PublicTransport,
    pub(crate) prematch: prematch::PrematchRuntime,
    pub(crate) social: social::SocialRuntime,
    pub(crate) bots: bots::BotControllers,
    pub(crate) career: career_runtime::CareerRuntime,
    pub(crate) combat_log: CombatLog,
    pub(crate) passport_admissions: passport_admission::PassportAdmissions,
    pub(crate) socket: UdpSocket,
    pub(crate) world: GameWorld,
    pub(crate) victory_at: Option<Instant>,
    pub(crate) recv_buf: Vec<u8>,
    pub(crate) invalid_request_diagnostic: RateLimitedDiagnostic,
    pub(crate) snapshot_send_diagnostic: RateLimitedDiagnostic,
    pub(crate) last_snapshot_at: Instant,
    pub(crate) last_bootstrap_at: Instant,
    pub(crate) last_simulation_at: Instant,
    pub(crate) match_config: MatchConfig,
    pub(crate) targeting_qa: bool,
    pub(crate) server_epoch: u64,
    pub(crate) match_id: u64,
    pub(crate) snapshot_tick: u64,
    pub(crate) match_started_at: Option<Instant>,
    pub(crate) empty_since: Option<Instant>,
    pub(crate) metrics_players: HashMap<u64, (u32, bool)>,
    pub(crate) metrics_objectives: HashSet<u64>,
}

impl ServerRuntime {
    #[cfg(test)]
    pub(crate) fn new(socket: UdpSocket, match_config: MatchConfig) -> Self {
        Self::new_with_map(socket, match_config, shared::map::ResolvedMap::default())
    }

    pub(crate) fn new_with_map(
        socket: UdpSocket,
        match_config: MatchConfig,
        map_config: shared::map::ResolvedMap,
    ) -> Self {
        let server_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
            | 1;
        Self {
            sandbox: None,
            match_service: match_service::MatchService::default(),
            public_transport: public_transport::PublicTransport::default(),
            prematch: prematch::PrematchRuntime::default(),
            bots: bots::BotControllers::default(),
            social: social::SocialRuntime::default(),
            career: career_runtime::CareerRuntime::new(server_epoch),
            combat_log: CombatLog::default(),
            passport_admissions: passport_admission::PassportAdmissions::default(),
            socket,
            world: GameWorld::new(map_config, Instant::now()),
            victory_at: None,
            recv_buf: vec![0_u8; CLIENT_DATAGRAM_RECEIVE_CAPACITY],
            invalid_request_diagnostic: RateLimitedDiagnostic::default(),
            snapshot_send_diagnostic: RateLimitedDiagnostic::default(),
            last_snapshot_at: Instant::now(),
            last_bootstrap_at: Instant::now(),
            last_simulation_at: Instant::now(),
            match_config,
            targeting_qa: targeting_qa::enabled(match_config.mode),
            server_epoch,
            match_id: 1,
            snapshot_tick: 0,
            match_started_at: None,
            empty_since: None,
            metrics_players: HashMap::new(),
            metrics_objectives: HashSet::new(),
        }
    }
}

pub(crate) fn run() -> io::Result<()> {
    let map_path = std::env::var_os("OMOBA_MAP_CONFIG").map(std::path::PathBuf::from);
    let map_config = load_map_config(map_path.as_deref())?;
    println!(
        "Map profile: {} geometry: {} structures: {}",
        map_config.map_profile,
        map_config.geometry_id,
        map_config.structures.len()
    );
    let bind_addr = std::env::var("SERVER_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    let socket = UdpSocket::bind(&bind_addr)?;
    socket.set_nonblocking(true)?;
    println!("UDP game server is listening on {bind_addr}");

    let match_service = match_service::MatchService::from_env().map_err(io::Error::other)?;
    if match_service.is_public()
        && std::env::var("OMOBA_DATABASE_URL")
            .ok()
            .is_none_or(|v| v.trim().is_empty())
    {
        return Err(io::Error::other("Public roles require OMOBA_DATABASE_URL"));
    }
    let mut match_config = MatchConfig::from_env();
    if let Some(worker) = match_service.worker() {
        if worker.manifest.bind != bind_addr {
            return Err(io::Error::other("Worker bind differs from allocation"));
        }
        match_config = MatchConfig {
            mode: if worker.manifest.humans.len() == 10 {
                MatchMode::Release
            } else {
                MatchMode::Practice
            },
            team_size: 5,
        };
    }
    match match_config.mode {
        MatchMode::Release => println!(
            "Match mode: release - matches form to {}v{} before starting (OMOBA_MATCH_MODE=dev for instant start)",
            match_config.team_size, match_config.team_size
        ),
        MatchMode::Dev => {
            println!("Match mode: dev - first join starts the match immediately (NOT for release)")
        }
        MatchMode::Practice => println!(
            "Match mode: practice - solo start, {}v{} with server bots; local results, no career credit",
            match_config.team_size, match_config.team_size
        ),
    }

    let mut runtime = ServerRuntime::new_with_map(socket, match_config, map_config);
    runtime.match_service = match_service;
    if std::env::var("OMOBA_COMBAT_SANDBOX").as_deref() == Ok("1") {
        if match_config.mode != MatchMode::Dev
            || runtime.match_service.is_public()
            || runtime.match_service.worker().is_some()
            || !runtime.socket.local_addr()?.ip().is_loopback()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Combat Sandbox requires MATCH_MODE=dev and a loopback SERVER_ADDR",
            ));
        }
        runtime.sandbox = Some(sandbox::SandboxRuntime::new(Instant::now()));
        println!("Combat Sandbox enabled (local, unrated)");
    }
    loop {
        let step_started = Instant::now();
        let (now, dt) = runtime.prepare_tick();
        runtime.tick(now, dt);
        let step_elapsed = step_started.elapsed();
        if step_elapsed < SIMULATION_STEP_SLEEP {
            std::thread::sleep(SIMULATION_STEP_SLEEP - step_elapsed);
        }
    }
}
