//! Protect progress (LR-18): the account's protection in Profile, recovery codes
//! made in the game, and the Home card that asks a player to protect a profile.
//!
//! - **Status.** The game asks the Account API with its active device key
//!   (`POST /v1/auth/devices/recovery-codes`, `omoba.recovery.native.v1`): unused
//!   recovery codes, linked website sign-ins (VK ID) and active devices. A
//!   profile is protected when a sign-in is linked or a code is unused
//!   (`AccountProtection::protected`).
//! - **Codes.** The same route issues a new set of eight codes. They are shown
//!   once in the Protect modal, never stored or logged by the game, and cleared
//!   when the modal closes. A set that still has unused codes is replaced only
//!   after a second press.
//! - **Home card.** After the first completed match, and again after the first
//!   ranked match, a dismissible card asks to link VK ID or save a recovery code;
//!   a decline hides it for three completed matches, or until the first ranked
//!   match if that comes sooner. It stops when the profile is protected or after
//!   three declines ([`PromptRecord`], kept in the region folder as
//!   [`PROMPT_FILE`]).
//! - **Region.** The copy comes from the region's sign-in methods through
//!   [`vk_offered`], this module's one read of the market (LR-68 R0 swaps it for
//!   the bound region).
// i18n-strict
use super::*;
use crate::career_identity::CareerIdentity;
use omoba_passport::device_account::DeviceAccountApi;
use serde::{Deserialize, Serialize};
use shared::device_account::{AccountProtection, RecoveryAction};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};

/// The Home card's record in the region folder.
pub(crate) const PROMPT_FILE: &str = "protect_prompt.json";
/// After this many "Not now" presses the card stops coming back.
pub(crate) const MAX_DECLINES: u8 = 3;
/// A "Not now" lasts this many completed matches (the first ranked match ends it
/// sooner), so a player who never plays ranked is asked again too.
pub(crate) const REASK_AFTER_MATCHES: u32 = 3;
/// A record larger than this is not ours.
const MAX_RECORD_BYTES: u64 = 4096;
/// An automatic status read that failed waits this long before the next one.
const RETRY: Duration = Duration::from_secs(60);
/// A screen opened again within this time reuses the last answer.
const FRESH: Duration = Duration::from_secs(10);

/// Whether the region signs players in with VK ID: the prompt and the Protect
/// modal offer to link it. The module's one read of the market.
pub(crate) fn vk_offered() -> bool {
    devices::vk_button_visible(shared::market::FEATURES)
}

/// What the Protect modal and Profile show.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct ProtectionState {
    /// The last reply for this installation's key (never with codes).
    pub status: Option<AccountProtection>,
    /// Codes just issued: shown once, cleared when the modal closes.
    pub codes: Vec<String>,
    /// A screen asked for a fresh status read (Profile or Protect opened).
    pub wanted: bool,
    pub busy: bool,
    /// The last status read failed with this message.
    pub error: Option<String>,
    /// The result of the last code request.
    pub message: Option<String>,
    /// The next press replaces a set that still has unused codes.
    pub confirm_replace: bool,
}

/// How often the Home card was declined (the region folder's [`PROMPT_FILE`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PromptRecord {
    pub declines: u8,
    /// Completed matches when the card was last declined.
    pub declined_at_matches: u32,
    /// The last decline came after the first ranked match.
    pub declined_after_ranked: bool,
}
impl PromptRecord {
    /// The card is due for `profile` (when it is not protected): after the
    /// first completed match; after a decline, at the first ranked match or
    /// [`REASK_AFTER_MATCHES`] completed matches later, whichever comes first;
    /// never after [`MAX_DECLINES`].
    pub(crate) fn due(&self, profile: &ProfileSummary) -> bool {
        if self.declines >= MAX_DECLINES || profile.matches_played == 0 {
            return false;
        }
        if self.declines == 0 {
            return true;
        }
        let first_ranked = !self.declined_after_ranked && profile.rated_matches > 0;
        first_ranked
            || profile.matches_played
                >= self.declined_at_matches.saturating_add(REASK_AFTER_MATCHES)
    }
    fn decline(&mut self, profile: &ProfileSummary) {
        self.declines = self.declines.saturating_add(1).min(MAX_DECLINES);
        self.declined_at_matches = profile.matches_played;
        self.declined_after_ranked = profile.rated_matches > 0;
    }
}

