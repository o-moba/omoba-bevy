//! First native wallet integration: explicit terminal pairing before startup.
//! Tokens remain process-local; only scoped tickets enter the UDP protocol.
use omoba_passport::{
    NativeSession, PairingFlow, PairingState, PassportApi, pair_interactively, store, verify_local,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

static SESSION: OnceLock<NativeSession> = OnceLock::new();
type TicketResult = Arc<Mutex<Option<Result<String, String>>>>;
static TICKETS: OnceLock<Mutex<HashMap<(String, String), TicketResult>>> = OnceLock::new();

/// In-game wallet pairing, if one was started from the menu.
static ACCOUNT: Mutex<Option<omoba_passport::account::AccountSession>> = Mutex::new(None);
static ACCOUNT_FLOW: Mutex<Option<omoba_passport::account::AccountFlow>> = Mutex::new(None);
static ACCOUNT_BROWSER_OPENED: Mutex<Option<String>> = Mutex::new(None);
static ACCOUNT_REFRESH: Mutex<Option<std::time::Instant>> = Mutex::new(None);
const ACCOUNT_REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

static PAIRING: Mutex<Option<PairingFlow>> = Mutex::new(None);
static BROWSER_OPENED: Mutex<Option<String>> = Mutex::new(None);

/// What the menu shows about the wallet connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalletView {
    Disconnected,
    Starting,
    AwaitingApproval {
        user_code: String,
        verification_url: String,
    },
    Connected,
    Failed(String),
}

pub fn is_connected() -> bool {
    SESSION.get().is_some()
}

/// Start pairing from the menu. A running attempt is left alone.
pub fn connect() {
    if is_connected() {
        return;
    }
    let mut pairing = PAIRING.lock().unwrap();
    if pairing.as_ref().is_some_and(PairingFlow::in_progress) {
        return;
    }
    *BROWSER_OPENED.lock().unwrap() = None;
    *pairing = match PassportApi::from_env() {
        Ok(api) => Some(PairingFlow::start(api.client().clone())),
        Err(error) => {
            eprintln!("Wallet connection unavailable: {error}");
            None
        }
    };
}

/// Advance pairing once per frame. Returns true on the frame the wallet
/// becomes connected, so the menu can list the purchased avatars.
pub fn poll_wallet() -> bool {
    let mut pairing = PAIRING.lock().unwrap();
    let Some(flow) = pairing.as_ref() else {
        return false;
    };
    match flow.state() {
        PairingState::AwaitingApproval {
            verification_url, ..
        } => {
            // Open the approval page once; the link stays on screen either way.
            let mut opened = BROWSER_OPENED.lock().unwrap();
            if opened.as_deref() != Some(verification_url.as_str()) {
                if let Err(error) = omoba_passport::open_in_browser(&verification_url) {
                    eprintln!("{error}: {verification_url}");
                }
                *opened = Some(verification_url);
            }
            false
        }
        PairingState::Connected => {
            let connected = flow
                .take_session()
                .ok_or_else(|| "Wallet session was already collected".to_owned())
                .and_then(omoba_passport::accept_session)
                .is_ok_and(|session| SESSION.set(session).is_ok());
            *pairing = None;
            if connected {
                // A purchase made minutes ago may be newer than our catalogue.
                store::request_refresh();
            }
            connected
        }
        _ => false,
    }
}

pub fn wallet_view() -> WalletView {
    if is_connected() {
        return WalletView::Connected;
    }
    match PAIRING.lock().unwrap().as_ref().map(PairingFlow::state) {
        None | Some(PairingState::Cancelled | PairingState::Connected) => WalletView::Disconnected,
        Some(PairingState::Starting) => WalletView::Starting,
        Some(PairingState::AwaitingApproval {
            user_code,
            verification_url,
            ..
        }) => WalletView::AwaitingApproval {
            user_code,
            verification_url,
        },
        Some(PairingState::Failed(error)) => WalletView::Failed(error),
    }
}

/// One line for the menu, under "Choose Avatar".
pub fn wallet_status_line() -> String {
    match wallet_view() {
        WalletView::Disconnected => "Default avatars are free for everyone".into(),
        WalletView::Starting => "Contacting Ekza…".into(),
        WalletView::AwaitingApproval {
            user_code,
            verification_url,
        } => format!("Approve in your browser · code {user_code} · {verification_url}"),
        WalletView::Connected => {
            "Wallet connected · your Ekza avatars are listed below the defaults".into()
        }
        WalletView::Failed(error) => error,
    }
}

pub fn account_connected() -> bool {
    ACCOUNT.lock().unwrap().is_some()
}

