//! Мелочи Windows для приложения: один экземпляр, автозапуск, запуск с правами администратора.

use std::path::PathBuf;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOWNORMAL};

pub const WINDOW_TITLE: &str = "Nebula Client";

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// true, если мьютекс уже держит другой процесс. Свой держим до выхода (утечка намеренная).
pub fn already_running(name: &str) -> bool {
    let n = wide(name);
    let h: HANDLE = unsafe { CreateMutexW(std::ptr::null(), 0, n.as_ptr()) };
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _ = h;
    exists
}

pub fn focus_existing_window() -> bool {
    let t = wide(WINDOW_TITLE);
    let hwnd = unsafe { FindWindowW(std::ptr::null(), t.as_ptr()) };
    if hwnd.is_null() {
        return false;
    }
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
    true
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "Nebula Client";

/// Запускать tray при входе пользователя (HKCU\...\Run).
pub fn autostart_enabled() -> bool {
    let key = wide(RUN_KEY);
    let name = wide(RUN_NAME);
    let mut size = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == 0 }
}

pub fn set_autostart(on: bool) {
    let key = wide(RUN_KEY);
    let name = wide(RUN_NAME);
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let val = wide(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, val.as_ptr() as _, (val.len() * 2) as u32);
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr());
        }
    }
}

pub fn service_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_default().with_file_name("nebula-client-service.exe")
}

/// Поставить службу: UAC спросит подтверждение.
pub fn install_service() -> bool {
    let exe = wide(&service_exe().to_string_lossy());
    let verb = wide("runas");
    let args = wide("install");
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), exe.as_ptr(), args.as_ptr(), std::ptr::null(), SW_SHOWNORMAL) };
    r as isize > 32
}

pub fn open_window() {
    if focus_existing_window() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--window").spawn();
    }
}
