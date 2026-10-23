//! Linux: unix-сокет, PDEATHSIG для nebula, мягкая остановка, права на файлы.
//! Брандмауэр и тип сети тут не трогаем: это дело дистрибутива (на NixOS ping разрешён по умолчанию).

use crate::core::Core;
use common::{Envelope, Response};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Группа, участникам которой открыт сокет. Без неё сокет доступен только root.
const GROUP: &str = "nebula-client";

fn group_gid() -> Option<libc::gid_t> {
    let name = std::ffi::CString::new(GROUP).ok()?;
    let g = unsafe { libc::getgrnam(name.as_ptr()) };
    if g.is_null() {
        None
    } else {
        Some(unsafe { (*g).gr_gid })
    }
}

pub fn serve(core: Arc<Core>) {
    let path = common::socket_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            core.logs.note(format!("socket {}: {e}", path.display()));
            return;
        }
    };
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
    match group_gid() {
        Some(gid) => unsafe {
            libc::chown(c.as_ptr(), 0, gid);
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660));
        },
        None => {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
    for stream in listener.incoming().flatten() {
        let core = core.clone();
        std::thread::spawn(move || handle(core, stream));
    }
}

fn handle(core: Arc<Core>, s: UnixStream) {
    let mut line = String::new();
    let resp = match BufReader::new(&s).take(64 * 1024).read_line(&mut line) {
        Ok(_) => match serde_json::from_str::<Envelope>(&line) {
            Ok(env) => common::with_lang(env.lang.as_deref(), || core.dispatch(env.req)),
            Err(e) => Response::err(format!("bad request: {e}")),
        },
        Err(e) => Response::err(e.to_string()),
    };
    let mut out = serde_json::to_string(&resp).unwrap_or_else(|_| "{\"ok\":false}".into());
    out.push('\n');
    let _ = (&s).write_all(out.as_bytes());
}

pub fn quiet(_cmd: &mut Command) {}

/// Служба умерла — ядро пришлёт nebula SIGTERM.
pub fn die_with_parent(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
}

pub struct ChildGuard;

pub fn guard_child(_child: &Child) -> ChildGuard {
    ChildGuard
}

/// Сначала SIGTERM, чтобы nebula убрала за собой tun и маршруты, через 5 с добиваем.
pub fn stop_child(child: &mut Child) {
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn unpack(tgz: &[u8], dest: &Path) -> Result<(), String> {
    let gz = flate2::read::GzDecoder::new(tgz);
    let mut ar = tar::Archive::new(gz);
    ar.set_preserve_permissions(true);
    ar.unpack(dest).map_err(|e| e.to_string())?;
    for exe in ["nebula", "nebula-cert"] {
        let _ = std::fs::set_permissions(dest.join(exe), std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

pub fn protect_dir(dir: &Path) {
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

pub fn private_file(p: &Path) {
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
}

pub fn firewall(_nebula: &Path, _own: bool, _cidrs: &[String]) {}

pub fn remove_firewall() {}

pub fn set_private(_dev: &str) {}

pub fn migrate_script_install(_dir: &Path) -> bool {
    false
}

pub const OS: &str = "linux";
