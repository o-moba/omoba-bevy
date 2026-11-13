use super::*;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

fn pin(id: &str) -> Value {
    let seed = if id == "overlap" { 8 } else { 7 };
    let key = SigningKey::from_bytes(&[seed; 32])
        .verifying_key()
        .to_bytes();
    json!({"key_id":id,"public_key":key.iter().map(|n|format!("{n:02x}")).collect::<String>()})
}
fn spec() -> Value {
    json!({"version":1,"contour":"production","regions":[{"region":"ru","pins":[pin("current"),pin("overlap")],
        "fallback":{"id":"ru-one","location":"moscow","host":r"lobby.omoba.ru","addresses":["212.113.117.105"],"port":4000}}]})
}
fn parse(value: &Value) -> Result<BuildTrust, DirectoryError> {
    BuildTrust::parse(Some(&value.to_string()), "production").map(Option::unwrap)
}
#[test]
fn reviewed_build_input_has_scoped_cache_and_pinned_fallback_without_io() {
    let bundle = parse(&spec()).unwrap();
    let ru = bundle.region(RegionId::Ru).unwrap();
    assert_eq!(ru.region(), RegionId::Ru);
    assert_eq!(ru.contour(), "production");
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).unwrap();
    let suffix = nonce.iter().map(|n| format!("{n:02x}")).collect::<String>();
    let root = std::env::temp_dir().join(format!("omoba-build-trust-{suffix}"));
    assert!(!root.exists());
    assert_eq!(
        ru.cache_path(&root),
        root.join("game-directory/production/ru/last-good.json")
    );
    let admission = ru.admission(&root).unwrap();
    // Construction and audit create no cache state. Filesystem refresh has
    // its own independently owned fixture in the admission tests.
    assert_eq!(
        admission.decision_at(1, &ReleaseContract::current(), 0, None),
        super::super::admission::Decision::Pending
    );
    let audit = bundle.audit();
    assert!(
        !root.exists(),
        "construction/audit must not create cache state"
    );
    assert_eq!(
        audit["regions"][0]["concrete_endpoints"][0],
        "212.113.117.105:4000"
    );
    assert_eq!(audit["configured"], true);
    assert_eq!(audit["regions"][0]["pins"][0], pin("current"));
    assert_eq!(audit["regions"][0]["pins"][1], pin("overlap"));
    assert!(!audit.to_string().contains("\"seed\""));
    assert!(
        bundle.region(RegionId::Global).is_err(),
        "partial configuration must not use legacy/global authority"
    );
}
#[test]
fn contour_and_region_boundaries_are_not_inferred_from_input_destinations() {
    let input = spec();
    assert!(BuildTrust::parse(Some(&input.to_string()), "dev").is_err());
    let mut value = spec();
    value["regions"][0]["fallback"]["host"] = json!(r"lobby.omoba.io");
    assert!(parse(&value).is_err());
    let mut value = spec();
    value["regions"][0]["fallback"]["addresses"] = json!(["77.246.105.57"]);
    assert!(parse(&value).is_err());
    let mut value = spec();
    value["regions"][0]["fallback"]["addresses"] = json!(["127.0.0.1"]);
    assert!(parse(&value).is_err());
    let mut dev = spec();
    dev["contour"] = json!("dev");
    dev["regions"][0]["fallback"]["host"] = json!(r"lobby.dev.omoba.ru");
    dev["regions"][0]["fallback"]["addresses"] = json!(["146.103.102.18"]);
    dev["regions"][0]["fallback"]["port"] = json!(4100);
    let dev = BuildTrust::parse(Some(&dev.to_string()), "dev")
        .unwrap()
        .unwrap();
    assert_ne!(
        dev.region(RegionId::Ru)
            .unwrap()
            .cache_path(Path::new("root")),
        parse(&spec())
            .unwrap()
            .region(RegionId::Ru)
            .unwrap()
            .cache_path(Path::new("root"))
    );
}
#[test]
fn malformed_partial_and_unbounded_configuration_never_disable_the_guard() {
    assert!(BuildTrust::parse(None, "production").unwrap().is_none());
    for raw in ["", " ", "null", "{}", "{invalid"] {
        assert!(BuildTrust::parse(Some(raw), "production").is_err());
    }
    assert!(BuildTrust::parse(Some(&" ".repeat(MAX_BUILD_BYTES + 1)), "production").is_err());
    for (field, value) in [
        ("version", json!(2)),
        ("regions", json!([])),
        ("directory_url", json!("https://attacker.invalid")),
    ] {
        let mut input = spec();
        input[field] = value;
        assert!(parse(&input).is_err(), "{field}");
    }
    let mut duplicate = spec();
    let repeated = duplicate["regions"][0].clone();
    duplicate["regions"].as_array_mut().unwrap().push(repeated);
    assert!(parse(&duplicate).is_err());
    let mut invalid = spec();
    invalid["regions"][0]["pins"] = json!([]);
    assert!(parse(&invalid).is_err());
    let mut invalid = spec();
    invalid["regions"][0]["pins"] = json!([pin("same"), pin("same")]);
    assert!(parse(&invalid).is_err());
    for key in [
        "f".repeat(63),
        "G".repeat(64),
        "A".repeat(64),
        "0".repeat(66),
        "0".repeat(64),
        format!("01{}", "00".repeat(31)),
    ] {
        let mut invalid = spec();
        invalid["regions"][0]["pins"][0]["public_key"] = json!(key);
        assert!(parse(&invalid).is_err());
    }
    let mut invalid = spec();
    invalid["regions"][0]["pins"][0]["seed"] = json!("not-a-public-pin");
    assert!(parse(&invalid).is_err());
}
#[test]
fn actual_compiled_configuration_is_valid_for_this_exact_build_contour() {
    assert!(BuildTrust::compiled().is_ok());
}
#[test]
fn runtime_environment_cannot_replace_compiled_trust() {
    const CHILD: &str = "OMOBA_DIRECTORY_TRUST_ENV_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let parsed = BuildTrust::compiled().unwrap();
        assert_eq!(parsed.is_some(), COMPILED.is_some());
        return;
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "game_directory::config::tests::runtime_environment_cannot_replace_compiled_trust",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(
            "OMOBA_DEFAULT_GAME_DIRECTORY_TRUST",
            "not-valid-json-at-runtime",
        )
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
}

