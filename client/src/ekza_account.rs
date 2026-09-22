//! Persistent, project-scoped Ekza connection. All HTTP uses the Ekza SDK.
use omoba_passport::{
    PairingState,
    account::{self, AccountCredential, AccountError, AccountFlow, AccountSession},
    store,
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

struct State {
    session: Option<AccountSession>,
    flow: Option<AccountFlow>,
    path: Option<PathBuf>,
    generation: u64,
    worker: bool,
    last: Option<Instant>,
    opened: Option<String>,
    error: Option<String>,
}
impl State {
    const fn new() -> Self {
        Self {
            session: None,
            flow: None,
            path: None,
            generation: 0,
            worker: false,
            last: None,
            opened: None,
            error: None,
        }
    }

    fn accept_session(&mut self, session: AccountSession) {
        self.error = match &self.path {
            Some(path) if session.credential().save(path).is_ok() => None,
            _ => Some("Connected for this run; could not save the connection.".into()),
        };
        self.session = Some(session);
        self.last = Some(Instant::now());
    }

    fn sign_out(&mut self) {
        if let Some(path) = &self.path {
            if let Err(error) = std::fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    self.error =
                        Some("Could not remove the saved connection. Retry sign out.".into());
                    return;
                }
            }
        }
        self.generation = self.generation.wrapping_add(1);
        self.session = None;
        self.flow = None;
        self.worker = false;
        self.opened = None;
        self.error = None;
        self.last = Some(Instant::now());
    }

    /// A response from before logout or a new pairing must never restore identity.
    /// Returns true only when the current result changed the account/library.
    fn finish_refresh(
        &mut self,
        generation: u64,
        result: Result<Option<AccountSession>, AccountError>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.worker = false;
        match result {
            Ok(session) => {
                self.session = session;
                self.error = None;
                true
            }
            Err(AccountError::Unauthorized) => {
                self.session = None;
                if let Some(path) = &self.path {
                    let _ = std::fs::remove_file(path);
                }
                self.error = Some("Ekza connection expired or was revoked. Connect again.".into());
                true
            }
            Err(AccountError::InvalidCredential) => {
                // Preserve the file for its own configured server/project.
                self.error =
                    Some("Saved Ekza connection belongs to another server or game.".into());
                false
            }
            Err(_) => {
                // SDK diagnostics are not needed in a public-facing menu.
                self.error =
                    Some("Ekza is temporarily unavailable. Your connection is retained.".into());
                false
            }
        }
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());
const REFRESH: Duration = Duration::from_secs(15);

pub fn initialize(path: PathBuf) {
    STATE.lock().unwrap().path = Some(path);
}

pub fn account_connected() -> bool {
    STATE.lock().unwrap().session.is_some()
}

/// The menu button toggles between connecting and signing out of this installation.
pub fn connect_account() {
    let mut state = STATE.lock().unwrap();
    if state.session.is_some() {
        state.sign_out();
        return;
    }
    if let Some(flow) = state.flow.as_ref().filter(|flow| flow.in_progress()) {
        if let PairingState::AwaitingApproval {
            verification_url, ..
        } = flow.state()
        {
            state.error = crate::platform::open_external_url(&verification_url).err();
        }
        return;
    }
    state.generation = state.generation.wrapping_add(1);
    state.worker = false;
    state.opened = None;
    state.error = None;
    match account::start() {
        Ok(flow) => state.flow = Some(flow),
        Err(_) => state.error = Some("Ekza account is unavailable. Connect again to retry.".into()),
    }
}

fn refresh_session(
    path: &Path,
    active: Option<AccountSession>,
    client: impl FnOnce() -> Result<account::AccountClient, String>,
) -> Result<Option<AccountSession>, AccountError> {
    if let Some(mut session) = active {
        session.refresh_checked()?;
        return Ok(Some(session));
    }
    let saved = AccountCredential::load(path)
        .map_err(|_| AccountError::Unavailable("Could not read the saved connection.".into()))?;
    let Some(saved) = saved else {
        return Ok(None);
    };
    let api = client().map_err(AccountError::Unavailable)?;
    api.restore(&saved, &omoba_passport::selector()).map(Some)
}

