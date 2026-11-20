//! Persistent, project-scoped Ekza connection. All HTTP uses the Ekza SDK.
// i18n-strict
use super::{Notice, approval_line};
use crate::i18n::{tr, trf};
use omoba_passport::{
    PairingState,
    account::{self, AccountCredential, AccountError, AccountFlow, AccountSession},
    store, weapon_store,
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
    error: Option<Notice>,
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

    /// Save and publish a new pairing. `client` reaches the registry that
    /// ends the saved link this pairing replaces, if any.
    fn accept_session(
        &mut self,
        session: AccountSession,
        client: impl FnOnce() -> Result<account::AccountClient, String>,
    ) {
        let replaced = self.path.as_deref().and_then(saved_credential);
        let saved = self
            .path
            .as_ref()
            .is_some_and(|path| session.credential().save(path).is_ok());
        self.error = (!saved).then_some(Notice::Key("account.error.not_saved"));
        // A link saved earlier (e.g. one a startup outage left unrestored) is
        // replaced now: end it on the registry instead of leaving it active.
        // Only once the new link is saved, so a failed save keeps the old one.
        // The SDK refuses, without sending it, a token for another server or game.
        if let Some(old) = replaced.filter(|old| saved && !same_link(old, &session.credential())) {
            if let Ok(api) = client() {
                api.disconnect(old);
            }
        }
        self.session = Some(session);
        self.last = Some(Instant::now());
        self.publish_library();
    }

    /// The weapons "My Ekza library" lists, none without a session. Lock order
    /// is always this state, then the weapon store's.
    fn library_weapons(&self) -> Vec<ekza_bevy_sdk::assets::StoreAsset> {
        self.session
            .as_ref()
            .map(weapon_store::owned)
            .unwrap_or_default()
    }

    fn publish_library(&self) {
        weapon_store::set_library(self.library_weapons());
    }

    fn sign_out(&mut self) {
        if let Some(path) = &self.path {
            if let Err(error) = std::fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    self.error = Some(Notice::Key("account.error.not_removed"));
                    return;
                }
            }
        }
        self.generation = self.generation.wrapping_add(1);
        // Also end this link on the registry. In the background and best
        // effort: signing out never waits for, or depends on, the network.
        if let Some(session) = self.session.take() {
            session.disconnect();
        }
        self.flow = None;
        self.worker = false;
        self.opened = None;
        self.error = None;
        self.last = Some(Instant::now());
        self.publish_library();
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
                // A claim made on the web appears on this read, without a restart.
                self.publish_library();
                true
            }
            Err(AccountError::Unauthorized) => {
                self.session = None;
                if let Some(path) = &self.path {
                    let _ = std::fs::remove_file(path);
                }
                self.error = Some(Notice::Key("account.error.expired"));
                self.publish_library();
                true
            }
            Err(AccountError::InvalidCredential) => {
                // Preserve the file for its own configured server/project.
                self.error = Some(Notice::Key("account.error.other_server"));
                false
            }
            Err(_) => {
                // SDK diagnostics are not needed in a public-facing menu.
                self.error = Some(Notice::Key("account.error.unavailable_retained"));
                false
            }
        }
    }
}

/// The connection saved at `path`; none when missing or unreadable.
fn saved_credential(path: &Path) -> Option<AccountCredential> {
    AccountCredential::load(path).ok().flatten()
}

/// Whether two credentials hold the same link (server, game and token). The
/// SDK keeps the fields private; the comparison stays in memory.
fn same_link(a: &AccountCredential, b: &AccountCredential) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

static STATE: Mutex<State> = Mutex::new(State::new());
const REFRESH: Duration = Duration::from_secs(15);

pub fn initialize(path: PathBuf) {
    STATE.lock().unwrap().path = Some(path);
}

pub fn account_connected() -> bool {
    STATE.lock().unwrap().session.is_some()
}

