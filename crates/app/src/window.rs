//! Окно на egui. В Windows это отдельный процесс: закрыли окно — процесс завершился и отдал память.
//! Данные спрашиваются у службы в фоновом потоке и только пока окно открыто.

use crate::icons::{self, Look};
use common::{tr, trf, ConnState, NetStatus, OpenNet, Peer, Request, Status};
use eframe::egui::{self, Color32, RichText, Sense};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(0x2e, 0xc2, 0x7e);
const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa8);
const DANGER: Color32 = Color32::from_rgb(0xe0, 0x4f, 0x4f);
pub const WINDOW_TITLE: &str = "Nebula Client";

#[derive(Default)]
struct Shared {
    status: Option<Status>,
    svc_err: Option<String>,
    peers: HashMap<String, Vec<Peer>>,
    logs: Vec<String>,
    /// Итог последнего действия: (успех, текст).
    note: Option<(bool, String)>,
    acting: bool,
    /// Свободная регистрация на сервере из формы: (сервер, сети или ошибка).
    open: Option<(String, Result<Vec<OpenNet>, String>)>,
    /// Только что вошли в эту сеть: показать её.
    joined: Option<String>,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Peers,
    Logs,
}

struct App {
    sh: Arc<Mutex<Shared>>,
    want_logs: Arc<AtomicBool>,
    poke: Arc<AtomicBool>,
    /// Какая сеть открыта (None: экран «Добавить сеть»).
    sel: Arc<Mutex<Option<String>>>,
    adding: bool,
    tab: Tab,
    server: String,
    name: String,
    invite: String,
    open_asked: Option<String>,
    /// Выбранная сеть со свободной регистрацией, если их несколько.
    pick: Option<String>,
    confirm_leave: Option<Instant>,
}

pub fn run() {
    #[cfg(windows)]
    if crate::sys::already_running("Local\\NebulaClientWindow") {
        crate::sys::focus_existing_window();
        return;
    }
    let icon = egui::IconData { rgba: icons::rgba(Look::Up), width: icons::SIZE, height: icons::SIZE };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_app_id("nebula-client")
            .with_inner_size([460.0, 660.0])
            .with_min_inner_size([380.0, 500.0])
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    let _ = eframe::run_native(WINDOW_TITLE, opts, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))));
}

impl App {
    fn new(ctx: &egui::Context) -> App {
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = ACCENT.linear_multiply(0.6);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        ctx.set_style(style);

        let sh = Arc::new(Mutex::new(Shared::default()));
        let want_logs = Arc::new(AtomicBool::new(false));
        let poke = Arc::new(AtomicBool::new(true));
        let sel = Arc::new(Mutex::new(None));
        spawn_poller(ctx.clone(), sh.clone(), want_logs.clone(), poke.clone(), sel.clone());
        App {
            sh,
            want_logs,
            poke,
            sel,
            adding: false,
            tab: Tab::Peers,
            server: common::DEFAULT_SERVER.into(),
            name: common::default_device_name(),
            invite: String::new(),
            open_asked: None,
            pick: None,
            confirm_leave: None,
        }
    }

