//! arena-sync — pull Ekza Arena avatar cards into the omoba roster.
//!
//! Reads `ArenaAssetData` accounts straight from the Solana RPC (no anchor
//! client), keeps the Avatar cards, fetches each card's metadata JSON, checks
//! the model-format classifier against what omoba supports (VRM/GLB — both
//! load through Bevy's glTF loader), downloads the model + thumbnail into
//! `client/assets/avatars/`, and merges the entries into `manifest.json`.
//!
//! The game then picks the new avatars up at startup: `shared::avatar_roster()`
//! reads the manifest at runtime (embedded copy is only a fallback), so no
//! rebuild is needed — run this tool, restart the client, the arena avatars
//! appear in the "Choose Avatar" grid.
//!
//! Usage:
//!   cargo run -p arena-sync -- [--rpc http://127.0.0.1:8899] \
//!     [--arena-program D3a99Wj3eLLn4jbXU5rLDbaFT6giQiUbmcPkiyQSM8iZ] \
//!     [--assets client/assets] [--dry-run]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const DEFAULT_RPC: &str = "http://127.0.0.1:8899";
const DEFAULT_ARENA_PROGRAM: &str = "D3a99Wj3eLLn4jbXU5rLDbaFT6giQiUbmcPkiyQSM8iZ";
/// Model formats omoba can actually load (both are glTF-binary containers for
/// Bevy's loader). Mirrors the on-chain ProjectProfile "omoba" registered in
/// solana-stellar (docs/INTEGRATION.md "Model formats").
const SUPPORTED_FORMATS: [&str; 2] = ["vrm", "glb"];

struct Args {
    rpc: String,
    arena_program: String,
    assets: PathBuf,
    dry_run: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        rpc: DEFAULT_RPC.to_string(),
        arena_program: DEFAULT_ARENA_PROGRAM.to_string(),
        assets: PathBuf::from("client/assets"),
        dry_run: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--rpc" => {
                args.rpc = argv[index + 1].clone();
                index += 1;
            }
            "--arena-program" => {
                args.arena_program = argv[index + 1].clone();
                index += 1;
            }
            "--assets" => {
                args.assets = PathBuf::from(&argv[index + 1]);
                index += 1;
            }
            "--dry-run" => args.dry_run = true,
            "--help" | "-h" => {
                println!(
                    "arena-sync [--rpc url] [--arena-program pubkey] [--assets client/assets] [--dry-run]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
        index += 1;
    }
    args
}

/// Minimal borsh cursor for the fields we need out of `ArenaAssetData`.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(slice)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

struct AvatarCard {
    metadata_pointer: String,
    archetype_id: String,
}

/// Parse the head of an `ArenaAssetData` account (fields up to and including
/// `archetype_id`); returns None for non-Avatar cards or foreign layouts.
fn parse_avatar_card(data: &[u8], discriminator: &[u8; 8]) -> Option<AvatarCard> {
    if data.len() < 8 || &data[..8] != discriminator {
        return None;
    }
    let mut cursor = Cursor::new(&data[8..]);
    let metadata_pointer = cursor.string()?;
    cursor.take(32)?; // creator
    cursor.take(8)?; // index
    let card_kind = cursor.u8()?;
    if card_kind != 0 {
        return None; // 0 = Avatar, 1 = Modifier
    }
    let archetype_id = cursor.string()?;
    Some(AvatarCard {
        metadata_pointer,
        archetype_id,
    })
}

fn account_discriminator(name: &str) -> [u8; 8] {
    let digest = Sha256::digest(format!("account:{name}").as_bytes());
    digest[..8].try_into().expect("sha256 is at least 8 bytes")
}

fn fetch_program_accounts(
    client: &reqwest::blocking::Client,
    rpc: &str,
    program: &str,
) -> Result<Vec<Vec<u8>>, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [program, { "encoding": "base64", "commitment": "confirmed" }],
    });
    let response: Value = client
        .post(rpc)
        .json(&body)
        .send()
        .map_err(|e| format!("rpc request failed: {e}"))?
        .json()
        .map_err(|e| format!("rpc response is not JSON: {e}"))?;
    let result = response
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("unexpected rpc response: {response}"))?;
    let engine = base64::engine::general_purpose::STANDARD;
    let mut accounts = Vec::new();
    for entry in result {
        let Some(encoded) = entry.pointer("/account/data/0").and_then(Value::as_str) else {
            continue;
        };
        if let Ok(bytes) = engine.decode(encoded) {
            accounts.push(bytes);
        }
    }
    Ok(accounts)
}

fn http_get_bytes(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("GET {url}: HTTP {}", response.status()));
    }
    response
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("GET {url}: body: {e}"))
}

fn is_glb(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[..4] == b"glTF"
}

