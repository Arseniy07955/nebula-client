//! Значок в трее: цвет показывает общее состояние, в меню у каждой сети своя галочка, клик открывает окно.
//! Статус спрашивается у службы раз в 3 секунды, это один короткий запрос по локальному каналу.

use crate::icons::{self, Look};
use crate::sys;
use common::{tr, trf, ConnState, NetStatus, Request, Status};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT,
};

fn icon(look: Look) -> Icon {
    Icon::from_rgba(icons::rgba(look), icons::SIZE, icons::SIZE).expect("icon")
}

fn net_line(n: &NetStatus) -> String {
    let state = match (n.want_up, n.state) {
        (false, _) => tr("отключено", "off").to_string(),
        (true, ConnState::Running) => n.ip.clone(),
        (true, ConnState::Error) => tr("ошибка", "error").to_string(),
        (true, _) => tr("подключаюсь…", "connecting…").to_string(),
    };
    format!("{}: {state}", n.title)
}

fn describe(st: &Option<Status>, svc_err: bool) -> String {
    if svc_err {
        return tr("Служба Nebula Client не запущена", "Nebula Client service is not running").into();
    }
    let Some(s) = st else { return "Nebula Client".into() };
    if let Some(b) = &s.busy {
        return b.clone();
    }
    if s.networks.is_empty() {
        return tr("Нет сетей", "No networks").into();
    }
    s.networks.iter().map(net_line).collect::<Vec<_>>().join("\n")
}

/// Общий цвет: ошибка важнее, потом «подключаюсь», потом «в сети».
fn aggregate(st: &Option<Status>, svc_err: bool) -> Look {
    if svc_err {
        return Look::Error;
    }
    let Some(s) = st else { return Look::Down };
    let on: Vec<ConnState> = s.networks.iter().filter(|n| n.want_up).map(|n| n.state).collect();
    if on.contains(&ConnState::Error) {
        Look::Error
    } else if on.iter().any(|s| *s != ConnState::Running) {
        Look::Starting
    } else if on.is_empty() {
        Look::Down
    } else {
        Look::Up
    }
}

struct Items {
    menu: Menu,
    status: MenuItem,
    open: MenuItem,
    auto: CheckMenuItem,
    quit: MenuItem,
    /// (id пункта, slug сети)
    nets: Vec<(MenuId, String, CheckMenuItem)>,
    /// «Обновить до X», если служба нашла новую версию.
    update: Option<MenuItem>,
    /// По этому ключу видно, что меню пора собрать заново.
    key: String,
}

fn menu_key(st: &Option<Status>) -> String {
    let Some(s) = st else { return String::new() };
    let nets: Vec<&str> = s.networks.iter().map(|n| n.slug.as_str()).collect();
    format!("{}|{:?}|{}", nets.join(","), s.update.available, s.update.installing)
}

