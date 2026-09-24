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
static PAIRING: Mutex<Option<PairingFlow>> = Mutex::new(None);
static WALLET_ERROR: Mutex<Option<String>> = Mutex::new(None);
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
    if let Some(flow) = pairing.as_ref().filter(|flow| flow.in_progress()) {
        if let PairingState::AwaitingApproval {
            verification_url, ..
        } = flow.state()
        {
            *WALLET_ERROR.lock().unwrap() =
                crate::platform::open_external_url(&verification_url).err();
        }
        return;
    }
    *WALLET_ERROR.lock().unwrap() = None;
    *BROWSER_OPENED.lock().unwrap() = None;
    *pairing = match PassportApi::from_env() {
        Ok(api) => Some(PairingFlow::start(api.client().clone())),
        Err(error) => {
            *WALLET_ERROR.lock().unwrap() = Some(format!("Wallet connection unavailable: {error}"));
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
                if let Err(error) = crate::platform::open_external_url(&verification_url) {
                    *WALLET_ERROR.lock().unwrap() = Some(error);
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
pub fn wallet_button_label() -> &'static str {
    match wallet_view() {
        WalletView::Starting => "Connecting wallet…",
        WalletView::AwaitingApproval { .. } => "Open wallet approval",
        WalletView::Connected => "Wallet connected",
        _ => "Connect wallet (optional)",
    }
}

pub fn wallet_status_line() -> String {
    if let Some(error) = WALLET_ERROR.lock().unwrap().as_ref() {
        return error.clone();
    }
    match wallet_view() {
        WalletView::Disconnected => {
            "Optional wallet connection for owned avatars · no purchase is made here".into()
        }
        WalletView::Starting => "Contacting Ekza…".into(),
        WalletView::AwaitingApproval {
            user_code,
            verification_url,
        } => format!(
            "{} · code {user_code} · {verification_url}",
            crate::platform::browser_approval_hint()
        ),
        WalletView::Connected => {
            "Wallet connected · your Ekza avatars are listed below the defaults".into()
        }
        WalletView::Failed(error) => error,
    }
}

#[path = "ekza_account.rs"]
mod ekza_account;
pub use ekza_account::{
    account_button_label, account_connected, account_status_line, connect_account, library_avatars,
    poll_account,
};

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
    ekza_account::initialize(root.join("account/session.json"));
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
        .filter(|avatar| {
            !store::free_access(&avatar.slug).unwrap_or(avatar.free) && can_select(avatar)
        })
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
        .filter(|avatar| {
            store::free_access(&avatar.slug).unwrap_or(avatar.free) && !mine.contains(&avatar.slug)
        })
        .collect()
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
    store::free_access(&avatar.slug).unwrap_or(avatar.free)
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
    if store::free_access(&avatar.slug).unwrap_or(avatar.free) {
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

/// Shared order and source labels for Collection and the pre-match picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AvatarCatalogueSource {
    Default,
    Library,
    Purchased,
    Community,
}

impl AvatarCatalogueSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Included",
            Self::Library => "Your library",
            Self::Purchased => "Owned",
            Self::Community => "Free · Studio",
        }
    }
}

pub struct AvatarCatalogueEntry {
    pub avatar: shared::AvatarDefinition,
    pub source: AvatarCatalogueSource,
}

pub struct AvatarCatalogue {
    pub entries: Vec<AvatarCatalogueEntry>,
    /// Identity/metadata/entitlement based; equal-count replacements change it.
    pub revision: u64,
    pub status: store::CatalogueStatus,
}

