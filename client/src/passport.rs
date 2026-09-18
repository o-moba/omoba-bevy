//! First native wallet integration: explicit terminal pairing before startup.
//! Tokens remain process-local; only scoped tickets enter the UDP protocol.
use omoba_passport::{NativeSession, PassportApi, pair_interactively, store, verify_local};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

static SESSION: OnceLock<NativeSession> = OnceLock::new();
type TicketResult = Arc<Mutex<Option<Result<String, String>>>>;
static TICKETS: OnceLock<Mutex<HashMap<(String, String), TicketResult>>> = OnceLock::new();

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
        .filter(|avatar| can_select(avatar))
        .collect()
}

/// Empty-state copy for the purchased section.
pub fn purchased_hint() -> &'static str {
    if SESSION.get().is_some() {
        "No Ekza avatars approved for Omoba in this wallet yet · buy one on avatar.ekza.io"
    } else {
        "Launch with OMOBA_PASSPORT_CONNECT=1 to wear avatars you own on Ekza"
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
    avatar.passport.as_ref().is_none_or(|protected| {
        SESSION
            .get()
            .is_some_and(|session| session.owns(protected).is_some())
    })
}

pub fn status() -> &'static str {
    if SESSION.get().is_some() {
        "Wallet connected · your Ekza avatars are listed below the defaults"
    } else {
        "Default avatars are free for everyone"
    }
}

pub fn ticket_for_slug(slug: Option<&str>, session_id: &str) -> TicketPoll {
    let Some(avatar) = slug.and_then(shared::avatar_definition) else {
        return TicketPoll::Free;
    };
    let Some(protected) = &avatar.passport else {
        return TicketPoll::Free;
    };
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
