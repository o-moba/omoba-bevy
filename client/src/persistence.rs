//! On-disk client preferences: graphics, character selection, optional server address.
//!
//! **Load path**: `OMOBA_CLIENT_CONFIG_DIR` if set, else platform default
//! (`~/.config/omoba-bevy/client_preferences.json` on Unix,
//! `%APPDATA%/omoba-bevy/client_preferences.json` on Windows).
//!
//! **Server address precedence** matches [`crate::session_config`] docs.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::session_config::DEFAULT_GAME_SERVER_ADDR;
use crate::team::CharacterChoice;
use crate::world::{
    LightingSettings, MAX_AMBIENT_BRIGHTNESS, MAX_LIGHT_ILLUMINANCE, MAX_LIGHT_PITCH_DEG,
    MAX_LIGHT_YAW_DEG, MAX_MODEL_TARGET_HEIGHT, MIN_AMBIENT_BRIGHTNESS, MIN_LIGHT_ILLUMINANCE,
    MIN_LIGHT_PITCH_DEG, MIN_LIGHT_YAW_DEG, MIN_MODEL_TARGET_HEIGHT, ModelScaleSettings,
};

const SCHEMA_VERSION: u32 = 1;
const PREFS_FILENAME: &str = "client_preferences.json";

/// Address loaded from disk for use when `GAME_SERVER_ADDR` is unset (validated).
#[derive(Resource, Default, Clone)]
pub struct FileGameServerAddr(pub Option<String>);

/// Last address used for UDP (env, file, or default), for persisting alongside graphics prefs.
#[derive(Resource, Default, Clone)]
pub struct ResolvedServerAddressForPrefs(pub String);

pub struct ClientPersistencePlugin;

impl Plugin for ClientPersistencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FileGameServerAddr>()
            .init_resource::<ResolvedServerAddressForPrefs>()
            .init_resource::<ClientPrefsSaveGate>()
            .add_systems(Startup, load_persistent_client_settings)
            .add_systems(Update, save_client_preferences_on_change);
    }
}

/// Prevents spurious saves during the first frames after startup.
#[derive(Resource, Default)]
pub struct ClientPrefsSaveGate {
    pub suppress_saves: u8,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClientPreferencesFile {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    game_server_addr: Option<String>,
    #[serde(default)]
    character: Option<CharacterChoice>,
    #[serde(default)]
    model_target_height: Option<f32>,
    #[serde(default)]
    illuminance: Option<f32>,
    #[serde(default)]
    ambient_brightness: Option<f32>,
    #[serde(default)]
    light_pitch_deg: Option<f32>,
    #[serde(default)]
    light_yaw_deg: Option<f32>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

fn preferences_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OMOBA_CLIENT_CONFIG_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join(PREFS_FILENAME));
        }
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("APPDATA")
            .map(|base| PathBuf::from(base).join("omoba-bevy").join(PREFS_FILENAME));
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join(".config")
                .join("omoba-bevy")
                .join(PREFS_FILENAME)
        })
    }
}

/// Validates `host:port` or a parseable [`SocketAddr`] string for client config.
pub fn validate_game_server_addr(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if t.parse::<SocketAddr>().is_ok() {
        return Some(t.to_string());
    }
    let (host, port_str) = t.rsplit_once(':')?;
    let host = host.trim();
    if host.is_empty() || host.contains([' ', '\t', '\n', '\r']) {
        return None;
    }
    if host.len() > 253 {
        return None;
    }
    let port: u16 = port_str.trim().parse().ok()?;
    Some(format!("{host}:{port}"))
}

pub fn clamp_model_target_height(value: f32) -> f32 {
    value.clamp(MIN_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT)
}

pub fn clamp_lighting_settings(mut s: LightingSettings) -> LightingSettings {
    s.illuminance = s
        .illuminance
        .clamp(MIN_LIGHT_ILLUMINANCE, MAX_LIGHT_ILLUMINANCE);
    s.ambient_brightness = s
        .ambient_brightness
        .clamp(MIN_AMBIENT_BRIGHTNESS, MAX_AMBIENT_BRIGHTNESS);
    s.light_pitch_deg = s
        .light_pitch_deg
        .clamp(MIN_LIGHT_PITCH_DEG, MAX_LIGHT_PITCH_DEG);
    s.light_yaw_deg = s.light_yaw_deg.clamp(MIN_LIGHT_YAW_DEG, MAX_LIGHT_YAW_DEG);
    s
}

fn read_preferences_file(path: &Path) -> io::Result<ClientPreferencesFile> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn write_preferences_file(path: &Path, prefs: &ClientPreferencesFile) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(prefs)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(path, data)
}

