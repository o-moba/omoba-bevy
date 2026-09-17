//! Compile the platform-owned Swift bridges for every iOS build path.
use std::{env, path::PathBuf, process::Command};

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
    println!("cargo:rerun-if-changed=../mobile/ios/SupporterStoreKit.swift");
    println!("cargo:rerun-if-changed=../mobile/ios/OmobaGameController.swift");
    println!("cargo:rerun-if-env-changed=IPHONEOS_DEPLOYMENT_TARGET");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("ios") {
        return;
    }
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
        .arg("../mobile/ios/OmobaGameController.swift")
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
    println!("cargo:rustc-link-lib=framework=UIKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
