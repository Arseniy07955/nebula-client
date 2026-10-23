//! Русский и английский. Язык берётся из NEBULA_CLIENT_LANG, иначе из языка системы:
//! русский для ru/uk/be, английский для всех остальных. Служба отвечает на языке окна,
//! которое прислало запрос (`with_lang`), а свои фоновые сообщения пишет на языке системы.

use std::cell::Cell;
use std::sync::OnceLock;

static SYSTEM_EN: OnceLock<bool> = OnceLock::new();

thread_local! {
    static OVERRIDE: Cell<Option<bool>> = const { Cell::new(None) };
}

fn detect_en() -> bool {
    if let Ok(l) = std::env::var("NEBULA_CLIENT_LANG") {
        return !l.to_lowercase().starts_with("ru");
    }
    !system_is_russian()
}

#[cfg(windows)]
fn system_is_russian() -> bool {
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // младшие 10 бит: основной язык. 0x19 русский, 0x22 украинский, 0x23 белорусский
    let primary = unsafe { GetUserDefaultUILanguage() } & 0x3ff;
    matches!(primary, 0x19 | 0x22 | 0x23)
}

#[cfg(not(windows))]
fn system_is_russian() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .map(|v| ["ru", "uk", "be"].iter().any(|p| v.starts_with(p)))
        .unwrap_or(false)
}

pub fn is_en() -> bool {
    OVERRIDE.with(|o| o.get()).unwrap_or_else(|| *SYSTEM_EN.get_or_init(detect_en))
}

pub fn lang() -> &'static str {
    if is_en() { "en" } else { "ru" }
}

pub fn tr(ru: &'static str, en: &'static str) -> &'static str {
    if is_en() { en } else { ru }
}

/// Выполнить f на языке запроса (для ответов службы окну).
pub fn with_lang<T>(lang: Option<&str>, f: impl FnOnce() -> T) -> T {
    let v = lang.map(|l| !l.to_lowercase().starts_with("ru"));
    let prev = OVERRIDE.with(|o| o.replace(v));
    let r = f();
    OVERRIDE.with(|o| o.set(prev));
    r
}

/// `trf!("Привет, {}", "Hello, {}", name)`: format! на нужном языке.
#[macro_export]
macro_rules! trf {
    ($ru:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::is_en() { format!($en $(, $a)*) } else { format!($ru $(, $a)*) }
    };
}
