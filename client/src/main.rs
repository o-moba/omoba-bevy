//! Desktop and iOS executable entry; Android uses the library NativeActivity entry.
// A release Windows build is a windowed game: no extra console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--check-directory-trust") {
        match client::game_directory::config::audit_compiled() {
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
