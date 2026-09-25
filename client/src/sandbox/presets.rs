use shared::sandbox::*;
use std::path::PathBuf;

pub(super) const BUILTINS: [&str; 4] = ["duel", "late-game", "dps", "animation"];
pub(super) fn builtin(name: &str) -> Option<SandboxConfig> {
    let mut c = SandboxConfig::default();
    match name {
        "duel" => {
            c.player.level = 10;
            c.player.ranks = [3; 4];
            c.enemy.enabled = true;
            c.enemy.actor.level = 10;
            c.enemy.actor.ranks = [3; 4];
            c.enemy.behavior = BotBehavior::Fight;
        }
        "late-game" => {
            c.player.level = 10;
            c.player.ranks = [3; 4];
            c.player.inventory = shared::shop::items()
                .iter()
                .map(|i| i.id)
                .take(shared::shop::INVENTORY_CAPACITY)
                .collect();
            c.enemy.enabled = true;
            c.enemy.actor.level = 10;
            c.enemy.actor.ranks = [3; 4];
            c.enemy.behavior = BotBehavior::Fight;
            c.environment.minions = true;
        }
        "dps" => {
            c.player.level = 10;
            c.player.ranks = [3; 4];
            c.player.infinite_resource = true;
            c.dummy.enabled = true;
        }
        "animation" => {
            c.player.unlock_all = true;
            c.player.infinite_resource = true;
            c.player.no_cooldowns = true;
            c.dummy.enabled = true;
            c.environment.time_scale = 0.25;
        }
        _ => return None,
    }
    Some(c)
}
pub(super) fn directory() -> PathBuf {
    // Separate from per-session client preferences so presets survive launcher runs.
    std::env::var_os("OMOBA_SANDBOX_PRESET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            omoba_passport::assets::client_asset_root()
                .parent()
                .and_then(|p| p.parent())
                .unwrap_or(std::path::Path::new("."))
                .join("target/combat-presets")
        })
}
fn named_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err("Preset name: 1–64 letters, numbers, - or _".into());
    }
    Ok(directory().join(format!("{name}.json")))
}
pub(super) fn load(name: &str) -> Result<SandboxConfig, String> {
    if let Some(c) = builtin(name) {
        return Ok(c);
    }
    let path = if name.ends_with(".json") {
        PathBuf::from(name)
    } else {
        named_path(name)?
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("Cannot load {}: {e}", path.display()))?;
    if bytes.len() > 64 * 1024 {
        return Err("Preset exceeds 64 KiB".into());
    }
    let c: SandboxConfig =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid preset: {e}"))?;
    if c.version != PRESET_VERSION {
        return Err("Unsupported preset version".into());
    }
    Ok(c)
}
pub(super) fn save(name: &str, config: &SandboxConfig) -> Result<PathBuf, String> {
    let path = named_path(name)?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_presets_reject_missing_partial_malformed_and_unknown_versions() {
        let path = std::env::temp_dir().join(format!(
            "omoba-preset-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let name = path.to_str().unwrap();
        assert!(load(name).is_err());
        for bytes in ["not json", "{}", "{\"version\":1}"] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load(name).is_err());
        }
        let mut config = builtin("late-game").unwrap();
        config.version = PRESET_VERSION + 1;
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        assert!(load(name).unwrap_err().contains("version"));
        config.version = PRESET_VERSION;
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        assert_eq!(load(name).unwrap(), config);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn presets_cover_real_catalogue_and_roundtrip() {
        for name in BUILTINS {
            let c = builtin(name).unwrap();
            let encoded = serde_json::to_vec(&c).unwrap();
            let read: SandboxConfig = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(c, read);
            assert_eq!(read.version, PRESET_VERSION);
        }
        assert_eq!(
            builtin("late-game").unwrap().player.inventory.len(),
            shared::shop::items()
                .len()
                .min(shared::shop::INVENTORY_CAPACITY)
        );
        assert!(named_path("../../escape").is_err());
    }
}
