//! Определение состояния VPN.
//!
//! Используется три независимых способа, в порядке из `AppConfig::vpn_detection_order`
//! (по умолчанию: адаптер → публичный IP). Для Hiddify (режим TUN) и
//! AmneziaWG (AmneziaWG-адаптер) основной и самый надёжный способ — "adapter":
//! оба создают отдельный виртуальный сетевой адаптер с состоянием "up",
//! который мы ищем по подстрокам в описании/имени (wintun, wireguard,
//! amneziawg, hiddify, sing-box, tun, tap — см. config.rs).
//!
//! "public_ip" — резервный способ: сверяем текущий внешний IP с тем,
//! что был до включения VPN (или просто проверяем, что IP не совпадает
//! с адресом физического провайтера, определённым при первом запуске).
//! Он медленнее (сетевой запрос) и используется, если способ "adapter"
//! не дал однозначного ответа — например, адаптер поднят, но реального
//! туннеля ещё нет (авторизация ключа ещё идёт).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VpnState {
    Connected,
    Disconnected,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnAdapterInfo {
    pub name: String,
    pub description: String,
    pub is_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnStatus {
    pub state: VpnState,
    pub method: String, // "adapter" | "public_ip" | "none"
    pub matched_adapter: Option<VpnAdapterInfo>,
}

/// Перечисляет все сетевые адаптеры системы вместе с их рабочим статусом.
#[cfg(windows)]
pub fn list_adapters() -> anyhow::Result<Vec<VpnAdapterInfo>> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_PREFIX, IP_ADAPTER_ADDRESSES_LH,
    };
    // IF_OPER_STATUS формально объявлен в ifdef.h вместе с остальными API
    // IpHelper, но в крейте windows лежит в соседнем модуле Ndis.
    use windows::Win32::NetworkManagement::Ndis::IF_OPER_STATUS;
    use windows::Win32::Networking::WinSock::AF_UNSPEC;

    let mut size: u32 = 15_000;
    let mut buf: Vec<u8>;
    let mut result;

    loop {
        buf = vec![0u8; size as usize];
        let ptr = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_PREFIX,
                None,
                Some(ptr),
                &mut size,
            )
        };
        // ERROR_BUFFER_OVERFLOW = 111
        if result != 111 {
            break;
        }
    }

    if result != 0 {
        anyhow::bail!("GetAdaptersAddresses вернул код ошибки {result}");
    }

    let mut adapters = Vec::new();
    let mut cursor = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;

    unsafe {
        while !cursor.is_null() {
            let entry = &*cursor;

            let friendly_name = entry
                .FriendlyName
                .to_string()
                .unwrap_or_default();
            let description = entry.Description.to_string().unwrap_or_default();
            let oper_status: IF_OPER_STATUS = entry.OperStatus;
            let is_up = oper_status.0 == 1; // IfOperStatusUp = 1

            adapters.push(VpnAdapterInfo {
                name: friendly_name,
                description,
                is_up,
            });

            cursor = entry.Next;
        }
    }

    Ok(adapters)
}

#[cfg(not(windows))]
pub fn list_adapters() -> anyhow::Result<Vec<VpnAdapterInfo>> {
    // На платформах, отличных от Windows, этот путь не используется в продакшене
    // (программа предназначена для Windows 10/11), но оставлен, чтобы `cargo check`
    // проходил при разработке/CI на других ОС.
    Ok(Vec::new())
}

/// Ищет среди адаптеров VPN-подобный по подстрокам из `hints` в имени/описании,
/// предпочитая адаптеры в состоянии "up". Адаптеры, совпавшие с `exclude_hints`
/// (например, встроенный в Windows Teredo, чьё имя содержит "tun" от
/// "Tunneling"), отбрасываются ещё до проверки `hints`.
pub fn find_vpn_adapter(
    hints: &[String],
    exclude_hints: &[String],
) -> anyhow::Result<Option<VpnAdapterInfo>> {
    let adapters = list_adapters()?;
    let lower_hints: Vec<String> = hints.iter().map(|h| h.to_lowercase()).collect();
    let lower_excludes: Vec<String> = exclude_hints.iter().map(|h| h.to_lowercase()).collect();

    let matches: Vec<&VpnAdapterInfo> = adapters
        .iter()
        .filter(|a| {
            let n = a.name.to_lowercase();
            let d = a.description.to_lowercase();
            if lower_excludes.iter().any(|h| n.contains(h) || d.contains(h)) {
                return false;
            }
            lower_hints.iter().any(|h| n.contains(h) || d.contains(h))
        })
        .collect();

    if matches.is_empty() {
        return Ok(None);
    }

    // Предпочитаем адаптер, который реально "up"; если таких нет — берём первый найденный.
    let chosen = matches
        .iter()
        .copied()
        .find(|a| a.is_up)
        .unwrap_or(matches[0]);
    Ok(Some(chosen.clone()))
}

/// Определение по публичному IP: просто факт успешного ответа интерпретируется
/// как "сеть доступна"; сравнение с "адресом без VPN" делает вызывающий код
/// (в main.rs/команде Tauri), так как для этого нужно хранить baseline IP.
pub async fn check_public_ip_reachable() -> bool {
    super::ip_info::get_public_ip_info().await.is_ok()
}

/// Комбинированная проверка статуса VPN согласно конфигурации.
pub async fn detect_vpn_status(
    order: &[String],
    hints: &[String],
    exclude_hints: &[String],
) -> VpnStatus {
    for method in order {
        match method.as_str() {
            "adapter" => {
                if let Ok(Some(adapter)) = find_vpn_adapter(hints, exclude_hints) {
                    let state = if adapter.is_up {
                        VpnState::Connected
                    } else {
                        VpnState::Disconnected
                    };
                    return VpnStatus {
                        state,
                        method: "adapter".into(),
                        matched_adapter: Some(adapter),
                    };
                }
                // адаптер не найден вообще — пробуем следующий способ
            }
            "public_ip" => {
                let reachable = check_public_ip_reachable().await;
                return VpnStatus {
                    state: if reachable {
                        VpnState::Unknown // наличие сети не равно наличию VPN; см. README
                    } else {
                        VpnState::Disconnected
                    },
                    method: "public_ip".into(),
                    matched_adapter: None,
                };
            }
            _ => {}
        }
    }

    VpnStatus {
        state: VpnState::Unknown,
        method: "none".into(),
        matched_adapter: None,
    }
}
