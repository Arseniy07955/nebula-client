//! nebula-client.exe: значок в трее (по умолчанию) и окно (`--window`).
//!
//! Окно живёт в отдельном процессе: tray запускает его по клику, а при закрытии процесс
//! завершается целиком и отдаёт память. В фоне остаются только tray и служба.

#![windows_subsystem = "windows"]

mod icons;
mod sys;
mod tray;
mod window;

fn main() {
    if std::env::args().any(|a| a == "--window") {
        window::run();
    } else {
        tray::run();
    }
}
