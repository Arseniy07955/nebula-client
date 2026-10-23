//! Обновления: раз в сутки служба смотрит релизы на GitHub. В Windows найденный установщик
//! скачивается, сверяется с SHA256SUMS.txt того же релиза и запускается тихо. В Linux только
//! сообщаем: там обновляют install.sh, пакетный менеджер или Nix.

use crate::api;
use common::{now, tr, trf, UpdateMode, UpdateState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const CHECK_EVERY: i64 = 24 * 3600;

/// Лента релизов. NEBULA_CLIENT_UPDATE_FEED задаёт свою при сборке или при запуске (для проверки).
fn feed() -> String {
    std::env::var("NEBULA_CLIENT_UPDATE_FEED").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        option_env!("NEBULA_CLIENT_UPDATE_FEED").unwrap_or("https://api.github.com/repos/Arseniy07955/nebula-client/releases?per_page=15").into()
    })
}

/// update.json в папке данных.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub mode: UpdateMode,
    /// Пока все релизы тестовые, без этого обновлений не было бы вовсе.
    #[serde(default = "yes")]
    pub prerelease: bool,
    #[serde(default)]
    pub last_check: Option<i64>,
}

fn yes() -> bool {
    true
}

impl Default for Saved {
    fn default() -> Self {
        Saved { mode: UpdateMode::Auto, prerelease: true, last_check: None }
    }
}

impl Saved {
    pub fn load(dir: &Path) -> Saved {
        std::fs::read(dir.join("update.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        let _ = std::fs::write(dir.join("update.json"), serde_json::to_vec_pretty(self).unwrap_or_default());
    }
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub page: String,
    /// (имя, ссылка) установщика для этой системы и файла сумм.
    pub setup: Option<(String, String)>,
    pub sums: Option<String>,
}

impl Release {
    pub fn can_install(&self) -> bool {
        cfg!(windows) && self.setup.is_some() && self.sums.is_some()
    }
}

/// Текущее состояние для окна.
#[derive(Default)]
pub struct Upd {
    pub saved: Saved,
    pub available: Option<Release>,
    pub installing: bool,
    /// Ошибка проверки (сбрасывается следующей удачной проверкой).
    pub error: Option<String>,
    /// Почему не встала последняя версия: держится, пока не встанет или не выйдет новая.
    pub fail: Option<(String, String)>,
}

impl Upd {
    pub fn state(&self) -> UpdateState {
        UpdateState {
            mode: self.saved.mode,
            prerelease: self.saved.prerelease,
            available: self.available.as_ref().map(|r| r.version.clone()),
            page: self.available.as_ref().map(|r| r.page.clone()),
            can_install: self.available.as_ref().map(Release::can_install).unwrap_or(false),
            installing: self.installing,
            last_check: self.saved.last_check,
            error: self.error.clone().or_else(|| {
                let v = self.available.as_ref()?.version.clone();
                self.fail.as_ref().filter(|(fv, _)| *fv == v).map(|(_, e)| e.clone())
            }),
        }
    }
}

/// "v0.4.0", "0.4.0-beta" -> (0, 4, 0). Суффикс после дефиса не учитываем.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    let s = s.split(['-', '+']).next()?;
    let mut it = s.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

fn setup_name(version: &str) -> Option<String> {
    cfg!(all(windows, target_arch = "x86_64")).then(|| format!("nebula-client-{version}-setup.exe"))
}

/// Самый новый подходящий релиз, если он новее этой версии.
pub fn check(prerelease: bool) -> Result<Option<Release>, String> {
    let current = parse_version(env!("CARGO_PKG_VERSION")).unwrap_or_default();
    let list = api::download(&feed())?;
    let list: Vec<Value> = serde_json::from_slice(&list).map_err(|e| trf!("непонятный ответ GitHub: {}", "unexpected GitHub answer: {}", e))?;
    let best = list
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false) && (prerelease || !r["prerelease"].as_bool().unwrap_or(false)))
        .filter_map(|r| Some((parse_version(r["tag_name"].as_str()?)?, r)))
        .max_by_key(|(v, _)| *v);
    let Some((ver, r)) = best.filter(|(v, _)| *v > current) else { return Ok(None) };
    let version = format!("{}.{}.{}", ver.0, ver.1, ver.2);
    let asset = |name: &str| {
        r["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(name)).and_then(|a| a["browser_download_url"].as_str()).map(String::from)
    };
    let setup = setup_name(&version).and_then(|n| asset(&n).map(|u| (n, u)));
    Ok(Some(Release {
        version,
        page: r["html_url"].as_str().unwrap_or_default().to_string(),
        setup,
        sums: asset("SHA256SUMS.txt"),
    }))
}

/// Строка `<sha256>  <имя>` (или `*<имя>`) из SHA256SUMS.txt.
fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (h, n) = (it.next()?, it.next()?);
        (n.trim_start_matches('*') == name && h.len() == 64).then(|| h.to_ascii_lowercase())
    })
}

/// Скачать установщик в updates/ и проверить сумму. Папка данных доступна только администраторам,
/// так что между проверкой и запуском файл никто не подменит.
pub fn fetch(rel: &Release, dir: &Path) -> Result<PathBuf, String> {
    let (name, url) = rel.setup.as_ref().ok_or(tr("в релизе нет установщика для этой системы", "the release has no installer for this system"))?;
    let sums = rel.sums.as_ref().ok_or(tr("в релизе нет SHA256SUMS.txt", "the release has no SHA256SUMS.txt"))?;
    let sums = String::from_utf8_lossy(&api::download(sums)?).to_string();
    let want = sum_for(&sums, name).ok_or_else(|| trf!("в SHA256SUMS.txt нет {}", "{} is missing from SHA256SUMS.txt", name))?;
    let data = api::download(url)?;
    let got = format!("{:x}", Sha256::digest(&data));
    if got != want {
        return Err(trf!("сумма установщика не совпала ({}), обновление отменено", "installer checksum mismatch ({}), update cancelled", got));
    }
    let out = dir.join("updates");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let path = out.join(name);
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn due(s: &Saved) -> bool {
    s.mode != UpdateMode::Off && s.last_check.map(|t| now() - t >= CHECK_EVERY).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v1.10.3-beta"), Some((1, 10, 3)));
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn sums() {
        let h = "a".repeat(64);
        let text = format!("{h}  nebula-client-0.4.0-setup.exe\n{} *x.tar.gz\n", "b".repeat(64));
        assert_eq!(sum_for(&text, "nebula-client-0.4.0-setup.exe"), Some(h));
        assert_eq!(sum_for(&text, "x.tar.gz"), Some("b".repeat(64)));
        assert_eq!(sum_for(&text, "y"), None);
    }
}
