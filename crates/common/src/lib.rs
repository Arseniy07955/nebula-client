//! Общее для службы и приложения: пути, протокол канала и клиент к нему.
//!
//! Протокол простой: клиент открывает именованный канал, пишет одну строку JSON с запросом
//! и читает одну строку JSON с ответом. На каждый запрос своё соединение.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const PIPE_NAME: &str = r"\\.\pipe\nebula-client";
pub const SERVICE_NAME: &str = "NebulaClient";
pub const SERVICE_DISPLAY: &str = "Nebula Client";
pub const CLIENT_VERSION: &str = concat!("nebula-client/", env!("CARGO_PKG_VERSION"));

/// Адрес панели по умолчанию. Для своей сборки задайте NEBULA_CLIENT_SERVER при компиляции.
pub const DEFAULT_SERVER: &str = match option_env!("NEBULA_CLIENT_SERVER") {
    Some(s) => s,
    None => "https://cabinet.senkapopka.ru/nebula",
};

/// Папка данных: ключ, сертификат, конфиг, токен. Та же, что у PowerShell-скрипта.
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("NEBULA_CLIENT_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    PathBuf::from(base).join("NebulaClient")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    Peers,
    Logs,
    Enroll { server: String, name: String, invite: Option<String> },
    Connect,
    Disconnect,
    Sync,
    /// Выйти из сети: остановить Nebula и стереть ключ, сертификат и токен.
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnState {
    #[default]
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Status {
    pub enrolled: bool,
    pub server: String,
    pub name: String,
    pub ip: String,
    pub network: String,
    pub cidr: String,
    pub state: ConnState,
    /// Пользователь хочет быть подключённым (переживает перезагрузку).
    pub want_up: bool,
    pub error: Option<String>,
    /// Что служба делает прямо сейчас: «Скачиваю Nebula», «Подключаюсь» и т. п.
    pub busy: Option<String>,
    pub cert_not_after: Option<i64>,
    pub last_sync: Option<i64>,
    pub sync_error: Option<String>,
    pub service_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Peer {
    pub name: String,
    pub ip: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<Vec<Peer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<Vec<String>>,
}

impl Response {
    pub fn ok() -> Self {
        Response { ok: true, ..Default::default() }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Response { ok: false, error: Some(msg.into()), ..Default::default() }
    }
}

#[derive(Debug)]
pub enum IpcError {
    /// Служба не запущена или канала нет.
    NoService,
    Other(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::NoService => write!(f, "служба Nebula Client не запущена"),
            IpcError::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Отправить запрос службе. Ответ на enroll может идти долго (скачивание Nebula), таймаута на чтение нет.
pub fn call(req: &Request) -> Result<Response, IpcError> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let file = loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(PIPE_NAME) {
            Ok(f) => break f,
            Err(e) => {
                // 231 ERROR_PIPE_BUSY: все экземпляры заняты, ждём следующий
                if e.raw_os_error() == Some(231) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                return Err(match e.raw_os_error() {
                    Some(2) | Some(231) => IpcError::NoService,
                    _ => IpcError::Other(format!("канал службы: {e}")),
                });
            }
        }
    };
    let mut line = serde_json::to_string(req).map_err(|e| IpcError::Other(e.to_string()))?;
    line.push('\n');
    let mut w = &file;
    w.write_all(line.as_bytes()).map_err(|e| IpcError::Other(format!("запись в канал: {e}")))?;
    let mut buf = String::new();
    BufReader::new(&file).read_line(&mut buf).map_err(|e| IpcError::Other(format!("чтение из канала: {e}")))?;
    serde_json::from_str(&buf).map_err(|e| IpcError::Other(format!("ответ службы: {e}")))
}

/// Имя устройства по умолчанию из имени компьютера: латиница, цифры, точка, дефис, подчёркивание.
pub fn default_device_name() -> String {
    let raw = std::env::var("COMPUTERNAME").unwrap_or_default().to_lowercase();
    let s: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "pc".into() } else { s }
}

pub fn valid_device_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 63 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(c))
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
