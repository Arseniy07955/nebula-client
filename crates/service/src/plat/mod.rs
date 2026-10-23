//! Всё, что зависит от ОС: канал к окну, запуск и остановка nebula, брандмауэр, права на файлы.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;