fn build(st: &Option<Status>) -> Items {
    let status = MenuItem::new("Nebula Client", false, None);
    let open = MenuItem::new(tr("Открыть", "Open"), true, None);
    let auto = CheckMenuItem::new(tr("Запускать при входе в Windows", "Start at Windows sign-in"), true, sys::autostart_enabled(), None);
    let quit = MenuItem::new(tr("Выход", "Quit"), true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[&status, &PredefinedMenuItem::separator(), &open]);
    let upd = st.as_ref().map(|s| s.update.clone()).unwrap_or_default();
    let update = upd.available.as_ref().filter(|_| !upd.installing).map(|v| {
        let text = if upd.can_install { trf!("Обновить до {}", "Update to {}", v) } else { trf!("Доступна версия {}", "Version {} is available", v) };
        let item = MenuItem::new(text, true, None);
        let _ = menu.append(&item);
        item
    });
    let mut nets = Vec::new();
    let networks = st.as_ref().map(|s| s.networks.clone()).unwrap_or_default();
    if !networks.is_empty() {
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    for n in &networks {
        let item = CheckMenuItem::new(trf!("Сеть «{}»", "Network \"{}\"", n.title), true, n.want_up, None);
        let _ = menu.append(&item);
        nets.push((item.id().clone(), n.slug.clone(), item));
    }
    let _ = menu.append_items(&[&PredefinedMenuItem::separator(), &auto, &quit]);
    Items { menu, status, open, auto, quit, nets, update, key: menu_key(st) }
}

pub fn run() {
    if sys::already_running("Local\\NebulaClientTray") {
        // второй запуск exe: просто открыть окно
        sys::open_window();
        return;
    }
    // --autostart: установщик включает автозапуск при первом запуске
    if std::env::args().any(|a| a == "--autostart") {
        sys::set_autostart(true);
    }

    let mut items = build(&None);
    let mut look = Look::Down;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(items.menu.clone()))
        .with_menu_on_left_click(false)
        .with_tooltip("Nebula Client")
        .with_icon(icon(look))
        .build()
        .expect("tray icon");

    let (tx, rx) = mpsc::channel::<Result<Status, ()>>();
    let mut last: Option<Status> = None;
    let mut next_poll = Instant::now();
    let mut polling = false;

    loop {
        // сообщения Windows для значка и меню; ждём не дольше 200 мс
        unsafe {
            MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 200, QS_ALLINPUT);
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if !polling && Instant::now() >= next_poll {
            polling = true;
            let tx = tx.clone();
            std::thread::spawn(move || {
                let r = common::call(&Request::Status).ok().and_then(|r| r.status).ok_or(());
                let _ = tx.send(r);
            });
        }
        while let Ok(r) = rx.try_recv() {
            polling = false;
            next_poll = Instant::now() + Duration::from_secs(3);
            let svc_err = r.is_err();
            last = r.ok();
            if menu_key(&last) != items.key {
                // поменялся набор сетей или появилось обновление: меню собирается заново
                items = build(&last);
                tray.set_menu(Some(Box::new(items.menu.clone())));
            }
            if let Some(s) = &last {
                for (_, slug, item) in &items.nets {
                    if let Some(n) = s.net(slug) {
                        item.set_checked(n.want_up);
                    }
                }
            }
            let new_look = aggregate(&last, svc_err);
            if new_look != look {
                look = new_look;
                let _ = tray.set_icon(Some(icon(look)));
            }
            let text = describe(&last, svc_err);
            // подсказка значка в Windows обрезается после 127 символов
            let tip: String = format!("Nebula Client\n{text}").chars().take(127).collect();
            let _ = tray.set_tooltip(Some(tip));
            status_text(&items.status, &text);
        }

        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                sys::open_window();
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == items.open.id() {
                sys::open_window();
            } else if ev.id == items.auto.id() {
                sys::set_autostart(items.auto.is_checked());
            } else if items.update.as_ref().map(|u| u.id() == &ev.id).unwrap_or(false) {
                let upd = last.as_ref().map(|s| s.update.clone()).unwrap_or_default();
                if upd.can_install {
                    std::thread::spawn(|| {
                        let _ = common::call(&Request::UpdateInstall);
                    });
                    next_poll = Instant::now() + Duration::from_millis(500);
                } else {
                    sys::open_window();
                }
            } else if ev.id == items.quit.id() {
                // служба продолжает работать, закрывается только значок
                return;
            } else if let Some((_, slug, item)) = items.nets.iter().find(|(id, _, _)| *id == ev.id) {
                // галочка уже переключилась сама: её новое значение и есть желаемое
                let net = slug.clone();
                let req = if item.is_checked() { Request::Connect { net } } else { Request::Disconnect { net } };
                std::thread::spawn(move || {
                    let _ = common::call(&req);
                });
                next_poll = Instant::now() + Duration::from_millis(300);
            }
        }
    }
}

/// Первая строка статуса в меню (весь список сетей виден ниже галочками).
fn status_text(item: &MenuItem, text: &str) {
    let first = text.lines().next().unwrap_or("Nebula Client");
    let n = text.lines().count();
    item.set_text(if n > 1 { trf!("{} (+{})", "{} (+{})", first, n - 1) } else { first.to_string() });
}