pub fn poll_account() -> bool {
    let mut state = STATE.lock().unwrap();
    match state.flow.as_ref().map(AccountFlow::state) {
        Some(PairingState::AwaitingApproval {
            verification_url, ..
        }) => {
            if state.opened.as_ref() != Some(&verification_url) {
                state.error = crate::platform::open_external_url(&verification_url).err();
                state.opened = Some(verification_url);
            }
        }
        Some(PairingState::Connected) => {
            let session = state.flow.as_ref().and_then(AccountFlow::take_session);
            state.flow = None;
            if let Some(session) = session {
                state.accept_session(session);
                store::request_refresh();
                return true;
            }
        }
        Some(PairingState::Starting) => {}
        _ => {
            if state.worker || state.last.is_some_and(|at| at.elapsed() < REFRESH) {
                return false;
            }
            let Some(path) = state.path.clone() else {
                return false;
            };
            let generation = state.generation;
            let active = state.session.clone();
            state.worker = true;
            state.last = Some(Instant::now());
            std::thread::spawn(move || {
                let result = refresh_session(&path, active, account::client);
                if STATE.lock().unwrap().finish_refresh(generation, result) {
                    store::request_refresh();
                }
            });
        }
    }
    false
}

pub fn account_button_label() -> &'static str {
    let state = STATE.lock().unwrap();
    if state.session.is_some() {
        return "Sign out of Ekza";
    }
    match state.flow.as_ref().map(AccountFlow::state) {
        Some(PairingState::Starting) => "Connecting Ekza…",
        Some(PairingState::AwaitingApproval { .. }) => "Open Ekza approval",
        _ => "Connect Ekza",
    }
}

pub fn account_status_line() -> String {
    let state = STATE.lock().unwrap();
    if let Some(error) = &state.error {
        return error.clone();
    }
    if let Some(session) = &state.session {
        return format!(
            "Ekza account {} · connected on this device",
            session.username
        );
    }
    match state.flow.as_ref().map(AccountFlow::state) {
        Some(PairingState::Starting) => "Contacting Ekza…".into(),
        Some(PairingState::AwaitingApproval {
            user_code,
            verification_url,
            ..
        }) => format!(
            "{} · code {user_code} · {verification_url}",
            crate::platform::browser_approval_hint()
        ),
        Some(PairingState::Failed(error)) => format!("{error} · connect again to retry"),
        _ if state.worker => "Restoring Ekza connection…".into(),
        _ => "Connect your Ekza account to see your own library · no wallet needed".into(),
    }
}

fn library_entries<'a>(
    avatars: impl IntoIterator<Item = &'a shared::AvatarDefinition>,
    mut has: impl FnMut(&str) -> bool,
    mut current_free: impl FnMut(&str) -> Option<bool>,
) -> Vec<&'a shared::AvatarDefinition> {
    avatars
        .into_iter()
        .filter(|avatar| current_free(&avatar.slug).unwrap_or(avatar.free) && has(&avatar.slug))
        .collect()
}

