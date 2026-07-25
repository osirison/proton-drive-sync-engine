// Prevent an extra console window on Windows in release. (No-op on Linux, the target platform.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    proton_sync_gui_lib::run();
}
