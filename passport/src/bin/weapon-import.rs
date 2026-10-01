//! Import approved free handheld renditions via the installed Ekza SDK.
fn main() {
    let mut args = std::env::args().skip(1);
    let registry = args
        .next()
        .unwrap_or_else(omoba_passport::community::registry_url);
    let assets = omoba_passport::assets::client_asset_root();
    let manifest = assets.join("weapons/ekza-manifest.json");
    match omoba_passport::weapons::import_registry(&registry, &assets, &manifest) {
        Ok(n) => println!(
            "Imported {n} approved free handheld(s). Restart both client and server with OMOBA_WEAPON_MANIFEST={}",
            manifest.display()
        ),
        Err(e) => {
            eprintln!("Weapon import failed: {e}");
            std::process::exit(1);
        }
    }
}
