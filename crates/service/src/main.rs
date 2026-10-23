//! nebula-client-service: служба (Windows) или демон под systemd (Linux) и консольные команды к ней.

mod api;
mod core;
mod logs;
mod nebula;
mod plat;
mod update;

use common::{tr, trf, Request, UpdateMode};

const HELP_RU: &str = "nebula-client-service КОМАНДА

  run                         работать в консоли (под systemd в Linux так и запускается)
  install                     Windows: поставить и запустить службу (нужен администратор)
  uninstall [--purge]         Windows: удалить службу (--purge стирает и данные всех сетей)

  status                      сети устройства и их состояние
  join --name ИМЯ [--invite КОД] [--net СЕТЬ] [--server URL]
                              войти в сеть (без инвайта, если на панели свободная регистрация)
  open [--server URL]         какие сети на панели пускают без инвайта
  peers СЕТЬ                  участники сети
  connect СЕТЬ | disconnect СЕТЬ
  sync [СЕТЬ]                 продлить сертификат и обновить конфиг сейчас
  leave СЕТЬ                  выйти из сети и стереть её ключ
  dns СЕТЬ on|off             спрашивать имена сети (например, *.neb) у её DNS, пока сеть подключена
  logs                        журнал службы и nebula
  update [check|install]      проверить обновления / поставить найденное (Windows)
  update auto|notify|off [--pre yes|no]
                              ставить самому, только сообщать или не проверять; --pre: тестовые версии";

const HELP_EN: &str = "nebula-client-service COMMAND

  run                         run in the foreground (this is how systemd starts it on Linux)
  install                     Windows: install and start the service (needs administrator)
  uninstall [--purge]         Windows: remove the service (--purge also deletes all network data)

  status                      networks of this device and their state
  join --name NAME [--invite CODE] [--net NET] [--server URL]
                              join a network (no invite needed if the panel allows open sign-up)
  open [--server URL]         which networks on the panel accept devices without an invite
  peers NET                   network members
  connect NET | disconnect NET
  sync [NET]                  renew the certificate and refresh the config now
  leave NET                   leave the network and delete its key
  dns NET on|off              resolve the network's names (e.g. *.neb) with its DNS while connected
  logs                        service and nebula log
  update [check|install]      check for updates / install the one found (Windows)
  update auto|notify|off [--pre yes|no]
                              install automatically, only notify or never check; --pre: test versions";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let code = match cmd {
        #[cfg(windows)]
        "service" => {
            winsvc::dispatch();
            0
        }
        "run" => {
            let core = core::Core::new(common::data_dir());
            core.start();
            eprintln!("Nebula Client {}: {}", env!("CARGO_PKG_VERSION"), core.dir().display());
            #[cfg(unix)]
            {
                let c = core.clone();
                // systemd останавливает SIGTERM: гасим nebula и выходим
                std::thread::spawn(move || {
                    unix_wait_term();
                    c.shutdown();
                    std::process::exit(0);
                });
            }
            plat::serve(core);
            0
        }
        #[cfg(windows)]
        "install" => report(winsvc::install()),
        #[cfg(windows)]
        "uninstall" => report(winsvc::uninstall(args.iter().any(|a| a == "--purge"))),
        "" | "help" | "-h" | "--help" => {
            println!("{}", tr(HELP_RU, HELP_EN));
            0
        }
        _ => client(cmd, &args[1..]),
    };
    std::process::exit(code);
}

#[cfg(unix)]
fn unix_wait_term() {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        // сигналы блокируются во всех потоках (маска наследуется), ждём их здесь
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        let mut sig = 0;
        libc::sigwait(&set, &mut sig);
    }
}