#[test]
fn input_audit_uses_the_real_trust_parser_without_certifying_the_binary() {
    let audit = audit_input(spec().to_string().as_bytes(), "production").unwrap();
    assert_eq!(audit["input_valid"], true);
    assert_eq!(audit["compiled_configuration_checked"], false);
    assert_eq!(audit["audit"]["contour"], "production");
    assert_eq!(audit["audit"]["regions"][0]["pins"][0], pin("current"));
    assert_eq!(
        audit["audit"]["regions"][0]["concrete_endpoints"][0],
        "212.113.117.105:4000"
    );
    assert!(audit_input(spec().to_string().as_bytes(), "dev").is_err());
    assert!(audit_input(spec().to_string().as_bytes(), "production ").is_err());
    let mut input = spec();
    input["regions"][0]["pins"][0]["public_key"] = json!("0".repeat(64));
    assert!(audit_input(input.to_string().as_bytes(), "production").is_err());
    input = spec();
    input["regions"][0]["fallback"]["addresses"] = json!(["77.246.105.57"]);
    assert!(audit_input(input.to_string().as_bytes(), "production").is_err());
}

#[test]
fn input_audit_bounds_stream_reads_and_never_echoes_private_or_io_details() {
    let mut bytes = spec().to_string().into_bytes();
    bytes.resize(1024 * 1024, b' ');
    let mut large = std::io::Cursor::new(bytes);
    assert!(audit_input(&mut large, "production").is_err());
    assert_eq!(large.position(), MAX_BUILD_BYTES as u64 + 1);
    assert!(audit_input(&[0xff_u8][..], "production").is_err());
    let mut input = spec();
    input["regions"][0]["pins"][0]["seed"] = json!("private-input-must-not-be-echoed");
    let error = audit_input(input.to_string().as_bytes(), "production").unwrap_err();
    assert!(!format!("{error:?}").contains("private-input"));
    struct FailedRead;
    impl std::io::Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("private-io-error-detail"))
        }
    }
    let error = audit_input(FailedRead, "production").unwrap_err();
    assert!(!format!("{error:?}").contains("private-io"));
    for input in ["", "null", "{}", "{invalid"] {
        assert!(audit_input(input.as_bytes(), "production").is_err());
    }
}

#[test]
fn operator_audit_bad_arguments_never_fall_through_or_read_unrelated_stdin() {
    struct NeverRead;
    impl std::io::Read for NeverRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("this command must not consume stdin")
        }
    }
    assert!(audit_cli(&[], NeverRead).is_none());
    assert!(audit_cli(&["--ordinary-game-option".into()], NeverRead).is_none());
    assert!(audit_cli(&["--check-directory-trust".into()], NeverRead).is_some());
    for args in [
        vec!["--check-directory-trust-input"],
        vec!["--check-directory-trust", "unexpected-file.json"],
        vec!["--check-directory-trust-input", "production", "unexpected"],
        vec!["--check-directory-trust-input", "unknown-contour"],
    ] {
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(audit_cli(&args, NeverRead).unwrap().is_err());
    }
}

#[test]
fn operator_input_command_requires_explicit_contour_and_returns_public_audit() {
    let args = vec!["--check-directory-trust-input".into(), "production".into()];
    let input = spec().to_string();
    let result = audit_cli(&args, input.as_bytes()).unwrap().unwrap();
    assert_eq!(result["input_valid"], true);
    assert_eq!(result["compiled_configuration_checked"], false);
    assert_eq!(result["audit"]["regions"][0]["pins"][1], pin("overlap"));
    assert!(audit_cli(&args, "not-json".as_bytes()).unwrap().is_err());
}
