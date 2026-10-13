//! Блокировка запуска приложений, пока VPN выключен.
//!
//! Два режима (см. README):
//! - Простой («process-watcher», по умолчанию): фоновый цикл в этом файле
//!   каждую секунду сверяет список запущенных процессов со списком
//!   заблокированных в config.rs и завершает совпавшие вместе со всем
//!   деревом их дочерних процессов, если VPN не подключён. Не требует прав
//!   администратора (кроме случая, когда сам блокируемый процесс запущен
//!   от администратора — тогда Windows не даст его закрыть). Приложение
//!   может успеть мигнуть на экране на долю секунды.
//! - Надёжный (IFEO, core/ifeo.rs): приложение в принципе не запускается,
//!   но требует прав администратора на включение.
//!
//! Окно-уведомление (`show_block_notice`) общее для обоих режимов.
//!
//! Каждое срабатывание вотчера пишется в %APPDATA%\NetGuardian\blocker.log
//! (PID, родитель, результат завершения) — для диагностики.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use netguardian_core::config::BlockedApp;
use netguardian_core::vpn::VpnState;

const SCAN_INTERVAL: Duration = Duration::from_millis(1000);
const VPN_RECHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Окно-уведомление для одного и того же приложения показываем не чаще,
/// чем раз в это время. Это единственный механизм против спама — сама
/// блокировка (завершение процессов) при этом НИКОГДА не приостанавливается.
const NOTICE_COOLDOWN: Duration = Duration::from_secs(15);

/// Системные процессы, которые вотчер не трогает ни при каких настройках
/// (включая случай, когда пользователь по ошибке добавил их в список) —
/// завершение любого из них ломает Windows или убивает все приложения
/// рабочего стола разом, так как explorer.exe — родитель почти всех из них.
const PROTECTED_NAMES: &[&str] = &[
    "system.exe",
    "registry.exe",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "svchost.exe",
    "explorer.exe",
    "dwm.exe",
    "fontdrvhost.exe",
];

#[derive(Clone, Serialize)]
struct NoticePayload {
    app_name: String,
}

fn normalize(name: &str) -> String {
    let n = name.trim().to_lowercase();
    if n.ends_with(".exe") {
        n
    } else {
        format!("{n}.exe")
    }
}

fn normalized_name(process: &Process) -> Option<String> {
    process.name().to_str().map(normalize)
}

/// Дописывает строку в %APPDATA%\NetGuardian\blocker.log (с простой
/// ротацией по размеру, чтобы файл не рос бесконечно при цикле перезапусков).
fn log_line(msg: &str) {
    let path = netguardian_core::config::config_dir().join("blocker.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 512 * 1024 {
            let _ = std::fs::rename(&path, path.with_extension("log.old"));
        }
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

/// Показывает (или обновляет, если уже открыто) окно-уведомление
/// «включите VPN» для указанного приложения.
pub fn show_block_notice(app: &AppHandle, display_name: &str) {
    let payload = NoticePayload {
        app_name: display_name.to_string(),
    };

    if let Some(win) = app.get_webview_window("block-notice") {
        // Окно уже открыто — только обновляем текст, фокус не перехватываем
        // (иначе при повторных срабатываниях оно будет постоянно "прыгать"
        // поверх всего, что пользователь делает).
        let _ = win.emit("notice-update", payload);
        let _ = win.show();
        return;
    }

    let encoded = urlencoding::encode(display_name).into_owned();
    let url = WebviewUrl::App(format!("notice.html?app={encoded}").into());

    let builder = WebviewWindowBuilder::new(app, "block-notice", url)
        .title("NetGuardian")
        .inner_size(420.0, 270.0)
        .resizable(false)
        .decorations(true)
        .center()
        .always_on_top(true)
        .transparent(true);

    match builder.build() {
        Ok(win) => {
            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::{apply_acrylic, apply_mica};
                if apply_mica(&win, None).is_err() {
                    let _ = apply_acrylic(&win, Some((18, 18, 24, 160)));
                }
            }
            let _ = win.emit("notice-update", payload);
        }
        Err(e) => {
            eprintln!("Не удалось создать окно-уведомление: {e}");
        }
    }
}

