//! Connect Omoba to an Ekza account with no wallet.
//!
//! The session answers one question for the avatar picker: which free avatars approved
//! for Omoba did this player save or create. It grants nothing. The game server admits
//! a free avatar from its own registry read, connected account or not.

pub use ekza_bevy_sdk::account::{AccountClient, AccountFlow, AccountSession};

/// The registry the client store and the game server already use.
pub fn client() -> Result<AccountClient, String> {
    AccountClient::new(
        &crate::community::registry_url(),
        &crate::selector().project_id,
    )
}

pub fn start() -> Result<AccountFlow, String> {
    Ok(AccountFlow::start(client()?, crate::selector()))
}
