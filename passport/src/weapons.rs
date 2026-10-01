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
        if d.get(key)
            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return Err("Handheld v1 requires a static prop".into());
        }
    }
    if d["scenes"].as_array().is_none_or(|a| a.len() != 1)
        || d.get("scene").is_some_and(|v| v.as_u64() != Some(0))
        || d["nodes"]
            .as_array()
            .is_none_or(|a| a.is_empty() || a.len() > 64)
        || d["meshes"]
            .as_array()
            .is_none_or(|a| a.is_empty() || a.len() > 16)
    {
        return Err("Handheld scene exceeds profile limits".into());
    }
    for key in ["scenes", "nodes", "meshes"] {
        if d[key].as_array().unwrap().iter().any(|v| !v.is_object()) {
            return Err("Handheld scenes, nodes and meshes must contain objects".into());
        }
    }
    validate_scene_graph(&d)?;
    for node in d["nodes"].as_array().unwrap() {
        if node.get("matrix").is_some()
            && ["translation", "rotation", "scale"]
                .iter()
                .any(|key| node.get(key).is_some())
        {
            return Err("Handheld node must use either matrix or TRS".into());
        }
        for (key, count) in [
            ("matrix", 16),
            ("translation", 3),
            ("rotation", 4),
            ("scale", 3),
        ] {
            if node.get(key).is_some_and(|v| {
                v.as_array().is_none_or(|a| {
                    a.len() != count
                        || a.iter()
                            .any(|v| v.as_f64().is_none_or(|n| !(n as f32).is_finite()))
                })
            }) {
                return Err("Handheld node transform must contain finite float values".into());
            }
        }
    }
    if d.get("extensionsRequired").is_some_and(|v| {
        v.as_array().is_none_or(|a| {
            a.iter().any(|v| {
                !matches!(
                    v.as_str(),
                    Some("KHR_materials_unlit" | "KHR_texture_transform")
                )
            })
        })
    }) {
        return Err("Unsupported weapon extension".into());
    }
    if d["asset"]["version"].as_str() != Some("2.0") {
        return Err("Handheld must declare glTF asset version 2.0".into());
    }
    let grip: Grip = serde_json::from_value(d["asset"]["extras"]["ekza_handheld_v1"].clone())
        .map_err(|_| "Missing ekza_handheld_v1 grip metadata")?;
    grip.validate()?;
    Ok(grip)
}
/// The profile caps the graph at 64 nodes before this check. Validate the
/// entire forest, including unused nodes, before Bevy traverses any scene.
fn validate_scene_graph(document: &serde_json::Value) -> Result<(), String> {
    let nodes = document["nodes"].as_array().unwrap();
    let meshes = document["meshes"].as_array().unwrap();
    let index = |value: &serde_json::Value, bound: usize| -> Result<usize, String> {
        value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n < bound)
            .ok_or_else(|| "Handheld node or mesh reference must be an in-bounds integer".into())
    };
    let roots = document["scenes"][0]["nodes"]
        .as_array()
        .filter(|roots| !roots.is_empty())
        .ok_or("Handheld Scene0 needs nonempty root nodes")?;
    let roots: Vec<usize> = roots
        .iter()
        .map(|v| index(v, nodes.len()))
        .collect::<Result<_, _>>()?;
    if roots.iter().copied().collect::<HashSet<_>>().len() != roots.len() {
        return Err("Handheld scene roots must be unique".into());
    }
    let mut children = vec![Vec::new(); nodes.len()];
    let mut parents = vec![None; nodes.len()];
    for (parent, node) in nodes.iter().enumerate() {
        if let Some(mesh) = node.get("mesh") {
            index(mesh, meshes.len())?;
        }
        if let Some(values) = node.get("children") {
            let values = values
                .as_array()
                .ok_or("Handheld node children must be an array")?;
            for value in values {
                let child = index(value, nodes.len())?;
                if parents[child].replace(parent).is_some() {
                    return Err(
                        "Handheld nodes must have one parent and unique child references".into(),
                    );
                }
                children[parent].push(child);
            }
        }
    }
    if roots.iter().any(|&root| parents[root].is_some()) {
        return Err("Handheld scene roots cannot also be children".into());
    }
    fn visit(node: usize, children: &[Vec<usize>], state: &mut [u8]) -> Result<(), String> {
        match state[node] {
            1 => return Err("Handheld node graph contains a cycle".into()),
            2 => return Ok(()),
            _ => {}
        }
        state[node] = 1;
        for &child in &children[node] {
            visit(child, children, state)?;
        }
        state[node] = 2;
        Ok(())
    }
    let mut state = vec![0; nodes.len()];
    for node in 0..nodes.len() {
        visit(node, &children, &mut state)?;
    }
    let mut pending = roots;
    while let Some(node) = pending.pop() {
        if nodes[node].get("mesh").is_some() {
            return Ok(());
        }
        pending.extend(&children[node]);
    }
    Err("Handheld Scene0 must contain a reachable mesh".into())
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
    /// Preserve the real buffer payload while mutating untrusted JSON fields:
    /// these cases are valid GLB containers but unsafe runtime scene inputs.
    fn with_document(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let source = include_bytes!("../../client/assets/weapons/dawn-scepter.glb");
        let len = u32::from_le_bytes(source[12..16].try_into().unwrap()) as usize;
        let mut document = serde_json::from_slice(&source[20..20 + len]).unwrap();
        edit(&mut document);
        let mut json = serde_json::to_vec(&document).unwrap();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let remainder = &source[20 + len..];
        let total = 20 + json.len() + remainder.len();
        let mut bytes = source[..12].to_vec();
        bytes[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"JSON");
        bytes.extend(json);
        bytes.extend_from_slice(remainder);
        bytes
    }
    #[test]
    fn malformed_scene_objects_and_transforms_fail_before_loading() {
        use serde_json::json;
        assert!(validate_model(&with_document(|_| {})).is_ok());
        for (field, value) in [
            ("scenes", json!([null])),
            ("nodes", json!([null])),
            ("meshes", json!([null])),
            ("scene", json!(false)),
            ("scene", json!(null)),
            ("skins", json!({})),
            ("animations", json!(false)),
            ("extensionsRequired", json!("KHR_materials_unlit")),
            ("nodes", json!([{"translation":[0,0]}])),
            ("nodes", json!([{"rotation":[0,0,true,1]}])),
            ("nodes", json!([{"scale":[1,1,1e308]}])),
            (
                "nodes",
                json!([{"matrix":vec![0; 16], "translation":[0,0,0]}]),
            ),
        ] {
            let bytes = with_document(|d| d[field] = value.clone());
            assert!(validate_model(&bytes).is_err(), "accepted {field}: {value}");
        }
        assert!(validate_model(&with_document(|d| d["asset"]["version"] = json!("1.0"))).is_err());
    }
    #[test]
    fn malformed_scene_references_and_cycles_fail_before_loading() {
        use serde_json::json;
        for (path, value) in [
            ("/scenes/0/nodes", json!([11])),
            ("/scenes/0/nodes", json!([true])),
            ("/scenes/0/nodes", json!([0.0])),
            ("/scenes/0/nodes", json!([-1])),
            ("/scenes/0/nodes", json!([0, 0])),
            ("/scenes/0/nodes", json!([])),
            ("/scenes/0/nodes", json!(null)),
            ("/nodes", json!([{"mesh":999}])),
            ("/nodes", json!([{"mesh":true}])),
            ("/nodes", json!([{"mesh":null}])),
            ("/nodes", json!([{"mesh":0,"children":null}])),
            ("/nodes", json!([{"mesh":0,"children":[99]}])),
            ("/nodes", json!([{"mesh":0,"children":[true]}])),
            ("/nodes", json!([{"mesh":0,"children":[0]}])),
            ("/nodes", json!([{"children":[1,1]},{"mesh":0}])),
            (
                "/nodes",
                json!([{"children":[1,2]},{"children":[2]},{"mesh":0}]),
            ),
            ("/nodes", json!([{"mesh":0},{"children":[0]}])),
            (
                "/nodes",
                json!([{"mesh":0},{"children":[2]},{"children":[1]}]),
            ),
            ("/nodes", json!([{}, {"mesh":0}])),
        ] {
            let bytes = with_document(|d| {
                d["scenes"][0]["nodes"] = json!([0]);
                d["nodes"] = json!([{"mesh":0}]);
                *d.pointer_mut(path).unwrap() = value.clone();
            });
            assert!(validate_model(&bytes).is_err(), "accepted {path}: {value}");
        }
        let valid = with_document(|d| {
            d["scenes"][0]["nodes"] = json!([0]);
            d["nodes"] = json!([{"children":[1]}, {"mesh":0,"children":[]}, {"mesh":0}]);
        });
        assert!(
            validate_model(&valid).is_ok(),
            "nested mesh and unused valid nodes are allowed"
        );
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
