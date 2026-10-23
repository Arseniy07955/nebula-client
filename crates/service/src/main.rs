//! nebula-client-service.exe: служба Windows и консольные команды к ней.
//!
//!   nebula-client-service install            поставить и запустить службу (нужен администратор)
//!   nebula-client-service uninstall [--purge] удалить службу (--purge стирает и данные)
//!   nebula-client-service run                работать в консоли, для отладки
//!   nebula-client-service status | peers | logs | connect | disconnect | sync | leave
//!   nebula-client-service enroll --name ИМЯ [--server URL] [--invite КОД]

mod api;
mod core;
mod logs;
mod nebula;
mod pipe;
mod sys;

use common::{Request, SERVICE_DISPLAY, SERVICE_NAME};
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let code = match cmd {
        "service" => {
            let _ = service_dispatcher::start(SERVICE_NAME, ffi_service_main);
            0
        }
        "run" => {
            let core = core::Core::new(common::data_dir());
            core.start();
            println!("Nebula Client работает в консоли, данные в {}. Ctrl+C для выхода", core.dir().display());
            pipe::serve(core);
            0
        }
        "install" => report(install()),
        "uninstall" => report(uninstall(args.iter().any(|a| a == "--purge"))),
        "" | "help" | "-h" | "--help" => {
            println!("{}", include_str!("main.rs").lines().skip(2).take(5).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
            0
        }
        _ => client(cmd, &args[1..]),
    };
    std::process::exit(code);
}

fn report(r: Result<String, String>) -> i32 {
    match r {
        Ok(m) => {
            println!("{m}");
            0
        }
        Err(e) => {
            eprintln!("Ошибка: {e}");
            1
        }
    }
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
    std::thread::spawn(move || pipe::serve(c));
    set(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN);
    let _ = rx.recv();
    set(ServiceState::StopPending, ServiceControlAccept::empty());
    core.shutdown();
    set(ServiceState::Stopped, ServiceControlAccept::empty());
}

fn install() -> Result<String, String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .map_err(|e| format!("нет доступа к диспетчеру служб, запустите от имени администратора ({e})"))?;
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
    let svc = match mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::CHANGE_CONFIG) {
        Ok(s) => {
            s.change_config(&info).map_err(|e| e.to_string())?;
            s
        }
        Err(_) => mgr
            .create_service(&info, ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::CHANGE_CONFIG)
            .map_err(|e| format!("не удалось создать службу: {e}"))?,
    };
    let _ = svc.set_description("Подключение к сети Nebula: держит nebula.exe запущенной и продлевает сертификат");
    // упала: перезапуск через 5 с, 10 с, потом через минуту
    let _ = std::process::Command::new("sc.exe")
        .args(["failure", SERVICE_NAME, "reset=", "86400", "actions=", "restart/5000/restart/10000/restart/60000"])
        .output();
    if svc.query_status().map(|s| s.current_state != ServiceState::Running).unwrap_or(true) {
        svc.start::<&str>(&[]).map_err(|e| format!("служба создана, но не запустилась: {e}"))?;
    }
    Ok(format!("Служба {SERVICE_DISPLAY} установлена и запущена ({})", exe.display()))
}

fn uninstall(purge: bool) -> Result<String, String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("нет доступа к диспетчеру служб, запустите от имени администратора ({e})"))?;
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
        svc.delete().map_err(|e| format!("не удалось удалить службу: {e}"))?;
    }
    sys::remove_firewall();
    if purge {
        let _ = std::fs::remove_dir_all(common::data_dir());
        return Ok("Служба удалена, ключ, сертификат и Nebula стёрты".into());
    }
    Ok("Служба удалена. Данные остались в папке ProgramData\\NebulaClient, чтобы стереть их: uninstall --purge".into())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn client(cmd: &str, args: &[String]) -> i32 {
    let req = match cmd {
        "status" => Request::Status,
        "peers" => Request::Peers,
        "logs" => Request::Logs,
        "connect" => Request::Connect,
        "disconnect" => Request::Disconnect,
        "sync" => Request::Sync,
        "leave" => Request::Leave,
        "enroll" => Request::Enroll {
            server: flag(args, "--server").unwrap_or_else(|| common::DEFAULT_SERVER.into()),
            name: flag(args, "--name").unwrap_or_else(common::default_device_name),
            invite: flag(args, "--invite"),
        },
        _ => {
            eprintln!("Неизвестная команда {cmd}. Список: nebula-client-service help");
            return 2;
        }
    };
    match common::call(&req) {
        Ok(r) => {
            if let Some(e) = &r.error {
                eprintln!("Ошибка: {e}");
            }
            match cmd {
                "peers" => r.peers.unwrap_or_default().iter().for_each(|p| println!("{:<16} {}{}", p.ip, p.name, if p.online { "  в сети" } else { "" })),
                "logs" => r.logs.unwrap_or_default().iter().for_each(|l| println!("{l}")),
                _ => println!("{}", serde_json::to_string_pretty(&r.status).unwrap_or_default()),
            }
            if r.ok { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("Ошибка: {e}");
            1
        }
    }
}
