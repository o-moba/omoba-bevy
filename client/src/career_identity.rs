//! Device-held signing key and a server/session-bound career handshake.
//! The signing key is never a packet, log field, UI value or public profile ID.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use bevy::prelude::*;
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use shared::career::{AuthChallenge, CareerRequest, authorized_signing_bytes, normalize_nickname};

use crate::{
    career::{CareerClient, CareerUiSet, NicknameChanged},
    net::{ClientNetPipeline, ClientSession, GameStateSnapshot, NetworkCommand},
    persistence::ClientSessionId,
};

const KEY_FILE: &str = "career_identity.json";
const NAME_FILE: &str = "career_nickname.json";
const RETRY: Duration = Duration::from_secs(2);
const AUTH_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    version: u32,
    seed: String,
    public_key: String,
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}

fn decode_key(raw: &str) -> Option<[u8; 32]> {
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn read_key(path: &Path) -> Result<SigningKey, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "Cannot read the saved profile key.")?;
    if !metadata.is_file() || metadata.len() > 1024 {
        return Err("The saved profile key is invalid; it has been preserved.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("The saved profile key needs private file permissions (0600).".into());
        }
    }
    let bytes = fs::read(path).map_err(|_| "Cannot read the saved profile key.")?;
    let saved: IdentityFile = serde_json::from_slice(&bytes)
        .map_err(|_| "The saved profile key is damaged; it has been preserved.")?;
    let seed = decode_key(&saved.seed)
        .ok_or("The saved profile key is damaged; it has been preserved.")?;
    let key = SigningKey::from_bytes(&seed);
    if saved.version != 1 || saved.public_key != hex(key.verifying_key().as_bytes()) {
        return Err("The saved profile key is damaged; it has been preserved.".into());
    }
    Ok(key)
}