/// The Home card's state of this run and its file.
#[derive(Resource)]
pub(crate) struct ProtectPrompt {
    record: PromptRecord,
    file: Option<PathBuf>,
}
impl ProtectPrompt {
    /// The record in this region's settings folder.
    pub(crate) fn load() -> Self {
        Self::at(crate::platform::settings_directory().map(|directory| directory.join(PROMPT_FILE)))
    }
    pub(crate) fn at(file: Option<PathBuf>) -> Self {
        Self {
            record: file.as_deref().and_then(read_record).unwrap_or_default(),
            file,
        }
    }
    #[cfg(test)]
    pub(crate) fn record(&self) -> PromptRecord {
        self.record
    }
    /// "Not now": counts a decline and saves it. A record that cannot be
    /// written still counts for this run.
    pub(crate) fn decline(&mut self, profile: &ProfileSummary) {
        self.record.decline(profile);
        if let Some(path) = self.file.as_deref() {
            if let Err(error) = write_record(path, &self.record) {
                warn!("Protect-progress prompt not saved at {path:?}: {error}");
            }
        }
    }
}
fn read_record(path: &Path) -> Option<PromptRecord> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_RECORD_BYTES {
        return None;
    }
    serde_json::from_reader(file).ok()
}
fn write_record(path: &Path, record: &PromptRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(record).map_err(std::io::Error::other)?;
    crate::persistence::write_atomically(path, &data)
}

/// Whether Home shows the card now: the profile played, the card is due, and
/// the account service says the profile is not protected yet.
pub(crate) fn home_card(career: &CareerClient, prompt: Option<&ProtectPrompt>) -> bool {
    let (Some(profile), Some(prompt)) = (career.view.profile.as_ref(), prompt) else {
        return false;
    };
    prompt.record.due(profile)
        && career
            .protection
            .status
            .as_ref()
            .is_some_and(|status| !status.protected())
}

type Reply = Result<AccountProtection, String>;

/// The request in flight (one at a time) and the automatic-read backoff.
#[derive(Resource, Default)]
pub(crate) struct Worker {
    pending: Option<(RecoveryAction, Mutex<mpsc::Receiver<Reply>>)>,
    retry_at: Option<Instant>,
    /// When the last answer arrived.
    answered_at: Option<Instant>,
    /// Tests answer here instead of the Account API.
    #[cfg(test)]
    pub(crate) fake: Option<fn(RecoveryAction) -> Reply>,
}
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn launch(
    worker: &mut Worker,
    identity: &CareerIdentity,
    action: RecoveryAction,
) -> Result<(), String> {
    let (tx, rx) = mpsc::sync_channel(1);
    #[cfg(test)]
    if let Some(fake) = worker.fake {
        let _ = tx.send(fake(action));
        worker.pending = Some((action, Mutex::new(rx)));
        return Ok(());
    }
    let api = DeviceAccountApi::from_env()?;
    let proof = identity.sign_recovery_request(action, api.origin(), now_secs())?;
    worker.pending = Some((action, Mutex::new(rx)));
    std::thread::spawn(move || {
        let _ = tx.send(api.recovery(&proof));
    });
    Ok(())
}
fn start(
    career: &mut CareerClient,
    worker: &mut Worker,
    identity: &CareerIdentity,
    action: RecoveryAction,
) {
    let state = &mut career.protection;
    state.wanted = false;
    match launch(worker, identity, action) {
        Ok(()) => {
            state.busy = true;
            if action == RecoveryAction::Rotate {
                state.message = None;
            }
        }
        Err(error) => finish(career, worker, action, Err(error)),
    }
}
fn finish(career: &mut CareerClient, worker: &mut Worker, action: RecoveryAction, reply: Reply) {
    let state = &mut career.protection;
    state.busy = false;
    match (action, reply) {
        (_, Ok(mut status)) => {
            let codes = std::mem::take(&mut status.recovery_codes);
            if action == RecoveryAction::Rotate {
                state.codes = codes;
                state.confirm_replace = false;
                state.message = None;
            }
            state.status = Some(status);
            state.error = None;
            worker.answered_at = Some(Instant::now());
            worker.retry_at = None;
        }
        (RecoveryAction::Status, Err(error)) => {
            state.error = Some(error);
            worker.retry_at = Some(Instant::now() + RETRY);
        }
        (RecoveryAction::Rotate, Err(error)) => {
            state.confirm_replace = false;
            state.message = Some(error);
        }
    }
}

