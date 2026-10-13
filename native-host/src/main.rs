//! Нативный хост (Native Messaging) для браузерного расширения NetGuardian.
//!
//! Протокол Chrome/Edge/Firefox Native Messaging: каждое сообщение —
//! 4 байта длины (little-endian u32) + столько же байт UTF-8 JSON, в обе
//! стороны через stdin/stdout. Браузер сам запускает этот процесс (через
//! манифест хоста, см. install-native-host.ps1) и держит его живым, пока
//! открыт `chrome.runtime.connectNative(...)` на стороне расширения —
//! поэтому здесь цикл, а не одно сообщение и выход.
//!
//! Использует ту же логику определения VPN и то же config.json, что и
//! основное приложение NetGuardian (через общий крейт netguardian-core),
//! так что статус VPN в расширении и в десктоп-приложении всегда совпадает.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Status,
    GetBlockedDomains,
    Ping,
}

#[derive(Serialize)]
struct StatusResponse {
    connected: bool,
    method: String,
    adapter: Option<String>,
}

#[derive(Serialize)]
struct DomainsResponse {
    domains: Vec<String>,
}

fn read_message() -> io::Result<Option<Value>> {
    let stdin = io::stdin();
    let mut len_buf = [0u8; 4];
    {
        let mut lock = stdin.lock();
        if let Err(e) = lock.read_exact(&mut len_buf) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(None); // браузер закрыл канал — нормальное завершение
            }
            return Err(e);
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        // Защита от аномально большого объявленного размера (повреждённый
        // или злонамеренный ввод) — настоящие сообщения этого протокола крошечные.
        if len > 10 * 1024 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "сообщение слишком большое"));
        }
        let mut buf = vec![0u8; len];
        lock.read_exact(&mut buf)?;
        let value: Value = serde_json::from_slice(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Some(value))
    }
}

fn write_message(value: &impl Serialize) -> io::Result<()> {
    let data = serde_json::to_vec(value)?;
    let len = (data.len() as u32).to_le_bytes();
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(&len)?;
    lock.write_all(&data)?;
    lock.flush()
}

fn main() {
    let rt = tokio::runtime::Runtime::new().expect("не удалось создать tokio runtime");

    loop {
        let msg = match read_message() {
            Ok(Some(v)) => v,
            Ok(None) => break, // EOF — браузер завершил соединение
            Err(e) => {
                eprintln!("netguardian-native-host: ошибка чтения сообщения: {e}");
                break;
            }
        };

        let request: Request = match serde_json::from_value(msg) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("netguardian-native-host: нераспознанное сообщение: {e}");
                continue;
            }
        };

        match request {
            Request::Ping => {
                let _ = write_message(&serde_json::json!({ "pong": true }));
            }
            Request::Status => {
                let cfg = netguardian_core::config::load_config();
                let status = rt.block_on(netguardian_core::vpn::detect_vpn_status(
                    &cfg.vpn_detection_order,
                    &cfg.vpn_adapter_hints,
                    &cfg.vpn_adapter_exclude_hints,
                ));
                let resp = StatusResponse {
                    connected: status.state == netguardian_core::vpn::VpnState::Connected,
                    method: status.method,
                    adapter: status.matched_adapter.map(|a| a.name),
                };
                let _ = write_message(&resp);
            }
            Request::GetBlockedDomains => {
                let cfg = netguardian_core::config::load_config();
                let domains = cfg
                    .blocked_domains
                    .into_iter()
                    .filter(|d| d.enabled)
                    .map(|d| d.domain)
                    .collect();
                let _ = write_message(&DomainsResponse { domains });
            }
        }
    }
}
