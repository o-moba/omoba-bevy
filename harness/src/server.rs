//! Spawns the real authoritative server as a child process for a test.
//!
//! [`ServerProcess`] launches `server` bound to a unique loopback port via the
//! `SERVER_ADDR` environment variable, waits until the server reports it is
//! listening, and **kills the child on `Drop`** (RAII) so every test cleans up
//! after itself even on panic. Each [`ServerProcess`] owns a fresh port, so
//! tests never share global state and are safe to run in parallel.

use std::{
    io::{BufRead, BufReader},
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// How long to wait for the server to announce it is listening. Generous so a
/// cold `cargo run` (which may compile) still succeeds; a prebuilt binary is
/// near-instant.
const READY_TIMEOUT: Duration = Duration::from_secs(90);

/// How many times to retry with a fresh port if a spawn loses the port race
/// (the reserved port got taken between release and the server binding it).
const MAX_SPAWN_ATTEMPTS: usize = 3;

/// A running server child process bound to a unique loopback port.
pub struct ServerProcess {
    child: Child,
    addr: SocketAddr,
}

impl ServerProcess {
    /// Spawns a **dev-mode** server (instant match start on first join) on a
    /// free loopback port and blocks until it is ready.
    ///
    /// Dev mode preserves the instant-start assumption baked into the
    /// existing gameplay scenarios; matchmaking scenarios use
    /// [`Self::spawn_with_env`] to run the server in release mode instead.
    ///
    /// Retries up to [`MAX_SPAWN_ATTEMPTS`] times with a fresh port if the
    /// server loses the port race (binds the reserved port a moment too late).
    /// A lost race is detected immediately via the child exiting early, so the
    /// retry is fast — no waiting out the full [`READY_TIMEOUT`]. Panics with a
    /// clear message only after every attempt is exhausted.
    pub fn spawn() -> Self {
        Self::spawn_with_env(&[("OMOBA_MATCH_MODE", "dev")])
    }

    /// Like [`Self::spawn`] but with explicit extra environment variables
    /// (e.g. `OMOBA_MATCH_MODE=release`, `OMOBA_TEAM_SIZE=1`).
    pub fn spawn_with_env(envs: &[(&str, &str)]) -> Self {
        let mut last_error = String::new();
        for attempt in 1..=MAX_SPAWN_ATTEMPTS {
            match Self::try_spawn_once(envs) {
                Ok(server) => return server,
                Err(error) => last_error = format!("attempt {attempt}/{MAX_SPAWN_ATTEMPTS}: {error}"),
            }
        }
        panic!(
            "server failed to start after {MAX_SPAWN_ATTEMPTS} attempts ({last_error}); \
             build the server first (`cargo build -p server`)"
        );
    }

    /// Single spawn attempt on a freshly reserved port. Returns an error string
    /// (instead of panicking) so the caller can retry on a lost port race.
    fn try_spawn_once(envs: &[(&str, &str)]) -> Result<Self, String> {
        let port = free_loopback_port();
        let addr: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .expect("loopback addr should parse");

        let mut command = build_command();
        command
            .env("SERVER_ADDR", addr.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in envs {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to launch server process: {error}"))?;

        // Drain stdout on a background thread and signal once the server says it
        // is listening. Draining to EOF also prevents the child from blocking on
        // a full stdout pipe.
        let stdout = child
            .stdout
            .take()
            .expect("server child should expose a piped stdout");
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut signaled = false;
            for line in reader.lines().map_while(Result::ok) {
                if !signaled && line.contains("is listening") {
                    let _ = ready_tx.send(());
                    signaled = true;
                }
            }
        });

        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if ready_rx.try_recv().is_ok() {
                return Ok(ServerProcess { child, addr });
            }
            // A child that exits before announcing it is listening lost the port
            // race (or otherwise failed to bind) — retry fast with a new port.
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(format!(
                        "server on {addr} exited early ({status}) before listening \
                         (port likely already in use)"
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    return Err(format!("failed to poll server child: {error}"));
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "server did not report listening on {addr} within {READY_TIMEOUT:?}"
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// The loopback address bots should connect to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reserves a free UDP loopback port by binding to `:0` then releasing it.
///
/// There is a tiny race between releasing the port and the server rebinding it;
/// [`ServerProcess::spawn`] absorbs a lost race by retrying on a fresh port, and
/// each test still gets a distinct port, keeping runs independent.
fn free_loopback_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("failed to reserve a loopback UDP port");
    socket
        .local_addr()
        .expect("bound socket should have a local addr")
        .port()
}

/// Builds the command that launches the server.
///
/// Resolution order:
///   1. `HARNESS_SERVER_BIN` env var (explicit override).
///   2. A prebuilt `target/debug/server` binary (fast path; the Makefile target
///      builds this first).
///   3. `cargo run -q -p server` as a fallback (may compile on first use).
fn build_command() -> Command {
    if let Ok(explicit) = std::env::var("HARNESS_SERVER_BIN") {
        return Command::new(explicit);
    }

    if let Some(bin) = prebuilt_binary() {
        return Command::new(bin);
    }

    let mut command = Command::new(env!("CARGO"));
    command
        .arg("run")
        .arg("-q")
        .arg("-p")
        .arg("server")
        .current_dir(workspace_root());
    command
}

/// Workspace root, derived from this crate's manifest directory (`harness/`).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness crate should have a parent workspace directory")
        .to_path_buf()
}

/// Returns the prebuilt debug server binary path if it exists, honoring
/// `CARGO_TARGET_DIR` when set.
fn prebuilt_binary() -> Option<PathBuf> {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"));
    let candidate = target_dir.join("debug").join(server_bin_name());
    candidate.exists().then_some(candidate)
}

fn server_bin_name() -> &'static str {
    if cfg!(windows) { "server.exe" } else { "server" }
}
