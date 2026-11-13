//! Refuse unsafe fixture modes before connecting or creating any process state.
use std::process::Command;

#[test]
fn search_gate_requires_a_path_and_public_loopback_mode() {
    for arguments in [
        vec!["--search-gate"],
        vec!["--search-gate", "/not-used"],
        vec![
            "--public",
            "--server",
            "198.51.100.1:4000",
            "--search-gate",
            "/not-used",
        ],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_bots"))
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(
            result.stdout.is_empty(),
            "admission must not start on invalid arguments"
        );
        assert!(String::from_utf8_lossy(&result.stderr).contains("search-gate"));
    }
}
