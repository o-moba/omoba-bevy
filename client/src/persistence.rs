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
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::model_scale::{
    DEFAULT_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT, MIN_MODEL_TARGET_HEIGHT,
    ModelScaleSettings,
};
use crate::session_config::{DEFAULT_GAME_SERVER_ADDR, FALLBACK_GAME_SERVER_ADDR};
use crate::team::CharacterChoice;
use crate::world::{
    LightingSettings, MAX_AMBIENT_BRIGHTNESS, MAX_LIGHT_ILLUMINANCE, MAX_LIGHT_PITCH_DEG,
    MAX_LIGHT_YAW_DEG, MIN_AMBIENT_BRIGHTNESS, MIN_LIGHT_ILLUMINANCE, MIN_LIGHT_PITCH_DEG,
    MIN_LIGHT_YAW_DEG,
};

const SCHEMA_VERSION: u32 = 3;
const PREVIOUS_DEFAULT_MODEL_TARGET_HEIGHT: f32 = 1.15;
const PREFS_FILENAME: &str = "client_preferences.json";
const CLIENT_SESSION_ID_MAX_LEN: usize = 64;

/// Address loaded from disk for use when `GAME_SERVER_ADDR` is unset (validated).
#[derive(Resource, Default, Clone)]
pub struct FileGameServerAddr(pub Option<String>);

/// Last address used for UDP (env, file, or default), for persisting alongside graphics prefs.
#[derive(Resource, Default, Clone)]
pub struct ResolvedServerAddressForPrefs(pub String);

/// Stable per-install client id sent in Join packets for server-side reconnect reclaim.
#[derive(Resource, Clone)]
pub struct ClientSessionId(pub String);

impl Default for ClientSessionId {
    fn default() -> Self {
        Self(generate_client_session_id())
    }
}

pub struct ClientPersistencePlugin;

impl Plugin for ClientPersistencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FileGameServerAddr>()
            .init_resource::<ResolvedServerAddressForPrefs>()
            .init_resource::<ClientSessionId>()
            .init_resource::<ClientPreferencesInitialSavePending>()
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

#[derive(Resource, Default)]
pub(crate) struct ClientPreferencesInitialSavePending(bool);

#[derive(Debug, Serialize, Deserialize)]
struct ClientPreferencesFile {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    game_server_addr: Option<String>,
    #[serde(default)]
    client_session_id: Option<String>,
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
    // Files without an explicit version predate the current preference schema.
    1
}

fn generate_client_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("omoba-{}-{nanos}", std::process::id())
}

pub fn validate_client_session_id(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.len() > CLIENT_SESSION_ID_MAX_LEN {
        return None;
    }
    if !t
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(t.to_string())
}

fn preferences_path() -> Option<PathBuf> {
    crate::platform::preferences_file_path(
        std::env::var("OMOBA_CLIENT_CONFIG_DIR").ok().as_deref(),
        crate::platform::preferences_directory(),
        PREFS_FILENAME,
    )
}

/// Validates `host:port` or a parseable [`SocketAddr`] string for client config.
pub fn validate_game_server_addr(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(address) = t.parse::<SocketAddr>() {
        return (address.port() != 0).then(|| address.to_string());
    }
    let (host, port_str) = t.rsplit_once(':')?;
    let host = host.trim();
    // This is a UDP host:port field, never a web URL or an arbitrary path.
    let dns_name = host.strip_suffix('.').unwrap_or(host);
    if dns_name.is_empty()
        || dns_name.len() > 253
        || dns_name.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return None;
    }
    let port: u16 = port_str.trim().parse().ok()?;
    (port != 0).then(|| format!("{host}:{port}"))
}

pub fn clamp_model_target_height(value: f32) -> f32 {
    value.clamp(MIN_MODEL_TARGET_HEIGHT, MAX_MODEL_TARGET_HEIGHT)
}

