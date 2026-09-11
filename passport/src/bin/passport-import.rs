use omoba_passport::{PassportApi, import_owned, pair_interactively};
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let asset_root = shared::client_asset_root();
    let manifest = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| asset_root.join("avatars/passport-manifest.json"));
    let session = pair_interactively(PassportApi::from_env()?)?;
    let count = import_owned(&session, &asset_root, &manifest)?;
    println!(
        "Installed {count} approved purchased avatar(s). Public manifest: {}",
        manifest.display()
    );
    println!(
        "Restart BOTH server and clients with OMOBA_AVATAR_MANIFEST set to that file. Distribute the same validated assets to remote clients."
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Avatar import failed: {error}");
        std::process::exit(1);
    }
}
