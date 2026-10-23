//! Состояние службы: устройство, nebula.exe, синхронизация с панелью.
//!
//! Долгие операции (вход, синхронизация) идут под `op`, а `inner` блокируется только коротко,
//! чтобы окно могло спрашивать статус, пока, например, скачивается Nebula.

use crate::api::{self, NetInfo, Payload};
use crate::logs::Logs;
use crate::nebula::{self, Running};
use crate::sys;
use common::{now, ConnState, Peer, Request, Response, Status, CLIENT_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SYNC_EVERY: Duration = Duration::from_secs(6 * 3600);
const SYNC_RETRY: Duration = Duration::from_secs(15 * 60);
const PEERS_TTL: i64 = 15;

/// device.json: тот же формат, что у PowerShell-скрипта, поэтому старая установка подхватывается.
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Settings {
    #[serde(default = "yes")]
    want_up: bool,
    #[serde(default)]
    cert_not_after: Option<i64>,
    #[serde(default)]
    last_sync: Option<i64>,
}

fn yes() -> bool {
    true
}

#[derive(Default)]
struct Inner {
    dev: Option<Device>,
    set: Settings,
    child: Option<Running>,
    state: ConnState,
    error: Option<String>,
    busy: Option<String>,
    sync_error: Option<String>,
    peers: Vec<Peer>,
    peers_ts: i64,
    fails: u32,
    next_try: Option<Instant>,
    firewall_done: bool,
}

pub struct Core {
    dir: PathBuf,
    inner: Mutex<Inner>,
    op: Mutex<()>,
    pub logs: Arc<Logs>,
    sync_now: AtomicBool,
    stopping: AtomicBool,
}

fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    let text = text.replace("\r\n", "\n");
    if std::fs::read_to_string(path).map(|old| old == text).unwrap_or(false) {
        return Ok(false);
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

impl Core {
    pub fn new(dir: PathBuf) -> Arc<Core> {
        let logs = Arc::new(Logs::default());
        let dev: Option<Device> = std::fs::read(dir.join("device.json")).ok().and_then(|b| {
            let b = b.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&b).to_vec();
            serde_json::from_slice(&b).ok()
        });
        let set: Settings = std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Settings { want_up: true, ..Default::default() });
        logs.note(match &dev {
            Some(d) => format!("служба запущена, устройство {} {} в сети «{}»", d.name, d.ip, d.network.title),
            None => "служба запущена, устройство ещё не подключено к сети".into(),
        });
        Arc::new(Core {
            dir,
            inner: Mutex::new(Inner { dev, set, ..Default::default() }),
            op: Mutex::new(()),
            logs,
            sync_now: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        })
    }

    fn save_settings(&self, s: &Settings) {
        let _ = std::fs::write(self.dir.join("settings.json"), serde_json::to_vec_pretty(s).unwrap_or_default());
    }

    fn set_busy(&self, b: Option<&str>) {
        self.inner.lock().unwrap().busy = b.map(String::from);
        if let Some(b) = b {
            self.logs.note(b);
        }
    }

    pub fn status(&self) -> Status {
        let i = self.inner.lock().unwrap();
        let d = i.dev.clone();
        Status {
            enrolled: d.is_some(),
            server: d.as_ref().map(|d| d.server.clone()).unwrap_or_default(),
            name: d.as_ref().map(|d| d.name.clone()).unwrap_or_default(),
            ip: d.as_ref().map(|d| d.ip.clone()).unwrap_or_default(),
            network: d.as_ref().map(|d| d.network.title.clone()).unwrap_or_default(),
            cidr: d.as_ref().map(|d| d.network.cidr.clone()).unwrap_or_default(),
            state: i.state,
            want_up: i.set.want_up,
            error: i.error.clone(),
            busy: i.busy.clone(),
            cert_not_after: i.set.cert_not_after,
            last_sync: i.set.last_sync,
            sync_error: i.sync_error.clone(),
            service_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub fn dispatch(self: &Arc<Self>, req: Request) -> Response {
        let r = match req {
            Request::Status => Ok(Response { status: Some(self.status()), ..Response::ok() }),
            Request::Peers => Ok(Response { peers: Some(self.peers()), ..Response::ok() }),
            Request::Logs => Ok(Response { logs: Some(self.logs.tail(300)), ..Response::ok() }),
            Request::Enroll { server, name, invite } => self.enroll(&server, &name, invite.as_deref()).map(|_| Response::ok()),
            Request::Connect => {
                self.set_want(true);
                Ok(Response::ok())
            }
            Request::Disconnect => {
                self.set_want(false);
                Ok(Response::ok())
            }
            Request::Sync => self.sync().map(|_| Response::ok()),
            Request::Leave => self.leave().map(|_| Response::ok()),
        };
        let mut resp = r.unwrap_or_else(Response::err);
        if resp.status.is_none() {
            resp.status = Some(self.status());
        }
        resp
    }

    // ------------------------------------------------------------ вход в сеть

    fn enroll(self: &Arc<Self>, server: &str, name: &str, invite: Option<&str>) -> Result<(), String> {
        let _op = self.op.try_lock().map_err(|_| "служба уже занята, подождите".to_string())?;
        if self.inner.lock().unwrap().dev.is_some() {
            return Err("Это устройство уже подключено. Сначала выйдите из сети".into());
        }
        let server = api::check_server(server)?;
        let name = name.trim();
        if !common::valid_device_name(name) {
            return Err("Имя устройства: латиница, цифры, точка, дефис, подчёркивание, до 63 символов".into());
        }
        let r = self.enroll_steps(&server, name, invite.map(str::trim).filter(|s| !s.is_empty()));
        self.set_busy(None);
        match &r {
            Ok(()) => self.logs.note("устройство подключено к сети"),
            Err(e) => self.logs.note(format!("вход не удался: {e}")),
        }
        r
    }

    fn enroll_steps(self: &Arc<Self>, server: &str, name: &str, invite: Option<&str>) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        sys::protect_dir(&self.dir);
        if sys::migrate_script_install(&self.dir) {
            self.logs.note("старая служба Nebula от PowerShell-скрипта убрана, теперь Nebula запускает Nebula Client");
        }
        nebula::ensure_binaries(&self.dir, &|s| self.set_busy(Some(s)))?;
        self.set_busy(Some("Создаю ключ устройства"));
        let public_key = nebula::ensure_key(&self.dir)?;
        self.set_busy(Some("Получаю сертификат"));
        let body = json!({
            "invite": invite.unwrap_or(""),
            "name": name,
            "public_key": public_key,
            "conf_dir": self.dir.to_string_lossy().replace('\\', "/"),
            "client": CLIENT_VERSION,
        });
        let v = api::post(server, "/v1/enroll", None, body).map_err(|e| e.to_string())?;
        let p = api::payload(v).map_err(|e| e.to_string())?;
        let token = p.token.clone().ok_or("панель не выдала токен устройства")?;
        self.write_files(&p)?;
        nebula::test_config(&self.dir)?;
        let dev = Device {
            server: server.into(),
            token,
            name: p.name.clone(),
            ip: p.ip.clone(),
            network: p.network.clone(),
            enrolled: now().to_string(),
        };
        std::fs::write(self.dir.join("device.json"), serde_json::to_vec_pretty(&dev).unwrap()).map_err(|e| e.to_string())?;
        let mut i = self.inner.lock().unwrap();
        i.set = Settings { want_up: true, cert_not_after: Some(p.not_after), last_sync: Some(now()) };
        self.save_settings(&i.set);
        i.dev = Some(dev);
        i.peers = p.peers;
        i.peers_ts = now();
        i.error = None;
        i.fails = 0;
        i.next_try = None;
        i.firewall_done = false;
        Ok(())
    }

    fn write_files(&self, p: &Payload) -> Result<bool, String> {
        let mut changed = false;
        for (f, text) in [("host.crt", &p.cert), ("ca.crt", &p.ca), ("config.yml", &p.config)] {
            changed |= write_if_changed(&self.dir.join(f), text)?;
        }
        if let Some(k) = &p.ssh_host_key {
            changed |= write_if_changed(&self.dir.join("ssh_host_ed25519_key"), k)?;
        }
        Ok(changed)
    }

    // ---------------------------------------------------------- синхронизация

    pub fn sync(&self) -> Result<(), String> {
        let _op = self.op.lock().unwrap();
        let Some(dev) = self.inner.lock().unwrap().dev.clone() else {
            return Err("устройство не подключено к сети".into());
        };
        let body = json!({ "conf_dir": self.dir.to_string_lossy().replace('\\', "/"), "client": CLIENT_VERSION });
        let r = api::post(&dev.server, "/v1/sync", Some(&dev.token), body).and_then(api::payload);
        let p = match r {
            Ok(p) => p,
            Err(e) => {
                let msg = match e.code() {
                    401 | 403 => format!("панель отключила устройство: {e}"),
                    _ => e.to_string(),
                };
                self.logs.note(format!("синхронизация не удалась: {msg}"));
                self.inner.lock().unwrap().sync_error = Some(msg.clone());
                return Err(msg);
            }
        };
        let changed = self.write_files(&p)?;
        if changed {
            nebula::test_config(&self.dir)?;
        }
        let mut i = self.inner.lock().unwrap();
        i.set.cert_not_after = Some(p.not_after);
        i.set.last_sync = Some(now());
        self.save_settings(&i.set);
        i.sync_error = None;
        i.peers = p.peers;
        i.peers_ts = now();
        if changed {
            self.logs.note("сертификат или конфиг обновились, перезапускаю Nebula");
            if let Some(c) = i.child.take() {
                c.stop();
            }
            i.next_try = None;
        } else {
            self.logs.note("синхронизация: без изменений");
        }
        Ok(())
    }

    fn peers(&self) -> Vec<Peer> {
        let (dev, fresh, cached) = {
            let i = self.inner.lock().unwrap();
            (i.dev.clone(), now() - i.peers_ts < PEERS_TTL, i.peers.clone())
        };
        let Some(dev) = dev else { return vec![] };
        if fresh {
            return cached;
        }
        match api::get(&dev.server, "/v1/peers", Some(&dev.token)) {
            Ok(v) => {
                let peers: Vec<Peer> = serde_json::from_value(v["peers"].clone()).unwrap_or_default();
                let mut i = self.inner.lock().unwrap();
                i.peers = peers.clone();
                i.peers_ts = now();
                peers
            }
            Err(_) => {
                self.inner.lock().unwrap().peers_ts = now();
                cached
            }
        }
    }

    // ----------------------------------------------------- подключение и выход

    fn set_want(&self, up: bool) {
        let mut i = self.inner.lock().unwrap();
        i.set.want_up = up;
        self.save_settings(&i.set);
        i.error = None;
        i.fails = 0;
        i.next_try = None;
        self.logs.note(if up { "подключение включено" } else { "подключение выключено" });
    }

    fn leave(&self) -> Result<(), String> {
        let _op = self.op.lock().unwrap();
        let mut i = self.inner.lock().unwrap();
        if let Some(c) = i.child.take() {
            c.stop();
        }
        for f in ["device.json", "settings.json", "host.key", "host.pub", "host.crt", "ca.crt", "config.yml", "ssh_host_ed25519_key"] {
            let _ = std::fs::remove_file(self.dir.join(f));
        }
        sys::remove_firewall();
        *i = Inner { set: Settings { want_up: true, ..Default::default() }, ..Default::default() };
        self.logs.note("устройство вышло из сети, ключ и сертификат удалены");
        Ok(())
    }

    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(c) = self.inner.lock().unwrap().child.take() {
            c.stop();
        }
    }

    // ------------------------------------------------------------- фоновые циклы

    /// Следит, чтобы nebula.exe работала, когда пользователь хочет быть в сети.
    pub fn supervise(self: Arc<Self>) {
        while !self.stopping.load(Ordering::SeqCst) {
            self.supervise_tick();
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    fn supervise_tick(self: &Arc<Self>) {
        let mut i = self.inner.lock().unwrap();
        if let Some(c) = i.child.as_mut() {
            if let Some(code) = c.exited() {
                i.child = None;
                i.fails += 1;
                let wait = [2u64, 5, 15, 30, 60][(i.fails as usize - 1).min(4)];
                i.next_try = Some(Instant::now() + Duration::from_secs(wait));
                let why = self.logs.last_error().unwrap_or_else(|| format!("код выхода {code}"));
                self.logs.note(format!("Nebula остановилась: {why}. Перезапуск через {wait} с"));
                i.error = Some(why);
                i.state = ConnState::Error;
            } else if !c.net_ready && c.started.elapsed() > Duration::from_secs(4) {
                c.net_ready = true;
                i.state = ConnState::Running;
                i.fails = 0;
                i.error = None;
                let dev = i.dev.as_ref().map(|d| d.network.dev.clone()).unwrap_or_default();
                std::thread::spawn(move || {
                    sys::set_private(&dev);
                });
            }
        }
        let can_run = i.set.want_up && i.dev.is_some() && i.busy.is_none();
        if !can_run {
            if let Some(c) = i.child.take() {
                c.stop();
                self.logs.note("Nebula остановлена");
            }
            if i.state != ConnState::Error || !i.set.want_up {
                i.state = ConnState::Stopped;
            }
            return;
        }
        if i.child.is_none() && i.next_try.map_or(true, |t| Instant::now() >= t) {
            if !i.firewall_done {
                let cidr = i.dev.as_ref().map(|d| d.network.cidr.clone()).unwrap_or_default();
                sys::firewall(&nebula::nebula_exe(&self.dir), &cidr);
                i.firewall_done = true;
            }
            match Running::spawn(&self.dir, self.logs.clone()) {
                Ok(c) => {
                    self.logs.note("Nebula запущена");
                    i.child = Some(c);
                    i.state = ConnState::Starting;
                }
                Err(e) => {
                    i.fails += 1;
                    i.next_try = Some(Instant::now() + Duration::from_secs(30));
                    self.logs.note(&e);
                    i.error = Some(e);
                    i.state = ConnState::Error;
                }
            }
        }
    }

    /// Раз в 6 часов продлевает сертификат и забирает свежий конфиг, при ошибке повторяет через 15 минут.
    pub fn sync_loop(self: Arc<Self>) {
        let mut next = Instant::now() + Duration::from_secs(10);
        while !self.stopping.load(Ordering::SeqCst) {
            if self.sync_now.swap(false, Ordering::SeqCst) || Instant::now() >= next {
                let enrolled = self.inner.lock().unwrap().dev.is_some();
                next = Instant::now() + if !enrolled { SYNC_EVERY } else if self.sync().is_ok() { SYNC_EVERY } else { SYNC_RETRY };
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    pub fn start(self: &Arc<Self>) {
        // если раньше стояла PowerShell-версия, её служба держит тот же адаптер
        if self.inner.lock().unwrap().dev.is_some() && sys::migrate_script_install(&self.dir) {
            self.logs.note("старая служба Nebula от PowerShell-скрипта убрана, теперь Nebula запускает Nebula Client");
        }
        let a = self.clone();
        std::thread::spawn(move || a.supervise());
        let b = self.clone();
        std::thread::spawn(move || b.sync_loop());
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}
