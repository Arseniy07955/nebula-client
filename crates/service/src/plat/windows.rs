//! Windows: именованный канал, job-объект для nebula.exe, брандмауэр, тип сети,
//! права на папку и переход со старой установки PowerShell-скриптом.

use crate::core::Core;
use common::{Envelope, Response};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{FlushFileBuffers, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const FW_PROGRAM: &str = "Nebula Client: nebula.exe";
const FW_PING: &str = "Nebula Client: ping";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ------------------------------------------------------------------ канал

/// Писать в канал могут SYSTEM, администраторы и пользователи, вошедшие интерактивно.
/// Удалённые клиенты не принимаются.
pub fn serve(core: Arc<Core>) {
    let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)");
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
        core.logs.note("pipe: security descriptor failed");
        return;
    }
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
    let name = wide(&common::pipe_name());
    let mut first = true;
    loop {
        // первый экземпляр с FILE_FLAG_FIRST_PIPE_INSTANCE: если канал уже держит другая служба,
        // две копии не будут отвечать клиентам по очереди
        let flags = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                flags,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &sa,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            if first && unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
                let msg = common::tr("канал уже занят другой копией службы, выхожу", "the pipe is taken by another service instance, exiting");
                core.logs.note(msg);
                eprintln!("{msg}");
                std::process::exit(2);
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        first = false;
        if unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) == 0 && GetLastError() != ERROR_PIPE_CONNECTED } {
            unsafe { CloseHandle(h) };
            continue;
        }
        let core = core.clone();
        let hv = h as isize;
        std::thread::spawn(move || handle(core, hv as HANDLE));
    }
}

fn handle(core: Arc<Core>, h: HANDLE) {
    let file = unsafe { std::fs::File::from_raw_handle(h as _) };
    let mut line = String::new();
    let resp = match BufReader::new(&file).take(64 * 1024).read_line(&mut line) {
        Ok(_) => match serde_json::from_str::<Envelope>(&line) {
            Ok(env) => common::with_lang(env.lang.as_deref(), || core.dispatch(env.req)),
            Err(e) => Response::err(format!("bad request: {e}")),
        },
        Err(e) => Response::err(e.to_string()),
    };
    let mut out = serde_json::to_string(&resp).unwrap_or_else(|_| "{\"ok\":false}".into());
    out.push('\n');
    let _ = (&file).write_all(out.as_bytes());
    unsafe {
        FlushFileBuffers(h);
        DisconnectNamedPipe(h);
    }
}

// ------------------------------------------------------------ процессы

pub fn quiet(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

pub fn die_with_parent(_cmd: &mut Command) {}

/// Job-объект с KILL_ON_JOB_CLOSE: умерла служба — умерла и nebula.exe.
pub struct ChildGuard(HANDLE);

unsafe impl Send for ChildGuard {}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

pub fn guard_child(child: &Child) -> ChildGuard {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !job.is_null() {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32);
            AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE);
        }
        ChildGuard(job)
    }
}

/// Адаптер Wintun живёт, пока жив процесс, и удаляется вместе с ним, поэтому просто завершаем.
pub fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn unpack(zip_bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { continue };
        let out = dest.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut w = std::fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// --------------------------------------------------------- система

fn run(exe: &str, args: &[&str]) -> (bool, String) {
    let mut c = Command::new(exe);
    c.args(args).creation_flags(CREATE_NO_WINDOW);
    match c.output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)),
        Err(e) => (false, e.to_string()),
    }
}

/// Доступ к папке данных только у SYSTEM и администраторов: там приватные ключи и токены.
pub fn protect_dir(dir: &Path) {
    if std::env::var_os("NEBULA_CLIENT_DIR").is_some() {
        return; // своя папка для отладки, права не трогаем
    }
    let d = dir.to_string_lossy();
    run("icacls.exe", &[&d, "/inheritance:r", "/grant:r", "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F"]);
}

pub fn private_file(_p: &Path) {}

/// Служба не может показать окно «разрешить доступ», поэтому разрешаем nebula.exe заранее,
/// и ping из подсетей всех сетей устройства.
pub fn firewall(nebula: &Path, own: bool, cidrs: &[String]) {
    remove_firewall();
    if own {
        let program = format!("program={}", nebula.display());
        run("netsh.exe", &["advfirewall", "firewall", "add", "rule", &format!("name={FW_PROGRAM}"), "dir=in", "action=allow", &program, "enable=yes", "profile=any"]);
    }
    let cidrs: Vec<&str> = cidrs.iter().map(String::as_str).filter(|c| !c.is_empty() && c.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '/')).collect();
    if !cidrs.is_empty() {
        run("netsh.exe", &["advfirewall", "firewall", "add", "rule", &format!("name={FW_PING}"), "dir=in", "action=allow", "protocol=icmpv4:8,any",
                           &format!("remoteip={}", cidrs.join(",")), "enable=yes", "profile=any"]);
    }
}

pub fn remove_firewall() {
    for name in [FW_PROGRAM, FW_PING] {
        run("netsh.exe", &["advfirewall", "firewall", "delete", "rule", &format!("name={name}")]);
    }
}

/// Новый адаптер Windows считает общедоступной сетью и режет входящие. Ставим «частную».
pub fn set_private(dev: &str) {
    if dev.is_empty() || !dev.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return;
    }
    let script = format!(
        "$p = Get-NetConnectionProfile -InterfaceAlias '{dev}' -ErrorAction Stop; if ($p.NetworkCategory -ne 'Private') {{ Set-NetConnectionProfile -InterfaceAlias '{dev}' -NetworkCategory Private -ErrorAction Stop }}"
    );
    run("powershell.exe", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]);
}

