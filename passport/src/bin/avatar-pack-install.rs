//! Install a developer catalog through the same Ekza SDK store as the client.
use omoba_passport::{community, humanoid::validate_runtime_humanoid, store};
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or("Usage: avatar-pack-install STORE_ROOT REPORT_JSON")?,
    );
    let report = PathBuf::from(args.next().ok_or("Missing report path")?);
    if args.next().is_some() {
        return Err("Unexpected argument".into());
    }
    let registry = community::registry_url();
    let approved = community::fetch_free(&registry)?;
    if approved.is_empty() {
        return Err("Registry contains no approved free avatars".into());
    }
    store::initialize(root.clone(), true);
    let mut items = Vec::new();
    for item in approved {
        store::install_blocking(&item.slug)?;
        let path = root.join("avatars").join(format!("{}.glb", item.slug));
        omoba_passport::verify_local(&item.protected, &path)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let rig = validate_runtime_humanoid(&bytes)?;
        if !["rightHand", "leftHand"]
            .iter()
            .all(|bone| rig.bones.contains_key(*bone))
        {
            return Err(format!("{} has no semantic hands", item.name));
        }
        items.push(serde_json::json!({"name":item.name,"slug":item.slug,
            "avatar_id":item.protected.avatar_id,"sha256":ekza_bevy_sdk::sha256_hex(&bytes),
            "size_bytes":bytes.len(),"semantic_bones":rig.bones.len(),"source":"ekza-sdk-store"}));
        eprintln!("SDK installed and verified: {}", item.name);
    }
    let data = serde_json::to_vec_pretty(&serde_json::json!({"registry":registry,"items":items}))
        .map_err(|e| e.to_string())?;
    std::fs::write(report, data).map_err(|e| e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("SDK avatar import failed: {error}");
        std::process::exit(1);
    }
}
