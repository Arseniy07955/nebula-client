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

// ------------------------------------------------------------------ обновление

/// В Linux обновляют install.sh, пакетный менеджер или Nix, служба только сообщает о версии.
#[derive(Debug)]
pub struct DnsRule {
    pub slug: String,
    pub tun: String,
    pub server: String,
    pub domains: Vec<String>,
}

/// systemd-resolved: DNS и зоны-маршруты (~neb) на адаптере сети. Когда адаптер пропадает,
/// resolved забывает их сам, поэтому убирать нужно только с живых адаптеров без правила.
pub fn set_dns(rules: &[DnsRule]) -> Result<(), String> {
    let mut known = DNS_SET.lock().unwrap();
    for tun in known.iter().filter(|t| !rules.iter().any(|r| &&r.tun == t)) {
        let _ = Command::new("resolvectl").args(["revert", tun]).output();
    }
    known.clear();
    let mut errs = vec![];
    for r in rules {
        let routes: Vec<String> = r.domains.iter().map(|d| format!("~{d}")).collect();
        let steps: [Vec<&str>; 3] = [
            vec!["dns", &r.tun, &r.server],
            [vec!["domain", r.tun.as_str()], routes.iter().map(String::as_str).collect()].concat(),
            vec!["default-route", &r.tun, "false"],
        ];
        for args in steps {
            match Command::new("resolvectl").args(&args).output() {
                Ok(o) if o.status.success() => {}
                Ok(o) => {
                    errs.push(format!("{}: resolvectl {}: {}", r.slug, args[0], String::from_utf8_lossy(&o.stderr).trim()));
                    break;
                }
                Err(e) => {
                    errs.push(format!("resolvectl: {e} (нужен systemd-resolved)"));
                    break;
                }
            }
        }
        known.push(r.tun.clone());
    }
    if errs.is_empty() { Ok(()) } else { Err(errs.join("; ")) }
}

static DNS_SET: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn run_installer(_setup: &Path, _dir: &Path) -> Result<(), String> {
    Err(common::tr("в Linux обновление ставится вручную", "on Linux the update is installed manually").into())
}

pub fn relaunch_tray(_logs: &Arc<crate::logs::Logs>) {}

// ------------------------------------------------------------------ пинг

fn icmp_checksum(b: &[u8]) -> u16 {
    let mut sum: u32 = b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32).sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Один ICMP echo через «ping-сокет» (SOCK_DGRAM + IPPROTO_ICMP): служба работает от root,
/// ей он доступен всегда, внешний ping не нужен.
pub fn ping(ip: std::net::Ipv4Addr, timeout: Duration) -> Option<f32> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_ICMP);
        if fd < 0 {
            return None;
        }
        let r = ping_fd(fd, ip, timeout);
        libc::close(fd);
        r
    }
}

unsafe fn ping_fd(fd: libc::c_int, ip: std::net::Ipv4Addr, timeout: Duration) -> Option<f32> {
    let tv = libc::timeval { tv_sec: timeout.as_secs() as _, tv_usec: timeout.subsec_micros() as _ };
    libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as u32);
    let mut addr: libc::sockaddr_in = std::mem::zeroed();
    addr.sin_family = libc::AF_INET as _;
    addr.sin_addr.s_addr = u32::from_ne_bytes(ip.octets());
    let seq = (std::process::id() as u16) ^ (Instant::now().elapsed().subsec_nanos() as u16) ^ ip.octets()[3] as u16;
    let mut pkt = [0u8; 24];
    pkt[0] = 8; // echo request; идентификатор подставит ядро
    pkt[6..8].copy_from_slice(&seq.to_be_bytes());
    pkt[8..].copy_from_slice(b"nebula-client-pi");
    let cs = icmp_checksum(&pkt);
    pkt[2..4].copy_from_slice(&cs.to_be_bytes());
    let start = Instant::now();
    let sent = libc::sendto(fd, pkt.as_ptr() as *const libc::c_void, pkt.len(), 0, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in>() as u32);
    if sent < 0 {
        return None;
    }
    let mut buf = [0u8; 256];
    while start.elapsed() < timeout {
        let n = libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0);
        if n < 0 {
            return None;
        }
        // ответ ping-сокета приходит без IP-заголовка: тип 0 = echo reply
        if n >= 8 && buf[0] == 0 && buf[6..8] == seq.to_be_bytes() {
            return Some(start.elapsed().as_secs_f32() * 1000.0);
        }
    }
    None
}
