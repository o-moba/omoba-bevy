//! Where the packaged client assets live. Moved out of `shared` (step 13) so
//! the shared model stays free of environment and filesystem reads.

use std::path::PathBuf;

/// Environment override of the packaged asset root.
pub const ASSET_DIR_ENV: &str = "OMOBA_ASSET_DIR";

/// Resolve packaged assets before the development checkout. Launchers can
/// pin this path without requiring a source-tree working directory.
pub fn client_asset_root() -> PathBuf {
    // Android's AssetManager addresses paths inside the APK, not the build host.
    #[cfg(target_os = "android")]
    return PathBuf::from("assets");

    #[cfg(not(target_os = "android"))]
    desktop_or_ios_asset_root()
}

#[cfg(not(target_os = "android"))]
fn desktop_or_ios_asset_root() -> PathBuf {
    if let Some(path) = std::env::var_os(ASSET_DIR_ENV) {
        return path.into();
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            let assets = parent.join("assets");
            if assets.is_dir() {
                return assets;
            }
            // A macOS app bundle keeps data in Contents/Resources, beside
            // Contents/MacOS where the executable lives.
            let bundled = parent.join("../Resources/assets");
            if bundled.is_dir() {
                return bundled;
            }
        }
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("passport crate has workspace parent")
        .join("client/assets")
}