/// Protect actions from the career UI.
pub(super) fn act(
    action: &Action,
    career: &mut CareerClient,
    identity: &CareerIdentity,
    worker: &mut Worker,
) {
    match action {
        Action::ProtectOpen => career.open_protect_modal(),
        Action::ProtectRotate if !career.protection.busy => {
            let Some(status) = career.protection.status.as_ref() else {
                career.protection.wanted = true;
                return;
            };
            if status.recovery_codes_remaining > 0 && !career.protection.confirm_replace {
                career.protection.confirm_replace = true;
                return;
            }
            start(career, worker, identity, RecoveryAction::Rotate);
        }
        _ => {}
    }
}

/// Collects replies and reads the status when a screen or the Home card needs it.
pub(super) fn poll(
    mut career: ResMut<CareerClient>,
    mut worker: ResMut<Worker>,
    identity: Res<CareerIdentity>,
    prompt: Option<Res<ProtectPrompt>>,
) {
    if let Some((action, receiver)) = worker.pending.as_ref() {
        let action = *action;
        let reply = match receiver.lock().ok().map(|rx| rx.try_recv()) {
            Some(Ok(reply)) => reply,
            Some(Err(mpsc::TryRecvError::Disconnected)) => Err(tr("protect.error.stopped").into()),
            _ => return,
        };
        worker.pending = None;
        finish(&mut career, &mut worker, action, reply);
        return;
    }
    // Read-only until a read is due: no per-frame change detection on the UI.
    let state = &career.protection;
    let automatic = || {
        state.status.is_none()
            && worker.retry_at.is_none_or(|at| Instant::now() >= at)
            && career
                .view
                .profile
                .as_ref()
                .zip(prompt.as_deref())
                .is_some_and(|(profile, prompt)| prompt.record.due(profile))
    };
    if state.busy || !(state.wanted || automatic()) {
        return;
    }
    // A screen reopened right after an answer keeps it (the per-key limit is 10
    // a minute); the key must be registered: a profile exists only after the
    // server's sign-in.
    let fresh = state.status.is_some() && worker.answered_at.is_some_and(|at| at.elapsed() < FRESH);
    if fresh || career.view.profile.is_none() || identity.public_key().is_err() {
        if state.wanted {
            career.protection.wanted = false;
        }
        return;
    }
    start(&mut career, &mut worker, &identity, RecoveryAction::Status);
}

/// One line for Profile.
pub(super) fn profile_line(state: &ProtectionState) -> String {
    match &state.status {
        None if state.busy => tr("protect.status.checking").to_owned(),
        None => tr("protect.profile.unknown").to_owned(),
        Some(status) if !status.protected() => tr("protect.profile.unprotected").to_owned(),
        Some(status) => {
            let mut parts = Vec::new();
            if !status.linked_sign_ins.is_empty() {
                parts.push(tr("protect.profile.vk").to_owned());
            }
            if status.recovery_codes_remaining > 0 {
                parts.push(trf(
                    "protect.profile.codes",
                    &[("count", &status.recovery_codes_remaining)],
                ));
            }
            trf("protect.profile.protected", &[("ways", &parts.join(" · "))])
        }
    }
}

