//! Бинарники Nebula: где взять (скачать с проверкой SHA256 или системные), ключ устройства и запуск.

use crate::logs::Logs;
use crate::plat;
use common::{tr, trf};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

pub const NEBULA_VERSION: &str = "1.9.3";

/// Официальные релизы slackhq/nebula, суммы из SHASUM256.txt релиза.
#[cfg(windows)]
const BUILDS: &[(&str, &str, &str)] = &[
    ("x86_64", "nebula-windows-amd64.zip", "65ead0e8104a36e35230511e8d983458353a0edb6c4e332464da727b8efce11e"),
    ("aarch64", "nebula-windows-arm64.zip", "599c49e2a1360eb448795e48b1afe9259bd79043d41e959f1e920d1a8238ef58"),
];
#[cfg(unix)]
const BUILDS: &[(&str, &str, &str)] = &[
    ("x86_64", "nebula-linux-amd64.tar.gz", "3aa3dc989a50791602593f182e119e37e4bef6bc12bbc99a1c91ad2a046aaa60"),
    ("aarch64", "nebula-linux-arm64.tar.gz", "f0a3ba520a960c5f004b64df6567e57904f204f22c83f3c525f99c902f583fea"),
];

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// Где лежат nebula и nebula-cert.
pub struct Bins {
    pub nebula: PathBuf,
    pub cert: PathBuf,
    /// Наши (скачанные), а не системные: их можно обновлять и разрешать в брандмауэре.
    pub own: bool,
}

fn own_dir(data: &Path) -> PathBuf {
    data.join("bin")
}

fn in_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

/// В Linux сначала системная Nebula (NixOS, пакет дистрибутива), в Windows всегда своя.
pub fn bins(data: &Path) -> Bins {
    if let Some(d) = std::env::var_os("NEBULA_BIN_DIR") {
        let d = PathBuf::from(d);
        return Bins { nebula: d.join(format!("nebula{EXE}")), cert: d.join(format!("nebula-cert{EXE}")), own: false };
    }
    if cfg!(unix) {
        if let (Some(n), Some(c)) = (in_path("nebula"), in_path("nebula-cert")) {
            return Bins { nebula: n, cert: c, own: false };
        }
    }
    let d = own_dir(data);
    Bins { nebula: d.join(format!("nebula{EXE}")), cert: d.join(format!("nebula-cert{EXE}")), own: true }
}

pub fn ensure_binaries(data: &Path, progress: &dyn Fn(&str)) -> Result<Bins, String> {
    let b = bins(data);
    if !b.own {
        return if b.nebula.exists() && b.cert.exists() {
            Ok(b)
        } else {
            Err(trf!("не найдены {} и {}", "{} and {} not found", b.nebula.display(), b.cert.display()))
        };
    }
    let bin = own_dir(data);
    let marker = bin.join("VERSION");
    if b.nebula.exists() && std::fs::read_to_string(&marker).map(|v| v.trim() == NEBULA_VERSION).unwrap_or(false) {
        return Ok(b);
    }
    let arch = std::env::consts::ARCH;
    let (_, file, sum) = BUILDS
        .iter()
        .find(|x| x.0 == arch)
        .ok_or_else(|| trf!("архитектура {} не поддерживается", "architecture {} is not supported", arch))?;
    // установщик кладёт официальный архив рядом со службой: тогда GitHub не нужен
    let bundled = std::env::current_exe().ok().map(|e| e.with_file_name(file)).and_then(|p| std::fs::read(p).ok());
    let data_bytes = match bundled {
        Some(b) if format!("{:x}", Sha256::digest(&b)) == *sum => b,
        _ => {
            progress(&trf!("Скачиваю Nebula {}", "Downloading Nebula {}", NEBULA_VERSION));
            let url = format!("https://github.com/slackhq/nebula/releases/download/v{NEBULA_VERSION}/{file}");
            crate::api::download(&url)?
        }
    };
    let got = format!("{:x}", Sha256::digest(&data_bytes));
    if got != *sum {
        return Err(trf!(
            "контрольная сумма архива Nebula не совпала ({}), установка остановлена",
            "Nebula archive checksum mismatch ({}), installation stopped",
            got
        ));
    }
    progress(tr("Распаковываю Nebula", "Unpacking Nebula"));
    let _ = std::fs::remove_dir_all(&bin);
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    plat::unpack(&data_bytes, &bin)?;
    std::fs::write(&marker, NEBULA_VERSION).map_err(|e| e.to_string())?;
    Ok(b)
}

fn run_hidden(exe: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    plat::quiet(&mut cmd);
    let out = cmd.output().map_err(|e| format!("{}: {e}", exe.display()))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(text)
    } else {
        Err(text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("error").to_string())
    }
}

/// Ключ создаётся в папке сети один раз и никуда не уходит, панели нужна только публичная часть.
pub fn ensure_key(bins: &Bins, dir: &Path) -> Result<String, String> {
    let key = dir.join("host.key");
    let pubp = dir.join("host.pub");
    if !key.exists() || !pubp.exists() {
        let _ = std::fs::remove_file(&key);
        let _ = std::fs::remove_file(&pubp);
        run_hidden(&bins.cert, &["keygen", "-out-key", &key.to_string_lossy(), "-out-pub", &pubp.to_string_lossy()])
            .map_err(|e| trf!("nebula-cert не смог создать ключ: {}", "nebula-cert could not create a key: {}", e))?;
        plat::private_file(&key);
    }
    std::fs::read_to_string(&pubp).map_err(|e| e.to_string())
}

pub fn test_config(bins: &Bins, dir: &Path) -> Result<(), String> {
    run_hidden(&bins.nebula, &["-test", "-config", &dir.join("config.yml").to_string_lossy()])
        .map(|_| ())
        .map_err(|e| trf!("Nebula не приняла конфиг: {}", "Nebula rejected the config: {}", e))
}

/// Запущенная nebula одной сети. Если служба умрёт, ОС завершит и её (job-объект в Windows,
/// PDEATHSIG в Linux), так что второй экземпляр с тем же адаптером не появится.
pub struct Running {
    child: Child,
    _guard: plat::ChildGuard,
    pub started: std::time::Instant,
    pub net_ready: bool,
}

impl Running {
    pub fn spawn(bins: &Bins, dir: &Path, tag: &str, logs: Arc<Logs>) -> Result<Running, String> {
        let mut cmd = Command::new(&bins.nebula);
        cmd.args(["-config", &dir.join("config.yml").to_string_lossy()])
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        plat::quiet(&mut cmd);
        plat::die_with_parent(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| trf!("не удалось запустить nebula: {}", "could not start nebula: {}", e))?;
        let guard = plat::guard_child(&child);
        let streams: [Option<Box<dyn std::io::Read + Send>>; 2] =
            [child.stdout.take().map(|s| Box::new(s) as _), child.stderr.take().map(|s| Box::new(s) as _)];
        for s in streams.into_iter().flatten() {
            let logs = logs.clone();
            let tag = tag.to_string();
            std::thread::spawn(move || {
                for line in BufReader::new(s).lines().map_while(Result::ok) {
                    logs.push(format!("[{tag}] {line}"));
                }
            });
        }
        Ok(Running { child, _guard: guard, started: std::time::Instant::now(), net_ready: false })
    }

    /// Some(код), если процесс уже завершился.
    pub fn exited(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(st)) => Some(st.code().unwrap_or(-1)),
            Ok(None) => None,
            Err(_) => Some(-1),
        }
    }

    pub fn stop(mut self) {
        plat::stop_child(&mut self.child);
    }
}
