//! Identify the running build, never the checkout beside it.
use std::sync::OnceLock;

pub(crate) fn label() -> &'static str {
    static LABEL: OnceLock<String> = OnceLock::new();
    LABEL.get_or_init(|| {
        // Xcode packages this receipt with the executable. Do not use the asset
        // directory override: it may point to a different development checkout.
        let receipt = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|p| p.join("assets/legal/XCODE-BUILD.json"))
            })
            .and_then(|path| std::fs::read_to_string(path).ok());
        format_label(
            env!("CARGO_PKG_VERSION"),
            cfg!(debug_assertions),
            receipt.as_deref(),
        )
    })
}

fn format_label(version: &str, debug: bool, receipt: Option<&str>) -> String {
    let mode = if debug { "Debug" } else { "Release" };
    let fallback = format!("v{version} · {mode}");
    let Some(data) = receipt.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()) else {
        return fallback;
    };
    // Apple's marketing version omits the Cargo prerelease suffix.
    if data["version"].as_str() != version.split('-').next() {
        return fallback;
    }
    let Some(build) = data["build"].as_str().filter(|s| {
        !s.is_empty()
            && s.len() <= 20
            && s.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    }) else {
        return fallback;
    };
    let Some(revision) = data["source_revision"]
        .as_str()
        .filter(|s| s.len() >= 7 && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return fallback;
    };
    let modified = if data["source_dirty"].as_bool() == Some(true) {
        "*"
    } else {
        ""
    };
    format!(
        "v{version} ({build}) · {}{modified} · {mode}",
        &revision[..7]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_build_and_modified_sources_are_identifiable() {
        let receipt = r#"{"version":"0.28.1","build":"14","source_revision":"52bf02dac5985164a31631555a84af561cf3cf8e","source_dirty":true}"#;
        assert_eq!(
            format_label("0.28.1-rc.1", true, Some(receipt)),
            "v0.28.1-rc.1 (14) · 52bf02d* · Debug"
        );
    }

    #[test]
    fn absent_malformed_or_different_version_receipt_never_overrides_binary_version() {
        for receipt in [
            None,
            Some("invalid"),
            Some(r#"{"version":"0.27.0","build":"12","source_revision":"abcdef0"}"#),
            Some(r#"{"version":"0.28.1","build":"14","source_revision":"非ascii"}"#),
        ] {
            assert_eq!(format_label("0.28.1", false, receipt), "v0.28.1 · Release");
        }
    }
}