pub fn avatar_catalogue() -> AvatarCatalogue {
    let status = store::catalogue_status();
    let mut entries: Vec<_> = default_avatars()
        .into_iter()
        .map(|avatar| AvatarCatalogueEntry {
            avatar: avatar.clone(),
            source: AvatarCatalogueSource::Default,
        })
        .collect();
    let library: Vec<_> = library_avatars()
        .iter()
        .map(|avatar| avatar.slug.clone())
        .collect();
    let community: Vec<_> = community_avatars()
        .iter()
        .map(|avatar| avatar.slug.clone())
        .collect();
    let mut studio = Vec::new();
    for avatar in store::catalogue_definitions() {
        let source = if library.contains(&avatar.slug) {
            AvatarCatalogueSource::Library
        } else if community.contains(&avatar.slug) {
            AvatarCatalogueSource::Community
        } else if !avatar.free && can_select(&avatar) {
            AvatarCatalogueSource::Purchased
        } else {
            continue;
        };
        studio.push(AvatarCatalogueEntry { avatar, source });
    }
    let order = |source| match source {
        AvatarCatalogueSource::Default => 0,
        AvatarCatalogueSource::Library => 1,
        AvatarCatalogueSource::Purchased => 2,
        AvatarCatalogueSource::Community => 3,
    };
    studio.sort_by(|a, b| {
        order(a.source)
            .cmp(&order(b.source))
            .then(a.avatar.display_name.cmp(&b.avatar.display_name))
            .then(a.avatar.slug.cmp(&b.avatar.slug))
    });
    for entry in studio {
        if !entries
            .iter()
            .any(|existing| existing.avatar.slug == entry.avatar.slug)
        {
            entries.push(entry);
        }
    }
    let revision = catalogue_revision(&entries, &status);
    AvatarCatalogue {
        entries,
        revision,
        status,
    }
}

fn catalogue_revision(entries: &[AvatarCatalogueEntry], _status: &store::CatalogueStatus) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for entry in entries {
        entry.avatar.slug.hash(&mut hash);
        entry.avatar.display_name.hash(&mut hash);
        entry.avatar.thumbnail.hash(&mut hash);
        entry.source.hash(&mut hash);
        entry.avatar.free.hash(&mut hash);
        entry.avatar.license.hash(&mut hash);
        entry.avatar.author.hash(&mut hash);
        can_select(&entry.avatar).hash(&mut hash);
    }
    hash.finish()
}

pub fn refresh_avatar_catalogue() {
    store::request_user_refresh();
}

/// Collection-facing connection status: no transport diagnostics or credentials.
pub fn avatar_account_status_line() -> String {
    ekza_account::account_status_line()
}

/// Current menu copy without rewriting the immutable model/approval identity.
pub fn avatar_display_name(slug: Option<&str>) -> String {
    let Some(slug) = slug else {
        return "Default avatar".into();
    };
    store::catalogue_definitions()
        .into_iter()
        .find(|avatar| avatar.slug == slug)
        .map(|avatar| avatar.display_name)
        .or_else(|| shared::avatar_definition(slug).map(|avatar| avatar.display_name.clone()))
        .unwrap_or_else(|| "Default avatar".into())
}

#[cfg(test)]
mod catalogue_tests {
    use super::*;

    #[test]
    fn loading_and_empty_status_do_not_invalidate_avatar_controls() {
        let entries = vec![AvatarCatalogueEntry {
            avatar: shared::avatar_roster()[0].clone(),
            source: AvatarCatalogueSource::Default,
        }];
        assert_eq!(
            catalogue_revision(&entries, &store::CatalogueStatus::Loading { cached: 0 }),
            catalogue_revision(&entries, &store::CatalogueStatus::Empty)
        );
    }

    #[test]
    fn same_slug_metadata_changes_invalidate_the_catalogue_snapshot() {
        let avatar = shared::avatar_roster()[0].clone();
        let status = store::CatalogueStatus::Ready { count: 1 };
        let mut entries = vec![AvatarCatalogueEntry {
            avatar,
            source: AvatarCatalogueSource::Community,
        }];
        let before = catalogue_revision(&entries, &status);
        entries[0].avatar.display_name.push_str(" revised");
        assert_ne!(before, catalogue_revision(&entries, &status));
        let named = catalogue_revision(&entries, &status);
        entries[0].avatar.thumbnail = Some("changed.png".into());
        assert_ne!(named, catalogue_revision(&entries, &status));
        let pictured = catalogue_revision(&entries, &status);
        entries[0].avatar.free = !entries[0].avatar.free;
        assert_ne!(pictured, catalogue_revision(&entries, &status));
    }

    #[test]
    fn same_count_identity_and_source_changes_invalidate_catalogue() {
        let roster = shared::avatar_roster();
        assert!(roster.len() > 1);
        let make = |index: usize, source| {
            vec![AvatarCatalogueEntry {
                avatar: roster[index].clone(),
                source,
            }]
        };
        let status = store::CatalogueStatus::Ready { count: 1 };
        let first = catalogue_revision(&make(0, AvatarCatalogueSource::Default), &status);
        assert_ne!(
            first,
            catalogue_revision(&make(1, AvatarCatalogueSource::Default), &status)
        );
        assert_ne!(
            first,
            catalogue_revision(&make(0, AvatarCatalogueSource::Library), &status)
        );
    }
}
