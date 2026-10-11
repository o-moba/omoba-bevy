//! The registry's `license.attribution` line for Ekza Studio items.
//!
//! The pinned SDK's store templates (`StoreAsset`, `StoreAvatar`) keep the
//! creator name and the licence text, but not the attribution line that a
//! CC-BY item needs wherever the game credits it. Omoba reads that line from
//! the same registry documents, keyed by canonical asset id, and keeps a copy
//! next to each store's offline catalogue. It is display text only: it never
//! decides what is listed, installed or admitted.

use ekza_bevy_sdk::{assets::CatalogAsset, catalog::CatalogV2Avatar};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path};

/// Attribution text by canonical asset id (`ekza:weapon:<uuid>`, ...).
pub type Attributions = BTreeMap<String, String>;

/// Longest line shown, in characters; longer text ends with an ellipsis.
pub const MAX_CHARS: usize = 300;
const MAX_ENTRIES: usize = 4096;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const SCHEMA: &str = "omoba.attributions.v1";

/// Creator text as shown: whitespace and control characters collapse to single
/// spaces and the line is bounded. Blank text is `None`.
pub fn clean(text: &str) -> Option<String> {
    let line = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= MAX_CHARS {
        return Some(line);
    }
    let mut cut: String = line.chars().take(MAX_CHARS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    Some(cut)
}

fn collect<'a>(entries: impl IntoIterator<Item = (&'a str, &'a str)>) -> Attributions {
    entries
        .into_iter()
        .filter_map(|(id, text)| Some((id.to_owned(), clean(text)?)))
        .take(MAX_ENTRIES)
        .collect()
}

/// From `/v2/assets` items of any kind, as the public catalogue and the
/// account library's `assets` list them.
pub fn of_assets<'a>(items: impl IntoIterator<Item = &'a CatalogAsset>) -> Attributions {
    collect(items.into_iter().filter_map(|item| {
        Some((
            item.id.as_str(),
            item.license.as_ref()?.attribution.as_str(),
        ))
    }))
}

/// From `/v2/avatars` items.
pub fn of_avatars<'a>(items: impl IntoIterator<Item = &'a CatalogV2Avatar>) -> Attributions {
    collect(items.into_iter().filter_map(|item| {
        Some((
            item.id.as_str(),
            item.license.as_ref()?.attribution.as_str(),
        ))
    }))
}

/// Every asset the connected account's library lists, any kind.
pub fn of_library(session: &crate::account::AccountSession) -> Attributions {
    of_assets(session.library_assets().iter().map(|entry| &entry.asset))
}

#[derive(Serialize, Deserialize)]
struct Document {
    schema: String,
    attributions: Attributions,
}

/// The saved copy; a missing, oversized or malformed file is empty.
pub fn load(path: &Path) -> Attributions {
    let Ok(file) = std::fs::File::open(path) else {
        return Attributions::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_FILE_BYTES
    {
        return Attributions::new();
    }
    match serde_json::from_slice::<Document>(&bytes) {
        Ok(document) if document.schema == SCHEMA => collect(
            document
                .attributions
                .iter()
                .map(|(id, text)| (id.as_str(), text.as_str())),
        ),
        _ => Attributions::new(),
    }
}

/// Replace the saved copy atomically.
pub fn save(path: &Path, attributions: &Attributions) -> Result<(), String> {
    let bytes = serde_json::to_vec(&Document {
        schema: SCHEMA.into(),
        attributions: attributions.clone(),
    })
    .map_err(|error| error.to_string())?;
    ekza_bevy_sdk::cache::atomic_write(path, &bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creator_text_is_one_bounded_line() {
        assert_eq!(clean("  \n\t "), None);
        assert_eq!(
            clean(" Pyre Sorcerer by\nPolygonal Mind,\u{7}  CC0 1.0 ").as_deref(),
            Some("Pyre Sorcerer by Polygonal Mind, CC0 1.0")
        );
        let long = clean(&"word ".repeat(200)).unwrap();
        assert_eq!(long.chars().count(), MAX_CHARS);
        assert!(long.ends_with("word…"), "{long}");
        assert!(!long.contains("  "));
    }

    #[test]
    fn registry_items_map_by_canonical_id_and_skip_blank_lines() {
        let assets: Vec<CatalogAsset> = serde_json::from_value(serde_json::json!([
            {"assetKind":"weapon","id":"ekza:weapon:1","name":"Set",
             "license":{"text":"CC-BY-4.0","attribution":"Set by Alice, CC-BY-4.0"}},
            {"assetKind":"weapon","id":"ekza:weapon:2","name":"Blank",
             "license":{"text":"CC0-1.0","attribution":"  "}},
            {"assetKind":"avatar","id":"ekza:avatar:3","name":"No licence"}
        ]))
        .unwrap();
        assert_eq!(
            of_assets(&assets),
            Attributions::from([(
                "ekza:weapon:1".to_owned(),
                "Set by Alice, CC-BY-4.0".to_owned()
            )])
        );
        let avatars: Vec<CatalogV2Avatar> = serde_json::from_value(serde_json::json!([
            {"id":"ekza:avatar:4","name":"Pyre","license":{"text":"CC0-1.0",
             "attribution":"Pyre by Polygonal Mind"}}
        ]))
        .unwrap();
        assert_eq!(
            of_avatars(&avatars)
                .get("ekza:avatar:4")
                .map(String::as_str),
            Some("Pyre by Polygonal Mind")
        );
    }

    #[test]
    fn the_saved_copy_round_trips_and_bad_files_read_as_empty() {
        let dir = std::env::temp_dir().join(format!(
            "omoba-attributions-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("attributions.json");
        assert!(load(&path).is_empty());
        let saved = Attributions::from([("ekza:weapon:1".to_owned(), "Set by Alice".to_owned())]);
        save(&path, &saved).unwrap();
        assert_eq!(load(&path), saved);
        std::fs::write(&path, br#"{"schema":"other","attributions":{"a":"b"}}"#).unwrap();
        assert!(load(&path).is_empty());
        std::fs::write(&path, b"not json").unwrap();
        assert!(load(&path).is_empty());
        // Text edited on disk is cleaned again on the way in.
        std::fs::write(
            &path,
            br#"{"schema":"omoba.attributions.v1","attributions":{"a":" x\ny ","b":" "}}"#,
        )
        .unwrap();
        assert_eq!(
            load(&path),
            Attributions::from([("a".to_owned(), "x y".to_owned())])
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
