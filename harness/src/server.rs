//! Spawns the real authoritative server as a child process for a test.
//!
//! [`ServerProcess`] launches `server` bound to a unique loopback port via the
//! `SERVER_ADDR` environment variable, waits until the server reports it is
//! listening, and **kills the child on `Drop`** (RAII) so every test cleans up
//! after itself even on panic. Each [`ServerProcess`] owns a fresh port, so
//! tests never share global state and are safe to run in parallel.
//!
//! The server's stdout and stderr are kept in a ring buffer of the last
//! [`LOG_LINES`] lines; when a test panics while it holds the server, the drop
//! prints them so a CI failure shows what the server was doing. A prebuilt
//! `target/debug/server` older than the newest file under `server/src` or
//! `shared/src` produces a warning, since the harness would test stale code.

use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, Once, mpsc},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// How long to wait for the server to announce it is listening. Generous so a
/// cold `cargo run` (which may compile) still succeeds; a prebuilt binary is
/// near-instant.
const READY_TIMEOUT: Duration = Duration::from_secs(90);

/// How many times to retry with a fresh port if a spawn loses the port race
/// (the reserved port got taken between release and the server binding it).
const MAX_SPAWN_ATTEMPTS: usize = 3;

/// How many of the server's most recent output lines are kept for diagnostics.
const LOG_LINES: usize = 200;

/// The last [`LOG_LINES`] lines the server wrote to stdout or stderr.
#[derive(Clone, Default)]
struct ServerLog(Arc<Mutex<VecDeque<String>>>);

impl ServerLog {
    fn push(&self, line: String) {
        let mut lines = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if lines.len() == LOG_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// The buffered lines, oldest first, one per line.
    fn tail(&self) -> String {
        let lines = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        lines.iter().fold(String::new(), |mut text, line| {
            text.push_str(line);
            text.push('\n');
            text
        })
    }

    /// Reads `stream` line by line on a background thread into the buffer,
    /// calling `on_line` for each line first. Reading to EOF also keeps the
    /// child from blocking on a full pipe.
    fn drain(
        &self,
        stream: impl Read + Send + 'static,
        tag: &'static str,
        mut on_line: impl FnMut(&str) + Send + 'static,
    ) {
        let log = self.clone();
        thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                on_line(&line);
                log.push(format!("[{tag}] {line}"));
            }
        });
    }
}

/// A running server child process bound to a unique loopback port.
pub struct ServerProcess {
    child: Child,
    addr: SocketAddr,
    log: ServerLog,
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
                Err(error) => {
                    last_error = format!("attempt {attempt}/{MAX_SPAWN_ATTEMPTS}: {error}")
                }
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
            .stderr(Stdio::piped());
        for (key, value) in envs {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to launch server process: {error}"))?;

        // Drain both streams into the ring buffer on background threads and
        // signal once the server says it is listening.
        let stdout = child
            .stdout
            .take()
            .expect("server child should expose a piped stdout");
        let stderr = child
            .stderr
            .take()
            .expect("server child should expose a piped stderr");
        let log = ServerLog::default();
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let mut signaled = false;
        log.drain(stdout, "out", move |line| {
            if !signaled && line.contains("is listening") {
                let _ = ready_tx.send(());
                signaled = true;
            }
        });
        log.drain(stderr, "err", |_| {});

        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if ready_rx.try_recv().is_ok() {
                return Ok(ServerProcess { child, addr, log });
            }
            // A child that exits before announcing it is listening lost the port
            // race (or otherwise failed to bind) — retry fast with a new port.
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Give the readers a moment to collect the last words.
                    thread::sleep(Duration::from_millis(50));
                    return Err(format!(
                        "server on {addr} exited early ({status}) before listening \
                         (port likely already in use); last output:\n{}",
                        log.tail()
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
                    "server did not report listening on {addr} within {READY_TIMEOUT:?}; \
                     last output:\n{}",
                    log.tail()
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
        if thread::panicking() {
            // Captured with the failing test's output, so it is shown only
            // for failures (and always with `--nocapture`).
            eprintln!(
                "---- last {LOG_LINES} lines of the server on {} ----\n{}---- end of server output ----",
                self.addr,
                self.log.tail()
            );
        }
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
        warn_if_stale(&bin);
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
    if cfg!(windows) {
        "server.exe"
    } else {
        "server"
    }
}

/// Warns once per test binary when the prebuilt server is older than its
/// sources, the usual cause of a harness failure that does not reproduce.
fn warn_if_stale(binary: &Path) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(built) = std::fs::metadata(binary).and_then(|m| m.modified()) else {
            return;
        };
        let root = workspace_root();
        let newest = [root.join("server/src"), root.join("shared/src")]
            .iter()
            .filter_map(|dir| newest_modification(dir))
            .max();
        if let Some((_, source)) = newest.filter(|(modified, _)| *modified > built) {
            // Written to the real stderr, not the test capture, so it is seen.
            let _ = writeln!(
                std::io::stderr(),
                "warning: {} is older than {}; run `cargo build -p server` \
                 or the harness tests an outdated server",
                binary.display(),
                source.display()
            );
        }
    });
}

/// The newest modification time of any file under `dir`, with that file.
fn newest_modification(dir: &Path) -> Option<(SystemTime, PathBuf)> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if let Ok(modified) = metadata.modified() {
                if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
                    newest = Some((modified, entry.path()));
                }
            }
        }
    }
    newest
}

/// Slugs of the committed avatar roster, read as plain JSON from
/// `client/assets/avatars/manifest.json` (the manifest a server launched from
/// this checkout validates against). The harness stays black-box: it never
/// links the server's roster loader.
pub fn roster_avatar_slugs() -> Vec<String> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../client/assets/avatars/manifest.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let manifest: serde_json::Value = serde_json::from_str(&raw).expect("avatar manifest JSON");
    manifest["avatars"]
        .as_array()
        .expect("avatar manifest lists avatars")
        .iter()
        .filter_map(|avatar| avatar["slug"].as_str().map(str::to_owned))
        .collect()
}

/// The longest roster slug: the worst case for snapshot size budgets.
pub fn longest_roster_avatar_slug() -> String {
    roster_avatar_slugs()
        .into_iter()
        .max_by_key(String::len)
        .expect("the shipped avatar roster must not be empty")
}
