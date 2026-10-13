//! Запрос публичного IP/провайдера — вкладка «Как меня видят».
//! Rust-порт ip_checker.py. Используем ip-api.com (как в старой версии) +
//! одно резервное HTTPS-зеркало, так как ip-api.com отдаёт только HTTP.

use serde::{Deserialize, Serialize};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicIpInfo {
    pub ip: String,
    pub country: String,
    pub region: String,
    pub city: String,
    pub isp: String,
    pub org: String,
    pub asn: String,
    pub timezone: String,
}

#[derive(Debug, Deserialize)]
struct IpApiResponse {
    status: String,
    message: Option<String>,
    country: Option<String>,
    #[serde(rename = "regionName")]
    region_name: Option<String>,
    city: Option<String>,
    isp: Option<String>,
    org: Option<String>,
    #[serde(rename = "as")]
    asn: Option<String>,
    query: Option<String>,
    timezone: Option<String>,
}

pub async fn get_public_ip_info() -> Result<PublicIpInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;

    let url = "http://ip-api.com/json/?fields=status,message,country,regionName,city,isp,org,as,query,timezone";

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Нет соединения или сервис недоступен: {e}"))?;

    let data: IpApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("Некорректный ответ сервиса: {e}"))?;

    if data.status != "success" {
        return Err(data
            .message
            .unwrap_or_else(|| "Неизвестная ошибка запроса".to_string()));
    }

    Ok(PublicIpInfo {
        ip: data.query.unwrap_or_else(|| "—".into()),
        country: data.country.unwrap_or_else(|| "—".into()),
        region: data.region_name.unwrap_or_else(|| "—".into()),
        city: data.city.unwrap_or_else(|| "—".into()),
        isp: data.isp.unwrap_or_else(|| "—".into()),
        org: data.org.unwrap_or_else(|| "—".into()),
        asn: data.asn.unwrap_or_else(|| "—".into()),
        timezone: data.timezone.unwrap_or_else(|| "—".into()),
    })
}