fn main() {
    let args = parse_args();
    let avatars_dir = args.assets.join("avatars");
    let manifest_path = avatars_dir.join("manifest.json");
    if !manifest_path.exists() {
        eprintln!(
            "manifest not found at {} — run from the repo root or pass --assets",
            manifest_path.display()
        );
        std::process::exit(2);
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("http client");

    let accounts = match fetch_program_accounts(&client, &args.rpc, &args.arena_program) {
        Ok(accounts) => accounts,
        Err(error) => {
            eprintln!("failed to read arena accounts: {error}");
            std::process::exit(1);
        }
    };
    let discriminator = account_discriminator("ArenaAssetData");
    let cards: Vec<AvatarCard> = accounts
        .iter()
        .filter_map(|data| parse_avatar_card(data, &discriminator))
        .collect();
    println!(
        "arena: {} account(s), {} avatar card(s)",
        accounts.len(),
        cards.len()
    );

    // Existing manifest → slug-keyed map for an idempotent merge.
    let mut manifest: Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("read manifest"))
            .expect("manifest must be valid JSON");
    let entries = manifest["avatars"].as_array().cloned().unwrap_or_default();
    let mut by_slug: BTreeMap<String, Value> = entries
        .into_iter()
        .filter_map(|entry| Some((entry["slug"].as_str()?.to_string(), entry)))
        .collect();

    let mut added = 0usize;
    let mut skipped = 0usize;
    for card in &cards {
        let slug = card.archetype_id.replace('_', "-");
        if by_slug.contains_key(&slug) {
            continue; // already synced
        }
        if !card.metadata_pointer.starts_with("http") {
            eprintln!(
                "skip {}: metadata pointer is not fetchable here ({})",
                card.archetype_id, card.metadata_pointer
            );
            skipped += 1;
            continue;
        }
        let metadata: Value = match http_get_bytes(&client, &card.metadata_pointer)
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
        {
            Ok(value) => value,
            Err(error) => {
                eprintln!("skip {}: {error}", card.archetype_id);
                skipped += 1;
                continue;
            }
        };

        // Model-format gate: only formats omoba's loader supports.
        let format = metadata["format"].as_str().unwrap_or("");
        if !SUPPORTED_FORMATS.contains(&format) {
            eprintln!(
                "skip {}: format {:?} not in supported {:?} (see solana-stellar ProjectProfile \"omoba\")",
                card.archetype_id, format, SUPPORTED_FORMATS
            );
            skipped += 1;
            continue;
        }

        let Some(model_url) = metadata["model_vrm"]
            .as_str()
            .or_else(|| metadata["model"].as_str())
        else {
            eprintln!(
                "skip {}: no model pointer in card metadata",
                card.archetype_id
            );
            skipped += 1;
            continue;
        };
        let display_name = metadata["name"].as_str().unwrap_or(&card.archetype_id);
        let author = metadata
            .pointer("/attribution/author")
            .and_then(Value::as_str)
            .unwrap_or("");
        let license = metadata
            .pointer("/attribution/license")
            .and_then(Value::as_str)
            .unwrap_or("");
        let image_url = metadata["image"].as_str().unwrap_or("");

        if args.dry_run {
            println!("would sync {slug}: {display_name} ({format}, {license}) <- {model_url}");
            added += 1;
            continue;
        }

        // Download the model. VRM is a glTF-binary container, so staging it
        // with a .glb name makes Bevy's stock loader pick it up.
        let model_bytes = match http_get_bytes(&client, model_url) {
            Ok(bytes) if is_glb(&bytes) => bytes,
            Ok(_) => {
                eprintln!("skip {slug}: model is not glTF-binary");
                skipped += 1;
                continue;
            }
            Err(error) => {
                eprintln!("skip {slug}: {error}");
                skipped += 1;
                continue;
            }
        };
        fs::create_dir_all(&avatars_dir).expect("create avatars dir");
        fs::write(avatars_dir.join(format!("{slug}.glb")), &model_bytes).expect("write model glb");

        let mut thumbnail = None;
        if image_url.starts_with("http") {
            if let Ok(bytes) = http_get_bytes(&client, image_url) {
                let file = format!("{slug}.png");
                fs::write(avatars_dir.join(&file), bytes).expect("write thumbnail");
                thumbnail = Some(file);
            }
        }

        let entry = json!({
            "slug": slug,
            "display_name": display_name,
            "collection": "Ekza Arena",
            "license": license,
            "source_url": model_url,
            "author": author,
            "thumbnail": thumbnail,
        });
        println!("synced {slug}: {display_name} ({format}, {license})");
        by_slug.insert(slug, entry);
        added += 1;
    }

    if !args.dry_run && added > 0 {
        // Preserve original order for existing entries, append new ones sorted.
        let original: Vec<Value> = manifest["avatars"].as_array().cloned().unwrap_or_default();
        let mut merged = original.clone();
        let existing: Vec<String> = original
            .iter()
            .filter_map(|entry| entry["slug"].as_str().map(str::to_string))
            .collect();
        for (slug, entry) in &by_slug {
            if !existing.contains(slug) {
                merged.push(entry.clone());
            }
        }
        manifest["avatars"] = Value::Array(merged);
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("serialize manifest") + "\n",
        )
        .expect("write manifest");
    }

    println!(
        "done: {added} added, {skipped} skipped, manifest {}",
        manifest_path.display()
    );
    if added > 0 && !args.dry_run {
        println!("restart the client — the roster loads the manifest at startup.");
    }
}
