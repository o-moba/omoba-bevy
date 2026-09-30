//! Shipped and Ekza SDK imported handhelds share one bounded attachment contract.
use ekza_bevy_sdk::{cache::AssetCache, catalog::CatalogV2Avatar, registry::RegistryClient};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, sync::OnceLock};
pub const PROFILE: &str = "handheld-glb-v1";
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grip {
    /// Semantic VRM hand, independent of imported bone names.
    pub bone: String,
    /// Metres in canonical palm coordinates; origin is the palm grip centre.
    pub offset: [f32; 3],
    pub rotation_degrees: [f32; 3],
    pub scale: f32,
}
impl Grip {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.bone.as_str(), "rightHand" | "leftHand")
            || self.offset.iter().any(|v| !v.is_finite() || v.abs() > 0.3)
            || self
                .rotation_degrees
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 360.)
            || !self.scale.is_finite()
            || !(0.25..=2.).contains(&self.scale)
        {
            return Err("Invalid handheld grip".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    pub id: String,
    pub name: String,
    pub model: String,
    pub grip: Grip,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub items: Vec<Weapon>,
}
impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 256 * 1024 {
            return Err("Handheld catalog too large".into());
        }
        let c: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if c.schema_version != 1 || c.items.len() > 128 {
            return Err("Unsupported handheld catalog".into());
        }
        let mut ids = HashSet::new();
        for w in &c.items {
            w.grip.validate()?;
            if w.id.is_empty()
                || w.id.len() > 80
                || !w
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || !ids.insert(&w.id)
                || w.name.is_empty()
                || w.name.len() > 100
                || !w.model.starts_with("weapons/")
                || !w.model.ends_with(".glb")
                || w.model.contains("..")
                || w.model.len() > 180
                || !w
                    .model
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
            {
                return Err("Invalid handheld identity/path".into());
            }
            if let Some(hash) = &w.sha256 {
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Invalid weapon SHA-256".into());
                }
            }
        }
        Ok(c)
    }
}
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut base = Catalog::parse(include_bytes!("../../client/assets/weapons/manifest.json"))
            .expect("shipped handheld catalog");
        if let Some(path) = std::env::var_os("OMOBA_WEAPON_MANIFEST") {
            let loaded = std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| Catalog::parse(&bytes));
            match loaded {
                Ok(extra) => {
                    let root = crate::assets::client_asset_root();
                    for item in extra.items {
                        if base.items.iter().any(|w| w.id == item.id) {
                            continue;
                        }
                        let verified = (|| {
                            let bytes =
                                std::fs::read(root.join(&item.model)).map_err(|e| e.to_string())?;
                            if item.sha256.as_deref() != Some(&ekza_bevy_sdk::sha256_hex(&bytes)) {
                                return Err("Weapon hash mismatch".into());
                            }
                            if validate_model(&bytes)? != item.grip {
                                return Err("Grip differs from verified rendition".into());
                            }
                            Ok::<(), String>(())
                        })();
                        if verified.is_ok() {
                            base.items.push(item);
                        } else {
                            eprintln!("Handheld {} unavailable: {:?}", item.id, verified.err());
                        }
                    }
                }
                Err(e) => eprintln!("Handheld imports unavailable: {e}"),
            }
        }
        base
    })
}
/// The metadata lives inside the hashed GLB, so the grip travels with the asset.
pub fn validate_model(bytes: &[u8]) -> Result<Grip, String> {
    if !(20..=MAX_BYTES as usize).contains(&bytes.len()) {
        return Err("Handheld GLB exceeds bounds".into());
    }
    let report =
        ekza_bevy_sdk::validate_glb_bytes(bytes, &ekza_bevy_sdk::GlbValidationRules::default());
    if !report.is_valid() {
        return Err("Invalid handheld GLB container".into());
    }
    let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    if bytes.get(16..20) != Some(b"JSON") || len > bytes.len() - 20 {
        return Err("Invalid GLB JSON".into());
    }
    let d: serde_json::Value =
        serde_json::from_slice(&bytes[20..20 + len]).map_err(|e| e.to_string())?;
    for key in ["buffers", "images"] {
        if d[key]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.get("uri").is_some()))
        {
            return Err("Weapon resources must be embedded".into());
        }
    }
    for key in ["skins", "animations"] {
        if d[key].as_array().is_some_and(|a| !a.is_empty()) {
            return Err("Handheld v1 requires a static prop".into());
        }
    }
    if d["scenes"].as_array().is_none_or(|a| a.len() != 1)
        || d.get("scene").and_then(|v| v.as_u64()).unwrap_or(0) != 0
        || d["nodes"]
            .as_array()
            .is_none_or(|a| a.is_empty() || a.len() > 64)
        || d["meshes"]
            .as_array()
            .is_none_or(|a| a.is_empty() || a.len() > 16)
    {
        return Err("Handheld scene exceeds profile limits".into());
    }
    if d["extensionsRequired"].as_array().is_some_and(|a| {
        a.iter().any(|v| {
            !matches!(
                v.as_str(),
                Some("KHR_materials_unlit" | "KHR_texture_transform")
            )
        })
    }) {
        return Err("Unsupported weapon extension".into());
    }
    let grip: Grip = serde_json::from_value(d["asset"]["extras"]["ekza_handheld_v1"].clone())
        .map_err(|_| "Missing ekza_handheld_v1 grip metadata")?;
    grip.validate()?;
    Ok(grip)
}
/// Existing SDK transport, exact profile/approval, hash and size checks. Paid
/// entries stay excluded until an ownership-ticket flow is implemented for props.
pub fn import_registry(registry: &str, assets: &Path, manifest: &Path) -> Result<usize, String> {
    let client = RegistryClient::new(registry).map_err(|e| e.to_string())?;
    let entries = client
        .catalog_v2(Some("omoba"), Some(("desktop", PROFILE)))
        .map_err(|e| e.to_string())?;
    import_entries(&entries, assets, manifest)
}
pub fn import_entries(
    entries: &[CatalogV2Avatar],
    assets: &Path,
    manifest: &Path,
) -> Result<usize, String> {
    let cache = AssetCache::new(assets.join("weapons/imported"))
        .map_err(|e| e.to_string())?
        .with_limits(MAX_BYTES, 1024 * 1024);
    let mut items = Vec::new();
    for item in entries {
        if !item.is_free()
            || !item.project_support.iter().any(|s| {
                s.project_id == "omoba"
                    && s.platform == "desktop"
                    && s.profile == PROFILE
                    && s.status == "approved"
            })
        {
            continue;
        }
        let a = item.clone().into_avatar();
        let r = a
            .rendition("desktop", PROFILE)
            .ok_or("Approved weapon rendition missing")?;
        if r.format != "glb"
            || r.sha256
                .as_ref()
                .is_none_or(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
            || r.size_bytes.is_none_or(|n| !(20..=MAX_BYTES).contains(&n))
        {
            return Err("Weapon rendition must pin GLB, SHA-256 and bounded size".into());
        }
        let file = cache.fetch_model(r).map_err(|e| e.to_string())?;
        let grip = validate_model(&std::fs::read(&file.path).map_err(|e| e.to_string())?)?;
        let identity = ekza_bevy_sdk::sha256_hex(format!("{}:{}", a.id, file.sha256).as_bytes());
        items.push(Weapon {
            id: format!("ekza-{}", &identity[..32]),
            name: a.name,
            model: format!("weapons/imported/{}.glb", file.sha256),
            grip,
            source_id: Some(a.id),
            sha256: Some(file.sha256),
        });
    }
    let c = Catalog {
        schema_version: 1,
        items,
    };
    let bytes = serde_json::to_vec_pretty(&c).map_err(|e| e.to_string())?;
    Catalog::parse(&bytes)?;
    super::atomic_write(manifest, &bytes)?;
    Ok(c.items.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn shipped_props_and_catalog_follow_the_same_import_contract() {
        let c =
            Catalog::parse(include_bytes!("../../client/assets/weapons/manifest.json")).unwrap();
        for w in &c.items {
            let bytes = std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../client/assets")
                    .join(&w.model),
            )
            .unwrap();
            assert_eq!(validate_model(&bytes).unwrap(), w.grip);
        }
        let mut bad = c.clone();
        bad.items[0].model = "weapons/../../escape.glb".into();
        assert!(Catalog::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        bad = c.clone();
        bad.items[0].grip.bone = "hips".into();
        assert!(Catalog::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        bad = c.clone();
        bad.items.push(c.items[0].clone());
        assert!(Catalog::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    #[test]
    fn sdk_catalog_download_import_and_hash_rejection_use_real_http() {
        let bytes = include_bytes!("../../client/assets/weapons/dawn-scepter.glb").to_vec();
        let hash = ekza_bevy_sdk::sha256_hex(&bytes);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let entry:CatalogV2Avatar=serde_json::from_value(serde_json::json!({
            "id":"ekza:asset:handheld-test", "name":"SDK Scepter", "access":"free",
            "projectSupport":[{"projectId":"omoba","platform":"desktop","profile":PROFILE,"status":"approved"}],
            "renditions":[{"platform":"desktop","profile":PROFILE,"format":"glb","sha256":hash,"sizeBytes":bytes.len(),"downloadUrl":format!("{base}/scepter.glb")}]
        })).unwrap();
        let mut owned = entry.clone();
        owned.id = "owned-item".into();
        owned.access = "owned".into();
        let mut unapproved = entry.clone();
        unapproved.id = "unapproved".into();
        unapproved.project_support[0].status = "pending".into();
        let body=serde_json::to_vec(&serde_json::json!({"schema":"ekza.avatar.catalog.v2","count":3,"items":[entry,owned,unapproved]})).unwrap();
        let server = std::thread::spawn(move || {
            for body in [body, bytes] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 8192];
                let n = stream.read(&mut request).unwrap();
                assert!(n > 0);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let root = std::env::temp_dir().join(format!(
            "omoba-handheld-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manifest = root.join("weapons/ekza-manifest.json");
        assert_eq!(import_registry(&base, &root, &manifest).unwrap(), 1);
        server.join().unwrap();
        let imported = Catalog::parse(&std::fs::read(&manifest).unwrap()).unwrap();
        assert_eq!(
            imported.items[0].source_id.as_deref(),
            Some("ekza:asset:handheld-test")
        );
        assert_eq!(imported.items[0].sha256.as_deref(), Some(hash.as_str()));
        let model = root.join(&imported.items[0].model);
        assert_eq!(
            validate_model(&std::fs::read(&model).unwrap()).unwrap(),
            imported.items[0].grip
        );
        let r = entry.into_avatar().renditions.remove(0);
        assert!(AssetCache::verify_model(&r, b"tampered").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