/// Start connecting an Ekza account (email or Google in the browser, no wallet).
pub fn connect_account() {
    if account_connected() {
        return;
    }
    let mut flow = ACCOUNT_FLOW.lock().unwrap();
    if flow.as_ref().is_some_and(|flow| flow.in_progress()) {
        return;
    }
    *ACCOUNT_BROWSER_OPENED.lock().unwrap() = None;
    *flow = match omoba_passport::account::start() {
        Ok(flow) => Some(flow),
        Err(error) => {
            eprintln!("Ekza account connection unavailable: {error}");
            None
        }
    };
}

/// Drive the account connection; true once when the account has just connected.
/// While connected, re-reads the library in the background so an avatar saved in the
/// browser shows up without restarting the game.
pub fn poll_account() -> bool {
    let mut flow = ACCOUNT_FLOW.lock().unwrap();
    match flow.as_ref().map(|flow| flow.state()) {
        Some(PairingState::AwaitingApproval {
            verification_url, ..
        }) => {
            let mut opened = ACCOUNT_BROWSER_OPENED.lock().unwrap();
            if opened.as_deref() != Some(verification_url.as_str()) {
                if let Err(error) = omoba_passport::open_in_browser(&verification_url) {
                    eprintln!("{error}: {verification_url}");
                }
                *opened = Some(verification_url);
            }
            false
        }
        Some(PairingState::Connected) => {
            let session = flow.as_ref().and_then(|flow| flow.take_session());
            *flow = None;
            let connected = session.is_some();
            if connected {
                *ACCOUNT.lock().unwrap() = session;
                *ACCOUNT_REFRESH.lock().unwrap() = Some(std::time::Instant::now());
                store::request_refresh();
            }
            connected
        }
        _ => {
            drop(flow);
            refresh_account_in_background();
            false
        }
    }
}

fn refresh_account_in_background() {
    let mut last = ACCOUNT_REFRESH.lock().unwrap();
    if !last.is_some_and(|at| at.elapsed() >= ACCOUNT_REFRESH_EVERY) {
        return;
    }
    let Some(mut session) = ACCOUNT.lock().unwrap().clone() else {
        return;
    };
    *last = Some(std::time::Instant::now());
    std::thread::spawn(move || match session.refresh() {
        Ok(()) => {
            *ACCOUNT.lock().unwrap() = Some(session);
            store::request_refresh();
        }
        // Disconnected in Studio, or Ekza is unreachable: keep what is shown.
        Err(error) => eprintln!("Ekza library was not refreshed: {error}"),
    });
}

pub fn account_status_line() -> String {
    if let Some(session) = ACCOUNT.lock().unwrap().as_ref() {
        return format!(
            "Ekza account {} · save avatars on Ekza to see them here",
            session.username
        );
    }
    match ACCOUNT_FLOW
        .lock()
        .unwrap()
        .as_ref()
        .map(|flow| flow.state())
    {
        Some(PairingState::Starting) => "Contacting Ekza…".into(),
        Some(PairingState::AwaitingApproval {
            user_code,
            verification_url,
            ..
        }) => format!("Confirm in your browser · code {user_code} · {verification_url}"),
        Some(PairingState::Failed(error)) => error,
        _ => "Connect your Ekza account to see your own library · no wallet needed".into(),
    }
}

/// Free avatars from the connected account's library (saved or created), approved
/// for Omoba. A subset of [`community_avatars`]; empty when no account is connected.
pub fn library_avatars() -> Vec<&'static shared::AvatarDefinition> {
    let account = ACCOUNT.lock().unwrap();
    let Some(session) = account.as_ref() else {
        return Vec::new();
    };
    shared::store_avatars()
        .into_iter()
        .filter(|avatar| avatar.free && session.has(&avatar.slug))
        .collect()
}

pub enum TicketPoll {
    Free,
    Pending,
    Ready(String),
    Denied(String),
}

pub fn clear_tickets() {
    if let Some(tickets) = TICKETS.get() {
        tickets.lock().unwrap().clear();
    }
}

pub fn initialize() {
    if !std::env::var("OMOBA_PASSPORT_CONNECT").is_ok_and(|value| value == "1") {
        return;
    }
    match PassportApi::from_env().and_then(pair_interactively) {
        Ok(session) => {
            let _ = SESSION.set(session);
        }
        Err(error) => {
            eprintln!("Wallet connection unavailable: {error} Continuing with free avatars.")
        }
    }
}

