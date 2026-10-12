use crate::domain::*;
use std::{collections::BTreeMap, io::Read, sync::Arc, time::Duration};
#[derive(Debug, Clone)]
pub struct DownloadPolicy {
    pub hosts: Vec<String>,
    pub max_bytes: usize,
    pub max_redirects: usize,
}
impl DownloadPolicy {
    pub fn for_hosts(hosts: &[&str], max_bytes: usize) -> Self {
        Self {
            hosts: hosts.iter().map(|s| s.to_string()).collect(),
            max_bytes,
            max_redirects: 5,
        }
    }
}
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub url: String,
    pub status: u16,
    pub bytes: Vec<u8>,
    pub headers: BTreeMap<String, String>,
}
pub trait HttpTransport: Send + Sync {
    fn send(&self, url: &str, policy: &DownloadPolicy) -> AppResult<HttpResponse>;
}
#[derive(Clone)]
pub struct HttpClient {
    transport: Arc<dyn HttpTransport>,
}
impl Default for HttpClient {
    fn default() -> Self {
        Self {
            transport: Arc::new(NetworkTransport),
        }
    }
}
impl HttpClient {
    pub fn with_transport(transport: Arc<dyn HttpTransport>) -> Self {
        Self { transport }
    }
    pub fn get(&self, url: &str, policy: &DownloadPolicy) -> AppResult<HttpResponse> {
        let mut current = validate_url(url, policy)?;
        for _ in 0..=policy.max_redirects {
            let mut last = None;
            for attempt in 0..3 {
                match self.transport.send(current.as_str(), policy) {
                    Ok(response) => {
                        validate_url(&response.url, policy)?;
                        if response.bytes.len() > policy.max_bytes {
                            return Err(AppError::new(
                                ErrorCode::NetworkError,
                                "Respuesta demasiado grande",
                            ));
                        }
                        let retry = matches!(response.status, 408 | 429 | 500 | 502 | 503 | 504);
                        if retry && attempt < 2 {
                            last = Some(Err(AppError::new(
                                ErrorCode::NetworkError,
                                format!("HTTP {}", response.status),
                            )));
                        } else {
                            last = Some(Ok(response));
                            break;
                        }
                    }
                    Err(e) => {
                        let retry = e.retryable;
                        last = Some(Err(e));
                        if !retry || attempt == 2 {
                            break;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(250 * (attempt + 1)));
            }
            let response =
                last.ok_or_else(|| AppError::new(ErrorCode::NetworkError, "No hubo respuesta"))??;
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                let location = response
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                    .map(|(_, v)| v)
                    .ok_or_else(|| {
                        AppError::new(ErrorCode::NetworkError, "Redirección sin destino")
                    })?;
                let next = current
                    .join(location)
                    .map_err(|_| AppError::new(ErrorCode::NetworkError, "Redirección inválida"))?;
                current = validate_url(next.as_str(), policy)?;
                continue;
            }
            if response.status != 200 {
                return Err(AppError::new(
                    ErrorCode::NetworkError,
                    format!("La fuente devolvió HTTP {}", response.status),
                ));
            }
            return Ok(response);
        }
        Err(AppError::new(
            ErrorCode::NetworkError,
            "Demasiadas redirecciones",
        ))
    }
    pub fn json(&self, url: &str, policy: &DownloadPolicy) -> AppResult<serde_json::Value> {
        serde_json::from_slice(&self.get(url, policy)?.bytes).map_err(Into::into)
    }
}
pub fn validate_url(url: &str, policy: &DownloadPolicy) -> AppResult<reqwest::Url> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| AppError::new(ErrorCode::InvalidInput, "URL inválida"))?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !parsed
            .host_str()
            .is_some_and(|h| policy.hosts.iter().any(|a| a == h))
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "La fuente HTTPS no está permitida",
        ));
    }
    Ok(parsed)
}
struct NetworkTransport;
impl HttpTransport for NetworkTransport {
    fn send(&self, url: &str, policy: &DownloadPolicy) -> AppResult<HttpResponse> {
        validate_url(url, policy)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("omarchy-app-updates/0.1.0")
            .build()
            .map_err(network_error)?;
        let response = client.get(url).send().map_err(network_error)?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().into(), v.into())))
            .collect();
        let mut bytes = Vec::new();
        response
            .take(policy.max_bytes as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > policy.max_bytes {
            return Err(AppError::new(
                ErrorCode::NetworkError,
                "La fuente excedió el tamaño permitido",
            ));
        }
        Ok(HttpResponse {
            url: final_url,
            status,
            headers,
            bytes,
        })
    }
}
fn network_error(e: reqwest::Error) -> AppError {
    let mut err = AppError::new(
        ErrorCode::NetworkError,
        "No se pudo consultar la fuente HTTPS",
    );
    err.retryable = e.is_timeout() || e.is_connect();
    err
}
