//! Состояние службы: сети устройства, их nebula и синхронизация с панелью.
//!
//! Каждая сеть живёт в своей папке networks/<код>: свой ключ, сертификат, конфиг, токен,
//! свой процесс nebula и свой адаптер. Так можно быть одновременно, например, в основной
//! сети 10.0.10.0/24 и у друзей в 10.77.0.0/24, даже если это разные панели.
//!
//! Долгие операции (вход, синхронизация) идут под `op`, а `inner` блокируется только коротко,
//! чтобы окно могло спрашивать статус, пока, например, скачивается Nebula.

use crate::api::{self, NetInfo, Payload};
use crate::logs::Logs;
use crate::nebula::{self, Running};
use crate::plat;
use crate::update::{self, Upd};
use common::{now, tr, trf, ConnState, NetDns, NetStatus, OpenNet, Peer, Request, Response, Status, UpdateMode, CLIENT_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SYNC_EVERY: Duration = Duration::from_secs(6 * 3600);
const SYNC_RETRY: Duration = Duration::from_secs(15 * 60);
const PEERS_TTL: i64 = 15;

/// device.json: тот же формат, что у PowerShell-скрипта.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Device {
    server: String,
    token: String,
    name: String,
    ip: String,
    network: NetInfo,
    #[serde(default)]
    enrolled: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default = "yes")]
    want_up: bool,
    #[serde(default)]
    cert_not_after: Option<i64>,
    #[serde(default)]
    last_sync: Option<i64>,
    /// DNS сети с панели, обновляется при синхронизации.
    #[serde(default)]
    dns: Option<NetDns>,
    #[serde(default = "yes")]
    dns_on: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings { want_up: true, cert_not_after: None, last_sync: None, dns: None, dns_on: true }
    }
}

struct Net {
    dev: Device,
    set: Settings,
    /// Имя адаптера, как оно в config.yml на этом устройстве.
    tun: String,
    child: Option<Running>,
    state: ConnState,
    error: Option<String>,
    sync_error: Option<String>,
    peers: Vec<Peer>,
    peers_ts: i64,
    fails: u32,
    next_try: Option<Instant>,
    next_sync: Instant,
}

impl Net {
    fn new(dev: Device, set: Settings, tun: String) -> Net {
        Net {
            dev,
            set,
            tun,
            child: None,
            state: ConnState::Stopped,
            error: None,
            sync_error: None,
            peers: vec![],
            peers_ts: 0,
            fails: 0,
            next_try: None,
            next_sync: Instant::now() + Duration::from_secs(10),
        }
    }
}

#[derive(Default)]
struct Inner {
    nets: BTreeMap<String, Net>,
    busy: Option<String>,
    firewall_sig: Option<String>,
    dns_sig: Option<String>,
}

pub struct Core {
    dir: PathBuf,
    inner: Mutex<Inner>,
    op: Mutex<()>,
    pub logs: Arc<Logs>,
    sync_all: AtomicBool,
    stopping: AtomicBool,
    upd: Mutex<Upd>,
    update_now: AtomicBool,
}

const NET_FILES: &[&str] = &["device.json", "settings.json", "host.key", "host.pub", "host.crt", "ca.crt", "config.yml", "ssh_host_ed25519_key"];

fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    let text = text.replace("\r\n", "\n");
    if std::fs::read_to_string(path).map(|old| old == text).unwrap_or(false) {
        return Ok(false);
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// Имя адаптера из config.yml (секция tun, ключ dev).
fn tun_of(config: &str) -> String {
    config
        .lines()
        .skip_while(|l| !l.starts_with("tun:"))
        .find_map(|l| l.trim().strip_prefix("dev:").map(|v| v.trim().trim_matches(['\'', '"']).to_string()))
        .unwrap_or_default()
}

/// Если две сети (например, с разных панелей) хотят один адаптер, второй переименовываем.
fn unique_tun(config: &str, slug: &str, taken: &[String]) -> String {
    let dev = tun_of(config);
    if dev.is_empty() || !taken.contains(&dev) {
        return config.to_string();
    }
    let mut new: String = format!("nb-{slug}").chars().take(15).collect();
    let mut i = 2;
    while taken.contains(&new) {
        new = format!("nb{i}-{slug}").chars().take(15).collect();
        i += 1;
    }
    let mut in_tun = false;
    config
        .lines()
        .map(|l| {
            if !l.starts_with(' ') && !l.is_empty() {
                in_tun = l.starts_with("tun:");
            }
            if in_tun && l.trim().starts_with("dev:") {
                format!("  dev: {new}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

impl Core {
    pub fn new(dir: PathBuf) -> Arc<Core> {
        let logs = Arc::new(Logs::default());
        let upd = Mutex::new(Upd { saved: update::Saved::load(&dir), ..Default::default() });
        let core = Arc::new(Core {
            upd,
            update_now: AtomicBool::new(false),
            dir,
            inner: Mutex::new(Inner::default()),
            op: Mutex::new(()),
            logs,
            sync_all: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        });
        core.migrate_flat();
        core.load();
        core
    }

    fn nets_dir(&self) -> PathBuf {
        self.dir.join("networks")
    }

    fn net_dir(&self, slug: &str) -> PathBuf {
        self.nets_dir().join(slug)
    }

    /// Версии до 0.3 держали одну сеть прямо в папке данных. Переносим её в networks/<код>.
    fn migrate_flat(&self) {
        let Ok(b) = std::fs::read(self.dir.join("device.json")) else { return };
        let b = b.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&b).to_vec();
        let Ok(dev) = serde_json::from_slice::<Device>(&b) else { return };
        let slug = dev.network.slug.clone();
        if !common::valid_slug(&slug) {
            return;
        }
        let to = self.net_dir(&slug);
        if std::fs::create_dir_all(&to).is_err() {
            return;
        }
        for f in NET_FILES {
            let from = self.dir.join(f);
            if from.exists() {
                let _ = std::fs::rename(&from, to.join(f));
            }
        }
        // пути к ключам в конфиге: до следующей синхронизации поправим сами
        if let Ok(cfg) = std::fs::read_to_string(to.join("config.yml")) {
            let _ = std::fs::write(to.join("config.yml"), cfg.replace(&format!("{}/", fwd(&self.dir)), &format!("{}/", fwd(&to))));
        }
        self.logs.note(trf!("сеть «{}» перенесена в отдельную папку", "network \"{}\" moved to its own folder", dev.network.title));
    }

    fn load(&self) {
        let mut i = self.inner.lock().unwrap();
        let Ok(rd) = std::fs::read_dir(self.nets_dir()) else {
            self.logs.note(tr("служба запущена, сетей пока нет", "service started, no networks yet"));
            return;
        };
        for e in rd.flatten() {
            let slug = e.file_name().to_string_lossy().to_string();
            if !common::valid_slug(&slug) {
                continue;
            }
            let d = e.path();
            let Some(dev) = std::fs::read(d.join("device.json")).ok().and_then(|b| {
                let b = b.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&b).to_vec();
                serde_json::from_slice::<Device>(&b).ok()
            }) else {
                continue;
            };
            let set: Settings = std::fs::read(d.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let tun = std::fs::read_to_string(d.join("config.yml")).map(|c| tun_of(&c)).unwrap_or_default();
            self.logs.note(trf!("сеть «{}»: {} {}", "network \"{}\": {} {}", dev.network.title, dev.name, dev.ip));
            i.nets.insert(slug, Net::new(dev, set, tun));
        }
    }

    fn save_settings(&self, slug: &str, s: &Settings) {
        let _ = std::fs::write(self.net_dir(slug).join("settings.json"), serde_json::to_vec_pretty(s).unwrap_or_default());
    }

    fn set_busy(&self, b: Option<&str>) {
        self.inner.lock().unwrap().busy = b.map(String::from);
        if let Some(b) = b {
            self.logs.note(b);
        }
    }

    pub fn status(&self) -> Status {
        let i = self.inner.lock().unwrap();
        Status {
            networks: i
                .nets
                .iter()
                .map(|(slug, n)| NetStatus {
                    slug: slug.clone(),
                    title: n.dev.network.title.clone(),
                    cidr: n.dev.network.cidr.clone(),
                    server: n.dev.server.clone(),
                    name: n.dev.name.clone(),
                    ip: n.dev.ip.clone(),
                    dev: n.tun.clone(),
                    state: n.state,
                    want_up: n.set.want_up,
                    error: n.error.clone(),
                    cert_not_after: n.set.cert_not_after,
                    last_sync: n.set.last_sync,
                    sync_error: n.sync_error.clone(),
                    dns: n.set.dns.clone(),
                    dns_on: n.set.dns_on,
                })
                .collect(),
            busy: i.busy.clone(),
            service_version: env!("CARGO_PKG_VERSION").into(),
            os: plat::OS.into(),
            update: self.upd.lock().unwrap().state(),
        }
    }

    fn no_net(slug: &str) -> String {
        trf!("сети {} на этом устройстве нет", "this device is not in network {}", slug)
    }

    pub fn dispatch(self: &Arc<Self>, req: Request) -> Response {
        let r = match req {
            Request::Status => Ok(Response::ok()),
            Request::Peers { net } => self.peers(&net).map(|p| Response { peers: Some(p), ..Response::ok() }),
            Request::Logs => Ok(Response { logs: Some(self.logs.tail(400)), ..Response::ok() }),
            Request::OpenNets { server } => self.open_nets(&server).map(|o| Response { open: Some(o), ..Response::ok() }),
            Request::Enroll { server, name, invite, net } => self.enroll(&server, &name, invite.as_deref(), net.as_deref()).map(|s| Response { net: Some(s), ..Response::ok() }),
            Request::Connect { net } => self.set_want(&net, true).map(|_| Response::ok()),
            Request::Disconnect { net } => self.set_want(&net, false).map(|_| Response::ok()),
            Request::Sync { net } => match net {
                Some(n) => self.sync(&n).map(|_| Response::ok()),
                None => {
                    let slugs: Vec<String> = self.inner.lock().unwrap().nets.keys().cloned().collect();
                    let errs: Vec<String> = slugs.iter().filter_map(|s| self.sync(s).err().map(|e| format!("{s}: {e}"))).collect();
                    if errs.is_empty() { Ok(Response::ok()) } else { Err(errs.join("; ")) }
                }
            },
            Request::Leave { net } => self.leave(&net).map(|_| Response::ok()),
            Request::SetDns { net, on } => self.set_dns(&net, on).map(|_| Response::ok()),
            Request::UpdateCheck => self.update_check().map(|_| Response::ok()),
            Request::UpdateSet { mode, prerelease } => {
                let mut u = self.upd.lock().unwrap();
                let wake = u.saved.mode == UpdateMode::Off && mode != UpdateMode::Off || u.saved.prerelease != prerelease;
                u.saved.mode = mode;
                u.saved.prerelease = prerelease;
                u.saved.save(&self.dir);
                if wake {
                    self.update_now.store(true, Ordering::SeqCst);
                }
                Ok(Response::ok())
            }
            Request::UpdateInstall => self.update_install().map(|_| Response::ok()),
        };
        let mut resp = r.unwrap_or_else(Response::err);
        resp.status = Some(self.status());
        resp
    }

    fn open_nets(&self, server: &str) -> Result<Vec<OpenNet>, String> {
        let server = api::check_server(server)?;
        let v = api::get(&server, "/v1/ping", None).map_err(|e| e.to_string())?;
        Ok(serde_json::from_value(v["open"].clone()).unwrap_or_default())
    }

    // ------------------------------------------------------------ вход в сеть

    fn enroll(self: &Arc<Self>, server: &str, name: &str, invite: Option<&str>, net: Option<&str>) -> Result<String, String> {
        let _op = self.op.try_lock().map_err(|_| tr("служба занята, подождите", "the service is busy, please wait").to_string())?;
        let server = api::check_server(server)?;
        let name = name.trim();
        if !common::valid_device_name(name) {
            return Err(tr(
                "Имя устройства: латиница, цифры, точка, дефис, подчёркивание, до 63 символов",
                "Device name: Latin letters, digits, dot, dash, underscore, up to 63 characters",
            )
            .into());
        }
        let tmp = self.nets_dir().join(format!(".new-{}", now()));
        let r = self.enroll_steps(&server, name, invite.map(str::trim).filter(|s| !s.is_empty()), net, &tmp);
        if tmp.exists() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        self.set_busy(None);
        match &r {
            Ok(slug) => self.logs.note(trf!("устройство вошло в сеть {}", "device joined network {}", slug)),
            Err(e) => self.logs.note(trf!("вход не удался: {}", "join failed: {}", e)),
        }
        r
    }

    fn enroll_steps(self: &Arc<Self>, server: &str, name: &str, invite: Option<&str>, net: Option<&str>, tmp: &Path) -> Result<String, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        plat::protect_dir(&self.dir);
        if plat::migrate_script_install(&self.dir) {
            self.logs.note(tr("старая служба Nebula от PowerShell-скрипта убрана", "removed the old Nebula service from the PowerShell script"));
        }
        let bins = nebula::ensure_binaries(&self.dir, &|s| self.set_busy(Some(s)))?;
        std::fs::create_dir_all(tmp).map_err(|e| e.to_string())?;
        self.set_busy(Some(tr("Создаю ключ устройства", "Creating the device key")));
        let public_key = nebula::ensure_key(&bins, tmp)?;
        self.set_busy(Some(tr("Получаю сертификат", "Getting a certificate")));
        let body = json!({
            "invite": invite.unwrap_or(""),
            "network": net,
            "name": name,
            "public_key": public_key,
            "conf_dir": fwd(tmp),
            "client": CLIENT_VERSION,
        });
        let v = api::post(server, "/v1/enroll", None, body).map_err(|e| e.to_string())?;
        let p = api::payload(v).map_err(|e| e.to_string())?;
        let token = p.token.clone().ok_or(tr("панель не выдала токен устройства", "the panel did not issue a device token"))?;
        let slug = p.network.slug.clone();
        if !common::valid_slug(&slug) {
            return Err(format!("bad network code {slug}"));
        }
        if self.inner.lock().unwrap().nets.contains_key(&slug) {
            return Err(trf!(
                "это устройство уже в сети «{}». Лишнюю запись можно отозвать в панели",
                "this device is already in network \"{}\". The extra entry can be revoked in the panel",
                p.network.title
            ));
        }
        let dir = self.net_dir(&slug);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(tmp, &dir).map_err(|e| e.to_string())?;
        // конфиг выдан под временную папку, переводим пути на постоянную
        let p = Payload { config: p.config.replace(&format!("{}/", fwd(tmp)), &format!("{}/", fwd(&dir))), ..p };
        let tun = self.write_files(&slug, &p)?.1;
        nebula::test_config(&bins, &dir)?;
        let dev = Device { server: server.into(), token, name: p.name.clone(), ip: p.ip.clone(), network: p.network.clone(), enrolled: now().to_string() };
        std::fs::write(dir.join("device.json"), serde_json::to_vec_pretty(&dev).unwrap()).map_err(|e| e.to_string())?;
        plat::private_file(&dir.join("device.json"));
        let set = Settings { want_up: true, cert_not_after: Some(p.not_after), last_sync: Some(now()), dns: p.dns.clone(), dns_on: true };
        self.save_settings(&slug, &set);
        let mut net = Net::new(dev, set, tun);
        net.peers = p.peers;
        net.peers_ts = now();
        net.next_sync = Instant::now() + SYNC_EVERY;
        self.inner.lock().unwrap().nets.insert(slug.clone(), net);
        Ok(slug)
    }

    /// Записать сертификат и конфиг сети. Возвращает (что-то поменялось, имя адаптера).
    fn write_files(&self, slug: &str, p: &Payload) -> Result<(bool, String), String> {
        let dir = self.net_dir(slug);
        let taken: Vec<String> = self.inner.lock().unwrap().nets.iter().filter(|(s, _)| s.as_str() != slug).map(|(_, n)| n.tun.clone()).collect();
        let config = unique_tun(&p.config, slug, &taken);
        let mut changed = false;
        for (f, text) in [("host.crt", &p.cert), ("ca.crt", &p.ca), ("config.yml", &config)] {
            changed |= write_if_changed(&dir.join(f), text)?;
        }
        if let Some(k) = &p.ssh_host_key {
            changed |= write_if_changed(&dir.join("ssh_host_ed25519_key"), k)?;
            plat::private_file(&dir.join("ssh_host_ed25519_key"));
        }
        Ok((changed, tun_of(&config)))
    }

    // ---------------------------------------------------------- синхронизация

    pub fn sync(&self, slug: &str) -> Result<(), String> {
        let _op = self.op.lock().unwrap();
        let Some(dev) = self.inner.lock().unwrap().nets.get(slug).map(|n| n.dev.clone()) else {
            return Err(Self::no_net(slug));
        };
        let body = json!({ "conf_dir": fwd(&self.net_dir(slug)), "client": CLIENT_VERSION });
        let r = api::post(&dev.server, "/v1/sync", Some(&dev.token), body).and_then(api::payload);
        let p = match r {
            Ok(p) => p,
            Err(e) => {
                let msg = match e.code() {
                    401 | 403 => trf!("панель отключила устройство: {}", "the panel disabled this device: {}", e),
                    _ => e.to_string(),
                };
                self.logs.note(format!("[{slug}] sync: {msg}"));
                if let Some(n) = self.inner.lock().unwrap().nets.get_mut(slug) {
                    n.sync_error = Some(msg.clone());
                    n.next_sync = Instant::now() + SYNC_RETRY;
                }
                return Err(msg);
            }
        };
        let (changed, tun) = self.write_files(slug, &p)?;
        if changed {
            nebula::test_config(&nebula::bins(&self.dir), &self.net_dir(slug))?;
        }
        let mut i = self.inner.lock().unwrap();
        let Some(n) = i.nets.get_mut(slug) else { return Ok(()) };
        n.set.cert_not_after = Some(p.not_after);
        n.set.last_sync = Some(now());
        if n.set.dns != p.dns {
            self.logs.note(format!("[{slug}] {}", match &p.dns {
                Some(d) => trf!("DNS сети: {} для {}", "network DNS: {} for {}", d.all().join(", "), d.domains.join(", ")),
                None => tr("панель убрала DNS сети", "the panel removed the network DNS").to_string(),
            }));
            n.set.dns = p.dns.clone();
        }
        n.tun = tun;
        n.sync_error = None;
        n.peers = p.peers;
        n.peers_ts = now();
        n.next_sync = Instant::now() + SYNC_EVERY;
        let set = n.set.clone();
        if changed {
            self.logs.note(format!("[{slug}] {}", tr("сертификат или конфиг обновились, перезапускаю Nebula", "certificate or config changed, restarting Nebula")));
            if let Some(c) = n.child.take() {
                c.stop();
            }
            n.next_try = None;
        } else {
            self.logs.note(format!("[{slug}] {}", tr("синхронизация: без изменений", "sync: no changes")));
        }
        drop(i);
        self.save_settings(slug, &set);
        Ok(())
    }

    fn peers(&self, slug: &str) -> Result<Vec<Peer>, String> {
        let (dev, fresh, cached, running) = {
            let i = self.inner.lock().unwrap();
            let n = i.nets.get(slug).ok_or_else(|| Self::no_net(slug))?;
            (n.dev.clone(), now() - n.peers_ts < PEERS_TTL, n.peers.clone(), n.state == ConnState::Running)
        };
        if fresh {
            return Ok(cached);
        }
        let mut fetched = api::get(&dev.server, "/v1/peers", Some(&dev.token)).ok().map(|v| serde_json::from_value::<Vec<Peer>>(v["peers"].clone()).unwrap_or_default());
        // пинг по туннелю до тех, кто в сети; список спрашивает только открытое окно, так что в фоне это не крутится
        if let (Some(list), true) = (fetched.as_mut(), running) {
            std::thread::scope(|s| {
                for p in list.iter_mut().filter(|p| p.online && p.ip != dev.ip) {
                    s.spawn(move || {
                        p.rtt = p.ip.parse().ok().and_then(|ip| plat::ping(ip, Duration::from_millis(1500)));
                    });
                }
            });
        }
        let mut i = self.inner.lock().unwrap();
        let Some(n) = i.nets.get_mut(slug) else { return Ok(cached) };
        n.peers_ts = now();
        if let Some(p) = fetched {
            n.peers = p;
        }
        Ok(n.peers.clone())
    }

    // ----------------------------------------------------- подключение и выход

    fn set_want(&self, slug: &str, up: bool) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let n = i.nets.get_mut(slug).ok_or_else(|| Self::no_net(slug))?;
        n.set.want_up = up;
        n.error = None;
        n.fails = 0;
        n.next_try = None;
        let set = n.set.clone();
        drop(i);
        self.save_settings(slug, &set);
        self.logs.note(format!("[{slug}] {}", if up { tr("подключение включено", "connection enabled") } else { tr("подключение выключено", "connection disabled") }));
        Ok(())
    }

    fn set_dns(&self, slug: &str, on: bool) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let n = i.nets.get_mut(slug).ok_or_else(|| Self::no_net(slug))?;
        n.set.dns_on = on;
        let set = n.set.clone();
        drop(i);
        self.save_settings(slug, &set);
        self.logs.note(format!("[{slug}] {}", if on { tr("DNS сети включён", "network DNS enabled") } else { tr("DNS сети выключен", "network DNS disabled") }));
        Ok(())
    }

    fn leave(&self, slug: &str) -> Result<(), String> {
        let _op = self.op.lock().unwrap();
        let mut i = self.inner.lock().unwrap();
        let mut n = i.nets.remove(slug).ok_or_else(|| Self::no_net(slug))?;
        if let Some(c) = n.child.take() {
            c.stop();
        }
        i.firewall_sig = None;
        drop(i);
        let _ = std::fs::remove_dir_all(self.net_dir(slug));
        // панель отзывает сертификат и освобождает запись; без связи выйти всё равно можно
        let (server, token) = (n.dev.server.clone(), n.dev.token.clone());
        let logs = self.logs.clone();
        std::thread::spawn(move || {
            if let Err(e) = api::post(&server, "/v1/leave", Some(&token), json!({})) {
                logs.note(trf!("панель не узнала о выходе: {}", "could not tell the panel about leaving: {}", e));
            }
        });
        self.logs.note(trf!("устройство вышло из сети «{}», её ключ и сертификат удалены", "left network \"{}\", its key and certificate were deleted", n.dev.network.title));
        Ok(())
    }

    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let mut i = self.inner.lock().unwrap();
        for n in i.nets.values_mut() {
            if let Some(c) = n.child.take() {
                c.stop();
            }
        }
        // правила DNS указывают в туннель, без него они только мешают
        if i.dns_sig.take().is_some() {
            let _ = plat::set_dns(&[]);
        }
    }

    // ------------------------------------------------------------- фоновые циклы

    /// Следит, чтобы nebula каждой сети работала, когда пользователь хочет быть в этой сети.
    fn supervise(self: Arc<Self>) {
        while !self.stopping.load(Ordering::SeqCst) {
            self.firewall_tick();
            self.supervise_tick();
            self.dns_tick();
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    fn firewall_tick(&self) {
        let (sig, cidrs) = {
            let i = self.inner.lock().unwrap();
            let cidrs: Vec<String> = i.nets.values().map(|n| n.dev.network.cidr.clone()).collect();
            let sig = cidrs.join(",");
            if i.firewall_sig.as_deref() == Some(sig.as_str()) {
                return;
            }
            (sig, cidrs)
        };
        let b = nebula::bins(&self.dir);
        if cidrs.is_empty() {
            plat::remove_firewall();
        } else {
            plat::firewall(&b.nebula, b.own, &cidrs);
        }
        self.inner.lock().unwrap().firewall_sig = Some(sig);
    }

    /// Запросы к зонам сети идут на её DNS, только пока сеть подключена: иначе сервер всё равно недоступен.
    fn dns_tick(&self) {
        let rules: Vec<plat::DnsRule> = {
            let i = self.inner.lock().unwrap();
            i.nets
                .iter()
                .filter(|(_, n)| n.state == ConnState::Running && n.set.dns_on)
                .filter_map(|(slug, n)| {
                    let d = n.set.dns.as_ref()?;
                    let servers = d.all();
                    let ok = servers.iter().all(|s| s.parse::<std::net::Ipv4Addr>().is_ok()) && !d.domains.is_empty() && d.domains.iter().all(|z| common::valid_zone(z));
                    ok.then(|| plat::DnsRule { slug: slug.clone(), tun: n.tun.clone(), servers, domains: d.domains.clone() })
                })
                .collect()
        };
        let sig = format!("{rules:?}");
        if self.inner.lock().unwrap().dns_sig.as_deref() == Some(sig.as_str()) {
            return;
        }
        if let Err(e) = plat::set_dns(&rules) {
            self.logs.note(trf!("DNS сети не настроен: {}", "network DNS not set up: {}", e));
        }
        for r in &rules {
            self.logs.note(format!("[{}] {}", r.slug, trf!("имена {} спрашиваются у {}", "names in {} are resolved by {}", r.domains.iter().map(|d| format!("*.{d}")).collect::<Vec<_>>().join(", "), r.servers.join(", "))));
        }
        self.inner.lock().unwrap().dns_sig = Some(sig);
    }

    fn supervise_tick(self: &Arc<Self>) {
        let mut i = self.inner.lock().unwrap();
        let busy = i.busy.is_some();
        let bins = nebula::bins(&self.dir);
        for (slug, n) in i.nets.iter_mut() {
            if let Some(c) = n.child.as_mut() {
                if let Some(code) = c.exited() {
                    n.child = None;
                    n.fails += 1;
                    let wait = [2u64, 5, 15, 30, 60][(n.fails as usize - 1).min(4)];
                    n.next_try = Some(Instant::now() + Duration::from_secs(wait));
                    let why = self.logs.last_error(slug).unwrap_or_else(|| trf!("код выхода {}", "exit code {}", code));
                    let why = why.trim_end_matches('.').to_string();
                    self.logs.note(format!("[{slug}] {}", trf!("Nebula остановилась: {}. Перезапуск через {} с", "Nebula stopped: {}. Restarting in {} s", why, wait)));
                    n.error = Some(why);
                    n.state = ConnState::Error;
                } else if !c.net_ready && c.started.elapsed() > Duration::from_secs(4) {
                    c.net_ready = true;
                    n.state = ConnState::Running;
                    n.fails = 0;
                    n.error = None;
                    let tun = n.tun.clone();
                    std::thread::spawn(move || plat::set_private(&tun));
                }
            }
            if !n.set.want_up || busy {
                if let Some(c) = n.child.take() {
                    c.stop();
                    self.logs.note(format!("[{slug}] {}", tr("Nebula остановлена", "Nebula stopped")));
                }
                if !n.set.want_up {
                    n.state = ConnState::Stopped;
                }
                continue;
            }
            if n.child.is_none() && n.next_try.map_or(true, |t| Instant::now() >= t) {
                match Running::spawn(&bins, &self.net_dir(slug), slug, self.logs.clone()) {
                    Ok(c) => {
                        self.logs.note(format!("[{slug}] {}", tr("Nebula запущена", "Nebula started")));
                        n.child = Some(c);
                        n.state = ConnState::Starting;
                    }
                    Err(e) => {
                        n.fails += 1;
                        n.next_try = Some(Instant::now() + Duration::from_secs(30));
                        self.logs.note(format!("[{slug}] {e}"));
                        n.error = Some(e);
                        n.state = ConnState::Error;
                    }
                }
            }
        }
    }

    /// Раз в 6 часов каждая сеть продлевает сертификат и забирает свежий конфиг, при ошибке через 15 минут.
    fn sync_loop(self: Arc<Self>) {
        while !self.stopping.load(Ordering::SeqCst) {
            let all = self.sync_all.swap(false, Ordering::SeqCst);
            let due: Vec<String> = {
                let i = self.inner.lock().unwrap();
                i.nets.iter().filter(|(_, n)| all || Instant::now() >= n.next_sync).map(|(s, _)| s.clone()).collect()
            };
            for s in due {
                let _ = self.sync(&s);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    pub fn start(self: &Arc<Self>) {
        // если раньше стояла PowerShell-версия, её служба держит тот же адаптер
        if !self.inner.lock().unwrap().nets.is_empty() && plat::migrate_script_install(&self.dir) {
            self.logs.note(tr("старая служба Nebula от PowerShell-скрипта убрана", "removed the old Nebula service from the PowerShell script"));
        }
        let a = self.clone();
        std::thread::spawn(move || a.supervise());
        let b = self.clone();
        std::thread::spawn(move || b.sync_loop());
        self.after_update();
        let c = self.clone();
        std::thread::spawn(move || c.update_loop());
    }

    // ------------------------------------------------------------- обновления

    fn update_check(&self) -> Result<(), String> {
        let pre = self.upd.lock().unwrap().saved.prerelease;
        let r = update::check(pre);
        let mut u = self.upd.lock().unwrap();
        u.saved.last_check = Some(now());
        u.saved.save(&self.dir);
        match r {
            Ok(rel) => {
                if let Some(v) = rel.as_ref().map(|r| r.version.clone()) {
                    if u.available.as_ref().map(|a| a.version != v).unwrap_or(true) {
                        self.logs.note(trf!("доступна версия {}", "version {} is available", v));
                    }
                }
                u.available = rel;
                u.error = None;
                Ok(())
            }
            Err(e) => {
                u.error = Some(e.clone());
                Err(e)
            }
        }
    }

    fn update_install(self: &Arc<Self>) -> Result<(), String> {
        let rel = {
            let mut u = self.upd.lock().unwrap();
            let rel = u.available.clone().ok_or(tr("обновлений нет, сначала проверьте", "no update found, check first"))?;
            if !rel.can_install() {
                return Err(tr("эту версию нужно поставить вручную со страницы релиза", "install this version manually from the release page").into());
            }
            if u.installing {
                return Ok(());
            }
            u.installing = true;
            rel
        };
        let me = self.clone();
        std::thread::spawn(move || {
            me.logs.note(trf!("скачиваю обновление {}", "downloading update {}", rel.version));
            let r = update::fetch(&rel, &me.dir).and_then(|path| plat::run_installer(&path, &me.dir));
            match r {
                Ok(()) => me.logs.note(trf!("ставлю версию {}, служба перезапустится", "installing version {}, the service will restart", rel.version)),
                Err(e) => {
                    me.logs.note(trf!("обновление не удалось: {}", "update failed: {}", e));
                    let mut u = me.upd.lock().unwrap();
                    u.installing = false;
                    u.fail = Some((rel.version.clone(), e));
                }
            }
        });
        Ok(())
    }

    /// Служба запустилась после тихого обновления: вернуть значок в трей, убрать установщик.
    fn after_update(&self) {
        let dir = self.dir.join("updates");
        if !dir.exists() {
            return;
        }
        if std::fs::remove_file(dir.join("relaunch-tray")).is_ok() {
            self.logs.note(trf!("обновлено до {}", "updated to {}", env!("CARGO_PKG_VERSION")));
            plat::relaunch_tray(&self.logs);
        }
        // установщик ещё может работать (он и запустил службу), тогда уберём в следующий раз
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn update_loop(self: Arc<Self>) {
        // не мешаем старту сетей
        for _ in 0..90 {
            if self.stopping.load(Ordering::SeqCst) || self.update_now.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        while !self.stopping.load(Ordering::SeqCst) {
            let forced = self.update_now.swap(false, Ordering::SeqCst);
            let (due, mode) = {
                let u = self.upd.lock().unwrap();
                (update::due(&u.saved), u.saved.mode)
            };
            if (due || forced) && mode != UpdateMode::Off && self.update_check().is_ok() && mode == UpdateMode::Auto {
                let u = self.upd.lock().unwrap();
                let go = !u.installing && u.available.as_ref().map(|r| r.can_install()).unwrap_or(false);
                drop(u);
                if go {
                    let _ = self.update_install();
                }
            }
            for _ in 0..60 {
                if self.stopping.load(Ordering::SeqCst) || self.update_now.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "pki:\n  ca: /x/ca.crt\n\ntun:\n  dev: nebula1\n  drop_multicast: true\n\nsshd:\n  enabled: true\n";

    #[test]
    fn tun_name() {
        assert_eq!(tun_of(CFG), "nebula1");
    }

    #[test]
    fn tun_rename_on_clash() {
        let c = unique_tun(CFG, "friends", &["nebula1".into()]);
        assert_eq!(tun_of(&c), "nb-friends");
        assert!(c.contains("drop_multicast: true") && c.contains("sshd:"));
        assert_eq!(unique_tun(CFG, "friends", &["other".into()]), CFG);
        let c2 = unique_tun(CFG, "friends", &["nebula1".into(), "nb-friends".into()]);
        assert_eq!(tun_of(&c2), "nb2-friends");
    }
}
