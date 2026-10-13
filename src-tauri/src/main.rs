// На релизной сборке Windows не открывать консольное окно вместе с GUI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_blocking;

// Алиас, чтобы весь остальной код ниже мог писать `core::vpn::...`,
// `core::config::...` и т.д., как раньше, хотя эта логика теперь живёт
// в отдельном крейте netguardian-core (shared с native-host для
// браузерного расширения — см. README).
use netguardian_core as core;

use std::sync::Mutex;
use tokio::sync::Mutex as AsyncMutex;

use tauri::{Manager, State};

use crate::core::config::AppConfig;
use crate::core::sites::SiteMonitor;
use crate::core::traffic::TrafficMonitor;
use crate::core::vpn::VpnStatus;

// pub(crate), чтобы app_blocking (фоновый вотчер процессов) мог
// читать список заблокированных приложений и настройки определения VPN.
pub(crate) struct AppState {
    pub(crate) config: Mutex<AppConfig>,
    pub(crate) traffic: Mutex<TrafficMonitor>,
    // tokio::sync::Mutex: SiteMonitor::tick() делает .await внутри, а guard
    // std::sync::Mutex нельзя держать через точку await (не Send).
    pub(crate) sites: AsyncMutex<SiteMonitor>,
}

// ---------- Tauri-команды, вызываемые из UI (ui/js/app.js) ----------