/// The menu button toggles between connecting and signing out of this
/// installation. A build without Ekza never connects.
pub fn connect_account() {
    if !crate::account_region::features().ekza {
        return;
    }
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
            state.error = crate::platform::open_external_url(&verification_url)
                .err()
                .map(Notice::Text);
        }
        return;
    }
    state.generation = state.generation.wrapping_add(1);
    state.worker = false;
    state.opened = None;
    state.error = None;
    match account::start() {
        Ok(flow) => state.flow = Some(flow),
        Err(_) => state.error = Some(Notice::Key("account.error.unavailable")),
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
        // A diagnostic: `finish_refresh` shows its own line for it.
        .map_err(|_| AccountError::Unavailable("Could not read the saved connection.".into()))?; // i18n-allow
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
                state.error = crate::platform::open_external_url(&verification_url)
                    .err()
                    .map(Notice::Text);
                state.opened = Some(verification_url);
            }
        }
        Some(PairingState::Connected) => {
            let session = state.flow.as_ref().and_then(AccountFlow::take_session);
            state.flow = None;
            if let Some(session) = session {
                state.accept_session(session, account::client);
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
        return tr("account.button.sign_out");
    }
    tr(match state.flow.as_ref().map(AccountFlow::state) {
        Some(PairingState::Starting) => "account.button.connecting",
        Some(PairingState::AwaitingApproval { .. }) => "account.button.approve",
        _ => "account.button.connect",
    })
}

pub fn account_status_line() -> String {
    let state = STATE.lock().unwrap();
    if let Some(error) = &state.error {
        return error.text();
    }
    if let Some(session) = &state.session {
        return trf(
            "account.status.connected",
            &[("username", &session.username)],
        );
    }
    match state.flow.as_ref().map(AccountFlow::state) {
        Some(PairingState::Starting) => tr("account.status.contacting").into(),
        Some(PairingState::AwaitingApproval {
            user_code,
            verification_url,
            ..
        }) => approval_line(&user_code, &verification_url),
        // The SDK's reason, then the client's advice.
        Some(PairingState::Failed(error)) => trf("account.status.failed", &[("error", &error)]),
        _ if state.worker => tr("account.status.restoring").into(),
        _ => tr("account.status.idle").into(),
    }
}

fn library_entries<'a>(
    avatars: impl IntoIterator<Item = &'a omoba_passport::avatars::AvatarDefinition>,
    mut has: impl FnMut(&str) -> bool,
    mut current_free: impl FnMut(&str) -> Option<bool>,
) -> Vec<&'a omoba_passport::avatars::AvatarDefinition> {
    avatars
        .into_iter()
        .filter(|avatar| current_free(&avatar.slug).unwrap_or(avatar.free) && has(&avatar.slug))
        .collect()
}

