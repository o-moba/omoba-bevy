//! Build steps of the client:
//!
//! - every target: generate the i18n dictionary registry
//!   (`OUT_DIR/i18n_bundles.rs`) from the folders under `client/i18n/`, so a
//!   language is added by adding a folder (see `docs/i18n.md`);
//! - every target: generate the typed Verdant Crown design tokens
//!   (`OUT_DIR/ui_tokens.rs`) from `client/ui/tokens/verdant-crown.json`
//!   (see `build/ui_tokens.rs` and `docs/ui-kit.md`);
//! - every target: generate the typed Verdant UI asset table
//!   (`OUT_DIR/ui_assets.rs`) from `client/assets/ui/verdant/manifest.json`
//!   (see `build/ui_assets.rs`);
//! - iOS: compile the platform-owned Swift bridges (StoreKit, browser,
//!   GameController).
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

#[path = "build/ui_assets.rs"]
mod ui_assets;
#[path = "build/ui_tokens.rs"]
mod ui_tokens;

fn output(args: &[&str]) -> String {
    let result = Command::new("xcrun")
        .args(args)
        .output()
        .expect("Xcode command line tools required for iOS");
    assert!(
        result.status.success(),
        "xcrun {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout)
        .expect("Xcode tool path must be UTF-8")
        .trim()
        .to_owned()
}

fn main() {
    generate_i18n_registry();
    generate_ui_tokens();
    generate_ui_assets();
    println!("cargo:rerun-if-changed=../mobile/ios/SupporterStoreKit.swift");
    println!("cargo:rerun-if-changed=../mobile/ios/BrowserBridge.swift");
    println!("cargo:rerun-if-changed=../mobile/ios/OmobaGameController.swift");
    println!("cargo:rerun-if-changed=../mobile/ios/PhoneLayoutPreview.swift");
    println!("cargo:rerun-if-changed=../mobile/ios/FramePacing.swift");
    println!("cargo:rerun-if-env-changed=IPHONEOS_DEPLOYMENT_TARGET");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        build_ios_bridges();
    }
}

/// The dictionary folder, relative to the client crate.
const I18N_DIR: &str = "i18n";
/// The locale every other locale falls back to; it must exist.
const FALLBACK_LOCALE: &str = "en";

/// A locale code or namespace made of ASCII letters, digits, `-` and `_`.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn sorted_entries(directory: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("i18n directory entry").path())
        .collect();
    entries.sort();
    entries
}

/// Writes `OUT_DIR/i18n_bundles.rs`: one `RawLocale` per folder of
/// `client/i18n/`, English first, then by code. Every file is embedded with
/// `include_str!`; parsing and validation happen once at runtime
/// (`crate::i18n`), and the guard tests check the content.
fn generate_i18n_registry() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest dir"));
    let root = manifest.join(I18N_DIR);
    // A directory path makes Cargo rescan the whole tree for changes, so a
    // new locale folder or dictionary file regenerates the registry.
    println!("cargo:rerun-if-changed={I18N_DIR}");
    let mut locales = Vec::new();
    for folder in sorted_entries(&root) {
        if !folder.is_dir() {
            continue;
        }
        let code = folder
            .file_name()
            .and_then(|name| name.to_str())
            .expect("locale folder names are UTF-8")
            .to_owned();
        assert!(is_plain_name(&code), "invalid locale folder name {code:?}");
        let meta = folder.join("_meta.json");
        assert!(meta.is_file(), "locale {code} has no _meta.json");
        let mut files = Vec::new();
        for file in sorted_entries(&folder) {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .expect("dictionary file names are UTF-8")
                .to_owned();
            if name == "_meta.json" {
                continue;
            }
            let Some(namespace) = name.strip_suffix(".json") else {
                panic!("unexpected file {} in locale {code}", file.display());
            };
            assert!(
                is_plain_name(namespace),
                "invalid namespace file {name:?} in locale {code}"
            );
            println!("cargo:rerun-if-changed={}", file.display());
            files.push((namespace.to_owned(), file));
        }
        println!("cargo:rerun-if-changed={}", meta.display());
        locales.push((code, meta, files));
    }
    locales.sort_by_key(|(code, _, _)| (code != FALLBACK_LOCALE, code.clone()));
    assert!(
        locales
            .first()
            .is_some_and(|(code, _, _)| code == FALLBACK_LOCALE),
        "client/i18n/{FALLBACK_LOCALE}/ is required"
    );
    let mut out = String::from(
        "// @generated by client/build.rs from client/i18n/. Do not edit.\n\
         static LOCALES: &[RawLocale] = &[\n",
    );
    for (code, meta, files) in &locales {
        out.push_str(&format!(
            "    RawLocale {{\n        code: {code:?},\n        meta: include_str!({:?}),\n        files: &[\n",
            meta.display().to_string()
        ));
        for (namespace, file) in files {
            out.push_str(&format!(
                "            ({namespace:?}, include_str!({:?})),\n",
                file.display().to_string()
            ));
        }
        out.push_str("        ],\n    },\n");
    }
    out.push_str("];\n");
    let target =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output")).join("i18n_bundles.rs");
    fs::write(&target, out).expect("write i18n registry");
}