#[tauri::command]
fn get_config(state: State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(state: State<AppState>, cfg: AppConfig) -> Result<(), String> {
    core::config::save_config(&cfg).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

#[tauri::command]
async fn get_vpn_status(state: State<'_, AppState>) -> Result<VpnStatus, String> {
    let (order, hints, exclude_hints) = {
        let cfg = state.config.lock().unwrap();
        (
            cfg.vpn_detection_order.clone(),
            cfg.vpn_adapter_hints.clone(),
            cfg.vpn_adapter_exclude_hints.clone(),
        )
    };
    Ok(core::vpn::detect_vpn_status(&order, &hints, &exclude_hints).await)
}

#[tauri::command]
fn list_adapters() -> Result<Vec<core::vpn::VpnAdapterInfo>, String> {
    core::vpn::list_adapters().map_err(|e| e.to_string())
}

#[tauri::command]
fn traffic_sample(state: State<AppState>) {
    state.traffic.lock().unwrap().sample();
}

#[tauri::command]
fn traffic_interfaces(state: State<AppState>) -> Vec<String> {
    state.traffic.lock().unwrap().interface_names()
}

#[derive(serde::Serialize)]
struct TrafficSnapshot {
    series: Vec<core::traffic::Sample>,
    last_hour_recv: u64,
    last_hour_sent: u64,
    since_boot_recv: u64,
    since_boot_sent: u64,
}

#[tauri::command]
fn traffic_snapshot(state: State<AppState>, interface: String) -> TrafficSnapshot {
    let t = state.traffic.lock().unwrap();
    let (lh_recv, lh_sent) = t.get_last_hour_bytes(&interface);
    let (sb_recv, sb_sent) = t.get_since_boot_bytes(&interface);
    TrafficSnapshot {
        series: t.get_realtime_series(&interface),
        last_hour_recv: lh_recv,
        last_hour_sent: lh_sent,
        since_boot_recv: sb_recv,
        since_boot_sent: sb_sent,
    }
}

#[tauri::command]
async fn get_public_ip() -> Result<core::ip_info::PublicIpInfo, String> {
    core::ip_info::get_public_ip_info().await
}

#[tauri::command]
async fn sites_tick(state: State<'_, AppState>) -> Result<(), String> {
    // Блокировка держится только на время выбора адресов и записи результатов;
    // сами проверки идут параллельно и без блокировки (иначе из-за таймаутов
    // интерфейс зависал бы, пока проверяются все сайты по очереди).
    let (client, due) = {
        let mut sm = state.sites.lock().await;
        (sm.client(), sm.take_due())
    };
    if due.is_empty() {
        return Ok(());
    }
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut set = tokio::task::JoinSet::new();
    for url in due {
        let client = client.clone();
        let limit = limit.clone();
        set.spawn(async move {
            let _permit = limit.acquire_owned().await;
            core::sites::check_url(client, url).await
        });
    }
    while let Some(res) = set.join_next().await {
        if let Ok(outcome) = res {
            state.sites.lock().await.apply(outcome);
        }
    }
    Ok(())
}

#[tauri::command]
async fn sites_list(state: State<'_, AppState>) -> Result<Vec<core::sites::SiteStatus>, String> {
    Ok(state.sites.lock().await.sites.clone())
}

#[tauri::command]
async fn sites_add(state: State<'_, AppState>, url: String) -> Result<bool, String> {
    let added = state.sites.lock().await.add_site(&url);
    if let Some(url) = &added {
        let mut cfg = state.config.lock().unwrap();
        cfg.sites.push(core::config::SiteEntry { url: url.clone() });
        let _ = core::config::save_config(&cfg);
    }
    Ok(added.is_some())
}

#[tauri::command]
async fn sites_remove(state: State<'_, AppState>, url: String) -> Result<(), String> {
    state.sites.lock().await.remove_site(&url);
    let mut cfg = state.config.lock().unwrap();
    cfg.sites.retain(|s| s.url != url);
    let _ = core::config::save_config(&cfg);
    Ok(())
}

#[tauri::command]
fn set_autostart(state: State<AppState>, enable: bool) -> Result<(), String> {
    core::autostart::set_run_on_startup(enable, None).map_err(|e| e.to_string())?;
    let mut cfg = state.config.lock().unwrap();
    cfg.run_on_startup = enable;
    let _ = core::config::save_config(&cfg);
    Ok(())
}

#[tauri::command]
fn is_autostart_set() -> bool {
    core::autostart::is_run_on_startup_set()
}

// ---------- Блокировка приложений ----------

#[tauri::command]
fn blocked_apps_list(state: State<AppState>) -> Vec<core::config::BlockedApp> {
    state.config.lock().unwrap().blocked_apps.clone()
}

#[tauri::command]
fn blocked_apps_add(
    state: State<AppState>,
    exe_name: String,
    display_name: String,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if cfg.blocked_apps.iter().any(|a| a.exe_name.eq_ignore_ascii_case(&exe_name)) {
        return Err("Это приложение уже в списке".into());
    }
    cfg.blocked_apps.push(core::config::BlockedApp {
        exe_name,
        display_name,
        enabled: true,
        ifeo_enabled: false,
    });
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn blocked_apps_remove(state: State<AppState>, exe_name: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    // На всякий случай снимаем IFEO-перехват, если он был включён для этого приложения.
    if core::ifeo::is_guard_enabled(&exe_name) {
        let _ = core::ifeo::disable_guard(&exe_name);
    }
    cfg.blocked_apps.retain(|a| a.exe_name != exe_name);
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn blocked_apps_set_enabled(
    state: State<AppState>,
    exe_name: String,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if let Some(a) = cfg.blocked_apps.iter_mut().find(|a| a.exe_name == exe_name) {
        a.enabled = enabled;
    }
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn is_admin() -> bool {
    core::elevation::is_admin()
}

/// Перезапускает приложение с правами администратора через UAC. После
/// подтверждения UAC старый процесс нужно закрыть — проще всего попросить
/// пользователя закрыть текущее окно вручную (как в старой версии).
#[tauri::command]
fn relaunch_as_admin() -> Result<(), String> {
    core::elevation::relaunch_as_admin(&[])
}

/// Включает надёжный режим блокировки (IFEO) для приложения. Требует
/// запуска NetGuardian с правами администратора — иначе запись в HKLM
/// не пройдёт и Windows вернёт "Отказано в доступе".
#[tauri::command]
fn blocked_apps_enable_ifeo(state: State<AppState>, exe_name: String) -> Result<(), String> {
    if !core::elevation::is_admin() {
        return Err(
            "Нужны права администратора. Перезапустите NetGuardian через «Запуск от имени администратора» или кнопку ниже.".into(),
        );
    }
    core::ifeo::enable_guard(&exe_name).map_err(|e| e.to_string())?;
    let mut cfg = state.config.lock().unwrap();
    if let Some(a) = cfg.blocked_apps.iter_mut().find(|a| a.exe_name == exe_name) {
        a.ifeo_enabled = true;
    }
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn blocked_apps_disable_ifeo(state: State<AppState>, exe_name: String) -> Result<(), String> {
    if !core::elevation::is_admin() {
        return Err(
            "Нужны права администратора, чтобы снять надёжную блокировку. Перезапустите NetGuardian от имени администратора.".into(),
        );
    }
    core::ifeo::disable_guard(&exe_name).map_err(|e| e.to_string())?;
    let mut cfg = state.config.lock().unwrap();
    if let Some(a) = cfg.blocked_apps.iter_mut().find(|a| a.exe_name == exe_name) {
        a.ifeo_enabled = false;
    }
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

// ---------- Блокировка ссылок в браузере (список доменов для расширения) ----------

#[tauri::command]
fn blocked_domains_list(state: State<AppState>) -> Vec<core::config::BlockedDomain> {
    state.config.lock().unwrap().blocked_domains.clone()
}

#[tauri::command]
fn blocked_domains_add(state: State<AppState>, domain: String) -> Result<(), String> {
    let domain = domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_end_matches('/')
        .to_lowercase();
    if domain.is_empty() {
        return Err("Пустой домен".into());
    }
    let mut cfg = state.config.lock().unwrap();
    if cfg.blocked_domains.iter().any(|d| d.domain == domain) {
        return Err("Этот домен уже в списке".into());
    }
    cfg.blocked_domains.push(core::config::BlockedDomain {
        domain,
        enabled: true,
    });
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn blocked_domains_remove(state: State<AppState>, domain: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.blocked_domains.retain(|d| d.domain != domain);
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn blocked_domains_set_enabled(
    state: State<AppState>,
    domain: String,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if let Some(d) = cfg.blocked_domains.iter_mut().find(|d| d.domain == domain) {
        d.enabled = enabled;
    }
    core::config::save_config(&cfg).map_err(|e| e.to_string())
}

/// Запускает исходное приложение так, чтобы IFEO не перехватил его снова
/// (иначе получится бесконечная цепочка: стаб → приложение → стаб → ...).
/// Windows не применяет Debugger из IFEO к процессу, создаваемому с флагом
/// DEBUG_PROCESS; сразу после старта отцепляемся от процесса как отладчик.
fn launch_original_bypassing_ifeo(exe: &str, args: &[String]) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Diagnostics::Debug::{
            DebugActiveProcessStop, DebugSetProcessKillOnExit,
        };
        const DEBUG_PROCESS: u32 = 0x0000_0001;

        let spawned = std::process::Command::new(exe)
            .args(args)
            .creation_flags(DEBUG_PROCESS)
            .spawn();
        match spawned {
            Ok(child) => unsafe {
                // Не убивать приложение при нашем выходе и отцепить отладку.
                let _ = DebugSetProcessKillOnExit(false);
                let _ = DebugActiveProcessStop(child.id());
            },
            Err(e) => eprintln!("не удалось запустить {exe}: {e}"),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new(exe).args(args).spawn();
    }
}

/// Если NetGuardian запущен как IFEO-подмена (Windows перехватила запуск
/// заблокированного приложения и вызвала вместо него нас), обрабатывает
/// это здесь и завершает процесс, не поднимая обычное окно программы.
/// Возвращает true, если это был именно такой запуск.
fn maybe_run_as_ifeo_stub() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] != "--ifeo-stub" {
        return false;
    }
    // Windows подставляет исходный путь и аргументы после нашего "--ifeo-stub".
    if args.len() < 3 {
        std::process::exit(1);
    }
    let original_exe = args[2].clone();
    let original_args: Vec<String> = args[3..].to_vec();
    let display_name = std::path::Path::new(&original_exe)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| original_exe.clone());

    let rt = tokio::runtime::Runtime::new().expect("не удалось создать tokio runtime");
    let connected = rt.block_on(async {
        let cfg = core::config::load_config();
        let status = core::vpn::detect_vpn_status(
            &cfg.vpn_detection_order,
            &cfg.vpn_adapter_hints,
            &cfg.vpn_adapter_exclude_hints,
        )
        .await;
        status.state == core::vpn::VpnState::Connected
    });

    if connected {
        // VPN включён — прозрачно запускаем исходное приложение и выходим.
        launch_original_bypassing_ifeo(&original_exe, &original_args);
        std::process::exit(0);
    }

    // VPN выключен — показываем окно-уведомление и не запускаем приложение.
    // Нужен отдельный минимальный экземпляр Tauri-приложения. ВАЖНО: окна,
    // объявленные в tauri.conf.json (главное окно программы), здесь убираем —
    // иначе при каждой попытке запуска заблокированного приложения поверх
    // всего открывалось бы второе, "пустое" окно NetGuardian.
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();

    tauri::Builder::default()
        .setup(move |app| {
            crate::app_blocking::show_block_notice(&app.handle().clone(), &display_name);
            Ok(())
        })
        .run(context)
        .expect("ошибка показа окна-уведомления");

    std::process::exit(0);
}

fn main() {
    if maybe_run_as_ifeo_stub() {
        return; // недостижимо (стаб-режим сам вызывает process::exit), но для ясности
    }

    let cfg = core::config::load_config();
    let site_urls: Vec<String> = cfg.sites.iter().map(|s| s.url.clone()).collect();

    let state = AppState {
        config: Mutex::new(cfg),
        traffic: Mutex::new(TrafficMonitor::new()),
        sites: AsyncMutex::new(SiteMonitor::new(site_urls)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Если пользователь второй раз запускает .exe — поднимаем уже открытое окно
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(state)
        .setup(|app| {
            // Стеклянный эффект Windows 10/11 (Acrylic на 10, Mica/Acrylic на 11).
            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::{apply_acrylic, apply_mica};
                if let Some(window) = app.get_webview_window("main") {
                    if apply_mica(&window, None).is_err() {
                        let _ = apply_acrylic(&window, Some((18, 18, 24, 125)));
                    }
                }
            }
            // Фоновый вотчер процессов для блокировки приложений без VPN
            // (простой режим; не требует прав администратора).
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(crate::app_blocking::run_watcher(app_handle));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_vpn_status,
            list_adapters,
            traffic_sample,
            traffic_interfaces,
            traffic_snapshot,
            get_public_ip,
            sites_tick,
            sites_list,
            sites_add,
            sites_remove,
            set_autostart,
            is_autostart_set,
            blocked_apps_list,
            blocked_apps_add,
            blocked_apps_remove,
            blocked_apps_set_enabled,
            blocked_apps_enable_ifeo,
            blocked_apps_disable_ifeo,
            is_admin,
            relaunch_as_admin,
            blocked_domains_list,
            blocked_domains_add,
            blocked_domains_remove,
            blocked_domains_set_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("ошибка запуска NetGuardian");
}