    fn act(&self, ctx: &egui::Context, req: Request, ok_text: &'static str) {
        let sh = self.sh.clone();
        let poke = self.poke.clone();
        let ctx = ctx.clone();
        sh.lock().unwrap().acting = true;
        std::thread::spawn(move || {
            let r = common::call(&req);
            let mut s = sh.lock().unwrap();
            s.acting = false;
            s.note = match r {
                Ok(r) if r.ok => {
                    if r.net.is_some() {
                        s.joined = r.net;
                    }
                    (!ok_text.is_empty()).then(|| (true, ok_text.to_string()))
                }
                Ok(r) => Some((false, r.error.unwrap_or_else(|| "error".into()))),
                Err(e) => Some((false, e.to_string())),
            };
            drop(s);
            poke.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    fn ask_open(&mut self, ctx: &egui::Context) {
        let server = self.server.trim().to_string();
        if self.open_asked.as_deref() == Some(server.as_str()) {
            return;
        }
        self.open_asked = Some(server.clone());
        let sh = self.sh.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = match common::call(&Request::OpenNets { server: server.clone() }) {
                Ok(r) if r.ok => Ok(r.open.unwrap_or_default()),
                Ok(r) => Err(r.error.unwrap_or_default()),
                Err(e) => Err(e.to_string()),
            };
            sh.lock().unwrap().open = Some((server, r));
            ctx.request_repaint();
        });
    }
}

fn spawn_poller(ctx: egui::Context, sh: Arc<Mutex<Shared>>, want_logs: Arc<AtomicBool>, poke: Arc<AtomicBool>, sel: Arc<Mutex<Option<String>>>) {
    std::thread::spawn(move || {
        let mut last_peers = Instant::now() - Duration::from_secs(60);
        let mut last_status = Instant::now() - Duration::from_secs(60);
        loop {
            let poked = poke.swap(false, Ordering::SeqCst);
            if poked || last_status.elapsed() >= Duration::from_millis(1500) {
                last_status = Instant::now();
                let r = common::call(&Request::Status);
                {
                    let mut s = sh.lock().unwrap();
                    match r {
                        Ok(r) => {
                            s.status = r.status;
                            s.svc_err = None;
                        }
                        Err(e) => {
                            s.status = None;
                            s.svc_err = Some(e.to_string());
                        }
                    }
                }
                let cur = sel.lock().unwrap().clone();
                if let Some(net) = cur {
                    if poked || last_peers.elapsed() >= Duration::from_secs(5) {
                        last_peers = Instant::now();
                        if let Ok(r) = common::call(&Request::Peers { net: net.clone() }) {
                            if let Some(p) = r.peers {
                                sh.lock().unwrap().peers.insert(net, p);
                            }
                        }
                    }
                }
                if want_logs.load(Ordering::SeqCst) {
                    if let Ok(r) = common::call(&Request::Logs) {
                        sh.lock().unwrap().logs = r.logs.unwrap_or_default();
                    }
                }
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    });
}

fn date(ts: i64) -> String {
    // дата по UTC без внешних крейтов (алгоритм Howard Hinnant)
    let z = ts.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    if common::is_en() { format!("{y}-{m:02}-{d:02}") } else { format!("{d:02}.{m:02}.{y}") }
}

fn ago(ts: i64) -> String {
    let s = (common::now() - ts).max(0);
    match s {
        0..=59 => tr("только что", "just now").into(),
        60..=3599 => trf!("{} мин назад", "{} min ago", s / 60),
        3600..=86399 => trf!("{} ч назад", "{} h ago", s / 3600),
        _ => trf!("{} дн назад", "{} d ago", s / 86400),
    }
}

fn state_color(n: &NetStatus) -> Color32 {
    let [r, g, b] = Look::of(Some(if n.want_up { n.state } else { ConnState::Stopped })).rgb();
    Color32::from_rgb(r, g, b)
}

fn dot(ui: &mut egui::Ui, color: Color32, r: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(r * 2.0 + 4.0, r * 2.0 + 4.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), r, color);
}

fn copy_label(ui: &mut egui::Ui, text: &str) {
    let resp = ui.add(egui::Label::new(RichText::new(text).monospace()).sense(Sense::click())).on_hover_text(tr("Нажмите, чтобы скопировать", "Click to copy"));
    if resp.clicked() {
        ui.output_mut(|o| o.copied_text = text.to_string());
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (status, svc_err, peers, logs, note, acting, joined) = {
            let mut s = self.sh.lock().unwrap();
            (s.status.clone(), s.svc_err.clone(), s.peers.clone(), s.logs.clone(), s.note.clone(), s.acting, s.joined.take())
        };
        if let Some(j) = joined {
            *self.sel.lock().unwrap() = Some(j);
            self.adding = false;
            self.invite.clear();
            self.poke.store(true, Ordering::SeqCst);
        }
        self.want_logs.store(self.tab == Tab::Logs && !self.adding, Ordering::SeqCst);

        egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).inner_margin(18.0)).show(ctx, |ui| {
            if let Some(e) = svc_err {
                no_service(ui, &e, &self.poke);
            } else if let Some(st) = status {
                // выбранная сеть пропала (вышли) или ещё не выбрана: первая из списка
                let mut sel = self.sel.lock().unwrap().clone();
                if sel.as_ref().map_or(true, |s| st.net(s).is_none()) {
                    sel = st.networks.first().map(|n| n.slug.clone());
                    *self.sel.lock().unwrap() = sel.clone();
                }
                self.net_bar(ui, &st, sel.as_deref());
                ui.add_space(4.0);
                match sel.as_deref().and_then(|s| st.net(s)).filter(|_| !self.adding) {
                    Some(n) => {
                        let p = peers.get(&n.slug).cloned().unwrap_or_default();
                        self.network(ui, ctx, n, &p, &logs, acting);
                    }
                    None => self.join_form(ui, ctx, &st, acting),
                }
            } else {
                ui.spinner();
            }
            if let Some((ok, text)) = note {
                ui.add_space(6.0);
                ui.label(RichText::new(text).color(if ok { ACCENT } else { DANGER }));
            }
        });
        ctx.request_repaint_after(Duration::from_secs(2));
    }
}