fn private_create(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Publish a complete key with an exclusive hard link. Concurrent clients use
/// the winning file; neither an existing nor a corrupt identity is overwritten.
fn load_or_create_key(directory: &Path) -> Result<SigningKey, String> {
    let path = directory.join(KEY_FILE);
    match fs::symlink_metadata(&path) {
        Ok(_) => return read_key(&path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("Cannot access the saved profile key.".into()),
    }
    fs::create_dir_all(directory).map_err(|_| "Cannot create private profile storage.")?;
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|_| "Secure randomness is unavailable.")?;
    let key = SigningKey::from_bytes(&seed);
    let mut suffix = [0; 16];
    getrandom::fill(&mut suffix).map_err(|_| "Secure randomness is unavailable.")?;
    let temporary = directory.join(format!(".career-key-{}.tmp", hex(&suffix)));
    let mut created = false;
    let result = (|| {
        let mut file = private_create(&temporary).map_err(|_| "Cannot create the profile key.")?;
        created = true;
        let saved = IdentityFile {
            version: 1,
            seed: hex(&seed),
            public_key: hex(key.verifying_key().as_bytes()),
        };
        let encoded = serde_json::to_vec(&saved).map_err(|_| "Cannot encode the profile key.")?;
        file.write_all(&encoded)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot save the profile key.")?;
        match fs::hard_link(&temporary, &path) {
            Ok(()) => {
                #[cfg(unix)]
                fs::File::open(directory)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|_| "Cannot confirm profile key storage.")?;
                Ok(key)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => read_key(&path),
            Err(_) => Err("Cannot publish the profile key safely on this filesystem.".into()),
        }
    })();
    // Only this invocation's random temporary file can be removed here.
    if created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn identity_directory() -> Option<PathBuf> {
    crate::platform::preferences_file_path(
        std::env::var("OMOBA_CLIENT_CONFIG_DIR").ok().as_deref(),
        crate::platform::preferences_directory(),
        KEY_FILE,
    )
    .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn save_nickname(directory: &Path, nickname: &str) -> Result<(), String> {
    let nickname = normalize_nickname(nickname).map_err(str::to_owned)?;
    let mut suffix = [0; 16];
    getrandom::fill(&mut suffix).map_err(|_| "Cannot prepare nickname storage.")?;
    let temporary = directory.join(format!(".career-name-{}.tmp", hex(&suffix)));
    let mut created = false;
    let result = (|| {
        let mut file = private_create(&temporary).map_err(|_| "Cannot save the nickname.")?;
        created = true;
        let bytes = serde_json::to_vec(&nickname).map_err(|_| "Cannot save the nickname.")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot save the nickname.")?;
        fs::rename(&temporary, directory.join(NAME_FILE))
            .map_err(|_| "Cannot save the nickname.")?;
        Ok(())
    })();
    if created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[derive(Clone, PartialEq)]
struct AuthScope {
    server_addr: String,
    server_epoch: u64,
    session_id: String,
}

#[derive(Resource, Default)]
pub(crate) struct CareerIdentity {
    key: Option<SigningKey>,
    directory: Option<PathBuf>,
    error: Option<String>,
    scope: Option<AuthScope>,
    requested_nickname: String,
    challenge_requested: bool,
    signed_challenge: Option<AuthChallenge>,
    authenticate_sent: bool,
    auth_nonce: Option<String>,
    sequence: u64,
    started: Option<Instant>,
    next_retry: Option<Instant>,
    saved_nickname: String,
    failed_nickname: Option<String>,
    pending_rename: Option<(String, u64, String)>,
}

impl CareerIdentity {
    pub(crate) fn public_key(&self) -> Result<String, String> {
        self.key
            .as_ref()
            .map(|key| hex(key.verifying_key().as_bytes()))
            .ok_or_else(|| "The saved game identity is not ready.".into())
    }

    /// Only a validated portal challenge can be signed; the device key stays private.
    pub(crate) fn sign_web_pair(
        &self,
        challenge: shared::web_account::WebPairChallenge,
        decision: shared::web_account::WebPairDecision,
        trusted_origin: &str,
        now_secs: u64,
    ) -> Result<shared::web_account::SignedWebPair, String> {
        let public_key = self.public_key()?;
        challenge
            .validate(trusted_origin, &public_key, now_secs)
            .map_err(str::to_owned)?;
        let key = self
            .key
            .as_ref()
            .ok_or("The saved game identity is not ready.")?;
        let signature = hex(&key.sign(&challenge.signing_bytes(decision)).to_bytes());
        Ok(shared::web_account::SignedWebPair {
            challenge,
            decision,
            signature,
        })
    }

    fn invalidate(&mut self) {
        self.scope = None;
        self.signed_challenge = None;
        self.challenge_requested = false;
        self.authenticate_sent = false;
        self.auth_nonce = None;
        self.sequence = 0;
        self.started = None;
        self.next_retry = None;
    }

    fn matches_challenge(&self, challenge: &AuthChallenge) -> bool {
        self.challenge_requested
            && self.key.as_ref().is_some_and(|key| {
                challenge.public_key == hex(key.verifying_key().as_bytes())
                    && challenge.nickname == self.requested_nickname
                    && decode_key(&challenge.nonce).is_some()
                    && self.scope.as_ref().is_some_and(|scope| {
                        challenge.server_epoch == scope.server_epoch
                            && challenge.session_id == scope.session_id
                    })
            })
    }

    pub(crate) fn authenticated_for_scope(
        &self,
        server_addr: &str,
        server_epoch: u64,
        session_id: &str,
    ) -> bool {
        self.auth_nonce.is_some()
            && self.scope.as_ref().is_some_and(|scope| {
                scope.server_addr == server_addr
                    && scope.server_epoch == server_epoch
                    && scope.session_id == session_id
            })
    }

    pub(crate) fn prepare_request(
        &mut self,
        request: &CareerRequest,
        server_addr: &str,
        server_epoch: u64,
        session_id: &str,
    ) -> Result<CareerRequest, String> {
        let key = self.key.as_ref().ok_or_else(|| {
            self.error
                .clone()
                .unwrap_or_else(|| "Profile identity is unavailable.".into())
        })?;
        if !self.scope.as_ref().is_some_and(|scope| {
            scope.server_addr == server_addr
                && scope.server_epoch == server_epoch
                && scope.session_id == session_id
        }) {
            return Err("The profile connection is not ready. Try again shortly.".into());
        }
        match request {
            CareerRequest::Challenge {
                public_key,
                nickname,
                session_id,
            } if public_key == &hex(key.verifying_key().as_bytes())
                && nickname == &self.requested_nickname
                && self
                    .scope
                    .as_ref()
                    .is_some_and(|scope| &scope.session_id == session_id)
                && self.auth_nonce.is_none() =>
            {
                self.challenge_requested = true;
                return Ok(request.clone());
            }
            CareerRequest::Authenticate {
                challenge,
                signature,
            } if self.matches_challenge(challenge)
                && self.signed_challenge.as_ref() == Some(challenge)
                && signature == &hex(&key.sign(&challenge.signing_bytes()).to_bytes())
                && self.auth_nonce.is_none() =>
            {
                self.authenticate_sent = true;
                return Ok(request.clone());
            }
            CareerRequest::Challenge { .. }
            | CareerRequest::Authenticate { .. }
            | CareerRequest::Authorized { .. } => {
                return Err("Unexpected profile authentication request.".into());
            }
            _ => {}
        }
        let nonce = self
            .auth_nonce
            .as_ref()
            .ok_or("Your profile is still connecting. Try again shortly.")?;
        let action = request
            .account_action()
            .ok_or("Unexpected profile request.")?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Reconnect to refresh your profile session.")?;
        let signature = hex(&key
            .sign(&authorized_signing_bytes(
                server_epoch,
                nonce,
                self.sequence,
                &action,
            ))
            .to_bytes());
        Ok(CareerRequest::Authorized {
            session_nonce: nonce.clone(),
            sequence: self.sequence,
            action,
            signature,
        })
    }
}

pub(crate) struct CareerIdentityPlugin;
impl Plugin for CareerIdentityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CareerIdentity>()
            .add_systems(
                Startup,
                load_identity.after(crate::persistence::load_persistent_client_settings),
            )
            .add_systems(
                Update,
                identity_driver
                    .after(CareerUiSet)
                    .after(ClientNetPipeline::ApplySnapshot)
                    .before(ClientNetPipeline::SendCommands),
            );
    }
}

fn load_identity(mut identity: ResMut<CareerIdentity>, mut career: ResMut<CareerClient>) {
    if std::env::var_os("OMOBA_CAREER_QA_OUTPUT").is_some_and(|value| !value.is_empty()) {
        return;
    }
    let Some(directory) = identity_directory() else {
        let message = "Private profile storage is unavailable on this device.".to_owned();
        identity.error = Some(message.clone());
        career.request_failed(message);
        return;
    };
    match load_or_create_key(&directory) {
        Ok(key) => identity.key = Some(key),
        Err(error) => {
            career.request_failed(error.clone());
            identity.error = Some(error);
            return;
        }
    }
    if let Ok(bytes) = fs::read(directory.join(NAME_FILE))
        && let Ok(name) = serde_json::from_slice::<String>(&bytes)
        && let Ok(name) = normalize_nickname(&name)
    {
        career.nickname = name;
    }
    identity.saved_nickname.clone_from(&career.nickname);
    identity.directory = Some(directory);
}

fn identity_driver(
    mut identity: ResMut<CareerIdentity>,
    mut career: ResMut<CareerClient>,
    session: Res<ClientSession>,
    session_id: Res<ClientSessionId>,
    snapshot: Res<GameStateSnapshot>,
    mut names: MessageReader<NicknameChanged>,
    mut requests: MessageWriter<NetworkCommand>,
) {
    if std::env::var_os("OMOBA_CAREER_QA_OUTPUT").is_some_and(|value| !value.is_empty()) {
        names.clear();
        return;
    }
    for NicknameChanged(name) in names.read() {
        let request_id = career.rename_request_id();
        identity.pending_rename = Some((
            session.server_addr_display.clone(),
            request_id,
            name.clone(),
        ));
        if identity.auth_nonce.is_none() {
            // Keep the chosen name for the next authenticated login, not a fake
            // server-side rename while the profile service is unavailable.
            identity.invalidate();
            if let Some(directory) = &identity.directory {
                if let Err(error) = save_nickname(directory, name) {
                    career.request_failed(error);
                } else {
                    identity.saved_nickname.clone_from(name);
                }
            }
        }
    }
    if identity.key.is_none() || !session.is_connected() || snapshot.meta.server_epoch == 0 {
        identity.invalidate();
        return;
    }
    let scope = AuthScope {
        server_addr: session.server_addr_display.clone(),
        server_epoch: snapshot.meta.server_epoch,
        session_id: session_id.0.clone(),
    };
    if identity.scope.as_ref() != Some(&scope) {
        identity.invalidate();
        identity.scope = Some(scope);
        identity.requested_nickname =
            normalize_nickname(&career.nickname).unwrap_or_else(|_| "Player".into());
        identity.started = Some(Instant::now());
    }
    if let Some(challenge) = career.view.challenge.as_ref()
        && identity.auth_nonce.is_none()
        && identity.matches_challenge(challenge)
        && identity.signed_challenge.as_ref() != Some(challenge)
    {
        identity.signed_challenge = Some(challenge.clone());
        identity.authenticate_sent = false;
        identity.next_retry = None;
    }
    if let Some(challenge) = &identity.signed_challenge
        && identity.authenticate_sent
        && career.view.auth_nonce.as_ref() == Some(&challenge.nonce)
        && career.view.profile.is_some()
    {
        identity.auth_nonce = Some(challenge.nonce.clone());
    }
    if identity.auth_nonce.is_some() {
        if let Some((server_addr, request_id, nickname)) = identity.pending_rename.take()
            && server_addr == session.server_addr_display
        {
            requests.write(NetworkCommand::Career(CareerRequest::Rename {
                request_id,
                nickname,
            }));
        }
        if let Some(profile) = &career.view.profile
            && profile.nickname != identity.saved_nickname
            && identity.failed_nickname.as_ref() != Some(&profile.nickname)
            && let Some(directory) = &identity.directory
        {
            match save_nickname(directory, &profile.nickname) {
                Ok(()) => {
                    identity.saved_nickname.clone_from(&profile.nickname);
                    identity.failed_nickname = None;
                }
                Err(error) => {
                    identity.failed_nickname = Some(profile.nickname.clone());
                    career.request_failed(error);
                }
            }
        }
        return;
    }
    let now = Instant::now();
    if identity
        .started
        .is_some_and(|started| now.duration_since(started) >= AUTH_TIMEOUT)
    {
        if identity.next_retry.is_some() {
            career.request_failed("Profile sign-in timed out. Reconnect to try again.".into());
            identity.next_retry = None;
        }
        return;
    }
    if identity.next_retry.is_some_and(|next| now < next) {
        return;
    }
    let key = identity.key.as_ref().expect("checked above");
    let request = if let Some(challenge) = &identity.signed_challenge {
        CareerRequest::Authenticate {
            challenge: challenge.clone(),
            signature: hex(&key.sign(&challenge.signing_bytes()).to_bytes()),
        }
    } else {
        CareerRequest::Challenge {
            public_key: hex(key.verifying_key().as_bytes()),
            nickname: identity.requested_nickname.clone(),
            session_id: session_id.0.clone(),
        }
    };
    requests.write(NetworkCommand::Career(request));
    identity.next_retry = Some(now + RETRY);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary_directory() -> PathBuf {
        let mut suffix = [0; 16];
        getrandom::fill(&mut suffix).unwrap();
        std::env::temp_dir().join(format!("omoba-identity-test-{}", hex(&suffix)))
    }
    #[test]
    fn key_survives_reopen_and_corruption_is_not_overwritten() {
        let directory = temporary_directory();
        let first = load_or_create_key(&directory).unwrap();
        let second = load_or_create_key(&directory).unwrap();
        assert_eq!(first.verifying_key(), second.verifying_key());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(directory.join(KEY_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::write(directory.join(KEY_FILE), b"damaged").unwrap();
        assert!(load_or_create_key(&directory).is_err());
        assert_eq!(fs::read(directory.join(KEY_FILE)).unwrap(), b"damaged");
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn competing_creators_use_one_complete_identity() {
        let directory = temporary_directory();
        let other = directory.clone();
        let thread = std::thread::spawn(move || load_or_create_key(&other).unwrap());
        let first = load_or_create_key(&directory).unwrap();
        let second = thread.join().unwrap();
        assert_eq!(first.verifying_key(), second.verifying_key());
        fs::remove_dir_all(directory).unwrap();
    }
    fn identity() -> CareerIdentity {
        CareerIdentity {
            key: Some(SigningKey::from_bytes(&[7; 32])),
            scope: Some(AuthScope {
                server_addr: "localhost:4000".into(),
                server_epoch: 42,
                session_id: "session".into(),
            }),
            requested_nickname: "小明".into(),
            challenge_requested: true,
            ..default()
        }
    }
    #[test]
    fn unexpected_challenges_and_namespace_changes_are_refused() {
        let mut identity = identity();
        let challenge = AuthChallenge {
            public_key: hex(identity.key.as_ref().unwrap().verifying_key().as_bytes()),
            nonce: "a".repeat(64),
            server_epoch: 42,
            session_id: "session".into(),
            nickname: "小明".into(),
        };
        identity.challenge_requested = false;
        assert!(!identity.matches_challenge(&challenge));
        identity.challenge_requested = true;
        assert!(identity.matches_challenge(&challenge));
        for field in 0..5 {
            let mut wrong = challenge.clone();
            match field {
                0 => wrong.public_key = "b".repeat(64),
                1 => wrong.nickname = "other".into(),
                2 => wrong.session_id = "other".into(),
                3 => wrong.server_epoch += 1,
                _ => wrong.nonce = "invalid".into(),
            }
            assert!(!identity.matches_challenge(&wrong));
        }
        let mut identity = identity;
        identity.auth_nonce = Some(challenge.nonce);
        let request = CareerRequest::Friends { request_id: 1 };
        assert!(
            identity
                .prepare_request(&request, "other:4000", 42, "session")
                .is_err()
        );
        assert!(
            identity
                .prepare_request(&request, "localhost:4000", 43, "session")
                .is_err()
        );
        assert!(
            identity
                .prepare_request(&request, "localhost:4000", 42, "other-session")
                .is_err()
        );
    }
    #[test]
    fn signed_actions_have_independent_monotonic_sequence_and_verify() {
        use ed25519_dalek::{Signature, Verifier};
        let mut identity = identity();
        assert!(!identity.authenticated_for_scope("localhost:4000", 42, "session"));
        identity.auth_nonce = Some("a".repeat(64));
        assert!(identity.authenticated_for_scope("localhost:4000", 42, "session"));
        assert!(!identity.authenticated_for_scope("other:4000", 42, "session"));
        assert!(!identity.authenticated_for_scope("localhost:4000", 43, "session"));
        assert!(!identity.authenticated_for_scope("localhost:4000", 42, "other-session"));
        for sequence in 1..=2 {
            let request = if sequence == 1 {
                CareerRequest::Friends { request_id: 1 }
            } else {
                CareerRequest::Social {
                    request: shared::social::SocialRequest {
                        request_id: 2,
                        server_epoch: 42,
                        match_id: 1,
                        session_id: "session".into(),
                        command: shared::social::SocialCommand::Chat {
                            channel: shared::social::SocialChannel::Team,
                            text: "Hello".into(),
                        },
                    },
                }
            };
            let packet = identity
                .prepare_request(&request, "localhost:4000", 42, "session")
                .unwrap();
            let CareerRequest::Authorized {
                session_nonce,
                sequence: actual,
                action,
                signature,
            } = packet
            else {
                panic!("not authorized")
            };
            assert_eq!(actual, sequence);
            let mut bytes = [0; 64];
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&signature[index * 2..index * 2 + 2], 16).unwrap();
            }
            identity
                .key
                .as_ref()
                .unwrap()
                .verifying_key()
                .verify(
                    &authorized_signing_bytes(42, &session_nonce, actual, &action),
                    &Signature::from_bytes(&bytes),
                )
                .unwrap();
            assert!(
                identity
                    .key
                    .as_ref()
                    .unwrap()
                    .verifying_key()
                    .verify(
                        &authorized_signing_bytes(43, &session_nonce, actual, &action),
                        &Signature::from_bytes(&bytes)
                    )
                    .is_err()
            );
            assert!(
                identity
                    .prepare_request(
                        &CareerRequest::Authorized {
                            session_nonce,
                            sequence: actual,
                            action,
                            signature
                        },
                        "localhost:4000",
                        42,
                        "session"
                    )
                    .is_err()
            );
        }
        identity.invalidate();
        assert!(
            identity
                .prepare_request(
                    &CareerRequest::Friends { request_id: 2 },
                    "localhost:4000",
                    42,
                    "session"
                )
                .is_err()
        );
    }
}