/// Собирает множество PID, которые нельзя трогать: сам NetGuardian и вся
/// цепочка его родителей (например, PowerShell/cargo при запуске через
/// `cargo tauri dev`) — иначе блокировка такого процесса убила бы нас самих.
fn protected_pids(sys: &System) -> HashSet<Pid> {
    let mut protected = HashSet::new();
    if let Ok(own) = sysinfo::get_current_pid() {
        protected.insert(own);
        let mut cur = sys.process(own).and_then(|p| p.parent());
        while let Some(pid) = cur {
            if !protected.insert(pid) {
                break; // защита от циклов из-за повторного использования PID
            }
            cur = sys.process(pid).and_then(|p| p.parent());
        }
    }
    for (pid, p) in sys.processes() {
        if let Some(n) = normalized_name(p) {
            if PROTECTED_NAMES.contains(&n.as_str()) {
                protected.insert(*pid);
            }
        }
    }
    // Все потомки NetGuardian (в т.ч. msedgewebview2.exe, на которых
    // держится наш интерфейс) — тоже не трогаем.
    if let Ok(own) = sysinfo::get_current_pid() {
        for (pid, _) in sys.processes() {
            let mut cur = Some(*pid);
            let mut hops = 0;
            while let Some(c) = cur {
                if c == own {
                    protected.insert(*pid);
                    break;
                }
                hops += 1;
                if hops > 64 {
                    break;
                }
                cur = sys.process(c).and_then(|p| p.parent());
            }
        }
    }
    protected
}

/// Все PID-ы из `seeds` плюс все их потомки (рекурсивно), кроме защищённых.
/// Это и есть "дерево процессов приложения": Electron и подобные программы
/// запускают рендереры, GPU-процессы и т.д., часто с другими именами .exe,
/// и kill() по одному только имени оставляет такие "хвосты" работать.
fn collect_tree(
    seeds: &[Pid],
    children: &HashMap<Pid, Vec<Pid>>,
    protected: &HashSet<Pid>,
) -> Vec<Pid> {
    let mut visited: HashSet<Pid> = HashSet::new();
    let mut order = Vec::new();
    let mut stack: Vec<Pid> = seeds.to_vec();

    while let Some(pid) = stack.pop() {
        if protected.contains(&pid) || !visited.insert(pid) {
            continue;
        }
        order.push(pid);
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids.iter().copied());
        }
    }
    order
}

