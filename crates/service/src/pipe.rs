//! Именованный канал для окна и tray. Писать в него могут SYSTEM, администраторы
//! и пользователи, вошедшие в систему интерактивно. Удалённые клиенты не принимаются.

use crate::core::Core;
use common::{Request, Response, PIPE_NAME};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn serve(core: Arc<Core>) {
    // SY и BA: полный доступ, IU (интерактивные пользователи): чтение и запись
    let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)");
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) };
    if ok == 0 {
        core.logs.note("не удалось задать права на канал, окно не сможет подключиться");
        return;
    }
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
    let name = wide(PIPE_NAME);
    loop {
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &sa,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        let connected = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED };
        if !connected {
            unsafe { CloseHandle(h) };
            continue;
        }
        let core = core.clone();
        let hv = h as isize;
        std::thread::spawn(move || handle(core, hv as HANDLE));
    }
    #[allow(unreachable_code)]
    unsafe {
        LocalFree(sd as _);
    }
}

fn handle(core: Arc<Core>, h: HANDLE) {
    let file = unsafe { std::fs::File::from_raw_handle(h as _) };
    let mut line = String::new();
    let resp = match BufReader::new(&file).take(64 * 1024).read_line(&mut line) {
        Ok(_) => match serde_json::from_str::<Request>(&line) {
            Ok(req) => core.dispatch(req),
            Err(e) => Response::err(format!("непонятный запрос: {e}")),
        },
        Err(e) => Response::err(e.to_string()),
    };
    let mut out = serde_json::to_string(&resp).unwrap_or_else(|_| "{\"ok\":false}".into());
    out.push('\n');
    let mut w = &file;
    let _ = w.write_all(out.as_bytes());
    unsafe {
        FlushFileBuffers(h);
        DisconnectNamedPipe(h);
    }
    drop(file);
}
