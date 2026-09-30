//! Ekza RenditionWorker adapter: validate a static prop, preserve its exact bytes.
use std::{io::Read, path::PathBuf};
fn run() -> Result<serde_json::Value, String> {
    let mut args = std::env::args().skip(1);
    let (mut source, mut output) = (None, None);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--source" => source = args.next().map(PathBuf::from),
            "--output-dir" => output = args.next().map(PathBuf::from),
            _ => return Err("Use --source FILE --output-dir DIRECTORY".into()),
        }
    }
    let source = source.ok_or("Missing --source")?;
    let output = output.ok_or("Missing --output-dir")?;
    let mut bytes = Vec::new();
    std::fs::File::open(source)
        .map_err(|e| e.to_string())?
        .take(omoba_passport::weapons::MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    omoba_passport::weapons::validate_model(&bytes)?;
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let path = output
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join("handheld.glb");
    // Fresh worker scratch only; never follow or overwrite an existing output.
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?
        .write_all(&bytes)
        .map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"ok":true,"assetPath":path,"sha256":ekza_bevy_sdk::sha256_hex(&bytes),"sizeBytes":bytes.len(),"format":"glb"}),
    )
}
fn main() {
    match run() {
        Ok(report) => println!("{report}"),
        Err(message) => {
            println!(
                "{}",
                serde_json::json!({"ok":false,"issues":[{"code":"handheld_profile","message":message}]})
            );
            std::process::exit(2);
        }
    }
}