/// Startup: load JSON if present and apply to resources; always sets [`FileGameServerAddr`].
pub fn load_persistent_client_settings(
    mut file_addr: ResMut<FileGameServerAddr>,
    mut lighting: ResMut<LightingSettings>,
    mut model: ResMut<ModelScaleSettings>,
    mut team: ResMut<crate::team::TeamSelection>,
    mut gate: ResMut<ClientPrefsSaveGate>,
) {
    gate.suppress_saves = 3;
    file_addr.0 = None;

    let Some(path) = preferences_path() else {
        warn!("No home/config directory for client preferences; using defaults only.");
        return;
    };

    if !path.exists() {
        return;
    }

    let disk = match read_preferences_file(&path) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to read client preferences at {:?}: {e}", path);
            return;
        }
    };

    if disk.schema_version > SCHEMA_VERSION {
        warn!(
            "Client preferences schema {} is newer than supported {}; ignoring file {:?}",
            disk.schema_version, SCHEMA_VERSION, path
        );
        return;
    }

    if let Some(addr_raw) = disk.game_server_addr.as_deref() {
        if let Some(addr) = validate_game_server_addr(addr_raw) {
            file_addr.0 = Some(addr);
        } else {
            warn!("Ignoring invalid game_server_addr in preferences file.");
        }
    }

    if let Some(ch) = disk.character {
        team.character = ch;
    }

    if let Some(h) = disk.model_target_height {
        model.target_height = clamp_model_target_height(h);
    }

    if disk.illuminance.is_some()
        || disk.ambient_brightness.is_some()
        || disk.light_pitch_deg.is_some()
        || disk.light_yaw_deg.is_some()
    {
        if let Some(v) = disk.illuminance {
            lighting.illuminance = v;
        }
        if let Some(v) = disk.ambient_brightness {
            lighting.ambient_brightness = v;
        }
        if let Some(v) = disk.light_pitch_deg {
            lighting.light_pitch_deg = v;
        }
        if let Some(v) = disk.light_yaw_deg {
            lighting.light_yaw_deg = v;
        }
        *lighting = clamp_lighting_settings(*lighting);
    }
}

fn build_file_from_state(
    lighting: &LightingSettings,
    model: &ModelScaleSettings,
    character: CharacterChoice,
    game_server_addr: &str,
) -> ClientPreferencesFile {
    ClientPreferencesFile {
        schema_version: SCHEMA_VERSION,
        game_server_addr: Some(game_server_addr.to_string()),
        character: Some(character),
        model_target_height: Some(model.target_height),
        illuminance: Some(lighting.illuminance),
        ambient_brightness: Some(lighting.ambient_brightness),
        light_pitch_deg: Some(lighting.light_pitch_deg),
        light_yaw_deg: Some(lighting.light_yaw_deg),
    }
}

/// Writes current resources to disk (graphics + character + active server display string).
pub fn save_client_preferences_to_disk(
    lighting: &LightingSettings,
    model: &ModelScaleSettings,
    character: CharacterChoice,
    game_server_addr: &str,
) -> io::Result<()> {
    let Some(path) = preferences_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no preferences path (set HOME/APPDATA or OMOBA_CLIENT_CONFIG_DIR)",
        ));
    };
    let addr = validate_game_server_addr(game_server_addr).unwrap_or_else(|| {
        validate_game_server_addr(DEFAULT_GAME_SERVER_ADDR)
            .expect("default game server addr must validate")
    });
    let prefs = build_file_from_state(lighting, model, character, &addr);
    write_preferences_file(&path, &prefs)
}

fn save_client_preferences_on_change(
    mut gate: ResMut<ClientPrefsSaveGate>,
    lighting: Res<LightingSettings>,
    model: Res<ModelScaleSettings>,
    team: Res<crate::team::TeamSelection>,
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
) {
    if gate.suppress_saves > 0 {
        gate.suppress_saves -= 1;
        return;
    }

    let changed = lighting.is_changed()
        || model.is_changed()
        || team.is_changed()
        || resolved_addr.is_changed();
    if !changed {
        return;
    }

    let addr = resolved_addr.0.as_str();
    if let Err(e) = save_client_preferences_to_disk(
        lighting.as_ref(),
        model.as_ref(),
        team.character,
        if addr.is_empty() {
            DEFAULT_GAME_SERVER_ADDR
        } else {
            addr
        },
    ) {
        warn!("Failed to save client preferences: {e}");
    }
}

/// Resets graphics settings to defaults, persists, and re-opens save gate briefly.
pub fn reset_graphics_to_defaults(
    lighting: &mut LightingSettings,
    model: &mut ModelScaleSettings,
    gate: &mut ClientPrefsSaveGate,
    character: CharacterChoice,
    game_server_addr: &str,
) {
    *lighting = LightingSettings::default();
    *model = ModelScaleSettings::default();
    gate.suppress_saves = 1;
    if let Err(e) = save_client_preferences_to_disk(lighting, model, character, game_server_addr) {
        warn!("Failed to save preferences after reset: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_socket_addr() {
        assert_eq!(
            validate_game_server_addr("127.0.0.1:4000").as_deref(),
            Some("127.0.0.1:4000")
        );
    }

    #[test]
    fn validate_accepts_host_port() {
        assert_eq!(
            validate_game_server_addr("localhost:5000").as_deref(),
            Some("localhost:5000")
        );
    }

    #[test]
    fn validate_rejects_empty_and_garbage() {
        assert!(validate_game_server_addr("").is_none());
        assert!(validate_game_server_addr("   ").is_none());
        assert!(validate_game_server_addr("nocolon").is_none());
        assert!(validate_game_server_addr("host:").is_none());
    }

    #[test]
    fn clamp_model_respects_bounds() {
        assert_eq!(clamp_model_target_height(0.0), MIN_MODEL_TARGET_HEIGHT);
        assert_eq!(clamp_model_target_height(99.0), MAX_MODEL_TARGET_HEIGHT);
    }

    #[test]
    fn clamp_lighting_respects_bounds() {
        let mut s = LightingSettings::default();
        s.illuminance = 1.0;
        s.ambient_brightness = 999_999.0;
        let c = clamp_lighting_settings(s);
        assert_eq!(c.illuminance, MIN_LIGHT_ILLUMINANCE);
        assert_eq!(c.ambient_brightness, MAX_AMBIENT_BRIGHTNESS);
    }
}