/// Фоновый цикл процесс-вотчера. Запускается один раз в setup() через
/// tauri::async_runtime::spawn и работает всё время жизни приложения.
pub async fn run_watcher(app: AppHandle) {
    let mut sys = System::new();
    let mut last_vpn_check = Instant::now() - VPN_RECHECK_INTERVAL - Duration::from_secs(1);
    let mut vpn_connected = false;
    let mut notice_cooldown: HashMap<String, Instant> = HashMap::new();
    // PID-ы совпавших процессов на прошлом скане — чтобы отличать «приложение
    // снова запустили» от «старый процесс, который не удаётся завершить».
    let mut prev_seeds: HashSet<Pid> = HashSet::new();
    // PID-ы, которые не удалось завершить и о которых уже написано в лог.
    let mut stuck_logged: HashSet<Pid> = HashSet::new();
    let mut ifeo_admin_warned = false;

    log_line("вотчер запущен");

    loop {
        tokio::time::sleep(SCAN_INTERVAL).await;

        let state = app.state::<crate::AppState>();
        let (blocked_apps, order, hints, exclude_hints): (
            Vec<BlockedApp>,
            Vec<String>,
            Vec<String>,
            Vec<String>,
        ) = {
            let cfg = state.config.lock().unwrap();
            (
                cfg.blocked_apps.clone(),
                cfg.vpn_detection_order.clone(),
                cfg.vpn_adapter_hints.clone(),
                cfg.vpn_adapter_exclude_hints.clone(),
            )
        };

        let enabled: Vec<&BlockedApp> = blocked_apps.iter().filter(|a| a.enabled).collect();
        let any_ifeo = blocked_apps.iter().any(|a| a.ifeo_enabled);
        if enabled.is_empty() && !any_ifeo {
            continue;
        }

        if last_vpn_check.elapsed() >= VPN_RECHECK_INTERVAL {
            let status =
                netguardian_core::vpn::detect_vpn_status(&order, &hints, &exclude_hints).await;
            let now_connected = status.state == VpnState::Connected;
            if now_connected != vpn_connected {
                log_line(&format!(
                    "статус VPN: {:?} (метод: {}, адаптер: {})",
                    status.state,
                    status.method,
                    status
                        .matched_adapter
                        .as_ref()
                        .map(|a| a.name.as_str())
                        .unwrap_or("—")
                ));
            }
            vpn_connected = now_connected;
            last_vpn_check = Instant::now();

            // IFEO-перехват активен только пока VPN выключен (см. ifeo::suspend_guard).
            if any_ifeo {
                if netguardian_core::elevation::is_admin() {
                    for a in blocked_apps.iter().filter(|a| a.ifeo_enabled) {
                        let want_active = !vpn_connected && a.enabled;
                        let res = if want_active {
                            netguardian_core::ifeo::resume_guard(&a.exe_name)
                        } else {
                            netguardian_core::ifeo::suspend_guard(&a.exe_name)
                        };
                        match res {
                            Ok(true) => log_line(&format!(
                                "IFEO для {}: {}",
                                a.exe_name,
                                if want_active { "включён (VPN выключен)" } else { "приостановлен (VPN включён)" }
                            )),
                            Ok(false) => {}
                            Err(e) => log_line(&format!("IFEO для {}: ошибка {e}", a.exe_name)),
                        }
                    }
                } else if !ifeo_admin_warned {
                    ifeo_admin_warned = true;
                    log_line("IFEO: нет прав администратора — не могу приостанавливать перехват при включённом VPN");
                }
            }
        }

        if vpn_connected || enabled.is_empty() {
            prev_seeds.clear();
            stuck_logged.clear();
            continue; // VPN включён (или нечего блокировать)
        }

        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
        );

        // Карта "родитель → дети" на этот момент времени.
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, p) in sys.processes() {
            if let Some(parent) = p.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }

        let protected = protected_pids(&sys);

        // Группируем процессы-"семена" (совпавшие по имени) по приложению.
        let mut seeds_by_app: HashMap<&str, Vec<Pid>> = HashMap::new();
        for (pid, p) in sys.processes() {
            let Some(pname) = normalized_name(p) else {
                continue;
            };
            if let Some(matched) = enabled.iter().find(|a| normalize(&a.exe_name) == pname) {
                seeds_by_app
                    .entry(matched.exe_name.as_str())
                    .or_default()
                    .push(*pid);
            }
        }

        let current_seeds: HashSet<Pid> = seeds_by_app.values().flatten().copied().collect();

        for (exe_name, seeds) in seeds_by_app {
            let Some(app_cfg) = enabled.iter().find(|a| a.exe_name == exe_name) else {
                continue;
            };

            let targets = collect_tree(&seeds, &children, &protected);
            if targets.is_empty() {
                continue; // все совпавшие процессы оказались защищёнными
            }

            let new_seed = seeds.iter().any(|p| !prev_seeds.contains(p));

            let describe = |pid: &Pid| -> String {
                match sys.process(*pid) {
                    Some(p) => {
                        let path = p
                            .exe()
                            .map(|x| x.display().to_string())
                            .unwrap_or_else(|| "?".into());
                        let parent = p
                            .parent()
                            .map(|pp| {
                                let n = sys
                                    .process(pp)
                                    .map(|x| x.name().to_string_lossy().to_string())
                                    .unwrap_or_else(|| "?".into());
                                format!("{n}({pp})")
                            })
                            .unwrap_or_else(|| "—".into());
                        format!(
                            "{}({}) путь={} родитель={} старт={}",
                            p.name().to_string_lossy(),
                            pid,
                            path,
                            parent,
                            p.start_time()
                        )
                    }
                    None => format!("?({pid})"),
                }
            };

            let mut killed = 0usize;
            let mut failed: Vec<Pid> = Vec::new();
            for pid in &targets {
                if let Some(p) = sys.process(*pid) {
                    if p.kill() {
                        killed += 1;
                    } else {
                        failed.push(*pid);
                    }
                }
            }

            log_line(&format!(
                "приложение «{}»: совпавших {}, в дереве {}, завершено {}, не удалось {}, новых запусков: {}",
                app_cfg.display_name,
                seeds.len(),
                targets.len(),
                killed,
                failed.len(),
                if new_seed { "да" } else { "нет" }
            ));
            if killed > 0 && new_seed {
                for pid in targets.iter().take(12) {
                    log_line(&format!("    цель: {}", describe(pid)));
                }
            }
            for pid in &failed {
                if stuck_logged.insert(*pid) {
                    log_line(&format!(
                        "    НЕ УДАЛОСЬ завершить: {} — вероятно, запущен от администратора/как служба; запустите NetGuardian от администратора",
                        describe(pid)
                    ));
                }
            }

            // Окно показываем только если приложение действительно (пере)запустили
            // или мы что-то завершили; процесс, который просто не удаётся убить,
            // не должен вызывать уведомление каждые 15 секунд.
            let due = notice_cooldown
                .get(&app_cfg.exe_name)
                .map(|t| t.elapsed() >= NOTICE_COOLDOWN)
                .unwrap_or(true);
            if due && (killed > 0 || new_seed) {
                show_block_notice(&app, &app_cfg.display_name);
                notice_cooldown.insert(app_cfg.exe_name.clone(), Instant::now());
            }
        }

        prev_seeds = current_seeds;
        stuck_logged.retain(|p| sys.process(*p).is_some());
    }
}
