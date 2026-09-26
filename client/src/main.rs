//! Desktop and iOS executable entry; Android uses the library NativeActivity entry.
// A release Windows build is a windowed game: no extra console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
fn main() {
    client::main();
}