/// A code in groups of eight; the recovery field ignores the spaces.
fn grouped(code: &str) -> String {
    code.as_bytes()
        .chunks(8)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn body(parent: &mut ChildSpawnerCommands, career: &CareerClient) {
    let state = &career.protection;
    let vk = vk_offered();
    label(parent, tr("protect.title"), 26., ui::GOLD, "ProtectTitle");
    label(
        parent,
        tr(if vk {
            "protect.intro.vk"
        } else {
            "protect.intro.codes"
        }),
        16.,
        ui::IVORY,
        "ProtectIntro",
    );
    let (status, color) = match (&state.status, &state.error) {
        (Some(status), _) if status.protected() => (profile_line(state), ui::JADE),
        (Some(_), _) => (tr("protect.status.unprotected").to_owned(), ui::GOLD),
        (None, Some(error)) if !state.busy => (
            trf("protect.status.unavailable", &[("error", error)]),
            ui::GOLD,
        ),
        _ => (tr("protect.status.checking").to_owned(), ui::MUTED),
    };
    label(parent, status, 17., color, "ProtectStatus");
    if let Some(devices) = state.status.as_ref().map(|s| s.active_devices) {
        label(
            parent,
            trf("protect.status.devices", &[("count", &devices)]),
            14.,
            ui::MUTED,
            "ProtectDevices",
        );
    }
    if !state.codes.is_empty() {
        label(
            parent,
            tr("protect.codes.title"),
            20.,
            ui::GOLD,
            "ProtectCodesTitle",
        );
        label(
            parent,
            tr("protect.codes.notice"),
            15.,
            ui::IVORY,
            "ProtectCodesNotice",
        );
        for (index, code) in state.codes.iter().enumerate() {
            label(
                parent,
                format!("{}. {}", index + 1, grouped(code)),
                17.,
                ui::IVORY,
                &format!("ProtectCode{}", index + 1),
            );
        }
    }
    if state.confirm_replace {
        label(
            parent,
            tr("protect.replace_warning"),
            15.,
            ui::GOLD,
            "ProtectReplaceWarning",
        );
    }
    if state.busy {
        label(parent, tr("protect.busy"), 15., ui::MUTED, "ProtectBusy");
    } else {
        let linked = state
            .status
            .as_ref()
            .is_some_and(|s| !s.linked_sign_ins.is_empty());
        if vk && !linked && state.codes.is_empty() {
            button(
                parent,
                tr("protect.button.vk"),
                Action::SocialStart,
                "ProtectVk",
            );
        }
        match &state.status {
            Some(status) if state.codes.is_empty() => button(
                parent,
                tr(if status.recovery_codes_remaining == 0 {
                    "protect.button.create"
                } else if state.confirm_replace {
                    "protect.button.confirm_replace"
                } else {
                    "protect.button.replace"
                }),
                Action::ProtectRotate,
                "ProtectRotate",
            ),
            Some(_) => {}
            None => button(
                parent,
                tr("protect.button.retry"),
                Action::ProtectOpen,
                "ProtectRetry",
            ),
        }
    }
    label(
        parent,
        state.message.clone().unwrap_or_default(),
        15.,
        ui::GOLD,
        "ProtectMessage",
    );
    label(
        parent,
        tr("protect.recover_hint"),
        14.,
        ui::MUTED,
        "ProtectRecoverHint",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(matches: u32, ranked: u32) -> ProfileSummary {
        ProfileSummary {
            matches_played: matches,
            rated_matches: ranked,
            ..ProfileSummary::new("a".repeat(64), "Hero".into()) // i18n-allow
        }
    }

    #[test]
    fn the_card_follows_the_first_match_the_first_ranked_match_and_three_declines() {
        let mut record = PromptRecord::default();
        // A fresh install: nothing before the first completed match.
        assert!(!record.due(&profile(0, 0)));
        assert!(record.due(&profile(1, 0)));
        record.decline(&profile(1, 0));
        // The first ranked match brings it back before three casual matches do.
        assert!(!record.due(&profile(3, 0)));
        assert!(record.due(&profile(3, 1)));
        record.decline(&profile(3, 1));
        assert!(!record.due(&profile(3, 1)));
        // After that a decline lasts three completed matches.
        assert!(!record.due(&profile(5, 2)));
        assert!(record.due(&profile(6, 2)));
        record.decline(&profile(6, 2));
        assert_eq!(record.declines, MAX_DECLINES);
        assert!(!record.due(&profile(50, 20)));
        // A player who never plays ranked is asked again every three matches.
        let mut casual = PromptRecord::default();
        assert!(casual.due(&profile(1, 0)));
        casual.decline(&profile(1, 0));
        assert!(!casual.due(&profile(3, 0)));
        assert!(casual.due(&profile(4, 0)));
        casual.decline(&profile(4, 0));
        assert!(!casual.due(&profile(6, 0)));
        assert!(casual.due(&profile(7, 0)));
        casual.decline(&profile(7, 0));
        assert!(!casual.due(&profile(100, 0)));
        // A first match that was ranked: no second ranked trigger, three matches.
        let mut ranked_first = PromptRecord::default();
        ranked_first.decline(&profile(1, 1));
        assert!(!ranked_first.due(&profile(3, 3)));
        assert!(ranked_first.due(&profile(4, 4)));
    }

    #[test]
    fn the_record_persists_in_the_region_folder_and_bad_files_start_over() {
        let directory = std::env::temp_dir().join(format!(
            "omoba-protect-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let file = directory.join(PROMPT_FILE);
        let mut prompt = ProtectPrompt::at(Some(file.clone()));
        assert_eq!(prompt.record(), PromptRecord::default());
        prompt.decline(&profile(3, 0));
        let reloaded = ProtectPrompt::at(Some(file.clone()));
        assert_eq!(
            reloaded.record(),
            PromptRecord {
                declines: 1,
                declined_at_matches: 3,
                declined_after_ranked: false
            }
        );
        std::fs::write(&file, b"{not json").unwrap();
        assert_eq!(
            ProtectPrompt::at(Some(file)).record(),
            PromptRecord::default()
        );
        // Without a settings folder a decline holds for this run.
        let mut volatile = ProtectPrompt::at(None);
        volatile.decline(&profile(1, 0));
        assert_eq!(volatile.record().declines, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    fn codes() -> Vec<String> {
        (1..=8u8).map(|i| format!("{i:064x}")).collect()
    }
    fn unprotected(action: RecoveryAction) -> Reply {
        let rotate = action == RecoveryAction::Rotate;
        Ok(AccountProtection {
            recovery_codes_remaining: if rotate { 8 } else { 0 },
            linked_sign_ins: Vec::new(),
            active_devices: 1,
            recovery_codes: if rotate { codes() } else { Vec::new() },
        })
    }
    fn with_codes(action: RecoveryAction) -> Reply {
        let mut reply = unprotected(action)?;
        reply.recovery_codes_remaining = 8;
        Ok(reply)
    }
    fn unreachable_service(_: RecoveryAction) -> Reply {
        Err("Website service is unavailable. Try again.".into()) // i18n-allow
    }
    /// The career UI with its actions, the protection poll and the renderer;
    /// the Account API answers through `fake`.
    fn flow_app(fake: fn(RecoveryAction) -> Reply) -> App {
        let mut app = crate::ui::test_id::harness::kit_app();
        app.insert_resource(crate::ui::UiPlatform(UiProfile::Desktop))
            .init_resource::<CareerClient>()
            .insert_resource(CareerIdentity::authenticated_for_test(
                "127.0.0.1:4000",
                1,
                "session",
            ))
            .init_resource::<web::Worker>()
            .init_resource::<devices::Worker>()
            .insert_resource(Worker {
                fake: Some(fake),
                ..default()
            })
            .insert_resource(ProtectPrompt::at(None))
            .add_message::<NetworkCommand>()
            .add_message::<NicknameChanged>()
            .add_ui_action::<Action>()
            .add_systems(
                Update,
                (actions, poll, render)
                    .chain()
                    .after(crate::ui::UiSet::Dispatch),
            );
        app
    }
    fn set_profile(app: &mut App, matches: u32) {
        app.world_mut().resource_mut::<CareerClient>().view.profile = Some(profile(matches, 0));
    }
    fn state(app: &App) -> ProtectionState {
        app.world().resource::<CareerClient>().protection.clone()
    }
    fn text(app: &mut App, name: &str) -> Option<String> {
        app.world_mut()
            .query::<(&Name, &Text)>()
            .iter(app.world())
            .find(|(found, _)| found.as_str() == name)
            .map(|(_, text)| text.0.clone())
    }
    fn pending(app: &App) -> bool {
        app.world().resource::<Worker>().pending.is_some()
    }

    /// LR-18: the status is read only once the Home card could show (after the
    /// first match), and codes made in the game are shown once.
    #[test]
    fn in_game_codes_are_created_once_shown_and_forgotten_on_close() {
        use crate::ui::test_id::harness;
        let mut app = flow_app(unprotected);
        // Before the first match nothing is read.
        set_profile(&mut app, 0);
        app.update();
        app.update();
        assert!(!pending(&app) && state(&app).status.is_none());
        set_profile(&mut app, 1);
        app.update();
        assert!(state(&app).busy);
        app.update();
        let read = state(&app);
        assert!(!read.busy && !read.status.as_ref().unwrap().protected());

        app.world_mut()
            .resource_mut::<CareerClient>()
            .open_protect_modal();
        app.update();
        app.update();
        app.update();
        assert_eq!(
            text(&mut app, "ProtectStatus").as_deref(),
            Some("Not protected yet: only this installation can open your profile.")
        );
        assert_eq!(
            text(&mut app, "DeviceRecoveryPrompt"),
            None,
            "the Protect modal, not the device request"
        );
        harness::press(app.world_mut(), "ProtectRotate");
        app.update();
        app.update();
        app.update();
        let issued = state(&app);
        assert_eq!(issued.codes, codes());
        assert!(issued.status.as_ref().unwrap().protected());
        assert!(issued.status.as_ref().unwrap().recovery_codes.is_empty());
        assert_eq!(
            text(&mut app, "ProtectCode1").as_deref(),
            Some("1. 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000001")
        );
        assert!(text(&mut app, "ProtectCode8").is_some());
        // No second set from the same screen.
        assert!(harness::find(app.world_mut(), "ProtectRotate").is_none());
        app.world_mut().resource_mut::<CareerClient>().close();
        assert!(state(&app).codes.is_empty());
    }

    #[test]
    fn replacing_unused_codes_needs_a_second_press() {
        use crate::ui::test_id::harness;
        let mut app = flow_app(with_codes);
        set_profile(&mut app, 3);
        app.world_mut()
            .resource_mut::<CareerClient>()
            .open_protect_modal();
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            text(&mut app, "ProtectStatus").as_deref(),
            Some("Account protection: unused recovery codes: 8.")
        );
        harness::press(app.world_mut(), "ProtectRotate");
        app.update();
        app.update();
        assert!(state(&app).confirm_replace && !pending(&app));
        assert!(text(&mut app, "ProtectReplaceWarning").is_some());
        harness::press(app.world_mut(), "ProtectRotate");
        app.update();
        app.update();
        app.update();
        assert_eq!(state(&app).codes.len(), 8);
        assert!(!state(&app).confirm_replace);
    }

    #[test]
    fn a_failed_read_is_shown_and_retried_later_not_every_frame() {
        let mut app = flow_app(unreachable_service);
        set_profile(&mut app, 1);
        app.update();
        app.update();
        let failed = state(&app);
        assert!(failed.status.is_none() && failed.error.is_some());
        assert!(app.world().resource::<Worker>().retry_at.is_some());
        app.update();
        assert!(!pending(&app) && !state(&app).busy);
        // Opening the modal asks again at once and shows the reason.
        app.world_mut()
            .resource_mut::<CareerClient>()
            .open_protect_modal();
        for _ in 0..3 {
            app.update();
        }
        let shown = text(&mut app, "ProtectStatus").unwrap();
        assert!(
            shown.starts_with("Account protection is unavailable: "),
            "{shown}"
        );
        assert!(crate::ui::test_id::harness::find(app.world_mut(), "ProtectRetry").is_some());
    }

    #[test]
    fn codes_are_grouped_for_reading_and_paste_back_whole() {
        let code = "0123456789abcdef".repeat(4);
        let shown = grouped(&code);
        assert_eq!(shown.split(' ').count(), 8);
        let mut typed = String::new();
        devices::append(&mut typed, &shown).unwrap();
        assert_eq!(typed, code);
    }
}
