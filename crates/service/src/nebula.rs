//! Бинарники Nebula: скачивание с проверкой SHA256, ключ устройства и запуск nebula.exe.

use crate::logs::Logs;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub const NEBULA_VERSION: &str = "1.9.3";
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Официальные релизы slackhq/nebula, суммы из SHASUM256.txt релиза.
const BUILDS: &[(&str, &str, &str)] = &[
    ("AMD64", "nebula-windows-amd64.zip", "65ead0e8104a36e35230511e8d983458353a0edb6c4e332464da727b8efce11e"),
    ("ARM64", "nebula-windows-arm64.zip", "599c49e2a1360eb448795e48b1afe9259bd79043d41e959f1e920d1a8238ef58"),
];

pub fn bin_dir(dir: &Path) -> PathBuf {
    dir.join("bin")
}

pub fn nebula_exe(dir: &Path) -> PathBuf {
    bin_dir(dir).join("nebula.exe")
}

fn arch() -> String {
    std::env::var("PROCESSOR_ARCHITEW6432")
        .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
        .unwrap_or_default()
        .to_uppercase()
}

pub fn ensure_binaries(dir: &Path, progress: &dyn Fn(&str)) -> Result<(), String> {
    let bin = bin_dir(dir);
    let marker = bin.join("VERSION");
    if nebula_exe(dir).exists() && std::fs::read_to_string(&marker).map(|v| v.trim() == NEBULA_VERSION).unwrap_or(false) {
        return Ok(());
    }
    let a = arch();
    let (_, file, sum) = BUILDS.iter().find(|b| b.0 == a).ok_or(format!("архитектура {a} не поддерживается, нужна AMD64 или ARM64"))?;
    progress(&format!("Скачиваю Nebula {NEBULA_VERSION}"));
    let url = format!("https://github.com/slackhq/nebula/releases/download/v{NEBULA_VERSION}/{file}");
    let data = crate::api::download(&url)?;
    let got = format!("{:x}", Sha256::digest(&data));
    if got != *sum {
        return Err(format!("контрольная сумма архива Nebula не совпала ({got}), установка остановлена"));
    }
    progress("Распаковываю Nebula");
    let _ = std::fs::remove_dir_all(&bin);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| format!("архив Nebula повреждён: {e}"))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { continue };
        let out = bin.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut w = std::fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
    }
    std::fs::write(&marker, NEBULA_VERSION).map_err(|e| e.to_string())?;
    Ok(())
}

fn run_hidden(exe: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).output().map_err(|e| format!("{}: {e}", exe.display()))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(text)
    } else {
        Err(text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ошибка").to_string())
    }
}

/// Ключ устройства создаётся один раз и никуда не уходит, панели нужна только публичная часть.
pub fn ensure_key(dir: &Path) -> Result<String, String> {
    let key = dir.join("host.key");
    let pubp = dir.join("host.pub");
    if !key.exists() || !pubp.exists() {
        let _ = std::fs::remove_file(&key);
        let _ = std::fs::remove_file(&pubp);
        run_hidden(
            &bin_dir(dir).join("nebula-cert.exe"),
            &["keygen", "-out-key", &key.to_string_lossy(), "-out-pub", &pubp.to_string_lossy()],
        )
        .map_err(|e| format!("nebula-cert не смог создать ключ: {e}"))?;
    }
    std::fs::read_to_string(&pubp).map_err(|e| e.to_string())
}

pub fn test_config(dir: &Path) -> Result<(), String> {
    run_hidden(&nebula_exe(dir), &["-test", "-config", &dir.join("config.yml").to_string_lossy()])
        .map(|_| ())
        .map_err(|e| format!("Nebula не приняла конфиг: {e}"))
}

/// Запущенный nebula.exe. Сидит в job-объекте: если служба упадёт, Windows убьёт и его,
/// так что второй экземпляр с тем же адаптером не появится.
pub struct Running {
    child: Child,
    job: HANDLE,
    pub started: std::time::Instant,
    pub net_ready: bool,
}

unsafe impl Send for Running {}

impl Running {
    pub fn spawn(dir: &Path, logs: Arc<Logs>) -> Result<Running, String> {
        let mut child = Command::new(nebula_exe(dir))
            .args(["-config", &dir.join("config.yml").to_string_lossy()])
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("не удалось запустить nebula.exe: {e}"))?;
        let job = unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !job.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE);
            }
            job
        };
        for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
                       child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
            let logs = logs.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    logs.push(line);
                }
            });
        }
        Ok(Running { child, job, started: std::time::Instant::now(), net_ready: false })
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
        // адаптер Wintun живёт, пока жив процесс, и удаляется вместе с ним
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.job.is_null() {
            unsafe { CloseHandle(self.job) };
        }
    }
}
