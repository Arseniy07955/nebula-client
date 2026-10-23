//! Последние строки журнала службы и Nebula в памяти: для вкладки «Журнал» в окне.

use std::collections::VecDeque;
use std::sync::Mutex;

const KEEP: usize = 500;

#[derive(Default)]
pub struct Logs(Mutex<VecDeque<String>>);

impl Logs {
    pub fn push(&self, line: impl Into<String>) {
        let line = line.into();
        let line = line.trim_end();
        if line.is_empty() {
            return;
        }
        let mut q = self.0.lock().unwrap();
        if q.len() >= KEEP {
            q.pop_front();
        }
        q.push_back(line.to_string());
    }

    /// Сообщение самой службы, с отметкой времени как у Nebula.
    pub fn note(&self, msg: impl AsRef<str>) {
        self.push(format!("{} {}", stamp(), msg.as_ref()));
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        let q = self.0.lock().unwrap();
        q.iter().skip(q.len().saturating_sub(n)).cloned().collect()
    }

    /// Последняя строка nebula этой сети с уровнем error/fatal: чтобы показать причину падения.
    pub fn last_error(&self, tag: &str) -> Option<String> {
        let q = self.0.lock().unwrap();
        let prefix = format!("[{tag}] ");
        q.iter().rev().take(100).filter(|l| l.starts_with(&prefix)).find(|l| l.contains("level=error") || l.contains("level=fatal")).map(|l| {
            match (field(l, "msg"), field(l, "error")) {
                (Some(m), Some(e)) => format!("{m}: {e}"),
                (Some(m), None) => m,
                _ => l.clone(),
            }
        })
    }
}

/// Значение поля logrus: msg="текст с \"кавычками\"" или msg=слово.
fn field(line: &str, key: &str) -> Option<String> {
    let pat = format!(" {key}=");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    if let Some(body) = rest.strip_prefix('"') {
        let mut out = String::new();
        let mut esc = false;
        for c in body.chars() {
            match (esc, c) {
                (true, _) => {
                    out.push(c);
                    esc = false;
                }
                (false, '\\') => esc = true,
                (false, '"') => return Some(out),
                _ => out.push(c),
            }
        }
        Some(out)
    } else {
        Some(rest.split_whitespace().next().unwrap_or("").to_string())
    }
}

fn stamp() -> String {
    let t = common::now();
    let (h, m, s) = ((t / 3600) % 24, (t / 60) % 60, t % 60);
    format!("{h:02}:{m:02}:{s:02}Z")
}
