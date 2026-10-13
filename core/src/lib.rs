//! netguardian-core — общая логика, используемая и десктоп-приложением
//! (src-tauri), и нативным хостом для браузерного расширения (native-host).
//! Вынесено в отдельный крейт, чтобы оба места видели один и тот же
//! статус VPN и одни и те же настройки, а не две независимые
//! (и потенциально расходящиеся) реализации.

pub mod autostart;
pub mod config;
pub mod elevation;
pub mod ifeo;
pub mod ip_info;
pub mod sites;
pub mod traffic;
pub mod vpn;
