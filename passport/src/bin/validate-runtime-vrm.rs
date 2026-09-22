//! Offline inspection only: does not import, approve, or alter an avatar.
use omoba_passport::humanoid::{MAX_HUMANOID_GLB_BYTES, VrmVersion, validate_runtime_humanoid};
use std::{fs::File, io::Read, path::PathBuf};

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let file = arguments
        .next()
        .ok_or("Usage: validate-runtime-vrm <model.vrm|model.glb>")?;
    if file == "--help" || file == "-h" {
        println!("Usage: validate-runtime-vrm <model.vrm|model.glb>");
        println!("Checks local VRM humanoid animation compatibility without modifying the file.");
        println!("Does not grant Studio approval or change humanoid-glb-v1 admission.");
        return Ok(());
    }
    if arguments.next().is_some() {
        return Err("Usage: validate-runtime-vrm <model.vrm|model.glb>".into());
    }
    let path = PathBuf::from(file);
    let file = File::open(&path).map_err(|error| format!("Cannot open model: {error}"))?;
    let size = file
        .metadata()
        .map_err(|error| format!("Cannot inspect model: {error}"))?
        .len();
    if size > MAX_HUMANOID_GLB_BYTES as u64 {
        return Err("VRM model exceeds the 50 MiB runtime limit".into());
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_HUMANOID_GLB_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read model: {error}"))?;
    let rig = validate_runtime_humanoid(&bytes)?;
    let version = match rig.version {
        VrmVersion::Vrm0 => "VRM 0.x",
        VrmVersion::Vrm1 => "VRM 1.0",
    };
    println!(
        "Compatible local humanoid: {version}, {} nodes, {} semantic bones, {} scene-0 roots.",
        rig.nodes.len(),
        rig.bones.len(),
        rig.scene_roots.len(),
    );
    println!(
        "File unchanged: {} ({} bytes).",
        path.display(),
        bytes.len()
    );
    println!(
        "Embedded clips are optional for this local capability; Studio admission is unchanged."
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("VRM runtime validation failed: {error}");
        std::process::exit(1);
    }
}
