//! Настройки Windows: права на папку, брандмауэр, тип сети и переход со старой установки скриптом.

use crate::nebula::CREATE_NO_WINDOW;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const FW_PROGRAM: &str = "Nebula Client: nebula.exe";
const FW_PING: &str = "Nebula Client: ping";

fn run(exe: &str, args: &[&str]) -> (bool, String) {
    match Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)),
        Err(e) => (false, e.to_string()),
    }
}

/// Доступ к папке данных только у SYSTEM и администраторов: там приватный ключ и токен.
pub fn protect_dir(dir: &Path) {
    if std::env::var_os("NEBULA_CLIENT_DIR").is_some() {
        return; // своя папка для отладки, права не трогаем
    }
    let d = dir.to_string_lossy();
    run("icacls.exe", &[&d, "/inheritance:r", "/grant:r", "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F"]);
}

pub fn firewall(nebula_exe: &Path, cidr: &str) {
    remove_firewall();
    let program = format!("program={}", nebula_exe.display());
    // служба не может показать окно «разрешить доступ», поэтому разрешаем nebula.exe заранее
    run("netsh.exe", &["advfirewall", "firewall", "add", "rule", &format!("name={FW_PROGRAM}"), "dir=in", "action=allow", &program, "enable=yes", "profile=any"]);
    if cidr.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '/') && !cidr.is_empty() {
        run("netsh.exe", &["advfirewall", "firewall", "add", "rule", &format!("name={FW_PING}"), "dir=in", "action=allow",
                           "protocol=icmpv4:8,any", &format!("remoteip={cidr}"), "enable=yes", "profile=any"]);
    }
}

pub fn remove_firewall() {
    for name in [FW_PROGRAM, FW_PING] {
        run("netsh.exe", &["advfirewall", "firewall", "delete", "rule", &format!("name={name}")]);
    }
}

/// Новый адаптер Windows считает общедоступной сетью и режет входящие. Ставим «частную».
pub fn set_private(dev: &str) -> bool {
    if dev.is_empty() || !dev.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return false;
    }
    let script = format!(
        "$p = Get-NetConnectionProfile -InterfaceAlias '{dev}' -ErrorAction Stop; if ($p.NetworkCategory -ne 'Private') {{ Set-NetConnectionProfile -InterfaceAlias '{dev}' -NetworkCategory Private -ErrorAction Stop }}"
    );
    run("powershell.exe", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]).0
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
    if dir.join("nebula-join.ps1").exists() {
        let _ = std::fs::remove_file(dir.join("nebula-join.ps1"));
    }
    ours
}