/// The Verdant Crown token copy (see `scripts/sync_ui_tokens.py`).
const UI_TOKENS: &str = "ui/tokens/verdant-crown.json";

/// Writes `OUT_DIR/ui_tokens.rs` from [`UI_TOKENS`].
fn generate_ui_tokens() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest dir"));
    println!("cargo:rerun-if-changed={UI_TOKENS}");
    println!("cargo:rerun-if-changed=build/ui_tokens.rs");
    let source = fs::read_to_string(manifest.join(UI_TOKENS))
        .unwrap_or_else(|error| panic!("cannot read {UI_TOKENS}: {error}"));
    let tokens: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("{UI_TOKENS} is not a JSON object: {error}"));
    assert_eq!(
        tokens
            .get("$schema_version")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "{UI_TOKENS}: unsupported $schema_version"
    );
    let code = ui_tokens::render(&tokens, &manifest.join("assets"));
    let target = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output")).join("ui_tokens.rs");
    fs::write(&target, code).expect("write ui tokens");
}

/// The installed Verdant UI asset manifest (see `scripts/sync_ui_assets.py`).
const UI_ASSETS: &str = "assets/ui/verdant/manifest.json";

/// Writes `OUT_DIR/ui_assets.rs` from [`UI_ASSETS`].
fn generate_ui_assets() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest dir"));
    println!("cargo:rerun-if-changed={UI_ASSETS}");
    println!("cargo:rerun-if-changed=build/ui_assets.rs");
    let source = fs::read_to_string(manifest.join(UI_ASSETS))
        .unwrap_or_else(|error| panic!("cannot read {UI_ASSETS}: {error}"));
    let data: serde_json::Value = serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("{UI_ASSETS} is not JSON: {error}"));
    let code = ui_assets::render(&data, &manifest.join("assets"));
    let target = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output")).join("ui_assets.rs");
    fs::write(&target, code).expect("write ui assets");
}

fn build_ios_bridges() {
    let target = env::var("TARGET").expect("Cargo target");
    let simulator = target.ends_with("-sim") || target.starts_with("x86_64");
    let sdk_name = if simulator {
        "iphonesimulator"
    } else {
        "iphoneos"
    };
    let sdk = output(&["--sdk", sdk_name, "--show-sdk-path"]);
    let swiftc = PathBuf::from(output(&["--find", "swiftc"]));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output"));
    let arch = if target.starts_with("x86_64") {
        "x86_64"
    } else {
        "arm64"
    };
    let triple = format!(
        "{arch}-apple-ios15.0{}",
        if simulator { "-simulator" } else { "" }
    );
    let library = out.join("libOmobaStoreKit.a");
    let result = Command::new(&swiftc)
        .args([
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
            "-g",
            "-swift-version",
            "5",
            "-module-name",
            "OmobaStoreKit",
            "-target",
            &triple,
            "-sdk",
            &sdk,
            "-module-cache-path",
        ])
        .arg(out.join("swift-module-cache"))
        .arg("../mobile/ios/SupporterStoreKit.swift")
        .arg("../mobile/ios/BrowserBridge.swift")
        .arg("../mobile/ios/OmobaGameController.swift")
        .arg("../mobile/ios/PhoneLayoutPreview.swift")
        .arg("../mobile/ios/FramePacing.swift")
        .arg("-o")
        .arg(&library)
        .output()
        .expect("Swift compiler required");
    assert!(
        result.status.success(),
        "iOS Swift bridges failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let toolchain = swiftc
        .parent()
        .and_then(|p| p.parent())
        .expect("Xcode toolchain");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!(
        "cargo:rustc-link-search=native={}",
        toolchain.join("lib/swift").join(sdk_name).display()
    );
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-lib=static=OmobaStoreKit");
    println!("cargo:rustc-link-lib=framework=StoreKit");
    println!("cargo:rustc-link-lib=framework=GameController");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=UIKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