/// Migrates old defaults once, preserving custom sizes in the supported range.
/// Schema 3 records the larger hero default; a deliberate 1.15 saved under
/// schema 3 therefore survives subsequent launches. Earlier subminimum values
/// belong to the obsolete world scale (whose default was 0.26).
pub fn migrate_model_target_height(stored: f32, schema_version: u32) -> f32 {
    if schema_version < 3
        && (stored < MIN_MODEL_TARGET_HEIGHT
            || (stored - PREVIOUS_DEFAULT_MODEL_TARGET_HEIGHT).abs() < f32::EPSILON)
    {
        DEFAULT_MODEL_TARGET_HEIGHT
    } else {
        clamp_model_target_height(stored)
    }
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
    mut client_session_id: ResMut<ClientSessionId>,
    mut initial_save_pending: ResMut<ClientPreferencesInitialSavePending>,
    mut gate: ResMut<ClientPrefsSaveGate>,
) {
    gate.suppress_saves = 3;
    file_addr.0 = None;
    initial_save_pending.0 = false;

    let Some(path) = preferences_path() else {
        warn!("No home/config directory for client preferences; using defaults only.");
        return;
    };

    if !path.exists() {
        initial_save_pending.0 = true;
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
    // Startup's save gate outlives Bevy's resource change tick. Explicitly
    // queue the migrated schema so the one-time default migration is persisted.
    initial_save_pending.0 = disk.schema_version < SCHEMA_VERSION;

    if let Some(addr_raw) = disk.game_server_addr.as_deref() {
        if let Some(addr) = validate_game_server_addr(addr_raw) {
            file_addr.0 = Some(addr);
        } else {
            warn!("Ignoring invalid game_server_addr in preferences file.");
        }
    }

    if let Some(raw_session_id) = disk.client_session_id.as_deref() {
        if let Some(session_id) = validate_client_session_id(raw_session_id) {
            client_session_id.0 = session_id;
        } else {
            warn!("Ignoring invalid client_session_id in preferences file.");
            initial_save_pending.0 = true;
        }
    } else {
        initial_save_pending.0 = true;
    }

    if let Some(ch) = disk.character {
        team.character = ch;
    }

    if let Some(h) = disk.model_target_height {
        let resolved = migrate_model_target_height(h, disk.schema_version);
        if (resolved - h).abs() > f32::EPSILON && disk.schema_version < SCHEMA_VERSION {
            info!("Migrating legacy model target height {h:.3} -> {resolved:.3}.");
        }
        model.target_height = resolved;
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
    client_session_id: &str,
) -> ClientPreferencesFile {
    ClientPreferencesFile {
        schema_version: SCHEMA_VERSION,
        game_server_addr: Some(game_server_addr.to_string()),
        client_session_id: Some(client_session_id.to_string()),
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
    client_session_id: &str,
) -> io::Result<()> {
    let Some(path) = preferences_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no preferences path (set HOME/APPDATA or OMOBA_CLIENT_CONFIG_DIR)",
        ));
    };
    let addr = validate_game_server_addr(game_server_addr).unwrap_or_else(|| {
        validate_game_server_addr(DEFAULT_GAME_SERVER_ADDR)
            .unwrap_or_else(|| FALLBACK_GAME_SERVER_ADDR.to_string())
    });
    let session_id =
        validate_client_session_id(client_session_id).unwrap_or_else(generate_client_session_id);
    let prefs = build_file_from_state(lighting, model, character, &addr, &session_id);
    write_preferences_file(&path, &prefs)
}

fn save_client_preferences_on_change(
    mut gate: ResMut<ClientPrefsSaveGate>,
    mut initial_save_pending: ResMut<ClientPreferencesInitialSavePending>,
    lighting: Res<LightingSettings>,
    model: Res<ModelScaleSettings>,
    team: Res<crate::team::TeamSelection>,
    resolved_addr: Res<ResolvedServerAddressForPrefs>,
    client_session_id: Res<ClientSessionId>,
) {
    if gate.suppress_saves > 0 {
        gate.suppress_saves -= 1;
        return;
    }

    let changed = lighting.is_changed()
        || model.is_changed()
        || team.is_changed()
        || resolved_addr.is_changed()
        || client_session_id.is_changed();
    if !changed && !initial_save_pending.0 {
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
        client_session_id.0.as_str(),
    ) {
        warn!("Failed to save client preferences: {e}");
    }
    initial_save_pending.0 = false;
}

