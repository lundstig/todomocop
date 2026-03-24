use std::time::Duration;

use anyhow::Result;

pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>>;
}

pub struct NoopHttpClient;

impl HttpClient for NoopHttpClient {
    fn get(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        Err(anyhow::anyhow!("no HTTP client configured"))
    }
}

#[derive(Debug, Clone)]
pub struct IntegrationConfig {
    pub github_token: Option<String>,
    pub linear_api_key: Option<String>,
    pub staleness_threshold: Duration,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            github_token: None,
            linear_api_key: None,
            staleness_threshold: Duration::from_secs(5 * 60),
        }
    }
}
