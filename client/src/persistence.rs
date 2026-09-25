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

use crate::audio_settings::AudioSettings;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::camera::CameraSettings;
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

const SCHEMA_VERSION: u32 = 5;
const LEGACY_DEFAULT_MODEL_TARGET_HEIGHT: f32 = 1.15;
const SCHEMA_3_DEFAULT_MODEL_TARGET_HEIGHT: f32 = 1.45;
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
            .init_resource::<AudioSettings>()
            .init_resource::<CameraSettings>()
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
    #[serde(default)]
    audio: AudioSettings,
    /// Follow-camera distance multiplier; absent in files written before it existed.
    #[serde(default)]
    camera_zoom: Option<f32>,
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
/// Schema 4 records the Verdant readability default. Only schema 3's exact 1.45
/// default and pre-schema-3 defaults (1.15 or obsolete subminimum values) migrate.
/// A deliberate 1.15 under schema 3, or either old default saved under schema 4,
/// survives subsequent launches.
pub fn migrate_model_target_height(stored: f32, schema_version: u32) -> f32 {
    let legacy_default = schema_version < 3
        && (stored < MIN_MODEL_TARGET_HEIGHT
            || (stored - LEGACY_DEFAULT_MODEL_TARGET_HEIGHT).abs() < f32::EPSILON);
    let previous_default =
        schema_version == 3 && (stored - SCHEMA_3_DEFAULT_MODEL_TARGET_HEIGHT).abs() < f32::EPSILON;
    if legacy_default || previous_default {
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
    write_atomically(path, &data)
}

/// Writes `data` to a sibling temp file, then renames it over `path`. A
/// crash or full disk mid-write leaves the previous file (and its
/// `client_session_id`) intact instead of a truncated one.
fn write_atomically(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(".tmp");
    let temp = path.with_file_name(temp_name);
    let result = (|| {
        use std::io::Write;
        let mut file = fs::File::create(&temp)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Startup: load JSON if present and apply to resources; always sets [`FileGameServerAddr`].
pub(crate) fn load_persistent_client_settings(
    mut file_addr: ResMut<FileGameServerAddr>,
    mut lighting: ResMut<LightingSettings>,
    mut model: ResMut<ModelScaleSettings>,
    mut team: ResMut<crate::team::TeamSelection>,
    mut client_session_id: ResMut<ClientSessionId>,
    mut initial_save_pending: ResMut<ClientPreferencesInitialSavePending>,
    mut gate: ResMut<ClientPrefsSaveGate>,
    mut audio: ResMut<AudioSettings>,
    mut camera: ResMut<CameraSettings>,
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
    *audio = disk.audio.sanitized();
    if let Some(zoom) = disk.camera_zoom {
        *camera = CameraSettings { zoom }.sanitized();
    }

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
    audio: &AudioSettings,
    camera: &CameraSettings,
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
        audio: audio.sanitized(),
        camera_zoom: Some(camera.sanitized().zoom),
    }
}

/// Writes current resources to disk (graphics + character + active server display string).
pub(crate) fn save_client_preferences_to_disk(
    lighting: &LightingSettings,
    model: &ModelScaleSettings,
    character: CharacterChoice,
    game_server_addr: &str,
    client_session_id: &str,
    audio: &AudioSettings,
    camera: &CameraSettings,
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
    let prefs = build_file_from_state(
        lighting,
        model,
        character,
        &addr,
        &session_id,
        audio,
        camera,
    );
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
    audio: Res<AudioSettings>,
    camera: Res<CameraSettings>,
) {
    if gate.suppress_saves > 0 {
        gate.suppress_saves -= 1;
        return;
    }

    let changed = lighting.is_changed()
        || model.is_changed()
        || team.is_changed()
        || resolved_addr.is_changed()
        || client_session_id.is_changed()
        || audio.is_changed()
        || camera.is_changed();
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
        audio.as_ref(),
        camera.as_ref(),
    ) {
        warn!("Failed to save client preferences: {e}");
    }
    initial_save_pending.0 = false;
}

/// Resets graphics settings (lighting, model scale, camera distance) to
/// defaults, persists, and re-opens save gate briefly.
pub(crate) fn reset_graphics_to_defaults(
    lighting: &mut LightingSettings,
    model: &mut ModelScaleSettings,
    camera: &mut CameraSettings,
    gate: &mut ClientPrefsSaveGate,
    character: CharacterChoice,
    game_server_addr: &str,
    client_session_id: &str,
    audio: &AudioSettings,
) {
    *lighting = LightingSettings::default();
    *model = ModelScaleSettings::default();
    *camera = CameraSettings::default();
    gate.suppress_saves = 1;
    if let Err(e) = save_client_preferences_to_disk(
        lighting,
        model,
        character,
        game_server_addr,
        client_session_id,
        audio,
        camera,
    ) {
        warn!("Failed to save preferences after reset: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omoba-prefs-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// O7: the settings file is replaced by rename, never truncated in place.
    #[test]
    fn preferences_are_written_atomically() {
        let dir = scratch_dir("atomic");
        let path = dir.join("client.json");
        write_atomically(&path, b"old").unwrap();
        write_atomically(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("client.json")]);

        // A write that cannot complete (the temp path is taken by a
        // directory) fails without touching the previous file.
        fs::create_dir(dir.join("client.json.tmp")).unwrap();
        assert!(write_atomically(&path, b"lost").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn schema_four_gets_audio_defaults_without_repeating_readability_migration() {
        let old: ClientPreferencesFile = serde_json::from_str(r#"{"schema_version":4,"model_target_height":1.45,"client_session_id":"old-install","game_server_addr":"game.local:4000","illuminance":23000}"#).unwrap();
        assert_eq!(old.audio, AudioSettings::default());
        assert_eq!(
            migrate_model_target_height(old.model_target_height.unwrap(), old.schema_version),
            1.45
        );
        assert_eq!(old.client_session_id.as_deref(), Some("old-install"));
        assert_eq!(old.illuminance, Some(23000.0));
        assert_eq!(old.game_server_addr.as_deref(), Some("game.local:4000"));
    }

    #[test]
    fn camera_distance_round_trips_and_older_files_keep_the_default() {
        let file = build_file_from_state(
            &LightingSettings::default(),
            &ModelScaleSettings::default(),
            CharacterChoice::Paco,
            "game.local:5000",
            "camera-test-install",
            &AudioSettings::default(),
            &CameraSettings { zoom: 0.8 },
        );
        let restored: ClientPreferencesFile =
            serde_json::from_slice(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(restored.camera_zoom, Some(0.8));
        // Out-of-range values are clamped on the way to disk.
        let clamped = build_file_from_state(
            &LightingSettings::default(),
            &ModelScaleSettings::default(),
            CharacterChoice::Paco,
            "game.local:5000",
            "camera-test-install",
            &AudioSettings::default(),
            &CameraSettings { zoom: 9.0 },
        );
        assert_eq!(clamped.camera_zoom, Some(crate::camera::CAMERA_MAX_ZOOM));
        let legacy: ClientPreferencesFile =
            serde_json::from_str(r#"{"schema_version":5,"audio":{}}"#).unwrap();
        assert_eq!(legacy.camera_zoom, None);
    }

    #[test]
    fn saved_audio_round_trip_keeps_independent_buses_mute_and_existing_preferences() {
        let audio = AudioSettings {
            master: 0.55,
            music: 0.1,
            effects: 0.95,
            ui: 0.4,
            muted: true,
        };
        let file = build_file_from_state(
            &LightingSettings::default(),
            &ModelScaleSettings { target_height: 2.4 },
            CharacterChoice::Paco,
            "game.local:5000",
            "audio-test-install",
            &audio,
            &CameraSettings::default(),
        );
        let bytes = serde_json::to_vec(&file).unwrap();
        let restored: ClientPreferencesFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.schema_version, 5);
        assert_eq!(restored.audio, audio);
        assert_eq!(restored.model_target_height, Some(2.4));
        assert_eq!(restored.character, Some(CharacterChoice::Paco));
        assert_eq!(
            restored.game_server_addr.as_deref(),
            Some("game.local:5000")
        );
        assert_eq!(
            restored.client_session_id.as_deref(),
            Some("audio-test-install")
        );
        // The graphics reset writer receives the current audio mix unchanged.
        let reset = build_file_from_state(
            &LightingSettings::default(),
            &ModelScaleSettings::default(),
            CharacterChoice::Paco,
            "game.local:5000",
            "audio-test-install",
            &restored.audio,
            &CameraSettings::default(),
        );
        assert_eq!(reset.audio, audio);
        let invalid = build_file_from_state(
            &LightingSettings::default(),
            &ModelScaleSettings::default(),
            CharacterChoice::Paco,
            "game.local:5000",
            "audio-test-install",
            &AudioSettings {
                master: f32::NAN,
                effects: -5.0,
                ..audio
            },
            &CameraSettings::default(),
        );
        let value = serde_json::to_value(invalid).unwrap();
        assert!(value["audio"]["master"].is_number());
        assert_eq!(value["audio"]["effects"], 0.0);
    }

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
            migrate_model_target_height(LEGACY_DEFAULT_MODEL_TARGET_HEIGHT, 2),
            DEFAULT_MODEL_TARGET_HEIGHT
        );
        // Old custom values survive, including values near the old default.
        for custom in [0.3, 0.5, 1.149, 1.151, 1.2, 1.45, 2.4, 3.0] {
            assert_eq!(migrate_model_target_height(custom, 2), custom);
        }
        assert_eq!(
            migrate_model_target_height(99.0, 2),
            MAX_MODEL_TARGET_HEIGHT
        );
        // A user deliberately restoring the old height after this release
        // must not have it upgraded again on every launch.
        for custom in [1.15, 1.45] {
            assert_eq!(migrate_model_target_height(custom, SCHEMA_VERSION), custom);
        }
        assert_eq!(
            migrate_model_target_height(0.26, SCHEMA_VERSION),
            MIN_MODEL_TARGET_HEIGHT
        );
    }

    #[test]
    fn schema_three_migrates_only_its_default_and_preserves_custom_sizes() {
        assert_eq!(
            migrate_model_target_height(SCHEMA_3_DEFAULT_MODEL_TARGET_HEIGHT, 3),
            DEFAULT_MODEL_TARGET_HEIGHT
        );
        for custom in [0.3, 0.5, 1.15, 1.449, 1.451, 1.8, 2.4, 3.0] {
            assert_eq!(migrate_model_target_height(custom, 3), custom);
        }
        // Older and future schemas must not mistake a custom 1.45 for schema 3's default.
        for schema in [1, 2, SCHEMA_VERSION, SCHEMA_VERSION + 1] {
            assert_eq!(migrate_model_target_height(1.45, schema), 1.45);
        }
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

        let previous: ClientPreferencesFile =
            serde_json::from_str(r#"{"schema_version":3,"model_target_height":1.45}"#).unwrap();
        assert_eq!(
            migrate_model_target_height(
                previous.model_target_height.unwrap(),
                previous.schema_version
            ),
            DEFAULT_MODEL_TARGET_HEIGHT
        );

        for target_height in [1.15, 1.45, DEFAULT_MODEL_TARGET_HEIGHT, 2.4] {
            let saved = build_file_from_state(
                &LightingSettings::default(),
                &ModelScaleSettings { target_height },
                CharacterChoice::Paco,
                DEFAULT_GAME_SERVER_ADDR,
                "scale-migration-test",
                &AudioSettings::default(),
                &CameraSettings::default(),
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
