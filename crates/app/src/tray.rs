//! Значок в трее: цвет показывает состояние, меню подключает и отключает, клик открывает окно.
//! Статус спрашивается у службы раз в 3 секунды, это один короткий запрос по локальному каналу.

use crate::icons::{self, Look};
use crate::sys;
use common::{ConnState, Request, Status};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT,
};

fn icon(look: Look) -> Icon {
    Icon::from_rgba(icons::rgba(look), icons::SIZE, icons::SIZE).expect("значок")
}

fn describe(st: &Option<Status>, svc_err: bool) -> String {
    if svc_err {
        return "Служба Nebula Client не запущена".into();
    }
    let Some(s) = st else { return "Nebula Client".into() };
    if !s.enrolled {
        return "Не подключено к сети".into();
    }
    if let Some(b) = &s.busy {
        return b.clone();
    }
    match s.state {
        ConnState::Running => format!("В сети «{}»: {}", s.network, s.ip),
        ConnState::Starting => "Подключаюсь…".into(),
        ConnState::Error => format!("Ошибка: {}", s.error.clone().unwrap_or_default()),
        ConnState::Stopped => "Отключено".into(),
    }
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

    let status_item = MenuItem::new("Nebula Client", false, None);
    let open_item = MenuItem::new("Открыть", true, None);
    let toggle_item = MenuItem::new("Подключиться", false, None);
    let auto_item = CheckMenuItem::new("Запускать при входе в Windows", true, sys::autostart_enabled(), None);
    let quit_item = MenuItem::new("Выход", true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[&status_item, &PredefinedMenuItem::separator(), &open_item, &toggle_item, &PredefinedMenuItem::separator(), &auto_item, &quit_item]);

    let mut look = Look::Down;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Nebula Client")
        .with_icon(icon(look))
        .build()
        .expect("значок в трее");

    let (tx, rx) = mpsc::channel::<Result<Status, ()>>();
    let mut last: Option<Status> = None;
    let mut svc_err: bool;
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
            svc_err = r.is_err();
            last = r.ok();
            let new_look = if svc_err { Look::Error } else { Look::of(last.as_ref().filter(|s| s.enrolled).map(|s| s.state)) };
            if new_look != look {
                look = new_look;
                let _ = tray.set_icon(Some(icon(look)));
            }
            let text = describe(&last, svc_err);
            let _ = tray.set_tooltip(Some(format!("Nebula Client\n{text}")));
            status_item.set_text(text);
            let enrolled = last.as_ref().map(|s| s.enrolled).unwrap_or(false);
            let up = last.as_ref().map(|s| s.want_up).unwrap_or(false);
            toggle_item.set_enabled(enrolled);
            toggle_item.set_text(if up { "Отключиться" } else { "Подключиться" });
        }

        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                sys::open_window();
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == open_item.id() {
                sys::open_window();
            } else if ev.id == toggle_item.id() {
                let up = last.as_ref().map(|s| s.want_up).unwrap_or(false);
                std::thread::spawn(move || {
                    let _ = common::call(if up { &Request::Disconnect } else { &Request::Connect });
                });
                next_poll = Instant::now() + Duration::from_millis(300);
            } else if ev.id == auto_item.id() {
                sys::set_autostart(auto_item.is_checked());
            } else if ev.id == quit_item.id() {
                // служба продолжает работать, закрывается только значок
                return;
            }
        }
    }
}
