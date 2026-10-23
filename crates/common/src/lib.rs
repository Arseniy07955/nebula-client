//! Общее для службы и приложения: пути, протокол, клиент к службе и локализация.
//!
//! Протокол простой: клиент подключается к службе (именованный канал в Windows, unix-сокет в Linux),
//! пишет одну строку JSON с запросом и читает одну строку JSON с ответом. На каждый запрос своё соединение.

mod i18n;
pub use i18n::{is_en, lang, tr, with_lang};

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

pub const SERVICE_NAME: &str = "NebulaClient";
pub const SERVICE_DISPLAY: &str = "Nebula Client";
pub const CLIENT_VERSION: &str = concat!("nebula-client/", env!("CARGO_PKG_VERSION"));

/// Канал службы. NEBULA_CLIENT_SOCKET меняет имя: для второй копии при отладке.
#[cfg(windows)]
pub fn pipe_name() -> String {
    match std::env::var("NEBULA_CLIENT_SOCKET") {
        Ok(s) if s.starts_with(r"\\") => s,
        Ok(s) if !s.is_empty() => format!(r"\\.\pipe\{s}"),
        _ => r"\\.\pipe\nebula-client".into(),
    }
}

/// Адрес панели по умолчанию. Для своей сборки задайте NEBULA_CLIENT_SERVER при компиляции.
pub const DEFAULT_SERVER: &str = match option_env!("NEBULA_CLIENT_SERVER") {
    Some(s) => s,
    None => "https://svyaz.msk1.catpaws.ru",
};

/// Папка данных службы: Nebula, а в networks/<код сети> ключ, сертификат, конфиг и токен каждой сети.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("NEBULA_CLIENT_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
        PathBuf::from(base).join("NebulaClient")
    }
    #[cfg(not(windows))]
    {
        // systemd: StateDirectory=nebula-client
        std::env::var_os("STATE_DIRECTORY").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/var/lib/nebula-client"))
    }
}

#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    if let Some(s) = std::env::var_os("NEBULA_CLIENT_SOCKET") {
        return PathBuf::from(s);
    }
    PathBuf::from("/run/nebula-client/nebula-client.sock")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    Peers { net: String },
    Logs,
    /// Какие сети на панели пускают без инвайта.
    OpenNets { server: String },
    /// net: код сети со свободной регистрацией, если на панели их несколько.
    Enroll {
        server: String,
        name: String,
        invite: Option<String>,
        #[serde(default)]
        net: Option<String>,
    },
    Connect { net: String },
    Disconnect { net: String },
    /// Без net синхронизируются все сети.
    Sync { net: Option<String> },
    /// Выйти из сети: остановить её Nebula и стереть ключ, сертификат и токен.
    Leave { net: String },
}

/// Запрос вместе с языком окна, чтобы служба отвечала на нём же.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(flatten)]
    pub req: Request,
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
pub struct NetStatus {
    /// Код сети на панели, по нему к ней обращаются все команды.
    pub slug: String,
    pub title: String,
    pub cidr: String,
    pub server: String,
    pub name: String,
    pub ip: String,
    /// Имя адаптера Nebula на этом устройстве.
    pub dev: String,
    pub state: ConnState,
    /// Пользователь хочет быть в этой сети (переживает перезагрузку).
    pub want_up: bool,
    pub error: Option<String>,
    pub cert_not_after: Option<i64>,
    pub last_sync: Option<i64>,
    pub sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Status {
    pub networks: Vec<NetStatus>,
    /// Что служба делает прямо сейчас: «Скачиваю Nebula», «Получаю сертификат» и т. п.
    pub busy: Option<String>,
    pub service_version: String,
    pub os: String,
}

impl Status {
    pub fn net(&self, slug: &str) -> Option<&NetStatus> {
        self.networks.iter().find(|n| n.slug == slug)
    }
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
pub struct OpenNet {
    pub slug: String,
    pub title: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open: Option<Vec<OpenNet>>,
    /// Код сети, в которую только что вошли.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
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
    /// Служба не запущена или к ней нет доступа.
    NoService,
    Denied,
    Other(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::NoService => write!(f, "{}", tr("служба Nebula Client не запущена", "the Nebula Client service is not running")),
            IpcError::Denied => write!(
                f,
                "{}",
                tr("нет доступа к службе Nebula Client (нужна группа nebula-client или root)", "no access to the Nebula Client service (join the nebula-client group or use root)")
            ),
            IpcError::Other(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(windows)]
fn connect() -> Result<std::fs::File, IpcError> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(pipe_name()) {
            Ok(f) => return Ok(f),
            // 231 ERROR_PIPE_BUSY: все экземпляры заняты, ждём следующий
            Err(e) if e.raw_os_error() == Some(231) && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                return Err(match e.raw_os_error() {
                    Some(2) | Some(231) => IpcError::NoService,
                    Some(5) => IpcError::Denied,
                    _ => IpcError::Other(e.to_string()),
                })
            }
        }
    }
}

#[cfg(unix)]
fn connect() -> Result<std::os::unix::net::UnixStream, IpcError> {
    std::os::unix::net::UnixStream::connect(socket_path()).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => IpcError::NoService,
        std::io::ErrorKind::PermissionDenied => IpcError::Denied,
        _ => IpcError::Other(e.to_string()),
    })
}

/// Отправить запрос службе. Ответ на enroll может идти долго (скачивание Nebula), таймаута на чтение нет.
pub fn call(req: &Request) -> Result<Response, IpcError> {
    let conn = connect()?;
    let env = Envelope { lang: Some(lang().into()), req: req.clone() };
    let mut line = serde_json::to_string(&env).map_err(|e| IpcError::Other(e.to_string()))?;
    line.push('\n');
    (&conn).write_all(line.as_bytes()).map_err(|e| IpcError::Other(e.to_string()))?;
    let mut buf = String::new();
    BufReader::new(&conn).read_line(&mut buf).map_err(|e| IpcError::Other(e.to_string()))?;
    serde_json::from_str(&buf).map_err(|e| IpcError::Other(e.to_string()))
}

/// Имя устройства по умолчанию из имени компьютера: латиница, цифры, точка, дефис, подчёркивание.
pub fn default_device_name() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let s: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "pc".into() } else { s }
}

pub fn valid_device_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 63 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(c))
}

/// Код сети годится как имя папки.
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