fn no_service(ui: &mut egui::Ui, err: &str, poke: &AtomicBool) {
    ui.heading(tr("Служба не запущена", "The service is not running"));
    ui.label(RichText::new(err).color(MUTED));
    ui.add_space(8.0);
    ui.label(tr(
        "Nebula Client работает через службу: она держит подключение, даже когда окно закрыто. Её нужно поставить один раз с правами администратора.",
        "Nebula Client works through a background service that keeps you connected while the window is closed. It has to be installed once with administrator rights.",
    ));
    ui.add_space(8.0);
    #[cfg(windows)]
    {
        if crate::sys::service_exe().exists() {
            if ui.button(tr("Установить службу", "Install the service")).clicked() {
                crate::sys::install_service();
                poke.store(true, Ordering::SeqCst);
            }
        } else {
            ui.label(RichText::new(tr("Рядом с программой нет nebula-client-service.exe, переустановите Nebula Client", "nebula-client-service.exe is missing next to the app, reinstall Nebula Client")).color(DANGER));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = poke;
        ui.label(tr("Запустите службу и добавьте себя в группу nebula-client:", "Start the service and add yourself to the nebula-client group:"));
        ui.label(RichText::new("sudo systemctl enable --now nebula-client\nsudo usermod -aG nebula-client $USER").monospace());
        ui.label(RichText::new(tr("На NixOS: services.nebula-client.enable = true и users = [ \"имя\" ].", "On NixOS: services.nebula-client.enable = true and users = [ \"name\" ].")).color(MUTED));
    }
}

impl App {
    /// Строка переключения сетей и кнопка «добавить».
    fn net_bar(&mut self, ui: &mut egui::Ui, st: &Status, sel: Option<&str>) {
        if st.networks.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for n in &st.networks {
                let on = sel == Some(n.slug.as_str()) && !self.adding;
                let resp = ui
                    .scope(|ui| {
                        ui.horizontal(|ui| {
                            dot(ui, state_color(n), 4.0);
                            ui.selectable_label(on, RichText::new(&n.title).strong())
                        })
                        .inner
                    })
                    .inner;
                if resp.clicked() {
                    *self.sel.lock().unwrap() = Some(n.slug.clone());
                    self.adding = false;
                    self.confirm_leave = None;
                    self.poke.store(true, Ordering::SeqCst);
                }
            }
            if ui.selectable_label(self.adding, tr("+ Добавить сеть", "+ Add network")).clicked() {
                self.adding = true;
            }
        });
        ui.separator();
    }

    fn join_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, st: &Status, acting: bool) {
        self.ask_open(ctx);
        ui.heading(if st.networks.is_empty() { tr("Подключение к сети", "Join a network") } else { tr("Ещё одна сеть", "Add another network") });
        ui.label(
            RichText::new(tr(
                "Ключ устройства создаётся на этом компьютере и никуда не уходит. Панель выдаст адрес и сертификат. У каждой сети свой ключ и свой адаптер.",
                "The device key is created on this computer and never leaves it. The panel issues an address and a certificate. Each network gets its own key and adapter.",
            ))
            .color(MUTED),
        );
        ui.add_space(8.0);
        ui.label(tr("Имя устройства", "Device name"));
        ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
        ui.label(RichText::new(tr("Латиница, цифры, точка, дефис. Его увидят другие участники", "Latin letters, digits, dot, dash. Other members will see it")).small().color(MUTED));
        ui.add_space(4.0);
        ui.label(tr("Инвайт", "Invite"));
        ui.add(egui::TextEdit::singleline(&mut self.invite).hint_text(tr("NF-XXXX-XXXX-XXXX, если сеть закрытая", "NF-XXXX-XXXX-XXXX for a closed network")).desired_width(f32::INFINITY));
        let open = self.sh.lock().unwrap().open.clone();
        if let Some((srv, r)) = open.filter(|(s, _)| Some(s) == self.open_asked.as_ref()) {
            let _ = srv;
            match r {
                Ok(list) if list.len() == 1 => {
                    self.pick = None;
                    ui.label(RichText::new(trf!("Без инвайта можно войти в «{}»", "You can join \"{}\" without an invite", list[0].title)).color(ACCENT));
                }
                Ok(list) if !list.is_empty() => {
                    if self.pick.as_ref().map_or(true, |p| !list.iter().any(|o| &o.slug == p)) {
                        self.pick = Some(list[0].slug.clone());
                    }
                    ui.label(RichText::new(tr("Без инвайта можно войти в одну из сетей:", "You can join one of these networks without an invite:")).color(ACCENT));
                    ui.horizontal_wrapped(|ui| {
                        for o in &list {
                            ui.radio_value(&mut self.pick, Some(o.slug.clone()), &o.title);
                        }
                    });
                }
                Ok(_) => {
                    ui.label(RichText::new(tr("На этой панели все сети по инвайтам", "All networks on this panel need an invite")).color(MUTED));
                }
                Err(e) => {
                    ui.label(RichText::new(e).color(MUTED));
                }
            }
        }
        egui::CollapsingHeader::new(tr("Сервер", "Server")).default_open(self.server != common::DEFAULT_SERVER).show(ui, |ui| {
            if ui.add(egui::TextEdit::singleline(&mut self.server).desired_width(f32::INFINITY)).lost_focus() {
                self.ask_open(ctx);
            }
        });
        ui.add_space(8.0);
        let busy = acting || st.busy.is_some();
        let valid = common::valid_device_name(self.name.trim());
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy && valid, egui::Button::new(RichText::new(tr("Подключиться", "Join")).strong()).fill(ACCENT.linear_multiply(0.5))).clicked() {
                let inv = self.invite.trim().to_string();
                let req = Request::Enroll { server: self.server.trim().into(), name: self.name.trim().into(), net: if inv.is_empty() { self.pick.clone() } else { None }, invite: (!inv.is_empty()).then_some(inv) };
                self.act(ctx, req, tr("Готово, устройство в сети", "Done, the device is in the network"));
            }
            if busy {
                ui.spinner();
                ui.label(st.busy.clone().unwrap_or_else(|| tr("Подключаюсь…", "Joining…").into()));
            }
            if !st.networks.is_empty() && ui.button(tr("Отмена", "Cancel")).clicked() {
                self.adding = false;
            }
        });
        if !valid {
            ui.label(RichText::new(tr("Имя: латиница, цифры, точка, дефис, подчёркивание", "Name: Latin letters, digits, dot, dash, underscore")).color(DANGER));
        }
    }

    fn network(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, n: &NetStatus, peers: &[Peer], logs: &[String], acting: bool) {
        let title = match (n.state, n.want_up) {
            (_, false) => tr("Отключено", "Disconnected"),
            (ConnState::Running, _) => tr("В сети", "Connected"),
            (ConnState::Starting, _) => tr("Подключаюсь…", "Connecting…"),
            (ConnState::Error, _) => tr("Ошибка подключения", "Connection error"),
            _ => tr("Запускаю…", "Starting…"),
        };
        ui.horizontal(|ui| {
            dot(ui, state_color(n), 7.0);
            ui.label(RichText::new(title).size(22.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, req) = if n.want_up {
                    (tr("Отключиться", "Disconnect"), Request::Disconnect { net: n.slug.clone() })
                } else {
                    (tr("Подключиться", "Connect"), Request::Connect { net: n.slug.clone() })
                };
                if ui.add_enabled(!acting, egui::Button::new(text)).clicked() {
                    self.act(ctx, req, "");
                }
            });
        });
        if let (Some(e), true) = (&n.error, n.want_up) {
            ui.label(RichText::new(e).color(DANGER));
        }
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).rounding(8.0).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("info").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new(tr("Ваш адрес", "Your address")).color(MUTED));
                copy_label(ui, &n.ip);
                ui.end_row();
                ui.label(RichText::new(tr("Сеть", "Network")).color(MUTED));
                ui.label(format!("{}  ({})", n.title, n.cidr));
                ui.end_row();
                ui.label(RichText::new(tr("Устройство", "Device")).color(MUTED));
                ui.label(format!("{}  ·  {}", n.name, n.dev));
                ui.end_row();
                if let Some(t) = n.cert_not_after {
                    ui.label(RichText::new(tr("Сертификат", "Certificate")).color(MUTED));
                    ui.label(trf!("до {}, продлится сам", "until {}, renews itself", date(t)));
                    ui.end_row();
                }
                ui.label(RichText::new(tr("Синхронизация", "Sync")).color(MUTED));
                match (&n.sync_error, n.last_sync) {
                    (Some(e), _) => ui.label(RichText::new(e).color(DANGER)),
                    (None, Some(t)) => ui.label(ago(t)),
                    _ => ui.label(tr("ещё не было", "not yet")),
                };
                ui.end_row();
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Peers, trf!("Участники ({})", "Members ({})", peers.len()));
            ui.selectable_value(&mut self.tab, Tab::Logs, tr("Журнал", "Log"));
        });
        ui.separator();
        let h = (ui.available_height() - 64.0).max(80.0);
        match self.tab {
            Tab::Peers => {
                egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                    if peers.is_empty() {
                        ui.label(RichText::new(tr("Пока никого", "Nobody yet")).color(MUTED));
                    }
                    for p in peers {
                        ui.horizontal(|ui| {
                            dot(ui, if p.online { ACCENT } else { Color32::from_gray(90) }, 4.0);
                            ui.label(RichText::new(&p.name).strong());
                            if p.ip == n.ip {
                                ui.label(RichText::new(tr("это вы", "you")).small().color(MUTED));
                            } else if p.role == "lighthouse" {
                                ui.label(RichText::new(tr("маяк", "lighthouse")).small().color(MUTED));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| copy_label(ui, &p.ip));
                        });
                    }
                });
                ui.label(
                    RichText::new(tr(
                        "Нажмите на адрес, чтобы скопировать. Игры подключайте по IP: поиск «в локальной сети» через Nebula не работает",
                        "Click an address to copy it. Connect games by IP: LAN discovery does not work over Nebula",
                    ))
                    .small()
                    .color(MUTED),
                );
            }
            Tab::Logs => {
                egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                    for l in logs {
                        ui.label(RichText::new(l).monospace().size(11.0));
                    }
                });
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(!acting, egui::Button::new(tr("Синхронизировать", "Sync now"))).clicked() {
                    self.act(ctx, Request::Sync { net: Some(n.slug.clone()) }, tr("Синхронизировано", "Synced"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let armed = self.confirm_leave.map(|t| t.elapsed() < Duration::from_secs(5)).unwrap_or(false);
                    let text = if armed {
                        RichText::new(tr("Точно выйти? Нажмите ещё раз", "Really leave? Click again")).color(DANGER)
                    } else {
                        RichText::new(tr("Выйти из сети", "Leave network"))
                    };
                    if ui.add_enabled(!acting, egui::Button::new(text)).clicked() {
                        if armed {
                            self.confirm_leave = None;
                            self.act(ctx, Request::Leave { net: n.slug.clone() }, tr("Устройство вышло из сети", "The device left the network"));
                        } else {
                            self.confirm_leave = Some(Instant::now());
                        }
                    }
                });
            });
        });
    }
}
