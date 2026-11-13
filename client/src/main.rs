//! Desktop and iOS executable entry; Android uses the library NativeActivity entry.
// A release Windows build is a windowed game: no extra console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(
        args.first().map(String::as_str),
        Some("--check-directory-trust" | "--check-directory-trust-input")
    ) {
        let result = client::game_directory::config::audit_cli(&args, std::io::stdin().lock())
            .expect("recognized directory audit command");
        match result {
            Ok(audit) => println!("{audit}"),
            Err(error) => {
                eprintln!("Directory trust check failed: {error:?}");
                std::process::exit(2);
            }
        }
        return;
    }
    client::main();
}
