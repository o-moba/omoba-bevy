//! Export game-owned authoring metadata or validate a downloaded class build.
use shared::workshop::{self, ClassBuildDocument};
use std::{io::Read, path::PathBuf};

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut output = None;
    let mut build = None;
    let mut check = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => output = Some(PathBuf::from(args.next().ok_or("--output needs a file")?)),
            "--class-build" => {
                build = Some(PathBuf::from(
                    args.next().ok_or("--class-build needs a JSON file")?,
                ))
            }
            "--check" => check = true,
            _ => return Err(format!("Unknown option: {arg}")),
        }
    }
    let value = if let Some(path) = build {
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        file.take((workshop::MAX_DOCUMENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        let doc = ClassBuildDocument::parse(&bytes).map_err(|error| error.to_string())?;
        serde_json::to_value(doc.sandbox_config().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?
    } else {
        workshop::catalog()
    };
    let text = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())? + "\n";
    if let Some(path) = output {
        if check {
            if std::fs::read_to_string(&path).map_err(|error| error.to_string())? != text {
                return Err(format!("Workshop export is stale: {}", path.display()));
            }
        } else {
            std::fs::write(path, text).map_err(|error| error.to_string())?;
        }
    } else if check {
        return Err("--check requires --output".into());
    } else {
        print!("{text}");
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Class workshop: {error}");
        std::process::exit(1);
    }
}
