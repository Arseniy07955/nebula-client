//! HTTP к панели: /v1/enroll, /v1/sync, /v1/peers.

use common::{tr, trf, NetDns, Peer, CLIENT_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetInfo {
    pub slug: String,
    pub title: String,
    pub cidr: String,
    pub dev: String,
}

/// Что панель отдаёт на enroll и sync.
#[derive(Debug, Deserialize)]
pub struct Payload {
    pub name: String,
    pub ip: String,
    pub network: NetInfo,
    pub cert: String,
    pub ca: String,
    pub config: String,
    pub not_after: i64,
    #[serde(default)]
    pub ssh_host_key: Option<String>,
    #[serde(default)]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub dns: Option<NetDns>,
}

#[derive(Debug)]
pub enum ApiErr {
    /// Панель ответила ошибкой: код и текст из поля error.
    Http(u16, String),
    /// Сеть, TLS, таймаут.
    Net(String),
}

impl ApiErr {
    pub fn code(&self) -> u16 {
        match self {
            ApiErr::Http(c, _) => *c,
            ApiErr::Net(_) => 0,
        }
    }
}

impl std::fmt::Display for ApiErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiErr::Http(_, m) => write!(f, "{m}"),
            ApiErr::Net(m) => write!(f, "{}", trf!("нет связи с панелью: {}", "cannot reach the panel: {}", m)),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent(CLIENT_VERSION)
        .build()
}

fn handle(r: Result<ureq::Response, ureq::Error>) -> Result<Value, ApiErr> {
    match r {
        Ok(resp) => resp.into_json().map_err(|e| ApiErr::Net(trf!("непонятный ответ: {}", "unexpected answer: {}", e))),
        Err(ureq::Error::Status(code, resp)) => {
            let msg = resp
                .into_json::<Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .unwrap_or_else(|| trf!("панель ответила {}", "the panel answered {}", code));
            Err(ApiErr::Http(code, msg))
        }
        Err(e) => Err(ApiErr::Net(e.to_string())),
    }
}

pub fn post(server: &str, path: &str, token: Option<&str>, body: Value) -> Result<Value, ApiErr> {
    let mut req = agent().post(&format!("{server}{path}")).set("Accept-Language", common::lang());
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    handle(req.send_json(body))
}

pub fn get(server: &str, path: &str, token: Option<&str>) -> Result<Value, ApiErr> {
    let mut req = agent().get(&format!("{server}{path}")).set("Accept-Language", common::lang());
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    handle(req.call())
}

pub fn payload(v: Value) -> Result<Payload, ApiErr> {
    serde_json::from_value(v).map_err(|e| ApiErr::Net(trf!("непонятный ответ панели: {}", "unexpected panel answer: {}", e)))
}

/// Скачать файл целиком в память (архив Nebula ~14 МБ).
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let resp = agent().get(url).call().map_err(|e| trf!("не удалось скачать {}: {}", "could not download {}: {}", url, e))?;
    let mut buf = Vec::new();
    resp.into_reader().take(64 << 20).read_to_end(&mut buf).map_err(|e| trf!("обрыв при скачивании: {}", "download interrupted: {}", e))?;
    Ok(buf)
}

pub fn check_server(s: &str) -> Result<String, String> {
    let s = s.trim().trim_end_matches('/').to_string();
    let local = s.starts_with("http://127.0.0.1:") || s.starts_with("http://localhost:");
    if !(s.starts_with("https://") || local) {
        return Err(tr("Адрес панели должен начинаться с https://", "The panel address must start with https://").into());
    }
    Ok(s)
}
