//! Offline D1 signer/verifier. No networking, migration or key generation.
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use shared::game_directory::{Directory, MAX_WIRE_BYTES, SignedDirectory};
use shared::public_transport::decode_hex;
use shared::region::RegionId;
use std::{fs::File, io::Read, path::Path};

fn read(path: &str, limit: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|_| "Cannot open input file")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect input file")?
        .is_file()
    {
        return Err("Input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read input file")?;
    if bytes.len() > limit {
        return Err("Input file exceeds directory limit".into());
    }
    Ok(bytes)
}
fn verify(
    bytes: &[u8],
    public: &[u8; 32],
    key_id: &str,
    region: RegionId,
    contour: &str,
    now: u64,
    minimum: u64,
) -> Result<Directory, String> {
    let envelope = SignedDirectory::parse(bytes)?;
    if envelope.key_id != key_id {
        return Err("Untrusted directory key identity".into());
    }
    let key = VerifyingKey::from_bytes(public).map_err(|_| "Invalid pinned public key")?;
    let signature = decode_hex::<64>(&envelope.signature).ok_or("Invalid signature")?;
    key.verify_strict(
        &envelope.signing_bytes()?,
        &Signature::from_bytes(&signature),
    )
    .map_err(|_| "Directory signature rejected")?;
    let doc = Directory::parse(&envelope.payload)?;
    doc.validate_at(region, contour, now, minimum)?;
    Ok(doc)
}
fn run(args: &[String]) -> Result<String, String> {
    // Fixed positional arguments keep private key material out of argv.
    // Sign: sign INPUT KEY_FILE KEY_ID REGION CONTOUR NOW_MS
    // Verify: verify INPUT PUBLIC_KEY KEY_ID REGION CONTOUR NOW_MS MIN_SEQUENCE
    let sign = args.first().is_some_and(|v| v == "sign");
    if !((sign && args.len() == 7)
        || (!sign && args.first().is_some_and(|v| v == "verify") && args.len() == 8))
    {
        return Err("Use sign INPUT KEY_FILE KEY_ID REGION CONTOUR NOW_MS, or verify INPUT PUBLIC_KEY KEY_ID REGION CONTOUR NOW_MS MIN_SEQUENCE".into());
    }
    let region = match args[4].as_str() {
        "ru" => RegionId::Ru,
        "global" => RegionId::Global,
        _ => return Err("Region must be ru or global".into()),
    };
    let now = args[6]
        .parse::<u64>()
        .map_err(|_| "Invalid UTC milliseconds")?;
    let input = read(&args[1], MAX_WIRE_BYTES)?;
    if sign {
        let payload = std::str::from_utf8(&input).map_err(|_| "Directory must be UTF-8")?;
        let doc = Directory::parse(payload)?;
        doc.validate_at(region, &args[5], now, 0)?;
        let metadata = std::fs::symlink_metadata(Path::new(&args[2]))
            .map_err(|_| "Cannot inspect signing key file")?;
        if !metadata.is_file() {
            return Err("Signing key must be a regular file".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err("Signing key file must not be accessible to group or others".into());
            }
        }
        let mut raw = read(&args[2], 65)?;
        let mut seed = std::str::from_utf8(&raw)
            .ok()
            .and_then(|s| decode_hex::<32>(s.trim()))
            .ok_or("Signing key file must contain one 32-byte hexadecimal seed")?;
        let key = SigningKey::from_bytes(&seed);
        seed.fill(0);
        raw.fill(0);
        let mut envelope = SignedDirectory {
            key_id: args[3].clone(),
            payload: serde_json::to_string(&doc).map_err(|_| "Cannot encode directory")?,
            signature: String::new(),
        };
        envelope.signature = key
            .sign(&envelope.signing_bytes()?)
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let encoded =
            serde_json::to_vec(&envelope).map_err(|_| "Cannot encode signed directory")?;
        verify(
            &encoded,
            &key.verifying_key().to_bytes(),
            &args[3],
            region,
            &args[5],
            now,
            0,
        )?;
        String::from_utf8(encoded).map_err(|_| "Cannot encode signed directory".into())
    } else {
        let public = decode_hex::<32>(&args[2]).ok_or("Invalid pinned public key")?;
        let minimum = args[7]
            .parse::<u64>()
            .map_err(|_| "Invalid minimum sequence")?;
        let doc = verify(&input, &public, &args[3], region, &args[5], now, minimum)?;
        serde_json::to_string_pretty(&doc).map_err(|_| "Cannot encode verified directory".into())
    }
}
fn main() {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signed() -> (SigningKey, Vec<u8>) {
        // Publicly known test identity only; never a production trust root.
        let key = SigningKey::from_bytes(&[7; 32]);
        let doc = serde_json::json!({"version":1,"region":"ru","contour":"production","sequence":4,"issued_at_ms":100000,"expires_at_ms":200000,
            "policy":{"min_release":"0.45.0","maintenance":true,"update_url":null,"message":null},"lobbies":[]});
        let mut e = SignedDirectory {
            key_id: "test-only".into(),
            payload: doc.to_string(),
            signature: String::new(),
        };
        e.signature = key
            .sign(&e.signing_bytes().unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        (key, serde_json::to_vec(&e).unwrap())
    }
    #[test]
    fn pinned_signatures_exact_body_scope_and_rollbacks_are_enforced() {
        let (key, bytes) = signed();
        let public = key.verifying_key().to_bytes();
        assert!(
            verify(
                &bytes,
                &public,
                "test-only",
                RegionId::Ru,
                "production",
                150000,
                4
            )
            .is_ok()
        );
        for (region, contour, now, sequence) in [
            (RegionId::Global, "production", 150000, 4),
            (RegionId::Ru, "dev", 150000, 4),
            (RegionId::Ru, "production", 200000, 4),
            (RegionId::Ru, "production", 150000, 5),
        ] {
            assert!(verify(&bytes, &public, "test-only", region, contour, now, sequence).is_err());
        }
        assert!(
            verify(
                &bytes,
                &SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes(),
                "test-only",
                RegionId::Ru,
                "production",
                150000,
                4
            )
            .is_err()
        );
        assert!(
            verify(
                &bytes,
                &public,
                "another-key",
                RegionId::Ru,
                "production",
                150000,
                4
            )
            .is_err()
        );
        let mut altered = SignedDirectory::parse(&bytes).unwrap();
        altered.payload.push(' ');
        assert!(
            verify(
                &serde_json::to_vec(&altered).unwrap(),
                &public,
                "test-only",
                RegionId::Ru,
                "production",
                150000,
                4
            )
            .is_err()
        );
        let mut alias = SignedDirectory::parse(&bytes).unwrap();
        alias.key_id = "alias".into();
        assert!(
            verify(
                &serde_json::to_vec(&alias).unwrap(),
                &public,
                "alias",
                RegionId::Ru,
                "production",
                150000,
                4
            )
            .is_err()
        );
        let mut bad = SignedDirectory::parse(&bytes).unwrap();
        bad.signature = "0".repeat(128);
        assert!(
            verify(
                &serde_json::to_vec(&bad).unwrap(),
                &public,
                "test-only",
                RegionId::Ru,
                "production",
                150000,
                4
            )
            .is_err()
        );
        let mut wrong_domain = SignedDirectory::parse(&bytes).unwrap();
        wrong_domain.signature = key
            .sign(wrong_domain.payload.as_bytes())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(
            verify(
                &serde_json::to_vec(&wrong_domain).unwrap(),
                &public,
                "test-only",
                RegionId::Ru,
                "production",
                150000,
                4
            )
            .is_err()
        );
    }
    #[test]
    fn offline_cli_round_trip_keeps_key_material_out_of_output() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!(
            "omoba-directory-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let (key, bytes) = signed();
        let envelope = SignedDirectory::parse(&bytes).unwrap();
        let input = root.join("directory.json");
        let key_path = root.join("fixture-only.key");
        let output = root.join("signed.json");
        std::fs::write(&input, &envelope.payload).unwrap();
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let raw_seed = "07".repeat(32);
        options
            .open(&key_path)
            .unwrap()
            .write_all(raw_seed.as_bytes())
            .unwrap();
        let sign_args = vec![
            "sign".into(),
            input.to_str().unwrap().into(),
            key_path.to_str().unwrap().into(),
            "test-only".into(),
            "ru".into(),
            "production".into(),
            "150000".into(),
        ];
        let encoded = run(&sign_args).unwrap();
        assert!(!encoded.contains(&raw_seed));
        std::fs::write(&output, &encoded).unwrap();
        let public: String = key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let verified = run(&[
            "verify".into(),
            output.to_str().unwrap().into(),
            public,
            "test-only".into(),
            "ru".into(),
            "production".into(),
            "150000".into(),
            "4".into(),
        ])
        .unwrap();
        assert_eq!(
            Directory::parse(&verified).unwrap(),
            Directory::parse(&envelope.payload).unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).unwrap();
            let error = run(&sign_args).unwrap_err();
            assert!(error.contains("group or others"));
            assert!(!error.contains(&raw_seed));
        }
        std::fs::remove_dir_all(root).unwrap(); // Only this test's three tiny fixture files.
    }
}
