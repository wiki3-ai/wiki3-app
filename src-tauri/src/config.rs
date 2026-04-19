use serde::{Deserialize, Serialize};

/// Trusted origins that receive privileged desktop integration capabilities.
pub const TRUSTED_ORIGINS: &[&str] = &["https://wiki3.ai", "https://www.wiki3.ai"];

/// Default production URL loaded in the main window.
pub const DEFAULT_PRODUCTION_URL: &str = "https://wiki3.ai";

/// Environment variable to override the loaded URL for development.
pub const DEV_URL_ENV: &str = "WIKI3_DEV_URL";

/// App configuration persisted across launches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// The URL to load in the main window.
    pub site_url: String,
    /// Trusted origins for desktop integration.
    pub trusted_origins: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            site_url: DEFAULT_PRODUCTION_URL.to_string(),
            trusted_origins: TRUSTED_ORIGINS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl AppConfig {
    /// Resolve the effective site URL, preferring the dev override if set.
    pub fn effective_url(&self) -> String {
        std::env::var(DEV_URL_ENV).unwrap_or_else(|_| self.site_url.clone())
    }

    /// Check whether a given origin is trusted.
    pub fn is_trusted_origin(&self, origin: &str) -> bool {
        self.trusted_origins
            .iter()
            .any(|trusted| origin == trusted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.site_url, DEFAULT_PRODUCTION_URL);
        assert!(config.trusted_origins.contains(&"https://wiki3.ai".to_string()));
    }

    #[test]
    fn test_trusted_origin_check() {
        let config = AppConfig::default();
        assert!(config.is_trusted_origin("https://wiki3.ai"));
        assert!(config.is_trusted_origin("https://www.wiki3.ai"));
        assert!(!config.is_trusted_origin("https://evil.com"));
        assert!(!config.is_trusted_origin("http://wiki3.ai"));
    }
}
