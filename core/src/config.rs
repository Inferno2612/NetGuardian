//! Хранение настроек приложения в %APPDATA%\NetGuardian\config.json
//! Rust-порт старого config.py. Формат файла умышленно почти совместим,
//! чтобы при желании можно было вручную перенести список сайтов.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const APP_NAME: &str = "NetGuardian";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedApp {
    /// Имя исполняемого файла для простого режима (process-watcher), напр. "telegram.exe"
    pub exe_name: String,
    /// Отображаемое имя в интерфейсе
    pub display_name: String,
    /// Включена ли блокировка этого приложения
    pub enabled: bool,
    /// Установлен ли надёжный режим через IFEO (требует прав администратора)
    pub ifeo_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedDomain {
    pub domain: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteEntry {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub run_on_startup: bool,

    #[serde(default)]
    pub sites: Vec<SiteEntry>,

    #[serde(default)]
    pub blocked_apps: Vec<BlockedApp>,

    #[serde(default)]
    pub blocked_domains: Vec<BlockedDomain>,

    #[serde(default)]
    pub window_geometry: Option<serde_json::Value>,

    /// Какой способ определения VPN использовать первым.
    /// "adapter" | "proxy" | "public_ip" — пробуются по очереди как фолбэк,
    /// это поле задаёт только порядок, а не единственный источник истины.
    #[serde(default = "default_vpn_detection_order")]
    pub vpn_detection_order: Vec<String>,

    /// Имена подстрок виртуальных адаптеров, которые считаются VPN
    /// (Hiddify/sing-box TUN, AmneziaWG, WireGuard и т.д.)
    #[serde(default = "default_vpn_adapter_hints")]
    pub vpn_adapter_hints: Vec<String>,

    /// Подстроки адаптеров, которые НИКОГДА не считаются VPN, даже если
    /// совпали с vpn_adapter_hints. Проверяются раньше hints — нужно,
    /// например, потому что встроенный в Windows псевдоадаптер
    /// "Teredo Tunneling Pseudo-Interface" содержит подстроку "tun" (от
    /// "Tunneling") и без этого списка ошибочно распознавался бы как VPN.
    #[serde(default = "default_vpn_adapter_exclude_hints")]
    pub vpn_adapter_exclude_hints: Vec<String>,
}

fn default_vpn_detection_order() -> Vec<String> {
    vec!["adapter".into(), "public_ip".into()]
}

fn default_vpn_adapter_hints() -> Vec<String> {
    vec![
        "wintun".into(),
        "wireguard".into(),
        "amneziawg".into(),
        "hiddify".into(),
        "sing-box".into(),
        "tun".into(),
        "tap".into(),
    ]
}

fn default_vpn_adapter_exclude_hints() -> Vec<String> {
    vec![
        // Встроенные в Windows псевдоадаптеры IPv6-перехода — всегда
        // присутствуют в системе и часто в статусе "up", но не VPN.
        "teredo".into(),
        "isatap".into(),
        "6to4".into(),
        // На всякий случай — не-VPN виртуальные интерфейсы, которые
        // теоретически могли бы зацепиться за широкие подстроки вроде "tap".
        "loopback".into(),
        "bluetooth".into(),
    ]
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            run_on_startup: false,
            sites: Vec::new(),
            blocked_apps: Vec::new(),
            blocked_domains: Vec::new(),
            window_geometry: None,
            vpn_detection_order: default_vpn_detection_order(),
            vpn_adapter_hints: default_vpn_adapter_hints(),
            vpn_adapter_exclude_hints: default_vpn_adapter_exclude_hints(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = directories::BaseDirs::new()
        .map(|b| b.data_dir().to_path_buf()) // %APPDATA% on Windows
        .unwrap_or_else(|| PathBuf::from("."));
    let path = base.join(APP_NAME);
    let _ = fs::create_dir_all(&path);
    path
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load_config() -> AppConfig {
    let path = config_path();
    if !path.exists() {
        let cfg = AppConfig::default();
        let _ = save_config(&cfg);
        return cfg;
    }
    match fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str::<AppConfig>(&data).unwrap_or_else(|_| {
            let cfg = AppConfig::default();
            let _ = save_config(&cfg);
            cfg
        }),
        Err(_) => AppConfig::default(),
    }
}

pub fn save_config(cfg: &AppConfig) -> anyhow::Result<()> {
    let path = config_path();
    let tmp_path = path.with_extension("tmp");
    let data = serde_json::to_string_pretty(cfg)?;
    fs::write(&tmp_path, data)?;
    fs::rename(&tmp_path, &path)?; // атомарная замена, как в старой версии
    Ok(())
}