#[cfg(windows)]
fn report(r: Result<String, String>) -> i32 {
    match r {
        Ok(m) => {
            println!("{m}");
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", tr("Ошибка", "Error"));
            1
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn positional(args: &[String]) -> Option<String> {
    args.first().filter(|a| !a.starts_with("--")).cloned()
}

/// Аргумент, который не флаг и не значение флага: `join http://...` вместо `join --server http://...`.
/// Такое не пропускаем молча, иначе устройство уйдёт на сервер по умолчанию.
fn stray(args: &[String]) -> Option<&String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a.starts_with("--") {
            it.next();
        } else {
            return Some(a);
        }
    }
    None
}

fn client(cmd: &str, args: &[String]) -> i32 {
    if matches!(cmd, "join" | "enroll" | "open") {
        if let Some(a) = stray(args) {
            eprintln!("{}", trf!("Непонятный аргумент «{}». Пример: join --name pc --server URL", "Unexpected argument \"{}\". Example: join --name pc --server URL", a));
            return 2;
        }
    }
    let net = || positional(args).ok_or(());
    let server = || flag(args, "--server").unwrap_or_else(|| common::DEFAULT_SERVER.into());
    let req = match cmd {
        "status" => Ok(Request::Status),
        "logs" => Ok(Request::Logs),
        "open" => Ok(Request::OpenNets { server: server() }),
        "join" | "enroll" => Ok(Request::Enroll {
            server: server(),
            name: flag(args, "--name").unwrap_or_else(common::default_device_name),
            invite: flag(args, "--invite"),
            net: flag(args, "--net"),
        }),
        "peers" => net().map(|net| Request::Peers { net }),
        "connect" => net().map(|net| Request::Connect { net }),
        "disconnect" => net().map(|net| Request::Disconnect { net }),
        "leave" => net().map(|net| Request::Leave { net }),
        "sync" => Ok(Request::Sync { net: positional(args) }),
        "dns" => match (positional(args), args.get(1).map(String::as_str)) {
            (Some(net), Some("on")) => Ok(Request::SetDns { net, on: true }),
            (Some(net), Some("off")) => Ok(Request::SetDns { net, on: false }),
            _ => {
                eprintln!("dns {} on|off", tr("СЕТЬ", "NET"));
                return 2;
            }
        },
        "update" => match positional(args).as_deref() {
            None | Some("check") => Ok(Request::UpdateCheck),
            Some("install") => Ok(Request::UpdateInstall),
            Some(m @ ("auto" | "notify" | "off")) => {
                let mode = match m {
                    "auto" => UpdateMode::Auto,
                    "notify" => UpdateMode::Notify,
                    _ => UpdateMode::Off,
                };
                // без --pre оставляем как было
                let cur = common::call(&Request::Status).ok().and_then(|r| r.status).map(|s| s.update.prerelease).unwrap_or(true);
                let pre = flag(args, "--pre").map(|v| matches!(v.as_str(), "yes" | "on" | "1" | "true")).unwrap_or(cur);
                Ok(Request::UpdateSet { mode, prerelease: pre })
            }
            Some(_) => Err(()),
        },
        _ => {
            eprintln!("{}", tr(HELP_RU, HELP_EN));
            return 2;
        }
    };
    let Ok(req) = req else {
        eprintln!("{}", tr("Укажите код сети, список: status", "Specify the network code, see: status"));
        return 2;
    };
    match common::call(&req) {
        Ok(r) => {
            if let Some(e) = &r.error {
                eprintln!("{}: {e}", tr("Ошибка", "Error"));
            }
            match cmd {
                "peers" => r.peers.unwrap_or_default().iter().for_each(|p| {
                    println!("{:<16} {}{}", p.ip, p.name, if p.online { tr("  в сети", "  online") } else { "" })
                }),
                "logs" => r.logs.unwrap_or_default().iter().for_each(|l| println!("{l}")),
                "open" => r.open.unwrap_or_default().iter().for_each(|o| println!("{:<16} {}", o.slug, o.title)),
                "update" => {
                    let st = r.status.unwrap_or_default();
                    let u = st.update;
                    println!("{} {}, {:?}{}", tr("версия", "version"), st.service_version, u.mode, if u.prerelease { tr(", с тестовыми", ", with test versions") } else { "" });
                    match (&u.available, u.installing) {
                        (Some(v), true) => println!("{}", trf!("ставлю {}", "installing {}", v)),
                        (Some(v), false) => println!("{}  {}", trf!("доступна {}", "{} is available", v), u.page.unwrap_or_default()),
                        (None, _) if u.error.is_none() => println!("{}", tr("обновлений нет", "up to date")),
                        _ => {}
                    }
                    if let Some(e) = u.error {
                        println!("{}: {e}", tr("ошибка", "error"));
                    }
                }
                _ => {
                    let st = r.status.unwrap_or_default();
                    if st.networks.is_empty() {
                        println!("{}", tr("Сетей пока нет. Войти: join --name ИМЯ [--invite КОД]", "No networks yet. Join: join --name NAME [--invite CODE]"));
                    }
                    for n in &st.networks {
                        println!(
                            "{:<12} {:<16} {:<10} {:?}{}{}",
                            n.slug,
                            n.ip,
                            n.dev,
                            n.state,
                            if n.want_up { "" } else { tr(" (выключена)", " (disabled)") },
                            n.error.as_ref().map(|e| format!("  {e}")).unwrap_or_default()
                        );
                        if let Some(d) = &n.dns {
                            let zones = d.domains.iter().map(|z| format!("*.{z}")).collect::<Vec<_>>().join(", ");
                            println!("{:<12} DNS {} -> {}{}", "", zones, d.server, if n.dns_on { "" } else { tr(" (выключен)", " (off)") });
                        }
                    }
                    if let Some(b) = st.busy {
                        println!("… {b}");
                    }
                }
            }
            if r.ok { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("{}: {e}", tr("Ошибка", "Error"));
            1
        }
    }
}

#[cfg(windows)]
mod winsvc {
    use crate::{core, plat};
    use common::{tr, trf, SERVICE_DISPLAY, SERVICE_NAME};
    use std::ffi::OsString;
    use std::sync::mpsc;
    use std::time::Duration;
    use windows_service::service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState,
        ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{define_windows_service, service_dispatcher};

    define_windows_service!(ffi_service_main, service_main);

    pub fn dispatch() {
        let _ = service_dispatcher::start(SERVICE_NAME, ffi_service_main);
    }

    fn service_main(_args: Vec<OsString>) {
        let (tx, rx) = mpsc::channel();
        let handler = move |ev| match ev {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(h) = service_control_handler::register(SERVICE_NAME, handler) else { return };
        let set = |state, accept| {
            let _ = h.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: accept,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::from_secs(10),
                process_id: None,
            });
        };
        let core = core::Core::new(common::data_dir());
        core.start();
        let c = core.clone();
        std::thread::spawn(move || plat::serve(c));
        set(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN);
        let _ = rx.recv();
        set(ServiceState::StopPending, ServiceControlAccept::empty());
        core.shutdown();
        set(ServiceState::Stopped, ServiceControlAccept::empty());
    }

    fn no_admin(e: impl std::fmt::Display) -> String {
        trf!("нет доступа к диспетчеру служб, запустите от имени администратора ({})", "no access to the service manager, run as administrator ({})", e)
    }

    pub fn install() -> Result<String, String> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE).map_err(no_admin)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let info = ServiceInfo {
            name: SERVICE_NAME.into(),
            display_name: SERVICE_DISPLAY.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe.clone(),
            launch_arguments: vec!["service".into()],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::CHANGE_CONFIG;
        let svc = match mgr.open_service(SERVICE_NAME, access) {
            Ok(s) => {
                s.change_config(&info).map_err(|e| e.to_string())?;
                s
            }
            Err(_) => mgr.create_service(&info, access).map_err(|e| e.to_string())?,
        };
        let _ = svc.set_description(tr(
            "Подключение к сетям Nebula: держит nebula.exe запущенной и продлевает сертификаты",
            "Connects to Nebula networks: keeps nebula.exe running and renews certificates",
        ));
        // упала: перезапуск через 5 с, 10 с, потом через минуту
        let _ = std::process::Command::new("sc.exe")
            .args(["failure", SERVICE_NAME, "reset=", "86400", "actions=", "restart/5000/restart/10000/restart/60000"])
            .output();
        if svc.query_status().map(|s| s.current_state != ServiceState::Running).unwrap_or(true) {
            svc.start::<&str>(&[]).map_err(|e| trf!("служба создана, но не запустилась: {}", "the service was created but did not start: {}", e))?;
        }
        Ok(trf!("Служба {} установлена и запущена ({})", "Service {} installed and started ({})", SERVICE_DISPLAY, exe.display()))
    }

    pub fn uninstall(purge: bool) -> Result<String, String> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(no_admin)?;
        if let Ok(svc) = mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) {
            if svc.query_status().map(|s| s.current_state != ServiceState::Stopped).unwrap_or(false) {
                let _ = svc.stop();
                for _ in 0..30 {
                    if svc.query_status().map(|s| s.current_state == ServiceState::Stopped).unwrap_or(true) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
            svc.delete().map_err(|e| e.to_string())?;
        }
        plat::remove_firewall();
        let _ = plat::set_dns(&[]);
        if purge {
            // ключи сейчас сотрутся: пусть панели сразу отзовут сертификаты этого устройства
            let nets = common::data_dir().join("networks");
            for e in std::fs::read_dir(&nets).into_iter().flatten().flatten() {
                let Ok(text) = std::fs::read_to_string(e.path().join("device.json")) else { continue };
                let Ok(d) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                if let (Some(server), Some(token)) = (d["server"].as_str(), d["token"].as_str()) {
                    let _ = crate::api::post(server, "/v1/leave", Some(token), serde_json::json!({}));
                }
            }
            let _ = std::fs::remove_dir_all(common::data_dir());
            return Ok(tr("Служба удалена, ключи, сертификаты и Nebula стёрты", "Service removed, keys, certificates and Nebula deleted").into());
        }
        Ok(tr("Служба удалена. Данные остались в ProgramData\\NebulaClient, стереть: uninstall --purge", "Service removed. Data is kept in ProgramData\\NebulaClient, to delete it: uninstall --purge").into())
    }
}
