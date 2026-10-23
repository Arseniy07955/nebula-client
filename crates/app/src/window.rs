//! Окно на egui. Отдельный процесс: закрыли окно — процесс завершился и отдал память.
//! Данные спрашиваются у службы в фоновом потоке и только пока окно открыто.

use crate::icons::{self, Look};
use crate::sys;
use common::{ConnState, Peer, Request, Status};
use eframe::egui::{self, Color32, RichText, Sense};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(0x2e, 0xc2, 0x7e);
const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa8);
const DANGER: Color32 = Color32::from_rgb(0xe0, 0x4f, 0x4f);

#[derive(Default)]
struct Shared {
    status: Option<Status>,
    svc_err: Option<String>,
    peers: Vec<Peer>,
    logs: Vec<String>,
    /// Итог последнего действия: (успех, текст).
    note: Option<(bool, String)>,
    acting: bool,
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
    tab: Tab,
    server: String,
    name: String,
    invite: String,
    confirm_leave: Option<Instant>,
}

pub fn run() {
    if sys::already_running("Local\\NebulaClientWindow") {
        sys::focus_existing_window();
        return;
    }
    let icon = egui::IconData { rgba: icons::rgba(Look::Up), width: icons::SIZE, height: icons::SIZE };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(sys::WINDOW_TITLE)
            .with_inner_size([440.0, 640.0])
            .with_min_inner_size([360.0, 480.0])
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    let _ = eframe::run_native(sys::WINDOW_TITLE, opts, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))));
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
        spawn_poller(ctx.clone(), sh.clone(), want_logs.clone(), poke.clone());
        App {
            sh,
            want_logs,
            poke,
            tab: Tab::Peers,
            server: common::DEFAULT_SERVER.into(),
            name: common::default_device_name(),
            invite: String::new(),
            confirm_leave: None,
        }
    }

    fn act(&self, ctx: &egui::Context, req: Request, ok_text: &'static str) {
        let sh = self.sh.clone();
        let poke = self.poke.clone();
        let ctx = ctx.clone();
        sh.lock().unwrap().acting = true;
        std::thread::spawn(move || {
            let note = match common::call(&req) {
                Ok(r) if r.ok => (true, ok_text.to_string()),
                Ok(r) => (false, r.error.unwrap_or_else(|| "ошибка".into())),
                Err(e) => (false, e.to_string()),
            };
            let mut s = sh.lock().unwrap();
            s.acting = false;
            s.note = if note.1.is_empty() { None } else { Some(note) };
            drop(s);
            poke.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }
}

fn spawn_poller(ctx: egui::Context, sh: Arc<Mutex<Shared>>, want_logs: Arc<AtomicBool>, poke: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut last_peers = Instant::now() - Duration::from_secs(60);
        let mut last_status = Instant::now() - Duration::from_secs(60);
        loop {
            let poked = poke.swap(false, Ordering::SeqCst);
            if poked || last_status.elapsed() >= Duration::from_millis(1500) {
                last_status = Instant::now();
                let r = common::call(&Request::Status);
                let enrolled = {
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
                    s.status.as_ref().map(|s| s.enrolled).unwrap_or(false)
                };
                if enrolled && (poked || last_peers.elapsed() >= Duration::from_secs(5)) {
                    last_peers = Instant::now();
                    if let Ok(r) = common::call(&Request::Peers) {
                        sh.lock().unwrap().peers = r.peers.unwrap_or_default();
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
    format!("{d:02}.{m:02}.{y}")
}

fn ago(ts: i64) -> String {
    let s = (common::now() - ts).max(0);
    match s {
        0..=59 => "только что".into(),
        60..=3599 => format!("{} мин назад", s / 60),
        3600..=86399 => format!("{} ч назад", s / 3600),
        _ => format!("{} дн назад", s / 86400),
    }
}

fn dot(ui: &mut egui::Ui, color: Color32, r: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(r * 2.0 + 4.0, r * 2.0 + 4.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), r, color);
}

fn copy_label(ui: &mut egui::Ui, text: &str, mono: bool) {
    let rt = if mono { RichText::new(text).monospace() } else { RichText::new(text) };
    let resp = ui.add(egui::Label::new(rt).sense(Sense::click())).on_hover_text("Нажмите, чтобы скопировать");
    if resp.clicked() {
        ui.output_mut(|o| o.copied_text = text.to_string());
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (status, svc_err, peers, logs, note, acting) = {
            let s = self.sh.lock().unwrap();
            (s.status.clone(), s.svc_err.clone(), s.peers.clone(), s.logs.clone(), s.note.clone(), s.acting)
        };
        self.want_logs.store(self.tab == Tab::Logs, Ordering::SeqCst);

        egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).inner_margin(18.0)).show(ctx, |ui| {
            if let Some(e) = svc_err {
                self.no_service(ui, &e);
            } else if let Some(st) = status {
                if st.enrolled {
                    self.connected(ui, ctx, &st, &peers, &logs, acting);
                } else {
                    self.enroll_form(ui, ctx, &st, acting);
                }
            } else {
                ui.spinner();
            }
            if let Some((ok, text)) = note {
                ui.add_space(8.0);
                ui.label(RichText::new(text).color(if ok { ACCENT } else { DANGER }));
            }
        });
        ctx.request_repaint_after(Duration::from_secs(2));
    }
}

impl App {
    fn no_service(&self, ui: &mut egui::Ui, err: &str) {
        ui.heading("Служба не запущена");
        ui.label(RichText::new(err).color(MUTED));
        ui.add_space(8.0);
        ui.label("Nebula Client работает через службу Windows: она держит подключение, даже когда окно закрыто. Её нужно поставить один раз, Windows спросит права администратора.");
        ui.add_space(8.0);
        if sys::service_exe().exists() {
            if ui.button("Установить службу").clicked() {
                sys::install_service();
                self.poke.store(true, Ordering::SeqCst);
            }
        } else {
            ui.label(RichText::new("Рядом с программой нет nebula-client-service.exe, переустановите Nebula Client").color(DANGER));
        }
    }

    fn enroll_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, st: &Status, acting: bool) {
        ui.heading("Подключение к сети");
        ui.label(RichText::new("Ключ устройства создаётся на этом ПК и никуда не уходит. Панель выдаст адрес и сертификат.").color(MUTED));
        ui.add_space(10.0);
        ui.label("Имя устройства");
        ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
        ui.label(RichText::new("Латиница, цифры, точка, дефис. Его увидят другие участники").small().color(MUTED));
        ui.add_space(4.0);
        ui.label("Инвайт");
        ui.add(egui::TextEdit::singleline(&mut self.invite).hint_text("NF-XXXX-XXXX-XXXX, если сеть закрытая").desired_width(f32::INFINITY));
        egui::CollapsingHeader::new("Сервер").default_open(self.server != common::DEFAULT_SERVER).show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.server).desired_width(f32::INFINITY));
        });
        ui.add_space(10.0);
        let busy = acting || st.busy.is_some();
        let valid = common::valid_device_name(self.name.trim());
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy && valid, egui::Button::new(RichText::new("Подключиться").strong()).fill(ACCENT.linear_multiply(0.5))).clicked() {
                let inv = self.invite.trim().to_string();
                self.act(ctx, Request::Enroll { server: self.server.trim().into(), name: self.name.trim().into(), invite: (!inv.is_empty()).then_some(inv) }, "Готово, устройство в сети");
            }
            if busy {
                ui.spinner();
                ui.label(st.busy.clone().unwrap_or_else(|| "Подключаюсь…".into()));
            }
        });
        if !valid {
            ui.label(RichText::new("Имя: латиница, цифры, точка, дефис, подчёркивание").color(DANGER));
        }
    }

    fn connected(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, st: &Status, peers: &[Peer], logs: &[String], acting: bool) {
        let look = Look::of(Some(st.state));
        let [r, g, b] = look.rgb();
        let title = match (st.state, st.want_up) {
            (ConnState::Running, _) => "В сети",
            (ConnState::Starting, _) => "Подключаюсь…",
            (ConnState::Error, true) => "Ошибка подключения",
            (_, false) => "Отключено",
            _ => "Запускаю…",
        };
        ui.horizontal(|ui| {
            dot(ui, Color32::from_rgb(r, g, b), 7.0);
            ui.label(RichText::new(title).size(22.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, req, ok) = if st.want_up {
                    ("Отключиться", Request::Disconnect, "")
                } else {
                    ("Подключиться", Request::Connect, "")
                };
                if ui.add_enabled(!acting, egui::Button::new(text)).clicked() {
                    self.act(ctx, req, ok);
                }
            });
        });
        if let Some(e) = &st.error {
            if st.want_up {
                ui.label(RichText::new(e).color(DANGER));
            }
        }
        ui.add_space(6.0);
        egui::Frame::group(ui.style()).rounding(8.0).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("info").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new("Ваш адрес").color(MUTED));
                copy_label(ui, &st.ip, true);
                ui.end_row();
                ui.label(RichText::new("Сеть").color(MUTED));
                ui.label(format!("{}  ({})", st.network, st.cidr));
                ui.end_row();
                ui.label(RichText::new("Устройство").color(MUTED));
                ui.label(&st.name);
                ui.end_row();
                if let Some(t) = st.cert_not_after {
                    ui.label(RichText::new("Сертификат").color(MUTED));
                    ui.label(format!("до {}, продлится сам", date(t)));
                    ui.end_row();
                }
                ui.label(RichText::new("Синхронизация").color(MUTED));
                match (&st.sync_error, st.last_sync) {
                    (Some(e), _) => ui.label(RichText::new(e).color(DANGER)),
                    (None, Some(t)) => ui.label(ago(t)),
                    _ => ui.label("ещё не было"),
                };
                ui.end_row();
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Peers, format!("Участники ({})", peers.len()));
            ui.selectable_value(&mut self.tab, Tab::Logs, "Журнал");
        });
        ui.separator();
        let bottom = 44.0;
        let h = (ui.available_height() - bottom).max(80.0);
        match self.tab {
            Tab::Peers => {
                egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                    if peers.is_empty() {
                        ui.label(RichText::new("Пока никого").color(MUTED));
                    }
                    for p in peers {
                        ui.horizontal(|ui| {
                            dot(ui, if p.online { ACCENT } else { Color32::from_gray(90) }, 4.0);
                            let me = p.ip == st.ip;
                            ui.label(RichText::new(&p.name).strong());
                            if me {
                                ui.label(RichText::new("это вы").small().color(MUTED));
                            } else if p.role == "lighthouse" {
                                ui.label(RichText::new("маяк").small().color(MUTED));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| copy_label(ui, &p.ip, true));
                        });
                    }
                });
                ui.label(RichText::new("Нажмите на адрес, чтобы скопировать. Игры подключайте по IP: поиск «в локальной сети» через Nebula не работает").small().color(MUTED));
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
                if ui.add_enabled(!acting, egui::Button::new("Синхронизировать")).clicked() {
                    self.act(ctx, Request::Sync, "Синхронизировано");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let armed = self.confirm_leave.map(|t| t.elapsed() < Duration::from_secs(5)).unwrap_or(false);
                    let text = if armed { RichText::new("Точно выйти? Нажмите ещё раз").color(DANGER) } else { RichText::new("Выйти из сети") };
                    if ui.add_enabled(!acting, egui::Button::new(text)).clicked() {
                        if armed {
                            self.confirm_leave = None;
                            self.act(ctx, Request::Leave, "Устройство вышло из сети");
                        } else {
                            self.confirm_leave = Some(Instant::now());
                        }
                    }
                });
            });
        });
    }
}