/// Resets graphics settings to defaults, persists, and re-opens save gate briefly.
pub fn reset_graphics_to_defaults(
    lighting: &mut LightingSettings,
    model: &mut ModelScaleSettings,
    gate: &mut ClientPrefsSaveGate,
    character: CharacterChoice,
    game_server_addr: &str,
    client_session_id: &str,
) {
    *lighting = LightingSettings::default();
    *model = ModelScaleSettings::default();
    gate.suppress_saves = 1;
    if let Err(e) = save_client_preferences_to_disk(
        lighting,
        model,
        character,
        game_server_addr,
        client_session_id,
    ) {
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
    fn server_entry_accepts_lan_and_ipv6_but_rejects_urls_and_zero_port() {
        assert_eq!(
            validate_game_server_addr(" [::1]:4000 ").as_deref(),
            Some("[::1]:4000")
        );
        assert_eq!(
            validate_game_server_addr(" game.local.:4000 ").as_deref(),
            Some("game.local.:4000")
        );
        for invalid in [
            "https://game.local:4000",
            "game.local/path:4000",
            "127.0.0.1:0",
            "[::1]:0",
            "game..local:4000",
            "-game.local:4000",
            "game.local:65536",
        ] {
            assert!(
                validate_game_server_addr(invalid).is_none(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn validate_client_session_id_accepts_safe_tokens() {
        assert_eq!(
            validate_client_session_id("player_01.alpha-2").as_deref(),
            Some("player_01.alpha-2")
        );
    }

    #[test]
    fn validate_client_session_id_rejects_empty_spaces_and_long_values() {
        assert!(validate_client_session_id("").is_none());
        assert!(validate_client_session_id("bad token").is_none());
        assert!(validate_client_session_id(&"x".repeat(CLIENT_SESSION_ID_MAX_LEN + 1)).is_none());
    }

    #[test]
    fn clamp_model_respects_bounds() {
        assert_eq!(clamp_model_target_height(0.0), MIN_MODEL_TARGET_HEIGHT);
        assert_eq!(clamp_model_target_height(99.0), MAX_MODEL_TARGET_HEIGHT);
    }

    #[test]
    fn migrate_model_target_height_resets_only_legacy_defaults() {
        // Legacy world scale (old default 0.26, old range 0.08..1.2): below
        // the current minimum means "saved before the world rescale".
        assert_eq!(
            migrate_model_target_height(0.26, 1),
            DEFAULT_MODEL_TARGET_HEIGHT
        );
        assert_eq!(
            migrate_model_target_height(0.08, 2),
            DEFAULT_MODEL_TARGET_HEIGHT
        );
        assert_eq!(
            migrate_model_target_height(PREVIOUS_DEFAULT_MODEL_TARGET_HEIGHT, 2),
            DEFAULT_MODEL_TARGET_HEIGHT
        );
        // Old custom values survive, including values near the old default.
        for custom in [0.3, 0.5, 1.149, 1.151, 1.2, 2.4, 3.0] {
            assert_eq!(migrate_model_target_height(custom, 2), custom);
        }
        assert_eq!(
            migrate_model_target_height(99.0, 2),
            MAX_MODEL_TARGET_HEIGHT
        );
        // A user deliberately restoring the old height after this release
        // must not have it upgraded again on every launch.
        assert_eq!(migrate_model_target_height(1.15, SCHEMA_VERSION), 1.15);
        assert_eq!(
            migrate_model_target_height(0.26, SCHEMA_VERSION),
            MIN_MODEL_TARGET_HEIGHT
        );
    }

    #[test]
    fn unversioned_preferences_migrate_and_current_serialization_keeps_custom_size() {
        let legacy: ClientPreferencesFile =
            serde_json::from_str(r#"{"model_target_height":1.15}"#).unwrap();
        assert_eq!(legacy.schema_version, 1);
        assert_eq!(
            migrate_model_target_height(legacy.model_target_height.unwrap(), legacy.schema_version),
            DEFAULT_MODEL_TARGET_HEIGHT
        );

        for target_height in [1.15, DEFAULT_MODEL_TARGET_HEIGHT, 2.4] {
            let saved = build_file_from_state(
                &LightingSettings::default(),
                &ModelScaleSettings { target_height },
                CharacterChoice::Paco,
                DEFAULT_GAME_SERVER_ADDR,
                "scale-migration-test",
            );
            let round_trip: ClientPreferencesFile =
                serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
            assert_eq!(round_trip.schema_version, SCHEMA_VERSION);
            assert_eq!(
                migrate_model_target_height(
                    round_trip.model_target_height.unwrap(),
                    round_trip.schema_version
                ),
                target_height
            );
        }
    }

    #[test]
    fn clamp_lighting_respects_bounds() {
        let s = LightingSettings {
            illuminance: 1.0,
            ambient_brightness: 999_999.0,
            ..Default::default()
        };
        let c = clamp_lighting_settings(s);
        assert_eq!(c.illuminance, MIN_LIGHT_ILLUMINANCE);
        assert_eq!(c.ambient_brightness, MAX_AMBIENT_BRIGHTNESS);
    }
}