/// PowerShell-версия клиента ставила nebula.exe отдельной службой «Nebula» и задачу планировщика.
/// Теперь Nebula запускает эта служба, старое убираем, если оно из нашей папки.
pub fn migrate_script_install(dir: &Path) -> bool {
    let (ok, qc) = run("sc.exe", &["qc", "Nebula"]);
    let ours = ok && qc.to_lowercase().contains(&dir.join("bin").to_string_lossy().to_lowercase());
    if ours {
        run("sc.exe", &["stop", "Nebula"]);
        std::thread::sleep(std::time::Duration::from_secs(2));
        run("sc.exe", &["delete", "Nebula"]);
    }
    run("schtasks.exe", &["/Delete", "/TN", "Nebula Client Sync", "/F"]);
    let _ = std::fs::remove_file(dir.join("nebula-join.ps1"));
    ours
}

pub const OS: &str = "windows";

// ------------------------------------------------------------------ обновление

/// Запустить скачанный установщик тихо. Он остановит службу, заменит файлы и запустит её снова;
/// значок в трее он закрывает, поэтому оставляем метку, чтобы новая служба вернула его.
pub fn run_installer(setup: &Path, dir: &Path) -> Result<(), String> {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let upd = dir.join("updates");
    let marker = upd.join("relaunch-tray");
    std::fs::write(&marker, b"1").map_err(|e| e.to_string())?;
    let r = Command::new(setup)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/UPDATE=1"])
        .arg(format!("/LOG={}", upd.join("setup.log").display()))
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn();
    match r {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&marker);
            Err(format!("{}: {e}", setup.display()))
        }
    }
}

/// Запустить nebula-client.exe в сессии пользователя, который сидит за компьютером.
/// Служба работает от SYSTEM, поэтому берём токен этой сессии.
pub fn relaunch_tray(logs: &Arc<crate::logs::Logs>) {
    use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows_sys::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
    use windows_sys::Win32::System::Threading::{CreateProcessAsUserW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW};

    let Some(app) = std::env::current_exe().ok().map(|e| e.with_file_name("nebula-client.exe")).filter(|p| p.exists()) else { return };
    let logs = logs.clone();
    std::thread::spawn(move || {
        // установщик ещё дописывает файлы и ярлыки
        std::thread::sleep(std::time::Duration::from_secs(8));
        unsafe {
            let sid = WTSGetActiveConsoleSessionId();
            let mut tok: HANDLE = std::ptr::null_mut();
            if sid == u32::MAX || WTSQueryUserToken(sid, &mut tok) == 0 {
                // никто не вошёл: значок поднимется сам при входе (автозапуск)
                return;
            }
            let mut env: *mut core::ffi::c_void = std::ptr::null_mut();
            let has_env = CreateEnvironmentBlock(&mut env, tok, 0) != 0;
            let mut desktop = wide(r"winsta0\default");
            let mut si: STARTUPINFOW = std::mem::zeroed();
            si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
            si.lpDesktop = desktop.as_mut_ptr();
            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
            let mut cmd = wide(&format!("\"{}\"", app.display()));
            let cwd = wide(&app.parent().map(|p| p.display().to_string()).unwrap_or_default());
            let ok = CreateProcessAsUserW(
                tok,
                std::ptr::null(),
                cmd.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                CREATE_UNICODE_ENVIRONMENT,
                if has_env { env } else { std::ptr::null_mut() },
                cwd.as_ptr(),
                &si,
                &mut pi,
            );
            if ok != 0 {
                CloseHandle(pi.hProcess);
                CloseHandle(pi.hThread);
            } else {
                logs.note(format!("tray: CreateProcessAsUser {}", GetLastError()));
            }
            if has_env {
                DestroyEnvironmentBlock(env);
            }
            CloseHandle(tok);
        }
    });
}

// ------------------------------------------------------------------ пинг

/// Один ICMP echo через IcmpSendEcho: прав администратора не нужно, консоль не мигает.
pub fn ping(ip: std::net::Ipv4Addr, timeout: std::time::Duration) -> Option<f32> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY};
    let data = *b"nebula-client-ping";
    let mut reply = vec![0u8; std::mem::size_of::<ICMP_ECHO_REPLY>() + data.len() + 8];
    unsafe {
        let h = IcmpCreateFile();
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let n = IcmpSendEcho(
            h,
            u32::from_ne_bytes(ip.octets()),
            data.as_ptr() as _,
            data.len() as u16,
            std::ptr::null(),
            reply.as_mut_ptr() as _,
            reply.len() as u32,
            timeout.as_millis() as u32,
        );
        IcmpCloseHandle(h);
        if n == 0 {
            return None;
        }
        let r = std::ptr::read_unaligned(reply.as_ptr() as *const ICMP_ECHO_REPLY);
        // Status 0 = IP_SUCCESS; время у Windows целыми миллисекундами
        (r.Status == 0).then_some(r.RoundTripTime as f32)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ping_loopback() {
        assert!(super::ping("127.0.0.1".parse().unwrap(), std::time::Duration::from_secs(1)).is_some());
    }

    /// NEBULA_PING_TEST=10.77.0.1 cargo test ping_target -- --ignored --nocapture
    #[test]
    #[ignore]
    fn ping_target() {
        let ip = std::env::var("NEBULA_PING_TEST").unwrap();
        println!("{ip}: {:?}", super::ping(ip.parse().unwrap(), std::time::Duration::from_millis(1500)));
    }
}
