//! Сбор статистики сетевого трафика — Rust-порт traffic_monitor.py,
//! но ПО ИНТЕРФЕЙСАМ отдельно, а не суммарно.
//!
//! В старой версии psutil.net_io_counters() без аргументов суммировал
//! все адаптеры разом. Когда активен VPN с TUN-адаптером (Hiddify TUN,
//! AmneziaWG), один и тот же пользовательский трафик считается дважды:
//! один раз как исходный пакет на виртуальном адаптере, второй раз как
//! уже зашифрованный пакет на физическом адаптере (Wi-Fi/Ethernet).
//! Поэтому здесь трафик VPN-адаптера и физического канала считаются
//! и показываются раздельно.

use std::collections::VecDeque;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sysinfo::Networks;

const REALTIME_HISTORY_SECONDS: usize = 300;
const HOURLY_LOG_CAP: usize = 60 * 60 * 24; // запас на сутки посекундных точек

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp: u64,
    pub down_bps: f64,
    pub up_bps: f64,
}

#[derive(Default, Clone)]
struct Counters {
    recv: u64,
    sent: u64,
}

struct PerInterfaceState {
    last: Counters,
    boot: Counters,
    samples: VecDeque<Sample>,
    hourly_log: VecDeque<(Instant, u64, u64)>, // (instant, recv_delta, sent_delta)
}

impl PerInterfaceState {
    fn new(initial: Counters) -> Self {
        Self {
            boot: initial.clone(),
            last: initial,
            samples: VecDeque::with_capacity(REALTIME_HISTORY_SECONDS),
            hourly_log: VecDeque::with_capacity(1024),
        }
    }
}

pub struct TrafficMonitor {
    networks: Networks,
    last_sample_at: Instant,
    state: std::collections::HashMap<String, PerInterfaceState>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl TrafficMonitor {
    pub fn new() -> Self {
        let networks = Networks::new_with_refreshed_list();
        let mut state = std::collections::HashMap::new();
        for (name, data) in networks.iter() {
            state.insert(
                name.clone(),
                PerInterfaceState::new(Counters {
                    recv: data.total_received(),
                    sent: data.total_transmitted(),
                }),
            );
        }
        Self {
            networks,
            last_sample_at: Instant::now(),
            state,
        }
    }

    /// Вызывать раз в секунду (из таймера GUI / фонового тика).
    pub fn sample(&mut self) {
        // refresh_list(), а не просто refresh(): нужно подхватывать и
        // новые интерфейсы, которые появились после старта программы —
        // например, VPN-адаптер Hiddify/AmneziaWG, поднятый уже после
        // запуска NetGuardian, а не только обновлять данные уже известных.
        self.networks.refresh_list();
        let now_instant = Instant::now();
        let dt = now_instant
            .duration_since(self.last_sample_at)
            .as_secs_f64()
            .max(0.001);
        let ts = now_unix();

        for (name, data) in self.networks.iter() {
            let recv = data.total_received();
            let sent = data.total_transmitted();
            let entry = self
                .state
                .entry(name.clone())
                .or_insert_with(|| PerInterfaceState::new(Counters { recv, sent }));

            let recv_delta = recv.saturating_sub(entry.last.recv);
            let sent_delta = sent.saturating_sub(entry.last.sent);

            let down_bps = recv_delta as f64 / dt;
            let up_bps = sent_delta as f64 / dt;

            if entry.samples.len() >= REALTIME_HISTORY_SECONDS {
                entry.samples.pop_front();
            }
            entry.samples.push_back(Sample {
                timestamp: ts,
                down_bps,
                up_bps,
            });

            if entry.hourly_log.len() >= HOURLY_LOG_CAP {
                entry.hourly_log.pop_front();
            }
            entry
                .hourly_log
                .push_back((now_instant, recv_delta, sent_delta));

            entry.last = Counters { recv, sent };
        }

        self.last_sample_at = now_instant;
    }

    pub fn interface_names(&self) -> Vec<String> {
        self.state.keys().cloned().collect()
    }

    pub fn get_realtime_series(&self, interface: &str) -> Vec<Sample> {
        self.state
            .get(interface)
            .map(|s| s.samples.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn get_last_hour_bytes(&self, interface: &str) -> (u64, u64) {
        let Some(entry) = self.state.get(interface) else {
            return (0, 0);
        };
        let cutoff = Instant::now() - std::time::Duration::from_secs(3600);
        let mut recv_total = 0u64;
        let mut sent_total = 0u64;
        for (t, r, s) in entry.hourly_log.iter() {
            if *t >= cutoff {
                recv_total += r;
                sent_total += s;
            }
        }
        (recv_total, sent_total)
    }

    pub fn get_since_boot_bytes(&self, interface: &str) -> (u64, u64) {
        self.state
            .get(interface)
            .map(|s| (s.last.recv.saturating_sub(s.boot.recv), s.last.sent.saturating_sub(s.boot.sent)))
            .unwrap_or((0, 0))
    }

    /// Суммарный трафик по физическим (не виртуальным) адаптерам —
    /// примерный эквивалент того, что раньше показывала старая версия,
    /// но без двойного счёта VPN-адаптера.
    pub fn total_physical_bytes_since_boot(&self, vpn_hints: &[String]) -> (u64, u64) {
        let mut recv = 0u64;
        let mut sent = 0u64;
        for (name, s) in self.state.iter() {
            if is_virtual_hint(name, vpn_hints) {
                continue;
            }
            recv += s.last.recv.saturating_sub(s.boot.recv);
            sent += s.last.sent.saturating_sub(s.boot.sent);
        }
        (recv, sent)
    }

    pub fn format_bytes(mut n: f64) -> String {
        for unit in ["Б", "КБ", "МБ", "ГБ", "ТБ"] {
            if n.abs() < 1024.0 {
                return format!("{n:.1} {unit}");
            }
            n /= 1024.0;
        }
        format!("{n:.1} ПБ")
    }

    pub fn format_bps(n: f64) -> String {
        let mut bits = n * 8.0;
        for unit in ["бит/с", "Кбит/с", "Мбит/с", "Гбит/с"] {
            if bits.abs() < 1000.0 {
                return format!("{bits:.1} {unit}");
            }
            bits /= 1000.0;
        }
        format!("{bits:.1} Тбит/с")
    }
}

fn is_virtual_hint(name: &str, hints: &[String]) -> bool {
    let lname = name.to_lowercase();
    hints.iter().any(|h| lname.contains(&h.to_lowercase()))
}
