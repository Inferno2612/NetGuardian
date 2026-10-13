//! Периодическая проверка доступности сайтов (без ограничения по количеству) — Rust-порт site_monitor.py.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);
const TIMEOUT: Duration = Duration::from_secs(10);
const HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteHistoryPoint {
    pub timestamp: u64,
    pub is_up: Option<bool>,
    pub response_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteStatus {
    pub url: String,
    pub is_up: Option<bool>,
    pub status_code: Option<u16>,
    pub response_ms: Option<f64>,
    pub last_checked: Option<u64>,
    pub last_error: Option<String>,
    pub history: Vec<SiteHistoryPoint>,
}

impl SiteStatus {
    fn new(url: String) -> Self {
        Self {
            url,
            is_up: None,
            status_code: None,
            response_ms: None,
            last_checked: None,
            last_error: None,
            history: Vec::new(),
        }
    }
}

/// Результат одной проверки; применяется к монитору через `SiteMonitor::apply`.
#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub url: String,
    pub status_code: Option<u16>,
    pub response_ms: f64,
    pub error: Option<String>,
}

/// Одна HTTP-проверка. Свободная функция (не метод), чтобы вызывающий код мог
/// запускать много проверок параллельно, не удерживая блокировку на мониторе.
pub async fn check_url(client: reqwest::Client, url: String) -> CheckOutcome {
    let start = Instant::now();
    let result = client.get(&url).send().await;
    let response_ms = start.elapsed().as_secs_f64() * 1000.0;
    match result {
        Ok(resp) => CheckOutcome {
            url,
            status_code: Some(resp.status().as_u16()),
            response_ms,
            error: None,
        },
        Err(e) => CheckOutcome {
            url,
            status_code: None,
            response_ms,
            error: Some(e.to_string()),
        },
    }
}

/// Приводит ввод пользователя к адресу: без схемы добавляет https://.
pub fn normalize_url(input: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    if t.starts_with("http://") || t.starts_with("https://") {
        Some(t.to_string())
    } else {
        Some(format!("https://{t}"))
    }
}

pub struct SiteMonitor {
    /// Порядок добавления сохраняется — строки в интерфейсе не «прыгают».
    pub sites: Vec<SiteStatus>,
    last_check_instant: HashMap<String, Instant>,
    client: reqwest::Client,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl SiteMonitor {
    pub fn new(urls: Vec<String>) -> Self {
        let mut sites: Vec<SiteStatus> = Vec::new();
        for u in urls {
            if !sites.iter().any(|s| s.url == u) {
                sites.push(SiteStatus::new(u));
            }
        }
        Self {
            sites,
            last_check_instant: HashMap::new(),
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .expect("reqwest client"),
        }
    }

    /// Добавляет сайт; возвращает нормализованный адрес, если он добавлен
    /// (None — пустой ввод или такой адрес уже есть).
    pub fn add_site(&mut self, input: &str) -> Option<String> {
        let url = normalize_url(input)?;
        if self.sites.iter().any(|s| s.url == url) {
            return None;
        }
        self.sites.push(SiteStatus::new(url.clone()));
        Some(url)
    }

    pub fn remove_site(&mut self, url: &str) {
        self.sites.retain(|s| s.url != url);
        self.last_check_instant.remove(url);
    }

    pub fn client(&self) -> reqwest::Client {
        self.client.clone()
    }

    /// Адреса, которым пора проверяться. Сразу помечает их как проверяемые,
    /// чтобы два одновременных вызова не запустили одну проверку дважды.
    pub fn take_due(&mut self) -> Vec<String> {
        let mut due = Vec::new();
        for site in &self.sites {
            let is_due = match self.last_check_instant.get(&site.url) {
                Some(last) => last.elapsed() >= CHECK_INTERVAL,
                None => true,
            };
            if is_due {
                due.push(site.url.clone());
            }
        }
        for u in &due {
            self.last_check_instant.insert(u.clone(), Instant::now());
        }
        due
    }

    pub fn apply(&mut self, o: CheckOutcome) {
        let Some(site) = self.sites.iter_mut().find(|s| s.url == o.url) else {
            return; // сайт успели удалить, пока шла проверка
        };
        match o.status_code {
            Some(code) => {
                site.is_up = Some(code < 500);
                site.status_code = Some(code);
                site.last_error = None;
            }
            None => {
                site.is_up = Some(false);
                site.status_code = None;
                site.last_error = o.error;
            }
        }
        site.response_ms = Some(o.response_ms);
        let ts = now_unix();
        site.last_checked = Some(ts);
        site.history.push(SiteHistoryPoint {
            timestamp: ts,
            is_up: site.is_up,
            response_ms: site.response_ms,
        });
        if site.history.len() > HISTORY_LIMIT {
            site.history.remove(0);
        }
    }
}