pub fn library_avatars() -> Vec<&'static shared::AvatarDefinition> {
    let state = STATE.lock().unwrap();
    let Some(session) = &state.session else {
        return Vec::new();
    };
    library_entries(
        shared::store_avatars(),
        |slug| session.has(slug),
        store::free_access,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    /// Stop-aware loopback SDK endpoint; no real account or process-global state.
    struct Fixture {
        api: account::AccountClient,
        status: Arc<Mutex<u16>>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let api = account::AccountClient::new(&format!("http://{address}"), "omoba").unwrap();
            let status = Arc::new(Mutex::new(200));
            let stop = Arc::new(AtomicBool::new(false));
            let (responses, stopping) = (status.clone(), stop.clone());
            let worker = thread::spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("account fixture: {error}"),
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    assert!(stream.read(&mut request).unwrap() > 0);
                    let body = r#"{"schema":"ekza.account.library.v1","projectId":"omoba","account":{"username":"Saved player"},"items":[]}"#;
                    write!(
                        stream,
                        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        *responses.lock().unwrap(),
                        body.len()
                    )
                    .unwrap();
                }
            });
            Self {
                api,
                status,
                stop,
                worker: Some(worker),
                root: std::env::temp_dir().join(format!(
                    "omoba-account-{}-{}",
                    std::process::id(),
                    address.port()
                )),
            }
        }
        fn state(&self) -> State {
            State {
                path: Some(self.root.join("private/session.json")),
                ..State::new()
            }
        }
        fn session(&self) -> AccountSession {
            self.api
                .session("local-fixture-token".into(), &omoba_passport::selector())
                .unwrap()
        }
        fn refresh(&self, state: &mut State) -> bool {
            let result =
                refresh_session(state.path.as_ref().unwrap(), state.session.clone(), || {
                    Ok(self.api.clone())
                });
            state.finish_refresh(state.generation, result)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.worker.take().unwrap().join().unwrap();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn connection_restores_after_restart_and_logout_rejects_late_refresh() {
        let fixture = Fixture::new();
        let mut original = fixture.state();
        original.accept_session(fixture.session());
        let path = original.path.clone().unwrap();
        assert!(path.is_file());
        assert!(original.error.is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(original);
        let mut restarted = fixture.state();
        assert!(restarted.session.is_none());
        assert!(fixture.refresh(&mut restarted));
        assert_eq!(restarted.session.as_ref().unwrap().username, "Saved player");
        let delayed = restarted.session.clone();
        let generation = restarted.generation;
        restarted.sign_out();
        assert!(!path.exists());
        assert!(restarted.session.is_none());
        assert!(!restarted.finish_refresh(generation, Ok(delayed)));
        assert!(restarted.session.is_none());
        assert!(fixture.refresh(&mut restarted));
        assert!(restarted.session.is_none());
    }

    #[test]
    fn outage_preserves_session_and_credential_but_revocation_clears_both() {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        state.accept_session(fixture.session());
        *fixture.status.lock().unwrap() = 503;
        assert!(!fixture.refresh(&mut state));
        assert_eq!(state.session.as_ref().unwrap().username, "Saved player");
        assert!(state.path.as_ref().unwrap().is_file());
        // A startup outage must retain the saved file without trusting its identity.
        let mut restarted = fixture.state();
        assert!(!fixture.refresh(&mut restarted));
        assert!(restarted.session.is_none());
        assert!(restarted.path.as_ref().unwrap().is_file());
        *fixture.status.lock().unwrap() = 401;
        assert!(fixture.refresh(&mut state));
        assert!(state.session.is_none());
        assert!(!state.path.as_ref().unwrap().exists());
    }

    #[test]
    fn wrong_backend_scope_retains_the_saved_credential() {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        state.accept_session(fixture.session());
        state.session = None;
        let result = refresh_session(state.path.as_ref().unwrap(), None, || {
            account::AccountClient::new("http://127.0.0.1:1", "other-game")
        });
        assert!(matches!(result, Err(AccountError::InvalidCredential)));
        assert!(!state.finish_refresh(state.generation, result));
        assert!(state.path.as_ref().unwrap().is_file());
        assert!(state.session.is_none());
    }

    #[test]
    fn library_uses_current_entitlements_and_requires_account_membership() {
        let mut originally_free = shared::avatar_roster()[0].clone();
        originally_free.free = true;
        let mut originally_paid = shared::avatar_roster()[1].clone();
        originally_paid.free = false;
        let avatars = [&originally_free, &originally_paid];
        let fallback = library_entries(avatars, |_| true, |_| None);
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].slug, originally_free.slug);
        // The same immutable avatar definition must immediately reflect Registry changes.
        assert_eq!(library_entries(avatars, |_| true, |_| Some(false)).len(), 0);
        assert_eq!(library_entries(avatars, |_| true, |_| Some(true)).len(), 2);
        assert!(library_entries(avatars, |_| false, |_| Some(true)).is_empty());
    }
}
