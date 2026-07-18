#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 启动 Tauri 桌面应用。
fn main() {
    meeting_desk_lib::run();
}