pub fn library_avatars() -> Vec<&'static omoba_passport::avatars::AvatarDefinition> {
    let state = STATE.lock().unwrap();
    let Some(session) = &state.session else {
        return Vec::new();
    };
    library_entries(
        omoba_passport::avatars::store_avatars(),
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
        body: Arc<Mutex<String>>,
        /// Held before each answer, after the request is recorded.
        delay: Arc<Mutex<Duration>>,
        /// "<method> <path> <authorization>" of every request, in order.
        requests: Arc<Mutex<Vec<String>>>,
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
            let body = Arc::new(Mutex::new(library(&[])));
            let delay = Arc::new(Mutex::new(Duration::ZERO));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (responses, bodies, stopping) = (status.clone(), body.clone(), stop.clone());
            let (delays, seen) = (delay.clone(), requests.clone());
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
                    // Accepted sockets can inherit O_NONBLOCK on macOS.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let read = stream.read(&mut request).unwrap();
                    assert!(read > 0);
                    seen.lock()
                        .unwrap()
                        .push(request_line(&String::from_utf8_lossy(&request[..read])));
                    thread::sleep(*delays.lock().unwrap());
                    let body = bodies.lock().unwrap().clone();
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
                body,
                delay,
                requests,
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
        fn session_with(&self, token: &str) -> AccountSession {
            self.api
                .session(token.into(), &omoba_passport::selector())
                .unwrap()
        }
        /// A new pairing, as `poll_account` accepts it.
        fn accept(&self, state: &mut State, token: &str) {
            state.accept_session(self.session_with(token), || Ok(self.api.clone()));
        }
        /// Whether `line` arrives within a few seconds (revokes run in the background).
        fn received(&self, line: &str) -> bool {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if self
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|seen| seen == line)
                {
                    return true;
                }
                thread::sleep(Duration::from_millis(5));
            }
            false
        }
        fn count(&self, line: &str) -> usize {
            let requests = self.requests.lock().unwrap();
            requests.iter().filter(|seen| *seen == line).count()
        }
        fn refresh(&self, state: &mut State) -> bool {
            let result =
                refresh_session(state.path.as_ref().unwrap(), state.session.clone(), || {
                    Ok(self.api.clone())
                });
            state.finish_refresh(state.generation, result)
        }
    }
    /// "<method> <path> <authorization or ->" of one HTTP request head.
    fn request_line(head: &str) -> String {
        let mut lines = head.lines();
        let mut start = lines.next().unwrap_or_default().split(' ');
        let (method, path) = (start.next().unwrap_or("-"), start.next().unwrap_or("-"));
        let authorization = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("authorization"))
            .map_or("-", |(_, value)| value.trim());
        format!("{method} {path} {authorization}")
    }
    const REVOKE_OLD: &str = "DELETE /v1/account/session Bearer old-fixture-token";
    const REVOKE_NEW: &str = "DELETE /v1/account/session Bearer new-fixture-token";

    /// A library document; `assets` is omitted when empty, as before C1.
    fn library(assets: &[String]) -> String {
        let assets = if assets.is_empty() {
            String::new()
        } else {
            format!(r#","assets":[{}]"#, assets.join(","))
        };
        format!(
            r#"{{"schema":"ekza.account.library.v1","projectId":"omoba","account":{{"username":"Saved player"}},"items":[]{assets}}}"#
        )
    }

    /// One `/v2/assets` weapon item approved for `platform`, as the library lists it.
    fn weapon(uuid: &str, name: &str, platform: &str, sha: char) -> String {
        let sha = sha.to_string().repeat(64);
        format!(
            r#"{{"id":"ekza:weapon:{uuid}","assetKind":"weapon","name":"{name}","license":{{"text":"CC-BY-4.0","attribution":"wildspark-maker"}},"creator":{{"name":"wildspark-maker"}},"access":"free","renditions":[{{"platform":"{platform}","profile":"handheld-glb-v1","format":"glb","sha256":"{sha}","sizeBytes":4096,"downloadUrl":"https://registry.ekza.io/v1/studio/assets/{sha}/rendition"}}],"projectSupport":[{{"projectId":"omoba","platform":"{platform}","profile":"handheld-glb-v1","status":"approved","requirementId":"wildspark.handheld.set"}}],"library":{{"reason":"claimed"}}}}"#
        )
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
        fixture.accept(&mut original, "local-fixture-token");
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
        fixture.accept(&mut state, "local-fixture-token");
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
        fixture.accept(&mut state, "local-fixture-token");
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
    fn a_web_claim_reaches_the_weapon_library_on_the_next_refresh() {
        const SET: &str = "11111111-2222-4333-8444-555555555555";
        let fixture = Fixture::new();
        let mut state = fixture.state();
        fixture.accept(&mut state, "local-fixture-token");
        assert!(state.library_weapons().is_empty());
        // Claimed on app.omoba.io: only the desktop handheld revision is listed
        // in the game; another platform's revision and bad entries are skipped.
        *fixture.body.lock().unwrap() = library(&[
            weapon(SET, "Candy Neon set", "desktop", 'c'),
            weapon(
                "22222222-3333-4444-8555-666666666666",
                "Phone only",
                "android",
                'd',
            ),
            r#"{"id":"ekza:avatar:aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa","assetKind":"weapon"}"#
                .into(),
            "null".into(),
        ]);
        assert!(fixture.refresh(&mut state));
        let owned = state.library_weapons();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].asset_id, format!("ekza:weapon:{SET}"));
        assert_eq!(owned[0].name, "Candy Neon set");
        assert_eq!(owned[0].author.as_deref(), Some("wildspark-maker"));
        assert_eq!(owned[0].license.as_deref(), Some("CC-BY-4.0"));
        assert!(owned[0].slug.starts_with("ekza-"));
        // The library item carries its own attribution line to the weapon store.
        assert_eq!(owned[0].attribution.as_deref(), Some("wildspark-maker"));
        // An outage keeps the listed weapons; a healthy empty library clears them.
        *fixture.status.lock().unwrap() = 503;
        assert!(!fixture.refresh(&mut state));
        assert_eq!(state.library_weapons(), owned);
        *fixture.status.lock().unwrap() = 200;
        *fixture.body.lock().unwrap() = library(&[]);
        assert!(fixture.refresh(&mut state));
        assert!(state.library_weapons().is_empty());
        *fixture.body.lock().unwrap() = library(&[weapon(SET, "Candy Neon set", "desktop", 'c')]);
        assert!(fixture.refresh(&mut state));
        assert_eq!(state.library_weapons(), owned);
        state.sign_out();
        assert!(state.library_weapons().is_empty());
    }

    #[test]
    fn signing_out_ends_this_link_on_the_registry_without_waiting_for_it() {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        fixture.accept(&mut state, "new-fixture-token");
        // A slow, failing registry must neither hold the menu nor keep the
        // player signed in.
        *fixture.status.lock().unwrap() = 503;
        *fixture.delay.lock().unwrap() = Duration::from_millis(1500);
        let started = Instant::now();
        state.sign_out();
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
        assert!(state.session.is_none());
        assert!(state.error.is_none());
        assert!(!state.path.as_ref().unwrap().exists());
        // This link's own revoke route; the token travels in the header only.
        assert!(fixture.received(REVOKE_NEW));
        assert_eq!(fixture.count(REVOKE_NEW), 1);
        let requests = fixture.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .all(|seen| !seen.split(' ').nth(1).unwrap().contains("fixture-token")),
            "{requests:?}"
        );
    }

    #[test]
    fn a_new_pairing_ends_the_saved_link_it_replaces_and_no_other() {
        let fixture = Fixture::new();
        let mut first = fixture.state();
        // Nothing saved yet, so nothing to end: no registry client is built.
        first.accept_session(fixture.session_with("old-fixture-token"), || {
            panic!("no saved link to end")
        });
        drop(first);
        // A startup outage leaves the saved link unrestored; the player pairs again.
        let mut restarted = fixture.state();
        let path = restarted.path.clone().unwrap();
        fixture.accept(&mut restarted, "new-fixture-token");
        assert!(restarted.error.is_none());
        assert!(fixture.received(REVOKE_OLD));
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("new-fixture-token") && !saved.contains("old-fixture-token"));
        // Accepting the saved link itself again never ends it.
        restarted.accept_session(fixture.session_with("new-fixture-token"), || {
            panic!("the saved link is the accepted one")
        });
        // A saved link of another game on the same server is refused by the
        // SDK: its token is never sent.
        let mut other =
            serde_json::to_value(fixture.session_with("other-game-token").credential()).unwrap();
        other["project_id"] = "other-game".into();
        serde_json::from_value::<AccountCredential>(other)
            .unwrap()
            .save(&path)
            .unwrap();
        fixture.accept(&mut restarted, "new-fixture-token");
        restarted.sign_out();
        assert!(fixture.received(REVOKE_NEW));
        // Revokes run in the background: give a stray one time to arrive.
        thread::sleep(Duration::from_millis(200));
        assert_eq!(fixture.count(REVOKE_OLD), 1);
        assert_eq!(fixture.count(REVOKE_NEW), 1);
        let requests = fixture.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .all(|seen| !seen.contains("other-game-token")
                    || seen.starts_with("GET /v1/account/library ")),
            "{requests:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_pairing_that_cannot_be_saved_keeps_the_saved_link() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let mut state = fixture.state();
        fixture.accept(&mut state, "old-fixture-token");
        let path = state.path.clone().unwrap();
        let directory = path.parent().unwrap();
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        state.accept_session(fixture.session_with("new-fixture-token"), || {
            panic!("the saved link is still the only one on disk")
        });
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            state.error,
            Some(Notice::Key("account.error.not_saved"))
        ));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("old-fixture-token")
        );
    }

    #[test]
    fn library_uses_current_entitlements_and_requires_account_membership() {
        let mut originally_free = omoba_passport::avatars::avatar_roster()[0].clone();
        originally_free.free = true;
        let mut originally_paid = omoba_passport::avatars::avatar_roster()[1].clone();
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