/// Start the Ekza avatar store under the private settings directory. With a
/// paired wallet the first catalogue read is awaited so purchases are on the
/// first menu; otherwise it happens in the background.
pub fn initialize_store() -> Option<std::path::PathBuf> {
    let root = crate::platform::preferences_file_path(
        std::env::var("OMOBA_CLIENT_CONFIG_DIR").ok().as_deref(),
        crate::platform::preferences_directory(),
        "ekza-store",
    )?;
    std::fs::create_dir_all(root.join("avatars")).ok()?;
    store::initialize(root.clone(), SESSION.get().is_some());
    Some(root)
}

/// Every avatar this player may pick: the shipped roster, then owned store
/// avatars. Unowned store entries stay hidden; they are bought on the web.
pub fn selectable_avatars() -> Vec<&'static shared::AvatarDefinition> {
    let mut avatars = default_avatars();
    avatars.extend(purchased_avatars());
    avatars
}

/// Avatars that ship with the game (plus roster entries staged by an
/// operator), as far as this player may pick them.
pub fn default_avatars() -> Vec<&'static shared::AvatarDefinition> {
    shared::avatar_roster()
        .iter()
        .filter(|avatar| can_select(avatar))
        .collect()
}

/// Avatars that are not in the box: bought on Ekza by the paired wallet and
/// delivered through the SDK store.
pub fn purchased_avatars() -> Vec<&'static shared::AvatarDefinition> {
    shared::store_avatars()
        .into_iter()
        .filter(|avatar| !avatar.free && can_select(avatar))
        .collect()
}

/// Free avatars creators prepared for Omoba and an Omoba owner approved. Anyone
/// may wear them; the server admits them from its own read of the registry.
pub fn community_avatars() -> Vec<&'static shared::AvatarDefinition> {
    let mine: Vec<_> = library_avatars()
        .iter()
        .map(|avatar| avatar.slug.clone())
        .collect();
    shared::store_avatars()
        .into_iter()
        .filter(|avatar| avatar.free && !mine.contains(&avatar.slug))
        .collect()
}

/// Empty-state copy for the purchased section.
pub fn purchased_hint() -> &'static str {
    if SESSION.get().is_some() {
        "No Ekza avatars approved for Omoba in this wallet yet · buy one on avatar.ekza.io"
    } else {
        "Connect your Ekza wallet to wear avatars you own"
    }
}

/// Asset path of an avatar thumbnail, wherever its file lives.
pub fn thumbnail_asset_path(avatar: &shared::AvatarDefinition) -> Option<String> {
    let file = avatar.thumbnail.as_deref()?;
    Some(if store::knows(&avatar.slug) {
        store::thumbnail_asset_path(file)
    } else {
        format!("avatars/{file}")
    })
}

pub fn can_select(avatar: &shared::AvatarDefinition) -> bool {
    avatar.free
        || avatar.passport.as_ref().is_none_or(|protected| {
            SESSION
                .get()
                .is_some_and(|session| session.owns(protected).is_some())
        })
}

pub fn ticket_for_slug(slug: Option<&str>, session_id: &str) -> TicketPoll {
    let Some(avatar) = slug.and_then(shared::avatar_definition) else {
        return TicketPoll::Free;
    };
    let Some(protected) = &avatar.passport else {
        return TicketPoll::Free;
    };
    if avatar.free {
        // No ownership to prove. The model installs on first use like any store
        // avatar another player wears.
        return TicketPoll::Free;
    }
    let Some(session) = SESSION.get() else {
        return TicketPoll::Denied("Connect your wallet before choosing a purchased avatar".into());
    };
    let key = (avatar.slug.clone(), session_id.to_owned());
    let mut tickets = TICKETS.get_or_init(Default::default).lock().unwrap();
    let state = tickets.entry(key).or_insert_with(|| {
        let result = Arc::new(Mutex::new(None));
        let output = result.clone();
        let session = session.clone();
        let protected = protected.clone();
        let session_id = session_id.to_owned();
        let path = shared::client_asset_root()
            .join("avatars")
            .join(format!("{}.glb", avatar.slug));
        let slug = avatar.slug.clone();
        std::thread::spawn(move || {
            // Store avatars install on first use; roster entries staged by
            // passport-import are verified where they were placed.
            let installed = if store::knows(&slug) {
                store::install_blocking(&slug)
            } else {
                verify_local(&protected, &path)
            };
            let verified = installed
                .and_then(|()| omoba_passport::ticket(&session, &protected, &session_id))
                .map(|ticket| ticket.ticket);
            *output.lock().unwrap() = Some(verified);
        });
        result
    });
    match state.lock().unwrap().as_ref() {
        None => TicketPoll::Pending,
        Some(Ok(ticket)) => TicketPoll::Ready(ticket.clone()),
        Some(Err(error)) => TicketPoll::Denied(error.clone()),
    }
}
