//! nebula-client: окно (`--window`, в Linux всегда) и значок в трее (Windows, по умолчанию).
//!
//! Окно живёт в отдельном процессе: tray запускает его по клику, а при закрытии процесс
//! завершается целиком и отдаёт память. В фоне остаются только tray и служба.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod icons;
#[cfg(windows)]
mod sys;
#[cfg(windows)]
mod tray;
mod window;

fn main() {
    #[cfg(windows)]
    if !std::env::args().any(|a| a == "--window") {
        tray::run();
        return;
    }
    window::run();
}
